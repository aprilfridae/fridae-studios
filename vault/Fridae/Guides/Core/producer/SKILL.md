---
name: producer
description: "Splitting a large request between parallel agents with rbx_team. Break a large Roblox request into parallel work with rbx_team - when to split, how to cut tasks so workers do not collide in one Studio place, how to write a complete brief, and how to checkpoint, verify, review and integrate the results. Use for big requests that can be split between several agents."
examples:
  - "Split this big request between several agents"
  - "Write briefs for parallel workers"
  - "Integrate and review the workers' results"
applies: game
owner: app
---

# Producer

You are coordinating: you cut the work, brief the workers, check what
comes back, and put it together. `rbx_team` spawns worker and critic
agents (action `spawn` with a task), reports on them (`status`) and blocks
until they finish (`wait`). Check the tool's parameters for exact names.

Workers know **nothing** of this conversation. Each one starts with only
the brief you write, the guides it loads, `GAME.md` and the place itself.

## When to split, and when not to

Split when all of these hold:

- The request has three or more parts that are each substantial (roughly
  more than ten minutes of work for one agent).
- The parts can be built in **different places**: different scripts,
  different folders, different areas of the map.
- The connections between the parts can be written down in advance as
  names and data shapes.

Do it yourself when:

- The work is small, or is one coherent thing (one system, one screen).
- The parts depend on each other in order (B cannot start until A exists
  and has been looked at). Do those in sequence.
- The design is still undecided. Parallel workers multiply an unclear
  plan into several incompatible ones. Settle the design first.
- The parts all edit the same script or the same model.
- It is a bug hunt. Debugging needs one mind following one trail.

Coordination has a cost: briefs, waiting, checking, merging. Two workers
are rarely worth it. **Never more than 5 workers at once**; three or four
is usually the most that stays manageable.

## Cutting the work for one shared place

All workers edit the same open Studio place at the same time. Nothing
merges their changes: the last write wins. Collisions are prevented by the
cut, not repaired afterwards.

- **One owner for every instance.** Each worker gets named containers
  that only it may create or change: a folder in Workspace, a ScreenGui, a
  folder of scripts, a module. List them in the brief.
- **No two workers edit the same script.** If two parts need changes to
  one file, give the file to one worker and have the other expose a
  module that the first one calls, or make that change yourself afterwards.
- **Shared containers are created by you** before spawning: the Remotes
  folder, the shared config module, the folder for shared types. Workers
  add only the entries named in their brief.
- **Shared interfaces are agreed first** and written into `GAME.md`
  under Technical notes, before anyone starts:
  - remote names, direction and exact arguments;
  - module names, paths and the functions each exposes, with signatures;
  - the shape of saved and replicated data (field names and types);
  - attribute and tag names, and the config keys each side reads.
  A worker builds against the written interface, not against another
  worker's unfinished code.
- **Playtests are shared too.** A running playtest blocks editing for
  everyone, and edits made while it runs are thrown away when it stops.
  Tell workers not to start a playtest unless the brief allows it, and
  give that permission to at most one worker at a time. Otherwise run the
  playtests yourself once all workers are idle.
- **Global settings belong to you.** Lighting, Workspace properties,
  StarterPlayer settings, SoundService groups and game settings are not
  touched by workers unless one of them is given explicit ownership.
- Leave the integration points (the one script that wires the systems
  together, the main client loader) to yourself.

If you cannot draw clean borders between the parts, the task is not ready
to split. Do it in sequence instead.

## Writing a brief

A worker that has to guess will guess differently from the others. Each
brief is complete on its own and contains:

1. **Goal**: what to build and what it is for, in two or three sentences,
   including how the player meets it.
2. **Context**: the project folder, that `GAME.md` must be read first,
   which guides to load, and which Studio place if several are open.
3. **Exact names and locations**: every instance, script, remote and
   module it creates or uses, with full paths.
4. **Interfaces**: copied in full from `GAME.md`: what it must provide
   and what it may rely on from others. For things that do not exist yet,
   what to build against (the agreed signature, or a stub you created).
5. **What done looks like**: checks that can be observed. "The shop opens
   from the HUD button, lists the six items from `Config.Items`, and a
   purchase fires `Remotes.BuyItem(itemId)`", not "make a nice shop".
6. **What not to touch**: everything outside its containers, named
   explicitly where there is a risk, and the playtest rule.
7. **What to report**: what was created (paths), what was verified and
   how, what is unfinished or assumed, and anything it needed that the
   brief did not provide.

Also state the style to follow (the style guide and palette from
`GAME.md`) for any worker producing visible things, so the parts look like
one game.

Read each brief once as if you knew nothing else. If a question comes to
mind, answer it in the brief.

