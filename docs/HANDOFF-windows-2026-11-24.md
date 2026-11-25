# Handoff: continuing Fridae Studios on Windows (2026-11-24)

Paste this file's path into the first message of the new Windows session, e.g.
"Read docs/HANDOFF-windows-2026-11-24.md and continue from it."

The Mac session stopped mid-work because the user changed computers. Nothing was running when it stopped, and every
checkout was clean apart from untracked report files.

## 1. Getting the code onto Windows

Remote: `https://github.com/aprilfridae/fridae-studios.git`.

**Nothing below exists on GitHub until the user has said yes to a push from the Mac.** If a branch is missing after
`git fetch`, it was not pushed. Ask the user; don't rebuild the work.

| Branch | Head | State |
|---|---|---|
| `feature/ui-overhaul` | a6c15bd | Main line of work. It was 39 commits ahead of origin. Open PR #4 goes into `feature/knowledge-lookup`. |
| `feature/skills-vault` | eec38a5 | Phase 3 of universal skills, Tasks 1–4. It has never been pushed. |
| `feature/knowledge-exam` | ccea240 | +13 commits not in ui-overhaul (knowledge exam / Librarian bench). Not on origin. |
| `feature/rag-scale-bench` | 2e0c188 | +2 (scale bench for knowledge search at 1x and 10x). Not on origin. |
| `worktree-agent-acf61a7ac5cc3f507` | 52b396c | +21 (expert sidebar tree, plain-English expert summaries design). Not on origin. Check what is already merged before reusing it. |
| `worktree-agent-a5a8a084d12c73fba` | b9a726d | +10 (each added Gemini account gets its own keychain). Not on origin. |
| `worktree-agent-a694dca318806868b` | d1a8d48 | +1 ("Gemini's kept process is the default"). Not on origin. |

Every other local branch is already contained in `feature/ui-overhaul`. `main` (1f4bc49) is behind it.

Setup on Windows:

```bat
git clone https://github.com/aprilfridae/fridae-studios.git "fridae studios"
cd "fridae studios"
git checkout feature/ui-overhaul
python -m venv .venv
.venv\Scripts\python -m pip install pyinstaller numpy pillow opencv-python "fastembed==0.9.*" "qdrant-client==1.20.*"
```

To build, run `build_exe.bat`, which writes `dist\Fridae Studios`; it needs Rust. To install, run `install.bat`.
Rust tests run from `cargo test` in `app-rs/`; the Python tests are the repo's `test_*.py` files.

## 2. Windows gaps in the current code (check these first)

- **There are no OpenCode plan writers on Windows.** `opencode::writers()` returns an empty list when `cfg!(windows)`
  (app-rs/src/opencode.rs, `pub fn writers`). `advice_writers()` returns empty when `writers()` does.
- **There is no Claude Code fall-back on Windows.** `claude_allowed(windows, keys_on)` is `!windows && !keys_on`
  (app-rs/src/compact.rs).
- So skill advice, chat summaries and the Librarian have no writer at all on Windows. Before building on Windows,
  find out why `cfg!(windows)` was excluded (git log -S "cfg!(windows)" on those files). If the reason no longer
  holds, lifting it is probably the first real Windows task. Ask the user before changing it.
- These are Mac-only and must be skipped or adapted:
  - `build_mac.py` (use `build_exe.bat`);
  - the codesign checks;
  - the `~/Library/Application Support/Fridae Studios/...` paths. Fridae's OpenCode store and logs live under the
    Windows app-data equivalent; check `paths.rs`.
- The app has no log file. On the Mac its stderr was captured with `open --stderr FILE`. On Windows, run the exe from a
  terminal to see it.

## 3. Where each piece of work stands

### Universal skills, phase 1: done and merged

It is merged into `feature/ui-overhaul`, through the final review and its fix wave. The old phase 1 worktree
(`feature/shared-skills`) is fully merged and can be deleted.

### Skill advice writers: done (a6c15bd, 5d38e9d)

Skill recommendations ask these writers in order:

1. `opencode-go/glm-5.3-flash#low`, then
2. `opencode-go/claude-haiku-5-5#none`, then
3. the general writers (DeepSeek, GLM).

The `#variant` part goes to OpenCode as the session model's `variant`. The limits are 240 s in all, 120 s per model and
60 s held in reserve (`ADVICE_LIMITS`).

Measured on the user's 8 decisions:

| Writer | Result | Typical | Max |
|---|---|---|---|
| GLM flash #low | 8/8 | 22 s | 64 s |
| Haiku #none | 8/8 | 19 s | 26 s |
| DeepSeek | | 89–116 s | |

The 6 recommendations that Claude Haiku had written were rewritten by GLM flash.

Known oddities, already reported to the user:

- The "why" on roblox-security echoes the prompt's wording. Tightening the advice prompt is a possible follow-up.
- skill-creator vs writing-skills shows "no clear pick" because a merge built from cut text is refused, by design.
- 7 cards recommend a merge. The user was warned not to press "Accept all" without reading them.

### Universal skills, phase 3 (shared Fridae vault): in progress, on `feature/skills-vault`

