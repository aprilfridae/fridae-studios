# Third-party material in rbxkit

## Design guides from open-design

These folders under `skills/` are copied unchanged from
https://github.com/nexu-io/open-design (`skills/`), which is licensed
Apache-2.0 (`licenses/open-design-LICENSE.txt`). Folders that carry their own
`LICENSE` file are under that licence instead (MIT); each folder's
`SOURCES.md` or front matter names its upstream author.

- `brutalist-skill`
- `emil-design-eng`
- `minimalist-skill`
- `motion-design`
- `redesign-skill`
- `reference-design-contract`
- `soft-skill`

## Roblox subject guides from roblox-skills

`guides/roblox-skills/` is copied unchanged from
https://github.com/EL4CTEO/roblox-skills (version 1.1.2), MIT licensed
(`licenses/roblox-skills-LICENSE.txt`). 22 guides (its `roblox-studio-mcp`
guide is left out because rbxkit's own workflow guide covers the same ground): Luau, architecture,
security, networking, data stores, UI, world building, physics, audio, input,
NPC AI, monetization, testing, performance and more.

## Check-UI

The simulator UI look described in `skills/roblox-sim-ui` follows Check-UI
Clean Roblox by diggofun / TemporaryLooker22 (MIT). See
`skills/roblox-sim-ui/LICENSE.md`.

## LihFolk Vault V2

`guides/lihfolk-vault/` (1,330 notes, the rigs in `18 Rigs and Skeletons`, the Blender build scripts in
`45 Procedural Blender Assets/scripts`, the dated API snapshot) and `guides/lihfolk/` (11 guides with their Luau
modules, examples, scripts and rubrics) are from LihFolk Vault V2 by LihFolk,
https://github.com/lihfolk-cy/v2-vault, licensed CC BY 4.0 (`licenses/lihfolk-vault-LICENSE.md`).
Changes: guide text that pointed at the Claude Code plugin's folders now points at the `rbx_vault` tool;
mentions of LihFolk guides left out (because rbxkit already had one on the subject) point at rbxkit's.
`rbxvault.py` is LihFolk's `_tools/vault.py` search, reading from rbxkit's folders.
Left out: the Obsidian theme, renders and images, the LihFolk UI kit, Figma, asset-pack, council, hook and
benchmark parts, and guides whose subject rbxkit already covered (secure remotes, anti-exploit review,
DataStore safety, performance audit, monetization compliance, UI builder). The LihFolk Asset Pack is not included.

## unslop

`skills/unslop/` (its SKILL.md and references) is from unslop by Damir Sokolovsky (@heydamir),
https://github.com/heydamir/unslop, licensed CC BY 4.0 (`licenses/unslop-LICENSE.txt`). Changes: an
"In Roblox" section was added near the top; its Node snapshot and story-video scripts and its HTML examples are
not included.
