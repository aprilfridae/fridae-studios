# Knowledge exam

Made 2026-11-26T07:18 on this Mac (arm64, Python 3.14.3). Corpora: 1x = full-1x-f22e895581dc885c (about 20551 chunks). Search code 069e48106cdd.

Code measured: commit `ccea240b67c33ceeda14eba8de90d4291b43f9ab` on `feature/knowledge-exam`. Each saved result carries the commit it was measured at (`_measured`); results from other commits: none.

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
| light | 1x | paraphrase | 58 | 53% | 71% | 71% | 0.63 | 53% | 71% | 71% |
| light | 1x | exact | 12 | 92% | 100% | 100% | 0.94 | 92% | 100% | 100% |
| light | 1x | old33 | 29 | 79% | 93% | 100% | 0.88 | 79% | 93% | 100% |
| light | 1x | typo | 15 | 53% | 80% | 80% | 0.66 | 53% | 80% | 80% |
| light | 1x | guide | 10 | 80% | 100% | 100% | 0.90 | 80% | 100% | 100% |
| light | 1x | stale | 10 | 50% | 70% | 70% | 0.62 | 50% | 70% | 70% |
| light | 1x | multihop | 10 | 80% | 90% | 100% | 0.88 | 80% | 90% | 100% |
| light | 1x | answerable | 144 | 65% | 82% | 84% | 0.74 | 65% | 82% | 84% |

Strict: the gold note itself. Content-right: the gold note or a distractor the corpus made from it (a split, summary or rewording of it, or a mix holding one of its sections; corpus.py records each distractor's source notes in distractors.json). Echo notes, which only borrow the gold note's title words, never count.

Multi-hop, stale pairs, guides and game scope:

| method | scale | multi-hop both@5 / either@5 | stale: new above old (marked / text only) | guide found@5 / with tag | game note @3: no filter → game=this | general @3 with game=general |
|---|---|---|---|---|---|---|
| keyword | 1x | 50% / 90% | marked 100% (outdated in top 5: 100%, mean rank 2.4) / text only 0% (outdated in top 5: 100%, mean rank 1.4) | 100% / 100% | 100% → 100% | 50% |
| light | 1x | 60% / 100% | marked 100% (outdated in top 5: 100%, mean rank 2.0) / text only 0% (outdated in top 5: 100%, mean rank 1.2) | 100% / 100% | 100% → 100% | 100% |

Cost per search and per index:

| method | scale | tokens k=3 | k=5 | k=8 | engine s p50/p95 | rbx_vault s p50/p95 (k=5) | over 3 s | cold build | open (warm) | one-note update | RSS open MB | peak RSS MB | Qdrant RSS MB | disk MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| keyword | 1x | 631 | 894 | 1292 | 0.325 / 0.487 | 0.389 / 4.225 | 19 | 3 s (keyword indexes) | – | 7.47 s | 158 | 210 | – | 14 |
| light | 1x | 900 | 1327 | 1745 | 0.032 / 0.071 | 0.307 / 0.390 | 0 | 38 s (0 embedded, 24224 cached) | 8.3 s | 0.10 s | 493 | 592 | – | 45 |

Tokens: rbx_vault's search text as an agent reads it (about 4 characters a token). Cold build: from no index (vectors already in the exam's vector cache are not embedded again; the counts say how many were). Open: an existing index loaded and checked. Over 3 s: searches the app would have answered with the keyword search instead (its timeout).

Silent fallback: the same searches (k=5) with rbx_vault's real 3 s timeout. A search that gets no index answer in time is answered by the keyword search, and the agent is not told.

| method | scale | searches | fell back to keyword | rbx_vault s p50 / p95 |
|---|---|---|---|---|
| light | 1x | 183 | 0 (0%) | 0.21 / 0.31 |

### Telling "not in the knowledge base" apart

Each answerable question's and each negative question's best score, and the threshold that separates them best (balanced accuracy: the mean of answerable kept and negatives refused). Negatives: 35.

| method | scale | signal | threshold | balanced | answerable kept | negatives refused |
|---|---|---|---|---|---|---|
| keyword | 1x | keyword coverage | 0.722 | 79% | 83% | 74% |
| keyword | 1x | keyword BM25 | 4.970 | 72% | 67% | 77% |
| light | 1x | keyword coverage | 0.722 | 79% | 83% | 74% |
| light | 1x | keyword BM25 | 4.970 | 72% | 67% | 77% |
| light | 1x | dense cosine | 0.731 | 71% | 54% | 89% |

## 2. Ablations

