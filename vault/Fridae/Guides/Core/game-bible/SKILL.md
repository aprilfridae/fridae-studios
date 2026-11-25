---
name: game-bible
description: "Game: the GAME.md format and rules; writing one for an existing game. The GAME.md game bible - its exact template, the rules for reading and updating it, and how to write one for an existing Roblox game by inspecting the place. Use at the start of work on any project, and whenever a design decision is made or changed."
examples:
  - "Write a GAME.md for my existing game"
  - "Record this design decision in GAME.md"
  - "Start work on this project by reading its bible"
applies: game
core: true
---

# Game bible (GAME.md)

`GAME.md` sits at the top of the project folder. It is the single written
agreement between the user and every agent about what the game is. Chats
end and agents change; this file is what carries decisions forward.

## Rules

1. **Read it at the start of every task**, before inspecting the place. If
   the project has none, write one (see the last two sections) before doing
   substantial work.
2. **Never contradict it silently.** If a request conflicts with it (a new
   feature that breaks a pillar, a price that differs from the table,
   something listed as out of scope), say what conflicts and ask which wins.
   Then either do the request and update the file, or leave both as they are.
3. **Update it in the same turn** that something is decided or changed: a
   new system, a renamed remote, a changed price, a dropped feature. Add a
   Decision log line every time. Work that changes the game without changing
   the file is unfinished.
4. **Keep it short and factual.** Aim for under 300 lines. State what is,
   not what might be nice. No marketing language, no history outside the
   Decision log. When a section grows long, trim it to what a new agent needs.
5. **Numbers live in one place each.** In the file: the table under
   Progression and economy. In code: one config module, named in Technical
   notes. Change both together; never copy a number into a third place.
6. Mark anything not confirmed by the user as `(inferred)` or put it under
   Open questions. Do not present a guess as a decision.
7. Edit sections in place. Do not append a new version of a section below
   the old one.

When the project is a git repository, take a checkpoint with `rbx_project`
(action `checkpoint`) after a meaningful change to the file, so the design
history is recoverable.

## Template

Use these sections, in this order, with these headings. Keep a section that
does not apply yet and write "None yet" in it.

```markdown
# <Game name>

## One-line pitch
<What the player does and why it is fun, in one sentence.>

## Pillars
1. <Quality every decision is checked against>
2. <...>
3. <...>

## Core loop
<Action -> reward -> reason to repeat, for the 30-second loop.>
<Then the longer loops: per session, per day.>

## Player and devices
- Audience: <age range, what they already play>
- Primary device: <phone / tablet / PC / console>; also supported: <...>
- Session length: <minutes>; natural stopping point: <...>
- Players per server: <n>; solo / co-op / competitive

## Progression and economy
<What grows over time and what the player returns for.>

| Item | Cost | Gives | Time to afford | Notes |
|---|---|---|---|---|
| <name> | <n> <currency> | <effect> | <minutes> | <...> |

- Currencies: <name: earned how, spent on what>
- Rebirth / prestige: <rule and multiplier, or "none">
- Drop tables: <item: chance, pity rule>
- Monetisation stance: <none / cosmetic / convenience>; products: <...>

## Art direction
- Style guide: <brutalist-skill / minimalist-skill / soft-skill>
- UI direction: <clean and restrained by default; the user's own design files, fonts and colour values when they give them>
- Palette: <named colours with RGB>
- Materials: <which Roblox materials, which are banned>
- Lighting: <technology, time of day, mood>
- References: <links or file paths, with what to take from each>

## Audio direction
<Mood, music style, which events have sound, SoundGroups and levels.>

## World and content list
- <Area / map / mode>: <state: planned, blocked out, built, polished>
- <Items, enemies, pets, stages: counts and names>

## UI inventory
- <Screen or HUD element>: <purpose> - <path in the place> - <state>

## Technical notes
- Data saved: <store name, key format, fields, schema version>
- Remotes: <name: direction, arguments, who validates>
- Architecture: <where server, client and shared code live; framework>
- Config module: <path of the one module that holds the numbers>
- Known constraints: <streaming, max parts, target memory>

## Naming conventions
<Instances, scripts, remotes, attributes, tags, asset names.>

## Out of scope
- <Not in this version>
- <Never in this game>

## Open questions
- <Question> - <who answers, what is blocked by it>

## Decision log
| Date | Decision | Why |
|---|---|---|
| 2026-01-31 | <what was decided> | <reason in one line> |
```

