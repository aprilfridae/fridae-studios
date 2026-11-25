# Universal Skills, Phase 1 (one local library) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every AI in Fridae uses one library of skills (today's guides included). About a dozen skills are pinned
in its instructions, the rest are found by search and suggested each turn, and overlapping skills or notes wait for
the user's Keep / Replace / Merge / Keep both on an AI's recommendation.

**Architecture:**
- **Python** (`rbxskills.py`) owns the library: where skills live, front matter, import from other tools' folders,
  the overlap check and the decision store. It serves `rbx_skill` on the tool server and the `rbx skills ...` CLI.
- **The search** (`rbxvault.py` keyword, `rbxrag` index) gains a `skills` shelf with one entry per skill.
- **Rust** (`skills.rs`) builds each agent's pinned list and the per-turn suggestion line, asks a cheap model for
  recommendations (`compact.rs`), and serves `/api/skills` to the page.
- **The page** (`ui/index.html`) gets a Setup › Skills section.

**Tech Stack:** Python 3 (stdlib, unittest), Rust (app-rs, cargo test), one-file HTML/JS UI (`node --check`).

**Spec:** `docs/superpowers/specs/2026-11-24-universal-skills-design.md` (approved 2026-11-24). Read "Rulings" below:
three spec points are settled differently or more exactly here.

**Where:** worktree `/Users/angela/fridae-skills`, branch `feature/shared-skills`. It builds on `fb64721` (the
`rbx_skill` tool and catalog).

## Rulings (decided while planning; recorded here because the spec says otherwise or says less)

1. **Claude's account folders are not rebuilt.**
   - Fridae runs Claude on the user's main account with the user's own `~/.claude` (no `CLAUDE_CONFIG_DIR`;
     `accounts.rs` only makes folders for second accounts), so a Fridae-built skills folder cannot reach the main
     account without touching `~/.claude`.
   - Instead, for Claude, pins and suggestions leave out a library skill whose copy is the unchanged import of one
     Claude Code already lists for that account (`rbxskills.claude_native`, as on the branch).
   - Merged or replaced winners are pinned and suggested with the precedence line (Task 6).
   - The spec's "builds that folder's skills from the library" is dropped. Cost if wrong: Claude may see a plugin's
     older copy beside Fridae's winner, as the spec already allows.
2. **Guides stay in their folders.**
   - Each `<name>/<name>.md` is renamed to `<name>/SKILL.md` in place, gains the skill front matter, and has its owner
     set there. The folders stay: `vault/Fridae/Guides/Core`, `Roblox subjects`, `LihFolk`.
   - The spec's move to `vault/Skills/` waits for the unified vault (phase 3), to avoid churning every path the Rust
     code, `routing.txt` and tests use. Cost if wrong: one `git mv` later.
3. **"Review card" means Setup › Skills.**
   - Review is per project (`studio_run`). Skill and note decisions are for the whole library, so they live in a new
     Setup section, with a line on the Overview page when any wait.
   - Cost if wrong: the cards move; the API is the same.

## Global Constraints

- Never write to the user's `~/.claude`, `~/.gemini`, `~/.cursor`, `~/.config/opencode` or `~/.local/share/opencode`.
  Fridae reads them only.
- The user's own skills live in `os.path.join(rbxpaths.HOME_DIR, "skills")` (`~/Documents/Fridae Studios/skills/` when
  packaged).
- Skill front matter fields, exactly: `name`, `description`, `examples`, `applies` (`game` | `plain` | `both`, default
  `both`), `core` (bool), `owner` (`app` | `shipped` | `community` | `user`), `source`, `licence`, `supersedes`,
  `updated`.
- `owner: app` skills (workflow, studio, critic, tester, producer, promo) keep their name, are never replaced, merged,
  edited or submitted, and ship read-only inside the app.
- The seven `APP_ONLY` skills are never imported or listed.
- A project's own skills (`.claude/skills`, `.agents/skills`) are listed for that project only, never copied into the
  library.
- At most 3 skills per turn are suggested, as one line: `Skills that may fit: a (why), b (why)`.
- A skill that loses is never deleted: it moves to `<user skills>/.replaced/<name>/`, and the winner gains
  `supersedes: [<name>]`.
