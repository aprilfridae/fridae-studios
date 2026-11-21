# Studio mode: a Lead, a team of experts, and a sign-off inbox

Design, 2026-11-21. Status: for approval.

## Why

The user wants Fridae Studios to be the free, open-source replacement for the paid "AI game studio" kind of app
(RoAutomation is the one they pointed at): "a program that just spits things out and you just sign off on it every
once in a while and it just goes". They are a team of exactly three people, not a company, and believe this kind of
tool should not be closed software.

That product's shape, from its own site: you talk to one Lead; it hands each part to one of about fourteen
specialists (ideas, design, code, UI, world, assets, two playtesters, exploit review, economy, shop pricing,
icon/thumbnails/promos, live numbers, producer); they keep building while you are away; you approve the design
plan, and finished pictures and clips wait in a Review screen for "your yes or your changes".

## What Fridae already has (and this builds on)

- One generalist agent per chat, with about fifty guides loaded by topic (`routing.txt`, `rbx_guide`).
- Run modes under the message box: Ask me, Full auto, Overnight (up to 24 hours, `text/overnight.txt`), with a
  checkpoint at the start and "Undo the night".
- Helper agents: `rbx_team` (rbxteam.py → `work/team-requests` → `app-rs/src/team.rs`) starts a `worker` or a
  `critic` in its own chat and hands its report back. Helpers cannot start helpers. `MAX_AGENTS` is 6.
- A board per game (`<project>/.fridae/board.json`, `rbx_board`, Board tab) with todo / doing / needs / done.
- `GAME.md` (the game bible) per project, with a decision log.
- Research tab: reads Roblox's charts, pitches five ideas, "Build it on autopilot".
- Promo maker and Promos tab (plan 1 of the promo work, on the branch this one starts from).

## What is missing

1. A named team with a Lead: today it is one generalist plus anonymous "helpers".
2. A pipeline with stages, so a whole game moves forward without the user steering each step.
3. A sign-off inbox: one place where finished work waits with a picture or clip and Approve / Change.
4. Live numbers after launch (needs a Roblox Open Cloud key). Out of scope here; see Non-goals.

## Goals

- Start a game from an idea (typed, or a Research pitch) with one button, and have it move through the stages
  without the user, stopping only at the gates below.
- The user sees only what is theirs: decisions that block the run, and finished work to look at.
- "Change" on any piece of work reaches the expert who made it, and the run carries on.
- Everything stays local, free and editable: the roster and every expert's instructions are plain files.
- Works with what exists: Overnight, checkpoints and undo, the board, GAME.md, helper agents, the guides.

## Non-goals

- Live numbers / liveops, publishing for the user, a phone remote, a hosted asset library. Later projects.
- New AI models or training. An "expert" is a role file plus the guides it loads.
- Changing how ordinary chats work. Studio mode is opt-in per chat.

## Anti-goals (would make it a failure even if it works)

- Publishing, spending money or deleting the user's work without a yes. The existing Full auto rule stays.
- A wall of approvals. If the user has to click through every step, it is babysitting, not a studio.
- Copying RoAutomation's agent names, playbook text or look. The roster uses plain job names and Fridae's own guides.
- Burning the user's AI plan silently. The run shows what it is using and has a limit on experts at once.

## Approaches considered

- **A. Studio mode on top of what exists (chosen).** The Lead is the game's chat in a new run mode. Experts are
  helper agents with a role file. Stages and review items are small files in the project, like the board. A Review
  tab reads them. Keeps Overnight, undo, the board and every guide. Smallest thing that gives the whole experience.
- **B. A separate orchestrator** (a new long-running service that owns the pipeline and drives agents as jobs).
  More control over ordering and retries, but it duplicates agents.rs, loses undo and the board, and is weeks of
  work before the user sees anything.
- **C. Only rename the helpers and rewrite the autopilot prompt.** Cheap, but there is no sign-off inbox and no
  visible pipeline, which is the part the user described.

The user approved A in conversation ("Yes, write it up").

