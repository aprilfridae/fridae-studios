# Start Here

This folder is Fridae Studios' knowledge vault: everything the app's AI agents know about making Roblox games, in
plain Markdown. Open the whole `vault` folder in Obsidian (or any Markdown editor) to read it. The agents never load
it all at once: they search it with the `rbx_vault` tool and read only the section they need.

## What is in it

- **Roblox/**: about 1,300 notes on Roblox development, a trimmed copy of
  [LihFolk Vault V2](https://github.com/lihfolk-cy/v2-vault) by LihFolk (CC BY 4.0, see `Roblox/LICENSE.md`).
  Its own front page is [[00 Start Here]]. The numbered folders are its topics: scripting, data, UI, building,
  effects, animation, game design, monetisation, performance, publishing and more.
- **Fridae/Guides/**: the how-to guides the agents follow, one folder per guide with the guide itself as
  `<name>.md` and any reference files beside it.
  - `Core/`: Fridae's own guides (workflow, Studio mode, playtesting, UI, effects, game design, economy, promo...)
    and the design guides adapted from open-design and unslop. Claude Code users also get these as skills.
  - `Roblox subjects/`: subject guides from roblox-skills (MIT, see `Roblox subjects/LICENSE.txt`): Luau, security,
    data stores, networking, UI and more.
  - `LihFolk/`: LihFolk's guides with their ready-made Luau modules and scripts (CC BY 4.0, see `LihFolk/LICENSE.txt`).
- **Fridae/Team/**: the Studio team. One note per expert (`coder.md`, `ui.md`, ...) says how that expert works;
  `roster.json` lists them. A game can replace an expert with its own `.fridae/team/experts/<id>.md` and the team
  with its own `.fridae/team/roster.json`, in the game's project folder.

## Your notes and house rules

Fridae keeps notes when you steer it: when you correct an agent, turn down something it made or point it at a
better way, the agent saves a short note of what it was doing and what you want instead. Next time, agents search
those notes before they start, so they reach your answer sooner. Your notes are not in this folder (it is replaced on
every update): they stay on your computer, with your own data.

- **For everything**: the `notes` folder in Fridae's home folder (`Documents/Fridae Studios/notes` in the app). Search
  shelf `yours`; it works in any chat, with or without a game.
- **For one game**: `.fridae/notes/` in the game's project folder. Search shelf `game`, beside `GAME.md` (the game bible
  and its decision log).
- **House rules**: `House rules.md` in either folder, one bullet per standing preference ("always...", "never...").
  The app gives them to every agent, the Lead and the experts included, at the start of every turn, so they are
  followed, not looked up. Together they hold about 4,000 characters at most.

Each note is a plain Markdown file: open the folder (Setup, General, Your notes, Open notes folder) to read, edit or
delete them. An agent asked to forget a note moves it to the folder's `.trash` instead of deleting it.

## Rules that are always read in full

The guides `workflow`, `studio`, `critic`, `producer` and `game-bible` always come whole when loaded, and the app's
own instructions and your house rules are always given to the agents in full. Everything else is looked up when
needed: an agent asks for a guide's outline (its opening, its rules sections in full and its list of sections) and
reads the sections it needs.

## Credits

Every borrowed part keeps its licence and credit beside it and in `THIRD_PARTY.md` at the top of the project.