Notes on filling it in:

- Pillars are qualities, not features: "readable at a glance on a phone",
  "always something to buy within two minutes", "funnier with a friend".
- The economy table holds real numbers produced with `economy-design` and
  checked with `rbx_economy`. "Time to afford" is what makes a table
  reviewable; do not leave it out.
- Paths are full instance paths (`ReplicatedStorage.Shared.Config`) or file
  paths under `src/`, so another agent can go straight there.
- Decision log dates are absolute (`2026-03-14`), newest at the bottom. One
  line each. A reversed decision gets a new line; the old one stays.

## Using it during work

- Starting a task: read the file, then check that the parts the task touches
  still match the place. If the place and the file disagree, the place shows
  what exists and the file shows what was agreed: report the difference and
  ask before changing either.
- Planning a feature: check it against the pillars and Out of scope first.
- Naming anything new: follow Naming conventions. If none covers the case,
  choose, then add the rule.
- Finishing a task: update World and content, UI inventory, Technical notes
  and the Decision log as needed. Mention in the reply that the file changed
  and what changed.
- Before splitting work between agents (`producer`): write the shared
  interfaces into Technical notes first.

## Writing it for an existing game

When the user brings a game that already exists, build the file from the
place, then ask only for what cannot be inferred.

1. Run `rbx_project` with action `info`, then `export_scripts` so the
   place's scripts are in `src/`. Take a `checkpoint` if the project is a
   git repository.
2. Inspect the place in Studio with `execute_luau` (Edit) and the query
   tools:
   - top-level layout of Workspace, ReplicatedStorage, ServerScriptService,
     ServerStorage, StarterGui, StarterPlayer;
   - StarterGui: each ScreenGui and its main frames (the UI inventory);
   - Workspace: maps, zones, spawn locations, part and mesh counts;
   - Lighting: technology, time, atmosphere and post effects;
   - SoundService and Sound instances: SoundGroups, music, counts;
   - `game.Players.MaxPlayers`, `workspace.StreamingEnabled`, the avatar
     type (R6 or R15), and `game.CreatorType`.
3. Read `src/` for the systems:
   - RemoteEvents and RemoteFunctions: names, arguments, validation;
   - DataStoreService use: store names, keys, the saved fields;
   - config or settings modules: prices, rewards, multipliers, drop rates.
     Copy the actual numbers into the economy table;
   - MarketplaceService use: game passes and developer products;
   - the framework and folder structure in use.
4. Take a few `screen_capture` images of the world and the main UI and
   derive the art direction from them: style family, palette, materials.
5. Run a short playtest (`playtest` guide) to see the loop a new player
   actually meets, and write the Core loop from what happens, not from what
   the code suggests.
6. Write `GAME.md`. Everything taken from the place rather than from the
   user is marked `(inferred)`. Pillars written from observation are
   inferred too.
7. Ask the user for what a place cannot tell you, one question per message:
   - the intent: pitch, pillars, who it is for;
   - what is finished, what is abandoned, what is planned;
   - what must not be changed;
   - the monetisation stance and any live players or saved data that
     changes could break;
   - what they consider out of scope.
8. Replace `(inferred)` marks with the confirmed answers, move anything
   unanswered to Open questions, and tell the user the file is ready to read.

A live game has saved player data. Record the data format exactly, and treat
any change to it as a decision that needs the user's approval.
