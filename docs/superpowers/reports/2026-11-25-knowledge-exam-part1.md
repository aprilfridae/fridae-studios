# Knowledge exam

Made 2026-11-24T10:22 on this Mac (arm64, Python 3.14.3). Corpora: 1x = full-1x-94da2483d343d513 (about 20551 chunks), 10x = full-10x-6c30ef6bcfbfd7ab (about 205420 chunks). Search code 5b8c081435e7.

Code measured: commit `8388eb1fd6fb34ed8b9333f0ff56dc0682a53fc1` on `feature/knowledge-exam`. Each saved result carries the commit it was measured at (`_measured`); results from other commits: none.

## 1. Retrieval

Hit@k: the gold note among the first k distinct notes the engine returns (asked for 8). MRR: mean of 1/rank. No model calls; the same questions for every method.

| method | scale | set | n | hit@1 | hit@3 | hit@5 | MRR | content-right hit@1 | hit@3 | hit@5 |
|---|---|---|---|---|---|---|---|---|---|---|
| keyword | 1x | paraphrase | 58 | 48% | 53% | 66% | 0.53 | 48% | 53% | 66% |
| keyword | 1x | exact | 12 | 100% | 100% | 100% | 1.00 | 100% | 100% | 100% |
| keyword | 1x | old33 | 29 | 41% | 72% | 76% | 0.56 | 41% | 72% | 76% |
| keyword | 1x | typo | 15 | 53% | 60% | 80% | 0.60 | 53% | 60% | 80% |
| keyword | 1x | guide | 10 | 40% | 100% | 100% | 0.70 | 40% | 100% | 100% |
| keyword | 1x | stale | 10 | 40% | 60% | 60% | 0.48 | 40% | 60% | 60% |
| keyword | 1x | multihop | 10 | 80% | 80% | 90% | 0.82 | 80% | 80% | 90% |
| keyword | 1x | answerable | 144 | 53% | 67% | 76% | 0.61 | 53% | 67% | 76% |
| keyword | 10x | paraphrase | 58 | 38% | 43% | 45% | 0.41 | 38% | 43% | 45% |
| keyword | 10x | exact | 12 | 92% | 92% | 92% | 0.93 | 92% | 92% | 92% |
| keyword | 10x | old33 | 29 | 17% | 55% | 62% | 0.36 | 17% | 55% | 62% |
| keyword | 10x | typo | 15 | 27% | 27% | 40% | 0.33 | 27% | 27% | 40% |
| keyword | 10x | guide | 10 | 50% | 100% | 100% | 0.75 | 50% | 100% | 100% |
| keyword | 10x | stale | 10 | 50% | 60% | 60% | 0.55 | 50% | 60% | 60% |
| keyword | 10x | multihop | 10 | 80% | 80% | 80% | 0.82 | 80% | 80% | 80% |
| keyword | 10x | answerable | 144 | 42% | 56% | 59% | 0.50 | 42% | 56% | 59% |
| light | 1x | paraphrase | 58 | 53% | 71% | 71% | 0.63 | 53% | 71% | 71% |
| light | 1x | exact | 12 | 92% | 100% | 100% | 0.94 | 92% | 100% | 100% |
| light | 1x | old33 | 29 | 79% | 93% | 100% | 0.88 | 79% | 93% | 100% |
| light | 1x | typo | 15 | 53% | 80% | 80% | 0.66 | 53% | 80% | 80% |
| light | 1x | guide | 10 | 80% | 100% | 100% | 0.90 | 80% | 100% | 100% |
| light | 1x | stale | 10 | 50% | 70% | 70% | 0.62 | 50% | 70% | 70% |
| light | 1x | multihop | 10 | 80% | 90% | 100% | 0.88 | 80% | 90% | 100% |
| light | 1x | answerable | 144 | 65% | 82% | 84% | 0.74 | 65% | 82% | 84% |
| light | 10x | paraphrase | 58 | 41% | 55% | 64% | 0.49 | 41% | 55% | 64% |
| light | 10x | exact | 12 | 83% | 92% | 92% | 0.88 | 83% | 92% | 92% |
| light | 10x | old33 | 29 | 59% | 72% | 76% | 0.67 | 59% | 72% | 76% |
| light | 10x | typo | 15 | 47% | 67% | 73% | 0.57 | 47% | 67% | 73% |
| light | 10x | guide | 10 | 80% | 90% | 90% | 0.85 | 80% | 90% | 90% |
| light | 10x | stale | 10 | 60% | 90% | 90% | 0.73 | 60% | 90% | 90% |
| light | 10x | multihop | 10 | 70% | 80% | 80% | 0.76 | 70% | 80% | 80% |
| light | 10x | answerable | 144 | 55% | 69% | 74% | 0.63 | 55% | 69% | 74% |
| heavy | 1x | paraphrase | 58 | 62% | 79% | 83% | 0.71 | 62% | 79% | 83% |
| heavy | 1x | exact | 12 | 83% | 92% | 100% | 0.88 | 83% | 92% | 100% |
| heavy | 1x | old33 | 29 | 69% | 86% | 90% | 0.78 | 69% | 86% | 90% |
| heavy | 1x | typo | 15 | 67% | 80% | 80% | 0.73 | 67% | 80% | 80% |
| heavy | 1x | guide | 10 | 100% | 100% | 100% | 1.00 | 100% | 100% | 100% |
| heavy | 1x | stale | 10 | 70% | 70% | 70% | 0.72 | 70% | 70% | 70% |
| heavy | 1x | multihop | 10 | 70% | 90% | 90% | 0.81 | 70% | 90% | 90% |
| heavy | 1x | answerable | 144 | 69% | 83% | 86% | 0.77 | 69% | 83% | 86% |

