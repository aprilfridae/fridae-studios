# Overnight review: `git diff 853d22a 82c90a9` (feature/ui-overhaul)

Read-only review of the 28 files that landed overnight (Strong Lead, Fast Librarian, guide excerpt tag, index
quality, vault upkeep). Nothing in the repo was changed except this file. Probes ran against temporary folders
only (HOME_DIR and WORK pointed at the session scratchpad); the real notes folder, the app, Ollama and OpenCode
were not touched.

`tests.test_upkeep`, `tests.test_librarian_lookup`, `tests.test_notes` and `tests.test_rag_quality` pass (89
tests). The Rust tests were not run (a build writes to `target/`); the Rust findings come from reading the code.

Findings are ordered most severe first. "Confirmed" means reproduced with a probe; "by reading" means the code
path is unambiguous but was not executed; "unconfirmed" is marked as such.

---

## 1. Keyword search: a superseded note still ranks above its replacement — important (confirmed)

`rbxvault.py:1509-1512` (`_keyword_pool`)

```python
if sup:  # already scaled: the replacement wins a tie and comes first (interleave keeps this order)
    pool = [x for _, x in demote(pool, ...)]
pool = interleave(pool, k, ...)
```

`demote` returns `(new score, item)`, but the new score is thrown away and each item keeps its old score in
`x[0]`. `interleave` (`rbxvault.py:1456`) then sorts by `-x[0]` again, so the order `demote` just made is undone.
The comment "interleave keeps this order" is wrong.

It bites whenever the old note matches the query words better than its replacement: the earlier `demote` on
`hits` lowers the old note's score to the replacement's, then `cov * s` gives the old note the higher pool score
again (its coverage is higher).

Trigger:
- note "Throw meter position": "The throw meter sits at the bottom centre of the HUD, 24 px above the safe area."
- note "Throw meter v2" with `supersedes: "Throw meter position"`: "The throw meter sits on the left edge of the screen now."
- `rbx_vault search`, shelf `yours`, query `throw meter position HUD` (also `HUD throw meter safe area`)

Result: `1. Throw meter position ... (replaced by Throw meter v2)`, `2. Throw meter v2`. Traced scores: pool after
demote is `[v2 0.091, old 0.2]`, final order `[old, v2]`. The same pool feeds the Librarian's keyword `lookup`
and the keyword ranking the knowledge index fuses.

## 2. Librarian, keyword path: guide pointers name a topic `rbx_guide` does not know — important (confirmed)

`rbxvault.py:1934` (`_keyword_passages`): `"title": d.get("label") or d["title"]`

For a guide, `label` is the file's H1 and `title` is the rbx_guide topic. The Rust side builds the pointer from
`title` (`app-rs/src/swarm/librarian.rs:400-411`). The RAG path (`_rag_passages`) sends the real topic, so only
the keyword path is wrong, and that is the path every user without the knowledge index is on (`rag_mode` is off
by default).

Trigger: `rbx lookup` with `{"queries": ["how to make code testable with unit tests in roblox"]}` and no index
running. The guides hit comes back with title `Testing Roblox games`; the topic is `roblox-testing`. The board
answer reads "Call rbx_guide with topic `Testing Roblox games`", which `rbxkit_mcp._guide_path` does not resolve.
Seen for three of three guide hits in the probe.

## 3. Librarian quote check accepts an altered quote and posts the altered words as the quote — important (by reading)

`app-rs/src/swarm/librarian.rs:317-332` (`quote_found`), posted at `:441`

The fuzzy branch needs `ceil(0.9 * n)` of the quote's words anywhere in a window of `n + 2` passage words, in any
order. From 10 words up, one word may be wrong. The wrong word can be the fact itself, and `render` posts the
writer's `c.quote`, not the passage text, under "Quotes:".

Trigger: passage "Ambient emitters: at most 20 particles a second each, 4 emitters per area." and a writer line
`{"part":1,"claim":"Emitters may use 200 particles a second.","quote":"Ambient emitters: at most 200 particles a second each, 4 emitters per area","note":"VFX Budgets"}`.
13 words, 12 needed, 12 found (all but "200"): kept, and the board shows `> "… at most 200 particles …" (VFX Budgets)`.
The same holds for "never" → "always". With one dropped claim no model check runs, and the model check only
compares the claim with the (altered) quote.

## 4. Knowledge index (Light): a superseded note is cut from the answer, not ranked below — important (confirmed by simulation)