## The design

### The team

A roster file ships with the kit: `team/roster.json`, plus one instruction file per expert in
`team/experts/<id>.md` (not `studio/`, which already holds the Roblox Studio plugin scripts). A project may override
either by placing a file of the same name under `<project>/.fridae/team/`. Both folders ship in the packaged app. Each roster entry:

```json
{"id": "coder", "name": "Coder", "job": "Writes and fixes the game's scripts",
 "stage": "build", "guides": ["roblox-luau", "roblox-architecture", "roblox-security"], "tools": [], "kind": "worker"}
```

`kind` is `worker` or `critic` (what the helper is started as). `guides` are `rbx_guide` topics; `tools` names tools the
expert is expected to use (for example `rbx_icon`, `rbx_promo`), mentioned in its instructions. The expert file is appended to that helper's
existing prompt: its job, the guides it must load first, what "done" means for it, and what it hands back (a report,
and for visible work a review item with a picture or clip).

First roster (plain job names; the builder may adjust wording, not the set, without asking):

| id | Name | Stage | Does | Built on |
|---|---|---|---|---|
| lead | Lead | all | The one you talk to. Plans stages, hands work out, checks it, files review items | autopilot, `producer` |
| scout | Scout | idea | Pitches ideas from Roblox's charts, scores an idea you bring | Research, `market-research` |
| designer | Designer | design | Writes GAME.md and the build plan | `game-brainstorm`, `game-bible`, `game-design` |
| coder | Coder | build | Scripts and systems | `roblox-luau`, `roblox-architecture`, `lihfolk-systems` |
| ui | UI builder | build | Menus and HUD | `ui-design`, `roblox-ui`, `unslop` |
| world | World builder | build | Map, terrain, lighting, models | `roblox-world-building`, `roblox-blender-modeling` |
| effects | Effects and sound | build | VFX, animation, sound | `vfx`, `motion-design`, `sound-design` |
| playtester | Playtester | test | Plays each build as a new player, files bugs | `playtest`, `player-testing` |
| devices | Device tester | test | Phone, tablet, gamepad layouts and performance | `player-testing`, `roblox-performance` |
| security | Security reviewer | test | Exploits, remotes, data safety (critic) | `critic`, `roblox-security`, `roblox-code-review` |
| economy | Economy and shop | test | Prices, rewards, drop rates, shop | `economy-design`, `roblox-monetization` |
| art | Icon and thumbnails | launch | Icon, thumbnails, store text | `rbx_icon`, style guides |
| promo | Promo maker | promote | Vertical promos from the user's clips | `promo`, `rbx_promo` |

The Lead is not a helper: it is the game's own chat. Helpers still cannot start helpers.

### Stages and gates

`idea → design → build → test → launch → promote`. The Lead owns the order and may loop back (test → build) as
often as needed. Each stage ends with a checkpoint (the existing one) and a line in GAME.md's decision log.

Two gates block the run by default:

1. **Design plan** (end of design): the user approves GAME.md's summary and the build plan, or says what to change.
2. **Ready to publish** (end of launch): a checklist of what the user must do themselves (publish, set prices live).
   Fridae never publishes.

Everything else produces non-blocking review items: the run keeps going while they wait. The user can add gates per
game ("also stop after build") or remove the first one, in the Review tab's run settings.

### Files (per project, travelling with the game like the board)

- `<project>/.fridae/studio.json`: the run. `{"on": bool, "stage": "build", "stages": [{"id","status","started",
  "ended"}], "gates": ["design","launch"], "max_experts": 2, "lead": "<agent id>", "idea": "...", "started": ...}`.
- `<project>/.fridae/review/<id>.json`: one review item.
  `{"id","stage","expert","title","summary","media":[relative paths],"blocking":bool,"status":"waiting"|"approved"
  |"changes","note":"","created","decided"}`. Media files live in `<project>/.fridae/review/<id>/` (png, jpg, mp4;
  copied in by the tool so they survive later changes).
