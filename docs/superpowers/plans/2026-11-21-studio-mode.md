# Studio Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Studio" run mode where one Lead chat moves a game through stages by handing work to named expert agents, and the user signs off in a Review tab.

**Architecture:** Everything rides on what exists. The run and its review items are small JSON files in the project's `.fridae/` folder (like the board), written by a new Python tool `rbx_stage` and read by the Rust app. Experts are the existing helper agents started through `rbx_team` with a roster entry. Studio mode is Overnight's carry-on loop with one new state (waiting at a gate) and its own prompt.

**Tech Stack:** Rust (app-rs: tiny_http server, agents), Python 3 stdlib (rbx*.py tools, MCP server), one HTML page (ui/index.html). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-21-studio-mode-design.md` (binding; its "Calls made after the builder check" section is part of it).

## Global Constraints

- Free, local, open source; no new paid services, accounts or dependencies. macOS, Windows, Linux.
- Never publish, spend money or delete the user's work without a yes (the existing Full auto rule).
- Roster uses plain job names; do not copy RoAutomation's agent names, text or look.
- Kit files: `team/roster.json`, `team/experts/<id>.md` (NOT `studio/`, which holds the Studio plugin). Project override: `<project>/.fridae/team/`.
- Project files: `<project>/.fridae/studio.json`, `<project>/.fridae/review/<id>.json`, media in `<project>/.fridae/review/<id>/`, `<project>/.fridae/lessons.md`; user-wide `WORK/studio-lessons.md`. All writes atomic (temp file with a unique name, then rename).
- Stages, in order: `idea`, `design`, `build`, `test`, `launch`, `promote`. Default gates: `design`, `launch`.
- `max_experts` default 2, allowed 1–4. `MAX_AGENTS` becomes 8. At most one expert with `"writes": true` runs per project at a time.
- Review item status: `waiting` | `approved` | `changes` | `superseded`. Media: png, jpg, jpeg, webp, mp4, mov; at most 8 files, 200 MB each.
- Every new app route is token-protected like the others; every path served or written is confined to the project's `.fridae/review/` folder; ids are plain file stems (no separators, no `..`).
- All dynamic text in the page goes in through `textContent`/`value`. Follow the page's existing visual language (tokens, pills, 4px grid, sentence-case labels, type scale 11/12/13/14/15/16/18).
- Paths with spaces and accents must work. Python image/video paths are only copied, never decoded.
- Commits end with: `Co-Authored-By: Claude Fable 5.2 <noreply@anthropic.com>`

## Review Focus

1. The app restarts (or the Lead's chat is closed) mid-run → the run shows Paused and Resume adopts it; nothing is lost or duplicated. (Task 7)
2. The user answers a review item while the Lead is busy, or answers the same item twice from two windows → one decision, delivered once. (Tasks 1, 5)
3. A spawn is refused (slots full, or a second writer) → the Lead waits and retries; the task is never dropped and the run never ends as "done". (Tasks 3, 4)
4. A review item's media file is missing, huge, or not media → the item is still filed with a clear note; nothing outside the review folder is ever served. (Tasks 1, 5)
5. A project folder with spaces/accents, inside a synced Documents folder → every file write is atomic and every route works. (Tasks 1, 5)

---

### Task 1: Run and review files, and the `rbx_stage` tool

**Files:**
- Create: `rbxstage.py`, `tests/test_stage.py`
- Modify: `rbx.py` (register in `names`), `rbxkit_mcp.py` (`t_stage`, TOOLS entry `rbx_stage`), `rbxkit.spec` (hidden import)

**Interfaces:**
- Produces (Python): `STAGES = ["idea","design","build","test","launch","promote"]`; `load_run(project) -> dict`; `save_run(project, run)`; `run(action, a) -> str` with actions `status`, `stage` (`id`, `status`: `start`|`done`), `review` (`title`, `summary`, `media`: [paths], `expert`, `blocking`, `replaces`), `decisions`, `lesson` (`text`, `scope`: `game`|`all`).
- `studio.json` = `{"on","stage","stages":[{"id","status":"todo"|"doing"|"done","started","ended"}],"gates":[...],"max_experts","lead","idea","started","paused","read_decisions":[ids]}`.
- Review item JSON exactly as in Global Constraints/spec, plus `"replaces": id|""`, `"by_chat"`.
- Project from `a["project"]` or `RBXKIT_PROJECT`; no project → `KitError` like rbxboard.

- [ ] **Step 1: Failing tests** (`tests/test_stage.py`, temp project dir with a space and "é" in its name):
  - `test_status_on_fresh_project`: text mentions all six stages and "no run".
  - `test_stage_start_and_done`: start `design` → `studio.json` stage `design`, status `doing`; `done` on a gated stage with no approved blocking item → raises `KitError` containing "approve"; after writing `status: "approved"` into its blocking item → `done` succeeds and sets `ended`.
  - `test_review_copies_media`: a 10-byte `a.png` and `b.mp4` → item JSON `media == ["a.png","b.mp4"]`, files exist under `.fridae/review/<id>/`, status `waiting`.
  - `test_review_bad_media`: a missing path, a `.exe`, and a 9th file → item still filed, reply text names each skipped file and why; nothing copied for them.
  - `test_review_replaces`: `replaces=<old id>` → old item status `superseded`, new item `replaces` set.
  - `test_decisions_once`: item set to `changes` with note → first `decisions` call returns the title and note; second returns "No new decisions".
  - `test_lesson_scopes`: `game` appends a dated line to `<project>/.fridae/lessons.md`; `all` to `WORK/studio-lessons.md` (patch `rbxpaths.WORK`).
  - `test_no_project`: raises `KitError`.
- [ ] **Step 2: Run** `python3 -m unittest tests.test_stage -v` → FAIL.
- [ ] **Step 3: Implement** `rbxstage.py` following `rbxboard.py`'s shape (load/save with unique temp names, `run(action, a)`), ids `uuid4().hex[:8]`. `status` prints the run, the roster names (Task 2 adds the roster; until then skip that line if `team/roster.json` is absent), open items and unread decisions. Register the CLI (`rbx stage …`) and MCP tool (description: one sentence + "load the `studio` guide").
- [ ] **Step 4: Run** → PASS. Also `python3 -m unittest tests.test_promo_mcp -v` (the AST stdin test must still pass).
- [ ] **Step 5: Commit** `studio: run and review files, rbx_stage tool`

### Task 2: The roster and expert files

**Files:**
- Create: `team/roster.json`, `team/experts/{lead,scout,designer,coder,ui,world,effects,playtester,devices,security,economy,art,promo}.md`, `rbxroster.py`, `tests/test_roster.py`
- Modify: `rbxstage.py` (status lists the roster), `rbxkit.spec` + `build_mac.py`/`build_app.py` only if `team/` is not picked up automatically (confirm how `promo/` and `skills/` are shipped and do the same)

**Interfaces:**
- Produces: `rbxroster.load(project=None) -> list[dict]` (kit roster with per-project overrides merged by `id`); `rbxroster.expert(id, project=None) -> dict` with keys `id,name,job,stage,guides,tools,kind,writes,text` (`text` = the expert file's content); unknown id → `KitError` listing valid ids.
- Roster entries exactly the 13 rows of the spec's table; `kind`: `critic` for `security`, else `worker`; `writes`: true for `coder, ui, world, effects, economy`, false for the rest; `lead` has `kind: "lead"`.

- [ ] **Step 1: Failing tests:** `test_kit_roster_complete` (13 ids, each has a non-empty expert file, every `guides` entry is a key of `rbxkit_mcp.GUIDES`, every `stage` in `STAGES` or `"all"`); `test_project_override` (a project `.fridae/team/experts/coder.md` replaces the text; a project `roster.json` with one changed entry overrides only that id); `test_unknown_expert`; `test_writers` (exactly the five ids above).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_roster -v` → FAIL.
- [ ] **Step 3: Write** the roster and the 13 expert files. Each expert file, 15–35 lines, in the plain second-person style of `app-rs/src/text/worker.txt`: its job; "Load these guides first" (the roster's list); what it may and may not touch; what done means; what to hand back (a complete report; for visible work, capture a picture or clip and name its file path in the report so the Lead can file it). `lead.md` is short: the Lead's standing duties not covered by Task 4's mode prompt (how to brief an expert, how to check a report).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `studio: roster and expert files`

### Task 3: Experts through `rbx_team`

**Files:**
- Modify: `rbxteam.py` (optional `expert` on `spawn`), `rbxkit_mcp.py` (schema field `expert`), `app-rs/src/team.rs`, `app-rs/src/agents.rs` (`MAX_AGENTS = 8`; role text for experts), `tests/test_team_expert.py` (new)
- Test: Rust unit tests in `team.rs`

**Interfaces:**
- Request file gains `"expert": id`, `"expert_name"`, `"expert_text"`, `"writes": bool` (filled by `rbxteam.spawn` from `rbxroster.expert`; the app does not read the roster itself).
- App: helper started with role string `"expert:<id>"`; its chat title is `"<Expert name>: <name>"` (34 chars as today); prompt = `WORKER_PROMPT` or `CRITIC_PROMPT` (by the request's `role`, which rbxteam sets from the roster `kind`) + `expert_text`. Result file gains `"expert"`.
- Writer lock: `team.rs` keeps the set of running writer helpers per project; a second `writes: true` request for the same project is refused with reason `"Another expert is changing the game right now (<name>). Wait for it, then start this one."`; released in `finished()`.
- When a studio Lead's spawn is refused because all agent slots are in use, the app sets `"note": "slots"` in that project's `studio.json` (cleared on its next successful spawn); Task 6 shows it as "Waiting for a free agent slot". Add a Rust test `slots_refusal_sets_note`.
- Produces for Task 6: `Agent::expert() -> Option<String>` (the id after `expert:`), and the BUS `agent_new` event carries `"expert": name`.

- [ ] **Step 1: Failing tests.** Python: `test_spawn_writes_expert_fields` (patch the request dir; spawn with `expert="coder"` → request JSON has `expert`, `expert_text` non-empty, `writes: true`, `role: "worker"`; `expert="security"` → `role: "critic"`); `test_unknown_expert_refused` (clear error, nothing written). Rust: `second_writer_is_refused` and `writer_lock_released_on_finish` on a pure function `writer_gate(&mut HashMap<String, String>, project, agent_name, writes) -> Result<(), String>`; `expert_title` (name + task title, truncated to 34 chars).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_team_expert -v` and `cargo test --manifest-path app-rs/Cargo.toml` → FAIL.
- [ ] **Step 3: Implement.** Without `expert`, behaviour is byte-for-byte as today. Refusals are written to the result file like the existing ones.
- [ ] **Step 4: Run** both → PASS; full `python3 -m unittest discover tests` once.
- [ ] **Step 5: Commit** `studio: named experts through rbx_team, one writer at a time`

### Task 4: The Studio run mode (prompt, waiting state, kickoff)

**Files:**
- Create: `app-rs/src/text/studio.txt`, `app-rs/src/text/kickoff_studio.txt`, `skills/studio/SKILL.md`
- Modify: `app-rs/src/agents.rs` (`Ask.studio: bool`; loop), `rbxkit_mcp.py` (GUIDES `studio`), `routing.txt` (one line: when Studio mode is on, load `studio` and `producer`)

**Interfaces:**
- `Ask { studio: bool, .. }`: implies `full` and the overnight loop (`overnight_until` set as for overnight). Prompt adds `AUTOPILOT_PROMPT`, `OVERNIGHT_PROMPT`, then `STUDIO_PROMPT`.
- New last-line status `STATUS: WAITING` (studio runs only): the app sends no carry-on, sets `st.studio_waiting = true`, stops the clock (store `overnight_left` seconds), and puts the chat's board card in `needs` with note "Waiting for your answer in Review". Any later user message or Task 5 decision message resumes: clock = now + `overnight_left` reset to the full 24 h, `studio_waiting = false`.
- Pure function for tests: `fn next_step(mode: Mode, got_result: bool, said: &str, state: &mut LoopState) -> Step` where `Step` is `CarryOn(&'static str)` | `Wait` | `End(&'static str)` | `Idle`; the existing overnight/autopilot branches move into it unchanged.
- Refused spawn text for the Lead (in `studio.txt`): wait with `rbx_team wait` on what is running, then retry; never drop a task.

- [ ] **Step 1: Failing Rust tests** on `next_step`: overnight behaviour unchanged (carry on; DONE twice ends; three failures end; NEEDS YOU carries on); studio + `STATUS: WAITING` → `Wait` and counters reset; studio + `STATUS: DONE` twice → `End`; non-studio + `STATUS: WAITING` → treated as an ordinary line (`CarryOn` in overnight, `Idle` otherwise).
- [ ] **Step 2: Run** `cargo test` → FAIL.
- [ ] **Step 3: Implement** the refactor and the state. Write `studio.txt` (the Lead's rules from the spec: follow the stages with `rbx_stage`; hand each part to the right expert with `rbx_team` `expert`; at most `max_experts` at once and one writer; check every report yourself; file a review item for anything the user can see, with the picture or clip; call `rbx_stage decisions` at every step and send each "changes" back to a fresh expert of the same kind with the note and the previous report, filing the result with `replaces`; a gated stage needs its blocking item approved; when everything left depends on the user, end with `STATUS: WAITING`; read `lessons.md` files at the start and add a lesson at each stage end; never publish). `kickoff_studio.txt` covers the three starts in the spec's call 7. `skills/studio/SKILL.md` is the longer how-to the prompt points at (briefing template, review item checklist, the stage definitions and what each must produce).
- [ ] **Step 4: Run** `cargo test`, `cargo build` (no new warnings) → PASS.
- [ ] **Step 5: Commit** `studio: Studio run mode, waiting at a gate, kickoff`

### Task 5: App side of runs and reviews

**Files:**
- Create: `app-rs/src/studio_run.rs`
- Modify: `app-rs/src/server.rs` (routes only), `app-rs/src/main.rs` or wherever watchers start (`thread::spawn(studio_run::watch)`), `app-rs/src/agents.rs` (a `pub fn wake_studio(&self, text: &str)`), `app-rs/src/media.rs` only if a helper is missing

**Interfaces:**
- `GET /api/review?path=<project>` → `{ok, run: <studio.json or null>, items: [...], needs: [board needs cards]}` (items newest first; `superseded` omitted).
- `POST /api/review {path, action: "approve"|"change"|"approve_all", id?, note?}` → `{ok, error?}`. `change` needs a non-empty note (≤ 600 chars). A decision on an item whose status is not `waiting` → `{ok:false, error:"Already answered."}` (decide under a lock; compare-and-set on the file).
- `POST /api/studio-run {path, action: "start"|"pause"|"resume"|"settings", idea?, gates?, max_experts?}` → `{ok, agent?}`. `start` writes a fresh `studio.json` (refuses when a run is `on` and not finished), starts or reuses the project's chat as the Lead with `Ask{studio:true}` and the kickoff; `resume` starts a new Lead chat when `lead` is not a live agent and writes its id; `settings` validates gates ⊆ STAGES and 1 ≤ max_experts ≤ 4.
- `GET /media/review/<key>/<item id>/<file>?t=` where `key` = first 12 hex chars of SHA-1 of the project path, resolved against the app's known projects; file name must be one listed in that item's `media`. Range support through `media.rs`.
- After a decision the app sends the Lead `wake_studio("Review: <title> — approved")` or `"… — change: <note>"`; if the Lead is busy the text is queued like any user message; if there is no live Lead the run is marked `paused`.
- Watcher: polls each known project's `.fridae/review/` and `studio.json` (id + mtime, like `promo.rs`), and emits BUS `review` = `{project, item}` and `studio_run` = `{project, run}`.

- [ ] **Step 1: Failing Rust tests** (temp project dirs with a space and "é"): `list_newest_first_skips_superseded`; `decide_once` (second approve → "Already answered."); `change_needs_note`; `media_confined` (file not in `media`, `../x`, unknown key, unknown item → None); `settings_validated`; `project_key_stable`; `start_refuses_running_run`.
- [ ] **Step 2: Run** `cargo test` → FAIL.
- [ ] **Step 3: Implement.** Reuse `promo.rs` helpers (`valid_id`, watcher shape) and `media.rs`.
- [ ] **Step 4: Run** `cargo test`, `cargo build` → PASS; by hand with curl + token against a hand-written run and two items (one with a png): list, approve, change, media 200/206/404.
- [ ] **Step 5: Commit** `studio: review and run routes, watcher, decisions wake the Lead`

### Task 6: The Review tab and starting a run

**Files:**
- Modify: `ui/index.html`

**Interfaces:**
- Consumes Task 5's routes and BUS events; Task 3's `agent_new.expert`.

- [ ] **Step 1: Build** (no JS test harness exists; verify in step 2):
  - Top-bar tab **Review** after Board, with a count of `waiting` items across games (hidden when 0).
  - Per game (a game picker when there are several, like the Board): the stage strip (six stages; done / current / to do); a run line ("Working", "Waiting for you", "Paused", "Waiting for a free agent slot", "Finished") with **Pause** / **Resume**; **Needs your answer** (blocking items + board needs cards); **To look at** (non-blocking items, newest first) with **Approve all**.
  - Item card: expert name, title, summary, media (images inline; video with controls; several as a row), **Approve**, **Change** (reveals a one-line box and Send). After a decision the card shows what was decided and leaves the list.
  - Run settings (a small disclosure): gate toggles per stage, experts at once (1–4).
  - Run-mode menu under the message box gains **Studio** with one line of help; selecting it on a chat with a project and pressing send starts the run through `/api/studio-run start` with the message as the idea. Disabled with a one-line reason for chats with no project or an AI where helpers don't work.
  - Research: "Build it on autopilot" becomes "Start a studio run".
  - Sidebar and Overview show an expert's name instead of "helper"; a review item filed from a chat shows there as one line with "Open in Review".
  - "Undo the night" reads "Undo this run" for studio runs (behaviour is Task 7).
- [ ] **Step 2: Verify by hand** in the dev app against a scratch work folder and a scratch project with hand-written `studio.json` and review items: every state above at ~1400px and ~900px; approve and change round-trip (files change, cards leave); an item added on disk appears without reload; the count updates.
- [ ] **Step 3: Commit** `studio: Review tab, Studio mode in the menu, experts by name`

### Task 7: Checkpoints per stage, undo, pause and resume

**Files:**
- Modify: `app-rs/src/agents.rs` (checkpoint/undo), `app-rs/src/studio_run.rs`, `rbxstage.py` (`stage done` asks the app for a checkpoint), `ui/index.html` (undo picker)

**Interfaces:**
- Checkpoint request: `rbxstage` writes `WORK/stage-requests/<id>.json` `{project, stage, lead}`; the app (watcher in `studio_run.rs`) takes the same checkpoint Overnight takes at its start and appends `{stage, commit, studio, at}` to `studio.json` `"checkpoints"`.
- Experts' Studio steps count toward the run: a helper whose parent is a studio Lead appends its step log to the Lead's run undo log (extend whatever `undo_night` reads; name the function `run_log(&self) -> Vec<Value>`).
- `POST /api/studio-run {action:"undo", path, stage?}` → undoes back to that stage's checkpoint (default: the latest), with the same refusals and report text style as `undo_night`.
- Pause: `studio.json` `paused: true`; the Lead's next carry-on is replaced by one message telling it to let running experts finish, start nothing new and end with `STATUS: WAITING`. Resume clears it and wakes the Lead (or adopts, Task 5).

- [ ] **Step 1: Failing Rust tests:** `checkpoint_appended_per_stage`; `undo_default_is_latest`; `undo_unknown_stage_refused`; `helper_steps_join_run_log` (pure function merging logs by time); `pause_replaces_carry_on` (on `next_step`'s caller-level helper).
- [ ] **Step 2: Run** `cargo test` → FAIL.
- [ ] **Step 3: Implement.** Add a Python test `test_stage_done_requests_checkpoint` (request file written with the stage).
- [ ] **Step 4: Run** all tests → PASS. By hand: start a run on a scratch git project without Studio connected; finish two stages by writing files as the agent would; Undo this run → latest stage restored, and choosing the earlier one restores further.
- [ ] **Step 5: Commit** `studio: stage checkpoints, undo this run, pause and resume`

### Task 8: End-to-end dry run, docs, packaging

**Files:**
- Create: `tests/test_studio_flow.py`
- Modify: `README.md` (a "Studio mode" section), `DEVELOPING.md`, `THIRD_PARTY.md` only if anything was added, the in-app Guide entries in `ui/index.html`, `rbxkit.spec`/build scripts if `team/` or `skills/studio` need listing

- [ ] **Step 1: Write `tests/test_studio_flow.py`**: drives the Python side as a Lead would with stub experts (no AI): start design → file a blocking review item → `done` refused → approve by editing the file → `done` ok → build stage with two items, one `changes` → `decisions` returns it once → file a replacement with `replaces` → final `status` shows design done, build doing, one superseded item hidden from open items.
- [ ] **Step 2: Run** `python3 -m unittest tests.test_studio_flow -v` → PASS (fix whatever it finds).
- [ ] **Step 3: Docs.** README section: what Studio mode is, how to start a run, the two default gates, Review, Change, Pause, Undo this run, that it runs on the user's own AI plan and how to lower "experts at once", how to edit an expert or the roster, and that Fridae never publishes. Guide entries for Review and Studio.
- [ ] **Step 4: Packaged check on macOS:** `.venv/bin/python build_mac.py`; confirm `team/` and `skills/studio` are inside the app; launch from `build-out` with a scratch work folder; start a real studio run on a scratch project with a one-line idea; confirm the Lead starts a Designer by name, a design review item appears, Approve wakes the Lead, and Pause stops new work. Record exactly what was and was not seen.
- [ ] **Step 5: Run everything:** `.venv/bin/python -m unittest discover tests`, `cargo test --manifest-path app-rs/Cargo.toml`, node tests → PASS.
- [ ] **Step 6: Commit** `studio: end-to-end dry run, docs, packaging`