Strict: the gold note itself. Content-right: the gold note or a distractor the corpus made from it (a split, summary or rewording of it, or a mix holding one of its sections; corpus.py records each distractor's source notes in distractors.json). Echo notes, which only borrow the gold note's title words, never count.

Where the bigger vault's drop comes from (answerable questions with the gold note in the top 5 at the smaller scale):

| method | scales | found at small | still found | lost | lost to a copy (right content) | really missed |
|---|---|---|---|---|---|---|
| keyword | 1x → 10x | 109 | 84 | 25 | 0 (0%) | 25 |
| light | 1x → 10x | 121 | 101 | 20 | 0 (0%) | 20 |

Multi-hop, stale pairs, guides and game scope:

| method | scale | multi-hop both@5 / either@5 | stale: new above old (marked / text only) | guide found@5 / with tag | game note @3: no filter → game=this | general @3 with game=general |
|---|---|---|---|---|---|---|
| keyword | 1x | 50% / 90% | marked 100% (outdated in top 5: 100%, mean rank 2.4) / text only 0% (outdated in top 5: 100%, mean rank 1.4) | 100% / 100% | 100% → 100% | 50% |
| keyword | 10x | 40% / 80% | marked 100% (outdated in top 5: 80%, mean rank 2.0) / text only 20% (outdated in top 5: 100%, mean rank 1.8) | 100% / 100% | 100% → 100% | 25% |
| light | 1x | 60% / 100% | marked 100% (outdated in top 5: 100%, mean rank 2.0) / text only 0% (outdated in top 5: 100%, mean rank 1.2) | 100% / 100% | 100% → 100% | 100% |
| light | 10x | 70% / 80% | marked 100% (outdated in top 5: 80%, mean rank 2.0) / text only 20% (outdated in top 5: 100%, mean rank 1.6) | 90% / 90% | 100% → 100% | 50% |
| heavy | 1x | 70% / 90% | marked 100% (outdated in top 5: 100%, mean rank 2.0) / text only 40% (outdated in top 5: 100%, mean rank 1.4) | 100% / 100% | 100% → 100% | 100% |

Cost per search and per index:

| method | scale | tokens k=3 | k=5 | k=8 | engine s p50/p95 | rbx_vault s p50/p95 (k=5) | over 3 s | cold build | open (warm) | one-note update | RSS open MB | peak RSS MB | Qdrant RSS MB | disk MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| keyword | 1x | 632 | 896 | 1293 | 0.275 / 0.301 | 0.274 / 0.301 | 0 | 3 s (keyword indexes) | – | 2.90 s | 172 | 227 | – | 14 |
| keyword | 10x | 622 | 878 | 1250 | 2.901 / 3.286 | 2.888 / 3.282 | 59 | 28 s (keyword indexes) | – | 30.47 s | 415 | 1173 | – | 124 |
| light | 1x | 896 | 1325 | 1743 | 0.016 / 0.024 | 0.252 / 0.277 | 0 | 8 min (24110 embedded, 114 cached) | 4.2 s | 0.11 s | 472 | 566 | – | 45 |
| light | 10x | 889 | 1328 | 1748 | 0.088 / 0.225 | 3.169 / 3.623 | 0 | 48 min (184777 embedded, 24224 cached) | 52.0 s | 1.07 s | 1317 | 2124 | – | 349 |
| heavy | 1x | 895 | 1326 | 1739 | 1.452 / 1.545 | 1.717 / 1.834 | 0 | 21 min (50012 embedded, 242 cached) | 4.2 s | 0.28 s | 1530 | 1661 | 193 | 776 |

Tokens: rbx_vault's search text as an agent reads it (about 4 characters a token). Cold build: from no index (vectors already in the exam's vector cache are not embedded again; the counts say how many were). Open: an existing index loaded and checked. Over 3 s: searches the app would have answered with the keyword search instead (its timeout).

