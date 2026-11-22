# Knowledge lookup: design and results (2026-11-22)

Branch `feature/knowledge-lookup` (from `feature/studio-mode`). Not pushed.

## What was built

(Paths in this section are as they were in the first round. The second round moved everything into `vault/`;
see "One real vault" below.)

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

## Bench (`tests/knowledge_bench.py`), corrected in round 3

The first two rounds measured only an ordinary chat (workflow + one search + three sections) and reported 39% of the
old intake, 15% without `workflow`. A review showed that was not what the agents did: the expert files told each
expert to "read the opening" of every guide, and a guide under 12 KB (54 of 63) comes back whole, so an expert still
loaded nearly everything whole; the bench also left `critic` out of the security task. That 39%/15% claim is
withdrawn.

Round 3 changed the expert files and `routing.txt` (only the job's rule guides whole, the others as outlines that
carry their rules sections, then search and sections) and the bench now measures both kinds of agent from the tools'
own answers:

- **Old**: the whole guides the old `routing.txt` (chat) or the old expert file (expert) named for the task, at commit
  9b0e7fd, including `workflow` and, for security, `critic`.
- **New, chat**: `workflow` whole (rbx_guide), one `rbx_vault` search on shelf guides, reading its top three sections.
- **New, expert**: exactly what its file in `vault/Fridae/Team` says: its rule guides whole in one rbx_guide call, one
  rbx_guide call with `outline: true` for its other guides, then the same search and three sections. The bench checks
  that every guide it loads is named in the expert's file.

| Task | Chat old | Chat new | Chat % | Expert | Expert old | Expert new | Expert % |
|---|---:|---:|---:|---|---:|---:|---:|
| save player coins between sessions | 85,065 | 29,264 | 34% | coder | 85,065 | 43,879 | 52% |
| make a shop menu that fits phones | 76,341 | 24,304 | 32% | ui | 82,076 | 37,179 | 45% |
| a sword slash effect | 90,011 | 26,869 | 30% | effects | 106,119 | 45,472 | 43% |
| playtest the tutorial as a new player | 50,464 | 24,848 | 49% | playtester | 42,503 | 29,587 | 70% |
| price the first three upgrades | 37,071 | 26,992 | 73% | economy | 36,252 | 30,498 | 84% |
| review remotes for exploits | 40,824 | 24,641 | 60% | security | 41,132 | 39,097 | 95% |
| **Total** | 379,776 | 156,918 | 41% | | 393,147 | 225,712 | 57% |

The target was every task below 50% for both kinds of agent. Six of the twelve figures miss it, and why:

