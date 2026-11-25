# One Folder, One Vault: Stage A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Roblox guides join the Fridae vault beside the skills: one Pull newest for all of `Knowledge/`, a Keep the vault's / Keep mine card for every clash, search reading `Knowledge/Roblox`, and sharing, the inbox and Publish carrying guides as well as skills.

**Architecture:** The phase 3 skills vault engine (`rbxvaultsync.py`, `vault_sync.rs`, `vault_git.rs`, `share.rs`, `vault_inbox.rs`) is widened, not copied. The fixed `ALLOWED_TOP` becomes the "what travels" rule on both sides. `apply()` handles a guide file per file and a skill as one folder, and a clash now opens a waiting choice kept in the base record (`clashes`) with the vault's copy in `.fridae-vault/clash/`, never a file beside the user's. A three-way merge written in Python (`rbxmerge.py`) replaces `git merge-file`. The app stops bundling `vault/Roblox`. Create uploads it once, and a last task, gated on the user's word, removes it from the code repo.

**Tech Stack:** Python 3 standard library (unittest, difflib, hashlib), Rust (serde_json, ureq via `github.rs`), the single-file page `ui/index.html`, PyInstaller (`rbxkit.spec`).

**Spec:** `docs/superpowers/specs/2026-11-25-one-folder-vault-design.md` (Stage A and "Settled after the builder check"). Background: `2026-11-24-unified-knowledge-vault-design.md`, `2026-11-24-universal-skills-design.md` (phase 3 "As built"), DEVELOPING.md "The Fridae vault: getting and sharing skills".

## Global Constraints

- Anti-goal: "Losing or overwriting a single user note, skill or edit, during the move or on any pull."
- "All-or-nothing per file; a bad or cut-off pull changes nothing."
- "Fridae never publishes, pushes, closes a submission or creates the vault repo without the user's click and confirm. Pull newest only downloads." Nothing commits to the vault repo without the maintainer's click and confirm either.
- Anti-goal: "A clash resolved silently. Clean combines are reported; clashes are always asked."
- Anti-goal: "Git conflict marks (`<<<<<<<`) ever appearing inside a user's note."
- Anti-goal: "Removing LihFolk's credit from his notes, or presenting the vault as his."
- Anti-goal: "Keys, tokens or sign-ins inside the portable folder or in anything shared."
- "One cross-platform codebase: macOS and Windows behave the same (paths, separators, reserved names, links)."
- "Tests never contact github.com and never install anything."
- "Plain English in the app; "computer", not "Mac"." Setup's existing styles only: no new colours, badges, emoji, uppercase labels or motion.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Python tests: `PYTHONPATH=. H:/fridae-venv/Scripts/python -m unittest tests.<module>`, run from the repo root in Git Bash.
- Rust tests: `RUSTUP_HOME=/h/rust/rustup CARGO_HOME=/h/rust/cargo PATH=/h/fridae-venv/Scripts:/h/rust/mingw64/bin:/h/rust/cargo/bin:$PATH CARGO_TARGET_DIR=H:/rust/fridae-mingw TMP=H:/rust/tmp TEMP=H:/rust/tmp cargo test --manifest-path app-rs/Cargo.toml -- <module>::`. Below this is written `$RUST cargo test ... -- <module>::`. **Never run the full Rust suite**: one test kills the process on Windows.
- Measured vault (2026-11-25): `vault/Roblox` holds 1,352 files and 20,948,267 bytes. 1,311 are `.md`; the rest are 11 `.py`, 9 `.fbx`, 9 `.glb`, 9 `.blend` and 3 `.json`. The largest is 993,180 bytes (`18 Rigs and Skeletons/Spider_8Leg/Spider_8Leg.fbx`), the longest relative path is 118 characters, and there are no images. `_tools/` and `_data/` hold 4 of those files (416,229 bytes).

## Review Focus

1. **Windows will not let Fridae replace one guide** (the file is open in another program, or its path is over 260 characters). Only that file is listed as not put in place, its base is not recorded, everything else is applied, and the next Pull newest tries it again. Test: Task 3 `test_one_refused_file_does_not_stop_the_rest`.
2. **A Pull newest cut off part way** (the app quit, the computer slept) after files were written but before the base record was saved. The next Pull finishes without a single clash card for the files already written. Test: Task 3 `test_cut_off_pull_finishes_without_cards`.
3. **A guide re-saved with Windows line endings (CRLF)** while the vault changed another line. It combines cleanly, the user's line endings are kept on their own lines, and no clash is opened. Test: Task 2 `test_crlf_local_lf_vault_combines`.
4. **A first Download onto a `Knowledge/Roblox` that already holds the guides** (copied in by hand, or kept from developer mode). Identical files are only recorded, with no card. Only files that really differ ask. Test: Task 3 `test_identical_local_copies_are_only_recorded`.
5. **Hundreds of clash cards at once.** The page stays usable: the first 20 cards are shown, then a "Show all N" button. Check: Task 12 step "300 clashes fixture".

## File map