| Item | Where |
|---|---|
| Plan | `docs/superpowers/plans/2026-11-24-universal-skills-phase3.md` (13 tasks) |
| Spec | `docs/superpowers/specs/2026-11-24-universal-skills-design.md` and `unified-knowledge-vault-design.md` |
| Build ledger and notes | `docs/superpowers/handoff/phase3-sdd/` on that branch, copied there because the `.superpowers/sdd/` copy is git-ignored |

**Before resuming,** run the subagent-driven-development skill's `sdd-workspace` script on the plan, then copy
`docs/superpowers/handoff/phase3-sdd/*` into the workspace folder it prints. That puts the ledger back where the skill
expects it.

Progress:

| Task | Status | Commits |
|---|---|---|
| Task 1: vault skills appear in the library | complete | a6c15bd..6afacc7 |
| Task 2: keys, emails and the username are hidden before sharing | complete | 6afacc7..1088c32 |
| Task 3: vault updates never overwrite the user's edits | complete after 3 fix rounds | 1088c32..be94afd |
| Task 4: a vault skill that overlaps the user's own waits for their decision | implemented, re-review not run | 8195185, 135ee98, fix round 1 eec38a5 |

Task 4's review found 2 Important problems:

- a skill renamed upstream could vanish;
- a Replace that failed partway could leave two copies of a skill visible.

Fix round 1 (eec38a5) addresses both. Its scoped re-review was stopped unfinished.

**Resume at Task 4's re-review.** Review `135ee98..eec38a5` against `task-4-findings-r1.md` and the
`rereview-instructions.md` in that folder. Then go on to Task 5; its brief, `task-5-brief.md`, is ready. Tasks 5–13
still to do:

- GitHub downloads;
- sharing as pull requests;
- the maintainer's Submissions inbox;
- releases;
- the Setup screens.

The rulings (A–H, M3) and the deferred minors are in `progress.md`. The phase rulings are in `constraints.md`. In short:

- skills only for now, notes later;
- the repo is the public `aprilfridae/fridae-vault`, created only when the maintainer clicks and confirms;
- downloads are anonymous;
- files are moved, never deleted;
- a three-way conflict is reported in the vault row.

The ledger's Ruling C says the worktree's `.venv` is a symlink to the main checkout's venv. On Windows, make a venv
(or a junction) for the worktree instead.

### Still on the list (not started)

- Universal skills phase 2: the starter library and **Put back**.
- The overnight 10x skills bench. Run long benchmarks overnight, and say how long a run will take before starting it.
- Remote-runs task (an earlier proposal card, id task_7dcb9624). It is not carried over; ask the user if it is
  still wanted.
- Tightening the skill-advice prompt so the "why" does not echo the instructions.
- Deleting the merged worktrees and branches (`feature/shared-skills`, and the old `worktree-agent-*` branches once
  their work is confirmed merged or dropped). Ask first.

## 4. Standing rules from the user (carry these into every session)

The Mac Claude memory does not travel, so these are copied here.

**Secrets and other tools' settings**

- Never print secrets: the fridae-url token, the OpenCode password, the GitHub token, the RAG token.
- Fridae reads but never writes the user's `~/.claude`, `~/.gemini`, `~/.cursor` and OpenCode config. Never modify
  the user's own OpenCode data or config dirs, and never touch the user's own `opencode serve --service`.
- Never click OS privacy or keychain/credential prompts, and never Reset To Defaults.
- Never log out of Claude, Cursor or Gemini.
- Never touch the user's git, gh or keychain setup.

**Merging, pushing and spending**

- Nothing merges to main without the user's word. Push only when the user asks.
- Fridae never publishes, spends or deletes without the user's yes. Creating the public `fridae-vault` repo needs
  the user's yes. Sharing by contributors is opt-in.

**Models**

- Use the cheap models (DeepSeek/GLM on OpenCode Go) only for simple jobs. Never use them for plans, hard builds or
  UI.
- Never use a pay-per-use model.

**Builds, tests and installs**

- Builders and tests never contact github.com and never install Ollama or Qdrant.
- Install a new build only when the app is idle and the user is around. On the Mac each install re-asked for Documents
  access, so check whether Windows has an equivalent.
- Quick dev mode by day: build the best guess now. Run long tests overnight, and say how long a run will take up front.

**Commits and writing**

- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Talk to the user in plain, non-technical English.

## 5. The user's preferences (from the Mac memory)

**Product direction**

- **t3code is the benchmark** (pingdotgg/t3code). Adopt what is promising from it.
- **Fridae is becoming the user's main AI harness.** Projects are typed as game or plain/other.
- **One unified knowledge vault.** The user's notes merge into the LihFolk vault in their portable folder, never kept in
  two places.
- **RAG is planned:** local hybrid search over the vault, the user's notes and game docs. It is bundled or embedded,
  with no daemons.
- **RAG is judged on scale.** Bench search at 10x the notes (meaning and tokens), not today's small bench.

**Look and feel**

- **Kids' game UI is not minimal.** The user rejected a muted, "AI-looking" UI. It should be fun, naturalistic and
  tasteful.
- **Gemini is for AI images.** The user wants real AI-generated icons and art made with Gemini, including transparent
  ones. Gemini's quota is small.

**Models and accounts**

- **Second accounts roll over automatically** to a second Claude or Gemini account at the limit. The risk has already
  been explained to the user once.
- **Plain-English expert summaries.** A cheap model summarizes each finished expert's changes in non-technical words,
  shown to the user.