Silent fallback: the same searches (k=5) with rbx_vault's real 3 s timeout. A search that gets no index answer in time is answered by the keyword search, and the agent is not told.

| method | scale | searches | fell back to keyword | rbx_vault s p50 / p95 |
|---|---|---|---|---|
| light | 1x | 160 | 0 (0%) | 0.25 / 0.27 |
| light | 10x | 160 | 0 (0%) | 3.16 / 3.59 |
| heavy | 1x | 160 | 0 (0%) | 1.72 / 1.82 |

### Telling "not in the knowledge base" apart

Each answerable question's and each negative question's best score, and the threshold that separates them best (balanced accuracy: the mean of answerable kept and negatives refused). Negatives: 12.

| method | scale | signal | threshold | balanced | answerable kept | negatives refused |
|---|---|---|---|---|---|---|
| keyword | 1x | keyword coverage | 0.661 | 97% | 93% | 100% |
| keyword | 1x | keyword BM25 | 4.918 | 72% | 68% | 75% |
| keyword | 10x | keyword coverage | 0.672 | 83% | 75% | 92% |
| keyword | 10x | keyword BM25 | 4.996 | 72% | 85% | 58% |
| light | 1x | keyword coverage | 0.661 | 97% | 93% | 100% |
| light | 1x | keyword BM25 | 4.918 | 72% | 68% | 75% |
| light | 1x | dense cosine | 0.726 | 74% | 57% | 92% |
| light | 10x | keyword coverage | 0.672 | 83% | 75% | 92% |
| light | 10x | keyword BM25 | 4.996 | 72% | 85% | 58% |
| light | 10x | dense cosine | 0.725 | 74% | 65% | 83% |
| heavy | 1x | keyword coverage | 0.661 | 97% | 93% | 100% |
| heavy | 1x | keyword BM25 | 4.918 | 72% | 68% | 75% |
| heavy | 1x | dense cosine | 0.682 | 80% | 77% | 83% |
| heavy | 1x | reranker score | -2.616 | 82% | 73% | 92% |

## 2. Ablations

Each row switches one piece off (or changes it) and compares with the app as shipped on the same index and questions. Deltas in percentage points over the answerable questions; negative means the piece helps.

