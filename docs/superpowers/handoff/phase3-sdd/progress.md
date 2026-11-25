# SDD ledger — plan: docs/superpowers/plans/2026-11-24-universal-skills-phase3.md

Worktree: /Users/angela/fridae studios/.superpowers/worktrees/skills-vault, branch feature/skills-vault, cut from feature/ui-overhaul a6c15bd. User approved the plan, subagent-driven, 2026-11-24.
Spec: docs/superpowers/specs/2026-11-24-universal-skills-design.md (+ unified-knowledge-vault-design.md).

## Preflight scan
| pair / task | produces vs consumes | finding |
|---|---|---|
| T1-T3 | T1 creates rbxvaultsync.py with read_base(); T3 lists "Create: rbxvaultsync.py" | conflict -> Ruling A |
| T3-T4 | T3 tests "patch newcomer checks off"; newcomer() is T4's | gap -> Ruling B |
| T4-T7 | SHARED_FILE in rbxskills (T4) imported by rbxshare (T7) | ok |
| T2-T8 | mask_for_sharing | ok |
| T5-T6/8/10/11 | vault_git fns | ok |
| T6-T8 | watch() gains share::auto_tick in T8 | ok |
| T6-T9/T10 | /api/vault/status merges share::status (T8), role (T10, null until then) | ok, T8/T10 extend the route |
| T6-T11 | parse_manifest round-trips publish's manifest | ok |
| T3-T6 | path rule in Python and Rust, same cases | ok |
| T9-T12 | same ui file, separate blocks | ok |
| T10-T11 | same file vault_inbox.rs | ok |
| each task self-consistency | T1..T13 tests vs code | consistent; T1 cargo filter `skills::tests::vault` matches `vault_folder_is_watched` |

Ruling A: T3 modifies rbxvaultsync.py (created in T1) — T1 needs read_base for library() — none if wrong.
Ruling B: T3 adds `newcomer(name, staged_dir) -> "install"` as a stub that apply() calls for each new skill folder; T4 replaces its body — keeps T3 testable alone — small rework if wrong.
Ruling C: .venv in the worktree is a symlink to the main checkout's venv (excluded via info/exclude) — tests need it — none.

## Tasks
Task 1: dispatched (base a6c15bd, implementer aecd1bbcbee47050c, sonnet)
Task 1: implementer DONE_WITH_CONCERNS (no separate Rust RED; test_meta_defaults gains credit; print_of fold) -> reviewing (a48b0fe5a8f26fb76)
Task 1: complete (commits a6c15bd..6afacc7, review clean; 5 Minor)
  Minor carried: APP_ONLY names not filtered for origin vault -> fold into Task 4 dispatch (library filters vault APP_ONLY; newcomer declines them). print_of combo untested (minor, left).
Task 2: dispatched (base 6afacc7)
Task 2: implementer DONE (1088c32) -> reviewing (a6ca5a28079d3b67b)
Task 2: complete (commits 6afacc7..1088c32, review clean; 6 Minor)
  Minor carried to final wave: user word case-insensitive ((?i)) for privacy; dead_code warning until Task 8 calls it.
Task 3: dispatched (base 1088c32)
Task 3: implementer DONE (5d38e9d; added state_dir(); loose files under Skills/ skip newcomer) -> reviewing
Task 3: review Needs fixes (1 Critical, 3 Important + spec gaps) -> fix round 1 dispatched; findings in task-3-findings-r1.md. Rulings: limits bind Python too (defence in depth); two-segment Skills/ paths refused (skills live in folders).
Task 3: fix round 1 implemented (87b9b1b) -> scoped re-review afe9fa8064cd2661c
Task 3: fix round 1/5 (11 addressed, 2 open — M2 name-length budget, M8 vacuous limits test; commits 5d38e9d..87b9b1b). Ruling: Knowledge itself may be a symlink; symlinks below it refused — portable folders — cost: none.
Task 3: fix round 2 implemented (06eddd0) -> re-review a3b0f44bbdbcbe449
Task 3: fix round 2/5 (5 addressed in code, 2 tests vacuous — N1 symlinked dir under base/, N3 renamed mapping; commits 87b9b1b..06eddd0) -> round 3
Task 3: fix round 3/5 (2 addressed, 0 open; commits 06eddd0..be94afd)
Task 3: complete (commits 1088c32..be94afd, review clean after 3 rounds)
  Task 3: minor (deferred): staged file read whole before len==size compare; beside numbering cap 99 untested; only file-name segment budgeted, not folder segments.
Task 4: dispatched (base be94afd); carries Task 1 minor (APP_ONLY for vault origin)
Task 4: implementer DONE_WITH_CONCERNS (8195185; 678 lines; O(n^2) newcomer; keep_both name edit vs base; replace refuses vault a)
  Ruling D: split newcomer/decide_vault into rbxvaultnew.py — file past the plan's size intent — cost: an import move.
  Ruling E: library+idf computed once per apply (apply-scoped context replaces module global) — first download of hundreds of skills — none.
  Ruling F: renamed skills: upstream SKILL.md bytes get the local name rewrite before compare/merge; base copy stores rewritten bytes; base.files keeps upstream sha — so keep_both copies still merge updates — small rework if wrong.
  Ruling G: replace with a vault-installed `a` moves a to .fridae-vault/removed/<release>/ and declines a's name — two overlapping vault skills must be decidable — none.
  Ruling H: declined:same not persisted (re-checked each release) — cheap once E lands — none.
Task 4: concerns addressed (135ee98) -> task review
Task 4: review Needs fixes (2 Important: upstream rename vanishes skill; half rollback) -> fix round 1. Ruling M3: merge into vault a -> merged text becomes a new user skill, vault a removed+declined — keeps the merge the user's own — cost: small rework.
Task 4: fix round 1 implemented (eec38a5) -> re-review
STOPPED 2026-11-24 at the parent session's request (user moving to Windows): Task 4 fix round 1 committed (eec38a5), its re-review stopped unfinished. Resume: re-review 135ee98..eec38a5 with task-4-findings-r1.md, then Task 5 (brief ready).