`rbxrag/engine.py:629-635`

The fused RRF scores are multiplied by 0.3. With `rrf_k = 60` the best possible score is 2/61 = 0.0333, times 0.3
= 0.0100, which is below the 30th fused candidate (0.0127 in the simulation). So the superseded note always lands
at position 30 of 30 and `[:k]` (k at most 8) removes it. The module text and `rbxvault.py` both promise "never
dropped".

It matters when the replacement is not among the candidates: it was trashed (finding 5), it is another game's
note (finding 6), or the search is limited to a shelf the replacement is not on. Then the answer silently loses
the only note on the subject. Heavy (three lists) can keep it a little higher but still below most candidates.

## 5. `forget` on a replacement leaves the old note marked and demoted for good — important (confirmed)

`rbxnotes.py:496` (`forget`), marks written by `rbxnotes.py:371` (`mark_superseded`), read by `rbxvault.py:826`

Nothing removes `superseded_by` (or the `superseded.json` entry) when the replacing note goes to the trash, and
`superseded_map` never checks that the replacement exists.

Trigger: file "Throw meter v2" with `supersedes: Throw meter position`, then `forget` "Throw meter v2". Search
still shows `Throw meter position ... (replaced by Throw meter v2)`, scored at 0.3, pointing at a note that is in
`.trash`. On the Light index it disappears from answers (finding 4). The upkeep sweep does not report it either.

## 6. A game note that supersedes a general note demotes that note in every other game — important (by reading)

`rbxnotes.py:317-345` (`supersede_targets`), `:371-384` (`mark_superseded`)

For scope `game`, the second root searched is the for-everything folder, and the mark is written into that
note's own front matter. The mark is global; the replacement is visible only in the one game
(`rbxrag/engine.py:496-507` takes marks with project `""` for every project; the keyword path reads the same
front matter).

Trigger: in game A (as a Lead, not an expert) file a `scope: game` note with `supersedes: "Button shape"`, where
"Button shape" is a note for every game. In game B, "Button shape" now ranks at 0.3 and says "(replaced by
Lantern buttons)", a note game B cannot read. `tests/test_upkeep.py:124` asserts the mark is written, so the
write is intended; the cross-game effect is the defect.

## 7. Superseded marks match by title alone, across shelves — important (confirmed)

`rbxvault.py:842-850` (`replaced_by`), map built at `:826-839`

The map is `{old title casefolded: new title}` with no shelf or path, and `replaced_by` also tries the note's
H1 `label`. Any note on any shelf with the same title (or a guide whose H1 equals it) is demoted and marked.

Trigger: the user has their own note titled exactly like a vault note ("Damage Formula Pipeline - Armor,
Resistances, Crits and Order of Operations") and files a new note superseding it. `supersede_targets` marks the
user's note (correct), but search then shows the LihFolk vault note at rank 4 instead of 1 with "(replaced by My
newer take)". Reproduced on the keyword path; the engine uses the same map.

## 8. Vault index cache has no format in its signature: an updated packaged app keeps label-less notes — important (unconfirmed for the packaged build)

`rbxvault.py:154-161` (`signature`), `:163-170` (`load_index`), new fields added at `:122-133`

