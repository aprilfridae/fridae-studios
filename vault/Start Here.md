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
  - `Roblox subjects/`: subject guides from roblox-skills (MIT): Luau, security, data stores, networking, UI and more.
  - `LihFolk/`: LihFolk's guides with their ready-made Luau modules and scripts (CC BY 4.0).
- **Fridae/Team/**: the Studio team. One note per expert (`coder.md`, `ui.md`, ...) says how that expert works;
  `roster.json` lists them. A game can replace any of them in its own `.fridae/team/` folder.

Each game's own notes are not in this folder: `GAME.md` (the game bible and its decision log) and
`.fridae/lessons.md` live in the game's project folder, and the agents search them alongside this vault.

## Rules that are always read in full

The guides `workflow`, `studio`, `critic`, `producer` and `game-bible`, and the app's own instructions, are always
given to the agents in full. Everything else is looked up when needed.

## Credits

Every borrowed part keeps its licence and credit beside it and in `THIRD_PARTY.md` at the top of the project.
