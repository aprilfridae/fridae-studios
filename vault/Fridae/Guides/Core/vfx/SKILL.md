---
name: roblox-vfx
description: "Visual effects: modules, capture, textures, meshes. Build, improve or check Roblox visual effects - hits, slashes, explosions, spells, projectiles, beams, auras, pickups, lightning, an ability's look. Covers the Emit and Lightning modules, frame-by-frame capture in Studio, drawing particle textures and flipbooks, and effect meshes. Use before any VFX work."
examples:
  - "Make a sword slash effect"
  - "Build an explosion for the rocket launcher"
  - "Add a glowing aura to the boss"
  - "Capture an effect frame by frame and check it"
applies: game
---

# Roblox visual effects

An effect is ParticleEmitters, Beams, Trails, lights and a few thin meshes,
played from attributes by one module. It is not a solid object modelled in
Blender; Blender and numpy only make its textures and thin mesh shapes.

## What to read

Always, before building: `references/vfx-basics.md` (what an effect is made
of, layers, timing, brightness, textures, budgets, what to ask of a capture).

If this folder also holds the longer topic files, read the ones the effect
needs: `vfx-design.md` and `vfx-craft.md` (always), `vfx-textures.md`,
`vfx-motion.md`, `vfx-mesh-shapes.md`, `vfx-camera-world.md`,
`vfx-rendered-flipbooks.md`, `vfx-lightning.md`. Where one of them describes
a one-call tool, use the steps in this file instead; this file is the
authority on tools.

## The kit

Everything lives in `{{RBXKIT_WIN}}`. Run the command line as
`python {{RBXKIT}}/rbx.py <command>`.

| Need | Use |
|---|---|
| Effect player | `studio/Emit.lua` -> `ReplicatedStorage.VFX.Emit` |
| Lightning, impact frames | `studio/Lightning.lua` -> `ReplicatedStorage.VFX.Lightning` |
| Frame-by-frame capture | `studio/vfx_capture.lua` + `screen_capture` |
| Draw textures / flipbooks | `rbx.py tex --script file.py --folder <Effect>` |
| Check any PNG or sheet | `rbx.py sheet <paths>` |
| Mesh shapes, rendered flipbooks | `rbx.py blender` with `rbx.vfx_arc`, `vfx_ring`, `vfx_cone`, `vfx_swirl`, `vfx_shell`, `vfx_surface`, `rbx.flipbook` |

### Installing the modules in a place

Check first: `search_game_tree` on `ReplicatedStorage`, keywords `VFX`. If
`ReplicatedStorage.VFX.Emit` is missing, run `rbx.py package`. It builds the
two modules into one file, copies it into this computer's Studio install, and
prints an `install_luau` line: run that line with `execute_luau` (Edit). It
adds `Emit` and `Lightning` to `ReplicatedStorage.VFX` and skips any that are
already there, so a copy the user edited is never replaced. Fallback if the
load fails: read `studio/Emit.lua` and create the ModuleScript with
`multi_edit` (`className: "ModuleScript"`, first edit with empty
`old_string`). In a Rojo project put the files in the synced tree instead.

To try the modules without adding them to the place, load them unparented:
`local vfx = game:GetObjects("rbxasset://rbxkit/RbxKitVFX.rbxmx")[1]` and
`require(vfx.Emit)`.

The modules are readable source, not minified. Extend them when an effect
needs more, keeping the attribute names.

## How to work

1. **Plan** the role, the size from the player's camera, the shapes and about
   five colours (`references/vfx-basics.md`).
2. **Reference.** Use the user's images or video (`rbx.py video frames <file>`
   tiles a video's frames so they can be looked at). With none, plan from the
   description; there is no reference-sheet generator.
   Organic textures that are hard to draw in code (smoke, flame tongues,
   a jagged impact star) can be generated with `rbx.py gen "..." --kind
   particle` (automatic with a Gemini key, otherwise the user makes it in
   their Gemini chat and it is picked up); shapes you can name with
   parameters are still better drawn.
3. **Build a template** Model in `ReplicatedStorage.VFX` with `execute_luau`
   (Edit), timed with attributes, using built-in textures as stand-ins
   (`references/vfx-basics.md`).
4. **Capture it** (below), fix what the frames show, repeat until the timing
   and shapes work.
5. **Draw the textures** (below), try them with the free preview ids, capture
   again.
6. **Upload** the settled textures once, swap the ids in, wire the effect
   into the game, and ask the user to watch it at full speed.

## Capturing an effect

Effects play in the edit viewport, so no playtest is needed. A screenshot
takes longer than a burst lasts, so the effect is held still for each frame.

1. Once per Studio session, run the contents of `studio/vfx_capture.lua`
   through `execute_luau` (Edit). It defines `shared.RbxVfx`.
