# One Fridae Studios vault, in one portable folder

Asked for by the user on 2026-11-24:
- "I want you to make this easily transportable ... right now it doesnt seem organized to my little human brain."
- "the lihfolk vault will never be updated. JUST make my notes and his notes unified ... i dont like having two places with similar information. its all KNOWLEDGE."
- "credit lihfolk for HIS part of the notes but its not HIS Vault anymore its fridae studios vault"

The user approved the layout below the same day. Build it after the local RAG (`2026-11-23-fridae-rag-design.md`) lands, because both touch `rbxvault.py`.

## Intent

All of the user's Fridae data lives in one folder they can understand at a glance and copy to another computer. Everything learned is one body of knowledge, the **Fridae Studios vault**: the Roblox knowledge that shipped with the app (LihFolk Vault V2) plus everything the user and the AIs learn, filed together by topic.

## Layout

```
~/Documents/Fridae Studios/      the one folder to copy
  Knowledge/                     the Fridae Studios vault (an Obsidian vault)
    House rules.md
    Taste/                       one note per aesthetic word the user uses ("Softer.md")
    01 Studio and Workflow/ ...  the shipped topic folders; new notes are filed into these
    <new topic folders>          e.g. general programming knowledge from plain projects
  Games/                         each game: its files, GAME.md, .fridae/notes (game-only notes)
  Chats/                         chat history, readable
  Made/                          promos and images the user made
  Settings.json                  the user's choices: no keys, tokens or sign-ins
```

Everything else moves to the app's Application Support folder: temp and tool files, caches, process lists, the RAG index, sign-ins, engines, and the second accounts (already there). None of it is the user's to carry; it is rebuilt or redone on a new Mac.

## Behaviour

- **The vault becomes the user's.** On first run of this version, Fridae copies the shipped Roblox vault into `Knowledge/` and stops treating it as app content. Updates never overwrite or "refresh" it; the shipped copy only seeds a new install.
- **Notes are filed into the vault by topic.** When an agent saves knowledge (`rbx_vault` action `note`), it is filed into the best-matching topic folder and linked (`[[...]]`) to related notes. There is no separate "yours" pile for general knowledge.
- **No duplicates.** When a note on the topic already exists, the agent adds to it under a short "Learned in Fridae" section instead of creating a near-copy. This applies to LihFolk's notes too.
- **Credit LihFolk for his part only.** His original notes keep their CC BY 4.0 credit line. The vault itself is named and presented as the Fridae Studios vault, never as his. New notes carry no LihFolk credit, and a note of his that gains a "Learned in Fridae" section keeps his credit for his part.
- **Taste notebook.** `Knowledge/Taste/` holds one note per word like "softer", "juicy" or "naturalistic". Each says what the user means in concrete terms (sizes, colours, motion), with liked and rejected examples. Agents save or update one when the user uses such a word and then approves or rejects the result, when the user corrects them, or when they work out something reusable. A cheap OpenCode model (DeepSeek/GLM) writes the note. Before designing, agents look up any taste words in the request.
- **House rules** sit at the top of the vault and are still added to every agent's instructions.
- **Game-only notes stay with their game** (`<game>/.fridae/notes`), so they travel with it. Everything general goes in the vault.
- **Fridae's own agent instructions stay in the app.** The guides and team docs change with updates and are app internals, not "similar information".
- **Search** (keyword or RAG) treats `Knowledge/` as one shelf. The old shelf names `vault` and `yours` both mean it, so existing agent habits keep working.
- **One-time move of existing data.** Fridae sorts the user's current notes into topics, moves `work/` contents to their new homes (chats to `Chats/`, renders to `Made/`, the rest to Application Support) and moves `games/` to `Games/`. Nothing is deleted: a full backup is kept until the user confirms, and the move can be undone.
- **Moving to a new computer:** copy the folder, install Fridae, choose "Use my existing Fridae folder" on first launch, and sign in to the AIs again. The RAG index rebuilds itself.

## The vault is its own download, with updates that never override the user's own (user, 2026-11-24)

The user asked: "make the vault a specific download from the app that only grabs new changes and doesnt override your
own."

- The app no longer bundles the vault. Setup has a **Knowledge vault** row with **Download** (the whole vault into
  `Knowledge/`) and **Check for updates**. Fridae may also check quietly on its own; it only downloads, never uploads.
- Each Fridae vault release publishes a manifest: every file's path and SHA-256, and the files themselves, from
  Fridae's own GitHub release or tagged repo. Every file is checked against its hash before use. An update fetches only
  the files whose hash changed.
