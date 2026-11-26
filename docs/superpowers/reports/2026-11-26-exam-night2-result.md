# Night 2 knowledge exam: what it found

Run on commit ccea240, branch `feature/knowledge-exam`. It used Light and keyword search at today's vault size, on 84 answer questions: 45 answerable, 8 guide questions and 31 unanswerable. Opus graded every answer.

The full report is in [2026-11-26-knowledge-exam-night2.md](2026-11-26-knowledge-exam-night2.md).

**How the run went.** The run itself took 60 minutes of awake time and every answer was graded. The Mac was on battery at 8% with the lid closed, so it slept between short wake-ups. As a result the run started at 00:00 Wed and only finished at 07:18 Thu. No answer failed because of it.

Next time, leave the Mac plugged in: caffeinate keeps the Mac awake only on power.

**Read the headline "correct" carefully.** The report's correct rate counts the unanswerable and guide questions too. For example, the Librarian on Light scores 42% on that measure. On the 45 questions that have a real answer, the picture is much lower:

| | answerable correct | key facts given |
|---|---|---|
| Librarian (Light) | 5 / 45 | 26% |
| Sonnet reader (Light) | 13 / 45 | 40% |

## 1. How much the writer varies (the same Librarian run twice)

Very little. Between the two runs:
- only 1 of the 84 questions changed between right and wrong;
- 6 single facts changed;
- correct was 42% against 40%, and facts given 29% against 26%.

So treat any difference of about 3 points or less as noise. By that measure:
- **low effort** (40%) is a tie with default;
- earlier wins of 2 to 4 points also fall inside the noise.

## 2. Does giving the writer 5 passages instead of 3 help?

**For 3-fact questions, no.** Neon asked whether the writer misses facts because it gets too few passages ("passage starvation"). The answer is no:
- The Librarian gets **0 of 17** 3-fact questions fully right with either 3 or 5 passages.
- It gives the same 10% of those facts either way.
- Sonnet gets 2 of 17 either way, with facts given rising from 22% to 27%.

The facts are usually already in the passages. The most common reason an answer fails is "writer omission": the right passage was there and the writer left the fact out. The writer only writes a claim or two per question part, so a 3-fact question can't be fully answered. More passages don't change that. Earlier we tried letting the writer give up to 4 claims; it was no better, and we reverted it.

**For declining questions it can answer, yes.** With 5 passages:
- The Librarian wrongly said "not in the knowledge base" on 1 answerable question instead of 5 (11% → 2%).
- Sonnet did so on 7% instead of 16%.

Correct and wrong facts stayed level. The cost is about 300 more tokens per answer (1271 → 1565 in) and about 1 s more (3.3 → 4.2 s median).

Worth considering: k=5 for the Librarian, to stop it turning down questions it can answer. It won't fix the missing facts in 3-fact answers. That needs the writer to say more, or a better writer.

## 3. The 35 unanswerable questions

All 35 are in the search tests; 31 of them are in the answer tests.

**The writer's own "not found" works.** On Light, the Librarian declined 22 of the 31. Every one of the 9 it got wrong was the same failure: the guide-pointer step answered before the writer ever saw the question. For example:
- "a recipe for chocolate chip cookies" was pointed to the `unslop` guide;
- "Kubernetes autoscaler settings" was pointed to `roblox-architecture/streaming`.

On every unanswerable question that reached the writer, the writer declined.

**Keyword mode is worse.** It sent all 31 unanswerable questions to a guide pointer, so none were declined.

**Sonnet declined all 31 in every setup.**

**The next fix.** The guide pointer fires on a weak match: a single section heading such as "Assets" or "8. Visuals". It should only fire when the match is strong, and otherwise pass the question to the writer, which already declines correctly. That would likely take Light from 22/31 to about 31/31.

The search's own scores can't make this call reliably:
- The best single signal, the share of the question's words found in the results, is right only 79% of the time.
- It keeps 83% of answerable questions and turns away 74% of unanswerable ones.

## Other numbers from the run

- **Search on Light:** the right note is in the top 5 for 84% of answerable questions; keyword gets 76%.
- **Light vs keyword:** Light is also faster (0.03 s against 0.33 s per search) and never fell back to keyword search.
- **Link following:** turning off [[link]] following raised top-5 hits by 4 points on Light. The last run got the same at today's vault size, but at 10x the vault size links helped (2 to 3 points). So leave link following on.
- **Text-only "this replaces X" notes:** when a newer note only says in its text that it replaces an older one, the old note still ranks first. A `supersedes:` mark fixes that every time.