- Recommendations come from the user's OpenCode plan first (DeepSeek/GLM), then the user's default AI. With neither,
  the pair shows with no recommendation. Never a pay-per-use model.
- Builders and tests never contact github.com and never install Ollama or Qdrant.
- Test commands: `.venv/bin/python -m unittest tests.test_skills` (plus any file named in a task);
  `cargo test --manifest-path app-rs/Cargo.toml <filter>`. UI: extract the `<script>` blocks and `node --check` them.
- Commit messages end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Commit on
  `feature/shared-skills` only; never push or merge.

## Review Focus

1. **A skill folder with broken or missing front matter** (no `name`, bad YAML, a binary `SKILL.md`): it is listed
   under its folder name with "(no description)" and never breaks list, search or the catalog. Test in Task 1.
2. **Two imports with the same name but different content** (the user's `brainstorming` and a plugin's): one
   decision, never two library entries and never a silent overwrite. Test in Task 4.
3. **A source skill deleted after import:** the library copy stays and is still served. Its import link is marked
   gone, and it is not re-imported or flagged. Test in Task 4.
4. **The index service down or still building** when a turn starts: suggestions come from the Rust keyword match
   within 50 ms, and the turn is never delayed past that. Test in Task 6.
5. **A merge recommendation for an `owner: app` skill:** refused by the API, and never offered by the page. Test in
   Task 5.

---

### Task 0: Bring the branch up to date

**Files:** none changed by hand.

- [ ] **Step 1:** In `/Users/angela/fridae-skills`, run `git merge --no-edit feature/ui-overhaul`. Expected: a clean
  merge (it brings `5ec8ee6` and the spec commit `0c45ec2`). If it conflicts, stop and report.
- [ ] **Step 2:** Run `.venv/bin/python -m unittest tests.test_skills` and
  `cargo test --manifest-path app-rs/Cargo.toml skills::`. Expected: all pass.

### Task 1: The library model

**Files:**
- Modify: `rbxskills.py`
- Test: `tests/test_skills.py`

**Interfaces:**
- Produces:
  - `KIT_ROOTS: list[tuple[str, str]]`: `(folder under KIT, default owner)`, namely
    `("vault/Fridae/Guides/Core", "shipped")`, `("vault/Fridae/Guides/Roblox subjects", "shipped")`,
    `("vault/Fridae/Guides/LihFolk", "shipped")`.
  - `user_dir() -> str`.
  - `meta(fm: dict, folder: str, default_owner: str) -> dict` with every Global Constraints field filled (lists for
    `examples` and `supersedes`).
  - `library(project: str | None = None) -> list[dict]`: each skill is
    `{id, name, description, examples, applies, core, owner, source, licence, supersedes, updated, dir, origin}`.
    - `origin` is one of `"app"`, `"shipped"`, `"user"`, `"project"`.
    - Order and precedence: app, then shipped, then user, then project. One entry per id. An `owner: app` name can
      never be taken by a later root.
  - `skills(project)` keeps its name and returns `library(project)`. `find`, `load` and `read_file` use it.
  - External folders (`_roots` minus project) are no longer listed directly. Task 4 imports them.

- [ ] **Step 1: Write the failing tests.** Use temporary folders: patch `KIT`, `user_dir` and `HOME`.
  - `test_kit_guides_are_skills_with_owner_from_front_matter`: a kit folder `Core/critic/SKILL.md` with `owner: app`
    is listed with `owner == "app"`; one without `owner` gets `"shipped"`.
  - `test_app_owned_name_cannot_be_taken`: a user skill `critic` beside an app `critic` gives one entry, the app's.
  - `test_broken_front_matter_is_listed_by_folder_name`: `SKILL.md` = `"---\nname: [oops\n"` gives
    `name == folder name` and `description == ""`; `catalog()` and `listing()` do not raise.
  - `test_meta_defaults`:
    `meta({}, "x", "user") == {... "applies": "both", "core": False, "owner": "user", "examples": [], "supersedes": [] ...}`.