| method | scale | variant | hit@1 | hit@5 | MRR | Δ hit@1 | Δ hit@5 | Δ MRR (×100) |
|---|---|---|---|---|---|---|---|---|
| keyword | 1x | nolinks | 53% | 76% | 0.61 | +0 | +1 | +0 |
| keyword | 10x | nolinks | 42% | 59% | 0.49 | +0 | +0 | +0 |
| light | 1x | dense | 58% | 81% | 0.68 | -7 | -3 | -6 |
| light | 1x | lexical | 55% | 71% | 0.63 | -10 | -13 | -12 |
| light | 1x | nolinks | 65% | 88% | 0.75 | +0 | +4 | +0 |
| light | 10x | dense | 46% | 71% | 0.56 | -9 | -3 | -6 |
| light | 10x | lexical | 42% | 61% | 0.51 | -13 | -13 | -12 |
| light | 10x | nolinks | 55% | 72% | 0.62 | +0 | -2 | +0 |
| heavy | 1x | norerank | 63% | 83% | 0.74 | -6 | -3 | -3 |
| heavy | 1x | dense | 64% | 85% | 0.73 | -6 | -1 | -4 |
| heavy | 1x | lexical | 55% | 71% | 0.63 | -15 | -15 | -14 |
| heavy | 1x | sparse | 53% | 80% | 0.64 | -17 | -6 | -13 |
| heavy | 1x | dense+sparse | 67% | 88% | 0.76 | -2 | +1 | -1 |
| heavy | 1x | app+sparse | 60% | 86% | 0.72 | -10 | +0 | -5 |
| heavy | 1x | nolinks | 69% | 88% | 0.77 | +0 | +2 | +0 |
| light | 1x | index: no context line | 58% | 79% | 0.68 | -8 | -5 | -6 |
| light | 1x | index: no gist | 60% | 76% | 0.68 | -6 | -8 | -6 |

Variants: dense = the dense ranking alone; lexical = the keyword search's ranking alone (through the engine); sparse = bm25 sparse vectors alone; dense+sparse = the two fused without the keyword ranking; app+sparse = the app's plan with the sparse ranking added; norerank = Heavy without its reranker vote; nolinks = no [[link]] following. Index variants rebuild the index without the chunk's context line, or without the note's gist in it.