`parse()` now adds `labels`, `links`, `supersedes` to every vault doc, but the cache signature is only
`VERSION` + each file's mtime and size + `rbxvault.py`'s own stat. `INDEX_FORMAT` (bumped to 3) covers the
guides cache only. `VERSION` is still "2.7.3", and in the packaged app there is no `rbxvault.py` on disk (the
code's own comment says so), so an existing `lihfolk-vault-index.json` stays valid.

Effect with an old cache, keyword path: `labels_match(None, filters)` fails for `kind`, `source` and `since`, so
a filtered search returns no vault note at all; vault results carry no label; vault `[[links]]` are never
followed; `supersedes:` inside vault notes is ignored. From source the cache is rebuilt (the file's mtime
changed), which is why tests pass. Unconfirmed part: whether an installer keeps the vault files' mtimes. If it
does, this hits every existing user until the version changes.

## 9. Untitled notes that open with the same eight words are refused as duplicates, and the advice is to overwrite — important (confirmed)

`rbxupkeep.py:238-239` (`is_duplicate`: same title is 1.0, whatever the text), `rbxnotes.py:97` (`title_from`),
`rbxnotes.py:584-587`, advice text at `rbxupkeep.py:309-315`

Before tonight a second note with the same title was saved as "Title 2". Now it is refused, the reply is ordinary
tool text (not an error), and its first suggestion is `replace: true`, which overwrites the existing note with no
copy kept.

Trigger: `note` with text "The user wants the shop to use coins only, never Robux, because kids cannot buy
Robux.", then `note` with text "The user wants the shop to use coins for pets but gems for skins and trails."
Both get the title "The user wants the shop to use coins…". Second reply: "Not saved: a note like this exists
already (the same title)." Callers that file untitled notes and are not told about this: `app-rs/src/text/system.txt:25`
and `studio.txt:14` ("Save each Change as a note too ... quoting the user's note"). Only the Librarian's prompt
(`swarm.rs:1153`) was updated. A cheap model that reads "Not saved" as done loses the user's correction; one that
follows the first suggestion destroys the earlier note.

## 10. The `date` label is the file's modification day for 94% of the vault — minor (confirmed)

`rbxvault.py:694` (`note_labels`: `day or _file_day(path)`)

Only 77 of 1,311 vault notes have a date key in their front matter. The rest get the day the file was installed
or checked out. In the probe every LihFolk note showed `[... · lihfolk · 2026-11]` although the vault is dated
October 2026. `since: 2026-11` therefore keeps the whole vault, and the month is also embedded in each chunk's
context line. In the index, a file that is only touched is "restamped" without re-embedding
(`rbxrag/engine.py:354-358`), so its stored date then differs from the keyword path's.

## 11. Per-search work that grows with the vault — minor now, important at 10x (measured)

- `rbxvault.py:1978-1984` (`_vault_superseded`), called at `:2068` for every keyword search with shelf `vault`:
  it calls `load_index()`, which re-reads and parses the whole cache (12.5 MB today), and then the old
  `main()` path loads it again. Measured 0.40 s against 0.27 s without it. At 10x that is over a second added
  to each vault search, inside a 3 s budget on the Librarian path.
- `rbxvault.py:1543-1556` (`_linked_pool`): on every keyword search with links it builds a dict of every vault
  note and a name map of every section doc before looking at three results.
- `rbxvault.py:1481` (`superseded_for`): scans every vault doc on every search (the section docs are cached on
  `Docs`, the vault list is not).

None is quadratic; all are linear per query and could be built once per loaded index.

## 12. Engine: `_sup` is replaced while a query may be using it — minor (by reading)

`rbxrag/engine.py:494` sets `self._sup = {}` at the end of each sync; `:501-507` does
`if key not in self._sup: self._sup[key] = ...` and then reads `self._sup[key]`. A sync finishing between the
two (queries run on other threads) raises `KeyError`; the service answers 500 and `rag_search` falls back to the
keyword search for that one query. Rare and self-healing.

---

## Checked and found sound

- **Index format.** `FORMAT` 1 → 2 forces a full rebuild through `_invalidate` + `store.reset`; the service
  answers 503 until the first full sync ends and callers fall back to the keyword search. A query's quick sync
  raises `NeedsRebuild` rather than rebuilding. Qdrant's `reset` recreates the collection with the new payload
  indexes. Light and Heavy store the same label fields (Heavy adds `day`; Light derives it per doc), and
  section-level labels ("Learned in Fridae") are kept in both.
- **Filters** reach all three paths: keyword (`labels_match`), RAG (`_rag_hits` → `normalize_filters` again in
  `serve.py` → store masks), and linked results. The Librarian's `lookup` sends none, as intended.
- **Guide excerpt tag** is added in `_section_lines` (both searches) and `read_file_section`.
- **Guides never reach the writer** in `librarian.rs` (`material`, `cited`, `fallback_text` all filter
  `is_guide`).
- **Rust panics.** `parts[c.part]` is bounded by `parse_claims`; string slices use regex or `char_indices`
  offsets; `clean` truncates only ASCII tails; `quote_found` cannot underflow. Timeouts: 15 s search (child
  killed), 45 s writer, 15 s check.
- **Strong Lead.** `send` re-reads `self.ai()` after the hand-over; `swarm::leads` is not called under the
  agent's own lock, and wakes run on their own threads, so no lock inversion was found.
- **Upkeep report** lands in `<notes>/upkeep/`, which `note_files` (not recursive) does not index.
  `superseded.json` is written and read at the same path.
- **Upkeep sweep** over 10,020 notes ran in 5.4 s in the suite.