- [ ] **Step 2:** Run `.venv/bin/python -m unittest tests.test_skills -k library -k owner -k broken -k meta`.
  Expected: FAIL (`library` not defined).
- [ ] **Step 3:** Implement the interfaces in `rbxskills.py`. `front()` already parses simple YAML; extend it for
  lists (`examples:` followed by `- item` lines, and `[a, b]`).
- [ ] **Step 4:** Run `.venv/bin/python -m unittest tests.test_skills`. Expected: PASS. Update the branch's older
  tests that relied on external roots being listed directly so they go through Task 4's import. Until Task 4, mark
  them `@unittest.skip("imported in Task 4")`.
- [ ] **Step 5:** Commit: `Skills: one library (app, shipped, user, project) with the skill front matter`.

### Task 2: Guides become skills; `rbx_skill load` does what `rbx_guide` did

**Files:**
- Rename: every `vault/Fridae/Guides/{Core,Roblox subjects,LihFolk}/<name>/<name>.md` to `<name>/SKILL.md`
  (`git mv`).
- Modify: each renamed file's front matter.
- Modify: `rbxkit_mcp.py` (`GUIDES`, `_register_roblox_skills`, `_guide_path`, `_one_guide`, `rbx_guide`, `rbx_skill`).
- Modify: `rbxvault.py` (`guide_files`).
- Modify: `install.py`, if it copies guide files by name.
- Test: `tests/test_skills.py`, plus the existing guide tests in `tests/` (run `grep -ln rbx_guide tests/*.py`).

**Interfaces:**
- Consumes: `library()` (Task 1).
- Produces:
  - `GUIDES` is now built from `library()`, mapping name to `(relative path, description)`.
  - Non-skill entries stay as `GUIDE_FILES = {"ui-helpers": ..., "ui-audit": ..., "tester": ...}` and resolve as
    before.
  - `rbx_skill` actions: `load` takes `name` or `names` (several) and an optional `section`, and returns an outline
    when the skill is over the existing 12 KB rule. `name/ref` reads `references/ref.md`. `GUIDE_CAP`, the web-guide
    note and the LihFolk credit apply to every skill.
  - `rbx_guide` keeps its arguments (`topic`, `topics`, `section`) and calls the same code.

- [ ] **Step 1:** Front matter for each renamed guide:
  - `name` (unchanged), `description` (the `GUIDES` description), `examples` (3 to 5 tasks it is for), `applies`
    (Core design and style guides `both`; game and Roblox guides `game`).
  - `owner: app` for workflow, studio, critic, tester, producer and promo.
  - `core: true` for the Task 6 pinned set.
  - LihFolk guides: `source: LihFolk Vault V2`, `licence: CC BY 4.0`.
- [ ] **Step 2: Write the failing tests.**
  - `test_every_old_guide_name_still_loads`: for each name in the old `GUIDES` keys (frozen as a list in the test),
    `rbx_guide topic=<name>` and `rbx_skill action=load name=<name>` return the same text.
  - `test_load_section_and_outline`: `load name=workflow section=<a heading>` returns only that section; a skill over
    12 KB without `section` returns its outline.
  - `test_reference_topic`: `rbx_skill load name=roblox-luau/type-system` equals today's `rbx_guide` answer for it.
- [ ] **Step 3:** Run the new tests and the existing guide tests. Expected: the new ones FAIL.
- [ ] **Step 4:** Implement. Fix every path that named `<name>/<name>.md`: run
  `grep -rn "Guides/.*\.md\|<name>.md" app-rs/src rbx*.py routing.txt install.py rbxkit.spec`.
- [ ] **Step 5:** Run `.venv/bin/python -m unittest tests.test_skills` and every guide test file found in Step 2's
  grep, then `cargo test --manifest-path app-rs/Cargo.toml`. Expected: PASS.
- [ ] **Step 6:** Commit: `Guides are skills: SKILL.md in place, rbx_guide an alias of rbx_skill load`.

### Task 3: The `skills` search shelf

