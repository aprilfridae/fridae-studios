## Rulings (the user's answers on 2026-11-24, while planning, plus the builder calls that follow from them)

1. **Skills now, notes later.**
   - The download/update, submissions and inbox are built for any file under the vault, but only `Skills/` is
     allowed in this phase (`ALLOWED_TOP = ("Skills/",)` on both sides).
   - Notes, the `Knowledge/` move and the notes overlap check on updates and submissions come with the unified vault
     build, on this same machinery. The spec's phase 3 list loses "the overlap check on notes" until then.
2. **Where the vault lives.**
   - The repo is public, `aprilfridae/fridae-vault`. Fridae creates it only when the maintainer presses **Create the
     vault repo** and confirms.
   - Downloaded skills go in `~/Documents/Fridae Studios/Knowledge/Skills/` (`<HOME_DIR>/Knowledge/Skills` from
     source). The base record lives in `Knowledge/.fridae-vault/`.
   - The app's own and today's shipped guides stay inside the app (phase 1's ruling 2). The vault carries community
     skills now, and phase 2's starter library later.
3. **Review.**
   - In the maintainer's copy, an in-app Submissions inbox shows scripts first, then an AI comparison with the vault.
     Each skill gets Add / Edit then add / Reject.
   - Accepted skills go out only when the maintainer presses **Publish vault release** (confirmed each time).
     GitHub's own PR page still works for reading, but Publish refuses skills whose front matter the inbox did not
     set (Task 11).
4. **Only skills people made.**
   - Shareable: skills in the user's library with no import link, not `APP_ONLY`, not a project's, not named like one
     of Fridae's, and not a near copy of any skill Fridae can see.
   - Edited imports and edited community skills are never sent in this phase (a known limit).
   - The contributor agrees once to CC BY 4.0 for text and MIT for scripts.
5. **Vault skills are trusted like imports** (builder call).
   - A vault skill's own `core` and `supersedes` front matter never count. Only what Fridae writes in the base record's
     `patch` counts, the same rule as phase 1's import link.
   - Its owner is only ever `community` or `shipped`.
6. **Downloads are anonymous** (builder call). Getting skills needs no GitHub sign-in. Sharing and the inbox do.
7. **Files are moved, not deleted** (builder call). A file removed upstream and unchanged locally moves to
   `Knowledge/.fridae-vault/removed/<release>/`, never deleted.
8. **A three-way conflict is reported in the Fridae vault row** (builder call), not as a decision card. The new
   version is saved beside the old as `<stem> (Fridae update)<ext>`, and the row lists it with Reveal.

## Global Constraints

- Fridae never publishes, spends or deletes without the user's yes:
  - a share is sent only on the user's Send (Review each batch) or under their standing choice (Send automatically);
  - the repo is created, skills are added and releases are published only on the maintainer's click and confirm;
  - no agent has a tool for any of these.
- Builders and tests never contact github.com, api.github.com or raw.githubusercontent.com:
  - in `cfg(test)`, every base address comes from `github::STAND_IN` and is empty when unset;
  - Python tests have no network;
  - no task creates the real repo.
- The GitHub token stays in the Rust process. It is never in an env, argv, log, the page, a Python CLI call or a file
  outside `github::folder()`.
- `VAULT_REPO = "aprilfridae/fridae-vault"`. Raw files: `https://raw.githubusercontent.com/<repo>/<tag>/<path>`.
  Latest release: `GET https://api.github.com/repos/<repo>/releases/latest` (no token).
- Manifest: `{"format": 1, "release": "<tag>", "made": "<ISO-8601>", "files": [{"path", "sha256", "size"}]}`, paths
  sorted. Tags: `vault-YYYY.MM.DD`, then `vault-YYYY.MM.DD.2`, `.3`...
- Limits:
  - a manifest: at most 5,000 files, 1 MiB per file, 100 MiB in all;
  - a shared skill: text files only, 200 KB per file, 40 files, 1 MB in all;
  - a batch: at most 20 skills, at most one automatic batch every 24 h.
- The licence of a submission, written into its front matter exactly: `licence: CC-BY-4.0 (text), MIT (scripts)`.
- Every file Fridae writes goes through a temp file and a rename. Fridae never writes through a symlink, and never
  outside `Knowledge/`, the user's skill library or the app's own folder (`accounts::base().join("fridae-vault")`,
  called `VAULT_APP` below).
- The spec's anti-goals still hold:
  - an agent never sees two versions of a skill;
  - no community skill reaches anyone without the maintainer's review;
  - nothing without an open licence is shipped or submitted;
  - the cost of a turn does not grow;
  - no user skill or note is lost or silently overwritten.
- UI copy is plain English for a non-technical user. Use the exact strings given in tasks.
- Work on branch `feature/skills-vault`, cut from `feature/ui-overhaul` in a worktree. Never push or merge, and never
  touch main.
- Test commands: `.venv/bin/python -m unittest tests.test_vaultsync tests.test_share tests.test_skills`;
  `cargo test --manifest-path app-rs/Cargo.toml <filter>`. UI: extract the `<script>` blocks and `node --check` them.
- Commit messages end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

