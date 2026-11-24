# Knowledge exam: appendix

Two measurements made after the main report.

## A. More claims per answer (tried, then reverted)

**Question:** does letting the Librarian's writer state up to 4 claims for a one-part question (instead of 2) fix the missing facts?

**Method:**
- The same 61 questions in 4 setups.
- Each setup used the passages part 1 had already saved, so retrieval was identical before and after.
- The new writer prompt answered them, and the same Opus judge with the same prompt graded them.

| setup | run | judged | correct | fact recall | wrong facts | 3-fact questions right | unanswerable: said "not in KB" |
|---|---|---|---|---|---|---|---|
| light-1x | before (2 claims) | 60 | 28% | 30% | 6% | 0/17 | 4/8 |
| light-1x | after (4 claims) | 61 | 28% | 33% | 8% | 1/17 | 1/8 |
| light-10x | before | 61 | 30% | 34% | 2% | 0/17 | 3/8 |
| light-10x | after | 61 | 26% | 32% | 6% | 0/17 | 3/8 |
| heavy-1x | before | 61 | 25% | 32% | 4% | 0/17 | 1/8 |
| heavy-1x | after | 61 | 28% | 36% | 9% | 0/17 | 3/8 |
| keyword-10x | before | 61 | 33% | 29% | 8% | 0/17 | 5/8 |
| keyword-10x | after | 61 | 23% | 27% | 8% | 0/17 | 1/8 |

**Result:**
- The writer used the room: 1.6–1.7 claims per answer before, 2.3–2.7 after.
- With 2 claims allowed, no answer had 3 or more. With 4 allowed, 24–32 of 61 did.
- Correct answers did not rise, and wrong facts rose slightly.
- Reverted.

**Things worth noting:**
- Sonnet, reading the same passages, also gets only 1 of 17 three-fact questions. The likely limit is the passages: 3 per part, about 600 characters each, which often don't hold every key fact. Untested.
- The "said not in KB" column swings a lot between two runs of the same setup (light-1x: 4/8, then 1/8). With only 8 unanswerable questions, the writer's own randomness is as large as most differences in this table.

## B. A "not in the knowledge base" gate: how good is word coverage really?

**The signal:** for each question, the best keyword result's share of the question's words (`kw_cov`).
- The main report's best threshold looked excellent at 1x: 97% balanced.
- That threshold was fitted on only 12 unanswerable questions.

The trade-off at fixed thresholds, over 148 answerable and 12 unanswerable questions:

| vault | threshold | unanswerable refused | answerable wrongly refused |
|---|---|---|---|
| 1x | 0.45 | 1/12 | 0/148 |
| 1x | 0.50 | 1/12 | 1/148 |
| 1x | 0.55 | 6/12 | 2/148 |
| 1x | 0.60 | 7/12 | 4/148 |
| 1x | 0.65 | 11/12 | 10/148 |
| 10x | 0.45 | 3/12 | 4/148 |
| 10x | 0.50 | 4/12 | 6/148 |
| 10x | 0.55 | 7/12 | 11/148 |
| 10x | 0.60 | 7/12 | 20/148 |
| 10x | 0.65 | 10/12 | 29/148 |

**Reading:**
- At today's size, a threshold around 0.55 refuses half the unanswerable questions and wrongly refuses about 1% of answerable ones.
- At 10x the two groups overlap much more: the same threshold wrongly refuses 7% of answerable questions.
- Computing this signal also means a full keyword search: about 0.3 s at 1x and 3 s at 10x.

**Open questions:**
- Is a gate worth that?
- Would a signal taken from the passages the writer already gets, or a stricter writer instruction, do better?