**Files:**
- Modify: `rbxvault.py`
  - `SHELVES`, `SHIPPED`;
  - a new `skill_docs(project)`;
  - `search_sections` / `combined_search`;
  - `rag_search` (the shelf name passes through);
  - `_kit_files`: skills are no longer indexed as `guides` body chunks.
- Modify: `rbxrag/chunks.py` (`corpus_files`, `chunks_for_file`).
- Modify: `rbxkit_mcp.py` (`rbx_skill` action `search`; `rbx_vault` search results add skill pointers).
- Test: `tests/test_skills.py`, `tests/test_rag_vault.py`.

**Interfaces:**
- Consumes: `library()`.
- Produces:
  - `rbxvault.skill_docs(project=None) -> list[dict]`: one doc per skill, with
    `{shelf: "skills", title: id, text: "<name>\n<description>\nFor: <examples joined by '; '>", applies, owner}`.
  - `rbxskills.search(q: str, kind: str = "", k: int = 5, project=None) -> list[dict]`: `[{id, description, why,
    score}]`.
    - It uses the index service (`rag_endpoint`) when it is running, else the keyword search over `skill_docs`.
    - It drops skills whose `applies` excludes `kind` (`"game"` / `"plain"`).
    - `why` is the matched example or description words, at most 60 characters.
  - The RAG chunk for a skill is that same text as one chunk: shelf `skills`, never body chunks.
  - `rbx_skill search` answers `- <id>: <description> (load with rbx_skill load <id>)` lines.

- [ ] **Step 1: Write the failing tests.**
  - `test_search_finds_skill_by_example`: a temporary user skill `menu-polish` with example `make a shop menu feel
    juicy` is first for `q="shop menu feels flat"`.
  - `test_search_respects_kind`: a skill with `applies: game` is absent for `kind="plain"`.
  - `test_skill_chunk_is_one_summary`: `chunks_for_file("skills", ...)` for a 40 KB `SKILL.md` yields exactly 1 chunk
    whose text starts with the name.
  - `test_vault_search_points_at_skill`: `rbx_vault search` for a skill's example lists
    `rbx_skill load <id>` in its answer.
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `.venv/bin/python -m unittest tests.test_skills tests.test_rag_vault`. Expected: PASS.
- [ ] **Step 5:** Commit: `Skills: a search shelf of one entry per skill, rbx_skill search, pointers from rbx_vault`.

### Task 4: Import, the overlap check and the decision store

**Files:**
- Modify: `rbxskills.py`; `rbx.py` (`register` gains the subcommands).
- Test: `tests/test_skills.py`.