- `<project>/.fridae/lessons.md`: what this game taught (short dated lines). The Lead reads it and the user-wide
  `work/studio-lessons.md` at the start of a run and appends at each stage end. This is the "gets better with every
  game" part, kept honest and inspectable.

All writes are atomic (temp file, then rename), like the board.

### The agent side: `rbx_stage`

A new tool (module `rbxstage.py`, registered in `rbx.py` and `rbxkit_mcp.py` like the others), only for chats that
belong to a project:

- `status`: the run, the roster, open review items and any decisions made since the last call.
- `stage` (`id`, `status`): start or finish a stage. Finishing a gated stage requires its review item approved.
- `review` (`title`, `summary`, `media` paths, `expert`, `blocking`): file a review item; returns its id.
- `decisions`: the user's answers since the last call (approved, or changes with the note), marking them read.
- `lesson` (`text`, `scope`: game or all): append a lesson.

`rbx_team` gains an optional `expert` field. With it, the app starts the helper as that roster entry: the chat is
titled with the expert's name, the role shown in the app is the expert's name, and the expert file is added to its
prompt. Without it, `rbx_team` behaves exactly as today.

A new run mode, **Studio**, sits beside Ask me / Full auto / Overnight. It is Overnight's loop (never waits, keeps
choosing the next job, questions go to the board's needs column) with a studio prompt (`text/studio.txt`) that
tells the Lead to: follow the stages, hand each part to the right expert with `rbx_team`, never run more than
`max_experts` at once, check every report itself, file a review item for anything the user can see, call
`decisions` at each step and route "changes" back to the expert that made the work, and stop only at a gate. The
24-hour limit restarts whenever the user answers a review item or sends a message, so a run can last days as long
as the user signs off now and then. At a gate with nothing else to do, the Lead ends its turn; the user's decision
wakes it.

### The app side

- **Starting a run.** The run-mode menu gets "Studio". In Research, "Build it on autopilot" becomes "Start a studio
  run" (it creates the project folder if needed, then starts the Lead with the pitch). On the new-chat screen for a
  project chat, a short line under the composer explains Studio mode when it is selected.
- **Review tab** (top bar, after Board, with a count of waiting items). Per game:
  - A stage strip: the six stages, the current one marked, each with its state.
  - **Needs your answer**: blocking review items and the board's needs cards. These stop the run.
  - **To look at**: non-blocking items, newest first. Each shows the expert, title, summary and its pictures or
    clip (same media route pattern as promo cards: token-protected, confined to the review folder).
  - On every item: **Approve**, and **Change** (opens a one-line box; sending it sets status `changes`).
    "Approve all" for the non-blocking list.
  - Run settings: gates on/off, experts at once (1–4, default 2), Pause and Resume.
- **Deciding wakes the Lead.** When the user approves or asks for a change, the app writes the decision into the
  item and sends the Lead's chat a short message ("Review: <title> — approved" / "— change: <note>"), which
  restarts it in Studio mode if it was idle.
- **Chats** show experts by name (sidebar, Overview) instead of "helper", and a review item filed from a chat
  shows there as one line with "Open in Review", like promos.
- The Board tab is unchanged; needs cards also appear in Review so there is one place to answer.

### Limits and safety

- Experts at once defaults to 2 (plus the Lead), hard cap 4, inside `MAX_AGENTS`.
- The usual Full auto rule: no spending, publishing or deleting without the user.
- Every stage starts from a checkpoint; "Undo the night" covers a Studio run the same way (rename the button's
  label to "Undo this run" when the run is a studio run).
- Pause stops new work being handed out and lets running experts finish.

## Calls made after the builder check (mine, not the user's; correct any of them)

1. **Waiting at a gate.** Studio is Overnight's loop plus one new state. When the Lead has nothing left that does not
   depend on a blocking item, it ends its turn with `STATUS: WAITING`. The app then sends no "carry on", stops the
   24-hour clock and shows the run as "Waiting for you". A decision on any review item, or a message from the user,
   resumes the loop and restarts the clock. `STATUS: DONE` twice still ends the run, as today.
