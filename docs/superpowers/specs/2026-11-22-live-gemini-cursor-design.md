# Gemini and Cursor kept open per chat (ACP)

Asked for by the user on 2026-11-22, after Claude Code was moved to one kept process per chat
(`2026-11-22-claude-long-lived-process-design.md`): "plan to implement same thing for gemini and cursor".
Not yet approved for building. Measurements behind every claim here are in
`.superpowers/live-gemini-cursor-findings.md` (git-ignored working notes) and were taken with agy 1.5.14 and
cursor-agent 2026.11.22.

## Why

Today each message to Gemini or Cursor starts the tool afresh and reads what it prints. A kept connection gives what
Claude now has: Stop that takes effect at once (measured: 6 ms for agy, 4 ms for Cursor, against a kill of the process
today), faster replies, and permission requests that come from the tool itself instead of being read out of its output.

## What was found

Both tools speak the same protocol, ACP (JSON-RPC, one line per message, protocol version 1): `agy acp` and
`cursor-agent acp`. Neither has a kept-open stream-json mode like Claude Code.

| | Gemini (`agy acp`) | Cursor (`cursor-agent acp`) |
|---|---|---|
| Existing chats carry on | Yes: a per-message conversation was resumed over ACP | No: "Session not found"; separate stores |
| Stop | Confirmed in 6 ms, the running command ended, next message worked | Confirmed in 4 ms |
| Token usage | In the answer to each prompt | None anywhere |
| The tool's own allow and deny rules | Not applied: every tool call is asked of the app | Applied; a program not on the list arrives as a live request |
| Images, model and effort changes | Work, also mid-session | Model ids differ (`auto` is `default[]`) |
| Not measured | Full auto flag over ACP; usage-limit shape | Sandbox over ACP; writes outside the workspace; a tool's picture result |

## Goals

- One kept process per chat for Gemini, then for Cursor, behind the same rules Claude's kept process follows: started
  on the first message, closed when idle, never holding a slot when not working, discarded and restarted with resume
  when anything it was started with changes (mode, model, allowed programs, guarded settings, folders, tool servers),
  recorded in the process ledger, ended on quit and reaped after a crash.
- The per-message path stays and is chosen automatically when the kept one fails; the message the user typed is never
  lost or sent twice. One environment switch per AI turns the kept path off.
- Nothing a user can do today becomes less safe.

## Non-goals

- OpenCode (already on a live server) and ChatGPT (not offered in the window).
- Windows: per-message stays there until someone can test it.
- Any change to how experts, studio runs or review work.

## Anti-goals

- A mode offered that the app cannot enforce. If Cursor's sandbox does not hold over ACP, Cursor's Full auto and
  Studio stay per-message, or Cursor stays per-message entirely.
- A chat's history silently lost on fallback.

## Approaches

1. **One ACP client, shared by both, on top of the kept-process holder made for Claude (chosen).** The protocol is the
   same; per tool only the start command, how a session is made or resumed, model ids, where usage comes from and how
   the process is ended differ. Any future ACP agent then costs a table row.
2. **A client per tool.** Less abstraction up front, twice the code and twice the bugs for one protocol. Rejected.
3. **Keep per-message and only warm the tool between turns.** Measured not to be a real alternative for Cursor; gives
   none of Stop, permissions or speed. Rejected.

## Design

- **Holder.** `claude_live::Live` and its policy (launch key, idle close, keep at most three idle, two failures then
  per-message for that chat, the per-turn switch) are made generic over the AI and keyed on the capability table
  (`caps.live`). Claude's behaviour does not change; its tests stay green unchanged.
- **Client.** A new `acp.rs`: line-delimited JSON-RPC over the child's stdin and stdout; `initialize`; `session/new` or
  resume/load; `session/prompt`, whose answer is the end of the turn; `session/update` notifications mapped to the
  same `Out` values the per-message parsers produce, so the chat, experts, review and usage code are untouched;
  `session/cancel` with the confirmed-Stop rule (15 s, then the process group is ended); `session/request_permission`.
- **Permissions, Gemini.** Because agy's own rules do not apply over ACP, the app is the gate for every tool call, with
  the checks it already has: `paths::command_allowed` for commands, the protected-paths list for writes, the folder
  set of the chat for everything else. Ask me: allowed things are answered at once, the rest show the approval card.
  Full auto and Studio: session mode stays `default` and the app allows everything except protected paths (this is
  stricter than today, where agy in Full auto can write a differently-cased name of a protected file). The Full auto
  flag over ACP is not used.
- **Permissions, Cursor.** Cursor's own allow list, deny rules and per-chat isolated folder stay exactly as today; live
  requests for programs not on the list go to the approval card in Ask me and are refused in Full auto, as now.
- **Existing chats.** Gemini: resumed over ACP, and a fallback resumes the same conversation per-message. Cursor: a
  chat keeps the path it started on; new chats start on ACP; a fallback inside an ACP chat hands the transcript over as
  text, the same way an AI switch does, and says so in one line.
- **Usage.** Gemini: from each prompt's answer. Cursor: none is reported, so the usage page says "not reported by
  Cursor over this connection" for such turns instead of showing zero.
- **Order.** Gemini first (no losses). Cursor second, and only after three measurements pass: the sandbox holds over
  ACP, a write outside the workspace is refused, and a tool server's picture result reaches the chat. Each that fails
  narrows Cursor's kept path (first to Ask me only, then to none).

## Tests

- Recorded real ACP traffic per tool replayed through the real client and turn ending, only the pipe faked: plain
  reply, two messages on one process, a tool call, a permission request allowed and one refused, Stop mid-turn, a
  process that dies mid-turn and idle, signed out, resume of a per-message conversation (Gemini), "Session not found"
  (Cursor).
- The launch key: each thing a process is started with, changed, restarts it.
- Every Gemini permission decision the app now makes, including a protected path in another case and a command the
  strict check refuses.
- A scratch-home run in the Browser pane per tool: two messages on one process, Stop at once, an approval card.

## Left to the builder

- The exact trait or table shape that makes the holder generic.
- Field names per tool, taken from the recordings, never from memory.
- Wording of the new chat lines and the usage note.

## Open for the user

- Cursor loses token usage on the kept path. If that matters more than instant Stop, Cursor should stay per-message.