2. Start: `return shared.RbxVfx.start("Slam", Vector3.new(0, 3, 0))`. It
   returns the duration, what `Emit.check` finds, and a `camera` and
   `look_at` position. Options table as third argument: `size` (studs across
   to fit), `view` (`iso`, `front`, `back`, `left`, `right`, `top`),
   `distance` (camera distance, for the player's-eye view), and `play`: Luau
   that starts the effect itself and returns its handle, with `Emit`,
   `effect`, `at` and `parent` defined (for projectiles, chains, lightning).
3. For each moment, in ascending order:
   `return shared.RbxVfx.to(0.1)`, then `screen_capture` with
   `camera_position` and `look_at_position` from step 2. Two calls a moment.
   Pick the 4-6 that matter: anticipation, flash, peak, body at about 0.3 s,
   smoke, and just past the end to see nothing is left behind.
4. `return shared.RbxVfx.stop()` when done. It removes
   `Workspace.RbxVfxPreview` and everything the capture started.

- **Trails** lose their segments when held: `shared.RbxVfx.to(t, 0.04)` keeps
  the effect crawling instead of stopped.
- **Both views.** Capture the close view to judge shapes and the far view
  (`distance` = how far the player's camera really is) to judge whether it
  reads. Restart with `start` for the second view.
- **On a character in a playtest:** start the playtest (`start_stop_play`),
  run `vfx_capture.lua` with `datamodel_type: "Client"`, drive it the same
  way, and capture with no camera arguments so the frame is the player's
  view. Stop the playtest afterwards.
- A module of your own that `play` requires is cached by Studio in edit
  mode: `require(module:Clone())`.

`references/vfx-basics.md` lists what to ask of each frame.

## Textures

Write a Python script that draws with the `rbx` helpers, and run it:

```
python {{RBXKIT}}/rbx.py tex --folder Fireball --script draw.py
```

Inside the script `rbx.draw_texture`, `rbx.draw_flipbook` and every `tex_*`
helper listed below exist with the same names and arguments
(`tex_coords`, `tex_polar`, `tex_noise`, `tex_cells`, `tex_sample`,
`tex_curve`, `tex_stroke`, `tex_blob`, `tex_warp`, `tex_edge`, `tex_ease`),
plus `rbx.ring`, `rbx.glow` and `rbx.flare`. `numpy` is imported as `numpy`
and `np`. This runs in plain Python, not Blender, so it is fast.

Files go to `rbxkit/work/vfx/<folder>`. The command then reports, from the
pixels of every file written: the grid, each cell's coverage, problems (empty
cells, drawing cut off at a cell edge), notes (frames that hold), the
`FlipbookLayout`, `FlipbookMode`, `LightEmission` and `Lifetime` to use, a
`look_at` contact sheet on a dark ground (read it), and a `preview_id`.

**Preview ids** (`rbxasset://textures/rbxkit/...`) are the PNG copied into
this computer's Studio install. Set a `ParticleEmitter.Texture` to one and it
plays as an upload would, for free. A redrawn texture gets a new id. They
work on this computer only: replace every one before calling the work done
(walk `game:GetDescendants()` for textures starting with that prefix).

`rbx.py sheet <png> ...` gives the same report for any PNG made another way.

## Meshes and rendered flipbooks

```
python {{RBXKIT}}/rbx.py blender --model SlamShapes --script shapes.py
```

In the script, `rbx.vfx_arc(...)` and the other shape helpers make objects in
a `VFX` collection; `rbx.export_model("VFX", name="SlamShapes")` writes the
FBX. `rbx.flipbook(name, grid=4, mode="additive", folder=WORK + "/vfx/<Effect>")`
renders the scene's animation through `scene.camera` into a sheet; check it
with `rbx.py sheet`. Blender starts fresh on each call and reloads the saved
`.blend`, so variables do not persist between calls, but the scene does.

## Uploading (textures and meshes)

Uploads are permanent and moderated. Settle the look on previews first, look
at every image, tell the user how many are going up, and ask before uploading
anything that shows blood, gore or other content Roblox may moderate.

Textures: `open_cloud_invoke` POST `/assets/v1/assets` with
`assetType: "Image"`, the PNG as `fileContent`, scopes `asset:read` and
`asset:write`; poll the returned operation for the asset id. Meshes: the same
with `assetType: "Model"` and the FBX, then `insert_asset`. Details, the
creator (user or group) to upload as, and the placement script are in
roblox-studio-workflow.

## Rules worth keeping in view

- A burst emitter has `Rate = 0` and an `EmitCount`; give streams an
  `EmitDuration`. An emitter with neither ignores slow motion and never stops.
- Beams and trails need two attachments and a cleanup owner.
- Play effects on clients; the server sends a name and a CFrame over an
  `UnreliableRemoteEvent`. Never let damage wait on an effect.
- Pool frequent effects, and budget particle counts, lights and
  post-processing for phones.
