# Knowledge lookup: design and results (2026-11-22)

Branch `feature/knowledge-lookup` (from `feature/studio-mode`). Not pushed.

## What was built

- **`rbxvault.py`**: the LihFolk vault engine is unchanged (note-level BM25F, symptom lines, its own cache).
  Added around it: section-level search over three more shelves. `guides` covers every Markdown guide in
  `rbxkit_mcp.GUIDES` plus each guide folder's `references/*.md` (100 files). `team` covers `team/experts/*.md`,
  which a game's `.fridae/team/experts/` copies replace. `game` covers the game's `GAME.md`, `.fridae/lessons.md` and
  `work/studio-lessons.md`, only when `RBXKIT_PROJECT` is set. A section is a `##`/`###` heading plus its own text.
  Fenced `#` lines are not headings. The text before the first heading is `(opening)`, and a file with no headings is
  `(whole file)`. The guides and team index is cached in `work/cache/knowledge-sections-index.json`, keyed on the file
  list, mtimes and sizes. The game shelf and project overrides are read on the fly and never cached.
- **`rbxkit_mcp.py`**: `rbx_vault` is widened, `rbx_guide` gains `section`, `whole` and outlines, and
  `ALWAYS_WHOLE`/`OUTLINE_OVER` are added.
- **`rbxstage.py`**: the `lesson` reply says lessons are searchable with `rbx_vault` shelf `game`.
- **Agent texts**: `routing.txt` and the 12 expert files were rewritten. So was one sentence in the `workflow` guide
  ("Which guides to read"), plus the UI label for `rbx_vault` in `friendly.json`.
- **Docs**: DEVELOPING.md now has "How agents find knowledge". README has a plain paragraph. THIRD_PARTY.md notes
  the trim.