2. **Undo.** Each finished stage is a checkpoint (git commit of the project plus the Studio script snapshot, the same
   mechanism Overnight uses once). "Undo this run" offers the stage checkpoints, newest first, and goes back to the
   one chosen; the default is the latest. It must cover the experts' work too, so the app records each expert's
   Studio steps against the Lead's run (helpers started by a studio run add to the run's undo log).
3. **Agent slots.** `MAX_AGENTS` goes from 6 to 8. A run uses at most 1 + `max_experts` of them. When a spawn is
   refused because slots are full, the Lead waits and retries (it never drops the task), and the Review tab shows
   "Waiting for a free agent slot" on the run.
4. **One Studio place.** Prompt-level rules are not enough to promise, so the rule is simple and enforced by the Lead's
   instructions and by `rbx_team`: at most one expert that *changes* the place runs at a time (coder, UI, world,
   effects, economy when it edits scripts). Experts that only read, test, review or work outside Studio (playtester on a
   finished build, security review, designer, art, promo) may run alongside, up to `max_experts`. The roster entry
   carries `"writes": true|false`; `rbx_team` refuses a second writer for the same project with a clear reason.
   Truly parallel building in one place is a later project.
5. **Changes go to a fresh expert.** Helpers are one-shot. On "Change", the Lead starts a new helper as the same
   roster expert, handing it the review item, the user's note and the previous report. Nothing needs to stay alive.
   The new item replaces the old one in Review (the old one is kept as `superseded`, linked by `replaces`).
6. **Review media.** Pictures and clips come from the tools experts already have (Studio `screen_capture`, the VFX
   and video capture tools, `rbx_icon`, promo outputs). `rbx_stage review` copies any image or video file path the
   agent gives into the item's folder (png, jpg, webp, mp4, mov; up to 200 MB each, 8 files). An item with no media
   (code-only work) is allowed and shows its summary and a "what changed" list as text.
7. **Starting.** Studio has its own kickoff text. A new game with an idea or a Research pitch starts at **design**
   (the idea stage is marked done); with no idea it starts at **idea** and the Scout's pitches are the first review
   item. An existing game with a GAME.md starts at **design** too: the Designer reads the game and proposes "the plan
   for this run", which is the gate. `kickoff_new.txt` / `kickoff_existing.txt` stay for ordinary chats.
8. **One run per game, and it survives restarts.** `studio.json` is the truth. If the app restarts or the Lead's chat
   is closed, the run shows as Paused in Review; **Resume** starts a new Lead chat that adopts the run (reads
   `studio.json`, GAME.md, the board, open review items and lessons) and updates `lead`.
9. **Which AI.** The whole team runs on the Lead's AI. Studio mode is offered only where helper agents work; where
   they do not, the menu entry is disabled with one line saying why.

## Constraints

- Free, local, open source; runs on the user's own AI plan (Claude first; other AIs as the existing AI picker
  allows). macOS, Windows and Linux. No new paid services or accounts.
- One Studio place: experts working at the same time must not collide (the `producer` guide's rules apply; the
  studio prompt repeats the rule that two experts never edit the same instances or scripts).
- Paths with spaces and accents; project folders possibly inside iCloud/OneDrive (small files only, atomic writes).

## Left to the builder

- Exact wording of `studio.txt` and each expert file (follow the tone of the existing prompts and guides).
- The Review tab's layout, in the app's existing visual language (quiet chrome; the media is the only colour).
- Internal names, file layout inside app-rs, and tests (Rust unit tests for the file handling and routes, Python
  tests for `rbx_stage`, a dry run of the Lead with stub experts).

## How we will know it works

From a clean project: type an idea, press start in Studio mode, approve the design plan when it appears in Review,
and walk away. Coming back, the Review tab shows finished pieces with pictures, the board shows what was done,
nothing was published, and one "Change" note results in a new version of that piece from the same expert.