**Interfaces:**
- Consumes: `library()`, `search()`, `_roots()` (the external sources from `fb64721`).
- Produces:
  - `folder_hash(d: str) -> str`: sha256 over sorted relative paths and contents, ignoring `.DS_Store` and dot-files.
  - `import_external() -> dict`. For each external skill (not `APP_ONLY`, not project):
    - **already linked** (`<user_dir>/<name>/.fridae-source.json` holds `{source, hash, imported, gone}`): when the
      source changed and the copy is unedited (the copy's hash still equals the recorded hash), refresh it.
      When both changed, save the new one beside it as `<name> (update)` and add a decision.
    - **identical to an existing skill** (same hash): record the link only.
    - **new and overlapping** (`similar() >= OVERLAP`) with an existing skill: add a decision and do not copy it into
      the library yet. Keep it in `<user_dir>/.incoming/<name>/`.
    - **otherwise:** copy it in.
    - A source that disappeared sets `gone: true` and leaves the copy.
    - Returns `{"added": [...], "linked": [...], "refreshed": [...], "pending": [...]}`.
  - `similar(a: dict, b: dict) -> float` in 0..1: the larger of name similarity and keyword cosine over the
    `skill_docs` text, or the index service's score when it runs. `OVERLAP = 0.55` is a starting value; Task 9 sets it.
  - Decision store `<user_dir>/.decisions.json`, a list of
    `{id, kind: "skill"|"note", a, b, a_dir, b_dir, made, recommendation: null|{choice, why, merged}, state: "open"}`.
    - `pending() -> list`.
    - `decide(id, choice, merged_text=None) -> dict`. `choice` is `keep` | `replace` | `merge` | `keep_both`.
      - The loser moves to `.replaced/<name>/`. The winner gains `supersedes`.
      - `merge` writes `merged_text` as the winner's `SKILL.md` and keeps the old one in `.replaced/`.
      - `keep_both` renames the newcomer `<name>-2`.
      - It refuses any choice that would change an `owner: app` skill, with the error "That is one of Fridae's own
        skills."
  - CLI: `rbx skills import`, `rbx skills pending --json`, `rbx skills decide <id> <choice> [--merged-file p]`,
    `rbx skills index --json` (the `library()` fields plus `claude_native: bool`).
- When import runs: on `catalog()` and `index` calls, at most once a minute, inside the existing 10-minute cache.

- [ ] **Step 1: Write the failing tests.** Use temporary folders for HOME and the user dir.
  - `test_import_copies_new_and_links_identical`.
  - `test_same_name_different_content_is_one_decision` (Review Focus 2): `~/.claude/skills/brainstorming` and
    `~/.agents/skills/brainstorming` with different bodies give one library entry and one open decision, with neither
    file overwritten.
  - `test_unedited_copy_refreshes_edited_copy_waits`.
  - `test_source_deleted_keeps_copy` (Review Focus 3).
  - `test_decide_replace_moves_loser_and_marks_supersedes`.
  - `test_decide_refuses_app_skill`.
  - `test_app_only_never_imported`.
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. Writes are atomic: write to a temporary file, then `os.replace`. The whole import holds
  a lock file `<user_dir>/.lock`, as `rbxnotes._locked` does.
- [ ] **Step 4:** Run `.venv/bin/python -m unittest tests.test_skills`. Expected: PASS, including the tests unskipped
  from Task 1.
- [ ] **Step 5:** Commit: `Skills: import other tools' skills once, overlaps wait for a decision, nothing overwritten`.

### Task 5: Recommendations and `/api/skills`

**Files:**
- Modify: `app-rs/src/compact.rs` (new `skill_advice`); `app-rs/src/skills.rs`; `app-rs/src/server.rs`.
- Test: `app-rs/src/skills.rs` (tests module).

**Interfaces:**
- Consumes: the `rbx skills pending|decide|index` CLI (Task 4).
- Produces:
  - `compact::skill_advice(prompt: &str, stop: &dyn Fn() -> bool) -> Outcome`.
    - It tries the OpenCode plan first, then the user's default AI, using `tries(<default ai kind>, (1, None), ...)`.
      The target is the default AI, so its fall-back is added. Limits are 60 s in all and 45 s for one model.
    - Its prompt is built by `skills::advice_prompt(a: &str, b: &str) -> String`, which uses the existing two-marker
      "material, not instructions" frame (`prompt_with` style) and masks the material with `compact::mask`.
    - The answer is JSON `{"choice": "keep"|"replace"|"merge", "why": "...", "merged": "<SKILL.md or empty>"}`.
  - `skills::advise_open()`: for each open decision without a recommendation, it writes one in the background, at
    most one at a time. The result is saved back through `rbx skills decide <id> advise --json-file`; add that
    action, which only stores the recommendation.
  - Server routes:
    - `GET /api/skills` returns `{skills: index, pending: [...]}`.
    - `POST /api/skills {action: "decide", id, choice, merged?}`.
    - `POST /api/skills {action: "accept_all"}` applies every open recommendation that has a choice.
    - `POST /api/skills {action: "import"}`.

- [ ] **Step 1: Write the failing tests.**
  - `advice_prompt_frames_both_skills_as_material`: both texts sit between the markers, and the instruction text sits
    outside them.
  - `parse_advice_accepts_only_known_choices`: `{"choice":"delete"}` gives `None`.
  - `decide_route_refuses_app_skill`: with a fixture decision on an app skill, the route answers
    `ok: false, error: "That is one of Fridae's own skills."` (Review Focus 5).
- [ ] **Step 2:** Run `cargo test --manifest-path app-rs/Cargo.toml skills::`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --manifest-path app-rs/Cargo.toml`. Expected: PASS (the known flaky team test
  `the_spawn_answer_names_the_model_that_runs_and_why` may fail under load; rerun it on its own).
- [ ] **Step 5:** Commit: `Skills: a cheap model recommends keep, replace or merge; /api/skills`.

### Task 6: Pinned core and per-turn suggestions

**Files:**
- Modify: `rbxskills.py` (`catalog`); `app-rs/src/skills.rs`; `app-rs/src/agents.rs` (beside
  `text = with_handover(...)` at the turn's send, and where `skills::catalog` is added).
- Test: `tests/test_skills.py`, `app-rs/src/skills.rs` tests.

**Interfaces:**
- Consumes: `library()`, `search()` (Task 3), `rbx skills index --json` (Task 4).
- Produces:
  - `catalog(ai, project, config, kind)` lists only `core: true` skills whose `applies` fits `kind`, leaving out (for
    Claude) unchanged imports of skills Claude Code lists itself. It ends with exactly these lines:
    - `Other skills: search with rbx_skill (action search) when a task needs know-how you do not have; the app also suggests matching skills on your messages.`
    - for Claude only: `When a Fridae skill (rbx_skill) and a Claude Code skill cover the same thing, use the Fridae one.`
  - The pinned set: brainstorming, systematic-debugging, test-driven-development, verification-before-completion,
    writing-plans and ui-ux-pro-max (`both`); workflow, playtest, ui-design, game-bible, game-design and critic
    (`game`). A name missing from the library is skipped.
  - `skills::suggest(text: &str, kind: &str, ai: &str, already: &[String]) -> Option<String>` returns the suggestion
    line or `None`.
    - It asks the index service with `POST /query {q, shelves: ["skills"], k: 6}` and a 300 ms timeout. Otherwise, or
      on failure, it scores keywords in Rust over the cached index entries' text.
    - It keeps scores at or above `SUGGEST_MIN` (starting value 0.35; Task 9 sets it), at most 3, and drops pinned
      ones, ones in `already` and, for Claude, native ones.
    - Each `why` is at most 40 characters.
  - The agent keeps `loaded_skills: Vec<String>` in its state, filled when `rbx_skill load` succeeds in its chat. Use
    the same tool-event hook that already tracks `rbx_guide` loads, or add one beside it.
  - The line goes on the turn text sent to the AI, after the user's words, never into the stored user message.

- [ ] **Step 1: Write the failing tests.**
  - Python `test_catalog_is_core_only_and_kind_filtered`: with 20 skills of which 3 are core and `applies: game` on
    one, a plain catalog lists 2 skill lines plus the "Other skills" line.
  - Rust `suggest_falls_back_to_keywords_quickly` (Review Focus 4): with no endpoint file,
    `suggest("my shop menu looks flat", "game", "cursor", &[])` returns within 50 ms and names the fixture skill.
  - Rust `suggest_drops_pinned_and_loaded`.
  - Rust `suggestion_line_format`: `Skills that may fit: menu-polish (shop menu), ...`, at most 3 items.
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. Reuse `skills.rs`'s cache (`KEEP`, watched folders) for the index JSON, and add
  `user_dir()` and the kit roots to the watched folders.
- [ ] **Step 4:** Run `.venv/bin/python -m unittest tests.test_skills` and
  `cargo test --manifest-path app-rs/Cargo.toml`. Expected: PASS.
- [ ] **Step 5:** Commit: `Skills: a dozen pinned, the rest suggested each turn and found by search`.

### Task 7: Setup › Skills in the page

**Files:**
- Modify: `ui/index.html` (a new `<section data-s="skills">` in Setup, its nav entry, JS, and an Overview line).

**Interfaces:**
- Consumes: `/api/skills` (Task 5).
- Produces, in Setup › Skills:
  - **"Needs your decision"**: one card per open decision, showing each side's name, owner, source, description,
    whether it has scripts (listed first, with "runs on your Mac"), and the recommendation with its why.
    - Buttons: Keep, Replace, Merge (only when a merged text exists) and Keep both.
    - No button may change an `owner: app` skill.
    - With 2 or more cards: "Accept all recommendations".
  - **"Your skills"**: a filterable list (name, owner, applies, pinned mark).
  - **An Import now button.**
  - The Overview totals line gains `N skill decisions wait`, linking to Setup › Skills, when N > 0.
  - Note decisions (Task 8) show in the same list with "Note" as their kind.

- [ ] **Step 1:** Build it, following the existing Setup sections' markup and the review-card styles.
- [ ] **Step 2:** Extract the `<script>` blocks and run `node --check`. Expected: no errors.
- [ ] **Step 3:** Check it in the built app (Task 9 builds it): open Setup › Skills with a fixture decision.
- [ ] **Step 4:** Commit: `Setup, Skills: decisions with the recommendation, the library, import`.

### Task 8: Notes upkeep as decisions

**Files:**
- Modify: `rbxupkeep.py` (`sweep` results feed the store); `rbxskills.py` (decisions of `kind: "note"`); `app-rs/src/skills.rs` (weekly run); `ui/index.html` (a "Check notes now" button in Setup › Skills).
- Test: `tests/test_skills.py` (or the upkeep test file, if one exists: `ls tests | grep upkeep`).

**Interfaces:**
- Consumes: `rbxupkeep.sweep(projects=())`, its duplicate and conflict results, and the decision store (Task 4).
- Produces:
  - `rbxupkeep.decisions_from(res) -> list[dict]`: one note decision per duplicate pair and per conflicting pair, on
    the user's own notes only (not shipped ones and not game notes).
  - `rbx upkeep --decisions` adds them to the store, without duplicates by pair.
  - Note decisions use the same choices. `merge` writes the merged note and moves the other to the notes `.trash`,
    as `forget` does.
  - Rust runs it once a week while the app is idle (no busy agent), keeping the last run time in config
    `skills_upkeep_at`, and on the Setup button.

- [ ] **Step 1: Write the failing tests.**
  - `test_contradicting_notes_become_one_decision`: two user notes, "hit freeze 80-150 ms" and "hit freeze 66 ms",
    give one decision of kind `note`.
  - `test_sweep_decisions_not_repeated`.
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the Python tests and `cargo test ... skills::`. Expected: PASS.
- [ ] **Step 5:** Commit: `Notes upkeep: duplicates and contradictions become decisions with a recommendation`.

### Task 9: Quick check, thresholds, docs, build

**Files:**
- Create: `tests/skills_bench.py` and `tests/skills_bench_cases.json` (30 cases).
- Modify: `DEVELOPING.md` ("Skills for every AI").
- Modify: `rbxskills.py` and `skills.rs` (the final `OVERLAP` and `SUGGEST_MIN`).

- [ ] **Step 1:** Write 30 cases: 15 game and 15 plain, each `{q, kind, want}`, where `want` is the skill a good
  engineer would load. Use real skills in the library.
- [ ] **Step 2:** `skills_bench.py` reports top-3 hit rate for keyword and for the index service (when running), plus
  the token count of the instructions' skills part before (`fb64721`'s full catalog) and after (pinned). It runs in
  about 10 minutes at most; say so before running.
- [ ] **Step 3:** Choose `SUGGEST_MIN` as the lowest value that keeps wrong suggestions under 1 per 3 turns on the
  cases. Choose `OVERLAP` from the scores of known-duplicate pairs (the user's imports against shipped guides).
  Record both choices and the hit rate in `DEVELOPING.md`.
- [ ] **Step 4:** Run the full Python suite in the background (about 11 min) and the full Rust suite. Expected: PASS.
- [ ] **Step 5:** Run `.venv/bin/python build_mac.py`, then
  `tests/long_calls_proof.py "build-out/Fridae Studios.app/Contents/Resources/tools/fridae-cli" --mcp`. Expected:
  `PROOF HOLDS`. Do not install: installing is the controller's job, and only with the user awake and the app idle.
- [ ] **Step 6:** Commit: `Skills: the quick check, chosen thresholds, docs`.

Later plans (not this one): phase 2 (the starter library, licence by licence) and phase 3 (vault download and
community submissions for skills), as in the spec's build order.