Each row switches one piece off (or changes it) and compares with the app as shipped on the same index and questions. Deltas in percentage points over the answerable questions; negative means the piece helps.

| method | scale | variant | hit@1 | hit@5 | MRR | Δ hit@1 | Δ hit@5 | Δ MRR (×100) |
|---|---|---|---|---|---|---|---|---|
| keyword | 1x | nolinks | 53% | 76% | 0.61 | +0 | +1 | +0 |
| light | 1x | dense | 58% | 81% | 0.68 | -7 | -3 | -6 |
| light | 1x | lexical | 55% | 71% | 0.63 | -10 | -13 | -12 |
| light | 1x | nolinks | 65% | 88% | 0.75 | +0 | +4 | +0 |
| light | 1x | index: no context line | 58% | 79% | 0.68 | -8 | -5 | -6 |
| light | 1x | index: no gist | 60% | 76% | 0.68 | -6 | -8 | -6 |

Variants: dense = the dense ranking alone; lexical = the keyword search's ranking alone (through the engine); sparse = bm25 sparse vectors alone; dense+sparse = the two fused without the keyword ranking; app+sparse = the app's plan with the sparse ranking added; norerank = Heavy without its reranker vote; nolinks = no [[link]] following. Index variants rebuild the index without the chunk's context line, or without the note's gist in it.