- **The fixed floor is `workflow` whole: 18,497 characters.** It is the rules for working in the place, and the
  decision keeps it whole for every agent that works there. For a task whose old intake was small (economy 36 KB,
  security 41 KB, the chat's upgrades 37 KB and remotes 41 KB) `workflow` alone is 45-51% of the old total before
  anything is looked up. One search plus three sections adds about 6-8 KB.
- **Security** also loads `critic` whole (7,271), as decided: workflow + critic is 25,768, already 63% of 41,132.
  The two other guides come as outlines (about 3 KB) and the lookup adds 7 KB: 95%. Its old intake was mostly rules
  that are still loaded whole, so there is little to save.
- **Economy** (84%) and **playtester** (70%): workflow plus outlines of two or three small guides (about 1-2 KB each)
  plus the lookup. Their old intake was three or four small guides, mostly workflow.
- **Coder** (52%): six outlines are about 14.6 KB, of which `lihfolk-systems` alone is 4.5 KB because its "Hard rules"
  section must be carried in full (A2).
- Where the old intake was large (coins, shop, sword) the new way is 30-52%. Over all six tasks: 41% for chats, 57%
  for experts. Without `workflow` (which both ways load), the chat is 18% of the old intake and the expert 41%.

Getting every task under 50% would need `workflow` itself to be outlined or split (its rules sections are about
3.5 KB of its 18.5 KB). That changes what every agent is guaranteed to read, so it was not done here; it is a call
for the user.

**Index build, cold, on this Mac:** guides and team sections 0.24-0.30 s. The vault took 2.44-2.87 s, against 2.5 s
before the change, so it is unchanged. Warm loads: sections 0.02 s, vault 0.12 s.

**Relevance**: all 6 tasks find an expected guide in the top 5 on shelf guides (asserted in
`RelevanceTests`). The top results:

- coins: data handoff (teleport guide), then `roblox-data-stores` session locking.
- shop/phones: `ui-design` "Input, devices, motion", then `roblox-ui` cross-platform input.
- sword: `lihfolk-vfx`.
- tutorial: `lihfolk-tutorial`.
- upgrades: weakest. `economy-design` "Cost curves" is only 5th, behind `game-brainstorm` and FTUE sections. Still 5th after round 3 (see known weak spots).
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

**A. One real vault: done (second round, option 1).** At first this stopped: `install.py` copied every `skills/<name>/`
folder into `~/.claude/skills/<name>/`, so Claude Code depended on that layout. The coordinator chose option 1: move
everything and have `install.py` build the skill folders from the vault.

The layout chosen is **one folder per guide**, with the guide itself named after the folder:

```
vault/
  Start Here.md                      the front page for a person (what is where, licences)
  Roblox/                            LihFolk Vault V2 notes (was guides/lihfolk-vault), numbered folders kept
  Fridae/Guides/Core/<guide>/<guide>.md          was skills/<folder>/SKILL.md (+ that folder's other files)
  Fridae/Guides/Roblox subjects/<g>/<g>.md       was guides/roblox-skills/<g>/SKILL.md
  Fridae/Guides/LihFolk/<g>/<g>.md               was guides/lihfolk/<g>/SKILL.md (+ Luau modules, scripts, rubrics)
  Fridae/Team/<id>.md, roster.json               was team/experts/*.md and team/roster.json
```

Why this layout:

- Each guide's own relative links (`references/x.md`, `src/...`) keep working unchanged, in Obsidian and in Claude
  Code.
- `_reference` stays "the guide's folder + `references/`".
- `install.py` only has to rename `<guide>.md` back to `SKILL.md`.
- The groups keep each source's licence boundary visible.

The front page is `Start Here.md`, not `00 Start Here.md`, because LihFolk's own `Roblox/00 Start Here.md` would
otherwise share its name, and Obsidian resolves `[[00 Start Here]]` by name. Guide names are unchanged. The `tester`
guide stays in `agents/`, which `install.py` also copies to `~/.claude/agents`.

All 1,602 files were moved with `git mv`. Every consumer was updated in the same commit:

- Python: `rbxkit_mcp.GUIDES` paths, registration from the two subject groups, the LihFolk credit check.
- `rbxvault.py`: `KB`, `TEAM`.
- `rbxroster.kit_dir()`: experts are now `<kit team>/<id>.md`. Project overrides are unchanged.
- Rust: the new `paths::kit_team()`, used for `lead.md` and the roster names.
- `rbxkit.spec` datas: `vault` replaces `skills`, `guides` and `team`.
- `tests/test_roster.py`, README, DEVELOPING, THIRD_PARTY and `rbxpaths`.

**`install.py`**: `claude_skills()` maps each `vault/Fridae/Guides/Core/<guide>/` folder to a skill folder.
`CLAUDE_SKILL_NAMES` holds the six whose skill name differs from the guide name, for example `workflow` becomes
`roblox-studio-workflow`. `<guide>.md` becomes `SKILL.md`, every other file keeps its relative path, and
`build_claude_skills()` writes them with the install placeholders filled, as before. `tests/claude_skills_baseline.json`
was captured from `skills/` before the move: 24 folders, 50 files, each file's front matter and SHA-1.
`tests/test_install_skills.py` builds into a temp folder and checks folder names, file lists, byte-identical sources
and front matter. `install_claude` is tested with a temporary HOME. No other external consumer of `skills/<name>/` was
found. The `rbx doctor` check looks at `~/.claude/skills/<name>/SKILL.md`, whose layout is unchanged.

**Link check** (`tests/vault_links.py`, Obsidian-style resolution by file name, plus relative Markdown links):

| | Markdown files | Wikilinks (broken) | Relative links (broken) |
|---|---:|---:|---:|
| Before the move (old folders checked as one vault) | 1,453 | 38,484 (40) | 54 (12) |
| After the move | 1,454 | 38,485 (40) | 54 (11) |
| After the second trim | 1,441 | 38,028 (30) | 44 (1) |

None of the remaining broken links come from the move:

- The 30 wikilinks are embeds of upstream images that were never bundled (rig sheets, renders).
- The 1 relative link points at upstream's `AGENTS.md`.
- One relative link was fixed: `LihFolk Systems Library` now points at the vault's `lihfolk-systems` guide.

Ten note names now occur twice or three times in the vault (`ui`, `vfx`, `promo`, `world`, `effects`, `animation`,
`model`, `system`, `checklist`, `LICENSE`). These are team notes, guides and quality-score rubrics sharing a name.
Only three wikilinks touch them (`[[Animation]]`, `[[Model]]`, meant for the atlas notes), and Obsidian may pick the
other file for those.

**B. Trim the Roblox vault: done in two passes.** Inventory from `tests/vault_inventory.py`, before the trim:

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

Second pass (the coordinator's call): removed the LihFolk plugin's 10 `Skill - …` notes and their hub `AI Skills Start
Here`. The hub was not on the coordinator's list, but its whole content describes those plugin skills. Also removed
`Open Questions - Whole Vault` and the Concept Sheets note. That is 13 notes. 102 wikilinks to them, in 81 notes,
became plain text, and `00 Start Here` no longer lists them. Plain-text mentions in auto-generated atlas lines remain
and are harmless.

After both passes: 1,310 Markdown files (1,308 indexed), down from 1,327 (1,321), and 20.9 MB, down from 21.6 MB.
The bench tasks' vault top fives are unchanged. One baseline query ("datastore session lock") changed its 5th result,
from "AI Pitfalls - Data and Monetization" to "Data Loss Prevention and Recovery", because term statistics shifted;
the fixture was updated. Kept as instructed: the rig files, the atlas notes, the animation notes, Core Concepts and
the Resource Library. `api` reads `_tools/roblox_api.json` and `api_overrides.json`, not the atlas notes, but the
atlas is kept anyway.

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
| Folder "Start Here" hubs | ~5-9 KB each | Navigation; damped in ranking |

## Tests run

- `.venv/bin/python -m unittest discover tests`: 269 OK after the first round, and 275 OK at the end (222 s). The
  final run includes `test_install_skills` (3), the interleave tests (2) and the frozen-app test (1).
- `cargo test --manifest-path app-rs/Cargo.toml`: 124 passed.
- `cargo build`: clean apart from the existing `is_gemini` unused warning, which is also on the base commit. No new
  warnings.
- `tests/knowledge_bench.py`: the table above. The final run matches it (the "old" column is 39 characters larger,
  from the longer LihFolk folder paths in guide footers). `tests/vault_links.py`: the link table above.

## Concerns and what was not verified

- No live agent run was made, so whether agents follow the new routing (search, then read sections) instead of
  loading guides whole is untested. The bench measures what a compliant agent would take in.
- Mixing shelves in `all` merges by query coverage times the score relative to each shelf's best. Since the second
  round, `interleave()` puts the best hit of each shelf scoring at least 0.2 first, then the rest by score. Team
  expert notes now show up for task-like queries ("review remotes for exploits" brings up the security expert),
  which is intended but new.
- Snippets of code-heavy sections show code lines.
- Weakest relevance: "price the first three upgrades" (`economy-design` is 5th).
- The packaged app was built once (`build_mac.py`, not installed). `vault/` is inside it, at
  `Contents/Resources/tools/_internal/vault`, with 1,441 Markdown files, and no `skills/`, `guides/` or `team/`
  remain. The packaged CLI (`--mcp`, with HOME and the work folder in a temp dir) served `rbx_guide workflow`,
  `rbx_guide unslop section color`, `rbx_vault search` (all shelves and shelf game), `rbx_vault read` on shelf team,
  and `rbx_stage status` (roster names).
- That first packaged run caught a real bug: the section index's cache key stat'ed `rbxvault.py`, which in the frozen
  app is inside the archive, so search failed there. Fixed (commit 83aa033) and covered by a test. The desktop window
  itself was not opened.
- The `.lua` guides (`ui-helpers`, `ui-audit`) are not indexed for search. They are code to paste, fetched whole
  with `rbx_guide`.

## Round 3: review fixes and the user's notes

### Review findings

| Item | Done | What |
|---|---|---|
| A1 outlines and expert files | yes | `rbx_guide` `outline: true` (also for several topics) returns any guide's description, opening (cut at 1.5 KB), rules sections and section table. Expert files load only their rule guides whole (`workflow`; security `critic`; designer `game-bible`; promo `promo`) and outline the rest in one call. `routing.txt` says the same for chats. Bench corrected above; the 50% target is missed for six figures, for the reasons given. |
| A2 rules in outlines | yes | `RULES_HEADING` (rules, do not, don't, never, must, always, checklist; whole words, any case) plus `ALWAYS_SECTIONS` (blender-modeling "Pipeline (do every step)", "Known traps"; motion-design "Quality while composing"); outermost sections only; 8 KB cap, then listed by name. Tested on all eight guides: lihfolk-systems 4.5 KB outline (of 18 KB), unslop 5.3 (33), lihfolk-vfx 7.4 (20), lihfolk-sfx 6.7 (25), redesign-skill 1.8 (15), emil-design-eng 7.9 (27), and the two in the map. |
| A3 stale Windows folders | yes | `splash.rs`: after the copy, `_internal/skills`, `_internal/guides` and `_internal/team` are removed, only when the target holds `Fridae Studios.exe`, only real folders inside it, and never one the new version ships. Pure path test plus a temp-folder test. |
| A4 dot-folders | yes | `.gitignore` has `vault/.obsidian/`, `vault/.trash/` (and `/notes/`, the user's notes when running from source). `rbxkit.spec` walks `vault/` and leaves out dot-folders and dot-files. The section index skips dot-files (the vault engine already skipped dot-folders). |
| A5 skills test | yes | Folder/file lists and front matter still against the baseline; built files compared with the current vault sources (placeholders filled), not frozen SHA-1s. |
| A6 short-note damp | yes | No 120-character damp on the game and yours shelves. |
| A7 folding | yes | The game and yours shelves are ranked apart from the kit shelves, with extra folding: "data store(s)" joined, irregular verbs (chose/chosen to choose, made, built, ...), on top of the vault stemmer's plural/-ing/-ed. "which datastore library did we choose" finds "We chose ProfileStore over raw datastores" first (tested). Each group's scores are scaled to its own best, so a handful of notes is not drowned by the big corpus. Vault and kit ranking unchanged (baseline and relevance tests pass). |
| A8 weak results | no (listed) | Not fixed without touching vault ranking; see known weak spots. |
| A9 cache | yes | `roster.json` is in the section cache key; the cache is written to a temp file and renamed. |
| A10 Start Here | yes | Overrides named exactly (`.fridae/team/experts/<id>.md`, `.fridae/team/roster.json`); "always come whole when loaded"; `LICENSE.txt` copied beside `Guides/LihFolk/` (CC BY 4.0, from `licenses/lihfolk-vault-LICENSE.md`; `.txt` because the Markdown copy carries wikilinks to upstream notes) and `Guides/Roblox subjects/` (MIT). |
| A11 Atlas links | yes | `[[57 API and Concept Atlas/Animation|Animation]]` and `.../Model`. Link check: 1,441 Markdown files, 38,028 wikilinks (30 broken, unchanged), 44 relative (1 broken, unchanged). |
| A12 lessons without a project | yes | Shelf `yours` (B). |
| A13 Lead text test | yes | `studio_lead_text_in(kit, project)`: the vault's `lead.md`, or the game's override. |
| A14 skill-name aliases | yes | `GUIDE_ALIASES` (the six `install.py` names plus the four design guides' upstream names); test checks every alias and every `CLAUDE_SKILL_NAMES` value. |

**Known weak spots** (vault ranking left as it is):

- "camera shake on explosion" (all): the guides hits are all `lihfolk-vfx`, the right guide, but generic sections
  (Budgets, API), because the guide's description names camera shake and is searched with every section. The vault's
  "Hit-Stop and Screen Shake Tuning by Numbers" is 4th on shelf vault and does not reach the merged top five.
- "my leaderstats coins reset when I rejoin" (all): the vault's best is "Rebirth and Prestige System End to End"; the
  guides' leaderstats section is 2nd. No data-store note reaches the top five.
- "price the first three upgrades" (guides): `economy-design` "Cost curves" is still 5th; "first" and "three" match
  onboarding sections. A title/heading boost does not help (the right section's heading has none of the words).

### The user's notes (B)

- **Where**: `<home>/notes/` (`rbxpaths.HOME_DIR`: `Documents/Fridae Studios` in the app, the repo folder from
  source, ignored by git) and `<game>/.fridae/notes/`. One Markdown file per note with front matter (title, when,
  scope, by, kind), plus `House rules.md`. Never in the shipped vault.
- **Writing** (`rbxnotes.py`, through `rbx_vault`): `note` (text at most 1,500; optional title; scope game|all, default
  game with a project; `rule: true` adds one bullet of at most 240 characters to `House rules.md`, at most 2,000
  characters, refused when full with a pointer to `rule_edit`), `notes`, `forget` (to `.trash/`), `rule_edit`. Titles
  are sanitised (separators, control and Windows-forbidden characters, reserved device names, the kept file names;
  accents and spaces kept; duplicates " 2"); names are claimed with O_EXCL so concurrent writers never collide; every
  write is a temp file plus rename; nothing outside the two folders is read or written. `rbx_stage lesson` writes a
  note (kind `lesson`) through the same code.
- **Reading**: shelf `yours` (for-everything notes, House rules, `studio-lessons.md`; the old work-folder
  `studio-lessons.md` is moved there on first use and still read if the move fails) works without a project; shelf
  `game` adds `.fridae/notes/` (the old `.fridae/lessons.md` is still read). Both indexed fresh every time.
- **Always loaded**: `agents.rs` appends both `House rules.md` files to every agent's system prompt each turn (the
  Lead and experts included), under "The user's house rules (follow these)", at most 4,000 characters (`house_rules_block`,
  a pure function with a unit test).
- **When agents write**: `system.txt` has the brief's paragraph and tells agents to search `yours` and `game` before
  design or build work. `studio.txt`, `lead.md` and the `studio` guide say to save a note for every Change in Review,
  quoting the user's note, and to search those shelves at the start of a run.
- **routing.txt**: the user's UI-taste paragraph became a generic default; the original wording is in
  `docs/house-rules-example.md` for the user to paste into their own House rules. Nothing was written into the real
  home folder.
- **App**: Setup, General, "Your notes" shows the counts for everything and for the front chat's game, with "Open notes
  folder" (and "Open this game's notes") buttons; `GET /api/notes` and `POST /api/notes-open` are token-protected and
  open only the home notes folder or a project the app knows (`notes.rs`, tested). One Guide entry, "Your notes".
- **Docs**: `vault/Start Here.md`, DEVELOPING.md ("The user's notes"), README (two sentences).

### Round 3 concerns

- The 50% target is missed for six of twelve bench figures (above): `workflow` whole is the floor.
- House rules are appended to the system prompt each turn for Claude Code. Gemini and ChatGPT get the standing
  instructions only on a chat's first turn (as before for all instructions), so a rule added mid-chat reaches them
  from the next new chat.
- Whether agents actually save notes when steered depends on the model following `system.txt`; no live run was made.
- The game and yours shelves are scored apart and scaled to their own best, so on shelf `all` the best personal hit
  is promoted whenever it covers at least 20% of the query; a note that shares one word with a short query can appear.