## Running it

1. Read `GAME.md`. Write the shared interfaces into it. Create the shared
   containers and any stubs in the place.
2. **Take a checkpoint** with `rbx_project` (action `checkpoint`; run
   `export_scripts` first so the scripts on disk are current). This is the
   state to return to if the split goes wrong.
3. Tell the user the plan in a few lines: the parts, who owns what, what
   you keep for yourself.
4. Spawn the workers with `rbx_team`, all in one go, so they run in
   parallel.
5. `rbx_team` `wait` for the results. While waiting, do only work that
   belongs to you and touches nothing a worker owns. Use `status` if a
   worker runs much longer than the others.
6. **Verify every result yourself by looking.** A worker's report is a
   claim. For each one: inspect the instances it says it made, read the
   key script, capture the screen or the model, and check that it stayed
   inside its containers. `rbx_project` `changes` shows which scripts were
   actually modified.
7. If a result is wrong or incomplete, send the worker back with the
   specific defect, or fix a small thing yourself. Do not integrate on top
   of something unverified.
8. **Integrate**: wire the parts together in the scripts you kept, then
   playtest the whole flow as a player (`playtest`, `player-testing`).
   Most problems show up here, at the borders between parts.
9. **Spawn a critic** with `rbx_team` when it works end to end. Tell it
   what to review and where it is; do not tell it the work is good. It
   follows the `critic` guide. Fix the blockers and majors it reports, or
   say why not.
10. Take a second checkpoint, update `GAME.md` (content list, UI
    inventory, Technical notes, Decision log), and report.

If the result is beyond repair, `rbx_project` `restore_scripts` returns
the scripts to the checkpoint. It restores scripts only: instances that
workers created in the place must be removed by hand, which is one more
reason every worker has named containers.

## Worked example

Request: "Add a pet system: eggs you buy and hatch, pets that follow you
and boost coins, and an inventory to equip them."

Agreed first and written into `GAME.md`:

- `ReplicatedStorage.Shared.PetConfig`: pets (id, name, rarity,
  multiplier, model name) and eggs (id, price, drop weights). Created by
  the producer, with numbers checked through `rbx_economy`.
- Remotes in `ReplicatedStorage.Remotes`: `HatchEgg(eggId)` client to
  server, returns the pet id or an error; `EquipPet(petUid, equipped)`
  client to server; `PetsChanged(petList)` server to client.
- Saved data: `profile.Pets = { {uid, id, equipped} }`, at most 3 equipped.
- Pet models live in `ReplicatedStorage.Assets.Pets.<model name>`.

| Worker | Builds | Owns | Must not touch |
|---|---|---|---|
| A: server | `PetService`: hatch rolls on the server, validation, equip limit, saving, the coin multiplier hook | `ServerScriptService.Pets` | Client code, UI, Workspace |
| B: inventory UI | The pets screen and its HUD button: list, equip, unequip, updates on `PetsChanged` | `StarterGui.PetsGui`, `StarterPlayerScripts.PetsUI` | Other ScreenGuis, server code |
| C: pet models | Six pet meshes, modelled, imported, named to match `PetConfig` | `ReplicatedStorage.Assets.Pets` | Scripts, Workspace |
| D: world and follow | The egg stands in the map and the client script that makes equipped pets follow their owner | `Workspace.Map.EggArea`, `StarterPlayerScripts.PetFollow` | The rest of the map, UI |

The producer keeps: `PetConfig`, the remotes, the one-line hook in the
existing coin script that applies the multiplier, the hatch effect and
sound once the parts exist, all playtests, and the final wiring.

Brief for worker B, in outline: goal (players open a pets screen from the
HUD, see what they own, equip up to three); read `GAME.md` and load
`workflow`, `ui-design`, `sim-ui`; create `StarterGui.PetsGui` and
`StarterPlayerScripts.PetsUI` only; use `PetConfig` for names and
rarities, listen to `Remotes.PetsChanged`, fire `Remotes.EquipPet`; until
the server exists, drive the screen from a local sample list of the agreed
shape; done when the screen opens and closes from the button, shows the
sample pets with rarity colours, marks equipped ones, refuses a fourth,
and fits a phone layout; do not start a playtest, do not edit other GUIs
or any server script; report paths created, captures taken, and
assumptions.

After all four return: verify each, connect the hook, playtest hatch to
equip to coin boost as a new player, spawn the critic, fix, checkpoint.

## Report to the user

- What was built, by part, with where it lives.
- What you verified yourself and how (playtest, captures).
- What the critic found, what was fixed, and what was left and why.
- What is not done or not verified.
- That `GAME.md` was updated, and the checkpoints taken.