How many results to return (rbx_vault's text at k = 3, 5, 8):

| method | scale | k=3: hit, tokens | k=5 | k=8 |
|---|---|---|---|---|
| keyword | 1x | 69%, 635 tok | 76%, 898 tok | 78%, 1297 tok |
| light | 1x | 78%, 899 tok | 81%, 1325 tok | 90%, 1748 tok |

## 3. Answers

84 questions (45 answer, 8 guide, 31 not_found), each with 2-4 key facts and some wrong facts. Librarian: the app's own answer path (split, writer, quote and support checks) through `fridae --librarian-exam`, the writer on the user's OpenCode plan. Reader: Claude Sonnet with the same passages, no tools, citing passages. Judge: Claude Opus, 8 answers a call, strict fixed prompt; its raw replies are kept. Correct: every key fact and no wrong one (negatives: says it is not in the knowledge base; guides: points at the right guide).

| config | asked/answered/judged | correct | fact recall | wrong facts | negatives abstained | false not-found | guides right | stale old fact | tokens in | out | reasoning | calls | secs p50 / p95 | first token s | format fails / errors | from a copy only |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| librarian-keyword-1x-default | 84/84/84 | 17% | 34% | 4% | 0/31 | 0% | 8/8 | 2/8 | 1146 | 67 | 391 | 1.0 | 3.8 / 18.3 | 1.4 | 1 | 0 |
| librarian-light-1x-default | 84/84/84 | 42% | 29% | 6% | 22/31 | 11% | 8/8 | 2/8 | 1271 | 76 | 353 | 1.0 | 3.3 / 13.0 | 1.3 | 2 | 0 |
| librarian-light-1x-low | 84/84/84 | 40% | 28% | 6% | 22/31 | 9% | 8/8 | 2/8 | 1271 | 79 | 264 | 1.0 | 3.0 / 12.0 | 1.3 | 2 | 0 |
| librarian-light-1x-k5 | 84/84/84 | 42% | 31% | 6% | 21/31 | 2% | 8/8 | 2/8 | 1565 | 91 | 397 | 1.0 | 4.2 / 15.5 | 1.4 | 1 | 0 |
| librarian-light-1x-rerun | 84/84/84 | 40% | 26% | 8% | 22/31 | 9% | 8/8 | 2/8 | 1242 | 76 | 362 | 1.0 | 3.6 / 16.6 | 1.4 | 4 | 0 |
| reader-keyword-1x | 84/84/84 | 60% | 46% | 4% | 31/31 | 18% | 8/8 | 2/8 | 1468 | 228 | – | 1.0 | 2.5 / 5.7 | – | 0 | 0 |
| reader-light-1x | 84/84/84 | 62% | 44% | 6% | 31/31 | 16% | 8/8 | 2/8 | 1592 | 228 | – | 1.0 | 2.7 / 5.8 | – | 0 | 0 |
| reader-light-1x-k5 | 84/84/84 | 62% | 48% | 6% | 31/31 | 7% | 8/8 | 2/8 | 2058 | 244 | – | 1.0 | 2.3 / 5.8 | – | 0 | 0 |

Wrong facts: share of answers (negatives aside) that assert a wrong fact or contradict a key fact. Tokens: per answer, all model calls (the Librarian's writer and support check; the reader's one call). First token: the Librarian writer's first streamed token (reasoning included). From a copy only: the passages held a distractor made from the gold note but not the note itself (the answer can still be right).

### Writer effort: DeepSeek Flash at its default and at its lowest ("low") effort

| effort | correct | fact recall | wrong facts | reasoning tokens | first token s p50 | secs p50 / p95 |
|---|---|---|---|---|---|---|
| default | 42% | 29% | 6% | 353 | 1.3 | 3.3 / 13.0 |
| low | 40% | 28% | 6% | 264 | 1.3 | 3.0 / 12.0 |

### Same questions, one thing changed

| comparison | config | correct | fact recall | wrong facts | negatives abstained | false not-found | 3-fact questions right | tokens in |
|---|---|---|---|---|---|---|---|---|
| the same Librarian run twice (the writer's own noise) | librarian-light-1x-default | 42% | 29% | 6% | 22/31 | 11% | 0/17 | 1271 |
|  | librarian-light-1x-rerun | 40% | 26% | 8% | 22/31 | 9% | 0/17 | 1242 |
| the Librarian with 5 passages a part instead of 3 | librarian-light-1x-default | 42% | 29% | 6% | 22/31 | 11% | 0/17 | 1271 |
|  | librarian-light-1x-k5 | 42% | 31% | 6% | 21/31 | 2% | 0/17 | 1565 |
| Sonnet with 5 passages a part instead of 3 | reader-light-1x | 62% | 44% | 6% | 31/31 | 16% | 2/17 | 1592 |
|  | reader-light-1x-k5 | 62% | 48% | 6% | 31/31 | 7% | 2/17 | 2058 |

### Judge spot-check sheet

Ten verdicts picked at random, for a person to check against the answers (raw replies in the judge stage's raw folder).

- **N13** (librarian-light-1x-k5, expect not_found): facts –; wrong []; abstained True; guide False. Answer: 1. how do I connect signals between nodes in Godot 4: not in the knowledge base.
- **N14** (librarian-light-1x-low, expect not_found): facts –; wrong []; abstained True; guide False. Answer: 1. set up Unreal Engine 5 Lumen lighting for an indoor level: not in the knowledge base.
- **E12** (librarian-light-1x-low, expect answer): facts Grant only in ProcessReceipt, never from=absent, Save first, then return PurchaseGranted=absent, Return NotProcessedYet when the player i=absent; wrong [False, False]; abstained False; guide False. Answer: 1. ProcessReceipt: - ProcessReceipt is set once in one server Script and handles every product. (Purchase Data and Receipts › What ProcessReceipt guarantees (Roblox reference, October 2026)) - A purchase is resolved only when the callback returns PurchaseGranted and Roblox records that on its backe…
- **N26** (librarian-light-1x-low, expect not_found): facts –; wrong []; abstained True; guide False. Answer: 1. how long does Nintendo take to approve a developer account: not in the knowledge base.
- **N10** (librarian-light-1x-low, expect not_found): facts –; wrong []; abstained True; guide False. Answer: 1. what did our 3 March 2025 playtest find about the fishing minigame: not in the knowledge base.
- **G02** (librarian-light-1x-default, expect guide): facts –; wrong []; abstained False; guide True. Answer: 1. walk me through verifying the signature on a Roblox webhook in an Express server: Recommended guide: roblox-open-cloud/webhooks — matched on "Node.js (Express)". Call rbx_guide with topic `roblox-open-cloud/webhooks` to load the full instructions.
- **N11** (reader-keyword-1x, expect not_found): facts –; wrong []; abstained True; guide False. Answer: Not in the knowledge base.
- **G01** (reader-light-1x-k5, expect guide): facts –; wrong []; abstained False; guide True. Answer: Recommended guide: roblox-testing/cloud-test-runner The passages also give some background on how this works. Open Cloud Luau Execution lets you POST a script to run against a place version on Roblox servers and poll for the result [3]. Anything that touches the engine needs a real place [3]. A tes…
- **P11** (reader-keyword-1x, expect answer): facts A heavy hit freezes for about 66 ms (the=present, Each hit has one strength value from whi=absent, Important stops should be 66 ms or more =present; wrong [False]; abstained False; guide False. Answer: A heavy hit's stop is listed at 66 ms, from the Heavy row of the tuning table [1]. The same passage says important stops on phones should be 66 ms or more, because a 50 ms stop is only 1 or 2 frames at 30 fps [1]. The excerpt doesn't label the other columns in that row (0.80, 0.25, 0.5%, 0.063), so…
- **P08** (librarian-light-1x-rerun, expect answer): facts Kick the player from all servers and wai=absent, Find every data store and key format the=absent, Dump the current values to disk before r=absent; wrong [False, False]; abstained False; guide False. Answer: 1. a tester's save got corrupted, how do I wipe only their progress so it doesn't come back…: - After removal, have the tester join and confirm a fresh profile with permanent purchases and no restored legacy data. (Resetting One Player's Data Safely › 5. Verify) Quotes: > "Confirm the fresh profile…

## 4. Why answers failed

Each answer that is not correct gets the first cause that explains it: format failure (the writer gave no usable answer), guide handling, distractor or stale won (a wrong or old fact, or an answer to a negative), retrieval miss (no gold note among the passages), check dropped a true claim (the writer had it, a check removed it), writer omission (the gold passage was there, the fact was not used).

| config | retrieval miss | writer omission | check dropped a true claim | distractor or stale won | format failure | guide handling |
|---|---|---|---|---|---|---|
| librarian-keyword-1x-default | 12 | 24 | 0 | 33 | 1 | 0 |
| librarian-light-1x-default | 5 | 33 | 0 | 11 | 0 | 0 |
| librarian-light-1x-low | 5 | 34 | 0 | 11 | 0 | 0 |
| librarian-light-1x-k5 | 4 | 33 | 0 | 12 | 0 | 0 |
| librarian-light-1x-rerun | 5 | 33 | 0 | 11 | 1 | 0 |
| reader-keyword-1x | 12 | 20 | 0 | 2 | 0 | 0 |
| reader-light-1x | 5 | 25 | 0 | 2 | 0 | 0 |
| reader-light-1x-k5 | 4 | 26 | 0 | 2 | 0 | 0 |

**retrieval miss** (examples):

- P12, librarian-keyword-1x-default: "kids open hundreds of crates and never see the legendary one, how can I promise they get it eventually" Missing: Use a pity system that guarantees the rare item after a number of failed tries; Keep drops in named, versioned drop tables as data; Keep weighted picks and independent chances in separate fields. Answer: 1. kids open hundreds of crates and never see the legendary one, how can I promise they get…: Recommended guide: market-research — matched on "3. What is fair to conclude". Call rbx_guide with topic `market-research` to…
- P13, librarian-keyword-1x-default: "only 500 of a special hat should ever exist and every copy needs its own number like #37" Missing: Keep the counter in a DataStore key updated only with UpdateAsync, enforcing su…; Every mint has a stable idempotency id (such as the PurchaseId); MemoryStore may only mirror the counter, never be its authority. Answer: 1. only 500 of a special hat should ever exist and every copy needs its own number like #37: Recommended guide: unslop — matched on "4. Copy". Call rbx_guide with topic `unslop` to load the full instructions.
- P15, librarian-keyword-1x-default: "my servers are full of people who leave their avatar standing there overnight just to collect the free reward" Missing: Lower and cap AFK income; Detect activity on the server from several observable signals, not movement alo…; Warn before an idle kick and exempt private and reserved servers. Answer: 1. my servers are full of people who leave their avatar standing there overnight just to…: Recommended guide: critic — matched on "3. An exploiter". Call rbx_guide with topic `critic` to load the full instructions.

**writer omission** (examples):

- P08, librarian-keyword-1x-default: "a tester's save got corrupted, how do I wipe only their progress so it doesn't come back next time they join" Missing: Find every data store and key format the code uses, including legacy reads, so …; Dump the current values to disk before removing them. Answer: 1. a tester's save got corrupted, how do I wipe only their progress so it doesn't come back…: - Kick the player, wait for the lock to release, then remove their data. (Resetting One Player's Data Safely) - Old data came…
- P09, librarian-keyword-1x-default: "people find a way to jump off the edge of the world and just fall forever, how do I bring them back" Missing: Use a server-side bounds sweep rather than relying only on Touched on a kill pl…; Return players to a server-tracked last safe position; Put boundary walls in their own collision group so combat raycasts ignore them. Answer: 1. people find a way to jump off the edge of the world and just fall forever, how do I…: Recommended guide: unslop/signs — matched on "Template look". Call rbx_guide with topic `unslop/signs` to load the full instructio…
- P26, librarian-keyword-1x-default: "in a storm the rumble should arrive a few seconds after the flash, and everybody should see the same bolt" Missing: The strike lasts under about 0.3 s with a flicker. Answer: 1. in a storm the rumble should arrive a few seconds after the flash, and everybody should…: - Thunder arrives later the further away the strike was. (Lightning Strikes and Thunder - Procedural Bolts and Sound Delay) - …

**distractor or stale won** (examples):

- N01, librarian-keyword-1x-default: "how do I turn on Nanite for foliage in Unreal Engine 5" Missing: –. Answer: 1. how do I turn on Nanite for foliage in Unreal Engine 5: Recommended guide: lihfolk-figma — matched on "Assets". Call rbx_guide with topic `lihfolk-figma` to load the full instructions.
- N03, librarian-keyword-1x-default: "validate StoreKit 2 receipts for my iOS app's in-app purchases" Missing: –. Answer: 1. validate StoreKit 2 receipts for my iOS app's in-app purchases: Recommended guide: roblox-security/checklist — matched on "Economy and data". Call rbx_guide with topic `roblox-security/checklist` to load the full ins…
- N05, librarian-keyword-1x-default: "which Kubernetes horizontal pod autoscaler settings should our game backend use" Missing: –. Answer: 1. which Kubernetes horizontal pod autoscaler settings should our game backend use: Recommended guide: roblox-analytics-liveops — matched on "Experience notifications (re-engagement)". Call rbx_guide with topic `roblox-…

**format failure** (examples):

- P11, librarian-keyword-1x-default: "how long should everything freeze for a split second when a heavy punch connects, and how strong should the wobble be" Missing: A heavy hit freezes for about 66 ms (the tuning note) or for 80 to 150 ms (Comb…; Each hit has one strength value from which stop, trauma and kick are derived. Answer: No writer answered in time, so here is what the knowledge base holds: 1. how long should everything freeze for a split second when a heavy punch connects, and how strong should the wobble be: - Hit-Stop and Screen Shake…
- M07, librarian-light-1x-rerun: "how should autosave relate to the session lock lease, and what does a profile library like ProfileStore do on a teleport" Missing: The autosave interval is shorter than the lease and renews it; With a session lock, the destination server waits until the source releases the…. Answer: No writer answered in time, so here is what the knowledge base holds: 1. how should autosave relate to the session lock lease, and what does a profile library like ProfileStore do on a teleport: - Session Locking and Pr…

### The vault disagrees with itself

Answer keys where two notes in the vault give different answers. Either note's claim is graded as correct; these are findings for vault upkeep, not exam noise.

- **P11**: How long hits freeze: Combat Animation Craft gives light hits 30-60 ms and heavy hits 80-150 ms; Hit-Stop and Screen Shake Tuning by Numbers gives light hits no hit-stop and a heavy hit 66 ms.

## 5. What makes it good, what makes it bad

- At 1x, the right note is in the first five results for light 84%, keyword 76%.
- keyword 1x can tell "not in the knowledge base" best by its keyword coverage (79% balanced: keeps 83% of answerable, refuses 74% of negatives).
- light 1x can tell "not in the knowledge base" best by its keyword coverage (79% balanced: keeps 83% of answerable, refuses 74% of negatives).
- keyword 1x, stale pairs: the newer note ranks above the outdated one 100% of the time when it carries `supersedes:`, 0% when only its text says so; the outdated note is still in the top 5 for 100% / 100%.
- light 1x, stale pairs: the newer note ranks above the outdated one 100% of the time when it carries `supersedes:`, 0% when only its text says so; the outdated note is still in the top 5 for 100% / 100%.
- The change that hurts most against the app as shipped: lexical for light 1x (-13 points of hit@5).
- A change that beats the app as shipped: nolinks for light 1x (+4 points of hit@5).
- Answers correct: reader-light-1x-k5 62%, reader-light-1x 62%, reader-keyword-1x 60%, librarian-light-1x-k5 42%, librarian-light-1x-default 42%, librarian-light-1x-rerun 40%, librarian-light-1x-low 40%, librarian-keyword-1x-default 17%.
- Most wrong facts: librarian-light-1x-rerun (8% of its answers).
- The most common reason an answer failed: writer omission (228 of 366 failures).

### What these numbers show and what they do not

- They show how often each method puts the right note in front of the model, on 183 retrieval questions over a vault padded with look-alike distractors, and how good the answers are on 84 questions graded by one strong judge.
- Sets are small (3-15 questions each outside paraphrase): a difference of one or two questions is noise. Treat gaps under about 10 points on a small set as a tie.
- The distractors are generated from the vault's own notes (split, summarised, reworded, mixed); real vault growth may be easier or harder to search.
- Latency was measured on this Mac while other work may have been running (load averages are saved with each build). The model calls' times depend on the providers' load at the time.
- The judge is a model: its verdicts are kept raw, and a spot-check sheet is above. Fact matching is strict on numbers and names.
- The failure causes are assigned by rules from what was kept (passages, the writer's raw claims, the checks); an answer has one cause even when several played a part.
