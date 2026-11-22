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
`parent`, `team` and `expert` in `agent_new`. The All projects graph (`overviewWanted` / `overviewFrame`) draws games,
their open and saved chats, and experts tied to their Lead; it runs only while that page is in front.
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
  tests.test_studio_flow`. `tests/test_studio_flow.py` is a dry run of a Lead through `rbxkit_mcp.handle` with a stub
  app thread that answers `rbx_team` requests as finished experts.
- Rust: `cargo test --manifest-path app-rs/Cargo.toml` (loop, studio_run, studio_undo and team tests use temp
  projects named with spaces and accents; under `cfg(test)` everything, the work folder included, goes in
  `<temp>/fridae-tests/<pid>/`, whatever `RBXKIT_WORK` says, so nothing writes to the real work folder; the next test
  run clears the folders of runs that are over).
- Stop: every agent starts in its own process group on macOS and Linux, and Stop (or quitting the app) sends the
  group SIGTERM, then SIGKILL after 2 s; Windows keeps `taskkill /T /F`. The tests check it with a real shell that
  starts a grandchild.
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

## Cursor as a chat's AI

A chat can run on Claude (Claude Code), Gemini (Antigravity's `agy`) or Cursor (`cursor-agent`, the Cursor Agent CLI);
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
    folders is still refused.
- **Safety gaps against Claude Code** (what Cursor can do that Claude Code cannot, in the same mode):
  - Ask me: Cursor's file tools may write into temp folders (`/tmp`, `/private/var/folders`), which Claude Code's
    `acceptEdits` would refuse. Writes elsewhere outside the workspace folders are refused, as for Claude Code.
  - Full auto and Studio: none known. Claude Code runs those with every check off; Cursor keeps its allow list, deny
    rules and sandbox, so it can do less. `cursor::gaps()` holds the one line the page would show under the message
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
turn; Gemini, ChatGPT and Cursor get standing instructions only when a chat's session starts, so a rule added mid-chat reaches
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

- Uploading models, images and animations to Roblox goes through Studio's own
  Open Cloud access. Studio only uploads from folders you approve in
  Assistant Settings > MCP Permissions: approve this folder's `work` folder.
- Roblox moderates every upload. The guides tell the agent to look at each
  image first and to ask you before uploading anything risky.
- Textures (tileable materials, particle sprites, decals) can be generated
  with Google's Gemini if you add your own Gemini API key in Setup. It is
  deliberately not used for UI, icons, logos or lettering.
- Texture previews and local module loading copy small files into your
  Studio install's `content/rbxkit` folder. Studio updates wipe that folder;
  nothing depends on it lasting.

## Credits

The simulator UI look follows Check-UI Clean Roblox by diggofun /
TemporaryLooker22 (MIT, see `vault/Fridae/Guides/Core/sim-ui/LICENSE.md`).