- **Tests**: `tests/test_knowledge.py` (26 tests), `tests/knowledge_bench.py`, `tests/vault_inventory.py`, and
  `tests/vault_baseline.json` (the old engine's top-5 notes for 12 queries).
- **Packaging**: no new modules or folders. `rbxvault` is already in `rbxkit.spec` hiddenimports. `guides`,
  `skills` and `team` are already in its datas.

## Tool surface

- `rbx_vault`:
  - `action` (required): search, read, sections, list or api.
  - `query`: for search.
  - `shelf`: all|vault|guides|team|game. Default all, which includes game only when the chat has a project.
  - `k`: results, default 5 (3 for shelf vault), at most 8. `count` is the old name for `k`.
  - `title` and `heading`: for read. `title` alone for sections.
  - `folder`: for list. `names`: for api.
  - Search results are labelled `[shelf]`. Each has a 2-3 line snippet (lines that share the query's words), its
    size and a `read: {...}` line with the exact arguments. Vault results show two TL;DR bullets and their "Read
    more" headings. Symptom and API lines are added when the vault is searched. `shelf: vault` returns exactly what
    it returned before, byte for byte (checked against the old code).
  - `read` returns one section, capped at 12,000 bytes. A larger section returns its first part and lists its
    sub-headings. With no heading, read lists the sections. Headings resolve in this order: an exact match,
    `Parent > Child`, a substring, all the words. `sections` lists headings with sizes (the vault listing gained
    sizes too).
- `rbx_guide`: `topics`, `topic`, plus the new `section` and `whole`.
  - A guide over 12 KB (12,288 bytes) returns its front-matter description, its opening and a table of sections with
    sizes, plus its reference list and credits. The exceptions are guides on `ALWAYS_WHOLE` and calls with
    `whole: true`.
  - `section` returns one section of one guide or reference file.
  - Caps are unchanged: 30 KB for helper agents, 120 KB for chats.

**Always whole:** `workflow`, `studio`, `critic`, `producer`, `game-bible`. Any guide under 12 KB is also always
whole, which is 54 of the 63 topics. Eight guides now come as outlines: `roblox-blender-modeling`, `redesign-skill`,
`lihfolk-systems`, `lihfolk-vfx`, `lihfolk-sfx`, `motion-design`, `emil-design-eng` and `unslop`. So do the large
reference files. `lihfolk-worldbuilding` is borderline at 12,019 bytes and stays whole.

## Bench (`tests/knowledge_bench.py`)

"Old" is the total of the whole guides that `routing.txt` or the expert file named for the task. "New" is `workflow`
in full plus one `rbx_vault` search (shelf guides) plus reading its top three sections.

| Task | Old: whole guides | New: workflow + search + 3 sections | Old without workflow | New without workflow |
|---|---:|---:|---:|---:|
| save player coins between sessions | 85,052 | 28,592 | 66,555 | 10,095 |
| make a shop menu that fits phones | 82,076 | 23,632 | 63,579 | 5,135 |
| a sword slash effect | 98,243 | 26,197 | 79,746 | 7,700 |
| playtest the tutorial as a new player | 53,578 | 24,176 | 35,081 | 5,679 |
| price the first three upgrades | 36,252 | 26,320 | 17,755 | 7,823 |
| review remotes for exploits | 41,132 | 23,969 | 22,635 | 5,472 |
| **Total** | 396,333 | 152,886 | 285,351 | 41,904 |

The new way is 39% of the old, or 15% leaving out `workflow`, which both ways load whole (18.5 KB).

**Index build, cold, on this Mac:** guides and team sections 0.24-0.30 s. The vault took 2.44-2.87 s, against 2.5 s
before the change, so it is unchanged. Warm loads: sections 0.02 s, vault 0.12 s.

**Relevance**: all 6 tasks find an expected guide in the top 5 on shelf guides (asserted in
`RelevanceTests`). The top results:

- coins: data handoff (teleport guide), then `roblox-data-stores` session locking.
- shop/phones: `ui-design` "Input, devices, motion", then `roblox-ui` cross-platform input.
- sword: `lihfolk-vfx`.
- tutorial: `lihfolk-tutorial`.
- upgrades: weakest. `economy-design` "Cost curves" is only 5th, behind `game-brainstorm` and FTUE sections.
- remotes: `roblox-security`, then `roblox-code-review`.

To get there I added two things, for the section shelves only so vault ranking is untouched. First, a small extra
fold (`price`/`prices` and `upgrade`/`upgrades`; the vault stemmer leaves those apart). Second, a 0.5× damp on
"Related", "Sources" and "Files" sections.

## routing.txt and the expert files

- `routing.txt` keeps the full map of which guides cover which work. It keeps the UI-taste paragraph, the slop rule,
  the style-guide rule and the vault rule for changing facts.
- Only the loading instruction changed: load `workflow` whole, search with `rbx_vault` shelf guides, read sections,
  and load a whole guide only when the task is centrally about it. The rule guides are marked "load in full" where
  `routing.txt` names them (Studio mode, reviewer, existing game).
- `unslop` keeps "run in full", now with `whole: true`, because that task is centrally about it.
- Expert files: each names its guides, loads `workflow` (and `critic`/`game-bible`/`promo` where those are the job)
  in full, reads the opening of each guide, then searches for sections. Each keeps "at most two topics per rbx_guide
  call" and the no-shell rule.
- Not changed: `lead.md`, which only says to load `producer` (always whole). `studio.txt` and `skills/studio/SKILL.md`
  only describe loading `studio` and `producer`, both always whole. The kickoff texts, `critic.txt` and `project.txt`
  name rule guides (`game-bible`, `critic`, `studio`, `producer`) or small guides that come whole anyway.
- The `workflow` guide's "Which guides to read" table still uses Claude Code skill folder names. Only its
  instruction sentence changed.

## The direction change: "one vault" and "trim the fat"

**A. One real vault: not done, stopped as instructed.** `install.py` (`install_claude`) copies every `skills/<name>/`
folder into `~/.claude/skills/<name>/`. DEVELOPING.md says "Claude Code users also get the guides as skills". So
Claude Code consumes the guides in their current `skills/<name>/SKILL.md` shape. Moving them to
`vault/Fridae/Guides/<Guide name>.md` would break that install path. The brief says to stop and report in that case,
so nothing was moved.

There are three ways forward, for the user to choose:

1. Move everything to `vault/` and change `install.py` to build each `~/.claude/skills/<name>/SKILL.md` from the
   vault file (plus a `references/` folder). This is the most seamless, but the installer then generates skills
   rather than copying them.
2. Move only what nothing outside the app reads: `guides/lihfolk-vault` to `vault/Roblox`, `guides/lihfolk` and
   `guides/roblox-skills` to `vault/Fridae/Guides`, and `team/experts` to `vault/Fridae/Team`. `skills/` would stay
   where it is.
3. Leave the files where they are. The search already presents them as one vault with four shelves.

The search code already takes its roots from a few constants (`KB`, `GUIDES` paths, `kit_team_dir()`), so any of
these is a contained change.

**B. Trim the Roblox vault: done, conservatively.** Inventory from `tests/vault_inventory.py`, before the trim:

- 1,327 Markdown files (1,321 indexed), 21.6 MB.
- Largest folders: `18 Rigs and Skeletons` 4.2 MB (9 rigs as .fbx, .glb and .blend), `08 Animation` 2.0 MB (130
  notes), `57 API and Concept Atlas` 1.05 MB (310 notes), root 724 KB, `_tools` 365 KB (the API snapshot).
- Exact duplicates: none. Near-duplicates (90% or more of a note's 8-word shingles inside another note): none.
- Stubs (under 400 characters of prose): 7, all generated atlas notes (`MouseButton1Click`,
  `Entity Component System`, `Honeypots`, `TouchEnded`, `FTUE`, `ProfileStore`, `Wave Function Collapse`).
- Link-list pages (60% or more link lines): 85. Of these, 82 are atlas notes, plus `00 Start Here`,
  `Animation Start Here` and `Engine Reference Cards Start Here`.
- Wikilinks: 38,566, of which 164 are broken (63 targets, mostly concept names never written, like `slerp` and
  `quaternions`).
- Over 73 plain-language queries (the 6 bench tasks plus 67 more), 827 notes never reached a top 10. That includes
  308 of 310 atlas notes and 92 of 130 animation notes.

Removed: 4 hand-made or generated index/router/home pages (`INDEX.md` 341 KB, `INDEX_SLIM.md` 216 KB, `ROUTER.md`
5 KB, `000 Home.md` 4 KB), all already excluded from the search index. Also the 2 Obsidian daily-note templates in
`_templates/`. That is 568 KB in total. One link to `000 Home` in `00 Start Here` was removed, and a note was added to
`_tools/README.md`. After the trim: 1,321 indexed notes (unchanged), 21.0 MB, 163 broken wikilinks. Vault rankings
are identical: the baseline test passes, and the indexed set did not change. The bench's top-5 hits are the same.
Everything removed is in git history (commit "vault: trim ...").

Kept on purpose: `SYMPTOMS.md` (searched), `LICENSE.md`, `_tools/*.json` (used by `api`), `_data/` (cited by notes),
the rigs and Blender scripts (the Skeleton Library and build notes use them), and `00 Start Here` (the doorway for a
person opening the folder in Obsidian).

Candidates the user could cut (left in):

| Candidate | Size | Why it might go |
|---|---:|---|
| `18 Rigs and Skeletons` binaries (9 rigs × .fbx/.glb/.blend) | 4.2 MB | Largest item; the repo also has `rigs/` |
| `57 API and Concept Atlas` (310 generated notes) | 1.05 MB | 308 never in a top 10; mostly link lists |
| `08 Animation` notes never in a top 10 (92 of 130) | ~1.4 MB | Low reach, but real material |
| `52 Core Concepts` (38 short notes, 36 never in a top 10) | 168 KB | Glossary-style, damped in ranking |
| `46 Resource Library` (18 link-list notes, none in a top 10) | 80 KB | Link lists |
| `Skill - …` notes (10) and `AI Skills Start Here` | ~38 KB | Instructions for the LihFolk Claude Code plugin |
| `Open Questions - Whole Vault.md` | 27 KB | Upstream's open questions |
| `19 Concept Sheets/Concept Sheets.md` | 1 KB | Its images were already left out, so most embeds are broken |
| Folder "Start Here" hubs | ~5-9 KB each | Navigation; damped in ranking |

## Tests run

- `.venv/bin/python -m unittest discover tests`: 269 tests OK after the lookup commits (220 s), and 269 OK again after
  the trim (223 s).
- `cargo test --manifest-path app-rs/Cargo.toml`: 124 passed.
- `cargo build`: clean apart from the existing `is_gemini` unused warning, which is also on the base commit. No new
  warnings.
- `tests/knowledge_bench.py`: the table above.

## Concerns and what was not verified

- No live agent run was made, so whether agents follow the new routing (search, then read sections) instead of
  loading guides whole is untested. The bench measures what a compliant agent would take in.
- Mixing shelves in `all` merges by query coverage times the score relative to each shelf's best. That is a
  heuristic, and vault notes often take the top of an `all` search. The routing points agents at shelf `guides` for
  guide material.
- Snippets of code-heavy sections show code lines.
- Weakest relevance: "price the first three upgrades" (`economy-design` is 5th).
- The packaged app was not built (PyInstaller). Nothing new needs packaging, by inspection of `rbxkit.spec`.
- The `.lua` guides (`ui-helpers`, `ui-audit`) are not indexed for search. They are code to paste, fetched whole
  with `rbx_guide`.