How many results to return (rbx_vault's text at k = 3, 5, 8):

| method | scale | k=3: hit, tokens | k=5 | k=8 |
|---|---|---|---|---|
| keyword | 1x | 69%, 635 tok | 76%, 898 tok | 78%, 1297 tok |
| keyword | 10x | 51%, 625 tok | 60%, 882 tok | 65%, 1256 tok |
| light | 1x | 78%, 899 tok | 81%, 1325 tok | 90%, 1748 tok |
| light | 10x | 65%, 890 tok | 72%, 1328 tok | 74%, 1748 tok |
| heavy | 1x | 80%, 896 tok | 84%, 1326 tok | 90%, 1740 tok |

## 3. Answers

61 questions (45 answer, 8 guide, 8 not_found), each with 2-4 key facts and some wrong facts. Librarian: the app's own answer path (split, writer, quote and support checks) through `fridae --librarian-exam`, the writer on the user's OpenCode plan. Reader: Claude Sonnet with the same passages, no tools, citing passages. Judge: Claude Opus, 8 answers a call, strict fixed prompt; its raw replies are kept. Correct: every key fact and no wrong one (negatives: says it is not in the knowledge base; guides: points at the right guide).

| config | asked/answered/judged | correct | fact recall | wrong facts | negatives abstained | false not-found | guides right | stale old fact | tokens in | out | reasoning | calls | secs p50 / p95 | first token s | format fails / errors | from a copy only |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| librarian-keyword-10x-default | 61/61/61 | 33% | 29% | 8% | 5/8 | 2% | 8/8 | 4/8 | 1060 | 110 | 518 | 1.0 | 5.8 / 33.4 | 1.1 | 2 | 0 |
| librarian-light-10x-default | 61/61/61 | 30% | 34% | 2% | 3/8 | 0% | 8/8 | 1/8 | 1116 | 117 | 550 | 1.0 | 5.4 / 23.4 | 1.1 | 6 | 0 |
| librarian-heavy-1x-default | 61/61/61 | 25% | 32% | 4% | 1/8 | 0% | 8/8 | 2/8 | 1207 | 126 | 490 | 1.0 | 5.9 / 17.1 | 1.1 | 6 | 0 |
| librarian-light-1x-default | 61/61/60 | 28% | 30% | 6% | 4/8 | 0% | 8/8 | 2/7 | 1228 | 130 | 550 | 1.0 | 5.8 / 15.6 | 1.1 | 3 | 0 |
| librarian-light-10x-low | 61/61/61 | 26% | 30% | 4% | 3/8 | 0% | 8/8 | 2/8 | 1166 | 125 | 459 | 1.0 | 5.2 / 15.5 | 1.2 | 4 | 0 |
| reader-keyword-10x | 61/61/61 | 38% | 33% | 8% | 8/8 | 22% | 8/8 | 4/8 | 1443 | 299 | – | 1.0 | 3.7 / 5.9 | – | 0 | 0 |
| reader-light-10x | 61/61/61 | 48% | 44% | 2% | 8/8 | 18% | 8/8 | 1/8 | 1555 | 304 | – | 1.0 | 3.6 / 6.0 | – | 0 | 0 |
| reader-heavy-1x | 61/61/61 | 43% | 47% | 8% | 8/8 | 9% | 8/8 | 2/8 | 1589 | 318 | – | 1.0 | 3.9 / 5.8 | – | 0 | 0 |

Wrong facts: share of answers (negatives aside) that assert a wrong fact or contradict a key fact. Tokens: per answer, all model calls (the Librarian's writer and support check; the reader's one call). First token: the Librarian writer's first streamed token (reasoning included). From a copy only: the passages held a distractor made from the gold note but not the note itself (the answer can still be right).

### Writer effort: DeepSeek Flash at its default and at its lowest ("low") effort

| effort | correct | fact recall | wrong facts | reasoning tokens | first token s p50 | secs p50 / p95 |
|---|---|---|---|---|---|---|
| default | 30% | 34% | 2% | 550 | 1.1 | 5.4 / 23.4 |
| low | 26% | 30% | 4% | 459 | 1.2 | 5.2 / 15.5 |

### Judge spot-check sheet

Ten verdicts picked at random, for a person to check against the answers (raw replies in the judge stage's raw folder).

- **G10** (reader-keyword-10x, expect guide): facts –; wrong []; abstained False; guide True. Answer: Recommended guide: roblox-code-review The guide ranks findings in three tiers: - **Critical:** Client-authoritative rewards, damage or currency, unvalidated remote args, purchases granted outside `ProcessReceipt` or non-idempotent, and data saved with `SetAsync` [1]. - **High:** Data loss paths suc…
- **M09** (librarian-heavy-1x-default, expect answer): facts The driver owns the vehicle: assign on s=absent, Test ownership bugs with two or more cli=absent; wrong [False, False]; abstained False; guide False. Answer: 1. who should own a vehicle while someone drives it, and how many clients do you need to…: - A driven car must be owned by the driver's client so input applies immediately. (Genre Playbook - Racing and Driving › Network ownership) - When the driver owns the assembly, the driver's machine is authori…
- **P21** (librarian-light-1x-default, expect answer): facts Huge values lose precision; clamp and ch=present, Every stored number needs a container ty=absent, Scale rewards with progress so small bon=present; wrong [False]; abstained False; guide False. Answer: 1. my money counter stopped going up properly once it got into the quadrillions, and one…: - A player's balance turning into NaN comes from overflow to inf and then inf - inf, and is fixed with clamp and isfinite checks before saving. (Big Numbers Precision and Formatting › Common mistakes) - A fix…
- **N01** (librarian-light-10x-low, expect not_found): facts –; wrong []; abstained False; guide False. Answer: The writer's answer could not be checked against the passages, so here is what the knowledge base holds: 1. how do I turn on Nanite for foliage in Unreal Engine 5: - Foliage Props and Scatter › Props in general: - **Kits over one-offs.** A zone kit of 50 to 90 models (lamps, rocks, fences, crates, …
- **S04** (librarian-light-10x-low, expect answer): facts One mesh may now hold up to 40,000 trian=contradicted, Above the limit, split the mesh by mater=absent; wrong [True]; abstained False; guide False. Answer: 1. how many triangles can one mesh have before I have to split it: - A single mesh cannot exceed 20,000 triangles on Roblox. (Quick notes - Meshing Voxels for Roblox - Greedy Meshing and Export › Four levels of meshing) - The 20,000-triangle limit applies per mesh, not per model, so larger models m…
- **G07** (reader-heavy-1x, expect guide): facts –; wrong []; abstained False; guide True. Answer: Recommended guide: game-bible (section "Writing it for an existing game") Two points from the excerpt: anything taken from the place rather than from the user is marked `(inferred)`, and pillars written from observation are inferred too [1]. The Core loop should be written from what a playtest show…
- **P11** (reader-heavy-1x, expect answer): facts A heavy hit freezes for about 66 ms (the=absent, Each hit has one strength value from whi=present, Important stops should be 66 ms or more =absent; wrong [False]; abstained False; guide False. Answer: The knowledge base gives a formula for the freeze, but no numbers for the wobble. **Freeze:** The reference project measured hit-stop on the attacker as `stop = 0.03 + 0.045 x strength` seconds [1]. Strengths ran from about 0.3 to 1.3, set per animation marker [1]. By that formula, that works out t…
- **Y07** (reader-heavy-1x, expect answer): facts WinchForce must be greater than mass tim=present, Use a positive WinchSpeed and change Win=absent; wrong [False]; abstained False; guide False. Answer: The passages don't give a full troubleshooting list, but these are the likely causes: - **Winch not enabled:** the winch is controlled by `WinchEnabled`, so make sure it is on [1]. - **Speed is zero or negative:** `WinchSpeed` must be positive. A negative value is treated as 0 and does not reverse …
- **P36** (librarian-light-1x-default, expect answer): facts Use attack tokens: a token budget per ta=absent, A coordinator assigns roles such as pinn=absent, Tokens are released on every exit path=absent; wrong [True]; abstained False; guide False. Answer: 1. all twenty enemies attack me at the exact same moment and it feels unfair: - Limit how many enemies attack one player at once; the reference allowed 2 attackers per target. (NPC AI and Behaviour › Attack tokens) - Enemies without a token circle or taunt, since six attacking at once are a stun-lo…
- **S05** (librarian-heavy-1x-default, expect answer): facts Icons fill 76% of the frame=present, Fill is computed from bounds, centred on=present; wrong [False]; abstained False; guide False. Answer: 1. how much of the frame should an item icon fill: - Icons now fill 76% of the frame instead of 82%, still on a 45-degree diagonal and centred. (Icon Framing Standard - Revised › (whole file)) - The fill is still computed from bounds and never framed by hand. (Icon Framing Standard - Revised › (who…

## 4. Why answers failed

Each answer that is not correct gets the first cause that explains it: format failure (the writer gave no usable answer), guide handling, distractor or stale won (a wrong or old fact, or an answer to a negative), retrieval miss (no gold note among the passages), check dropped a true claim (the writer had it, a check removed it), writer omission (the gold passage was there, the fact was not used).

| config | retrieval miss | writer omission | check dropped a true claim | distractor or stale won | format failure | guide handling |
|---|---|---|---|---|---|---|
| librarian-keyword-10x-default | 17 | 16 | 0 | 7 | 1 | 0 |
| librarian-light-10x-default | 9 | 26 | 0 | 4 | 4 | 0 |
| librarian-heavy-1x-default | 5 | 32 | 0 | 5 | 4 | 0 |
| librarian-light-1x-default | 4 | 33 | 0 | 5 | 1 | 0 |
| librarian-light-10x-low | 10 | 28 | 0 | 5 | 2 | 0 |
| reader-keyword-10x | 18 | 16 | 0 | 4 | 0 | 0 |
| reader-light-10x | 10 | 21 | 0 | 1 | 0 | 0 |
| reader-heavy-1x | 5 | 27 | 0 | 3 | 0 | 0 |

**retrieval miss** (examples):

- P08, librarian-keyword-10x-default: "a tester's save got corrupted, how do I wipe only their progress so it doesn't come back next time they join" Missing: Kick the player from all servers and wait for the session lock to be released b…; Find every data store and key format the code uses, including legacy reads, so …; Dump the current values to disk before removing them. Answer: 1. a tester's save got corrupted, how do I wipe only their progress so it doesn't come back…: - The knowledge base passages do not cover wiping an individual tester's corrupted save. (Device tester (team expert) › (whol…
- P11, librarian-keyword-10x-default: "how long should everything freeze for a split second when a heavy punch connects, and how strong should the wobble be" Missing: A heavy hit freezes for about 66 ms (the tuning note) or for 80 to 150 ms (Comb…; Each hit has one strength value from which stop, trauma and kick are derived; Important stops should be 66 ms or more because 50 ms is only 1-2 frames at 30 …. Answer: 1. how long should everything freeze for a split second when a heavy punch connects, and…: - A slow call does not just... (Prompt keys collide with ability, Never work in an AutoRecovery - field notes) - A tool call run…
- P12, librarian-keyword-10x-default: "kids open hundreds of crates and never see the legendary one, how can I promise they get it eventually" Missing: Use a pity system that guarantees the rare item after a number of failed tries; Keep drops in named, versioned drop tables as data; Keep weighted picks and independent chances in separate fields. Answer: 1. kids open hundreds of crates and never see the legendary one, how can I promise they get…: - Odds should not be shown only as 'Rare / Epic / Legendary' with no numbers. (Policy and Age Rules 2026 in brief (2)) - Base…

**writer omission** (examples):

- P09, librarian-keyword-10x-default: "people find a way to jump off the edge of the world and just fall forever, how do I bring them back" Missing: Use a server-side bounds sweep rather than relying only on Touched on a kill pl…; Put boundary walls in their own collision group so combat raycasts ignore them. Answer: 1. people find a way to jump off the edge of the world and just fall forever, how do I…: - The server remembers each player's last grounded position and puts them back if they fall too far below it. (Layout follows the …
- P26, librarian-keyword-10x-default: "in a storm the rumble should arrive a few seconds after the flash, and everybody should see the same bolt" Missing: The server sends a seed, position and strike time so every client renders the i…; The strike lasts under about 0.3 s with a flicker. Answer: 1. in a storm the rumble should arrive a few seconds after the flash, and everybody should…: - Thunder arrives later the further away the strike was. (Lightning Strikes and Thunder - Procedural Bolts and Sound Delay) - …
- P33, librarian-keyword-10x-default: "teammates should be able to pick each other up instead of dying the instant their health runs out" Missing: Intercept lethal damage in the damage core and down the player instead of letti…; The server owns a LifeState attribute; Revives go through a ProximityPrompt with server checks, and bleed-out uses ser…. Answer: 1. teammates should be able to pick each other up instead of dying the instant their health…: - Co-op games should treat dying as a state, not an event, with downed players revivable by a teammate within a time limit. (…

**distractor or stale won** (examples):

- N03, librarian-keyword-10x-default: "validate StoreKit 2 receipts for my iOS app's in-app purchases" Missing: –. Answer: 1. validate StoreKit 2 receipts for my iOS app's in-app purchases: - A gift purchase whose recipient is a client argument resolved at grant time should store the gift intent server-side before prompting. (Economy Exploi…
- N05, librarian-keyword-10x-default: "which Kubernetes horizontal pod autoscaler settings should our game backend use" Missing: –. Answer: 1. which Kubernetes horizontal pod autoscaler settings should our game backend use: - HttpService allows about 500 requests per minute per server, and one production game switched to one request per batch after hitting …
- N08, librarian-keyword-10x-default: "what fails a Nintendo Switch lot check certification" Missing: –. Answer: 1. what fails a Nintendo Switch lot check certification: - A SEV2 incident is defined as core loop broken for many players with no lasting harm. (Quick notes - Live Incident Response and Post-Mortems) Quotes: > "**SEV2*…

**format failure** (examples):

- P41, librarian-keyword-10x-default: "the name labels above fifty players' heads are tanking my framerate" Missing: Disable the built-in displays (DisplayDistanceType = None) where custom tags ex…; Create tags with a client-side tag manager, not on the server; Use tiers, declutter and a slow refresh, and never show enemy tags through walls. Answer: The writer's answer could not be checked against the passages, so here is what the knowledge base holds: 1. the name labels above fifty players' heads are tanking my framerate: - Chase Cameras and Speed Effects for Vehi…
- P08, librarian-light-10x-default: "a tester's save got corrupted, how do I wipe only their progress so it doesn't come back next time they join" Missing: Kick the player from all servers and wait for the session lock to be released b…; Find every data store and key format the code uses, including legacy reads, so …; Dump the current values to disk before removing them. Answer: No writer answered in time, so here is what the knowledge base holds: 1. a tester's save got corrupted, how do I wipe only their progress so it doesn't come back next time they join: - Resetting One Player's Data Safely…
- P12, librarian-light-10x-default: "kids open hundreds of crates and never see the legendary one, how can I promise they get it eventually" Missing: Use a pity system that guarantees the rare item after a number of failed tries; Keep drops in named, versioned drop tables as data; Keep weighted picks and independent chances in separate fields. Answer: The writer's answer could not be checked against the passages, so here is what the knowledge base holds: 1. kids open hundreds of crates and never see the legendary one, how can I promise they get it eventually: - Error…

### The vault disagrees with itself

Answer keys where two notes in the vault give different answers. Either note's claim is graded as correct; these are findings for vault upkeep, not exam noise.

- **P11**: How long hits freeze: Combat Animation Craft gives light hits 30-60 ms and heavy hits 80-150 ms; Hit-Stop and Screen Shake Tuning by Numbers gives light hits no hit-stop and a heavy hit 66 ms.

## 5. What makes it good, what makes it bad

- At 10x, the right note is in the first five results for light 74%, keyword 59%.
- light from 1x to 10x: hit@5 84% → 74% (-10 points).
- keyword from 1x to 10x: hit@5 76% → 59% (-17 points).
- keyword 1x can tell "not in the knowledge base" best by its keyword coverage (97% balanced: keeps 93% of answerable, refuses 100% of negatives).
- keyword 10x can tell "not in the knowledge base" best by its keyword coverage (83% balanced: keeps 75% of answerable, refuses 92% of negatives).
- light 1x can tell "not in the knowledge base" best by its keyword coverage (97% balanced: keeps 93% of answerable, refuses 100% of negatives).
- light 10x can tell "not in the knowledge base" best by its keyword coverage (83% balanced: keeps 75% of answerable, refuses 92% of negatives).
- heavy 1x can tell "not in the knowledge base" best by its keyword coverage (97% balanced: keeps 93% of answerable, refuses 100% of negatives).
- keyword 10x, stale pairs: the newer note ranks above the outdated one 100% of the time when it carries `supersedes:`, 20% when only its text says so; the outdated note is still in the top 5 for 80% / 100%.
- light 10x, stale pairs: the newer note ranks above the outdated one 100% of the time when it carries `supersedes:`, 20% when only its text says so; the outdated note is still in the top 5 for 80% / 100%.
- The change that hurts most against the app as shipped: lexical for heavy 1x (-15 points of hit@5).
- A change that beats the app as shipped: nolinks for light 1x (+4 points of hit@5).
- Answers correct: reader-light-10x 48%, reader-heavy-1x 43%, reader-keyword-10x 38%, librarian-keyword-10x-default 33%, librarian-light-10x-default 30%, librarian-light-1x-default 28%, librarian-light-10x-low 26%, librarian-heavy-1x-default 25%.
- Most wrong facts: reader-keyword-10x (8% of its answers).
- The most common reason an answer failed: writer omission (199 of 323 failures).

### What these numbers show and what they do not

- They show how often each method puts the right note in front of the model, on 160 retrieval questions over a vault padded with look-alike distractors, and how good the answers are on 61 questions graded by one strong judge.
- Sets are small (3-15 questions each outside paraphrase): a difference of one or two questions is noise. Treat gaps under about 10 points on a small set as a tie.
- The distractors are generated from the vault's own notes (split, summarised, reworded, mixed); real vault growth may be easier or harder to search.
- Latency was measured on this Mac while other work may have been running (load averages are saved with each build). The model calls' times depend on the providers' load at the time.
- The judge is a model: its verdicts are kept raw, and a spot-check sheet is above. Fact matching is strict on numbers and names.
- The failure causes are assigned by rules from what was kept (passages, the writer's raw claims, the checks); an answer has one cause even when several played a part.
