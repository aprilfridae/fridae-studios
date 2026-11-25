# One folder, one vault: skills, guides and notes alike

Date: 2026-11-25. Status: draft for the user's approval.
Builds on: `2026-11-24-unified-knowledge-vault-design.md` (the folder layout, the move, the Taste notebook) and
`2026-11-24-universal-skills-design.md` with its phase 3 "As built" (the skills vault engine this reuses).
Where this design and the 11-24 one differ, this one wins.

## Intent

The user's words (2026-11-25): "the skills/guides/notes vault should all be the same and people should be able to
click pull newest and it wont overwrite what they have and they have the option to keep the vaults one if its the
same guide or keep theirs." And: the whole one-folder reorganisation, not just the guides, built in three stages.

Why: today skills have a proper vault (download, updates that never overwrite, sharing, a review inbox, releases),
but the Roblox guides ship baked into the app, the user's own notes sit in a separate pile, and chats, games and
pictures are scattered under the app. The user wants one thing that behaves one way, and one folder they understand
and can copy to another computer.

## Goals

- One **Knowledge** folder, inside the one **Fridae Studios** folder, holding skills, the Roblox guides and the user's
  own notes, all handled by the same rules.
- One **Pull newest** action for all of it. It never overwrites what the user has.
- The same update rule for every file (skills included):
  - a file the user never changed is simply updated;
  - a file the user changed, that the vault did not, is left alone;
  - both changed, and the changes don't clash: Fridae combines them and says so ("Updated 3 guides you had edited");
  - both changed and they clash: Fridae asks, per file, **Keep the vault's** or **Keep mine**. Until answered, the
    user's version stays in use.
  - a new vault file that has the same name as one of the user's: the same question.
- Sharing, the maintainer's inbox and Publish work for guides and notes the way they do for skills.
- The user's chats, games and made things move into the one folder, once, with a full backup kept until they say
  it's fine.

## Non-goals

- Syncing between the user's own computers (copying the folder is the way).
- Sharing chats, games or made things. Only Knowledge is shared.
- Editing the vault on GitHub by hand as the normal path; the app's inbox and Publish are the path.

## Anti-goals (a failure even if it "works")

- Losing or overwriting a single user note, skill or edit, during the move or on any pull.
- A clash resolved silently. Clean combines are reported; clashes are always asked.
- Two places holding the same kind of knowledge after the move.
- Keys, tokens or sign-ins inside the portable folder or in anything shared.
- Removing LihFolk's credit from his notes, or presenting the vault as his.
- Git conflict marks (`<<<<<<<`) ever appearing inside a user's note.
- New colours, badges, emoji or "AI-looking" UI. Setup's existing styles only (the user is a graphic designer).

## Approaches considered

1. **Widen the skills vault engine to all of Knowledge (chosen by the user).** The engine built in phase 3
   (`rbxvaultsync.py`, `rbxvaultnew.py`, `rbxshare.py`, `app-rs/src/vault_git.rs`, `vault_sync.rs`, `share.rs`,
   `vault_inbox.rs`) already has hashed manifests, the base record, combine-or-keep, staging, masking, the inbox and
   Publish, but only accepts paths under `Skills/` (`ALLOWED_TOP`). Widening it makes "all the same" literally true:
   one code path.
2. A second engine for notes. Faster to start, but two systems that drift apart, the opposite of the goal. Dropped.
3. Make Knowledge a git clone and run git on "Pull newest". Clashes would surface as conflict marks in notes, and
   every user would need git. Dropped.

