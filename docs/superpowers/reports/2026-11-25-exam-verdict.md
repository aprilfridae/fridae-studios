# The knowledge exam: what makes it good, what makes it bad

Plain-English verdict on the full exam of Fridae's knowledge system (search plus the Librarian), run 2026-11-24.

- The numbers: [full report](2026-11-25-knowledge-exam.md) (part 1 alone: [here](2026-11-25-knowledge-exam-part1.md)).
- Part 2 added Heavy on a vault 10 times bigger.

**Scales:**
- **Today's size (1x):** the vault as it is, about 20,000 passages.
- **10x:** the same vault padded with look-alike notes made from its own notes (splits, summaries, rewordings, mixes), about 205,000 passages.

**Questions:**
- 160 search questions.
- 61 full questions answered and graded:
  - The Librarian answered each one, and Claude Sonnet answered each one from the same passages.
  - Claude Opus graded all the answers strictly: every key fact present and nothing wrong.

## The short version

1. **Search is good. Answering is the weak link.**
   - Light search puts the right note in the top 5:
     - 84% of the time at today's size.
     - 74% at 10x.
   - Given those passages, the Librarian's cheap writer gets the whole answer right only 25–33% of the time. Sonnet, reading the same passages, gets 38–48%.
   - The commonest failure, by far, is the writer leaving facts out (199 of 323 failed answers).
2. **Allowing more claims did not help, so it was undone.**
   - The writer was allowed at most 2 claims per part of a question, and no Librarian setup answered any question needing 3 facts fully (0 of 17).
   - I let a one-part question get 4 claims and re-ran all 61 questions in 4 setups.
   - The writer did use the room: 2.7 claims an answer instead of 1.7. But the extra claims were not the missing facts:

     | | Light 1x | Light 10x | Heavy 1x | Keyword 10x |
     |---|---|---|---|---|
     | Correct, before | 28% | 30% | 25% | 33% |
     | Correct, after | 28% | 26% | 28% | 23% |
     | Wrong facts, before | 6% | 2% | 4% | 8% |
     | Wrong facts, after | 8% | 6% | 9% | 8% |

   - Three-fact questions stayed at 0–1 of 17. So the limit was not the cause; reverted (361caf0).
   - Sonnet also gets only 1 of 17 three-fact questions, which suggests the 3 passages of about 600 characters each often don't hold every fact. That idea is untested.
3. **A speed bug, fixed in the repo.**
   - At 10x, every Light search took about 3.2 s, although the index answered in 0.09 s.
   - The cause: after the index answered, Fridae also ran the full keyword search over every note, just to find the matching "symptom" lines.
   - It now scores only the symptom lines, so a Light search at 10x should take well under a second.
4. **Light is the right default. Keyword-only does not scale.** At 10x, keyword-only search:
   - finds the right note in the top 5 only 59% of the time;
   - takes 2.9 s per search;
   - takes 30 s to update after one note changes.
5. **Heavy finds the right note more often, and the gap grows with the vault. It does not give better answers.**

   | | Heavy 1x | Light 1x | Heavy 10x | Light 10x |
   |---|---|---|---|---|
   | Right note in top 5 | 86% | 84% | 81% | 74% |
   | Right note first | 69% | 65% | 59% | 55% |
   | Engine time per search | 1.5 s | 0.02 s | 1.7 s | 0.09 s |
   | Build from nothing | 21 min | 8 min | 2.0 h | 48 min |
   | Disk | 0.8 GB | 45 MB | 1.8 GB | 350 MB |

   - Heavy uses about 1.2–1.5 GB of memory plus its Qdrant database.
   - Librarian answers with Heavy were no better: 25% right at today's size and 23% at 10x, against Light's 28% and 30%.
   - Sonnet reading Heavy's 10x passages got 46%, against 48% with Light's.
   - So the extra 7 points of retrieval at 10x get lost in the writer.
   - At 10x, 3 of 160 Heavy searches through Fridae went over the 3 s limit and silently fell back to the keyword search. Part of that is the symptom-line slowdown, now fixed.