- Fridae keeps a hidden base record (`Knowledge/.fridae-vault/base.json`) holding the hash of each file as last
  installed. On update it compares base, local and new:
  - A new upstream file is added (unless the user already has a file at that path, in which case the new one is saved
    beside it).
  - A file unchanged locally since the base is replaced by the new version.
  - A file edited locally and unchanged upstream is left alone.
  - A file edited both locally and upstream gets a three-way text merge. If that is clean it is applied; otherwise the
    user's version stays as it is, the new one is saved beside it as `<name> (Fridae update).md`, and it is flagged in
    Review.
  - A file the user deleted is recorded and never brought back.
  - A file removed upstream is deleted only if it is unchanged locally.
  - The user's own notes, `Taste/` and `House rules.md` are never touched.
- The update is all-or-nothing per file: written to a temporary file, then renamed. The base record is updated only
  for files that were applied.
- Seeding: a fresh install with no network can still work through keyword search on the user's own notes, and offers
  the download when online. The first-run move (above) treats the user's existing copy as both base and local, so
  nothing they have is overwritten.

## Community vault: everyone's notes come back as submissions the maintainer reviews (user, 2026-11-24)

The user, who maintains Fridae: "everyones notes are uploaded, and sumbitted as requests that i then go over and see
if its worth adding, i will incrimentally add them if i think they are good additions. this needs to work communally."
They chose GitHub over a hosted server.

- **The official vault gets its own public GitHub repo** (proposed `aprilfridae/fridae-vault`), separate from the code
  repo so submissions don't flood it. Vault releases and the update manifest (above) are published from it.
  Creating the repo is the user's yes at build time.
- **Contributing is each user's choice.** At setup and in Setup: "Share what Fridae learns with the community?" with
  Off / Review each batch first / Send automatically. The default is Off until the user answers. Asked once, changeable
  any time.
- **What gets sent:** general knowledge notes the AIs saved or updated in `Knowledge/`, including `Taste/` notes and
  "Learned in Fridae" additions. **Never sent:** game-only notes (`<game>/.fridae/notes`), GAME.md or anything in
  `Games/`, `House rules.md`, chats, settings, or anything outside `Knowledge/`.
- **Before anything leaves the Mac:**
  - Secrets are masked with the existing masker (`compact::mask_with`): keys, tokens, emails, and home-folder paths
    with the username.
  - Notes are checked against the official vault so unchanged shipped notes are not re-sent.
  - In "Review each batch first", the user sees exactly what will be sent and can drop notes.
- **How it reaches the maintainer:** each contributor signs in to GitHub once (a one-click device sign-in like the AI
  sign-ins; Fridae keeps only what GitHub's sign-in gives, in the app's own folder). A batch is sent as a pull request
  from their fork: one branch per batch, each note a file at its topic path, and a PR description listing each note's
  topic and why it is useful (one line each, written by a cheap OpenCode model). Batches are at most daily and at
  most 20 notes, to keep the inbox sane.
- **The maintainer's inbox:** in the maintainer's copy of Fridae (their GitHub login owns the vault repo), a
  **Submissions** screen lists open PRs and each note in them, with a preview and a diff against the vault's current
  note. Each note gets **Add**, **Edit then add**, or **Reject**. Accepted notes are committed to the vault repo's main
  branch, and accepting part of a PR is fine. The PR is closed with a thank-you when every note is decided. GitHub's
  own PR page works too.
- **Credit:** each accepted note records its contributor's GitHub name in the commit (and in the note's credit line if
  the contributor allows it). LihFolk keeps his credit on his notes.
- **Distribution:** what the maintainer accepts goes out in the next vault release and reaches everyone through the
  update that fetches only changes and never overrides their own notes (above).
- **Abuse:** submissions only become official through the maintainer's review. Fridae never applies a submission to
  anyone's vault by itself.

Open for the build: the GitHub OAuth app (the user registers it once; a device-flow client id is public), the
fork/PR scopes needed, and how the inbox signs in as the maintainer.

## Anti-goals

- Losing or overwriting a single user note during the move or on any update.
- Keys, tokens or sign-ins inside the portable folder.
- Two places holding the same kind of knowledge.
- Removing LihFolk's credit from his own notes.

## Open for the build

- How the topic for a new note is chosen: the RAG's nearest notes when it is on, keyword search otherwise, with a fallback "Inbox" folder when nothing fits.
- Exact folder for each `work/` item; how chats are made readable (Markdown export beside the app's own records).
- Windows paths (later).