Built in three stages (user's choice), each usable on its own: A, then B, then C.

## Layout (from the 11-24 design, unchanged except where noted)

```
Documents/Fridae Studios/        the one folder to copy (rbxpaths.HOME_DIR when packaged)
  Knowledge/                     the vault (an Obsidian vault)
    House rules.md
    Taste/                       one note per taste word ("Softer.md")       [stage B]
    Skills/                      as today
    Roblox/                      the Roblox topic folders (LihFolk's and Fridae's), plus notes filed into them   [stage A]
    <new topic folders>          e.g. general programming                    [stage B]
    Inbox/                       new notes with no fitting topic yet         [stage B]
    .fridae-vault/               base record and base copies (hidden), as today
  Games/                         each game, with GAME.md and .fridae/notes   [stage C]
  Chats/                         chat history, readable                      [stage C]
  Made/                          promos and images                           [stage C]
  Settings.json                  choices only: no keys, tokens or sign-ins   [stage C]
```

Fridae's own `vault/Fridae/Guides` and `Team` stay in the app: they are its instructions and change with app updates
(11-24 decision). Only the Roblox knowledge moves into Knowledge.

## Stage A: the guides join the vault, and one Pull newest

- **The vault repo** (`aprilfridae/fridae-vault`, not created yet; the user presses Create) holds `Skills/` and
  `Roblox/`. Its LICENSE covers LihFolk's notes (CC BY 4.0, credited to him, as `vault/Roblox` is today) next to the
  skills' terms. The engine's `ALLOWED_TOP` becomes `("Skills/", "Roblox/")` on both the Python and Rust sides, with
  the same tests. The manifest limits are raised to fit about 1,400 notes and their pictures (today's `vault/` is
  28 MB); the builder sets the numbers from the real vault plus headroom.
- **Pull newest.** Setup › Skills' "Fridae vault" row becomes its own Setup page or block, **Knowledge vault**, with
  one button: **Pull newest** (it says **Download** before the first one). The 24-hour quiet check stays.
- **The clash question.** Today a clash keeps the user's file and saves the vault's beside it as
  `<name> (Fridae update).md`, listed with Reveal. It becomes a card in the existing "Needs your decision" style:
  the file, a short plain-words view of both changes, and **Keep the vault's** / **Keep mine**. Keep the vault's
  replaces the user's file with the vault's (the user's version is kept in `.fridae-vault/replaced/`, never deleted);
  Keep mine records the choice so the same vault version doesn't ask again. This applies to skills too.
- **Clean combines** are reported in the pull's one-line report ("Updated 3 guides you had edited"), each openable.
- **Combining without git.** Today's combine runs `git merge-file`, which needs git installed. The builder replaces
  it with a three-way merge inside Fridae (or ships one), so Pull newest works on a computer without git.
- **Search reads Knowledge.** The `vault` shelf (keyword search in `rbxvault.py`, the RAG's chunks in
  `rbxrag/chunks.py`) reads `Knowledge/Roblox` instead of the app's `vault/Roblox`. Until the first download it
  falls back to nothing for Roblox knowledge, and Fridae says so with the Download button (11-24 decision: the app
  no longer bundles the guides).
- **Existing users** download like new ones (see "Settled after the builder check"). Notes that superseded a shipped
  note (`superseded.json`) keep working until stage B folds them in.

## Stage B: the user's notes join, and sharing covers everything

- **Notes are filed into Knowledge by topic.** The for-everything notes (`notes/` today, `rbxnotes.home_notes()`) are
  sorted into the best-matching topic folder, or `Inbox/`. Agents' new notes are filed the same way. A note on a topic
  that already exists is added to under a short "Learned in Fridae" section instead of a near-copy. LihFolk's notes
  keep his credit line for his part.
- **The shelves.** `vault` and `yours` both mean Knowledge, so agents' habits keep working. Game-only notes stay with
  their game.
- **Supersede becomes edit.** Shipped notes are the user's files now, so `superseded.json` is folded in: the newer
  note's text joins the old one's "Learned in Fridae" section, and the side file is retired.
- **Taste notebook** `Knowledge/Taste/`, as the 11-24 design describes, written by a cheap OpenCode model.
- **House rules** move to `Knowledge/House rules.md` and are still added to every agent's instructions.
- **Sharing** covers new notes and edits to vault guides, through the same Share your skills flow, renamed
  **Share your knowledge**: the same masking (keys, emails, the computer's username), the same Off / Check each batch
  / Automatic choice, the same licence line. An edit to a vault guide is sent as a change to that guide.
- **The inbox** shows guides and notes the way it shows skills: the file, the change against the vault's version,
  and Add / Edit then add / Reject, each confirmed as today.
- **Publish** publishes skills, guides and notes in one release.

## Stage C: the one-time move into the one folder

- Chats move to `Chats/` (readable: a Markdown copy beside the app's own records), renders to `Made/`, games to
  `Games/`. Temp and tool files, caches, process lists, the RAG index, sign-ins, engines and second accounts move to
  the app's own data folder (Application Support on a Mac, `%LOCALAPPDATA%\Fridae Studios` on Windows).
- `config.json` splits: the user's choices go to `Settings.json`; anything secret stays in the app's data folder.
- **Safety:** nothing is deleted. A full backup is made first and kept until the user confirms in Setup; the move can
  be undone from there. A move that fails part way puts everything back.
- **A new computer:** copy the folder, install Fridae, choose **Use my existing Fridae folder**, sign in again. The
  RAG index rebuilds.
- Running from source (developer mode, `HOME_DIR` is the checkout) keeps working.

## Constraints

- One cross-platform codebase: macOS and Windows behave the same (paths, separators, reserved names, links).
- Fridae never publishes, pushes, closes a submission or creates the vault repo without the user's click and confirm.
  Pull newest only downloads.
- Tests never contact github.com and never install anything. Cheap models only for simple jobs (the Taste writer,
  the topic picker if it is a model).
- All-or-nothing per file; a bad or cut-off pull changes nothing.
- Plain English in the app; "computer", not "Mac".

## Settled after the builder check (designer's calls, for the user to check)

- **No bundled guides, no seeding.** The app stops carrying `vault/Roblox` in stage A, for new and existing users
  alike: everyone presses Download once, and Setup and the Lead say so plainly until they do. Existing users lose
  nothing, because shipped notes were read-only (their additions live in `notes/` and `superseded.json`, which stage B
  folds in). In developer mode `Knowledge/` sits in the checkout and is git-ignored.
- **One home for the guides.** When the maintainer creates the vault repo, Create uploads the current
  `vault/Roblox` (minus the code's data files, below) with the skills scaffolding, in the same confirmed step. After
  that the code repo's `vault/Roblox` is removed; the vault repo is the only copy.
- **The maintainer's own edits take the same route as everyone's (user, 2026-11-25).** When the maintainer edits or
  adds a guide, note or skill in her own Knowledge, it goes out through Share like anyone's, arrives in her own inbox,
  and is added there; Publish only ever publishes what is on the vault repo's `main`. No direct "publish my Knowledge"
  path. Sharing must therefore allow the maintainer's own login (no self-exclusion), and her submissions show in the
  inbox like any other.
- **Code data stays in the app.** `_tools/roblox_api.json`, `api_overrides.json` and `_data/roblox_facts_2026.json`
  are read by code, so they stay in the app and the manifest leaves `_tools/` and `_data/` out.
- **What travels with the vault.** Everything in Knowledge except `House rules.md` and dot-folders: `Skills/`,
  `Roblox/`, other topic folders, `Inbox/` and `Taste/` (shared, per the 11-24 design). The fixed allow-list becomes
  this rule on both the Python and Rust sides, with the same path checks and tests as today.
- **Clash unit.** A skill is decided as a whole folder (never half the vault's, half yours); a note or guide per
  file. Pictures and models (`.blend`, `.fbx`, `.glb`, images) are never combined: a clash shows both by name, size
  and date, with Reveal for each.
- **After Keep mine** the vault's version becomes the starting point, so later vault changes to that file combine
  cleanly instead of asking every release. After Keep the vault's, the user's version goes to
  `.fridae-vault/replaced/`.
- **New vault notes get an overlap check** in stage B (the note version of skills' newcomer check: keep both, keep
  yours, use the vault's), so a vault note that repeats one the user filed under another name doesn't make two
  places hold the same knowledge. In stage A there are no user notes in Knowledge yet, so only same-name applies.
- **Credit (corrected while planning).** `vault/Roblox/LICENSE.md` says the notes, renders and section 45 scripts
  are LihFolk's, under CC BY 4.0, and few notes carry their own credit line, so there is no reliable per-note split.
  Stage A keeps `Roblox/LICENSE.md` and his notes (including "LihFolk Vault V2: Start Here") unchanged, and the
  vault's own LICENSE says that licence still applies to `Roblox/`. Any split between his notes and Fridae's is the
  user's call, made later.
- **Sharing guides comes in stage A (moved up while planning).** Once the code repo's copy is removed, Share and the
  inbox are the only way to change a guide, so guides and edited vault skills become shareable in stage A, and
  "Share your skills" becomes "Share your knowledge".
- **Create runs from a source checkout.** The installed app no longer carries the guides, so Create (which uploads
  them) works only in Fridae run from source, and says so plainly otherwise.
- **Stage C backup and undo.** The backup goes on the same disk; before moving, Fridae checks there is room and, if
  not, says how much is needed and moves nothing. Undo is a reverse move that carries anything made since back with
  it, never a plain restore that would drop new chats, games or notes.

## Left to the builder (the planner may settle these)

- How a new note's topic is chosen: the RAG's nearest notes when it is on, keyword search otherwise, `Inbox/` when
  nothing fits.
- The exact manifest limits for stage A.
- The three-way merge implementation that replaces `git merge-file`.
- How the clash card shows two versions of a long note (a short summary plus "show both").
- Exact folder for each `work/` item in stage C; the Markdown export format for chats.
- Whether the Knowledge vault gets its own Setup page or stays a block on the Skills page.