6. **Guides work exactly as intended.** Every setup pointed to the right guide on all 8 guide questions. The "excerpt" tag and the "point, don't summarise" rule both held.
7. **Gemini's "lower DeepSeek's effort" tip doesn't pay off here.**
   - At low effort the writer uses 17% fewer thinking tokens and is no faster.
   - It is not more accurate either: 26% right against 30%, a difference that is within noise.
   - On a flat-price plan the saving is worth nothing. Leave it at the default.
8. **"Not in the knowledge base" is handled badly by the Librarian.**
   - On 8 questions the vault cannot answer (Unreal, Kubernetes, Switch certification and so on), it said so only 1–5 times. Otherwise it stated loosely related facts.
   - Sonnet said so 8 of 8 times, but also wrongly said "not found" on 9–22% of questions the vault *could* answer.
   - The exam found a cheap, reliable signal: how much of the question's wording the best note covers.
     - At today's size it separates answerable from unanswerable questions 97% of the time.
     - At 10x it does so 83% of the time.
     - This is the next thing to build.

## What makes it good

- **Hybrid search earns its keep.** Removing either half costs points:
  - Light with keywords alone loses 13 points of top-5.
  - With meaning alone it loses 3 points of top-5 and 7–9 of first place.
  - Heavy's reranker adds 6 points of first place.
- **The context line and gist on every passage matter.** Building the index without them costs 5–8 points.
- **`supersedes:` marks work perfectly.**
  - With a mark, the newer note ranks above the outdated one 100% of the time, at both sizes.
  - With no mark, when only the text says "this replaces…", the newer note ranks first only 0–40% of the time.
- **Guides are always found:** in the top 5 every time except one guide question with Light at 10x.
- **The Librarian rarely says "not found" when the answer is there:** 0–2% of answerable questions, against Sonnet's 9–22%.
- **Wrong facts are rare** with Light: 2–6% of Librarian answers.
- **The silent fallback never fired.** No search's index answer hit the 3 s limit at either size, so the index's answer was always the one used.

## What makes it bad

- **The writer.** This is the biggest loss.
  - The cheap writer gets about 15 points fewer answers right than Sonnet on identical passages.
  - It picks weaker facts: on two-fact questions it gets about 0.9 facts per answer, against Sonnet's 1.2.
  - Letting it say more did not fix this (point 2).
- **Slow tails.**
  - A typical Librarian answer takes about 5.5 s, but the slowest 1 in 20 takes 15–33 s. Sonnet's slowest take 6 s.
  - 2–6 of 61 answers fell back to "could not be checked" or "no writer answered in time".
- **Unanswerable questions get padded answers** instead of "not in the knowledge base" (point 8 above).
- **At 10x, Light needs time and memory:**
  - 48 min to build from nothing;
  - 52 s to open;
  - 1.3 GB of memory.

  This is fine overnight. It's a cost to know about.
- **The vault contradicts itself in at least one place.** How long a heavy hit freezes:
  - Combat Animation Craft says 80–150 ms.
  - Hit-Stop Tuning says 66 ms.

  Vault upkeep should flag this kind of disagreement. Now that the exam has found this one, it's a ready test case.

## Recommendations, in order

1. **Keep Light as the default.**
   - Ship the symptom-line speed fix (in the repo).
2. **Build a "not in the knowledge base" gate before the writer.**
   - When the best passage covers too little of the question's wording, answer "not in the knowledge base" without calling the writer. This is also faster.
   - Measure the threshold on the exam's questions, so it doesn't start refusing answerable ones.
3. **Leave DeepSeek's effort at the default.**
4. **Have vault upkeep add `supersedes:` marks** when it files a note that replaces another. The text alone is not enough.
5. **Keep Heavy as an option, not the default.**
   - It finds more at 10x (81% against 74% top-5).
   - That only pays once the writer improves.
   - Until then it costs a 2-hour build, 1.8 GB of disk and seconds per search for no better answers.
6. **Consider a stronger writer only for the answers that fail the checks.** This would be a second try, not the default. It would cut the 15–33 s tails and the "could not be checked" answers. Measure before deciding.

## Caveats

- The question sets are small: 10–15 questions each outside the paraphrase set. Gaps under about 10 points on those are ties.
- The 10x vault is padded with generated look-alikes. Real growth may be easier or harder to search.
- The judge is a model. Its raw replies are kept, and the report has a spot-check sheet.
