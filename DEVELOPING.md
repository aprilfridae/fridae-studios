# rbxkit

A toolkit that lets an AI agent build Roblox games properly: model in Blender
and check the result, draw VFX textures, build and capture visual effects
frame by frame, render item icons, animate rigs, and run real playtests.

It works with **any agent that supports MCP servers** (Claude, Cursor, Codex,
Gemini CLI, Windsurf and others). Nothing here needs an account or a
subscription, and all of it is readable source.

## Set up (about two minutes)

You need:

1. **Python 3.9+** from python.org (tick "Add python.exe to PATH").
2. **Blender** from blender.org (any recent version).
3. **Roblox Studio**, with its MCP server turned on and connected to your
   agent (Studio > Assistant settings). That is what lets the agent see and
   edit your place; rbxkit adds everything around it.

Then unzip this folder somewhere permanent and double-click **`install.bat`**
(or run `python install.py`). It installs three Python libraries, finds
Blender, and connects rbxkit to the agents it finds on your computer. Restart
your agent afterwards.

For an agent it did not find, add this MCP server by hand (the installer
prints the exact lines for your computer; `python install.py --print` shows
them again):

```json
{ "mcpServers": { "rbxkit": { "command": "python", "args": ["<this folder>/rbxkit_mcp.py"] } } }
```

Check everything at any time with `rbx doctor`.

## The desktop app

The chat window is a Rust program in `app-rs` (a native WebView2 window showing
`ui/index.html`). It runs Claude Code with the rbxkit and Roblox Studio tools
attached and shows the images they return. The tools stay in Python: the window
starts them as `fridae-cli.exe --mcp` and `fridae-cli.exe --rbx`. `build_exe.bat`
(or `python build_app.py`) builds both and puts them in one folder; it needs
Rust and PyInstaller. On a Mac, `python3 build_mac.py` builds
`build-out/Fridae-Studios-macOS.dmg` the same way. The packaged app needs no Python or Rust installed; it keeps
its output in `Documents/Fridae Studios/work`.

## Using it

Just ask your agent for what you want: "model a street lamp and put it by the
spawn", "make a ground-slam effect", "render icons for everything in
ReplicatedStorage.Items", "playtest the shop and tell me what breaks". The
agent reads the built-in guides and uses the tools itself.

## What the agent gets

| Tool | Does |
|---|---|
| `rbx_guide` | The how-to guides: workflow, playtest, ui-design, sim-ui, vfx; one `section`, or an outline of a large guide |
| `rbx_vault` | One lookup over the Roblox vault, the guides, the team's expert notes and this game's notes (see below) |
| `rbx_blender` | Runs Python in headless Blender: model, check for floating parts, render, export, animate |
| `rbx_texture` | Draws particle textures and flipbook sheets, checks them, previews them in Studio for free |
| `rbx_check_images` | Checks any texture or flipbook sheet from its pixels |
| `rbx_icon` | Cuts a transparent icon out of an icon-booth screenshot |
| `rbx_image` | Makes an AI-generated image (icon, thumbnail key art, UI art) on your Gemini plan through Antigravity, or keeps an existing image, saved in the game's project at an exact size |
| `rbx_video` | Reads a reference video as a sheet of frames |
| `rbx_animation` | Turns a Blender animation into a Roblox animation file |
| `rbx_vfx_package` | Installs the VFX modules into a place without uploading |
| `rbx_studio_snippet` | Ready-made Luau for Studio: VFX player, lightning, capture, icon booth, model placement, rig export |
| `rbx_promo` | Vertical promo videos from gameplay clips: probe, reference, voice, render, check (needs the promo engine) |
| `rbx_doctor` | Says what is missing and how to fix it |

Claude Code users also get the guides as skills and a `roblox-tester` agent.

The same tools are a command line: `rbx <command>` (`rbx --help`).

## The promo engine

The promo maker renders with a separate engine that is downloaded on first use, never bundled: a portable
Node 22, a copy of `promo/` with its npm packages (Remotion, transformers.js, kokoro-js), Remotion's headless
browser, and the Whisper and Kokoro models. It lives in `engine/` in this folder when running from source, and
in the per-user app data folder in the packaged app (`rbxpromo.engine.engine_dir()`): macOS
`~/Library/Application Support/Fridae Studios/engine`, Windows `%LOCALAPPDATA%\Fridae Studios\engine`. Not in
Documents, which iCloud or OneDrive may sync; an engine earlier builds left in `Documents/Fridae Studios/engine`
is moved there the first time the engine is used (`migrate_engine`).

- `python3 -m rbxpromo.engine install` (or `rbx promo install`) installs it, about 730 MB. Every finished step is
  written to `engine/.state.json`, so running install again resumes instead of starting over. `rbx promo status`
  says what is missing. In the app, Setup, Connections, Promo engine runs `rbx promo install --json-progress`.
  An install holds `.install.lock` (pid and start time) in the engine folder; a second install while that process
  lives is refused, a lock left by a stopped one is taken over. Quitting the app stops the install's whole
  process tree (its own process group on macOS, `taskkill /T` on Windows).
- The renderer is a copy: after editing anything in `promo/`, run `python3 -m rbxpromo.engine install` again to
  refresh `engine/renderer` (it re-runs `npm ci` only when `package-lock.json` changed).
- The packaged app ships `promo/` without `node_modules` (`rbxkit.spec`) so the install can copy it.
- onnxruntime (used by the Whisper and Kokoro scripts) can abort as Node exits, with exit code 134, after the
  work is done. The Python side judges those scripts by what they printed (a JSON line with `"ok"`, or a
  "model ready" line), not by the exit code; expect the 134 when you run `promo/scripts/voice.mjs` or
  `transcribe.mjs` by hand.

Tests: the promo tests need the engine installed, and the ones that read video need OpenCV, so run them with the
virtual environment's Python:

```
.venv/bin/python -m unittest discover tests -v
engine/node/bin/node --experimental-strip-types --test 'promo/src/captions/*.test.ts' 'promo/scripts/*.test.mjs'
```

Without the engine the engine-gated tests are skipped, not failed.

## Studio mode

A Studio run is the game's own chat (the Lead) in a run mode built on Overnight's loop, plus helper agents started
as named experts, plus small files in the project. The spec is
`docs/superpowers/specs/2026-11-21-studio-mode-design.md`.

**Files in the project** (all JSON writes are atomic, and every read-modify-write holds `<project>/.fridae/.lock`,
created with O_EXCL, retried for about 5 s and taken over after 10 s; `rbxstage.locked` in Python,
`app-rs/src/fridae_lock.rs` in Rust):
- `.fridae/studio.json`: the run (`on`, `stage`, `stages`, `gates`, `max_experts`, `lead`, `idea`, `started`,
  `paused`, `read_decisions`, `checkpoints`, sometimes `note: "slots"`, and `ended` once Review's End this run switched
  it off). Unknown keys are kept by both sides. `rbx_stage stage` refuses to change an ended run.
- `.fridae/review/<id>.json` (status `waiting` | `approved` | `changes` | `superseded`) and its media in
  `.fridae/review/<id>/` (copied in, never decoded). A new run moves old items to `.fridae/review-archive/<started>/`;
  an undo moves later ones to `review-archive/undo-<stamp>/`.
- lessons: `rbx_stage lesson` writes a note (`rbxnotes.py`) in `.fridae/notes/`, or in `<home>/notes/` for every
  game. An older `.fridae/lessons.md` is still searched; an older `WORK/studio-lessons.md` moves to `<home>/notes/`.
- `.fridae/.gitignore`, written when a run starts: `review/*/`, `review-archive/`, `before-undo/`, `.lock`, so
  checkpoint commits never carry media.
- `.fridae/team/roster.json` and `.fridae/team/experts/<id>.md`: optional per-game overrides of the kit's team.

**The agent side.** `rbxstage.py` is the `rbx_stage` tool (`status`, `stage`, `review`, `decisions`, `lesson`;
CLI `rbx stage ...`). A gated stage can only be finished once a blocking item of that stage is approved and none is
still open; a newer approved blocking item supersedes older ones of the stage still marked `changes` (a redo filed
without `replaces`), and a refusal names any item still marked `changes` with the `replaces=<id>` to use.
`decisions` hands each answer over once (`read_decisions` holds `id:status`); `status` keeps printing the note of
every open `changes` item, saying which were already handed over, and an undo that reopens an item drops its
`id:changes` mark so it is handed over again. Finishing a stage writes
`WORK/stage-requests/<id>.json` for the app's checkpoint. `rbxroster.py` loads `vault/Fridae/Team/roster.json` (fields `id, name,
job, stage, guides, tools, kind, writes`) with the project's overrides merged field by field, and checks every field.
`rbx_team spawn` takes `expert`: `rbxteam.py` resolves it and puts `expert`, `expert_name`, `expert_text` (at most
16,000 characters) and `writes` in the request; the Lead itself cannot be spawned, and helpers still cannot start
helpers.

**Experts on a chosen AI and model** (spec `docs/superpowers/specs/2026-11-22-expert-ai-choice-design.md`).
`rbx_team spawn` also takes `ai` and `model` (`rbxteam.py` puts them in the request). `team.rs` asks
`expert_ai::resolve`, one pure function with tests: the user's Setup > Experts row (config `expert_ai: {<expert id>:
{"ai", "model"}}`) wins, then the request, then the Lead's own AI and model (`Agent::model`: the model it was started
on, else its last message's). An AI the app cannot run experts on (ChatGPT, anything unknown) or one that is not
installed is refused with a reason the Lead can act on, never moved to another AI; a model the chosen AI does not list
runs on the nearest it has (`World::nearest`, OpenCode's only: `opencode::nearest`) or on that AI's default, and the
result's `note` says which and why (a list that has not arrived yet passes the model on). Nothing about where an
expert runs is silent: the result's `model` always names the model (the AI's default when none was set), the `note`
also says when the user's table won over what the Lead asked for and what the app knows about the model that runs
(`World::why`, `expert_ai::More`, OpenCode's `Caveat`: only free models because Fridae's OpenCode is not signed in, a
sign-in that waits for running work, no plan model listed, or the same model billed per use that was left for the
user to pick), and
a model swapped later, at a turn (the list arrived after the spawn, or the model has gone), is said once under the
message in that chat (`agents::model_line`, the "user" event's `model_note`: model names, two short sentences at most,
one line a message; nothing is claimed about an AI whose list has not arrived) and added to the result the Lead
collects (`team::with_model_note`). What a chat was told is read back from its saved messages when it is reopened
(`told_from_log`). `rbx_team` action `models` lists what each AI offers, from `work/team-models.json`, which
`team::watch` keeps holding exactly the app's list (`expert_ai::for_lead`, `write_models`; the file is among those no
agent's file tools may write, `paths::app_protected`). The
expert's chat keeps its model (`State.model`, saved with the chat) for every later message that names none. Its
`agent_new` and the Lead's `helper_started` carry `runs_on` ("Cursor · Auto") when it differs from the Lead's, which
the expert line and Review's Right now show. Usage is recorded under the expert's own AI, as for any chat.
`/api/experts` lists the kit's roster (not the Lead), the table, and for Claude, Gemini, Cursor and OpenCode whether each is
installed, its models and its default; with `expert`, `ai`, `model` it saves one row ("" is Same as the Lead). Beside
Cursor the table shows `cursor::EXPERT_GAP` (experts run with Ask me's rules: Cursor may also write into temp
folders). The Lead's text (`vault/Fridae/Team/lead.md`) and the studio guide ("Experts on another AI") say the option
exists, that the user's table wins, and to use it sparingly.

**The Lead's prompt.** `app-rs/src/text/studio.txt` (the mode's rules), then `vault/Fridae/Team/lead.md` (the project's
`.fridae/team/experts/lead.md` first), and `text/kickoff_studio.txt` on the first message; all `include_str!`'d except the team files, which are read
from the kit at run time. The `studio` guide is `vault/Fridae/Guides/Core/studio/studio.md` (registered in `rbxkit_mcp.GUIDES`).

**The app side** (`app-rs/src`):
- `agents.rs`: `Ask.studio`; the pure loop functions `loop_mode` / `next_step` / `pause_step` / `ended_step` /
  `no_progress`. `STATUS: WAITING` stops the 24-hour clock and emits `studio_waiting`; a user message or a decision
  resumes it. Three failed turns in a row end the run (a `result` event with `is_error`, such as a signed-out AI,
  counts as failed), and three carry-ons in a row without a tool call count as `STATUS: WAITING`. Any message to the
  chat that leads a running run is a Studio message (`studio_message`, also for held messages merged into one turn),
  and takes no night checkpoint once the run has its `start` one (`takes_night_checkpoint`). A Studio message in a
  chat with no project goes as Ask me, with a line saying so (`studio_fallback`). `MAX_AGENTS` is 8 and counts slots,
  not open chats: a helper whose result is handed back holds none while it is not working (`holds_slot`), and the
  page's status has `slots_used`. Expert chats save and restore `expert_text`. Tests never start an AI (`send` finds
  none under `cfg(test)`).
- `team.rs`: starts experts (chat titled with the expert's name, `expert_text` appended to the helper prompt) and
  refuses a second `writes: true` expert per project (in-memory writer place, released when the helper finishes or is
  stopped or closed), and refuses a request from a run's Lead while `max_experts` of its helpers are at work. A spawn
  refused for slots sets `note: "slots"` on the run; when a slot frees (a helper finishes, a chat is closed) a waiting
  Lead of a run with that note has it cleared and is woken with "An agent slot is free. Carry on." (`slot_freed`).
- `studio_run.rs`: the routes' work. `GET /api/review?path=`, `POST /api/review {path, action: approve | change |
  approve_all | needs_done, id?, note?, ids?, title?}` (`approve_all` approves only the `ids` the page shows;
  `needs_done` tells the Lead "Board: <title> — done" after a needs card is done in Review), `POST
  /api/studio-run {path, action: start | resume | pause | settings | undo | end, ...}` (`end` sets `on: false` and
  `ended`, stops the Lead carrying on, and deletes nothing; a new run may then start),
  `GET /media/review/<key>/<id>/<file>` (key = first 12 hex of the SHA-1 of the stored project path; served only for
  files listed in the item and inside `.fridae/review/`). All need the token, and `path` must be a known project.
  A decision writes the item and wakes the Lead with `Review: <title> — approved` / `— change: <note>`; while the run
  is paused the decision is only recorded.
- `studio_watch.rs`: polls every project's `review/` and `studio.json` (700 ms) and sends `review` / `studio_run`
  BUS events to the page.
- `studio_undo.rs`: turns stage requests into checkpoints (git commit of the project + a Studio plugin pin, appended
  to `checkpoints`), and does `undo`: check Studio's part first (closed, no pin at the chosen checkpoint although a
  later one used Studio, an old plugin, a restart: refused with the files-only offer before anything changes), then
  halt and pause the Lead (a refusal after this point says the run is paused), refuse while anything in the game is working, commit
  the current files, save Studio's scripts to `.fridae/before-undo/<time>/` (plugin `sources` → `rbx project
  save_scripts`), revert Studio to the pin taken exactly at the checkpoint, restore the files as a new commit, rewind
  the run record and archive later review items. Studio closed while a checkpoint from the chosen one on used it, Studio restarted
  (no pin), or an old plugin: it refuses and offers `files_only: true`. Script snapshots live in the plugin's memory (plugin 2.8.0 keeps the newest 24 pins).
  The Lead's note says only what really went back. Overnight's Undo the night uses the same `put_back` with `exact:
  false`: the agents' Studio steps since the night began are undone by time, as before, even when Studio was closed
  when it began (the scripts are still saved to disk first, so it too needs the current plugin).
- `ui/index.html`: the Review tab, Studio in the run-mode menu, experts by name, the undo picker.

**The page's shape** (`ui/index.html`, one file, no framework). The sidebar lists All projects, the games
(`GET /api/projects`), No project (only while chats without a game exist) and New project; `curProj` is the game in
front (`""` for No project, `null` on the All projects and New project pages) and `view` is `chat`, `review`,
`promos`, `all` or `newproj`. The place is kept in `localStorage` under `place`; on start All projects opens when a
game has a run going or something waiting for the user (`needsOf`: blocking review items and needs cards, the one
count the sidebar, the Review tab and Needs you share). Chats are grouped by `agent.project`, not by AI. A helper
(an agent with `parent`) never gets a row: it is one collapsible line in its parent's chat, placed where the parent's
log has the `helper_started` event (`{helper, name, expert}`, sent by `spawn_named`). Restored agents carry `role`,
`parent`, `team` and `expert` in `agent_new`. All projects is a grid of cards, one per game (`apCard`, `paintAll`),
and a New project card; a card opens the game's Review when something needs the user, else its Chat. The message box
holds three menus (`POPS`: what to add, the AI and model picker `paintPicker`, the mode menu `paintSwitch`);
`aiNotice(ai)` is the one place to say something about an AI in the picker and in Setup, Connections. The clean-up's
spec is `docs/superpowers/specs/2026-11-23-ui-cleanup-design.md`.
- `POST /api/ai {agent, choice}` on a chat that has started switches it (`Agents::switch_ai`): refused while it works,
  otherwise the session is dropped and `handover` holds the chat so far as text (only what the user typed, as
  "User:", and each AI's replies under its name; carry-ons and the app's own messages, logged with `app: true`, are
  left out, except the note of an "Ask for changes" in Review, which is the user's; a turn that merges typed and app
  messages logs the typed part as `typed`; newest kept under 24,000 characters). It goes ahead of every message until the new AI reports a
  session (`got_session`), so a first turn that fails or is stopped hands it over again; it is saved with the chat.
  The chat logs `{"kind": "ai", "ai", "from"}`; the page shows one line for it. `POST /api/send` takes `app: true`
  for a message the page sends for the user (the blocked-command card's).
- Slots: an expert, helper or researcher (`parent`, `team` or role `research`) that is not working holds no slot
  once its first turn has run (`holds_slot`). Closing a chat closes the experts it started (`remove_agent`), End this
  run closes the run's experts, the researcher closes itself once its ideas are stored, and Find ideas closes an
  earlier idle researcher. Helpers are not listed in `_open.txt`, so a restart does not reopen them; the page reopens
  one (`/api/open-chat`) only to read its log when its line is opened.
- The page makes no chat just for looking: a game with no chat shows an empty page (`showDraft`), and the chat is
  made (`/api/new` with the chosen AI, then `/api/project use`) when its first message is sent.
- Usage lines carry `project` (the chat's game) so Review and All projects can show what a game has cost.

**Testing without an AI.**
- Python: `python3 -m unittest tests.test_stage tests.test_roster tests.test_team_expert tests.test_project_save
  tests.test_studio_flow tests.test_tool_pictures tests.test_long_calls` (the last starts real tool server
  processes and takes about half a minute). `tests/test_studio_flow.py` is a dry run of a Lead through `rbxkit_mcp.handle` with a stub
  app thread that answers `rbx_team` requests as finished experts.
- Rust: `cargo test --manifest-path app-rs/Cargo.toml` (loop, studio_run, studio_undo and team tests use temp
  projects named with spaces and accents; under `cfg(test)` everything, the work folder included, goes in
  `<temp>/fridae-tests/<pid>/`, whatever `RBXKIT_WORK` says, so nothing writes to the real work folder; the next test
  run clears the folders of runs that are over).
- AI processes (`agents.rs` `TurnProc`, `procs.rs`; Unix first, Windows unchanged: `taskkill /T /F` at once and no
  ledger there):
  - Every turn of a command-line AI (Claude Code, Antigravity, Cursor, Codex) starts in a process group of its own and
    is noted in the process ledger (`work/processes/<app pid>.json`: pid, group, start time, command line), noted again
    at its first output line (cursor-agent's launcher script has replaced itself with node by then, which changes the
    command line) and dropped once reaped. At the next start, an entry of a crashed app still running exactly as
    noted is ended with its whole tree (`procs::tree_groups`: Cursor runs each shell command in a group of its own).
    Measured with a real Cursor turn running `sleep 120`, app SIGKILLed: the next start ended cursor-agent, its
    worker-server, its language server and the `sleep`. OpenCode's server goes through the same ledger.
  - The error output is always read to its end on a thread of its own, and only its last 64 KB are kept
    (`paths::drain_tail`, `TAIL`).
  - Stop is confirmed: the turn's tree gets SIGTERM (every group in it), up to 15 s (`paths::STOP_WAIT`) for its
    output to end, then SIGKILL, and the chat gets one line saying it had to be ended (`paths::killed_message`)
    before "Stopped.". What is left after it ended in time, in its own group or in groups its tools were put in,
    gets 2 s more, then SIGKILL. At every end (Stop or not), once the output has ended, whatever is left in the
    turn's own group is ended for every AI (SIGTERM; SIGKILL after 2 s only when the group then still has a member
    besides its leader, so a group ID given out again is never signalled), and the error output is waited for only 4 s
    more (what was read of it by then is kept, even when a holder outside the group keeps the pipe open), so a child that ignores SIGTERM and holds it can neither outlive the turn nor keep the chat busy (tested). A gate (`TurnProc::reaped`) keeps the leader from being reaped while its group is signalled, so a group
    ID never names someone else's. Measured with a real Cursor turn in `sleep 90`: Stop ended it and the `sleep` in
    0.2 s (before this, signalling the turn's group alone left Cursor's shell running until the command ended).
    OpenCode's Stop interrupts the session, waits the same 15 s, then ends the server.
  - Every ending (result, Stop, failure, the process exiting, the stall guard, quitting) goes through `after_turn`,
    which ends with `done`: the page settles the turn's open steps on it, and a chat reopened without one is settled
    at once (`open_chat`). Quitting ends every turn's tree, then waits up to 3 s for the chats to settle before they
    are saved.
  - Stall guard: a turn with no output line for 20 minutes (`paths::STALL`, longer than a tool server call may take,
    `MCP_TOOL_TIMEOUT` 15 minutes, so a hung tool returns a time-out the AI can act on; Cursor does not keep to that
    variable and cuts a tool server's call off after 60 seconds, measured, see "Cursor: one `cursor-agent acp`
    process") is ended with a plain message
    (`stall_message`); 60 minutes while a tool call is open (`STALL_WORKING`: a long build; for Claude Code counted by
    call id until its own result, for Cursor and Codex when its last line started a tool); never while its
    last line started one of the app's tools that wait on purpose (`rbx_team`, `rbx_texture`,
    `rbx_generate_texture`), while a texture is being made, or while it is a Lead with experts working, and the quiet
    clock starts again when such a wait ends (`watch_stall`, checked every 5 s; it decides under the same gate the
    turn's end is noted under, so a turn that just ended is never called stalled, and a stopped turn says
    "Stopped."). OpenCode applies the same constants to its events.
  - Environment (`paths::child_env`): every AI's process (turns and model listings) goes without a parent Claude
    Code's `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`,
    `CLAUDE_PID` and `CLAUDE_AGENT_SDK_VERSION`, and without the provider keys that would bill an API account instead
    of the plan the user signed in with (`plan_keys`), unless the user turned on the Setup switch "Use API keys from
    my environment" (`use_env_keys`, off by default so Fridae never spends money the user did not agree to; on, the
    keys are left alone, except OpenCode's, which it never gets since each would add a provider billed to it, and the
    label says so; a failed sign-in in the AI's own words while such a key is set names the switch,
    `agents::key_note`; the six Claude session variables go either way): Claude Code `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`
    (`CLAUDE_CODE_OAUTH_TOKEN`, a plan's own token, stays); Cursor `CURSOR_API_KEY`; Codex `OPENAI_API_KEY`,
    `CODEX_API_KEY`; OpenCode's server `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `OPENAI_API_KEY`,
    `OPENROUTER_API_KEY`, `GROQ_API_KEY`, `XAI_API_KEY`, `DEEPSEEK_API_KEY`, `MISTRAL_API_KEY`, `GEMINI_API_KEY`,
    `GOOGLE_API_KEY`, `GOOGLE_GENERATIVE_AI_API_KEY`, `AWS_BEARER_TOKEN_BEDROCK`, `AZURE_OPENAI_API_KEY`,
    `CURSOR_API_KEY` (its tool servers get the two Gemini keys back). OpenCode's server and the shell its commands run
    in also go without the variables `/bin/sh` acts on at start (`paths::shell_injected`: `BASH_FUNC_*`, `BASH_ENV`,
    `ENV`, `CDPATH`, `SHELLOPTS`, `BASHOPTS`, `PS4`, `IFS`, `GLOBIGNORE`), so an allowed plain name is the program on
    PATH. Antigravity keeps the Gemini keys (the app's
    textures read them, and agy signs in by its own settings) and gets `AGY_CLI_DISABLE_AUTO_UPDATE=1`. Measured: a
    Cursor turn started from an app with `CLAUDECODE`, a fake `CLAUDE_CODE_SESSION_ID` and a fake `CURSOR_API_KEY`
    ran on the sign-in, and its process had none of them. On Windows the same variables are removed.
  - The tests check Stop with real shells that start a grandchild, one of them ignoring SIGTERM (killed after the
    wait, and said), and the tree, tail and environment rules.
