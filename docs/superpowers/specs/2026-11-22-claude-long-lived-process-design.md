# One long-lived Claude Code process per chat: design

Status: waiting for approval. Nothing is built yet.

## Why

Today every Claude turn starts `claude -p`, waits for it to load, connect its tool servers and read the saved
conversation, runs one message, and exits. Stop is a SIGTERM to the process tree. t3code's lesson 7
(`.superpowers/t3code-lessons.md`): keep one process per chat and talk to it over its own protocol. That gives a
Stop the tool itself confirms, no start-up cost on each message or carry-on, and a base for later steering and
live approvals.

## Goals

- A Claude chat keeps one process, `claude -p --input-format stream-json --output-format stream-json --verbose`,
  open across turns. Each message is one JSON line on its stdin.
- Stop asks Claude Code to interrupt and waits for its answer. The process stays usable.
- The per-turn path stays in the code as the fallback, chosen automatically.
- Nothing the user sees changes except speed and Stop: same events, same cards, same usage and limits records.

## Not in this change

- Sending a message into a running turn (steering). Messages typed while it works are held and sent after, as now.
- Live permission requests (`canUseTool`). Refusals are still reported at the turn's end, as now.
- Any other AI. Windows stays on the per-turn path (my call: I cannot measure the interrupt there).

## What was measured (Claude Code 2.1.319, cheapest model, one process)

- Five messages ran on one process with one session id. Every turn starts with a `system/init` line and ends
  with a `result` line. The process then waits for the next line on stdin.
- Interrupt: `{"type":"control_request","request_id":…,"request":{"subtype":"interrupt"}}` was answered with a
  `control_response` in 20 ms, then `[Request interrupted by user]` and a `result`
  (`error_during_execution`, `terminal_reason: aborted_streaming`) 10 ms later. The next message ran normally.
- Closing stdin: exit code 0 after about one second.
- `total_cost_usd` in each `result` is the running total, as the usage log already expects.
- `rate_limit_event` came with the first turn only, not with every turn.

## Approaches considered

1. **The CLI kept open, driven from Rust over stdin and stdout (chosen).** Same binary, flags, permission rules and
   output lines as today, so `consume`, the replay tests and the deny rules carry over. Small change to a core path.
2. **t3code's way: the TypeScript Agent SDK's `query()`.** Adds a Node sidecar and a second language to a Rust app
   for the same wire protocol underneath. Rejected.
3. **Keep per-turn, only pre-warm the next process.** Faster start, but Stop stays a kill and nothing is gained
   for steering or approvals. Rejected.

## Design

### The process

A new `ClaudeLive` (in a new `app-rs/src/claude_live.rs`) holds the child, its stdin, a line reader over its
stdout that lives as long as the process, its error-output tail, and the **launch key**: the full argument list
(without `--resume <id>`) plus the tool-server config it was started with. Each chat (`State`) holds at most one.

**Start.** At a message, when the chat has no live process. Same arguments as today plus `--input-format
stream-json`, with `--resume <id>` when the chat has a session. Own process group, `paths::child_env`, recorded
in the process ledger (`procs::record`) as now, so a crash of the app still ends it at the next start.

**A turn.** The message is written as one `user` line. `consume` reads from the process's reader and returns at
the turn's `result` line instead of at the end of output. `after_turn` runs unchanged. The stall guard runs per
turn as now.

**Mode, model, effort, allowed programs, house rules, project.** All of these are start arguments. Before each
message the app builds the arguments exactly as today and compares them with the live process's launch key. Same:
reuse. Different: close the process and start a new one with `--resume`. So a turn always runs under exactly the
flags the per-turn path would have given it, and no rule can go stale inside a kept process.

**End.** Closing means: close stdin, wait up to 5 seconds for the exit, then end its process tree as today
(SIGTERM, SIGKILL after the grace), reap it, `procs::forget`. A process is closed when:

- its launch key no longer matches (above);
- the chat is an expert, helper or researcher and its turn ended (they hold no slot and no memory between jobs,
  exactly as today; they gain only the confirmed Stop);
- a user's chat or a Lead has been idle for 10 minutes;
- more than 3 idle processes are open: the longest idle goes first;
- the chat is closed, deleted or switched to another AI, a studio run is ended, or the app quits.

An idle process holds no agent slot: slots are counted from working chats, as now.

### Stop

1. Stop marks the turn stopped (as now) and, once the turn's first line has arrived (so Claude Code has taken the
   message; waited for at most 5 seconds), writes the interrupt request.
2. It waits for the turn's `result`, up to `STOP_WAIT` (15 seconds). When it comes, the chat says "Stopped." and
   the process stays for the next message. Measured: 30 ms.
3. Tool processes the turn left running in process groups of their own are ended (SIGTERM, then SIGKILL), never
   the Claude process's own group, which holds its tool servers.
4. No `result` in time: the whole tree is killed as today, the chat says Claude did not stop in time, and the next
   message starts a process with `--resume`.

To be measured while building: that an interrupt during a running shell command ends that command.

### When it fails: back to per-turn

- **Could not start, or it exited before the turn showed anything:** the same message is sent again at once
  through the per-turn path (with `--resume` when there is a session). The user sees nothing but the answer.
- **It died mid-turn** (output had started): the message is not sent twice, since a tool may already have run.
  The turn ends with today's "exited unexpectedly" message and the error tail; the next message starts afresh
  with `--resume`.
- **Died while idle:** noticed at the next message (or by the idle check); a new process is started with
  `--resume`, without a word.
- **Stalled:** the process is ended as today; the next message resumes.
- **Two failures in one chat** (start or death) put that chat on the per-turn path for the rest of the app's run.
- `FRIDAE_CLAUDE_PER_TURN=1` forces the per-turn path everywhere (a way out, and how its tests stay honest).
- "No conversation found" on resume is handled as today in both paths.

### Usage and limits

- Usage: every turn still ends with a `result`; `usage::Turn::from_claude_session` reads it as now. The total is
  per session and already logged as differences, so reuse and restart both add up correctly.
- Limits: `rate_limit_event` is recorded in `consume` whenever Claude Code sends it, as now. Known cost: a kept
  process sends it at its first turn and not with each later one (measured), so in a long chat the plan-limit
  numbers can be older than today. They carry their `seen` time already. Each restart and every other chat's
  first turn refresh them.

### Capability table

`caps.rs` gets one new field, `live: bool` (keeps one process per chat and confirms Stop by protocol): true for
Claude, false for the rest. The code asks the table, not `if ai == "claude"`.

## Tests

Recorded from the real tool into a scratch folder, scrubbed as the others, replayed through `consume` and
`after_turn` (`tests/fixtures/claude/live-*.ndjson`):

1. plain reply; 2. two messages in one process (one recording, consumed as two turns, same session, usage twice);
3. a tool call; 4. Stop mid-turn (the interrupt's `control_response`, the interrupted `result`, then a message
that works); 5. a process that dies mid-turn (output ends with no `result`).

Plus tests with stand-in processes (shell scripts, as the Stop tests use today): launch-key change restarts with
`--resume`; start failure falls back to per-turn; death before output resends once; interrupt not answered is
killed after the wait and said; idle and over-the-cap processes are closed; quitting leaves none.

`cargo test` three times running. Then a scratch-home run in the Browser pane: a two-message chat on one process
(one pid in the ledger across both), and Stop ending a turn at once.

## Left to the builder

Names, the exact shape of `ClaudeLive`, how `consume` is split to read from a kept reader, and thread layout.