| File | Responsibility in stage A |
|---|---|
| `rbxmerge.py` (new) | `merge3` (three-way line merge, no git) and `is_binary` (never combined) |
| `rbxvaultsync.py` | the travel rule, limits, `apply` over all of Knowledge, the clash store, `clashes()`, `decide_clash()` |
| `rbxskills.py` | CLI actions `vault-clashes`, `vault-clash`, `pack-knowledge`, `mark-shared --unit` |
| `rbxshare.py` | `knowledge_changes`, `pack_knowledge`, `mark_shared_unit`, `compare` on a note file |
| `rbxvault.py` | `KB` is `Knowledge/Roblox`, `API_DIR` is the app's own data, the "not downloaded" sentence |
| `vault/Fridae/Data/` (new) | `roblox_api.json`, `api_overrides.json`, `roblox_facts_2026.json` (moved from `vault/Roblox/_tools`, `_data`) |
| `build_datas.py` (new) | `vault_datas(here)`: what of `vault/` the build bundles (never `vault/Roblox`) |
| `rbxkit.spec`, `.gitignore`, `THIRD_PARTY.md`, `rbxkit_mcp.py` | stop bundling, ignore `/Knowledge/`, credits, the tool's description |
| `app-rs/src/vault_sync.rs` | the travel rule, limits, clash routes, Reveal anywhere in Knowledge |
| `app-rs/src/vault_git.rs` | `commit_tree` (large uploads), `ensure_fork` for the maintainer's own repo |
| `app-rs/src/vault_inbox.rs` | Create with the guides, the licence split, inbox and Publish for guides |
| `app-rs/src/share.rs` | batches of skills and guides |
| `app-rs/src/server.rs` | `GET /api/vault/clashes`, `POST /api/vault/clash`, `which` on reveal, `unit` on inbox_decide |
| `ui/index.html` | the Knowledge vault block, clash cards, Share and Submissions for guides |
| `tests/guides.py` (new) | where tests find real guide notes (checkout's `vault/Roblox`, then `Knowledge/Roblox`) |
| `DEVELOPING.md` | each task updates its own paragraph of "The Fridae vault" |

Hooks left for stages B and C:
- The travel rule already accepts `Taste/`, `Inbox/` and any topic folder. `knowledge_changes` walks every travelling folder, so new topic folders share with no change.
- `rbxvault.KB` is one constant: stage B widens it to all of Knowledge.
- The stage B overlap check for new vault notes goes at `_plan`'s "no base, no local file: add" branch.
- `KNOWLEDGE_DIR` follows `HOME_DIR`, so stage C's move carries the base record with it.

---

### Task 1: The "what travels" rule and the new limits, on both sides

**Files:**
- Modify: `rbxvaultsync.py:16-22` (constants), `:77-85` (`check_path`), `:168-177` (`_skill`, `_local_path`)
- Modify: `app-rs/src/vault_sync.rs:24-29`, `:71-79` (`check_path`), `:163-172` (`local_path`), `:363-369` (`run`'s same-release retain)
- Modify: `app-rs/src/vault_git.rs:21-22` (`RAW_MAX`), `:239-240` (`BLOB_MAX`)
- Modify: `app-rs/src/vault_inbox.rs:286-289` (`refresh_main` size and "skipped"), `:807-825` (`MANIFEST_*`, `check_limits`)
- Test: `tests/test_vaultsync.py`, `app-rs/src/vault_sync.rs` tests, `app-rs/src/vault_inbox.rs` tests

**Interfaces:**
- Produces (Python, `rbxvaultsync`): `check_path(p: str) -> bool`, `unit_of(p: str) -> str` (`"Skills/<name>"` for a skill's file, else `p`), `CODE_DATA = ("Roblox/_tools/", "Roblox/_data/")`, `MAX_FILES, MAX_FILE, MAX_TOTAL = 20_000, 8 << 20, 256 << 20`. `ALLOWED_TOP` is removed.
- Produces (Rust, `vault_sync`): `pub fn check_path(p: &str) -> bool`, `pub(crate) fn unit_of(p: &str) -> String`, `pub(crate) const MAX_FILES: usize = 20_000`, `pub(crate) const MAX_FILE: u64 = 8 << 20`, `pub(crate) const MAX_TOTAL: u64 = 256 << 20`. `vault_git::RAW_MAX` and `BLOB_MAX` equal `vault_sync::MAX_FILE`. `vault_inbox` uses the `vault_sync` limits and drops its own `MANIFEST_*`.

The rule, the same on both sides:
- `p` is a string with no leading `/` and at least 2 parts.
- Every part passes `seg_ok` / `_seg_ok`. These already refuse dot-started names, so `.fridae-vault`, `.obsidian` and `.trash` never travel.
- A path under `Skills/` has at least 3 parts.
- `p` does not start with a `CODE_DATA` prefix.
- So `House rules.md` and every other top-level file (`README.md`, `LICENSE.md`, `manifest.json`) never travel.

`_skill`/`local_path`/the same-release `declined`/`waiting` lookups apply only to paths whose first part is `Skills`, so a guide folder named like a declined or renamed skill is never touched by those rules.

- [ ] **Step 1: Write the failing tests (Python).** In `tests/test_vaultsync.py`:
  - Extend `test_bad_paths_refuse_everything`'s list with `"House rules.md"`, `"Glossary.md"`, `".fridae-vault/base.json"`, `"Roblox/.obsidian/x.json"`, `"Roblox/_tools/roblox_api.json"`, `"Roblox/_data/roblox_facts_2026.json"`, `"Roblox"`, `"Roblox/"`, and remove `"Notes/a.md"`.
  - Add `test_guides_and_topic_folders_travel`: `check_path` is True for `"Roblox/Glossary.md"`, `"Roblox/02 Luau Scripting/Buffers and Binary Data.md"`, `"Roblox/18 Rigs and Skeletons/Spider_8Leg/Spider_8Leg.fbx"`, `"Notes/a.md"`, `"Taste/Softer.md"` and `"Inbox/x.md"`. `unit_of("Skills/a/ref/x.md") == "Skills/a"` and `unit_of("Roblox/Glossary.md") == "Roblox/Glossary.md"`.
  - Add `test_renamed_and_declined_touch_only_skills`: `_local_path("Roblox/x/y.md", {"x": "z"}) == "Roblox/x/y.md"`, and `_skill("Roblox/x/y.md") == ""`.
  - Change `test_manifest_limits`: 20,001 files refused and 20,000 accepted; a file of `8 << 20` accepted and `(8 << 20) + 1` refused; 33 files of `8 << 20` refused (over 256 MiB in all) and 32 accepted.
- [ ] **Step 2: Write the failing tests (Rust).** In `vault_sync` tests:
  - `manifest_rejects_bad_paths_and_limits` gets the same bad and good lists and the same limits.
  - The `WINDOWS_BAD`/`WINDOWS_GOOD` tables gain `"Roblox/CON.md"`, `"Roblox/a:b.md"` (bad) and `"Roblox/con-tips.md"` (good) on both sides.
  - Add `the_same_release_ignores_declined_only_for_skills`: base `declined: ["x"]` and a manifest path `Roblox/x/y.md` that differs locally, so `run()` still fetches it.
  - In `vault_inbox` tests, `publish_refuses_what_the_app_would_reject` uses a 9 MiB file (refused) and an 8 MiB file (accepted).
- [ ] **Step 3: Run them and see them fail.** `PYTHONPATH=. H:/fridae-venv/Scripts/python -m unittest tests.test_vaultsync` and `$RUST cargo test ... -- vault_sync::` → FAIL (`Notes/a.md` accepted, `Roblox/Glossary.md` refused, 1 MiB limit).
- [ ] **Step 4: Implement the rule and limits** in both files with the signatures above.
  - Also change `vault_git::raw`'s too-big sentence to "The file {path} is bigger than 8 MB, so Fridae did not take it.".
  - Change `check_limits`' sentences to "The vault has more than 20,000 files, more than Fridae can carry in one release.", "{path}: over 8 MB" and "The vault is over 256 MB in all, more than Fridae can carry in one release.".
  - `refresh_main` mirrors a blob when `check_path(path)`, it is not a link and its size ≤ `MAX_FILE`. It lists as skipped a blob path that `check_path` accepts but that is a link or too big.
- [ ] **Step 5: Run the tests and see them pass.** The two commands above, plus `$RUST cargo test ... -- vault_inbox::` → PASS.
- [ ] **Step 6: Update DEVELOPING.md** ("Getting skills" paragraph): the path rule becomes "everything in Knowledge but top-level files, dot-folders and `Roblox/_tools/`, `Roblox/_data/`; under `Skills/`, inside a skill's folder", and the limits become 20,000 files, 8 MiB each, 256 MiB in all.
- [ ] **Step 7: Commit.** `git add rbxvaultsync.py app-rs/src/vault_sync.rs app-rs/src/vault_git.rs app-rs/src/vault_inbox.rs tests/test_vaultsync.py DEVELOPING.md && git commit` with "Vault: everything in Knowledge travels but House rules and dot-folders; limits fit the guides".

### Task 2: A three-way merge inside Fridae

**Files:**
- Create: `rbxmerge.py`
- Create: `tests/test_merge3.py`
- Modify: `rbxvaultsync.py:143-165` (`_merge` calls `rbxmerge.merge3`; `subprocess`, `tempfile` and `MERGE_MAX` leave this file)
- Modify: `tests/test_vaultsync.py` (delete `test_merge_runs_git_without_a_console_window`)
- Modify: `rbxkit.spec` (`"rbxmerge"` in `hidden`)

**Interfaces:**
- Produces: `rbxmerge.merge3(local: bytes, base: bytes, new: bytes) -> bytes | None`. It returns the clean merge, or None when the hunks clash, any side is over `MERGE_MAX = 1 << 20`, or any side is not UTF-8 text without NUL.
- Produces: `rbxmerge.is_binary(path: str, data: bytes) -> bool`. True when the extension (lower-cased) is in `BINARY_EXT = (".blend", ".fbx", ".glb", ".gltf", ".obj", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".tga", ".bmp", ".psd", ".tif", ".tiff", ".ico")`, or `data` holds `b"\0"`, or `data` is not UTF-8.

Algorithm (the signature does not determine it):
- Split each side into lines with `keepends`.
- Compare lines by their key: the line without its trailing `\r\n`/`\n`.
- Take the changed hunks of base→local and base→new from `difflib.SequenceMatcher(None, base_keys, side_keys, autojunk=False).get_opcodes()`. Each hunk is a base range `[i1, i2)` and the side's lines.
- Two hunks from different sides clash when their base ranges overlap or touch (`a.i1 <= b.i2 and b.i1 <= a.i2`), unless their replacement keys are equal. Equal replacements are taken once, from local.
- Rebuild by walking base: an unchanged base line is written as local's own bytes for that line (so local's line endings stay), a hunk is written as its side's lines.
- Fast paths: `local == base` returns `new`, `new == base` returns `local`, and `local == new` returns `local`.
- Never emit conflict marks.

- [ ] **Step 1: Write the failing tests** in `tests/test_merge3.py`, class `Merge3Tests`:
  - `test_disjoint_changes_combine`: `merge3(b"A\nb\nc\nd\n", b"a\nb\nc\nd\n", b"a\nb\nc\nD\n") == b"A\nb\nc\nD\n"`
  - `test_same_line_changed_both_ways_is_none`: `merge3(b"a\nmine\nc\n", b"a\nb\nc\n", b"a\ntheirs\nc\n") is None`
  - `test_identical_change_is_clean`: `merge3(b"a\nX\nc\n", b"a\nb\nc\n", b"a\nX\nc\n") == b"a\nX\nc\n"`
  - `test_adjacent_lines_are_none`: `merge3(b"A\nb\nc\n", b"a\nb\nc\n", b"a\nB\nc\n") is None`
  - `test_both_append_different_lines_is_none`: `merge3(b"a\nx\n", b"a\n", b"a\ny\n") is None`
  - `test_crlf_local_lf_vault_combines` (Review Focus 3): `merge3(b"a\r\nb\r\nc\r\nd\r\n", b"a\nb\nc\nd\n", b"a\nb\nc\nD\n") == b"a\r\nb\r\nc\r\nD\n"`
  - `test_no_conflict_marks_ever`: for the clashing inputs above, the result is None (never bytes holding `b"<<<<<<<"`)
  - `test_binary_and_big_are_none`: `merge3(b"a\0", b"a", b"b") is None`, and `merge3(x, x, y) is None` when `len(x) > 1 << 20`
  - `test_is_binary`: True for `("Rig.FBX", b"text")`, `("a.md", b"\xff\xfe")` and `("a.md", b"a\0b")`; False for `("a.md", b"plain")` and `("build.py", b"import bpy\n")`
- [ ] **Step 2: Run** `PYTHONPATH=. H:/fridae-venv/Scripts/python -m unittest tests.test_merge3` → FAIL (no module `rbxmerge`).
- [ ] **Step 3: Implement `rbxmerge.py`** with the two functions and constants above. Make `rbxvaultsync._merge(local, base, new)` return `None` when `base is None or local is None`, else `rbxmerge.merge3(local, base, new)`. Add `"rbxmerge"` to `rbxkit.spec`'s `hidden`.
- [ ] **Step 4: Run** `tests.test_merge3` and `tests.test_vaultsync` → PASS (`test_both_changed_clean_merge` still passes, with no git).
- [ ] **Step 5: Update DEVELOPING.md:** "The merge is `git merge-file`; on a computer without git ..." becomes "The merge is Fridae's own (`rbxmerge.merge3`), so no git is needed; changes that touch the same or neighbouring lines are a clash, never conflict marks."
- [ ] **Step 6: Commit** (`rbxmerge.py tests/test_merge3.py rbxvaultsync.py tests/test_vaultsync.py rbxkit.spec DEVELOPING.md`) with "Vault: a three-way merge of Fridae's own, so Pull newest needs no git".

### Task 3: `apply()` over all of Knowledge, with clashes that wait for a choice

**Files:**
- Modify: `rbxvaultsync.py`: `read_base` (`:34-52`), `_no_symlinks`/`_validate` (`:180-240`), `_write` (`:122-127`), `_apply` (`:328-492`). `BESIDE`, `NAME_MAX`, `_beside` and `_put_beside` are removed.
- Test: `tests/test_vaultsync.py`

**Interfaces:**
- Consumes: `check_path`, `unit_of` (Task 1), `rbxmerge.merge3`, `rbxmerge.is_binary` (Task 2).
- Produces:
  - `read_base()` gains `"clashes": {}`: `{unit: {"release": str, "kind": "file" | "binary" | "skill", "paths": [upstream paths], "made": "<ISO time>"}}`.
  - `clash_dir() -> str` (`<state_dir>/clash`): the vault's copy of each waiting unit at `clash/<upstream path>`.
  - `_plan(p: str, local: bytes | None, old: str | None, base_bytes: bytes | None, new: bytes) -> tuple[str, bytes | None]`. The actions are `"add"`, `"record"`, `"write"`, `"keep"`, `"deleted"`, `"merge"` (with the merged bytes) and `"clash"`.
  - `apply(staged, manifest) -> dict` with keys `release, added, updated, merged, removed, skipped_deleted, declined, incoming, settled, clashes, errors`. `clashes` holds the units waiting after this run, and `kept` is gone.

Rules (the ones the tests below pin):
- `_plan` is today's `one()` decision table. Where `one()` called `_put_beside`, it returns `"clash"`. When both sides changed and `is_binary(p, local) or is_binary(p, new)`, it returns `"clash"` without trying a merge.
- A non-skill path is its own unit.
- A skill's files are planned together first. If any plans `"clash"`, nothing of that skill is written: the whole staged folder is copied to `clash/Skills/<name>/` and one clash `Skills/<name>` of kind `"skill"` is opened. Otherwise every plan is carried out as today.
- Opening a clash never changes the user's file or the base record of that unit.
- A unit already in `clashes` is planned again on each apply, against its unchanged base:
  - When it no longer clashes, it is applied and its clash closed (the copy removed).
  - When it still clashes, its copy is replaced by the new release's and `release` is updated.
  - When the manifest no longer has it, its copy moves to `removed/<release>/` and the clash is closed.
- `_validate` checks each target against the real folder of its own top part (`Knowledge/<top>`), not `vault_dir()`. `_no_symlinks` refuses a link at `Knowledge/<top>` for every top in the manifest, and at `state_dir()`.
- The newcomer, declined, renamed and waiting logic runs only for `Skills/` paths.
- `_save_base` runs after each skill folder, after every 50 other files and at the end.
- `_write` renames with `rbxpaths.replace`, which retries a Windows "Access is denied" for 2 s. An `OSError` on one file goes to `errors` as `"<path>: <reason>"`. That file is not recorded, and the rest go on.

- [ ] **Step 1: Write the failing tests.** Rewrite these:
  - `test_both_changed_conflict_keeps_mine_saves_beside` becomes `test_both_changed_conflict_opens_a_clash`: the local file is unchanged, `r["clashes"] == [P]`, the clash copy holds `"a\ntheirs\nc\n"`, the base hash is still `sha("a\nb\nc\n")`, no `"(Fridae update)"` file exists anywhere under Knowledge, and `"kept" not in r`.
  - `test_new_upstream_file_where_user_has_one` becomes the same-name clash: `r["clashes"] == [P]` and `P` not in the base.
  - Delete `test_existing_beside_file_is_not_replaced` and `test_beside_named_manifest_path_is_refused`.

  Add:
  - `test_guides_are_added_updated_and_merged_like_skills`: `Roblox/Glossary.md` and `Roblox/02 Luau Scripting/Buffers.md` added, then each of the update, merge and keep cases.
  - `test_skill_clash_holds_the_whole_folder`: base has `SKILL.md` = `"a\nb\nc\n"` and `ref.md` = `"r1"`; local has `SKILL.md` = `"a\nmine\nc\n"` and `ref.md` = `"r1"`; new has `"a\ntheirs\nc\n"` and `"r2"`. Then local `ref.md` is still `"r1"`, `r["clashes"] == ["Skills/x"]`, both files are in `clash/Skills/x/`, and both base hashes are unchanged.
  - `test_binary_both_changed_is_a_clash_never_combined`: `Roblox/18 Rigs/R.fbx` with text bytes changed on both sides, so the result is a clash of kind `"binary"`.
  - `test_newer_release_settles_an_open_clash_when_it_combines`: release 2 makes the vault's side equal the user's, so the clash is closed and the copy gone.
  - `test_newer_release_refreshes_the_waiting_copy`.
  - `test_clash_gone_upstream_is_closed_and_set_aside`.
  - `test_identical_local_copies_are_only_recorded` (Review Focus 4): 3 local guides equal to the new ones and 1 different give `r["clashes"]` with only that one, and the 3 are in the base.
  - `test_one_refused_file_does_not_stop_the_rest` (Review Focus 1): patch `rbxpaths.replace` to raise `PermissionError` when its target ends with `B.md`. Then `r["errors"]` names `Roblox/B.md`, `Roblox/B.md` is not in the base, `Roblox/A.md` and `Roblox/C.md` are written and recorded, and a second apply without the patch writes `B.md`.
  - `test_cut_off_pull_finishes_without_cards` (Review Focus 2): apply 4 guides, then delete 2 of them from `base["files"]` and their base copies (as if the save never ran). Re-applying the same release gives `r["clashes"] == []` and all 4 in the base.
  - `test_code_data_and_house_rules_never_travel`: a manifest with `Roblox/_tools/x.json` raises `ValueError(BAD)`, and a local `House rules.md` is byte-for-byte unchanged after any apply.
  - `test_symlinked_roblox_folder_is_refused` (skipped where symlinks cannot be made, as the existing link tests are).
  - `test_base_is_saved_every_50_guides`: with `_save_base` wrapped by a counter, 120 guides save at most 4 times.
- [ ] **Step 2: Run** `tests.test_vaultsync` → the new tests FAIL.
- [ ] **Step 3: Implement** the rules above in `rbxvaultsync.py`.
- [ ] **Step 4: Run** `tests.test_vaultsync tests.test_skills tests.test_upkeep` → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("The update rules"):
  - Every travelling file follows the rules.
  - A skill is one unit, a guide one file.
  - A clash waits in `.fridae-vault/clash/` and `base.json`'s `clashes` until the user chooses.
  - Nothing is saved beside the user's file.
- [ ] **Step 6: Commit** (`rbxvaultsync.py tests/test_vaultsync.py DEVELOPING.md`) with "Vault: guides update like skills; a clash waits for the user's choice instead of a file beside theirs".

### Task 4: Listing and settling clashes (Keep the vault's / Keep mine)

**Files:**
- Modify: `rbxvaultsync.py` (new functions after `_apply`)
- Modify: `rbxskills.py:1633-1662` (`register`: actions `vault-clashes`, `vault-clash`; flags `--unit`, `--choice`), `:1722` (dispatch)
- Test: `tests/test_vaultsync.py` (new class `ClashTests(VaultSyncTests)`, reusing its helpers)

**Interfaces:**
- Consumes: `read_base()["clashes"]`, `clash_dir()` (Task 3), `_record_base`, `_local_path`, `rbxmerge.is_binary`.
- Produces:
  - `replaced_dir() -> str`: `<state_dir>/replaced`.
  - `NOT_WAITING = "That choice is not waiting any more."`
  - `clashes() -> list[dict]`. Each item is `{"unit", "kind", "title", "release", "made", "both_new": bool, "mine": {"path": local Knowledge-relative path, "changed": int | None, "size": int, "modified": float | None}, "vault": {"changed": int | None, "size": int}, "files": [skill-relative paths that differ] (skill only, else []), "show": {"mine": str, "vault": str}}`.
    - `changed` is the number of lines added or removed against the base copy (`difflib.ndiff` count of `+ ` and `- ` lines), summed over a skill's text files. It is None when there is no base (`both_new`) or the file is binary.
    - `show` holds the unified diffs base→mine and base→vault, or both texts when `both_new`. Each is cut to 300 lines with a last line `"… (N more lines)"`, and is `""` for binaries.
    - `title` is the file name without `.md`, or the skill's name.
  - `decide_clash(unit: str, choice: str) -> dict` with `choice in ("vault", "mine")`. It returns `{"unit", "choice", "replaced": "<Knowledge-relative folder>" | None}`, takes `rbxskills._locked(state_dir())`, raises `LookupError(NOT_WAITING)` when the unit has no open clash or its copy is missing, and raises `ValueError("Choose vault or mine.")` for any other choice.
    - `"vault"`: every file of the user's (for a skill, every file under its local folder) that differs from or is absent in the vault's copy moves to `replaced/<YYYYMMDD-HHMMSS>/<local path>`. Then the vault's files are written and recorded with `_record_base`. A skill folder ends up exactly the vault's.
    - `"mine"`: the user's files are untouched. Each file of the vault's copy is recorded with `_record_base`. This is the after-Keep-mine rule: the vault's version becomes the starting point.
    - Both choices remove the copy and the clash entry, then save the base.
  - CLI: `rbx skills vault-clashes --json` prints `clashes()` as JSON. `rbx skills vault-clash --unit <u> --choice vault|mine --json` prints `{"ok": true, ...decide_clash}` or `{"ok": false, "error": "<sentence>"}`.

- [ ] **Step 1: Write the failing tests** in `ClashTests`:
  - `test_keep_the_vaults_sets_mine_aside_and_records_the_base`: after `decide_clash(P, "vault")` the local file is the vault's text, a copy of the user's text sits under `replaced/*/Skills/x/SKILL.md`, the base hash is the vault's, and `clashes() == []`.
  - `test_keep_mine_makes_the_vaults_version_the_base_so_later_changes_combine`: base `"a\nb\nc\nd\n"`, local `"a\nmine\nc\nd\n"`, release 1 `"a\ntheirs\nc\nd\n"` gives a clash; `decide_clash(P, "mine")` sets the base hash to `sha(release 1)`; release 2 `"a\ntheirs\nc\nD\n"` gives `r["merged"] == [P]`, a local file of `"a\nmine\nc\nD\n"`, and `r["clashes"] == []`.
  - `test_keep_mine_is_not_asked_again_for_the_same_release`: re-applying release 1 gives no clash and the file unchanged.
  - `test_skill_decided_as_a_whole_folder`: with a user's extra `notes.md` in the skill, `"vault"` moves it to `replaced/` and the folder's listing equals the vault's.
  - `test_binary_clash_lists_name_size_and_date`: kind `"binary"`, `mine.size`/`vault.size` right, `mine.modified` a float, and both `show` values `""`.
  - `test_clash_listing_summarises_both_sides`: `mine.changed == 2`, `vault.changed == 2` for the one-line edits above, and `show["vault"]` contains `"+theirs"`.
  - `test_same_name_clash_is_both_new`: `both_new` True and both `changed` None.
  - `test_deciding_a_clash_that_is_not_waiting_says_so`: `LookupError` with `NOT_WAITING`; and `ValueError` for choice `"merge"`.
  - `test_decide_clash_takes_the_vault_lock`: patch `rbxskills._locked` to record its root; it was called with `state_dir()`.
  - `test_cli_vault_clashes_and_vault_clash_print_json`: through `rbxskills.cmd_skills`, as `test_cli_success_prints_result` does.
- [ ] **Step 2: Run** `tests.test_vaultsync` → FAIL.
- [ ] **Step 3: Implement** the functions and CLI actions above.
- [ ] **Step 4: Run** `tests.test_vaultsync` → PASS.
- [ ] **Step 5: Update DEVELOPING.md** (new short paragraph "Clashes"): the two choices, `replaced/`, the after-Keep-mine rule, and a skill decided as a whole folder.
- [ ] **Step 6: Commit** (`rbxvaultsync.py rbxskills.py tests/test_vaultsync.py DEVELOPING.md`) with "Vault: Keep the vault's or Keep mine for each clash; after Keep mine the vault's version is the base".

### Task 5: The app side of Pull newest: clash routes, Reveal anywhere in Knowledge

**Files:**
- Modify: `app-rs/src/vault_sync.rs`: the module doc, `reveal_target`/`reveal` (`:174-233`), `run` (`:363-373`), `stage_and_apply` (`:433-438`), and new `clashes`/`decide_clash`
- Modify: `app-rs/src/server.rs:213` area (GET `/api/vault/clashes`), `:433-458` (POST `/api/vault/clash`, and `which` on `/api/vault/reveal`)
- Test: `app-rs/src/vault_sync.rs` tests

**Interfaces:**
- Consumes: `share::kit_json(args: &[&str]) -> Result<Value, String>`, the CLI actions from Task 4, `unit_of` (Task 1).
- Produces:
  - `pub fn clashes() -> Value`: `{ok: true, clashes: [...]}` from `vault-clashes --json`, or `fail(e)`.
  - `pub fn decide_clash(unit: &str, choice: &str) -> Value`. It refuses with `BUSY` while an update holds `claim()` (it takes the claim itself), refuses any choice but `"vault"`/`"mine"` with "Choose Keep the vault's or Keep mine.", and otherwise passes on the CLI's answer.
  - `pub fn reveal_target(path: &str, which: &str) -> Result<PathBuf, String>` and `pub fn reveal(path: &str, which: &str) -> Value`. `path` is a `check_path` path or a unit `Skills/<name>`. `which` is `"mine"` (the local file, else its folder) or `"vault"` (the clash copy under `.fridae-vault/clash/`). It never goes through a link and never leaves `Knowledge/`. `NOT_SHOWN` becomes "Fridae can only show files in your Knowledge folder."
  - Routes: `GET /api/vault/clashes`; `POST /api/vault/clash {unit, choice}`; `POST /api/vault/reveal {path, which}` (missing `which` means `"mine"`). All are page routes; no agent tool reaches them.

Other changes:
- `run()`'s same-release check also skips files whose `unit_of` has an open clash in `base["clashes"]` with `release == tag`.
- `stage_and_apply` also copies a not-needed file from `.fridae-vault/clash/<path>` when its bytes match the manifest.
- The status note text "Getting skills…" is the page's (Task 12); nothing in Rust says "skills" for the whole vault any more.

- [ ] **Step 1: Write the failing tests:**
  - `reveal_stays_in_the_skills_folder` becomes `reveal_stays_in_the_knowledge_folder`: `Roblox/Glossary.md` resolves; `House rules.md`, `../x` and a linked folder are refused.
  - `reveal_shows_the_vaults_copy_of_a_clash`: `which: "vault"` gives `.fridae-vault/clash/Roblox/Glossary.md`.
  - `clash_decisions_wait_for_a_running_update`: with `claim()` held, `decide_clash` returns `busy`/`BUSY`.
  - `a_waiting_clash_is_no_reason_to_download_again`.
  - `guides_download_like_skills`: the stub serves `Roblox/Glossary.md` and a skill; after `update(true)` both are under Knowledge and the report's `added` lists both.
- [ ] **Step 2: Run** `$RUST cargo test ... -- vault_sync::` → FAIL.
- [ ] **Step 3: Implement** the interfaces and the server routes.
- [ ] **Step 4: Run** `$RUST cargo test ... -- vault_sync::` → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("The CLI and routes"): the two new routes, `which` on reveal, and Reveal anywhere in Knowledge.
- [ ] **Step 6: Commit** (`app-rs/src/vault_sync.rs app-rs/src/server.rs DEVELOPING.md`) with "App: clash routes and Reveal for everything in Knowledge".

### Task 6: Search reads `Knowledge/Roblox`; the app keeps its own code data

**Files:**
- Modify: `rbxvault.py:1-20` (docstring, `KB`, new `API_DIR`, `NOT_DOWNLOADED`), `:437`, `:445` (`api_index` reads `API_DIR`), `:2230-2256` (`_vault_call`, `list`), and the `search` branch of `run`
- Move (git mv): `vault/Roblox/_tools/roblox_api.json`, `vault/Roblox/_tools/api_overrides.json` and `vault/Roblox/_data/roblox_facts_2026.json` go to `vault/Fridae/Data/`
- Modify: `rbxkit_mcp.py:1450` (the `rbx_vault` description), `.gitignore` (`/Knowledge/`)
- Create: `tests/guides.py`
- Modify: each test module that reads real guide notes (at least `tests/test_knowledge.py`, `tests/test_rag_vault.py`, `tests/test_upkeep.py`, `tests/vault_inventory.py`, `tests/scale_bench/corpus.py`)
- Test: `tests/test_knowledge.py`, `tests/test_rag_chunks.py`

**Interfaces:**
- Produces:
  - `rbxvault.KB = os.path.join(rbxpaths.KNOWLEDGE_DIR, "Roblox")`. It stays a module attribute, so `rbxrag/chunks.py`, `rbxrag/engine.py`, `rbxnotes.py` and `rbxupkeep.py` keep reading `rbxvault.KB` unchanged.
  - `rbxvault.API_DIR = os.path.join(KIT, "vault", "Fridae", "Data")`.
  - `rbxvault.NOT_DOWNLOADED = "The Roblox guides are not downloaded yet: ask the user to press Download in Setup, Skills, Knowledge vault."`
  - `tests.guides.GUIDES: str | None`: the first existing of `<repo>/vault/Roblox`, `<repo>/Knowledge/Roblox`, or env `FRIDAE_TEST_GUIDES`.
  - `tests.guides.use_guides() -> None`: sets `rbxvault.KB = GUIDES` for the rest of the run, or raises `unittest.SkipTest("No Roblox guides on this computer: run Download in developer mode, or set FRIDAE_TEST_GUIDES.")`.

Behaviour:
- `_vault_call` and `list` raise `KitError(NOT_DOWNLOADED)` when `KB` is missing.
- A `search` with shelf `all` or `vault` while `KB` is missing returns its other shelves' results followed by a blank line and `NOT_DOWNLOADED`. This is how the Lead and every agent learn it.
- `superseded.json` keeps working because it is keyed by title and shelf `vault`.

- [ ] **Step 1: Write the failing tests** in `tests/test_knowledge.py`:
  - `test_vault_shelf_is_knowledge_roblox`: `rbxvault.KB == os.path.join(rbxpaths.KNOWLEDGE_DIR, "Roblox")` at import.
  - `test_search_without_guides_says_how_to_get_them`: patch `rbxvault.KB` to a missing folder; `run("search", {"query": "datastore", "shelf": "vault"})` ends with `NOT_DOWNLOADED`, and `run("list", {})` raises `KitError` with it.
  - `test_api_reads_the_apps_own_data`: `rbxvault.api_status("TeleportService", "ReserveServer")` is non-empty with `KB` patched to an empty temp folder.

  In `tests/test_rag_chunks.py`, `test_vault_notes_come_from_knowledge`: a note written under a patched `rbxvault.KB` in a temp Knowledge folder is in `chunks.corpus_files(None)` with shelf `"vault"`.
- [ ] **Step 2: Run** `tests.test_knowledge tests.test_rag_chunks` → FAIL.
- [ ] **Step 3: Implement.**
  - `git mv` the three data files and change `rbxvault.py` as above.
  - Give `rbxkit_mcp.py`'s description the words "One lookup for Roblox knowledge (the Roblox guides of the Fridae vault, which the user downloads in Setup: 1,300+ notes dated October 2026), " in place of "(LihFolk Vault V2: 1,300+ notes dated October 2026), ".
  - Add `/Knowledge/` to `.gitignore` under the "running from source" comment.
  - Write `tests/guides.py`, and call `tests.guides.use_guides()` in `setUpModule` (or at the top of a bench script) of every module that reads real guide notes. Find them by running the whole Python suite before the change and after it.
- [ ] **Step 4: Run** the whole Python suite: `PYTHONPATH=. H:/fridae-venv/Scripts/python -m unittest discover -s tests -t .`. The set of passing tests must equal the set before the change (save the before-list in Step 3). The new tests PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("How agents find knowledge"): the `vault` shelf reads `<home>/Knowledge/Roblox`; without a Download it says `NOT_DOWNLOADED`; the API snapshot lives in `vault/Fridae/Data`.
- [ ] **Step 6: Commit** (`rbxvault.py rbxkit_mcp.py .gitignore vault/Fridae/Data vault/Roblox/_tools vault/Roblox/_data tests DEVELOPING.md`) with "Search reads Knowledge/Roblox; the API snapshot stays in the app".

### Task 7: The app stops bundling `vault/Roblox`

**Files:**
- Create: `build_datas.py`, `tests/test_build_datas.py`
- Modify: `rbxkit.spec:8-11` (the vault walk becomes `datas += build_datas.vault_datas(HERE)`, after `sys.path.insert(0, HERE)`)
- Modify: `THIRD_PARTY.md:36` area, `DEVELOPING.md` "Folders" (`:2331-2332`)

**Interfaces:**
- Produces: `build_datas.vault_datas(here: str) -> list[tuple[str, str]]`. It returns `(source file, destination folder relative to here)` for every file under `here/vault`, leaving out dot-folders, dot-files and the whole of `vault/Roblox`.

- [ ] **Step 1: Write the failing test** `tests/test_build_datas.py::BuildDatasTests::test_guides_are_never_bundled`: in a temp tree with `vault/Roblox/a.md`, `vault/Roblox/_tools/x.json`, `vault/Fridae/Data/roblox_api.json`, `vault/Fridae/Guides/Core/s/SKILL.md`, `vault/.obsidian/y.json` and `vault/Fridae/.DS_Store`, the sources returned are exactly the `Data` and `Guides` files, with destinations `vault/Fridae/Data` and `vault/Fridae/Guides/Core/s` (`os.sep`-joined).
- [ ] **Step 2: Run** `tests.test_build_datas` → FAIL.
- [ ] **Step 3: Implement** `build_datas.py` and the spec change.
  - `THIRD_PARTY.md`: the guides are no longer in the app; they arrive with the Fridae vault (credited there), and `vault/Fridae/Data/roblox_api.json` is LihFolk's API snapshot (CC BY 4.0).
  - DEVELOPING.md "Folders": `vault/` is Fridae's own guides, team and `Fridae/Data`, and the Roblox guides come from the Fridae vault into `<home>/Knowledge/Roblox`.
- [ ] **Step 4: Run** `tests.test_build_datas` → PASS. Then run `H:/fridae-venv/Scripts/python -c "import build_datas; d=build_datas.vault_datas('.'); assert not any('Roblox' in s for s,_ in d); print(len(d))"`, which prints a count with no error.
- [ ] **Step 5: Commit** (`build_datas.py tests/test_build_datas.py rbxkit.spec THIRD_PARTY.md DEVELOPING.md`) with "Build: the app no longer carries the Roblox guides".

### Task 8: Create uploads the guides with the scaffolding, and the licence split

**Files:**
- Modify: `app-rs/src/vault_git.rs` (new `commit_tree` after `commit_files`)
- Modify: `app-rs/src/vault_inbox.rs:29-35` (constants), `:150-212` (`LICENSE`, `CONTRIBUTING`, `README`), `:227-263` (`create_repo_in`), and new `guides_source`, `guide_files`
- Test: `app-rs/src/vault_git.rs` tests, `app-rs/src/vault_inbox.rs` tests

**Interfaces:**
- Consumes: `vault_sync::check_path` (Task 1).
- Produces:
  - `vault_git::commit_tree(repo: &str, branch: &str, files: &[(String, Vec<u8>)], message: &str) -> Result<String, String>`. It makes one commit holding all `files`, built as a chain of trees of at most `TREE_BATCH = 200` entries (each `base_tree` the last). A text file (UTF-8, no NUL) goes inline as `content`, and a binary as a blob `sha` (`POST git/blobs`), so roughly 1,350 guides cost about 7 tree calls and 27 blob calls. The branch moves forward only (`force: false`). It returns the commit's sha.
  - `vault_inbox::guides_source() -> PathBuf`: `paths::PATHS.kit/vault/Roblox`, or in tests `paths::test_dir("vault-guides")`.
  - `vault_inbox::guide_files(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String>`: every regular file below `root` as `("Roblox/<rel with />", bytes)`, skipping dot-entries, links and any path `check_path` refuses (so `_tools/` and `_data/` stay out). It returns `Err(NO_GUIDES)` when `root` is missing or yields nothing.
  - `NO_GUIDES = "This copy of Fridae has no Roblox guides to upload. Run Fridae from its source folder to create the vault."`
- `create_repo_in` reads `guide_files(&guides_source())` **before** anything is asked of GitHub, so a missing source makes nothing. It then makes the repo and commits the text files plus the guides with `commit_tree`, message "Add the licence, the readmes and the Roblox guides". The click and confirm route is unchanged.

Exact text (fixed by the spec's credit rule):
- `REPO_DESCRIPTION`: "The shared Fridae Studios vault: skills and Roblox guides, reviewed by Fridae's maintainer."
- `LICENSE` keeps today's two sections (CC BY 4.0 with its address, then the MIT text). Insert after its first paragraph:
  > Who to credit. Notes and files in `Roblox/` that credit LihFolk are from LihFolk Vault V2, (c) 2026 LihFolk, under CC BY 4.0: credit them as "LihFolk Vault V2 by LihFolk" and link the licence. `Roblox/LICENSE.md` is LihFolk's own licence for his work and still applies to it. Everything else in this vault is the Fridae Studios vault's, (c) Fridae Studios and the people credited in each file or in this repository's history, also under CC BY 4.0: credit it as "Fridae Studios vault".
- `README`:
  > # Fridae vault
  >
  > The Fridae Studios vault: skills and Roblox guides, reviewed by Fridae's maintainer.
  >
  > Fridae Studios downloads this vault for anyone who asks, with no sign-in: Download once, then Pull newest, which never overwrites what you changed.
  >
  > - `Skills/` holds the skills. Each one is a folder with a `SKILL.md` and, sometimes, scripts.
  > - `Roblox/` holds the Roblox guides: about 1,300 notes on making games in Roblox Studio, with rigs and Blender scripts.
  >
  > Who made what: notes that credit LihFolk come from LihFolk Vault V2 and keep his credit; the rest is the Fridae Studios vault's own. Both are under CC BY 4.0, and scripts in skills under MIT. LICENSE.md says how to credit each. To share a skill or a change to a guide, use Share your knowledge in Fridae; CONTRIBUTING.md says how it works.
- `CONTRIBUTING`: "Skills in this vault" becomes "Skills and guides in this vault". The "How sharing works" paragraph reads "open Setup, then Skills, and turn on Share your knowledge. Fridae shows you exactly which of your skills and changed guides are ready, and exactly which files would be sent." "By sharing a skill" becomes "By sharing a skill or a guide".

- [ ] **Step 1: Write the failing tests:**
  - `vault_git::commit_tree_sends_text_inline_and_binaries_as_blobs`: 450 files, 3 of them binary, give exactly 3 tree POSTs, 3 blob POSTs, 1 commit POST and 1 ref PATCH with `"force":false`. No tree entry has both `content` and `sha`.
  - `vault_inbox::create_repo_uploads_the_guides_without_code_data`: the guides source holds `Glossary.md`, `02 Luau/Buffers.md`, `18 Rigs/R.fbx` (bytes with a NUL), `_tools/roblox_api.json`, `_data/f.json` and `.obsidian/a.json`. The committed paths are exactly the four scaffolding files plus `Roblox/Glossary.md`, `Roblox/02 Luau/Buffers.md` and `Roblox/18 Rigs/R.fbx`.
  - `create_repo_makes_nothing_without_guides`: with an empty source the answer is `NO_GUIDES`, and the stub saw no `POST /user/repos`.
  - `licence_credits_lihfolk_and_fridae`: `LICENSE` contains "LihFolk Vault V2 by LihFolk", "Fridae Studios vault" and "CC-BY-4.0"; `README` contains "Roblox/" and not "LihFolk's vault".
  - Adapt `create_repo_only_for_owner_once` and `create_repo_says_when_the_first_files_would_not_go_in` to `commit_tree`.
- [ ] **Step 2: Run** `$RUST cargo test ... -- vault_git::` and `-- vault_inbox::` → FAIL.
- [ ] **Step 3: Implement** the interfaces and the text above.
- [ ] **Step 4: Run** both → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("The repository", "Create the vault repo"):
  - The repo holds `Skills/` and `Roblox/`.
  - Create uploads `vault/Roblox` from the copy it runs in (only a copy run from source has it), minus `_tools/` and `_data/`.
  - The licence names LihFolk for notes that credit him and Fridae for the rest.
- [ ] **Step 6: Commit** (`app-rs/src/vault_git.rs app-rs/src/vault_inbox.rs DEVELOPING.md`) with "Create: the vault repo starts with the Roblox guides, credited to LihFolk where they are his".

### Task 9: The maintainer's inbox and Publish carry guides

**Files:**
- Modify: `app-rs/src/vault_inbox.rs`:
  - `inbox_in` (`:423-516`): a `notes` list beside `skills`, and `unit` on each skill
  - `decide`/`decide_in` (`:688-753`): by unit
  - `add_skill` (`:756-801`): the credit rule for a skill the vault already has
  - new `add_note`
  - `preflight` (`:830-863`): Skills paths only
  - `by_name`/`release_notes` (`:879-899`): a guides line
  - `NOTHING`
- Modify: `rbxshare.py:235-261` (`compare` accepts a file)
- Modify: `app-rs/src/server.rs:448` (`inbox_decide` reads `unit`)
- Test: `app-rs/src/vault_inbox.rs` tests, `tests/test_share.py`

**Interfaces:**
- Consumes: `check_path`, `unit_of` (Task 1).
- Produces:
  - Inbox JSON per PR: `skills: [{unit: "Skills/<name>", name, ...as today}]` and `notes: [{unit: <path>, path, title, text, note?, binary: bool, in_vault: bool, diff: str | null, decided}]`. A binary note has `note: "Not shown: the file is not text."`.
  - `rbxshare.compare(new_dir, against)` with `new_dir` a file returns `{"same_name": os.path.isfile(against), "closest": None, "diff": unified diff of against → new (labels "vault/<name>", "submission/<name>") or None}`. Rust calls it as `compare --new <pr file> --against <mirror file>`.
  - `pub fn decide(pr: u64, unit: &str, choice: &str, text: Option<&str>, why: Option<&str>) -> Value`. A unit `Skills/<name>` goes to `add_skill` as today. Any other `check_path` unit goes to `fn add_note(repo: &str, author: &str, pr: u64, path: &str, bytes: Vec<u8>) -> Result<(), String>`, which commits only that file with message "Add <title> from <author> (#<pr>)", or "Change <title> ..." when the mirror has it. Edit then add uses the maintainer's text, at most 200 KB. A binary note cannot be edited ("Only text can be edited here."). `decided.json` is keyed by unit.
  - The `add_skill` rule for a skill the mirror already has (`same_named` is Some): keep the mirror's `source` and `owner`. `credit` becomes the mirror's credit followed by `", <author>"` unless that login is already in it. The first-time rule is unchanged.
  - `preflight` and the skill names of `release_notes` consider only paths whose first part is `Skills`.
  - `release_notes` appends one line "Guides: N new, N changed, N removed." (only the non-zero parts; no line when all are zero), counted per non-Skills path against the previous manifest.
  - `NOTHING = "The vault has nothing to publish yet."`
  - The thank-you lists units by their names (skill name or note title).

- [ ] **Step 1: Write the failing tests** (Rust, with the stand-in):
  - `inbox_lists_guides_with_their_change_against_the_vault`: a PR changing `Roblox/Glossary.md` (in the mirror) and adding `Roblox/02 Luau/New.md` lists 2 notes, `in_vault` true and false, the first with a diff containing `+`.
  - `add_note_commits_only_that_file`: the commit tree holds exactly that path with the PR's bytes and deletes nothing.
  - `edit_note_uses_the_maintainers_text`.
  - `a_change_to_a_vault_skill_keeps_its_credit_and_adds_the_sender`: mirror credit `alice`, author `bob`, so `credit: alice, bob` and `source` unchanged.
  - `publish_carries_guides_and_counts_them`: the manifest lists `Roblox/...` files, and the notes hold "Guides: 2 new.".
  - `publish_never_carries_code_data_or_top_files`: `Roblox/_tools/x.json` and `README.md` on main are absent from the manifest.
  - `last_decision_closes_with_thanks` covers a PR holding one skill and one note.

  Python, `tests/test_share.py::test_compare_a_note_file`: same-name diff, and `closest is None`.
- [ ] **Step 2: Run** `$RUST cargo test ... -- vault_inbox::` and `tests.test_share` → FAIL.
- [ ] **Step 3: Implement** the above.
- [ ] **Step 4: Run** both → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("The maintainer's inbox", "Publish vault release"): guides arrive as units of one file, Add commits that file, a change to an existing skill keeps its credit and adds the sender's, and the release notes count guides.
- [ ] **Step 6: Commit** (`app-rs/src/vault_inbox.rs app-rs/src/server.rs rbxshare.py tests/test_share.py DEVELOPING.md`) with "Inbox and Publish: guides beside skills".

### Task 10: What of Knowledge may be shared (guides and changed vault skills)

**Files:**
- Modify: `rbxshare.py` (new functions after `mark_shared`)
- Modify: `rbxskills.py:1636-1648` (action `pack-knowledge`, flag `--unit`), `:1682-1700` (`_share_cli`: `shareable` adds `knowledge`; `mark-shared --unit`)
- Test: `tests/test_share.py`

**Interfaces:**
- Consumes: `rbxvaultsync.check_path`, `unit_of`, `read_base`, `_cmp_hash`, `_read`, `state_dir`, `clash_dir` (Tasks 1, 3); `rbxmerge.is_binary`.
- Produces:
  - `rbxshare.SHARED_UNITS = "shared.json"`, kept in `rbxvaultsync.state_dir()`: `{unit: [{hash, pr, at}]}`.
  - `rbxshare.knowledge_changes() -> {"ready": [{unit, kind: "note" | "skill", title, hash, new: bool, files: int}], "held": [{unit, title, why}], "sent": [{unit, title, pr, at}]}`.
    - Notes: every regular, non-link file under Knowledge whose relative path passes `check_path` and is not under `Skills/`, when the base record does not know it (`new`) or its bytes differ from `_cmp_hash`. `hash` is the file's sha256.
    - Skills: each folder in `Knowledge/Skills` whose files differ from their base copies or include a file the base does not know. The unit is `Skills/<upstream name>` (undoing `renamed`), and `hash = rbxskills.folder_hash(folder)`.
    - Held, with these reasons:
      - binary: "Pictures and models cannot be shared yet."
      - a note over 200 KB: "Over 200 KB."
      - a skill over `rbxshare` `FILES_MAX`/`TOTAL_MAX`: "Too big to share (over 1 MB or 40 files)."
      - an open clash: "Decide the vault's change to it first, above."
    - Sent: the unit's present hash is in `shared.json`. A later edit makes it `ready` again.
  - `rbxshare.pack_knowledge(units: list[str], out_dir: str) -> dict[str, str]`. It refuses units not in `ready` (`ValueError("Not ready to share: ...")`) and returns `{unit: hash}`.
    - A note is written byte for byte at `out_dir/<path>`.
    - A skill is written to `out_dir/Skills/<upstream name>/`, with its `SKILL.md` front matter `name` set to the upstream name and nothing else changed (the vault keeps its credit, Task 9).
  - `rbxshare.mark_shared_unit(unit: str, hash: str, pr_url: str) -> None`.
  - CLI: `rbx skills shareable` JSON gains `"knowledge": knowledge_changes()`; `rbx skills pack-knowledge --out <dir> <unit> [<unit>...]` prints `{unit: hash}`; `rbx skills mark-shared --unit <u> --hash <h> --pr <url>`.
- No self-exclusion anywhere: the maintainer's own Knowledge is listed like anyone's.

- [ ] **Step 1: Write the failing tests** in `tests/test_share.py`, new class `KnowledgeShareTests` (temp `KNOWLEDGE_DIR`, with the base written through `rbxvaultsync` as `test_vaultsync.seed` does):
  - `test_changed_and_new_guides_are_ready`: one edited and one new note in `ready`, with `new` False and True.
  - `test_unchanged_guide_is_not_listed`.
  - `test_binary_guide_is_held` (`.fbx`).
  - `test_guide_with_open_clash_is_held`.
  - `test_sent_guide_is_ready_again_only_when_changed`.
  - `test_edited_vault_skill_is_ready_under_its_vault_name`: with `renamed {"x": "x-community"}`, the unit is `Skills/x`, and `pack_knowledge` writes `Skills/x/SKILL.md` with `name: x`.
  - `test_pack_knowledge_writes_exact_bytes_at_their_paths`.
  - `test_code_data_house_rules_and_dot_folders_are_never_offered`: `House rules.md`, `.obsidian/a.json` and `Roblox/_tools/x.json` are not listed.
  - `test_cli_shareable_lists_knowledge`.
- [ ] **Step 2: Run** `tests.test_share` → FAIL.
- [ ] **Step 3: Implement** the above.
- [ ] **Step 4: Run** `tests.test_share tests.test_vaultsync` → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("What may be shared, and why"): guides and changed vault skills, the held reasons, `shared.json`.
- [ ] **Step 6: Commit** (`rbxshare.py rbxskills.py tests/test_share.py DEVELOPING.md`) with "Share: guides you changed and vault skills you edited can be sent".

### Task 11: The app sends batches of skills and guides, and the maintainer shares from her own repo

**Files:**
- Modify: `app-rs/src/share.rs`: `ready` (`:167-181`), `prepare_in` (`:320-377`), `title`/`body` (`:392-407`), `send_in` (`:417-477`), `status` (`:269-309`)
- Modify: `app-rs/src/vault_git.rs:156-182` (`ensure_fork`)
- Test: `app-rs/src/share.rs` tests, `app-rs/src/vault_git.rs` tests

**Interfaces:**
- Consumes: the CLI from Task 10 (`shareable`'s `knowledge`, `pack-knowledge`, `mark-shared --unit`).
- Produces:
  - `ensure_fork(upstream)` returns `Ok(upstream.to_string())` with no `POST /forks` and no `merge-upstream` when the signed-in login equals the upstream's owner (ASCII case-insensitive). The branch is then made in the vault repo itself, and `open_pr` gets head `"<owner>:<branch>"` as today.
  - A batch holds up to `BATCH_MAX` (20) units in all: the user's library skills first (as today), then the `knowledge` units.
    - The page's `skills` list gains `unit` and `kind` (`"skill"`, `"vault skill"`, `"guide"`).
    - Guides are masked with `compact::mask_for_sharing` like every file.
    - `batch.json` records `{unit, kind, folder | path, hash}`.
  - `send_in` commits library skills at `Skills/<folder>/<rel>`, vault skills at `Skills/<upstream>/<rel>`, and guides at their path. After the PR opens, it marks library skills with `mark-shared <name>` and knowledge units with `mark-shared --unit`.
  - The title is "Skills from <login>: ...", "Guides from <login>: ..." or "Skills and guides from <login>: ...", each followed by the first three names and "+N more". The commit message follows the same rule. A body line for a guide is "- **<title>** (`<path>`): new guide" or "...: changed guide". The branch name is unchanged.

- [ ] **Step 1: Write the failing tests:**
  - `vault_git::the_maintainer_shares_from_a_branch_of_the_vault_itself`: login `o`, upstream `o/r`; no `/forks` and no `merge-upstream` request, and `ensure_fork` gives `"o/r"`.
  - `share::maintainer_send_opens_a_pull_request_in_her_own_repo`: the PR head is `"o:<branch>"`.
  - `share::guides_are_masked_and_sent_at_their_paths`: a guide holding the login name is sent with `[user]`, at `Roblox/...`.
  - `share::a_batch_holds_skills_and_guides_up_to_20`: 15 skills and 10 guides give 20 sent, skills first.
  - `share::status_lists_guides_ready_held_and_sent`.
  - `share::titles_and_script_rules` gains the three title forms.
- [ ] **Step 2: Run** `$RUST cargo test ... -- share::` and `-- vault_git::` → FAIL.
- [ ] **Step 3: Implement** the above.
- [ ] **Step 4: Run** both → PASS.
- [ ] **Step 5: Update DEVELOPING.md** ("Sharing", "The pull request's shape"): batches hold skills and guides, and the maintainer's own sends are branches of the vault repo that arrive in her inbox like anyone's.
- [ ] **Step 6: Commit** (`app-rs/src/share.rs app-rs/src/vault_git.rs DEVELOPING.md`) with "Share: batches of skills and guides; the maintainer's own changes go through her inbox".

### Task 12: The page: Knowledge vault, clash cards, Share and Submissions for guides

**Files:**
- Modify: `ui/index.html`:
  - markup `:1340-1345` (the vault block), `:1356` (Share heading), `:1519` (help entry)
  - script `:5787-5812` (`skPaint`), `:5829-5910` (`vtLoad`, `vtReportLine`, `vtPaint`, `vt-go`), the Share painter, the Submissions painter (`:6019-6260`), `SB_CREATE_Q`
  - CSS: none new (reuse `ask rv-item sk-dec`, `sk-sides`, `plain-words`, `moreinfo`, `pill`, `pill sm`, `vt-kept`)
- Test: manual in the app's browser pane, with made-up data (the painters only paint)

**Interfaces:**
- Consumes: `GET /api/vault/clashes`, `POST /api/vault/clash {unit, choice}`, `POST /api/vault/reveal {path, which}` (Task 5); the report's `clashes`/`merged` (Task 3); inbox `notes` and `decide` by `unit` (Task 9); Share's `kind`/`unit` (Task 11).
- Produces (page functions): `vtLoadClashes()`, `vtClashCard(c) -> HTMLElement` and `sbNoteCard(pr, n) -> HTMLElement`. `vtReportLine(last)` is rewritten.

Exact copy:
- The block heading `Fridae vault` becomes **Knowledge vault**. The button reads **Download** before the first one and **Pull newest** after. The note while running is "Getting the vault…".
- The line before the first Download: "Skills and Roblox guides from Fridae's vault, checked by Fridae's maintainer. The Roblox guides are not on this computer yet: press Download to get them. Until then, Fridae's searches have no Roblox guides." After the first Download: "Release … · checked …" as today.
- The report line, built from what is non-empty and joined with ", ":
  - "<n> new skill(s)" (with today's "(<n> wait(s) for your decision below)")
  - "<n> new guide(s)"
  - "<n> updated"
  - then a sentence per edited kind: "Updated <n> guide(s) you had edited." and "Updated <n> skill(s) you had edited."
  - then "<n> clash(es) wait for your choice below."

  A path under `Roblox/` is a guide and under `Skills/` a skill; anything else is a "note". Below the line, `details.moreinfo` "Show them" lists each merged path with a `pill sm` Reveal (`which: "mine"`), in the existing `vt-kept` row style. The old "Kept your version…" rows are removed.
- Clash cards go into the `Needs your decision` list after the decision cards. Its count adds the clashes, the wrap shows when either kind exists, and "Accept all recommendations" never touches clashes. Each card is `ask rv-item sk-dec`:
  - meta: "Guide", "Skill" or "File", then `ago(made)`
  - h4: `title`
  - `sk-sides`: two sides, "Yours" and "The vault's"
  - plain words, one of:
    - "The vault changed <v> line(s) since you got it; you changed <m>."
    - for a skill: "The vault changed <v> line(s) in <k> file(s) of this skill; you changed <m>."
    - for `both_new`: "The vault has a new file with the same name as yours."
    - for a binary: each side shows its size (e.g. "1.2 MB") and, for yours, "changed <ago>", with a Reveal `pill sm` on each (`which` `"mine"` / `"vault"`)
  - `details.moreinfo` "Show both", with two `pre` blocks headed "Yours" and "The vault's" (`show.mine`, `show.vault`), only when they are non-empty
  - buttons: **Keep the vault's** and **Keep mine** (`pill`)
  - a `sub` line: "Keep the vault's sets yours aside in the vault's replaced folder; nothing is deleted."
  - an error in `rv-err`

  After a choice the card folds to one line, as decision cards do: "Kept the vault's “<title>”" or "Kept yours of “<title>”". Only the first 20 clash cards show, then a `pill sm` "Show all <N>".
- Share heading: **Share your knowledge**. Its prompt reads "Share skills you make and guides you change with everyone who uses Fridae?" The terms line reads "Shared skills and guides are under CC BY 4.0 (text) and MIT (scripts), credited to your GitHub name. Fridae hides keys, emails and your computer's username first. Fridae's maintainer reads every one before anyone gets it." Each ready, held or sent row shows its kind after the name ("guide", "vault skill").
- Submissions: each PR shows its skill cards as today, then a card per note (`sbNoteCard`):
  - meta: "Guide · <author>"
  - h4: title
  - "New guide" or "Changes a guide in the vault"
  - `details.moreinfo` "Show the change" holding the diff, or the text for a new one
  - Add / Edit then add / Reject, with Add asking first through `sbConfirm`: "Add “<title>” to the vault? It goes out in the next release."
- `SB_CREATE_Q`: "Make aprilfridae/fridae-vault on GitHub, public, with the licence, the readmes and the Roblox guides from this copy of Fridae. It takes a few minutes. Nothing is published until you press Publish vault release."
- Help entry `:1519` becomes: `["Knowledge vault", "Setup, Skills: get the skills and Roblox guides in Fridae's vault, and share yours.", "", "Download gets Fridae's vault, with no sign-in: skills, and the Roblox guides Fridae searches. Pull newest brings in what changed without overwriting yours: a file you never changed is updated; one you changed is left alone or, when both changes fit together, combined. When they clash, Fridae asks you here: keep the vault's or keep yours. Keeping the vault's sets yours aside, never deletes it. A skill that overlaps one you have waits for your decision too. To share your own skills and your changes to guides, turn on Share your knowledge, connect GitHub, and look over exactly what will be sent: keys, emails and your computer's username are hidden first, and scripts are shown first. Fridae's maintainer reads each one before anyone else gets it."]`

- [ ] **Step 1: Implement** the markup, painters and copy above, reusing only existing classes. Then search the diff for `Mac`, `#`-colours, `text-transform`, `@keyframes` and emoji: none are added.
- [ ] **Step 2: Fixture check, report and cards.** Start the app in developer mode, open Setup › Skills in the browser pane, and run in the page:
  - `vtPaint({downloaded: true, release: "vault-2026.12.01", checked_at: Date.now()/1000, last: {ok: true, added: ["Roblox/A.md"], updated: [], merged: ["Roblox/B.md","Roblox/C.md","Roblox/D.md"], clashes: ["Roblox/E.md"], errors: []}})`: the line reads "1 new guide. Updated 3 guides you had edited. 1 clash waits for your choice below."
  - Then paint clash fixtures of the four kinds (text, skill, `both_new`, binary) through the same function `vtLoadClashes` uses. Take a screenshot.
  - Expected: cards in the existing decision style, no new colours, buttons "Keep the vault's" and "Keep mine".
- [ ] **Step 3: Fixture check, 300 clashes** (Review Focus 5): paint 300 text clashes. Expected: 20 cards and "Show all 300"; pressing it shows all 300 and the page scrolls normally.
- [ ] **Step 4: Live check against a scratch vault.** Use the stand-in or a scratch home folder:
  - Press Download, edit one guide in `Knowledge/Roblox`, and serve a release changing the same line: a clash card appears.
  - Keep mine: the file is unchanged, and a later release changing another line combines without a card.
  - Keep the vault's on another: the user's copy is in `.fridae-vault/replaced/`.
- [ ] **Step 5: Phone-width and dark check:** resize to 375 px and to dark mode. The cards wrap with no sideways scroll.
- [ ] **Step 6: Commit** (`ui/index.html`) with "Setup: Knowledge vault with Pull newest, clash cards, and guides in Share and Submissions".

### Task 13: Remove `vault/Roblox` from the code repo (only after the user's word)

This task does nothing until the user has said, in chat, that it may run. Each precondition is checked and reported first.

**Files:**
- Delete: `vault/Roblox/` (what is left: the notes, `_tools/README.md`)
- Modify: `tests/guides.py` (drop the checkout fallback), `tests/scale_bench/corpus.py:190` (reads `tests.guides.GUIDES`), `rbxvault.py:1-7` (docstring), `DEVELOPING.md`, `THIRD_PARTY.md`

**Interfaces:**
- Consumes: `tests.guides.GUIDES` (Task 6).

- [ ] **Step 1: Check the preconditions and report them to the user, without changing anything:**
  - (a) `aprilfridae/fridae-vault` exists, and its `main` holds `Roblox/` with as many files as `vault/Roblox` minus `_tools/` and `_data/` (compare counts and paths).
  - (b) A release is published, and its manifest lists those files.
  - (c) A Download in developer mode filled `Knowledge/Roblox`, and `tests.test_knowledge` passes against it.
- [ ] **Step 2: Stop and ask the user** whether to remove `vault/Roblox` from the code repo now. Name what will be deleted (file count and size), that the vault repo will be the only copy, and that git history keeps the old files. Wait for a clear yes; without one, end the task here.
- [ ] **Step 3: Remove and repoint.**
  - `git rm -r vault/Roblox`.
  - `tests/guides.py` drops `<repo>/vault/Roblox`, and `corpus.py` uses `tests.guides.GUIDES`.
  - Fix the docstrings and docs that name `vault/Roblox` (`grep -rn "vault/Roblox" --include=*.py --include=*.rs --include=*.md . | grep -v docs/superpowers` is empty after).
- [ ] **Step 4: Run** the whole Python suite (with `Knowledge/Roblox` downloaded), plus `$RUST cargo test ... -- vault_sync::`, `-- vault_inbox::`, `-- share::` and `-- vault_git::` → the same passing set as before Step 3.
- [ ] **Step 5: Commit** (`git add -A vault tests rbxvault.py DEVELOPING.md THIRD_PARTY.md`) with "The Roblox guides live only in the Fridae vault now".