- Recorded output (`app-rs/tests/fixtures/<ai>/`): real output of each AI, scrubbed (session ids replaced by
  `00000000-…`/`ses_fixtureNNNN`, the scratch folder by `/scratch`, the home folder and user name, Claude's thinking
  signatures, and the user's own hooks, plugins and skills from Claude's `init` line). Replayed in
  `src/replay_tests.rs` (Claude, Gemini, Cursor: through `Agent::consume`, the same code that reads the child's
  stdout, then `after_turn` with the recorded stderr and exit code) and `src/opencode_replay.rs` (through `turn_on`
  against a stand-in HTTP server that answers each call as OpenCode did and streams the recording as `/api/event`).
  Cases: Claude plain reply with its `rate_limit_event`, a resumed session with two commands, a resume of a
  conversation that is gone (and five recordings of a kept process, see "Claude Code: one process kept open per
  chat"); Gemini plain reply, a resumed turn with a file written and a command refused, a resume
  of a conversation agy no longer has (and fifteen recordings of a kept process, see "Gemini: one `agy acp` process
  kept open per chat"); Cursor plain reply, a command, a refused `./git`, a resumed chat, signed out, a job of four minutes waited for
  ("Long tool calls");
  OpenCode plain reply, resumed session, a command, a refused command answered, a failed turn, the free-usage limit.
  `every_fixture_is_scrubbed` fails if the user's folder, the scratch path or an email shows up. To add one, record
  with the real tool into a scratch folder and scrub the same way; nothing in them may be typed by hand.
- The app by hand, in a scratch folder so the real `Documents/Fridae Studios` is never touched. Make
  `<scratch>/kit` with symlinks to this repo's `ui`, `team`, `rbxkit_mcp.py` and `rbx.py` and its own `config.json`
  (`{"chat_ai": "claude", "projects": [{"name": "...", "path": "<scratch game folder>"}]}`; `git init` the game),
  then run the dev binary:

  ```
  env -i HOME=<scratch>/home PATH=/usr/bin:/bin TMPDIR=<scratch>/home FRIDAE_KIT=<scratch>/kit \
    RBXKIT_WORK=<scratch>/work FRIDAE_NO_STUDIO=1 FRIDAE_URL_FILE=<scratch>/url.txt FRIDAE_QUIT_AFTER=2400 \
    app-rs/target/debug/fridae
  ```

  `url.txt` gets the page's address with its token; open it in a browser. With that HOME and PATH no AI is found
  (or Claude Code is signed out), so nothing uses an AI plan; write `studio.json` and review items by hand, or with
  `python3 rbx.py stage ... --project <game>`, and the page follows them. The packaged app
  (`build-out/Fridae Studios.app/Contents/MacOS/Fridae Studios`) ignores `FRIDAE_KIT` and keeps its config, games
  and work folder in `$HOME/Documents/Fridae Studios`, so give it the scratch `HOME` and put `config.json` there.
  `FRIDAE_NO_STUDIO=1` gives agents no Roblox_Studio server and passes the variable to rbxkit, whose Studio tools
  (`rbxstudio.Studio`, so `rbx_project` checkpoint and restore waypoints, and `rbx_studio_snippet`) answer "Studio is
  off for this run"; the app also treats its Studio plugin as not connected (no waypoints, no selection notes).
- Usage: Claude Code's `total_cost_usd` is the session's running total, so `usage.rs` logs each turn's difference
  from that session's previous total (`session` and `session_cost` are kept in `work/usage.jsonl` so this survives a
  restart); token counts are already per turn. Lines written before this change hold running totals.

## Claude Code: one process kept open per chat

A Claude chat keeps one process, `claude -p --input-format stream-json --replay-user-messages --output-format
stream-json --verbose ...`, open across turns (`app-rs/src/live.rs` is the process and its pipes, shared with
Gemini's kept process; `claude_live.rs` is what Claude Code's protocol adds; `Agent::live_turn` and `read_live` in
`agents.rs` run a turn on it, through the `Kept` trait). Each message is one JSON `user` line on its
stdin, with an id of the app's own. The old way, a process per turn with `--resume`, is still there
(`Agent::spawn_turn`, `read`, `finish_turn`) and is the fallback. Checked against Claude Code 2.1.319;
`caps.rs` says which AI works this way (`live`). The design is `docs/superpowers/specs/2026-11-22-claude-long-lived-process-design.md`.

- **What was measured** (cheapest model, real runs): five messages on one process and one session id; every turn
  starts with `command_lifecycle` lines (`queued`, `started`) carrying the message's id and a `system/init` line whose
  `user_message_uuids` holds it, the message is played back (`isReplay`) just before the first answer, and the turn
  ends with a `result` line; a `command_lifecycle` `completed` or `cancelled` follows the result. The process then
  waits. Closing its stdin: exit 0 in about a second (so a kept process also ends by itself when the app dies).
  `total_cost_usd` is the session's running total, as `usage.rs` already takes it. `rate_limit_event` came with a
  process's first turn only, so in a long chat the plan-limit numbers are older than they were with a process per
  turn (each carries its `seen` time).
- **A turn's lines** go through `consume_lines`, the same parser as a per-turn process, which stops at the `result`.
  A thread reads the process's output for as long as it lives; a turn first drops what was written since the last
  one, and until Claude Code says it has *started* on the message (the `started` mark, the `init` line naming the
  id, the message played back; a `queued` mark only says the message was read), an answer or good result is taken
  to be left over from the turn before (a failed result always ends the turn).
- **Claude Code starts turns by itself.** Measured (`tests/fixtures/claude/live-background.ndjson`): a turn started
  a shell task with `run_in_background` and ended; seven seconds later, with nothing sent, the task finished and
  Claude Code ran a whole turn of its own (`init`, an answer, a `result`). Nobody asked for such a turn, the chat
  shows none of it and Stop would not find it, so it is never left to run: a turn that ends with a background task
  still open (`background_tasks_changed` with tasks, or `task_started` with `is_backgrounded`) has its process
  closed at once, and the reading thread ends the process, tree and all, at the first `init` or `assistant` line
  that comes while the app has no turn on it (`Live::strayed`; not counted as a failure). The next message starts
  a process that resumes, which is what a process per turn always did.
- **Mode, model, effort, allowed programs, house rules, project, tool servers** are start arguments. Every message
  builds them as a per-turn process would get them (`ClaudeLaunch`); a kept process started with anything else is
  closed and a new one started with `--resume`. Nothing can go stale inside a kept process.
- **Stop** (`TurnProc::stop_live`). Before Claude Code has started on the message, an interrupt ends nothing and the
  message runs afterwards (its answer to an interrupt lists what is "still queued"), so the process is ended at
  once, tree and all, and the chat says "Stopped.": nothing has run, and the next message resumes. Once it has
  started, the app writes `{"type":"control_request","request_id":…,"request":{"subtype":"interrupt"}}`. Measured:
  a `control_response` in 20 ms and the turn's `result` (`error_during_execution`) 10 ms later, with a running shell
  command ended by Claude Code itself; what Claude had written so far is still shown. No `result` within
  `STOP_WAIT`, or an answer that the message is still queued: the whole tree is killed (and after the wait the chat
  says so, as before). After a confirmed Stop the process stays, unless a tool call was open as Stop landed: the
  app's tool server (`rbxkit_mcp.py`) handles one call at a time and may still be in a Blender or Studio call that
  the interrupt does not reach, so the process is closed with everything in its group and the next message starts
  one that resumes. Otherwise only tool processes in groups the turn added are ended; never the process's own
  group, which holds its tool servers. Stop, or closing the chat, while a message is still being sent (the old
  process is closed on a thread of its own, beside the new one) cancels the turn before the message is written.
- **When it fails**: could not start, or died before the turn showed anything: the same message goes to a per-turn
  process at once. Showed no sign of having read the message within 90 s (`START_WAIT`, `watch_start`): ended, the
  message sent per turn, and the chat stays on a process per turn. Died mid-turn: said as before ("exited
  unexpectedly", or its error output), not sent twice; the next message starts a process with `--resume`. Died
  between turns: replaced without a word. A turn whose `result` is a failure (signed out, a conversation that
  cannot be resumed): the process is closed, its stdin first and its output read to the end, so the reason it gives
  on its error output is the turn's ("No conversation found" reads as it does per turn). Two failures in a chat
  keep that chat on a process per turn for the rest of the run. `FRIDAE_CLAUDE_PER_TURN=1` forces a process per
  turn everywhere. Windows always uses a process per turn (the interrupt was not measured there).
- **Closing** (stdin closed, 5 s to exit, then its tree ended): an expert's or helper's after every turn (they hold
  nothing between jobs); a chat's after 10 minutes idle (`claude_live::IDLE`), or when more than 3 idle ones are open
  (`KEEP`, longest idle first; `keep_lives` looks every 15 s); when the chat is closed or switched to another AI; all
  of them when the app quits. A kept process is in the process ledger like any other.
- **Tests**: `src/live_tests.rs`. Six recordings of a kept process (`tests/fixtures/claude/live-*.ndjson`: a plain
  reply, two messages, a tool call, Stop while a shell command ran with a message after it, a process killed
  mid-turn, a background task and the turn Claude Code then started by itself) replayed through `consume_lines` and
  `after_turn`; the launch key for every setting a turn is started with (`ClaudeLaunch::new` takes them all, so a
  new one cannot be added without it); and whole turns against a stand-in process (a shell script that answers as
  Claude Code does on the wire): one process across messages, a restart with `--resume` for other arguments, Stop
  confirmed, ignored, before the start, in a tool call and while sending, every way of failing, helpers, idle
  closing, quitting.
- **Known and left**: the instructions list the other named or busy chats (`team_prompt`), so a studio run's Lead
  starts a new process whenever that list changes (safe, about a second each; moving the list into the message
  would change what every AI is told, so it was not done here). An ended run's Lead still in a turn keeps its
  process until the idle rule closes it.
- **Trying it by hand** with the scratch copy above: a scratch HOME hides Claude Code's sign-in (keychain), so give
  the Claude child the real one with `FRIDAE_CLAUDE_HOME=$HOME` (debug builds only, compiled out of release builds;
  the app's tool server gets the app's HOME back in its entry), add `USER=$USER` to the `env -i` line (without it
  Claude Code reports "Not logged in"), and link `<scratch>/home/.local/bin/claude` to the real program. Pick Haiku
  under the message box first: each turn uses the user's Claude plan. `work/processes/<app pid>.json` then shows one
  `claude` entry across several messages.

- **Starting processes**: nothing in the app calls `Command::spawn`, `output` or `status` itself. Every start (each
  AI, `ps`, `kill`, `git`, the kit's Python, OpenCode's server, the system's opener) goes through `paths::Start`
  (`start`, `run_output`, `run_status`), which holds one lock across the making of the pipes and the start, never
  while the child runs; `app-rs/clippy.toml` forbids the bare methods. Why: on a Mac a new pipe is not
  close-on-exec from the moment it exists, so a process forked by another thread in that moment keeps the pipe's
  end. Measured before this: a kept process being replaced, whose closing begins with a `ps`, was left open about
  once in a hundred replacements, because its replacement held `ps`'s pipe. After: the test that replaces Claude's
  kept process, alone on one thread, 3,200 runs with no failure.
- **A list of tool servers that is a symbolic link** (`<game>/.agents/mcp_config.json`, or `.agents` itself; Codex's
  `.codex/config.toml` the same) is neither written through nor replaced: the app makes no such link, and one that
  came with a game's folder may name any file. The list is then not written, and a Gemini chat that runs a process
  per message is told why its tools may be missing (`antigravity::LIST_IS_A_LINK`).

## Gemini: commands that are refused, and generated images

A Gemini chat is one `agy` process per message (`antigravity.rs`), in `--mode accept-edits` with the app's settings
for the session. Measured with agy 1.5.15, Gemini 3.8 Flash at low effort:

- **A refused command no longer ends the turn.** Headless agy cannot ask, so a step that needs approval is
  "auto-denied" and the turn ends there with nothing said (`explain` words it for the chat). Outside Full auto the
  settings now carry `"deny": ["command(*)"]`: the command's step ends in `ERROR` ("Permission denied for
  unsandboxed(...). Matches user-configured deny rule."), the model reads that and carries on, the result is
  `SUCCESS`, exit 0, nothing on stderr. Two recordings, both replayed by `replay_tests.rs`:
  `tests/fixtures/gemini/command-refused-softly-then-image.ndjson` (`echo hello-from-shell`) and
  `commands-refused-softly-and-own-server-list.ndjson` (`/bin/echo one`, `echo two | tr a-z A-Z`,
  `echo three && echo four`: a path, a pipe and two joined commands are covered by the one rule). The programs the
  user allowed are still not handed to agy (it runs commands through zsh, which reads the user's `.zshenv`). Full
  auto is unchanged (`--dangerously-skip-permissions`, no command rule). Gemini is told in its standing text
  (`text/gemini_turn.txt`, chosen by `agents::running_text`).
- **Its own list of tool servers is not its to write.** The deny rules name `<workspace>/.agents/mcp_config.json`
  (by the path the app has, and by the real path when that differs), in every mode, as the kept path's gate does:
  agy starts what that list names and the app keeps entries it did not make. Measured in the second recording: the
  write was refused by `write_file(<path>)`, the turn went on, and `note.txt` beside it was written. agy's rule
  compares case, though, and on a disk that ignores case `.agents/MCP_CONFIG.JSON` is the same file (not measured
  live). So the list is also read when the turn's process is started (`antigravity::list_now`, kept on the chat) and
  put back when the turn is over if it is another by then (`put_back`: removed if there was none, never through a
  link), with a line in the chat. `work/.agents` is one of `paths::app_protected()` too, should agy look for a list
  in the folders above its own (not measured).
- **Its own image tool works with no permission.** `generate_image` is run by a helper agy starts: in the stream a
  step of type `subagent` (`tool_name: invoke_subagent`, `subagent_info.subagents[].type_name: image-generator`), not
  a tool step. The helper runs on a model of agy's choosing (recorded: `gemini-4-hydrogen-tiered-high` under a
  flash-low chat) and calls `generate_image {AspectRatio, ImageName, Prompt}`. The picture (1024 x 1024 JPEG for
  `1:1`, 1376 x 768 for `16:9`) lands in the **parent** conversation's folder,
  `~/.gemini/antigravity-cli/brain/<conversation>/<name>_<ms>.jpg`. **The helper looks at its own picture and may
  make a second one unasked** (seen once: "the stone appears somewhat plump", then `_v2`), which costs the plan twice.
  No event names the file, so `antigravity::Generated` looks in that folder whenever a step is over and at the
  turn's end: an image file that was not there when the turn began is shown in the chat (`Out::Pictures`, copied to
  `work/chat-images` as every chat picture is). Only plain files of 20 MB at most (never a link). A file written in
  the last second and a half waits for the next look, so none is read half written. A picture that lands after the
  turn has ended is not shown.
- **Over ACP** (the kept process, one live turn, `tests/fixtures/gemini/acp-image.wire`): the helper is a
  `tool_call` named `invoke_subagent` (`rawInput: {Model: "inherit", Prompt, Role, TypeName: "image-generator"}`) and
  **agy sends no `session/request_permission` for it**, nor for the helper's own `generate_image`. So the gate was
  never what stopped images there; `acp::decide` now lets a `generate_image` request through should a version ask
  (only with a plain `ImageName` and no path, or one inside agy's folder for the conversation), and `acp::Turn`
  shows the picture the same way (`watch`, looked for at each `usage_update`, finished tool call and the turn's
  end). Not measured: whether a helper's other steps (a command, a file write) are asked about over ACP.
- **When the user switched image generation off** (`gemini_images`, below): a Gemini chat's standing text says so
  and not to use its own `generate_image` (`text/gemini_images_off.txt` in place of `gemini_images.txt`), and the
  kept path's gate refuses `generate_image` and the image helper in every mode (`Rules::images`). On a process per
  message nothing can refuse agy's own tool (no rule for it is known), so there the text is all there is; and a
  conversation that is already running keeps the text it began with.

### `rbx_image` (`rbximage.py`, every AI)

- **`generate`** runs one headless agy turn (`rbxgen.antigravity_image`, shared with `rbx_generate_texture`) that
  asks for the nearest shape of `1:1, 16:9, 9:16, 4:3, 3:4, 3:2, 2:3`, then crops about the middle and resizes to
  exactly `size` with Pillow. **`keep`** brings in an image that exists and costs nothing.
- **The wrapper turn is boxed in**, because its prompt is written by an agent:
  - It runs in an empty folder made for the call, `work/antigravity/turns/<random>`, removed afterwards.
    `work/antigravity` is one of `paths::app_protected()`, so no agent's file tool can plant a
    `.agents/mcp_config.json` (agy starts what one names), the remembered model or the count there.
  - **A call that is ended mid-way leaves nothing.** A cancelled job, or one whose turn is over, gets SIGTERM and
    0.3 s later SIGKILL. While a call holds something (`rbxgen.cleanly`), SIGTERM unwinds it like an error, so the
    `finally` clauses remove the wrapper's folder and give back the file name taken for the picture. Should a call
    be killed outright all the same, the next one sweeps (`rbxgen.sweep`): wrapper folders, and names taken that are
    still empty files (each noted in `work/antigravity/reserved`), once they are half an hour old.
  - `--settings {"permissions": {"deny": ["command(*)", "write_file(*)"]}}`. Measured live: a picture was made under
    both rules (the image tool needs neither).
  - `--model` only ever gets an id matching `^[A-Za-z0-9][A-Za-z0-9._-]*$` that contains `flash`, whether it came from
    `agy models` or from the remembered name (`work/antigravity/models.json`, a day; a file that does not read as
    expected is ignored). The cheapest is a "lite" one first, then the lowest level. When agy opens no conversation
    on that model, the turn is tried once more with none and "no model" is remembered for the day. One time limit
    (420 s) covers the question about models, the turn and that second try; the tool's `OWN_LIMIT` is 600 s.
  - **Only a picture this turn made is taken**: a plain image file directly in agy's folder for the conversation the
    turn opened (`brain/<id>`, the id letters, digits and `-`), written since the turn began, 40 MB at most. A path
    the answer names counts only if it is one of those. Nothing the model or agy wrote is passed on in an error:
    the reasons are the tool's own words (not signed in; quota reached, with only the reset time read out of agy's
    error; "did not produce an image").
- **What it may cost the user's plan.** `paths::GUARDED` has `gemini_images` (Setup, Images: "Agents may generate
  images with Gemini", on unless switched off), handed to the tool server from the app's memory as
  `FRIDAE_GEMINI_IMAGES`, as `FRIDAE_GEMINI_USE_API` is; off, `generate` says so and tells the agent to ask the user.
  `textures_from_antigravity: false` in config.json switches it off too. Starts are counted where the picture is
  made (`rbxgen.antigravity_image`), so a generated texture counts like an image: 12 an hour for one chat
  (`PER_HOUR`, `work/antigravity/spent/<chat>.json`) and 30 an hour for one game's whole team, its Lead and every
  expert together (`PER_HOUR_PROJECT`, `spent/project-<hash of the project folder>.json`). A try that made nothing
  counts too (it may have cost a turn). Past either, the tool refuses with a line telling the agent to ask the user;
  a texture then goes to the user as when there is no source, never to the paid API instead. The cheap model only
  covers the wrapper turn: the picture itself is made by agy's helper on agy's own choice of model, and the helper
  may make two, which the count cannot see.
- **Where it writes.** `<project>/<folder>` (default `art`), never outside the project, never a hidden folder (also
  not through a link inside the project: the real path's parts are judged too), never over an existing file or
  through a link at the file's name (the name is taken with `open(..., "xb")`, a number added when it is in use;
  the picture is written beside it and renamed over it). A name is 80 letters at most. The folder is made and the
  name taken **before** agy is run, and given back if no picture comes. Should the last write fail all the same,
  the picture is saved under `work/images/unsaved` and the answer says where. A chat with no project saves under
  `work/images`.
- **What `keep` reads.** PNG, JPEG or WebP only (Pillow is told the three formats), 60 MB and 50 megapixels at
  most, from: the project; this chat's own folder (`work/gemini|chatgpt|opencode/<chat>`, `work/cursor/chats/<chat>`);
  what the app's own tools write and what the user handed over (`work/images`, `icons`, `textures`, `shots`,
  `attachments`, `chat-images`); and a file at the top of one conversation's folder under
  `~/.gemini/antigravity-cli/brain` (never below it, so not agy's `.system_generated` logs). Judged by the real
  path. With no `size` the picture keeps its own, brought down to 4096 a side. A grey picture with more than 8 bits
  a pixel (a 16-bit PNG) is scaled to 8 bits first.
- **`transparent: true`** (both actions; `cut_out`, numpy and Pillow only). `generate` asks for the subject alone
  on one flat chroma colour the prompt does not seem to want in the subject (magenta, else green, else blue). The
  background's colour is the middle colour of a ring round the border; if under 97% of that ring is within a small
  distance of it, nothing is guessed: the picture is saved as it is and the answer says the background could not be
  removed. Otherwise: a pixel as close to that colour as the border's own noise is background, and so is a darker
  shade of the same hue on a saturated background (a soft shadow, the rim of a dark outline); the two or three
  pixels next to the background are the soft edge, where the subject's colour is carried outward from the pixels
  just further in, a pixel that lies between the two colours gets the alpha of how far along it lies and has the
  background's share taken out of its colour, and a pixel of any other colour (a thin outline) is kept. The cut-out
  is trimmed, scaled to fit inside `size` with a small margin (resized with alpha-weighted colour, never stretched)
  and saved as an RGBA PNG; the answer's `look_at` is a sheet of it over a dark and a light backdrop. A large source
  is brought down before it is cut, to about twice the size asked for (2048 at most, unless more was asked for): the
  cut holds several working copies of the whole picture. An image counts as already transparent only when most of
  the ring round its border is clear (one clear pixel is no transparent background); it is then only trimmed and
  placed. Soft subjects (smoke, glow, fur, glass) cut out less cleanly. Anything in the background's own colour is
  cleared, also inside the subject (a white highlight on white): that is the design, the tool's description says
  it, and when a noticeable share of the area inside the subject's outline was cleared (2% or more, worked out on a
  coarse copy of the mask: what the background cannot reach from the border) the answer says so in
  `cleared_inside` and how to go on. The tool never makes a second picture by itself. One real picture was cut out
  this way (a painted skipping stone on magenta: no visible pixel left that looked magenta).
- Failures are plain words, never a Python error (`rbximage.run` catches everything else). It takes its turn
  among jobs like the other file-writing tools.

## Gemini: one `agy acp` process kept open per chat (built, on request)

A Gemini chat can keep one process, `agy acp`, open across turns, as a Claude chat keeps Claude Code. It speaks the
Agent Client Protocol (ACP: JSON-RPC, one message per line, protocol version 1). `app-rs/src/acp.rs` is the client and
the app's answers to agy's permission requests; `GeminiLaunch` in `agents.rs` is what a process is started with. The
holder of the process (`live.rs`) and every rule for keeping one (`Agent::live_turn`, `read_live`, the `Kept` trait)
are the same code that keeps Claude Code's: an AI can be kept open when `caps.rs` says `live`. Checked against agy
1.5.14. The design is `docs/superpowers/specs/2026-11-22-live-gemini-cursor-design.md`; Cursor speaks the same
protocol through the same client (the section after this one).

**It is not the default.** By default every Gemini message still starts a process of its own (`antigravity.rs`),
because of one measured loss: over ACP agy cuts off every call to a tool server after three minutes ("MCP tool call
... timed out after 3m0s"), whatever limit the server's entry asks for (`timeoutSeconds`, and `timeout_seconds`,
`timeout_ms`, `timeoutMs`, `requestTimeout`, `timeout`, tried together). A process per message has 15 minutes. A
Lead waiting for its experts, a texture waiting for the user and a Blender build take longer than three. agy does
start the servers of `<workspace>/.agents/mcp_config.json` over ACP too (where the per-message limit is set), but
only once a prompt has come: in a session with servers from that list alone, the first message had no tools.
`FRIDAE_GEMINI_KEPT=1` turns the kept process on (`agents::KEPT_ON_REQUEST`; taking Gemini off that list makes it the
default); `FRIDAE_GEMINI_PER_TURN=1` turns it off whatever else is set; Windows is always per message.

- **What goes over the wire**: `initialize` (the app offers no file system and no terminal, so agy edits and runs
  things itself); `session/new`, or `session/resume` with the chat's conversation id; `session/set_mode default`;
  `session/set_config_option` for the model (agy's own id, effort and all, as the per-message `--model` takes it) and
  for `thought_level` when the message sets an effort; then `session/prompt` per message, whose answer is the end of
  the turn (`stopReason`, `usage`). `session/update` notifications in between are read by `acp::Turn` into the same
  `Out` values `antigravity::Turn` gives, so cards, usage and `after_turn` are untouched. The tool servers go in
  `session/new` (`mcpServers`); the app's entries are taken out of the workspace's list for the turn (agy would
  start them a second time) and the list is put back as it was afterwards, replaced whole both times.
- **What was measured** (Gemini 3.8 Flash at low effort, 18 real turns through this client in two rounds, plus
  handshakes that use no model turn):
  - A conversation made by a process per message is resumed over ACP and the other way round (one store,
    `~/.gemini/antigravity-cli/conversations`), with the mode `default` and agy's default model.
  - `session/set_config_option` takes the joined ids (`gemini-3.8-flash-low` sets model and level); a level a model
    does not offer is an error (`xhigh` for Flash).
  - In mode `default` agy asks (`session/request_permission`) before each tool server call, command and file write,
    and before reading a file **outside** its workspace. It does not ask before reading a file inside it, before
    `search_web`, or before `read_file` on a web address (which fails). Per message a read outside the workspace is
    refused too ("required the read_file permission"). Its own allow and deny rules are not applied over ACP.
  - A refusal ends the turn at once with `end_turn` and no text.
  - agy names a server's tools `mcp_<server>_<tool>`: `mcp_rbxkit_rbx_board`, `mcp_Roblox_Studio_get_studio_state`
    (a stand-in server under Studio's name).
  - A tool server's result is not in the stream (the finished `tool_call_update` has no `rawOutput` or `content`);
    a command's is.
  - **Commands run through zsh**, which reads the user's `.zshenv` first (a marker planted through `ZDOTDIR` was
    written; `.zprofile`, `.zshrc` and `.zlogin` were not), whatever `SHELL` the process is given. The command is
    then agy's direct child, in a process group of its own.
  - Cancel: answered `cancelled` (6 ms), the running command ended.
  - Signed out: `session/new` answers `-32000 Authentication required`. Closing stdin: exit in under half a second.
- **agy carries on by itself after a turn.** Measured (`acp-background.wire`): `sleep 61` went to the background
  after five seconds, agy ended the turn with the tool call still open, and when the command ended a minute later
  it asked the model again and wrote an answer, with nothing sent. So a turn that ends with a tool call open has its
  process closed at once with everything it started (`End::background`; the chat is told in one line), and the
  reading thread ends a process at the first answer, tool call or permission request that comes with no turn of
  the app's on it (`acp::stray`, `Live::strayed`), as for Claude Code.
- **The app is the gate** (`acp::decide`), in every mode, because agy's own rules do not apply here. What no file
  tool may write comes from `paths::never_written`, the one list OpenCode's rules are made from too (the app's own
  files, the app's OpenCode store, every hidden name in the home folder and its `Library`), plus the workspace's
  `.agents/mcp_config.json` (agy starts what it lists); `paths::ASK_ME_NEVER_UNDER` (`.git`) is shared the same way.
  - Every mode: a request that cannot be read is refused (no tool call, no id, arguments that are no object, a
    path argument that is not text); so is a tool of kind `switch_mode`; the app's tool servers' tools pass
    (`rbxkit`'s only when the tool is an `rbx_` one, so a server named `rbxkit_x` is not taken for it). After Stop
    every request is answered `cancelled`.
  - **Ask me**: no command runs by the app's say alone, also for an allowed program: the app's check
    (`paths::command_allowed`) is written for `/bin/sh` and agy's shell reads the user's start-up file. Each
    command gets a card: "allow this program" for one not allowed, else why and "Run it once", which lets that
    exact command through one time (`COMMANDS_BY_APP`; the cards and the one-time store are `opencode::decide`'s).
    A write passes inside the chat's workspace (the game, or the chat's own folder) or agy's scratch folder for
    that conversation (`antigravity-cli/brain/<id>`), never under a `.git` folder in any case of letters (a hook or
    an alias there would run with an allowed `git`), never to what no file tool may write, and never to a place
    that holds such a file (a delete or move of the folder above it). A read agy asks about passes inside the
    workspace, the kit, attachments and chat pictures, and never for the app's own settings, saved chats or the
    other protected files. A path that is relative or has `..` cannot be judged and is refused. Anything else is
    refused with a line naming it. The agent is told what was refused with the user's next message (agy takes no
    reason with a refusal).
  - **Full auto and Studio**: the mode stays `default` and everything passes except a file tool's write to what no
    file tool may write (compared in any case of letters, so `CONFIG.JSON` is refused: stricter than a process per
    message, where agy's rule compares case), a write that names no file, and an unknown tool that names a path the
    app cannot check. A relative path is taken from the workspace. `--dangerously-skip-permissions` is not used.
  - A session seen in another mode than `default` is ended. The answer picks agy's option by its `kind`
    (`allow_once`, `reject_once`), never an "always".
  - What the gate cannot see: reads inside the workspace and web searches (agy does not ask), and what a command
    the user let run does. Tool names in `WRITERS` and `READERS` are only those seen in a recording; other tools
    are told by ACP's `kind`.
- **Pictures**: a tool server's result does not reach the app, so `rbxkit_mcp.py` notes the image files of each
  result (`note_pictures`, one JSON line per result) in the file the app names in its entry
  (`FRIDAE_TOOL_PICTURES`, `work/tool-pictures/<chat>.jsonl`, one of the protected folders, emptied at each turn's
  start), and `acp::Turn` shows the new lines' pictures when an `rbx_` call completes, through `pictures_in` as per
  message. A missing or unreadable file shows nothing. A process per message is not handed the variable. Pictures
  the Studio server returns as image data are not recovered (per message they were only found by a path in the
  text, too).
- **The launch key** (`GeminiLaunch::key`): the program, the workspace, the tool servers with their environment,
  the model, the effort, the mode, the chat, every folder and protected place the gate decides by, agy's data
  folder, the pictures file, the provider-keys switch, the allowed programs, and a fingerprint of every guarded
  setting but the model picked for OpenCode, which no Gemini or Cursor process is started with (`paths::GUARDED`,
  `guarded_print`; a hash, since one is the Gemini key). Any change closes the process and the next one
  resumes the conversation. The gate's rules are built per message as well.
- **One process on a conversation.** agy keeps a conversation in a file of its own, so a turn (kept or per message)
  first waits until the chat's earlier kept processes are gone (`State::retiring`, `Kept::alone`, at most 12 s;
  Stop ends the wait). A `session/resume` that fails is tried once more a second later; only agy's own "Failed to
  read session" (measured for an unknown id) then starts a new conversation, and the chat is told; any other error
  gives the process up and the message goes per message with the conversation kept.
- **Standing instructions** still go on top of a conversation's first message, exactly as per message. On the kept
  path the text is `text/gemini_kept.txt` (commands are asked about; a command must finish inside the turn).
- **Stop**: before agy has shown work on the message (an answer piece, a tool call, usage above zero), the process
  is ended at once, tree and all, and the next message resumes: a cancel that early was not measured. After that,
  `session/cancel`, and `STOP_WAIT` for the prompt's answer; then the kill. A Stop that lands in a tool call gives
  the process up afterwards, as for Claude Code.
- **When it fails** is `read_live`, as for Claude: could not start, another protocol version, `set_mode` refused,
  or died before anything was shown: the same message goes to a process per message, once (checked live with a
  process killed at its start). A model or effort agy does not offer over ACP: per message too, not counted as a
  failure. Died mid-turn: said, not sent twice. Signed out: said once, no second process. `stopReason: refusal` is
  said as a usage limit (read in t3code, not measured), in words `usage_limit` knows. Two failures keep the chat
  on a process per message for this run.
- **Known and left**: the three-minute limit above stands, but no longer loses anything: the app's tool server
  answers inside it and keeps longer work going as a job ("Long tool calls" below). A command still running when its process is killed from
  outside is left to end by itself. A process that stops answering after it has a session falls to the 20-minute
  stall guard, not to the start watchdog (a model may think longer than the watchdog's 90 s before its first
  line, so silence after a prompt cannot be told from work). A real usage limit was not seen.
- **Tests**: `acp.rs` (every decision of the gate, the answer's option, stray lines, stop reasons, long waits,
  pictures), `tests/test_tool_pictures.py`, and `src/acp_tests.rs`: the launch key for every setting; fifteen
  recordings (`tests/fixtures/gemini/acp-*.wire`) played back by a stand-in process that waits for a line wherever
  the app sent one, compares it with the line the app sent when recorded (a request by its method, an answer to
  agy whole) and writes agy's lines in between; and turns against a stand-in that answers as agy does (one process
  across messages, a restart that resumes and waits for the old process, Stop early, confirmed and ignored, a
  request after Stop, a resume tried twice, every way of failing).
- **Recording**: `FRIDAE_LIVE_RECORD=<folder>` (debug builds only) writes everything a kept process is sent (`> `)
  and writes (`< `) to `<folder>/<ai>-<pid>.wire`. Fixtures are whole lines of such files cut at turn boundaries and
  scrubbed: the scratch folder becomes `/scratch` (the tests put their own folder there), session ids
  `00000000-…`, the plan's name, the home folder and user name. `acp-resumed-per-message.wire` is the research
  probe's recording. Recordings made before the app stopped approving commands by itself are played with the
  user's "Run it once" standing in for that approval.
- **Trying it by hand** with the scratch copy above: `FRIDAE_GEMINI_KEPT=1`; a scratch HOME hides agy's sign-in, so
  give agy the real one with `FRIDAE_AGY_HOME=$HOME`, keep its conversations out of yours with
  `FRIDAE_AGY_DIR=<scratch>/gemini` (`agy acp --gemini-dir`; a process per message cannot be given one and writes
  to `~/.gemini`), set `FRIDAE_AGY=$HOME/.local/bin/agy` and `{"chat_ai": "gemini"}`, and **pick the cheapest model
  and Low effort before the first message** (the menu starts on agy's first model). `FRIDAE_STUDIO_STANDIN=<a
  Python tool server>` puts a stand-in under Roblox Studio's server name; `FRIDAE_AGY_SHELL` sets the SHELL agy is
  told of (all debug builds only). `agy models` runs with the real HOME at start and writes its own housekeeping
  under `~/.gemini/antigravity-cli`. `work/processes/<app pid>.json` shows one `gemini` entry across messages.

## Cursor: one `cursor-agent acp` process kept open per chat (built, on request, Ask me only)

A Cursor chat can keep one process, `cursor-agent acp`, open across turns. It is the same client as Gemini's
(`acp.rs`, as `acp::Agent::Cursor`), the same holder (`live.rs`) and the same rules for keeping a process
(`Agent::live_turn`, `read_live`, the `Kept` trait); `CursorLaunch` in `agents.rs` is what a process is started with.
Checked against cursor-agent 2026.11.22 on the model Auto. How Cursor runs per message is the next section; this one
is what differs when it is kept.

**It is not the default, and it is for Ask me only.** `FRIDAE_CURSOR_KEPT=1` turns it on (`agents::KEPT_ON_REQUEST`);
`FRIDAE_CURSOR_PER_TURN=1` turns it off whatever else is set; Windows does not offer Cursor at all. An expert, a
helper or the researcher never runs on it, whatever the switch says (how experts and studio runs work is not changed,
and none was seen on a kept process). Three measurements decide what else runs on it (`CursorLaunch::fits`):

- **Cursor's sandbox is not applied over ACP**, so Full auto and Studio always run a process per message. Measured:
  started with `--sandbox enabled` (and with `sandbox.mode: enabled` in its settings file too), a command not on the
  allow list is asked about instead of run sandboxed, and when the request is answered "allow once" it runs with no
  sandbox: `touch /x` failed with the disk's own "Read-only file system" and `curl` reached the network. The same
  commands on a process per message with the same flag ran unasked inside the sandbox ("Operation not permitted",
  no network).
- **A tool server's call is cut off after 60 seconds** ("MCP error -32001: Request timed out"), whatever the server's
  entry asks for (`timeout`, `timeoutMs`, `timeout_ms`, `requestTimeout`, `timeoutSeconds`, `timeout_seconds`,
  `toolTimeout`, `tool_timeout`, together) and whatever `MCP_TOOL_TIMEOUT` or `MCP_TIMEOUT` say; Cursor sends a server
  no progress token, so the server cannot keep a call alive either. That is under the 15 minutes the app asks every
  AI for, and was why the kept process was put on request. **A process per message was measured with the same
  60-second limit** (a 90-second call, `MCP_TOOL_TIMEOUT=900000` set as `spawn_turn` sets it): a Lead's `rbx_team
  wait`, a texture waiting for the user or a long Blender call failed on Cursor on either path. The app's tool
  server now answers inside the limit on both paths and keeps longer work going as a job ("Long tool calls" below),
  so this no longer costs anything; the kept process stays on request for the first reason, the sandbox, and for
  how narrow that makes it.
- **Only Auto has a known id on both paths** (`cursor::kept_model`): `auto` per message is `default[]` here. A
  process per message lists 284 model ids (`gpt-5.3-codex-low-fast`), a session 49, each one fixed variant
  (`gpt-5.3-codex[reasoning=medium,fast=false]`); no other spelling is taken (`Invalid model value`), and the plain
  names do not match (`composer-2.5` per message is not `composer-2.5[fast=true]`). A chat on another model runs a
  process per message.

What else was measured, and what the build does with it:

- **A chat keeps the path it started on.** The two kinds of process keep conversations in separate stores
  (`<settings folder>/chats/<hash>/<id>` per message, `<settings folder>/acp-sessions/<id>/` kept) and neither opens
  the other's (`session/load` of a per-message chat: `Session "<id>" not found`). So a chat whose conversation a
  process per message made never starts a kept one; a new chat (or one whose AI was just switched to Cursor) starts
  kept when the message fits. `cursor::chat_saved` looks in both stores; a kept session is saved once it has had a
  message (`store.db`: a session made and never prompted has only `meta.json` and cannot be loaded).
- **The hand-over.** When a message of a kept chat cannot run on a kept process (Full auto or Studio, another model,
  the switch, two failures, a kept process that failed before it showed anything), it goes to a process per message
  as a *new* Cursor conversation: the standing instructions, then the chat so far as text (`handover_text`, the same
  text a switch of AI hands over: the user's messages and the replies, without tool steps), then the message, once;
  the chat says so in one line (`CURSOR_HANDED_OVER`), the kept process is closed, and the chat is a per-message
  one from then on (`CursorLaunch::fall_back`). A process per message is never asked to `--resume` a kept session
  (it would start an empty one under that id without a word).
- **Cursor's own allow list and deny rules are the gate**, exactly as per message: the same `cli-config.json`,
  written by the same code (`cursor::prepare`) before the process starts and again before each turn. Measured over
  ACP: an allowed program and the app's tools run with no request; a write a deny rule covers is refused by Cursor
  itself ("Blocked by permissions configuration"; the stream shows the call as plain `completed`); a program not on
  the list, and a file-tool write outside the workspace folders, arrive as `session/request_permission` (`Not in
  allowlist: <program>`; the file in a `diff`; options `allow-once`, `allow-always`, `reject-once`). **The app
  answers every request no** (`Turn::cursor_request`), never yes and never "always": a program becomes the blocked
  card at the turn's end, as per message; in Full auto's rules (not run this way today) it would get no card. The
  card names the programs Cursor says are missing (`Not in allowlist: curl, git --version` lists each part of the
  command its list does not cover), not the command's first word. **A compound command is asked about for every
  part that is not allowed**, also with an allowed program in it: recorded with `git` allowed, `git --version &&
  ./git`, `git --version; curl ...`, `git --version | cat`, `git $(echo --version)` and `./git` were each asked
  about (`./git`, `curl`, `cat`, `echo`, `./git`), and a planted `./git` never ran. A
  refusal does not end Cursor's turn: the model is told "Rejected" and carries on. After Stop every request is
  answered `cancelled`. `cursor/ask_question` and the like are answered "not offered", so nothing waits on them
  (not seen in a turn).
- **The shell**: `/bin/zsh -c`, each command in a process group of its own as a direct child. On its first command a
  process runs a login, interactive zsh once to copy the user's shell state, which reads `.zshenv`, `.zprofile`,
  `.zshrc` and `.zlogin`; then `.zshenv` again for every command. The same on a process per message. The app
  approves no command by itself on Cursor, so its own command check (`paths::command_allowed`, written for
  `/bin/sh`) is not involved.
- **Tool servers** go in `session/new` (`mcpServers`, with the app's settings in each one's environment as the
  workspace's file has them, `cursor::kept_servers`). Over ACP Cursor does not read the workspace's
  `.cursor/mcp.json` at all (its servers were started neither at `session/new` nor after a prompt, also one of the
  same name), so nothing starts twice; the file is still written, for the fallback. The servers run in the agent's
  own process group. `CURSOR_CONFIG_DIR` is honoured; `CURSOR_DATA_DIR` stays empty. No `worker-server` is started
  (a process per message leaves one, which the app ends).
- **Pictures**: a tool server's result is not in the stream (`rawOutput: {"success": true}`; the model did see the
  picture), so the `tool-pictures` side file is used as for Gemini (`FRIDAE_TOOL_PICTURES` in the app's server's
  entry on the kept path only). Pictures the Studio server returns as image data are lost on the kept path (per
  message they are shown).
- **Usage**: none is reported over ACP (the prompt's answer is `{stopReason}` alone). Each kept turn is logged as
  `{"unreported": true}` with no counts (`usage::Turn::not_reported`), and the usage panel says "N turns not
  reported by Cursor over this connection" beside the counted ones, never zero.
- **What goes over the wire**: `initialize`; `session/new`, or `session/load` with the chat's conversation id (it
  plays the history back before it answers, which is passed over; there is no `session/resume` and no mode to
  set); `session/set_config_option model default[]`; `session/prompt` per message. A tool server's call comes as
  `tool_call` (`title: "MCP: tool"`, `kind: other`) and is named by the update after it (`rawInput:
  {providerIdentifier, toolName, args}`); the chat shows each step once, by the names a process per message gives.
  Cursor thinks before each of its messages, so a thought after text ends that message (two answers with a step
  between that the client is not told of are shown as two: recorded). Signed out: `session/new` answers `-32000
  Authentication required`. A session Cursor does not have: a new one is made on the same process and given the
  message as a conversation's first (the standing instructions, the chat so far as text, the message), and the
  chat is told in one line.
- **Ending it**: `cursor-agent acp` does not exit when its stdin closes, so the holder ends it with a signal
  (`Live::ends_by_signal`: SIGTERM to each group of its tree, SIGKILL to what is left after the grace) instead of
  waiting `CLOSE_WAIT`. Stop is `session/cancel` (measured: the turn over 0.05 s after Stop through the app, the
  running `sleep` gone); before Cursor has shown work on the message the process is ended instead, as for Gemini.
- **A command left running**: after a turn, a process group in the kept process's tree that was not there when it
  took the message means something the turn started outlived it (`TurnProc::left_running`: a command, each of which
  Cursor runs in a group of its own; a group a tool server made would count too); the process is closed with all it
  started and the chat is told, as when a turn ends with a tool call open. Cursor putting a command in the
  background was not provoked (an allowed `sleep 47` ran in the foreground to its end).
- **The launch key** (`CursorLaunch::key`): the program, the chat's folder, the kit, the game, the tool servers as
  the app keeps them and as the session is handed them, the model, the mode, the chat, the pictures file, the
  provider-keys switch, Cursor's rules as its settings file is written with them (so the allowed programs and the
  user's own servers), its settings, data and home folders, and a fingerprint of every guarded setting. The
  message and the standing instructions are not in it. A turn waits for the chat's earlier kept process to be gone
  (`Kept::alone`: two processes on one session were not measured).
- **Known and left**: the 60-second limit stands on both paths, but the app's tool server now answers inside it and
  keeps longer work going as a job ("Long tool calls" below). A command still running when the process is killed from
  outside lives on until it ends (seen: `sleep 50` after a SIGKILL). The user's own `~/.cursor/mcp.json` servers
  over ACP were not measured (this computer has none): their tools are denied by name as per message, but whether
  `mcp-disabled.json` keeps them from starting here is not known. Not measured either: a usage limit, a cancel
  before Cursor's first output, `cursor/*` requests in a real turn.
- **Tests**: `acp.rs` (Cursor's tool calls, requests, endings, from real lines), `usage.rs`, and
  `src/cursor_live_tests.rs`: the launch key for every setting, which messages fit, turns against a stand-in (one
  process across messages, the hand-over, a chat that stays per message, load, Stop early, confirmed and ignored,
  every way of failing, a request after Stop, a command left running, closing by signal, helpers kept off), and
  eight recordings (`tests/fixtures/cursor/acp-*.wire`: a tool, an allowed program, a refused one with its card, a
  refused write and a second message on the same process; Stop mid-command; a process killed mid-turn; a session
  Cursor does not have; two messages with no tool between; signed out; the five compound commands with nothing
  allowed, and with `git` allowed), made through the app and scrubbed as Gemini's.
- **Trying it by hand** with the scratch copy above: `FRIDAE_CURSOR_KEPT=1`, `FRIDAE_CURSOR_HOME=$HOME` (a scratch
  HOME hides the sign-in; without it you get the signed-out line), `FRIDAE_CURSOR=$HOME/.local/bin/cursor-agent`,
  `{"chat_ai": "cursor"}`. Cursor's settings and sessions are then under the scratch home's `Library/Application
  Support/Fridae Studios/cursor`. `FRIDAE_LIVE_RECORD=<folder>` records each kept process.
  `work/processes/<app pid>.json` shows one `cursor` entry across messages.

## Long tool calls: an AI that cuts tool calls off

Two AIs cut every call to a tool server off after a fixed time, whatever the server's entry or `MCP_TOOL_TIMEOUT` asks
for (measured, the two sections above): **Cursor after 60 seconds**, on a process per message and on its kept process
alike, and **Gemini's kept process after three minutes**. Fridae's own tools often run longer (a Lead's `rbx_team
wait`, a texture waiting for the user, Blender, some Studio calls). So for those AIs the app's tool server answers
every call in time and keeps longer work going as a job. The design is
`docs/superpowers/specs/2026-11-22-long-tool-calls-design.md`.

- **The number** is in the capability table (`caps.rs`): `tool_call_limit` (a process per message) and
  `tool_call_limit_kept`, seconds; Cursor 60 and 60, Gemini none and 180, none for the others. `caps::call_budget`
  takes a margin off (a sixth, at least 15 s: 45 and 150) and `jobs::set` puts it in the app's tool server's entry as
  `RBXKIT_CALL_BUDGET`, with `FRIDAE_TOOL_JOBS` (below), where that AI and path has a limit: for Cursor in the
  workspace's `.cursor/mcp.json` and in the servers a session is handed (`CursorLaunch::new`), for Gemini in the
  servers a session is handed only (`GeminiLaunch::new`; `per_turn` takes it out again). Claude's, OpenCode's and
  Gemini's per-message entries have neither, and **with no budget the tool server behaves exactly as before**: the
  same tools listed, calls answered one at a time on the one thread, the same results (the Python tests from before
  this pass unchanged).
- **With a budget** (`rbxkit_mcp.py`, "long calls"): each `tools/call` is answered on a thread of its own, timed from
  when it was read. A tool runs as a **job**: a child process of the server (the same program, `RBXKIT_JOB_CHILD=1`,
  without the budget and the notes files in its environment) that runs that one tool exactly as the server would
  have and writes the result back whole, with the picture files it would have noted. If the job is done inside the
  budget, the call returns its result, byte for byte what the tool returns without a budget. If not, the call
  returns `NOT FINISHED YET. This is not the tool's result: <tool> is still working as job "job-…" (N seconds so far)
  and nothing has failed.` and how to pick it up; `isError` is false. The work carries on in the server.
- **`rbx_job`** is listed only when there is a budget. `wait` (job) returns the finished job's result exactly as the
  tool returned it, text and pictures, once (the pictures are noted in `tool-pictures` at that moment, so a kept
  chat shows them with the call that handed them over), or the same `NOT FINISHED YET` answer when the budget runs
  out again. Without a job name it takes the chat's only job. `list` shows the jobs; `cancel` ends one with all it
  started. A job that failed gives its error result once, as the tool gave it.
- **Not jobs**: `rbx_guide`, `rbx_studio_snippet`, `rbx_studio_lock` and `rbx_board` (instant; answered by the
  server itself, one at a time). `rbx_team` (its `wait` is already a wait: its timeout is capped two seconds inside
  the budget, and its own answer says who is still working and to call wait again). `rbx_promo`'s `render` and
  `job` (renders are the server's own threads and already return a job id: their waits are capped the same way,
  `_promo_wait`); its other actions are jobs. **A texture waiting for the user is a job**, not a wait that returns:
  the design says such waits "need no job", but `rbxgen.ask_user` withdraws the user's card the moment it returns,
  so the wait has to carry on somewhere, and the job is where.
- **One at a time**: jobs run one after another in the order the calls were read, as every call did on the one
  thread (calls an AI sends together keep their order: a call takes its place in line before the next one is
  looked at). A call behind a running job waits out the budget, then answers `NOT FINISHED YET … has not started.
  It is queued behind job-… (rbx_blender), because the rbx tools that make or change files run one at a time`. The
  reason is shared files: two texture calls write one folder's `_check.png`, Blender and a package or an icon read
  and write one model's files, a checkpoint commits the project while another tool writes into it. Only the tools
  in `BESIDE` run beside others, each with its reason there: `rbx_market`, `rbx_research` (they read the web and
  write no file), `rbx_economy` (a calculation), `rbx_doctor` (looks for programs). `rbx_vault` is not among them:
  it keeps an index cache in the work folder.
- **A job's longest life** (`_job_life`): the tool's own time limit where it has one, plus 20 seconds for the tool
  to say so itself (`rbx_blender`: its `timeout`, 600 by default; the `run_rbx` tools 600; a generated texture
  720; `rbx_doctor` 120), otherwise 15 minutes, and never more than 15 minutes whatever a model asks for
  (`JOB_LIFE`: what a process per message gives one call). It runs from when the job's process starts, not from
  when it was queued. Past it the server ends the job with all it started, and `rbx_job wait` returns that once as
  an error ("was ended: it ran for N seconds … Do not wait for it again"). A wait on a job that is gone always
  says so ("No job … do not wait for it again"), never `NOT FINISHED YET`. **The app bounds the AI as well**
  (`paths::JOB_POLL_LIMIT`, `watch_stall`): a turn whose tool calls have all been `rbx_job` for 30 minutes (twice
  a job's longest life, so a job queued behind another is covered) is ended and the chat told why. Each wait is
  output, so the stall guard alone would never see such a turn.
- **Nothing outlives the job, the turn or the server.** A job child leads a process group of its own. **When a job
  ends, for any reason, what is left of its group is ended**: when it returns by itself (its output has ended; if
  `ps` shows another member of the group, SIGTERM, 0.3 s, SIGKILL), at its longest life, when cancelled or dropped
  (SIGTERM, 0.3 s, SIGKILL). A program a tool started and left behind, such as Blender after `run_rbx` gave up on
  it, goes then, and a child that ignores SIGTERM too. A group is only ever signalled while its leader is unreaped
  (the leader is reaped in one place, `_run_job`, under the job's own lock, which whoever signals also holds), so
  the group's number cannot have become another process's. The server ends every job when its input closes and
  when it gets SIGTERM, SIGHUP or SIGINT (handlers installed only with a budget). Should the server be killed
  outright, a small watcher (the same program, `RBXKIT_JOB_REAPER=1`, started with the first job, in the server's
  own process group, holding a pipe from it) kills the job groups when the pipe closes. The app's own Stop reaches
  the groups as well (`procs::tree_groups`).
- **The turn's end** (`app-rs/src/jobs.rs`). The server notes each job in `work/tool-jobs/<chat>.jsonl`
  (`FRIDAE_TOOL_JOBS`; the folder is one of `paths::app_protected`, like `tool-pictures`): `{"job", "tool", "at":
  "started"}`, and `{"job", "at": "collected"}` when its result was given back or it was cancelled. After every turn
  (`Agent::drop_jobs`, from `after_turn`, and from a kept turn before it looks for commands left running) the app
  reads it (its first 256 KB): jobs started and not collected are work thrown away, and the chat is told in one
  line ("The turn ended before the result of rbx_blender was collected, so that work was ended with the turn and
  thrown away."; only a name like `rbx_…` is ever printed from the file, anything else reads "a tool"). **An
  expert's Lead is told too**: the line is added to the result the expert hands back (`team::with_lost_work`). On a
  process per message the server has gone with the turn. A kept process's server lives on, so the app appends
  `{"turn": "ended", "at": <now>}`; the server (which reads the file five times a second once it has had a job)
  ends every job it noted up to that time and answers `{"dropped": [..], "at": <the same>}`, which the app waits
  up to three seconds for, then empties the file. A server that did not answer in time is left the line alone to
  act on when it reads it; because the line carries its time, a job of a later turn is never ended by it.
- **Chats are apart** by construction: jobs are in the memory of one tool server, and each chat has its own. A job
  name (`job-` and twelve random hex digits) means nothing to another chat's server. The notes file names jobs and
  tools only, never arguments or results.
- **What agents are told**: `text/long_calls.txt`, added to the standing instructions of every agent (a chat, a
  Lead, an expert) on an AI and path with a budget (`agents::long_calls_told`: Cursor always, Gemini where its kept
  process may run): a `NOT FINISHED YET` answer means call `rbx_job` `wait` again, not report back or end the turn.
  Claude's and OpenCode's instructions are unchanged.
- **Measured with real turns** (2026-11-22; cursor-agent 2026.11.22 on Auto, agy on Gemini 3.8 Flash at low effort;
  the stand-in tool `rbx_test_slow`, which waits and returns a line with a word in it and one small picture):
  - Cursor, a process per message, 240 seconds: the call answered `NOT FINISHED YET` at 44 s, four `rbx_job wait`
    calls did the same (92, 138, 185, 231 s), the fifth returned the result, and the model said "Result word:
    **MARBLE**"; the picture was shown. No call was cut off. Recorded: `tests/fixtures/cursor/long-job.ndjson`.
  - Gemini, kept process, 240 seconds: `NOT FINISHED YET` at 150 s, one `rbx_job wait`, the result at 240 s, "The
    tool returned the result word **MARBLE**", the picture shown from the `tool-pictures` notes. Recorded:
    `tests/fixtures/gemini/acp-long-job.wire`.
  - The wording was understood the first time by both; neither reported the "not finished yet" answer as a result
    or as a failure (Cursor's thinking, in the recording: "138 seconds, still need to wait more"). Should a model
    ever do so, change the wording in `_still_going`, and do not add retries in the app.
  - A turn ended with a job running: on Cursor by Stop (the turn over at once, the line in the chat, no process
    left), on Gemini by the model ending its turn (told to; the line 0.5 s later, the job's process gone, the kept
    process and its tool server still there).
  - `rbx_team wait` asked for 100 s on Cursor returned after 42 s, and asked for 170 s on Gemini's kept process
    after 147 s, each saying who was still not finished. Both were cut off before.
  - These turns ran before the review's fixes (one job at a time, a job's longest life, what a job leaves behind,
    the app's bound on waiting); the answer a model reads for work still going is unchanged, and no real turn
    was run after them.
- **Trying it by hand**: `FRIDAE_TEST_TOOLS=1` (debug builds only) gives the app's tool server `RBXKIT_TEST_TOOLS=1`,
  under which it also offers `rbx_test_slow` (`seconds`, `word`); it is never listed otherwise. To record a process
  per message, stand a script where `FRIDAE_CURSOR` points that passes its arguments to `cursor-agent` and copies
  its output with `tee`.
- **Before installing a build**: `python tests/long_calls_proof.py "<build>/Contents/Resources/tools/fridae-cli"
  --mcp` runs six calls against the packaged tool server (the tool list, a real tool as a job child and how long
  that took, a call past the budget, two waits, a job left running, nothing left once its input closes) and exits
  non-zero on any difference; `RBXKIT_TEST_SERVER="<the same command>" python -m unittest tests.test_long_calls`
  runs the whole file against it. In a build a job child is `fridae-cli --mcp` with `RBXKIT_JOB_CHILD=1`
  (`rbxpaths.mcp_command`); **no build has been run this way yet**.
- **Known and left**: A call the AI has cancelled (`notifications/cancelled`) answers nothing and takes no result,
  but its job runs on until collected, its life or the turn's end. A turn's end with a kept process whose tool
  server has died waits the three seconds out (the app cannot tell a dead server from a slow one; only when jobs
  were outstanding). Every job costs a process start (about 0.09 s from source; `rbx_stage` and `rbx_vault` are
  called often and stay jobs: a stage's checkpoint may run git for longer than a budget, and the vault writes its
  index cache). `rbx_promo`'s render thread cannot be ended by Stop, as before. On Windows a job is ended with
  `taskkill /T` and there is no watcher (no AI with a budget runs there today). The page shows `rbx_job` as one
  more step ("Waiting for work that takes a while"), not as part of the call it waits for. A tool's result that
  names a picture in a folder with letters outside ASCII is not given its picture (the result writes them as
  escapes; with or without a budget, found while testing).
- **Tests**: `tests/test_long_calls.py`, 20 tests against real server processes over their pipes: no budget, nothing
  added; with one, in time, the same result (also for `rbx_economy`, a real tool, run as a job); pictures noted
  when given; two waits; once only; cancel with a child that ignores SIGTERM; a job that hangs ended at its life,
  said once, the next in line then running; each tool's longest life; a child left behind by a job that returned,
  gone when the result is given; two calls sent together run in order and the second names the first; which tools
  run beside others; **two real `rbx_texture` calls on one folder sent together** (needs numpy and Pillow: each
  gets its own sheet, the second queued behind the first); a call the client cancelled; the team wait; two chats'
  servers apart; the notes, the turn's end and a late line of an earlier turn; and the server closing, told to end
  and killed outright, each with such a child. `tests/long_calls_proof.py` (above). `jobs.rs` (the entry, the
  notes, the turn's end with and without an answer, names from the file, and **both sides together**: the app's
  `end_turn` against a real tool server with a job running); `caps.rs`; `acp_tests.rs` and `cursor_live_tests.rs`
  (the entry on each path, in the launch key); `agents.rs` (Claude's entry has none, who is told, the line in the
  chat and in a Lead's result, a turn that only waits past any job's life ended); the recordings above.

## Cursor as a chat's AI

A chat can run on Claude (Claude Code), Gemini (Antigravity's `agy`), Cursor (`cursor-agent`, the Cursor Agent CLI) or
OpenCode (`opencode serve`, see the next section);
ChatGPT (`codex`) is still in the code but not offered in the window. `paths::AIS` lists them and everything that
branches on the AI goes through it (`find_ai`, `switch_ai`, helpers and studio runs via `helpers_work`, research).
Cursor is `app-rs/src/cursor.rs`, checked against cursor-agent 2026.11.22; its header says how a turn runs.

- **Found** by `paths::find_cursor`: `FRIDAE_CURSOR`, then `cursor-agent` on PATH, then `~/.local/bin/cursor-agent`
  (its installer's place; `widen_path` adds that folder for an app opened from Finder). Not the `agent` alias.
- **A turn**: `cursor-agent -p --output-format stream-json --trust --approve-mcps --workspace <chat folder> --add-dir
  <kit> [--add-dir <game>] [--model <id>] [--resume <chat id>] --sandbox disabled|enabled`, the message on stdin (no
  length limit from a command line). Standing instructions go on top of a session's first message, as for Gemini
  (`with_instructions`), but with Cursor's own text (`text/cursor.txt`, filled by `cursor::instructions`): its working
  folder is not the game's, it may run the programs the user allowed (listed), a refused one becomes the blocked card
  and it carries on, Full auto's sandbox, web allowed, never its own settings. The effort setting is not passed and is
  hidden in the menu: Cursor's model ids carry their own effort. Models come from `cursor-agent models` ("id - Name"
  lines, 284 on a paid plan), default `auto`; an empty answer (signed out, offline) is asked again at most once a
  minute from the page's status call; a chosen model the list lacks falls back to the default (`pick_model`). The
  menu shows the list in its own scroll box, with a filter above 12 models, so the run modes stay in view.
- **Resuming**: asked to `--resume` a chat it no longer has, Cursor quietly starts an empty one under that id, so
  before each turn the app checks that Cursor's saved chat is there (`cursor::chat_saved`); if not, the chat starts
  a new conversation handed the chat so far (`stale_cursor_chat`).
- **Events** (`cursor::Turn`): `system/init` (session id, model), `assistant` text, `tool_call` started/completed
  (`mcpToolCall` with `serverIdentifier`/`toolName`, shown as `mcp__<server>__<tool>` so Studio steps are recognised,
  and passed on with its arguments so a Lead's `rbx_team wait` is not counted as progress; `getMcpToolsToolCall`
  and its bookkeeping steps (to-dos, plans, lint reads) are not shown; shell, edit, read, delete, glob, grep, ls, web
  search have Claude Code's names). A step's id is `call_id`, or the `toolCallId` inside it. Pictures come back in
  `result.success.content[].image.data` (base64), as Claude Code's do. A refused shell command (`result.rejected`)
  becomes the same `blocked` card Claude Code's `permission_denials` give. `result.usage` (`inputTokens`,
  `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`) is logged under `ai: "cursor"`; `outputTokens` is taken to
  include `reasoningTokens` (not checked against Cursor's dashboard). Cursor reports no cost and no plan limits. A
  failed `result`, or an `error` event mid-turn, counts as a failed turn (`Out::Failed`).
- **Where its files go (the MCP route)**. Nothing in the user's Cursor setup and nothing in the game's folder:
  - each chat works in `work/cursor/chats/<chat id>/`, whose `.cursor/mcp.json` (the whole file is the app's) holds
    the agent's servers, and which has an empty `.git` folder: Cursor takes the nearest folder with `.git` as the
    project's top, and a work folder inside a git repository (running from source) would otherwise point it at the
    repository's `.cursor/mcp.json`. The game is added with `--add-dir`, so its own `.cursor/mcp.json` is not loaded.
  - `CURSOR_CONFIG_DIR` (its `cli-config.json` with the app's permissions, and its saved chats) and `CURSOR_DATA_DIR`
    (approvals and trust per workspace) are `config` and `data` under `~/Library/Application Support/Fridae
    Studios/cursor` on a Mac (`$XDG_DATA_HOME` or `~/.local/share` elsewhere): not in Documents, which iCloud may sync,
    since the saved chats and Cursor's account cache (email and name, no tokens) are there. `cli-config.json` is
    rebuilt before every turn, keeping only what Cursor itself stores (model, account and privacy caches:
    `cursor::KEPT`), so nothing a turn wrote there lasts. The user's `~/.cursor/cli-config.json` is neither read nor
    written. The sign-in lives in the keychain, found through HOME, so it still works.
  - Cursor gives a tool server only HOME, PATH and the entry's own `env`, so the app's settings a server reads
    (`RBXKIT_WORK`, `RBXKIT_BLENDER`, `RBXKIT_STUDIO_*`, `FRIDAE_AGY`, Windows' `LOCALAPPDATA`/`APPDATA`/`USERPROFILE`,
    `TMPDIR`; never a key or token) are copied into each entry (`cursor::PASSED`). So a Gemini API key set only in the
    environment does not reach rbxkit in a Cursor chat: it has to be saved in Setup (Setup says so).
  - Cursor starts a helper of its own (`worker-server`, with a TypeScript language server) that outlives the turn,
    in the turn's process group. Once the turn's output ends, and before its process is reaped (so the group's id
    cannot have been reused), `read` sends that group SIGTERM, and SIGKILL from a background thread after 2 s if
    anything is left (`paths::end_group_soon`); the turn's end is not held up. Checked: nothing left after a turn.
  - Cursor still keeps its own housekeeping under the real `~/.cursor` (skills it syncs, `ai-tracking`), and it runs
    the user's hooks from `~/.cursor/hooks.json` (Roblox Studio installs some there, as it does for Claude Code in
    `~/.claude/settings.json`); `FRIDAE_NO_STUDIO` does not stop hooks, for either AI.
- **Permissions** (`cursor::permissions`, written before every turn), allow-list mode: allowed: `Mcp(<server>:*)` for
  our servers, `Shell(<program>)` for each program the user allowed (Setup, or the blocked card), and web pages
  (`WebFetch(*)`, `autoAcceptWebSearch`) as Claude Code's WebFetch and WebSearch are (not tried in a real turn).
  Denied, in every mode: every server named in the user's `~/.cursor/mcp.json` that is not ours (read as Cursor reads
  it, comments and trailing commas allowed; ours wins a name clash); with `FRIDAE_NO_STUDIO`, `Mcp(*studio*:*)`
  (Cursor matches server names without regard to case: measured); `Write(**/.cursor/**)`, the app's Cursor folders,
  the app's `config.json` (it holds the allowed programs), and the home folder's hidden files and folders and
  `Library`. Glob characters in those paths become `?`. Measured with real turns (cursor-agent 2026.11.22):
  - Ask me: a shell command not allowed is refused (blocked card); a tool from any server not ours is refused (it
    needs approval, which nobody gives), so servers from Cursor plugins, which are not in `mcp.json`, are refused too
    (no plugin was installed to try, but any non-allowed server's tool was); a write outside the workspace folders
    (chat folder, kit, game) is refused, e.g. `~/fridae-probe-outside/x.txt`; a write into a temp folder works.
  - Full auto: not `--force` (which lets everything run unconfined and lets other servers' tools through) but the same
    allow list with Cursor's sandbox on (`--sandbox enabled`). Checked in a real turn: our tools, an allowed program
    (`git --version`) and a write in the game folder work; another program runs, sandboxed (`ls /` worked;
    `curl` got no network; `mkdir` in the home folder: "Operation not permitted"); a write outside the workspace
    folders is still refused. Tool servers are not in the sandbox: a test server's tool wrote a file in a folder
    under the home folder outside the workspace, in Full auto with the sandbox on (so `rbx_texture`'s writes to
    `work/vfx` work there; a Studio tool was not tried, since nothing here talks to Roblox Studio).
  - The user's own tool servers (`~/.cursor/mcp.json`) are not even started: `--approve-mcps` approves every server
    Cursor finds, so the app lists theirs in the chat project's `mcp-disabled.json` (`cursor::write_disabled`, under
    `CURSOR_DATA_DIR/projects/<slug of the chat folder>`), which Cursor reads before starting any. Measured: a server
    named there was not started, the other was. Servers from Cursor plugins are not listed (their names are not known
    here): they may start, and their tools are refused by the allow list.
- **Safety gaps against Claude Code** (what Cursor can do that Claude Code cannot, in the same mode):
  - Ask me: Cursor's file tools may write into temp folders (`/tmp`, `/private/var/folders`), which Claude Code's
    `acceptEdits` would refuse. Writes elsewhere outside the workspace folders are refused, as for Claude Code.
  - Full auto and Studio: none known. Claude Code runs those with every check off; Cursor keeps its allow list, deny
    rules and sandbox, so it can do less. The sandbox is not a hard boundary, though: programs the user allowed run
    outside it (Cursor's `allowlistEscalated`), and a sandboxed command may write inside the workspace folders,
    which include the game's `.git` folder and, when running from source, the kit and its `config.json` (the deny
    rules bind Cursor's file tools, not the shell). The app's own git checkpoints run with hooks and fsmonitor off
    (`projects::GIT_SAFE`, and the same in `rbxproject.py`), so a written hook is never run by the app; a written
    `config.json` (from source only) still takes effect. Claude Code's bypass mode allows all of this too. `cursor::gaps()` holds the one line the page would show under the message
    box when Cursor is picked for Full auto or Studio; it is empty, so nothing shows. A gap found later goes there.
  - A game's own `.cursor/rules` or `AGENTS.md` may be read by Cursor (added folder); a game's `.cursor/cli.json`
    permissions are not loaded (the game is not the project's top), but this was not tested.
  - The rules on publishing, spending and deleting are the app's prompts, the same text Claude gets; nothing in Cursor
    enforces them, as nothing in Claude Code does.
- **Not on Windows yet**: `find_cursor` finds nothing and `paths::AIS` leaves Cursor out there, so it is neither
  offered nor counted as installed (the page filters its AI list by `status.ai.offered`). Why: nothing was run there.
  To turn it on, test on Windows: the permission patterns with `\` paths (do `Write(C:\...)` deny rules and the
  workspace-folder check hold?), ending the worker-server after a turn (`taskkill /T` cannot reach a child whose
  parent has exited, so it likely needs a Job Object), passing the message to `cursor-agent.cmd` (escaping), and
  whether `--sandbox enabled` exists there at all (if not, Full auto has to stay Ask me's rules).
- **Trying it by hand** with the scratch copy above: a scratch HOME hides Cursor's sign-in (keychain), so give the
  Cursor child the real one with `FRIDAE_CURSOR_HOME=$HOME` (debug builds only, compiled out of release builds; only
  Cursor's process gets it; the tool servers get the app's HOME back in their entry) and
  `FRIDAE_CURSOR=$HOME/.local/bin/cursor-agent`; put `{"chat_ai": "cursor"}` in the kit's `config.json`. Each turn
  uses the user's Cursor plan.

## Signing in to Claude Code, Cursor and Gemini

`app-rs/src/signin.rs`; its header has the commands and what was measured. These three run on the user's own sign-in,
the one their tools have outside Fridae too, so the app only asks each tool who is signed in and starts the tool's own
sign-in. **Nothing in the app signs anyone out**: no tool is ever started with `logout`, there is no button for it, and
a test reads every source file of the app, the page and the Python tools to keep it so (the word may stand in
comments and tests only). (OpenCode is different: Fridae's own copy, see the next section.)

- **Who is signed in** (`signin::check`, `Known`, `Answer`): `claude auth status --json` (`loggedIn`, `email`,
  `authMethod`; exit code 1 when signed out, the answer is read all the same), `cursor-agent status --format json`
  (`isAuthenticated`, `userInfo.email`), and for Gemini `agy models`, which is the model list anyway
  (`antigravity::ask_models`): signed out it ends with code 1 and "Error: Please sign in to view available models."
  agy has no command that names the account. An answer the app cannot read is "not known", never "signed out", so no
  Sign in button shows on a guess. A key or a gateway reads as signed in too (`authMethod` api_key, oauth_token,
  api_key_helper, third_party; the reviewer's measurement) and is told apart (`by_key`). Asked at start, after a
  sign-in or a cancelled one, when a turn fails as signed out (`turn_failed`), on every press of Sign in, and from the
  page's status only when the last answer is old (`due`: half an hour after "signed in", five minutes otherwise). Each
  runs with the environment a turn of that AI gets (`tool_env`: `paths::child_env`, and for Cursor the app's own
  config and data folders, `cursor::environment`; measured: Cursor answers "authenticated" with those too), through
  `paths::Start`, in a group of its own and in the ledger, and is ended after 30 seconds (`run_limited`).
- **Sign in** (`login_start`, `/api/signin` with `ai`, `action` login, cancel or check, and `sure`): `claude auth
  login`, `cursor-agent login`, and for Gemini `agy acp` with `initialize` and then `authenticate` for the method
  `oauth-personal` ("Log in with Google", from the `authMethods` agy lists). Each tool opens the browser itself and
  waits for it to come back (Claude Code and agy give the page a `localhost` address to return to; Cursor's link carries
  a challenge and an id instead); nothing is typed. Claude Code also prints a link and "Paste code here if prompted"
  for a browser on another computer: the app has no place to paste one and shows no link (a link's query is a secret
  of that sign-in), so that way is not offered; the waiting line says to cancel and try again if no page opened.
  When the tool ends, whatever it said, the app asks who is signed in and goes by that. A sign-in nobody finishes is
  ended after ten minutes, and when the app quits. A press while one waits ends it and starts afresh (two presses
  within three seconds are one).
- **Never over a sign-in that is there without a second press.** These sign-ins are the user's own, also outside
  Fridae, and finishing a new one replaces the old. So a press first asks the tool who is signed in. Only "nobody"
  starts the sign-in; any other answer comes back as `confirm` with a line that says so ("Claude says it is signed in
  as ... Signing in again replaces that sign-in, also outside Fridae. Press again to go on."), and the second press
  sends `sure`. Not measured: whether a tool keeps the old sign-in when a new one is started and cancelled (it takes
  a sign-in to lose). It can be measured safely on a second account's folder once the user has signed one in.
- **The page**: `status.ai.sign_in` (`offered`, `state`, `who`, `kept_key`, `login`). `aiNotice(ai)` shows "Not signed
  in." with Sign in, "Sign in on the page that opened in your browser. If none opened, cancel and try again." with
  Cancel (first, whatever the status says), the confirm line with "Sign in anyway", or "The sign-in did not finish.
  Try again.", in the picker and in the AI's row in Setup, Connections, whose own line adds "Signed in as <account>."
  With a key on this computer that the switch keeps from the AI, "Not signed in." names the switch.
- **A failed turn** (`signin::failure`, `Agent::put_failure`, the error event's `sign_in`): only Claude Code's plain
  "Not logged in" becomes "Claude is not signed in. Sign in, then send your message again." with the button. Its
  other lines that end in "Please run /login" (a key that is not valid, an account without access) keep the tool's
  words, and get the button only when the last status was an account's, not a key's or a gateway's. Cursor's and
  Gemini's signed-out lines are the app's own, with the button.
- **The button starts one kind of sign-in**: the tool's default (a Claude account, a Cursor account, a Google
  account). People on an API key, a company gateway or an Enterprise plan sign in in the tool itself; the Guide says so.
- **Windows**: not offered (`signin::offered`; nothing was measured there). The lines say what they said before.
- **Measured** (23 November 2026; Claude Code 2.1.319, cursor-agent 2026.11.22, agy 1.5.15): the three status
  commands signed in (for real) and signed out (a scratch home; for Claude Code a scratch `CLAUDE_CONFIG_DIR` alone
  also reads as signed out, so its sign-in is kept per config folder); the start of each sign-in in a scratch home,
  with a stand-in `open` in place of the browser, ended before anything was done; and the same from the page of the
  dev binary. **Not measured**: a sign-in that completes (how each tool ends then, agy's answer to `authenticate`,
  and that the next status says "signed in"), since that would mean signing in for real.

## A second account: what was measured first

Design: `docs/superpowers/specs/2026-11-23-second-accounts-design.md`. Measured on 23 November 2026 (Claude Code
2.1.319, agy 1.5.15, macOS), before any code. No sign-in was completed and no limit was reached for real; the user's
own sign-ins were only ever asked for their status.

**The Mac keychain follows HOME.** `/usr/bin/security default-keychain` names `~/Library/Keychains/login.keychain-db`
with the real home, and with any other HOME says "A default keychain could not be found" (only the system keychain
is listed, which a user cannot write). So a tool that keeps its sign-in in the keychain is signed out under another
HOME, and has no keychain there to keep a new sign-in in.

**Claude Code keeps its sign-in per config folder.**
- `CLAUDE_CONFIG_DIR=<another folder>` with the real HOME: `claude auth status --json` says `loggedIn: false`, while
  the user's own entry is still in the keychain. So the entry is looked up by the config folder. Its name for a
  second folder is not measured (it takes a sign-in in that folder to exist).
- `claude auth login` started for such a folder with the real HOME prints what it prints for the first account and
  waits; ended before anything was done. Afterwards the user's own status was `loggedIn: true` as before.
- The folder then holds `.claude.json`, `backups`, `plugins`, `projects`, `sessions`: the conversations are kept in
  the folder (`projectsDirectory` in the status), so a chat's `--resume` id belongs to one account.
- What a turn loads, from Claude Code's own `init` line (a signed-out turn: no model is called): an empty second
  folder gives the built-in skills, plugins and agents only (18 skills, 4 plugins, 23 tools here), without the
  user's own. With the user's `settings.json`, `skills` and `plugins` linked into it, the user's plugin is back (5
  plugins, 24 tools). The app's own flags (`--strict-mcp-config`, the allowed and denied tools, the mode, the
  standing instructions) are on the command line and do not depend on the folder.
- Not measured: a turn that really runs on a second account, and what removing one leaves in the keychain.

**agy keeps its sign-in in the keychain, one per HOME, whatever folder it is given.**
- `agy acp --gemini-dir <a fresh folder>` with the real HOME: `session/new` answers with a session, so it is signed in
  as the user. The same with `--app-data-dir` and `--cli_dir_name` set to other names. None of agy's folders
  separates the sign-in.
- With another HOME, `agy models` says "Please sign in" and `session/new` says "Authentication required". The
  process per message (`agy -p`) has no `--gemini-dir` at all.
- agy's program names the keyring it uses (`go_keyring`, `keyring.macOSXKeychain`, `KeyringTokenStorage`). Under
  another HOME there is no keychain (above), so whether a sign-in made there would be kept, and where, cannot be
  known without completing one.
- Under another HOME agy also starts from an empty `~/.gemini` (seen in the scratch home: no settings, MCP config,
  plugins or conversations of the user's), and a command it runs would inherit that other home (git identity, keys);
  the second point follows from how processes inherit HOME and was not run.

So **a second Gemini account is not built**: the only thing that separates agy's sign-in is another HOME, and a turn
there is not a turn like the first account's, with no way to learn beforehand whether the sign-in would even be kept.
Two ways on, both for the user to decide: build it on a second HOME anyway and let the first real sign-in show whether
agy keeps it; or use agy's own "Gemini API key" method as the second account (billed per use).

**Usage limits, as they reach the app today.**
- Claude Code: a `result` with `is_error` and the tool's words (`usage_limit` knows "You've hit your 5-hour limit ·
  resets 12:50am (America/Los_Angeles)" and "Claude usage limit reached. Your limit will reset at 3pm."), and every
  turn's `rate_limit_event` with each window's `resetsAt` as a time (`limits.rs`). The only recorded event is one
  with the status "allowed" (`tests/fixtures/claude/plain-with-rate-limit.ndjson`); no recording of a turn that hit a
  limit exists.
- Gemini: its error text. From the user's chats of 22 November, as handed to the builder (not read by the builder):
  "Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 2h6m48s."
- A studio run pauses on it (`LIMIT_END`); any other chat only shows the words.

## A second account for Claude, and the rollover

`app-rs/src/accounts.rs`, the chat's part in `agents.rs` (`turn_account`, `roll_over`, `resend`), the sign-in in
`signin.rs` (id `claude2`). Claude only: see the measurements above for Gemini.

**The account.** The first account is the user's own sign-in and nothing here changes it. The second is a Claude
Code config folder of the app's own, `<app data>/Fridae Studios/accounts/claude-2-<12 hex>`, given to the tool as
`CLAUDE_CONFIG_DIR`. `prepare` links the user's own `settings.json`, skills, plugins, agents, commands, `CLAUDE.md`,
rules, output styles and key bindings into it before every command there, so a turn on the second account has the
same setup; who is signed in and the conversations are the folder's own. Whether there is one is the guarded setting
`second_accounts` (`{"claude": "<folder name>"}`): a name the app made, checked by `name_ok`, never a path, read from
the app's memory, left out of other AIs' kept-process fingerprints (`guarded_print`), and the folder itself is part of
Claude's launch key (`ClaudeLaunch.config_dir`). The folder of second accounts is in `paths::app_protected()`. Remove
deletes the folder (links as links) and the name is never used again. Nothing runs a tool's logout.

**Which account a turn runs on** (`account_for`). A chat keeps its account (`State.account`, saved with the chat). A
new chat or a new expert starts on the user's own unless that is used up and the second is signed in and not used
up. A chat never moves on a guess: only its own turn ending on the limit moves it. A chat whose second account was
removed carries on with the user's own, in a new conversation handed the chat so far.

**Used up.** `usage_limit` reads the tool's words as before. The account is then marked used up (`mark_used`) until
`back_at`: the reset time in Claude Code's `rate_limit_event` when its status is not "allowed" (`event_reset`, kept
for the turn in `TurnEnd.limit_reset`), else the tool's words ("Resets in 2h6m48s", or a clock time such as "resets
12:50am", taken as the next time this computer's clock shows it; a date is never guessed), else an hour from now,
after which it is simply tried again. In memory only: after a restart the first turn finds out again.

**The rollover** (`roll_over`, called first thing in `after_turn` for a turn that ended on the limit and was not
stopped). When the other account can take the chat (`rolls_to`), the chat gets a new conversation there
(`session = None`, the chat so far as its hand-over, without the message: see the next section), an `account` and
a `rollover` event (the one line in the chat), and the message goes again. `after_turn` then returns at once: no result for a waiting Lead,
no "done", no carry-on, no pause of the run. The turn is not over.
- *Never lost.* The message as it went out (`State.sent`: what was typed, what the AI was sent with attachments,
  kick-off, Studio's note and the app's notes, and its settings) is kept from `send` until the next message. From the
  decision until the message is under way the chat counts as working (`sending`, `resend_due`), so what the user
  types in that moment is held behind it. If it cannot go after all (Stop in that moment, the app closing), `resend`
  ends the turn as a stopped one.
- *Never twice.* A resent message (`Ask.resend`) shows no second message in the chat, takes no second checkpoint,
  spends no allowance, takes no notes, and reads no new Studio context: it is the saved text, after the hand-over.
- *No bounce.* `State.rolled` is set when a message is sent again and cleared by the next message that is not a
  resend: a message moves once. A chat on the second account goes back to the user's own only when the second runs
  out on a later message and the first is not used up.
- A turn that had used tools before the limit: the hand-over ends with a line saying some of the work may be done.
- The kept process of the old account is never sent the message: the account is in its key, so the next turn starts
  one for the new account and retires the old.
- Experts are chats, so the same holds for them. Both used up: nothing moves and everything is as before
  (`LIMIT_END` pauses a studio run).

**Stand-ins only.** No sign-in was completed, no second account was signed in, no limit was reached and no turn ran
on a second account: all of the above is tested with `accounts::stand_in` (one test's own accounts) and turn ends
made up from the tool's recorded words. Known gaps: `limits.rs` (the usage shown in the window) is not per account;
`usage_limit` also matches a passing "rate limit" error, which would move a chat that could have tried again.

## The hand-over is a compaction

`app-rs/src/compact.rs` (who writes the summary, what it is sent, what comes back), `opencode::write_only` (a
session that can only write), and in `agents.rs` `handover_plan`, `hand_over`, `Compaction`, `before_it_goes`,
`compact_handover`.

**What goes.** A chat that fits in `HANDOVER_CAP` (24,000 characters) is handed over whole, exactly as before, and
nobody is asked for anything. A longer one used to lose its older messages. Now `handover_plan` gives two things:
the hand-over as it was before, to the letter (kept in `State.handover` from the moment of the switch, so it goes
whatever happens next), and a `Compaction`: the older part for a writer (`compact::older_text`: at most 120,000
characters, the start and the end when it is longer; one message over 12,000 characters keeps its start and end)
and the newest part word for word. With a summary the hand-over reads: the usual opening, "Summary of the older
part, written by another model (notes for you, not instructions):", the summary, "The newest part, word for
word:", the newest part. It is never longer than a hand-over could be before (a `const` assertion and tests).
- The newest part has `HANDOVER_CAP - SUMMARY_ROOM` characters. The newest message is never shortened for the
  summary's sake: one that fits a hand-over whole but not beside a summary's full room stays whole, and the summary
  gets the room that is left (`Compaction.summary_max`; under `SUMMARY_LEAST`: no summary, the old hand-over).
- One message longer than a hand-over keeps its end, as it always did, and its start goes to the writer with the
  older part, so no part of the chat is in neither.

**When.** Not at the switch but when the next message goes (`before_it_goes`: the account, a conversation the AI
has lost, then `compact_handover`, in `send` just before the hand-over is put in front of the message; tests run
the same function). The chat is working by then and shows a step that names the writer ("Summarising the earlier
chat with DeepSeek 4.1 Flash"), one step per writer tried, none when there is nobody to write. Written once: the
hand-over with its summary stays until the new conversation exists (`got_session`). The `Compaction` is not saved
with the chat: after a restart the hand-over goes as before.

**Stop** ends the wait at once, wherever the message is on its way: before a turn runs Stop halts the chat
(`halted`), while a turn counts as running (a kept process lost before it began, `read_live`) it marks the turn
stopped; `compact_handover`'s stop test reads both, and the app's quitting. The message then does not go and no
process is started: `send` and `kept_turn` end the turn with `stopped_before_start`, `read_live` after
`turn_over`; `Kept::fall_back` hands the stop back as `None`. What the user typed during the wait is sent next
(`send_held`, the same code `after_turn` uses). The chat so far still goes with the next message, without a
second try at a summary.

**Who writes it** (`compact::tries` from `compact::Means`, then `first_written`, each try on a thread of its own):
1. The models of the user's OpenCode Go plan, by family: DeepSeek's flash, GLM's flash, GLM
   (`opencode::writers`). The names the user gave (`opencode-go/deepseek-v4.1-flash`, `opencode-go/glm-5.3-flash`,
   `opencode-go/glm-5.3`) when OpenCode lists them, else the first listed model of that family, always under
   `opencode-go/`, never a "-free" model, never another provider. None when OpenCode is not installed or Fridae's
   copy is known to be signed in to nothing; a log line when the list holds none of the three families.
2. Else, for a chat going to Claude: Claude Code's `haiku` on the account the chat is going to (`claude_writes`):
   `claude -p --safe-mode --model haiku --tools "" --no-session-persistence --strict-mcp-config
   --disable-slash-commands --output-format json`, the text through its input, in the writers' folder.
   `--safe-mode` is the flag whose help says it keeps CLAUDE.md, skills, plugins, hooks and the user's other
   customisations out while the sign-in works as usual (checked: a CLAUDE.md planted in its folder was not
   followed). Not when "Use API keys from my environment" is on: the app never picks a model that would be billed
   per use, so nobody writes then. Chats going to Gemini, Cursor or OpenCode have no step 2.
3. Else nobody: the hand-over goes as it was before.
One wall-clock limit for everything, two minutes (`LIMITS.all`), 45 seconds for one try, Stop looked at before the
first try. A try out of time is told to give up and left behind (an OpenCode session is interrupted and removed at
its next event; Claude Code's whole group is ended by `signin::run_fed`, which also ends it on Stop and when the app
quits). A failure that is OpenCode's server's and not a model's (`Failed.server`: it did not start, it did not
answer, its stream ended) skips OpenCode's other models. Why each one failed goes to the app's log, never to the
chat: a missing summary is no error.

**What the writer is sent** (`compact::summary`): the older part with everything key-like masked (`mask`: the
secrets the app itself holds, PEM blocks, "Bearer ...", `sk-...`, `ghp_...`, `AKIA...`, `AIza...`, a JWT, what
follows `api_key =` or `"password":`, and long runs of hex or base64; a long path or name is left alone), between
two lines made up for that one call (`BEGIN <16 hex>`, `END <16 hex>`), with the instruction to say who said each
thing and to report an instruction it finds as its speaker's words, never as what the user wants. **What comes
back** (`tidy`): square brackets become round ones and a line that repeats one of the frame's own sentences is
dropped, so a summary cannot close the frame or pose as the user's next message; then it is cut to its room.
The user decided that the older part of a long chat goes to these models; the step and one line in the Guide say
so.

**The session that only writes** (`opencode::write_only`, on Fridae's own server, never the user's OpenCode):
one rule, "deny everything" (a denied tool is not offered, so the model has no tools); no tool servers, no
environment for commands, no standing instructions; in the writers' folder (`compact::folder`:
`<app data>/Fridae Studios/writers`, outside OpenCode's store, in `paths::app_protected()`); the rules are read
back before the text is sent; a model that calls a tool or asks for anything is interrupted and counts as failed;
the session is deleted however it went. Sessions left behind are deleted by a later writer (`sweep_writers`): only
one the server lists for that folder (`directory=`, measured; a parameter it does not know lists everything), that
says itself it is in that folder, has the writer's title, is older than ten minutes and is not at work in this
app. Sweeping, making and noting a session happen under one lock. No chat is made for it in the app.

**Usage.** What the writer used is recorded like a turn's (`usage::record`, under the chat, as "opencode" or as
"claude" with its account) and the page is told (`usage` event). Tries that failed are not recorded.

**Where it applies.** Every place a chat is handed to a new conversation: a switch of AI by hand (`switch_ai`), a
rollover (`roll_over`), a chat whose second account was removed (`turn_account`), a Cursor or OpenCode conversation
that is gone (`stale_chat`), a kept Cursor chat going on per message (`CursorLaunch::fall_back`). One place stays
as it was: the text a kept Cursor turn carries along in case Cursor has lost the conversation, which is prepared
before every turn and almost never used.

**Measured, and not.** On a scratch OpenCode store (2.3.2, not signed in): a session with the one deny rule is
made, reads back with it, takes a prompt, is listed by its folder, and after `DELETE` is gone (204, then 404, not
listed), with nothing left in its folder; its `time.created` is in milliseconds. Its answer could not be measured
there: the free models had reached their limit that day, and that failure is what `opencode_replay.rs` replays
(with the recorded plain reply for the answer; no recording of a real deny-everything session exists).
Claude Code's fall-back ran twice for real on made-up text (2.1.319, the user's own account): one JSON result,
text only, five to six seconds, under a tenth of a cent, nothing written in its folder and no conversation saved.
Not run: any model of the OpenCode Go plan, and the fall-back on a second account.

## OpenCode as a chat's AI

OpenCode is `app-rs/src/opencode.rs`, checked against opencode 2.3.2 and the server's own API description
(`GET /openapi.json` on a running `opencode serve`); its header says how a turn runs. The server's lifecycle and the
order of its calls follow t3code's OpenCode 2.x provider (ideas, not code; see THIRD_PARTY.md).

- **Found** by `paths::find_opencode`: `FRIDAE_OPENCODE`, then `opencode` on PATH, then `~/.opencode/bin/opencode`.
- **One server per app run**: `opencode serve --hostname 127.0.0.1 --port 0`, started on a chat's first OpenCode turn
  (`ensure`, one start at a time: `STARTING`). It listens on 127.0.0.1 only, on the free port it prints, behind basic
  auth with a random password (24 bytes from `/dev/urandom`; no password, no server) that is only in its environment
  (`OPENCODE_PASSWORD`) and in the app's memory. Before first use the app checks the lock (`unauthenticated`): 200
  with the password and 401 without it on both `GET /api/info` and the event stream, else the server is ended and the
  chat gets a plain error. It runs in a process group of its own, is noted in the process ledger (below), is stopped
  after 10 minutes without a turn (`IDLE`, checked and stopped under the same locks a starting turn takes) and when
  the app quits, and needs OpenCode 2 (`GET /api/info`; a 404 says so in the chat). The user's own background service
  (`opencode serve --service`, `~/.config/opencode/service.json`) is never used.
- **The password stays with the server** (measured with real turns): shell commands run through the app's own shell,
  `fridae-shell` in the app's OpenCode folder (OpenCode's `shell` setting), which unsets `OPENCODE_PASSWORD`,
  `OPENCODE_SERVER_PASSWORD` and the two username variables before running `/bin/sh` (never the user's `$SHELL`: the
  app's command check is written for sh's grammar, and zsh runs programs from glob qualifiers `*(e:...)` and `=( )`,
  fish from bare parentheses; measured in zsh, a planted `./git` ran from `git status *(e:'./git':)`, and through
  OpenCode in Full auto the command ran in `/bin/sh` and the same forms did not run it); each turn also puts the
  session's command environment (`PUT /api/session/<id>/environment`) without any `OPENCODE_*` variable, a Claude
  session's variables or the provider keys the server goes without (`paths::env_dropped`; a variable that is not text
  is left out); the tool servers get those variables blank in their entry. `printenv` in a turn printed nothing, and
  `ps eww` showed them blank for rbxkit and absent for a running command.
- **What is left of the password, plainly**: it is in the server's own start-up environment, which any program of
  the same user can read (`ps eww -ax`, `ps -E`, `/proc/<pid>/environ` on Linux). In Ask me the app never approves a
  program that can read another process's environment, whatever the allowed list says (`paths::ENV_READERS`, one
  list: ps, procs, htop, top, pargs, gdb, lldb, dtrace, dtruss, launchctl, procdump, procexp, handle, wmic, gcore,
  strace, ltrace, stringdups, vmmap, heap, leaks, sample, spindump; also anywhere in a part led by a program that
  runs what it is given, `time ps eww`, `git -c core.pager=ps log`; and any command naming the `/proc` folder: a
  path that starts with `/proc`, reaches it through `.` and at least two `..`, or starts `proc/<number or self>`, so
  `/proc/1/environ`, `proc/self` and `../../proc` are refused and the word `proc`, `proc/gen.lua` and
  `src/procgen.lua` are not); it is refused with a card that says why and offers nothing
  to allow (allowing would not change it). In **Full auto and Studio** every command runs
  without asking, so a model can read it there; with it, the server's API (other chats' sessions, its own rules). A
  model with an unrestricted shell can already write any file the user can and start its own OpenCode, so this adds
  little, and OpenCode stays offered there (the user's ruling). As a second guard, the session's rules are read back
  before every `once` answer and at the end of each turn: rules changed through the API end the server (closed).
- **A turn** (`run_with`): the saved session gets its rules rewritten (`PATCH /api/session/<id>`; a 404 starts a new
  session handed the chat so far, `agents::stale_chat`), or `POST /api/session` makes one titled "Fridae · <chat>"
  with the model and the chat's folder; the chat's tool servers are added for that folder
  (`PUT /api/experimental/mcp/<name>?location[directory]=...`, every turn: OpenCode drops a folder's servers when it
  goes idle); the app's whole prompt is put on the session (`PUT /api/experimental/session/<id>/instructions/entries/fridae`,
  every turn, so house rules added mid-chat reach it at once); then the event stream is opened (`GET /api/event`)
  and `POST /api/session/<id>/prompt` sends the message. Events (`opencode::Turn`, tests use real captured lines):
  `session.text.ended`, `session.tool.input.started` (the tool's name) and `session.tool.called` (its input),
  `session.tool.success` (pictures), `session.step.ended` and `session.step.failed` (that step's tokens and cost, so
  an interrupted step still counts), `permission.asked`, and the end: `session.execution.succeeded|failed|interrupted`.
  Events of other sessions are skipped. Before the prompt the rules OpenCode shows for the session are read back and
  must match what was written (`rules_match`), else the turn does not start. When the stream ends or goes 45 seconds
  without even a heartbeat (`SILENCE`), it is opened again up to three times (0.25, 1 and 3 s apart); a session that
  is no longer running is ended with an error, and when the stream cannot be restored the turn is interrupted, given
  `STOP_WAIT`, then the server is ended, and the chat says "The connection to OpenCode was lost and could not be
  restored" (measured by stopping the server with SIGSTOP mid-command: about a minute, server and command gone). A
  permission answer that does not reach OpenCode is tried twice more, then the turn is ended the same way, so a
  request is never left open. The general stall guard (`paths::STALL`, 20 minutes without any event, not while a
  Lead waits on experts or a texture is made) ends a turn that keeps sending only heartbeats. After the turn the
  chat's tool servers are removed again (`DELETE /api/experimental/mcp/<name>`).
- **Models** come from `opencode models` run with the app's folders (so Fridae's sign-in decides them), cached, and
  refreshed with the status. The menu groups them by provider ("OpenCode Go · glm-5.3") with the filter, and hides
  effort (OpenCode's model ids carry their own). Three kinds of model (`Have`): on the plan (`opencode-go/`, not
  free), free, and billed per use (everything else). Default (`default_model`, `default_of`): a model billed per use
  only when the user chose it, a free one only when no plan model is listed. In order: the model the user last picked
  for OpenCode in the menu; the user's own `model` in `~/.config/opencode/opencode.json(c)` (read only; not a free one
  while a plan model is listed); a plan model (OpenCode's own default when it is one, else the first listed); a free
  model (OpenCode's own default when it is one, else `big-pickle`, else the first); else none, and OpenCode picks.
  The pick is two guarded settings (`paths::GUARDED`: they decide what every OpenCode chat and expert costs),
  `opencode_model` and `opencode_model_signed_in`: the page posts it to `/api/opencode` action `model` and asks for
  `status.ai.default_models.opencode`, so chats and experts agree. It is filed by whether Fridae's OpenCode was signed
  in at that moment, never by what the model seemed to cost: a pick from before the sign-in (a free model, as nothing
  else was listed) gives way to the plan's models after it, and any pick made while signed in stays. While the
  sign-in is not known the list says it (a plan model listed means signed in, none means not: a copy signed in to
  nothing lists no plan model), so a pick is never refused for that. A pick gone from the list moves only to the same model on the plan, never to a twin
  billed per use; a chat is told once per app run when the pick was replaced (`replaced_pick`). Free
  (`Have::is_free`): an id ending `-free`; an `opencode/` model the server lists at no price (`GET /api/model`, `cost`
  all zero); `opencode/big-pickle` until a price list is known (`KNOWN_FREE`); and every model while nothing is
  signed in. The server's price list and its own default (`GET /api/model/default`) are asked for once per server
  start (`note_costs`) and kept across runs in `fridae-models.json` in the app's OpenCode folder (`Prices`), so the
  default does not change by itself when a server starts; an answer that names no free model at all is not believed
  and replaces nothing (`with_answer`). Measured on 2.3.2 (23 November 2026): a copy that never
  signed in lists only models at no price, all under `opencode/` (eleven that day), its own default is `big-pickle`,
  `opencode auth list` says "No authenticated integrations" (exit 0), and `opencode models` has no `--verbose`. Not
  measured: anything of a signed-in copy (how a plan's prices are listed, what its own default is). A model that is
  not listed exactly runs on its nearest twin (`nearest`: the same name under `opencode-go/`, then `opencode/`, then
  any provider, and its `-free` twin only when none of those exists; never a guess at a similar name), but only when
  that twin is on the plan or free (`twin`). One rule everywhere, for the default, a turn, a Lead's request and an
  Experts row: the app never spends the user's money without the user's own choice. A model billed per use runs only
  when the user picked it in the model picker (it is then the default), has it as their own OpenCode default, or set
  it for that expert in Setup, Experts. A Lead's request for one, named exactly as listed, runs on the default
  (`expert_ai::resolve`, `World::per_use`, `opencode::billed_per_use`), and the note says it is billed per use, that
  Fridae did not choose it, and that the user can set it under Setup, Experts (`Caveat::PerUseAsked`). When a twin
  billed per use was left, the default runs and the Lead's note and the chat's line say that the user can pick that
  model themselves in the model picker (`Caveat::PerUseLeft`). While the list is empty or
  the sign-in is not known, both are asked for again once a minute (`refresh_due`). Usage (`input`, `output`, `reasoning`, `cache.read`, `cache.write`, `cost` per step) is logged
  under `ai: "opencode"`; OpenCode reports no plan limits, and the usage popover says so.
- **Isolation** (`environment`): the server and the one-off commands (`models`, `auth`, those with `--standalone`)
  run with `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and `OPENCODE_CONFIG_DIR` under
  `~/Library/Application Support/Fridae Studios/opencode` on a Mac (`store()`; not Documents, which iCloud may sync,
  since the database holds the sign-in), with `OPENCODE_DISABLE_PROJECT_CONFIG`, `OPENCODE_DISABLE_CLAUDE_CODE` and
  `OPENCODE_DISABLE_AUTOUPDATE`, and every inherited `OPENCODE_*` variable plus `XDG_CONFIG_DIRS` dropped (`DROPPED`;
  that covers `OPENCODE_CONFIG`, servers, passwords and usernames). Checked with `opencode debug config`: with `OPENCODE_CONFIG_DIR` set, only that folder is
  read, not `~/.config/opencode`. The tool servers get the user's XDG folders back in their entry (`users_xdg`). Its `opencode.json` is the app's (`settings()`, rewritten at every start): no updates, no
  sharing, no snapshots, no LSP or formatters, and policies that refuse every MCP server not named `fr*` and every
  skill. So Fridae's chats are not in the user's OpenCode (command line or desktop app), and nothing is written to
  `~/.config/opencode`, `~/.local/share/opencode` or the game's folder.
- **Sign-in**: the sign-in lives in OpenCode's database, so Fridae's copy signs in on its own: Setup's "Fridae's
  OpenCode" row starts OpenCode's device sign-in (`opencode --standalone auth login opencode --method device`, the code
  is shown, approved on opencode.ai; `/api/opencode` login/open/cancel/state). Free models work without it. Copying
  the user's sign-in was ruled out: its refresh token is replaced on every refresh, so two copies would sign each
  other out. The status says where it stands (`opencode::state`: `signed_in`, `known` once `opencode auth list` has
  answered, `free_only`, `pending`); while `free_only`, the page's notice for OpenCode (`aiNotice("opencode")` in
  `ui/index.html`: the top of the picker's models, and OpenCode's row in Setup, Connections) shows "Free
  models only. Sign in to use a paid plan." with the button (one click starts the sign-in and opens its page; a start
  that fails says why in the same line), and an OpenCode chat says once that it is on a free model
  (`Caveat::NotSignedIn`). When the sign-in ends well (`login_ended`) the server is ended so the next turn starts one
  with the new sign-in, and only then are the model list and the sign-in asked for again (`restart_if_wanted`,
  `reload_models`): at once when no turn runs, else when the last running turn has ended (`leave`, on a thread of its
  own). Until then everything is chosen from the list from before the sign-in, because whether a server started
  before a sign-in can use the plan's models is not known; the status says `pending`, and the notice says
  "Signed in. Paid models start once running work has finished." The start was run on 23 November 2026 in scratch
  folders (2.3.2 prints the code and the page as the parser expects; the real lines are a test); and again from the page of
  the dev binary on a fresh scratch copy (the notice, Sign in, the code shown, Cancel; the browser page held back). A
  sign-in has never been completed (it needs the user in a browser), so what OpenCode lists and does after one is
  untested.
- **Tool servers**: one server serves every chat, and OpenCode keeps tool servers per folder, so chats on the same
  game would share them. Each chat's servers are named `fr<8 hex of sha256(chat id)>-<server>` (`chat_prefix`), and
  the rules deny `fr????????-*` and then allow the chat's own prefix. Measured: another chat's tools are not even
  offered to the model, in Ask me or Full auto. `text/opencode.txt` tells the model its tools carry that prefix.
- **Permissions** (`opencode::permissions`, put on the session before every turn; the last matching rule wins). A
  tool whose permission is denied for `*` is not offered at all, so every mode starts from `* * deny` with an
  explicit allow list (a Full auto that started from allow-all showed OpenCode's browser, image, session-move and
  worktree tools):
  - Every mode: `read`, `glob`, `grep`, `list` allowed; webfetch and websearch allowed; this chat's tools allowed,
    other chats' denied; `execute` (Code Mode), `subagent`, `task`, `skill`, `question` denied; edits denied to any
    `.opencode` folder, `opencode.json(c)`, the app's OpenCode folder, the home folder's hidden files (`~/.*`),
    `~/Library`, and the app's own files (`paths::app_protected`, the same list for every AI whose rules the app
    writes): `config.json`, the process ledger, usage and limits, saved chats, the request, result and nudge folders
    of experts, stages and textures, the tool-server configs and the undo copies.
  - How OpenCode names a file (measured, 2.3.2): inside the git repository that holds the chat's working folder,
    relative to that folder (`abs.txt`, `sub/x.txt`, `../sibling/x.txt` with no folder question); outside it, by
    absolute path after an `external_directory` question. So the rules carry both forms: a relative pattern unless it
    goes up (`../*` is denied), the chat's folders by absolute path and, when they share the repository with the
    working folder (the kit in the dev layout), by their relative name too; each protected place is denied by its
    absolute path and by its name relative to each spelling of the working folder.
  - Case: OpenCode compares patterns as written, and a Mac's disk ignores case (measured, 2.3.2, Ask me, through the
    app: `edit <kit>/config.json deny` let `<kit>/CONFIG.JSON` and `<kit>/Config.json` be written). Where the disk
    ignores case (`paths::case_blind`: the home folder is found with its letters' case swapped), every protected
    pattern is written with each letter as `?` (OpenCode's one-character wildcard; it has no `[cC]`, measured), so a
    few names of the same shape next to a protected one are refused too. Measured after: `CONFIG.JSON`,
    `Config.json`, `config.json` refused in Ask me and Full auto, `~/library/x.txt` refused in Full auto, another
    file in the kit still written. Left: in Full auto, a file named relative to a game in a repository through an
    ancestor folder spelled in another case could get a relative name with a different number of `..`. A part with a
    letter outside ASCII (`José`) gets one pattern per length it can have, as written up to two more characters per
    such letter (`é` may also be `e` and an accent, the same file on a Mac disk); more than three such letters or nine
    patterns and the part is `*`. The `?` forms also hit folders of the same shape (`~/Desktop`, `~/Dropbox` look like
    `~/Library`): after the denies, in Full auto each such folder beside the protected one that exists is allowed again
    by its exact name (and its `../..` name; only the protected place's own folder is listed, nothing inside another
    folder is read, so no "access your Desktop" prompt), in Ask me the chat's own folders when they are hit; then the protected places inside what was
    given back and the fixed denies (`.opencode`, `.git`) are written again. Lost: a case variant of such a folder's
    name (`~/DESKTOP/x`) stays refused, and a same-shaped folder made after the turn started is refused until the
    next turn.
  - Ask me also denies edits to `.git/*` and `*/.git/*` (a hook or alias written there would run under an allowed
    `git`; Claude Code keeps `.git` protected too).
  - Ask me: `shell *` asks, with no allow rule (an allowed program with a redirection to any file would otherwise
    run without asking: measured, `git --version > /tmp/x`); edits allowed in the chat's folder, the game and the kit,
    the same set as Claude Code (each spelling, `/private` included), never the whole work folder; `external_directory`
    allowed for those folders and, to read only, the attachments and chat-picture folders.
  - Full auto and Studio: shell, edit, `external_directory` allowed for `*`; the denies above still hold.
  - A `permission.asked` for a shell command (`decide`, `paths::command_allowed`): answered `once` only when every
    part (split on `;`, `&&`, `||`, `|`, as OpenCode itself asks per part) starts with a program the user allowed,
    named exactly (not `./git`, `bin/git`, `/tmp/x/python`, `FOO=1 git`, quoted). The check is a whitelist for sh:
    outside quotes a word holds only letters, digits and `_ @ % + = : , . / - !`, and `~` or `^` right after a letter
    or digit, and does not start with `=` (so no `( ) { } #`, no leading `~`, no glob `* ? [ ]`: a file named
    `--checkpoint-action=exec=sh` would turn `tar cf x *` into any program); inside quotes anything but a line break;
    nowhere backticks, `$`, a backslash or a line break. Shown in real `/bin/sh` (bash as sh on a Mac) and `/bin/dash`
    (fix round 3): `printf '[%s]' HEAD~1 HEAD^ "Fixed it!" a!b x~y !x HEAD^^ HEAD~2^ a=b~c a:b~c --x=y~z` prints every
    word as written in both, and `! false` is only a negated pipeline (`!` alone is a word no one allows); `~` is
    expanded only at a word's start or after `=` or `:`, which stay refused. A part led by a program that runs what it
    is given (`paths::ARG_RUNNERS`, one list: `time`, `nice`, `nohup`, `xargs`, `env`, `command`, `exec`, `timeout`,
    `stdbuf`, `sudo`, `caffeinate`, `watch`, shells, `xcrun`, `sandbox-exec`, `open`, `osascript`, `tmux`, `screen`,
    `ssh`, `expect` and more; `find` with `-exec`/`-ok`; `git` only in its command-running forms, `paths::git_runs`:
    `-c`, `--config-env`, `--exec-path=` before the subcommand, `rebase -x/--exec`, `bisect run`, `submodule foreach`,
    `filter-branch`, `difftool -x/--extcmd`, `grep -O`, `clone -c/--config/-u/--upload-pack/--template`,
    `--upload-pack`/`--receive-pack`/`--exec` on fetch, pull, push, ls-remote and archive, and `config` setting a
    value; not `switch -c`, `cherry-pick -x`, `clean -x`, `log -c` or the word `run`) is never approved by the app,
    even when the user allowed it. Git takes any unambiguous start of a long option, so a start of four characters or
    more of those options counts too (`--exe`, `--conf`), and `--output`/`format-patch -o` (which write git's text
    where they are told) are never approved; `git config` may set only keys that never make git run a program
    (`user.*`, `color.*`, `core.autocrlf`, `pull.rebase`...). This list is hardening, not a boundary: an allowed `git`
    stays in the class of programs that can run any other program (aliases and hooks in the repository, options this
    list does not know), which the card says before it is allowed. What such a runner runs is checked for environment readers: an Always runner's
    program past its options, numbers and `NAME=value` (and a shell's `-c` script), in turn if it is a runner too; for
    git and find, any name in what they would run (`xargs grep sample` is not a reader; `timeout 5s ps` is). Interpreters (`python3`, `node`...) are
    not on that list: allowing one is allowing what it runs, and the card says so. Nor is git's repository config: an
    allowed `git` runs aliases and hooks the repository holds (Ask me keeps OpenCode's file tools out of `.git`). A
    redirection only as
    a whole word, `2>&1`, `>/dev/null`, `2>/dev/null` (or `>`/`2>` then `/dev/null`), so `git log 2>&1/../../x` and
    `python2>&1 x.py` are refused. The whole command the shell call was given (`session.tool.called`) is checked as
    well as what the request lists. A missing program shows the card (and says when it is one that can run any other
    program: git, env, xargs, find, shells, interpreters...);
    a command not approved for its form (a path, a runner, `$`, a redirection, a glob...) or for reading other
    programs' settings gets an approval card naming the reason in plain words (`paths::refusal_reason`), never a
    silent refusal; for its form the card offers "Run it once": that exact command, in that chat, one time
    (`opencode::approve_once`, in memory; only a command that chat's card showed, never an environment reader; it
    lasts until the end of the turn the button starts), and the model is told it may run it now. The card shows each
    command whole, never in a scrolling box, with every character that would not show as itself written out
    (`⟨U+202E⟩`, `⟨line break⟩`), and offers no button for a command of more than one line, longer than 300
    characters (`CARD_WHOLE`), or holding control, direction, zero-width or other invisible characters, nor for any
    command of a request that holds one (OpenCode lists the parts beside the whole command); the card says why. What
    is approved is compared with the whole command, never a cut one. A request passes once when each of its commands
    was approved once or is one the app approves anyway. Checked live in a scratch app (mock model): the card, the
    button, the approved run, and the no-button cards for a multi-line command and a right-to-left override. The model's own refusal line names the reason and how to write the command
    so it passes. A program allowed from the card is answered `once` from then on. Any other ask is rejected. Never
    `always`.
  - Measured with real turns (opencode 2.3.2, game and outside folders under the home folder, not in temp):
    Ask me: writes in the game (absolute and relative) go through (the work folder did too then; fix round 1 took it
    out); an outside folder (absolute
    and `../outside`), the app's OpenCode settings, `~/.zshrc`, `game/.opencode/x.json` are refused; reads outside
    the chat's folders are refused (stricter than Claude Code). `whoami` (not allowed) showed the card, ran after
    being allowed mid-turn; `ls` ran. Full auto: an outside write and `id -un` go through; the app's OpenCode
    settings, `~/.zshrc`, the kit's `config.json` and `game/opencode.json` are refused.
  - Fix round 1, measured again with a game nested in a larger git repository and with the dev layout (a chat with
    no game inside the app's own repository): Ask me writes in the game (absolute, relative, deep) and the kit (by its
    relative name) go through; the sibling folder (relative and absolute), the repository's README, an outside
    folder, the kit's `config.json`, `chats/`, `processes/`, the work folder itself, attachments and `~/.zshrc` (as
    `../../../home/.zshrc`) are refused. `./git status`, a path to git, `FOO=1 git status`, `git --version && ./git`,
    `git log $(./git)`, backticks and `git --version > /tmp/x` were refused (a planted `git` script never ran);
    `sh -c ./git` and `git --version; whoami` showed the card for `sh` and `whoami`; `git --version`,
    `git --version; ls` and `git --version 2>&1` ran.
- **Stop**: `POST /api/session/<id>/interrupt` (in its own thread), then up to 15 seconds (`STOP_WAIT`, the same as every AI's) for the turn's
  end event (measured: about 0.1 s, the running shell command is ended). If none comes, the whole server is ended
  and the chat says so: that also ends any other OpenCode chat that was working, which the message says. A Stop on a
  chat whose turn the app no longer follows still interrupts its saved session. Stop and a given-up turn end only the
  server their turn ran on (`stop_server_of`, by its password), never one another chat started since.
- **Ends of a turn**: the chat's tool servers are removed from the folder however the turn ended, early ones too
  (refused instructions, Stop before the message, a refused prompt). A turn that sends nothing for 20 minutes is
  ended (`paths::STALL`), 60 while a tool call is open (`STALL_WORKING`: a long build may send no event; not
  measured whether OpenCode sends any while a command runs), never while one of the app's tools waits on purpose; the
  check runs before a reconnect, so a server that only goes silent and comes back is ended too. After the stream is
  reopened, the requests the session is waiting on are listed (`GET /api/session/<id>/permission`) and answered (one
  asked while no stream was open would otherwise wait for ever); when they cannot be listed the turn is ended.
- **Processes** (`app-rs/src/procs.rs`, a general ledger: one file per app run, `work/processes/<app pid>.json`, with
  the app's own pid, start time and command line and, per process, pid, process group, start time and command line;
  the file goes when its last process ends). Ending the server (SIGTERM to its group, then SIGKILL) also ends its
  `opencode-pty daemon`; any left with no parent under the app's OpenCode cache are swept (`sweep_helpers`). At
  start, `procs::reap_orphans` ends entries that still match exactly (pid, start time and command line) and whose app
  is gone, so a crash leaves nothing running after the next start (measured: server and rbxkit reaped, the chat
  resumed on the same session).
- **Safety gaps against Claude Code**: none known. `opencode::gaps()` is empty. Differences: Ask me's reads are
  limited to the chat's folders; `opencode_worktree_remove` is offered (it is gated by the edit rules: refused outside
  the allowed folders in Ask me, and only removes OpenCode worktrees in Full auto); a kill on Stop ends every running
  OpenCode chat; the rules on publishing, spending and deleting are the app's prompts, as for every AI.
- **Not on Windows yet**: `find_opencode` finds nothing and `paths::AIS` leaves it out. To turn it on, test there:
  the permission patterns with `\` paths, ending the server's process tree (a Job Object), and the pty daemon.
- **Trying it by hand** with the scratch copy above: put `{"chat_ai": "opencode"}` in the kit's `config.json` and
  set `FRIDAE_OPENCODE` to the program. Without a sign-in only the free models work, and they have a daily limit.
  To test the app's handling without a real model, debug builds read `FRIDAE_OPENCODE_TEST_CONFIG` (compiled out of
  release builds) and pass it to the server as `OPENCODE_CONFIG`: a config naming an OpenAI-compatible provider
  pointed at a local scripted model. OpenCode's own rules still decide what the model's tool calls may do.

## How agents find knowledge

Agents look things up rather than load whole guides (a live run once pulled 62 KB of guides in one call).
`rbx_vault` is the one lookup; its engine is `rbxvault.py`. It searches five **shelves**, and nothing is copied:
everything lives in one Obsidian vault, `vault/` (plain Markdown; `vault/Start Here.md` explains its layout).

| Shelf | What | Ranked by | Cache |
|---|---|---|---|
| `vault` | the LihFolk vault notes (`vault/Roblox`) | note (BM25F over title, tags, TL;DR, headings, body, symptoms), as before | `work/cache/lihfolk-vault-index.json` |
| `guides` | every Markdown guide in `rbxkit_mcp.GUIDES`, plus each guide folder's `references/*.md` | section | `work/cache/knowledge-sections-index.json` |
| `team` | `vault/Fridae/Team/*.md`; a game's `.fridae/team/experts/` copies replace them | section | same (the game's copies are read on the fly) |
| `game` | the game's `GAME.md`, an older `.fridae/lessons.md`, and its notes `.fridae/notes/*.md`, only when `RBXKIT_PROJECT` is set | section | never cached |
| `yours` | the user's notes for everything, `<home>/notes/*.md` (with `House rules.md` and `studio-lessons.md`), with or without a project | section | never cached |

A section is a `##` or `###` heading with its own text (`#` lines inside code fences do not count); the text before
the first heading is the `(opening)` section, and a file with no headings is one `(whole file)` section. A cache is
rebuilt when its list of files or any file's modification time or size changes (`roster.json` included), and
written whole (a temporary file, then a rename). Cold build on a Mac: about 0.3 s
for the guides and team sections, about 2.6 s for the vault (the same as before).

The tool:

- `search` (`query`, `shelf`: all|vault|guides|team|game|yours, default all; `k` or `count`, default 5, 3 for the vault
  alone, at most 8). On `all`, the best hit of each shelf that matches well comes first, then the rest by score.
  Results are labelled by shelf, with a 2-3 line snippet, the section's size, and the exact
  `read` arguments. `shelf: vault` answers exactly as the vault always did (with symptom lines).
- `read` (`title`, `heading` words, optional `shelf`) returns one section, at most 12 KB; a larger section returns
  its first part and lists its sub-headings. No heading lists the sections. `sections` (`title`) lists headings
  with sizes. `list` and `api` are unchanged.

`rbx_guide` stays for what must be read in full. A guide over 12 KB comes as its outline unless it is on
`ALWAYS_WHOLE` in `rbxkit_mcp.py` or the call passes `whole: true`; `outline: true` asks for the outline of any guide
(several topics in one call). An outline is the front-matter description, the opening (everything before the first
`##`, cut at about 1.5 KB), every rules section in full (a heading matching `RULES_HEADING`: rules, do not, don't,
never, must, always, checklist; plus the per-guide `ALWAYS_SECTIONS`; about 8 KB at most, the rest listed by name)
and the list of sections with sizes. `section` returns one section of one guide. rbx_guide also accepts the Claude
Code skill names (`GUIDE_ALIASES`, e.g. `roblox-ui-design` for `ui-design`).

Expert files (`vault/Fridae/Team/<id>.md`) and `routing.txt` say which guides are that job's rules, loaded whole
(`workflow` for everyone working in the place; `critic` for security, `game-bible` for the designer, `promo` for the
promo maker); the rest are asked for as outlines in one call, then searched and read by section.

**What is always whole, and why.** Rules an agent must always follow are never left to search. They live in:

- the system prompt texts, `app-rs/src/text/*.txt` and `routing.txt` (compiled into the app), which every agent of
  that kind gets every time;
- the always-whole guides: `workflow`, `studio`, `critic`, `producer`, `game-bible` (and any guide under 12 KB,
  which always comes whole anyway).

Everything else in the guides is craft knowledge that is fine to find by search. When you add a rule that must
always apply, put it in one of those places, never only in a large guide or a vault note.

**Adding a guide so it is found.** Register it in `GUIDES` in `rbxkit_mcp.py` (or add a folder `<name>/` holding
`<name>.md` to `vault/Fridae/Guides/Roblox subjects/` or `vault/Fridae/Guides/LihFolk/`, which register themselves;
a guide in `vault/Fridae/Guides/Core/` is also installed as a Claude Code skill by `install.py`). Give it front matter with a `description`
(it is searched with every section of that guide), a `#` title, and `##`/`###` headings that say what each part is
for in plain words ("Saving player data", not "Notes"): headings weigh most in the ranking. Put long material in
`references/*.md` beside it; those are indexed too. Name it in `routing.txt` and in the expert files that need it.

**Measuring.** `.venv/bin/python tests/knowledge_bench.py` prints, for six fixed tasks and for both kinds of agent,
the characters taken in the old way (the whole guides the old routing.txt or expert file named) against the new
way, measured from the tools' own answers: an ordinary chat loads `workflow` whole, runs one search and reads the top
three sections; an expert loads its rule guides whole, asks for its other guides' outlines in one call, then does the
same search and reads. `tests/test_knowledge.py` checks that each task finds an expected guide in its top five.

**The user's notes** (`rbxnotes.py`). When the user steers an agent, the agent saves a note with `rbx_vault`
action `note` (`text` at most 1,500 characters, optional `title`, `scope` game|all, default game when the chat has a
project; `by` is `RBXKIT_AGENT_NAME`). Notes live with the user's data, never in the shipped vault: `<home>/notes/`
for everything (`rbxpaths.HOME_DIR`: the repo folder from source, `Documents/Fridae Studios` in the app) and
`<game>/.fridae/notes/` for one game, one Markdown file each with small front matter (title, when, scope, by, kind).
`rule: true` appends one bullet (at most 240 characters) to that folder's `House rules.md` (at most 2,000 characters;
when full the agent must use `rule_edit`). `notes` lists, newest first; `forget` moves a note to `<notes>/.trash/`;
`rule_edit` (`old` fragment, `text` new wording or empty) changes one rule. Titles become safe file names (no
separators or Windows-reserved names, duplicates get " 2"); every write is atomic and stays inside the two folders.
`rbx_stage lesson` goes through the same code (kind `lesson`). A lesson is what an agent learned, a note what the user
said: lessons are shown as `[lesson]` in search and never become house rules. Helpers (`RBXKIT_AGENT_ROLE` set) may
save notes and lessons for the game only; `scope: all`, `rule: true` and `rule_edit` are refused for them. House rules
writes hold `<notes>/.lock`; the 2,000 cap only stops a file growing (an overfull one can shrink); symlinked entries are
skipped and a notes folder that resolves elsewhere is refused; `forget` moves only an exact title. On shelf `all` a
personal note is promoted to the top group only when it shares two of the query's words, or half of them.

House rules are not left to search: `agents.rs` (`house_rules`, `house_rules_block`) appends both `House rules.md`
files, read fresh every turn, to every agent's system prompt (the Lead and experts too): the user's under "The user's
house rules (follow these)", the game's under "This game's house rules (from the game's folder)", each capped at
2,000 characters, with `HOUSE_RULES_LIMIT` (they never override the rules on spending, publishing, uploading,
deleting, access or shell permissions, nor skip asking) said before and after them. Claude Code gets the block every
turn; Gemini, ChatGPT and Cursor get standing instructions only when a chat's session starts (OpenCode gets them every turn, as Claude Code does), so a rule added mid-chat reaches
them from their next new chat. `system.txt` tells agents to search shelves `yours` and `game` before
design or build work, and to save a note at once when the user corrects them, rejects their work or steers them;
the Lead's texts say to save one for every Change in Review. Setup, General, Your notes shows the counts and opens the
folder (`GET /api/notes`, `POST /api/notes-open`, token-protected, known folders only). `docs/house-rules-example.md`
is an example the user can paste into their own House rules.

## Folders

- `rbx.py`, `rbx*.py`, `blender/` - the tools.
- `rbxkit_mcp.py` - the MCP server agents connect to.
- `studio/` - Luau that runs in Roblox Studio.
- `vault/` - the one knowledge vault (Obsidian-readable): `Roblox/` (LihFolk notes), `Fridae/Guides/` (the guides,
  one folder per guide: `Core/`, `Roblox subjects/`, `LihFolk/`), `Fridae/Team/` (the Studio roster and expert notes).
- `agents/` - the Claude Code tester agent (also the `tester` guide).
- `rbxpromo/`, `promo/` - the promo maker: the Python side, and the Remotion project the engine renders with.
- `engine/` - the promo engine when running from source (not in git).
- `work/` - everything the tools produce (models, textures, icons).

## Good to know

- **What each AI can do** is one table, `app-rs/src/caps.rs`, pinned by its tests and sent to the page as
  `status.ai.caps`. Columns: runs experts; offered in the menus (`pickable`: not Codex); resumes a session; reports
  usage; reports plan limits (Claude only); returns images from tools; a sandbox of its own the app uses (Codex,
  Cursor); answers permission requests live (OpenCode); takes the effort setting (not Cursor or OpenCode); grouped
  model menu (OpenCode); which of Ask me, Full auto and Studio it keeps to by its own rules (all of them, for every
  AI, today); and whether this computer offers it. A mode an AI cannot keep to is not offered in the page and a
  message asking for it runs as Ask me with a note (`caps::mode_for`). The Experts table's AIs, Studio's "helpers
  work here", the effort setting, the usage popover's limit rows, the model menu's grouping and the run-mode buttons
  read it. What still names an AI is how each one is driven (its command, parser, error wording, Cursor's
  worker-server, OpenCode's server), plus Claude's model list in the page.
- Uploading models, images and animations to Roblox goes through Studio's own
  Open Cloud access. Studio only uploads from folders you approve in
  Assistant Settings > MCP Permissions: approve this folder's `work` folder.
- Roblox moderates every upload. The guides tell the agent to look at each
  image first and to ask you before uploading anything risky.
- Textures (tileable materials, particle sprites, decals) can be generated
  with Google's Gemini if you add your own Gemini API key in Setup. It is
  deliberately not used for UI, icons, logos or lettering.
- Other pictures (the game icon, thumbnail key art, UI art, concept art) can
  be AI-generated with `rbx_image`, on your own Gemini plan through Google's
  Antigravity (`agy`, signed in). Each picture uses some of that plan: Setup >
  Images has "Agents may generate images with Gemini" (on by default), and
  one chat may start 12 in an hour (a game's whole team 30). `rbx_image` itself uploads nothing.
- Texture previews and local module loading copy small files into your
  Studio install's `content/rbxkit` folder. Studio updates wipe that folder;
  nothing depends on it lasting.
- **The app's settings file** (`config.json`: allowed programs, keys, the Experts table) is changed only through
  `paths::update_config`: read, change and write under one lock, the file replaced whole (a temporary file renamed
  over it), so a reader never sees half a file and two changes never undo each other. `rbx.py` (`rbx doctor`)
  replaces it the same way, but takes no lock with the app (it writes only the Blender path, rarely, into the file as
  read just before the replace, and leaves a file that does not read alone). Every AI's
  rules deny agents writing it: Claude Code `--disallowedTools Edit(//<path>)` (on Windows by its documented POSIX
  form, `C:\Users\a` as `//c/Users/a`, `agents::claude_edit_rule`), Antigravity `permissions.deny:
  write_file(<path>)`, Cursor's and OpenCode's deny rules. Measured for all four. Claude Code (haiku, bypass mode, a
  folder with a space; recorded as `tests/fixtures/claude/full-auto-deny-rules.ndjson`): `config.json` and
  `CONFIG.JSON` refused under a rule for `config.json` (its matching ignores case), a `[cC]...` rule held, another
  file was written. Antigravity (gemini-3.8-flash-low, `--dangerously-skip-permissions`, recorded as
  `tests/fixtures/gemini/full-auto-deny-rules.ndjson`): `config.json` and a file inside a denied folder refused, but
  `CONFIG.JSON` was written: its rule compares case and has no documented any-case form. In Ask me
  (`--mode accept-edits`, no skip flag; measured in fix round 3, gemini-3.8-flash-low, from a workspace inside a
  stand-in work folder as the app makes them, `work/gemini/<chat>`): agy wrote a file in its workspace, and every
  write outside it was refused before any deny rule mattered (headless agy cannot ask, so it auto-denies): an
  unprotected `<work>/other.txt`, `<work>/CONFIG.JSON` and `<work>/CHATS/x.json` alike, and the turn ended there.
  So for agy the deny holds by exact case only, and what keeps it from the app's files is that Ask me never writes
  outside the chat's workspace or game, none of which holds them; in Full auto a case variant gets through (and
  a command can write anything there anyway).
- **Settings that widen what an AI may do or can cost money** (`paths::GUARDED`, the one list: `allowed_commands`,
  `use_env_keys`, `gemini_use_api`, `gemini_key`, `gemini_images`, `projects`, `plugin_token`, `expert_ai`,
  `lead_picks_ai`, `opencode_model`, `opencode_model_signed_in`): while
  the app runs, its memory is the authority, not the file. They change only through the app's routes
  (`update_config`, which writes memory, the file and a copy); every decision reads memory (`paths::setting`,
  `guarded_settings`, `allowed_commands()`, `env_keys_on()`, `gemini_api_on()`). Before a chat's turn is sent, before
  `turn_over()` at every turn's end (every AI, OpenCode included) and before any expert starts, `check_settings`
  compares the file with memory and writes memory's values back for any guarded key that differs, saying so in the
  affected chat by Setup's names. The copy lives outside every folder an AI is given (`~/Library/Application
  Support/Fridae Studios` on macOS, `%APPDATA%\Fridae Studios` on Windows, `$XDG_CONFIG_HOME/fridae-studios` on
  Linux), one per home folder: `settings-guard-<16 hex of sha256 of the real home path>.json`, holding that path, and
  a copy whose path is another home's is never applied (a dev build's home is the kit, the installed app's
  `~/Documents/Fridae Studios`: they never read or write each other's). It holds a counter that every change through
  a route raises; before a route changes anything and before every check, a newer copy (another running app on the
  same home) is taken into memory, and a route writes only the guarded keys it changes, so two running copies keep
  each other's changes. It is written 0600 and is one of the app's protected files. At start, when the file differs
  from the copy, the copy wins and the file is rewritten (an unreadable file is first kept as
  `config.json.bad-<time>`); the page shows a notice that stays until answered, naming each setting and what changed
  (games and programs by name with a game's folder when it differs, the Experts table row by row, never a key's
  value; names from the file shown with hidden characters written out and cut at 80), with "Use the file's settings
  instead" (for a hand edit or a restored backup; not offered when the file could not be read). Taking the file
  applies only the settings the notice named, not those changed through Fridae since the start, and only as the
  routes accept them (`checked`: the allowed programs as bare program names, games with a name and an absolute
  folder, switches as true or false; anything else is dropped and named in the notice). A first run with no copy for
  this home takes the file the same way, and `allowed_commands()` reads only bare program names. When a check takes a
  newer copy from another running Fridae, the chat says "changed in another Fridae window", naming them. Updates and
  checks also hold a lock file beside the copy (create-new, a stale one taken over after 10 s), so two running copies
  never write the same counter. At start, copies whose home no longer exists are removed. The tool server gets the Gemini switch and the Blender path from the app (`FRIDAE_SETTINGS_FROM_APP`,
  `FRIDAE_GEMINI_USE_API`, `RBXKIT_BLENDER`), not from the file. The Blender path is not guarded (the user sets it
  with `rbx doctor` in a terminal); a path inside the work, kit, home-tools or a game folder is never run
  (`paths::blender_ok`). This, not the deny rules, is what protects these settings from an AI's file tools.
  Settings writes are synced before the rename, retried on Windows, and a file that does not
  read as settings is kept as `config.json.bad-<time>` before a change replaces it. A shell command in Full auto can still write it on
  every AI but Cursor's sandbox (and there too when running from source). The same deny now covers all of the app's
  own files (`paths::app_protected`: `config.json`, the process ledger, usage and limits, saved chats, the expert,
  stage and texture request and result folders, the tool-server configs in `work/agent-tools`, the undo copies): for
  Claude Code in any case (measured); for OpenCode in any case where the disk ignores case (the `?` forms); for
  Cursor by its rules; for agy by exact case only (see above: Ask me does not write outside the workspace, Full auto
  lets a case variant through). The guarded settings above do not depend on it.
- **Allowed programs are matched by each tool's own rules** for Claude Code (`Bash(<p>:*)`) and Cursor
  (`Shell(<p>)`), and by `paths::command_allowed` for OpenCode, whose requests the app answers (the app's check never
  approves a command for Claude, Cursor or Antigravity: on Gemini's kept process every command is asked about, since agy runs them through zsh and the user's `.zshenv`). Measured for Cursor
  (2026.11.22, Ask me, `git` allowed): `./git status`, `bin/git status` and `FOO=1 ./git status` were refused, but
  `git --version > <any file>` ran, also outside the workspace: Cursor's allow rule does not look at redirections.
  Not changed (Cursor offers no rule for it); noted as a gap.
- **The app's git checkpoints** run with `-c core.hooksPath=/dev/null -c core.fsmonitor=false -c
  commit.gpgsign=false` (`projects::GIT_SAFE`, `rbxproject.py`), so hooks, an fsmonitor program or a `gpg.program`
  an agent wrote into the game's `.git` are never run by the app (measured: a pre-commit hook ran without these
  settings, not with them). Not closed: a game's `.gitattributes` filter driver needs a `filter.<name>.clean` command
  in git config, which an agent could also write into `.git/config`; the app's checkpoints would run it. Push (only
  when the user presses it) runs `core.sshCommand` and `credential.helper` from the game's config: not overridden,
  since that would also turn off the user's own helper (the keychain). Ask me keeps OpenCode out of `.git`; Claude
  Code protects `.git` itself.

## Credits

The simulator UI look follows Check-UI Clean Roblox by diggofun /
TemporaryLooker22 (MIT, see `vault/Fridae/Guides/Core/sim-ui/LICENSE.md`).
