### Task 4: New vault skills meet the library: overlaps wait for a decision

**Files:**
- Modify: `rbxvaultsync.py`, `rbxskills.py` (`decide` hands decisions with `"from": "vault"` to
  `rbxvaultsync.decide_vault`; `_waiting` knows the incoming folder)
- Test: `tests/test_vaultsync.py`

**Interfaces:**
- Consumes: `rbxskills.similar`, `OVERLAP`, `library`, `_aside`, `_free`, `_set_front`, `folder_hash`, `_decisions`,
  `_save_decisions`, `APP_SKILL`.
- Produces in `rbxskills.py`: `SHARED_FILE = ".fridae-shared.json"` (Task 7's `rbxshare` imports it).
- Produces:
  - `incoming_dir() -> str` (`Knowledge/.fridae-vault/incoming`).
  - `newcomer(name: str, staged_dir: str) -> str` returns one of `"install"`, `"declined:app"`, `"declined:same"`,
    `"settled:own"` or `"incoming:<decision id>"`. A newcomer is a skill folder in the manifest with no path in base
    and no local folder.
  - The checks run in this order:
    1. The name equals an app skill's id (casefold): declined.
    2. A user library skill whose `.fridae-shared.json` lists a sent entry for this name, and whose `folder_hash`
       still equals that entry's `hash`: install, then `_aside` the user's copy. No card.
    3. Its SKILL.md body (front matter aside) is identical to a library skill's: declined ("you already have it").
    4. `similar()` with any non-project library skill is at least `OVERLAP`: stage the skill to `incoming/<name>/`,
       record `waiting[name] = release`, and open a decision
       `{"kind": "skill", "from": "vault", "a", "a_dir", "b": name, "b_dir": <incoming>, "reason": "overlap",
       "source": "Fridae vault <release>"}`.
    5. Otherwise, install.
  - While a name is in `waiting`, later updates to it replace the incoming copy, not a library folder.
  - `decide_vault(dec: dict, choice: str, merged_text: str | None) -> None`:
    - `keep`: the incoming copy moves to `removed/`, and the name goes in `declined`.
    - `replace`: the incoming copy moves into `Knowledge/Skills/<name>` and its files enter base. A user `a` is set
      aside with `_aside`. A shipped `a` gets `patch[name]["supersedes"] += [a]`. An app `a` is refused with
      `APP_SKILL`.
    - `merge`: phase 1's merge runs on `a`, and the name goes in `declined`.
    - `keep_both`: the skill is installed as `_free(vault_dir(), name + "-community", "-")`, with its front matter `name`
      set and `renamed[name]` recorded.

- [ ] **Step 1: Write the failing tests:**
  - `test_overlapping_newcomer_waits`: the library has user `debugging`, and the vault brings `debug-helper` with a
    near-identical description. Nothing appears in `Knowledge/Skills`, one open decision has `from == "vault"`, and
    `library()` lists only `debugging`.
  - `test_replace_moves_it_in_and_sets_mine_aside`: deciding `replace` gives `library()` the vault's `debug-helper`.
    The user's `debugging` is in `skills/.replaced/`, and `base.json` has its files.
  - `test_keep_declines_future_versions`: after `keep`, a later apply carrying `debug-helper` v2 writes nothing.
  - `test_keep_both_renames`: the skill is installed as `debug-helper-community`. A later update lands there.
  - `test_own_share_comes_back_without_a_card`: user `tidy` has `.fridae-shared.json`
    `{"sent": [{"name": "tidy", "hash": <its hash>, "pr": "...", "at": ...}]}`. The vault brings `tidy`, which
    installs. The user's copy is in `.replaced/` and no decision opens. If the user edited `tidy` after sending, a
    decision opens instead.
  - `test_app_name_is_declined`: vault `critic` is never installed, and `declined` contains `critic`.
  - `test_accept_all_settles_vault_decisions`: `rbxskills.decide(id, "replace")` works for a vault decision through
    the normal path.
- [ ] **Step 2:** Run `.venv/bin/python -m unittest tests.test_vaultsync -k newcomer -k replace -k keep -k own -k app
  -k accept`. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces. `_mine` stays the user-library check. `decide_vault` checks that every
  folder it moves is inside `vault_dir()`, `incoming_dir()` or the user library.
- [ ] **Step 4:** Run `.venv/bin/python -m unittest tests.test_vaultsync tests.test_skills`. Expected: PASS.
- [ ] **Step 5:** Commit: `Vault update: a new skill that overlaps yours waits for Keep / Replace / Merge / Keep both`.

