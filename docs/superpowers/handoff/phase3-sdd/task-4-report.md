# Task 4 report

Status: DONE_WITH_CONCERNS (rbxvaultsync.py is 678 lines, over ~550).

Implemented
- rbxvaultsync: incoming_dir, real newcomer (app/APP_ONLY decline, own share, identical body, similar >= OVERLAP -> staged to incoming/<name> + open decision), decide_vault (keep/replace/merge/keep_both), _renew (newer release replaces the waiting copy, clears recommendation), module-level _record_base shared with apply.
- rbxskills: SHARED_FILE, "vault" origin filtered against APP_ONLY in library(), _merge_skill factored out of decide (shared), _refuse_stale, decide delegates from=="vault" before taking its lock.
- Lock order: vault state lock first, then user library lock (comment in rbxvaultsync).

TDD
- RED: `.venv/bin/python -m unittest tests.test_vaultsync` -> FAILED (failures=13, errors=4) before implementation.
- GREEN: `.venv/bin/python -m unittest tests.test_vaultsync tests.test_skills tests.test_upkeep` -> Ran 194 tests OK.

Concerns / notes
- Size: would split the newcomer/decide_vault half (~280 lines) into rbxvaultnew.py.
- replace with an `a` that is itself a vault skill is refused (NOT_MINE); merge on it is allowed.
- newcomer calls library() and builds idf per newcomer: O(n^2) on a big first download.
- apply passes the release to newcomer via module global _applying (newcomer's signature is fixed).
- "declined:same" is not written to base declined (re-checked each release); "declined:app" is.
- Keep-both's front matter name edit makes SKILL.md differ from base, so a later upstream change to the same lines goes beside.

## Concerns addressed
- D: newcomer/decide_vault and helpers moved to rbxvaultnew.py (316 lines; rbxvaultsync.py now 472). rbxvaultsync keeps thin `newcomer` / `decide_vault` wrappers and `incoming_dir`; lock-order comment kept in rbxvaultnew. No import cycle (rbxvaultnew imports rbxvaultsync at top; the reverse is lazy).
- E: `rbxvaultnew.Ctx` (release, library, idf, app names, bodies) is built per apply and passed to `newcomer(name, staged_dir, ctx=None)`; `_applying` global removed; skills installed earlier in the same apply join the context. Test: test_library_is_read_once_for_thirty_newcomers (library() called once for 30 new skills); test_a_second_newcomer_sees_the_first_one_installed.
- F: a renamed skill's upstream SKILL.md is rewritten with the local name before comparing/merging; base copy stores the rewritten bytes, base["files"] keeps upstream sha256; comparisons use the base copy's hash (`_cmp_hash`). Test: test_upstream_change_reaches_a_keep_both_copy_cleanly. test_declined_and_renamed now expects the rewritten name.
- G: replace with a vault `a` moves its folder to removed/<release>/Skills/, forgets its base entries, adds its upstream name to declined, then installs (rolls back on failure). Test: test_replace_a_vault_skill_moves_it_to_removed_and_declines_it.
- H: declined:same left unpersisted.
- Note: the new tests for F/G/E were written after the refactor, so their RED run was not observed separately.

Command: `.venv/bin/python -m unittest tests.test_vaultsync tests.test_skills tests.test_upkeep`
Output: `Ran 198 tests in 34.966s` / `OK`

## Fix round 1
RED was observed first: 9 failures and 1 error among the new tests (`Ran 63 tests ... FAILED (failures=9, errors=1)`), then fixed.
- I1: what upstream dropped is now removed before newcomers are judged. Tests: test_upstream_rename_same_body_installs_the_new_name, test_upstream_rename_edited_body_asks_no_card.
- I2: `_install_folder` moves the folder back and forgets recorded entries if recording fails. Tests: test_failed_replace_is_rolled_back_whole, test_failed_keep_both_is_rolled_back_whole.
- M1: test_library_lock_timeout_on_a_waiting_skill_does_not_abort_the_update.
- M2: Ctx reads patch, own-share index and open decisions once (no separate test; covered by the 30-newcomer and own-share tests).
- M3: test_merge_into_a_vault_skill_makes_a_user_skill_and_removes_the_vault_one.
- M4: test_replace_a_hand_made_vault_folder_is_not_declined.
- M5: test_keep_both_avoids_a_name_a_library_skill_has.
- M6: test_symlinked_vault_folder_on_decide_gives_a_plain_message.
- M7: `len(...)` nit; I1/I2 tests added.
- M8: test_a_vault_decision_waits_only_while_its_folder_is_in_incoming.
- Two existing G/M3 tests now carry the old vault skill in the later manifest (otherwise upstream removes it first).

Command: `.venv/bin/python -m unittest tests.test_vaultsync tests.test_skills tests.test_upkeep`
Output: `Ran 208 tests in 35.652s` / `OK`
