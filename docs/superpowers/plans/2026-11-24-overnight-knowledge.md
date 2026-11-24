# Overnight: accurate knowledge at scale (2026-11-24 night)

The user, going to sleep: "just work on this all night dont ask me for shit ... implement everything you thought of, and then run your tests". Quick dev mode by day; long runs overnight (memory: quick-dev-mode-tests-overnight, rag-judged-on-scale). Nothing merges to main. Pushes go to `feature/ui-overhaul` as all day. Install only when the app is idle.

## Work, as separate branches (worktrees), then merged into feature/ui-overhaul

| # | What | Branch / worktree | Owner files |
|---|------|-------------------|-------------|
| A | Fast Librarian: the app searches every part at once, one cheap one-shot call writes the answer | feature/fast-librarian, /Users/angela/fridae-librarian | swarm.rs, compact.rs, a small rbx search subcommand |
| A2 | Quote-backed answers: each claim quotes its passage; a support check drops unsupported claims; the asker gets the quotes | same branch, after A | the same |
| B | Index quality: (1) a context line for each passage (title, section path, game, note gist, with a cached cheap-model one-liner per note); (2) note labels (game or all, kind, date, source) and search filters; (5) follow `[[links]]` from matching passages | feature/rag-quality, /Users/angela/fridae-rag-quality | rbxrag/*, rbxvault.py search |
| C | Lead on a strong model: a run's Lead never plans on DeepSeek/GLM (Setup switch, on by default, visible line) | feature/strong-lead, /Users/angela/fridae-strong-lead | agents.rs turn model, ui |
| D | Vault upkeep, after B: on filing, flag contradictions, merge near-duplicates, mark replaced notes; a nightly sweep report | feature/vault-upkeep | rbxvault.py note filing, Librarian |
| E | The full exam (user, 03:15: "as thorough as possible, test every mode, light, heavy, figure out what makes it good or bad"): feature/knowledge-exam, /Users/angela/fridae-exam — retrieval for keyword/Light/Heavy at 1x and 10x with ablations and new sets (typo, negative, multi-hop, stale, guide), end-to-end answers (real Librarian vs a strong reader, judged by Opus), failure analysis; report docs/superpowers/reports/2026-11-25-knowledge-exam.md. Build ~2 h, full run ~5–6 h. Was: the exam, after merge: the scale bench (feature/rag-scale-bench, 4500875) plus an answer-accuracy exam; full 1x and 10x runs overnight; results to docs/superpowers/reports/2026-11-25-knowledge-bench.md | feature/rag-scale-bench | tests/scale_bench |

## Ledger
- (entries appended as work lands)
- C done: strong Lead merged (feature/strong-lead 3f40fdf). Setting `strong_lead`, on by default; OpenCode picks the strongest plan model (never per-use); falls back to Claude, then Gemini, by moving the chat.
- A done: fast Librarian (feature/fast-librarian 20c01a3): `rbx lookup` searches all parts at once (0.87 s warm, keyword path), one cheap one-shot writer, snippet fallback. A2 (quotes plus support check) in progress on the same branch.
- Baseline exam running in /Users/angela/fridae-rag-bench → docs/superpowers/reports/2026-11-25-bench-baseline.md
- A2 done and merged (ed8899f → merge 4afe3df): each claim carries an exact quote; the app keeps only claims whose quote is in the cited passage (exact after normalising, or ≥90% of words in one stretch); one optional cheap check when 2+ claims drop; posted with a Quotes block, ~250 words max. Merged tree: Rust 554, Python 482, UI ok.
- Ruling: guides stay on-demand via rbx_guide (already a name+one-line menu with full-file loads); search hits on a guide could point to the whole guide instead of a fragment — the user then asked for a tag instead: every guide piece (search hit, section read, Librarian quote or fallback) now says to load the whole guide with rbx_guide before acting.
- Librarian guide rule (user's follow-up): guides are pointers only ("Recommended guide: X — matched on ... Call rbx_guide ..."), never summarised; guide passages never reach the writer; guides-only questions skip the writer. Answers from notes end with the escape hatch (rbx_vault read with the note's title and heading). Rust 555.
- B done and merged (feature/rag-quality e8b0936 → merge e7c6bbd): context line per passage (engine.FORMAT 2: indexes rebuild once), labels and filters (game/kind/source/since), [[link]] following. Conflict with the guide tag and the Librarian's `_rag_hits` resolved keeping both (filters pass through `_rag_hits`). Python 516, Rust 555.
- D started: feature/vault-upkeep, /Users/angela/fridae-upkeep — `supersedes:` marks and demotion, duplicate and contradiction checks at filing (the filing agent decides), a sweep report.
- The app was found closed (the user quit it), so the waiting install could never fire; stopped it. Fresh build from e7c6bbd to install and open now; the user will allow its Documents prompt.
- Ruling: Gemini's DeepSeek "reasoning_effort 1–100" claim is unverified; the exam measures the writer's reasoning tokens and latency, and compares effort levels if OpenCode offers them, before anything changes. Off-peak pricing does not apply (OpenCode Go is a flat plan).
