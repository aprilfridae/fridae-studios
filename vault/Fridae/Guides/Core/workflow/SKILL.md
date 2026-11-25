---
name: roblox-studio-workflow
description: "Start here: tools, ground rules, 3D models, animations, icons, uploads. How to work in the user's open Roblox Studio with the Roblox Studio MCP tools and the rbxkit toolkit - inspect the place, edit scripts, change edit-mode state, model in Blender and import it, animations, icons, uploads, video references, data resets. Use at the start of any Roblox Studio task, before the more specific roblox-* skills."
examples:
  - "Add a coin pickup to my open Studio place"
  - "Model a sword in Blender and put it in the game"
  - "Make an icon for the shop button"
  - "Upload an animation and play it on the character"
applies: game
core: true
owner: app
---

# Roblox Studio workflow

Two things do the work:

- The `mcp__Roblox_Studio__*` tools. Load the ones a task needs in one
  ToolSearch call. Call `list_roblox_studios` first and pass its `id` as
  `studio_id` everywhere; if several places are open, ask which one.
- **rbxkit**, a local toolkit at `{{RBXKIT_WIN}}`: headless Blender,
  texture drawing, icon cutting, animation and video tools, plus Luau
  snippets in `studio/` to paste into `execute_luau`. Run it as
  `python {{RBXKIT}}/rbx.py <command>`; `rbx.py --help` lists the
  commands. Its files are plain source; read one before using it the first
  time in a session.

## Which guides to read

Find what the other guides say about the request before starting, not just
this one: search them (from rbxkit, `rbx_vault` with shelf `guides`) and read
the sections that apply. Read a whole guide when the task is centrally about it.

| Request | Also read |
|---|---|
| Modelling, building, props, maps | `roblox-world-building`; the style guide matching the game's look (`brutalist-skill`, `minimalist-skill` or `soft-skill`); `reference-design-contract` when there is a reference image; `roblox-building` if installed |
| UI | `roblox-ui-design`, `roblox-ui`, `web-to-roblox`, the matching style guide, `emil-design-eng`; `redesign-skill` for UI that already exists |
| Visual effects | `roblox-vfx`, `motion-design`, the matching style guide |
| Animations, cutscenes | `roblox-characters-animation`, `motion-design` |
| Playtesting, tests | `roblox-playtest`, `roblox-testing` |
| Any scripting or gameplay code | `roblox-luau`, `roblox-architecture`, `roblox-security`, `roblox-networking`, plus the subject guides that apply: `roblox-data-stores`, `roblox-monetization`, `roblox-input`, `roblox-npc-ai`, `roblox-physics`, `roblox-audio`, `roblox-text-chat`, `roblox-teleport-matchmaking`, `roblox-game-systems`, `roblox-performance`, `roblox-analytics-liveops`, `roblox-open-cloud`, `roblox-tooling` |
| Reviewing code | `roblox-code-review` |
| Reviewing or polishing UI or motion | `emil-design-eng`, `redesign-skill` |
| UI that looks like slop, vibecoded, AI-made or generic | `unslop`, with `web-to-roblox` and `roblox-ui-design` |
| A new game or a new idea | `game-brainstorm`, `game-bible`, `game-design`, `market-research` |
| Starting work on an existing game for the first time | `game-bible` |
| Any design decision, new feature or system | `game-design`; `economy-design` when prices, rewards, drop rates or progression numbers are involved |
| Sounds and music | `sound-design` |
| Judging whether something is fun, clear or ready | `player-testing`, with `roblox-playtest` |
| Big requests that can be split into parallel work | `producer` |
| When spawned as a reviewer | `critic` |
| Maps, towns, lobbies; Blender models; importing models | `lihfolk-worldbuilding`; `roblox-blender-modeling`; `roblox-asset-import` |
| Ability and combat effects; combat and UI sounds | `lihfolk-vfx`; `lihfolk-sfx` |
| A new system end to end; ready-made data, shop, eggs, codes, daily, rebirth, quests, leaderboards | `roblox-system-builder`; `lihfolk-systems` |
| Something broken, errors, desync | `roblox-debugging` |
| Tutorials and onboarding; trend games; scoring work against a quality bar | `lihfolk-tutorial`; `roblox-trend-game`; `quality-score` |
| Limits, prices, policy, 2025-2026 changes, unclear bugs, anything no guide covers | search LihFolk Vault V2 with the `rbx_vault` tool |

The `lihfolk-*` guides, `roblox-system-builder`, `roblox-debugging`, `roblox-asset-import`, `roblox-blender-modeling`,
`roblox-trend-game` and `quality-score` come from LihFolk Vault V2 (CC BY 4.0) and are `rbx_guide` topics.

The `roblox-*` subject guides come from the roblox-skills collection. In
Claude Code they are the `roblox-skills` plugin's skills; through the rbxkit
tool server they are `rbx_guide` topics of the same name.

To pick the style guide, look at the place first (its UI, materials and
colours): raw, industrial, blocky or terminal looks are `brutalist-skill`;
clean and quiet is `minimalist-skill`; soft, premium or glossy is
`soft-skill`. If the place is new or unclear, choose the closest and say
which. The style guides are written for web pages: for a model, take their
palette, proportions, materials and restraint, not their CSS.

## Which tool for which job

| Job | Tool |
|---|---|
| List children, find by name or class | `search_game_tree` (`path`, `keywords`, `instance_type`, `max_depth`) |
| Read properties and attributes | `inspect_instance` |
| What the user has selected ("this part", "here") | `execute_luau` Edit: `game:GetService("Selection"):Get()` |
| Change edit-mode state | `execute_luau` with `datamodel_type: "Edit"` |
| Find code | `script_grep` (contents), `script_search` (script names) |
| Read / edit / create a script | `script_read`, `multi_edit` (`className` when creating) |
| Disable, move or rename a script | `execute_luau` Edit |
| Screenshot the viewport | `screen_capture` |
| Is a playtest running | `get_studio_state` |
| Playtest and simulated input | the `roblox-playtest` skill |
| Output log | `get_console_output` |
| Creator Store / inventory assets | `search_asset`, `insert_asset` |
| Upload a model, image or animation | `open_cloud_invoke` (below) |
| Model, check, render in Blender | `rbx.py blender` |
| Icons of models in the place | `studio/icon_booth.lua` + `rbx.py icon` |
| Look at a video the user gave | `rbx.py video info|frames` |
| Visual effects | the `roblox-vfx` skill |
| AI mesh, material, texture, animation | `generate_mesh`, `generate_material`, `generate_texture`, `generate_animation` |

`execute_luau` returns what the code `return`s; return a string describing the
result. It runs on Studio's main thread, so long loops must `task.wait()` and
big builds go in chunks. `shared` persists between calls in one Studio
session, which is how the capture and icon helpers keep state.

## Ground rules

- Live Studio is the truth. Check paths, hierarchy, the selection and
  properties with the tools before writing code that references them. Never
  ask the user to describe something that can be inspected.
- Read similar code in the project first and match its framework, naming and
  utilities. Search for every other place that needs the same change.
- In a Rojo or file-sync project, edit the local files. Otherwise edit
  scripts in Studio with `multi_edit`.
- Use `execute_luau` in the Edit datamodel only when the request is to change
  the place. For questions and inspection use the query tools.
- Wrap edit-mode changes in a ChangeHistoryService recording so the user can
  undo them in one step (see roblox-building).
- Don't claim runtime behaviour works, or is fixed, from reading code.
  Playtest it, or say it is unverified.
- Playtest when behaviour changed; skip it for comments, text and constants.
  Always stop the playtest afterwards.
- Stay in scope: a UI request changes UI, not the world or the economy.
- Pick reasonable defaults and say what was assumed, rather than stopping to
  ask about size, placement or style.
- Changes made to the place while a playtest is running are thrown away when
  it stops. Stop the playtest before editing.
- Assets inserted from the Creator Store can contain scripts. Inspect what
  came in before leaving it in the place.
- Anything a helper adds to the place for a moment (the icon booth, the VFX
  preview folder) must be removed again before finishing.

## Look at every 3D change

Never report a build, model or map change as done without looking at it.
Capture each changed model close up from a front 3/4 and a back 3/4 view and
fix anything floating, detached, clipped or misshapen.

```lua
local m = workspace.Arena
local cf, size = m:GetBoundingBox()
local r = size.Magnitude * 0.9
local front = cf.Position + Vector3.new(r * 0.7, r * 0.5, r * 0.7)
local back = cf.Position + Vector3.new(-r * 0.7, r * 0.5, -r * 0.7)
return string.format("look=%s front=%s back=%s", tostring(cf.Position), tostring(front), tostring(back))
```

Pass those to `screen_capture` as `look_at_position` and `camera_position`.
With no camera arguments it captures the current view (what a UI check wants).

## 3D models

Things the user wants to exist in the game (props, statues, vehicles,
buildings, creatures, furniture, weapons, trees, anything from a reference)
are real meshes, modelled in Blender and imported. Don't assemble them from
Parts; Parts are for plain geometry (floors, walls, platforms, hitboxes, a
blockout the user asked for). `generate_mesh` is a quick alternative for a
generic prop, and `search_asset` + `insert_asset` only when the user asks for
a free model.

### 1. Find the spot and the size

Locate what they mean with `search_game_tree`, `inspect_instance` and the
selection. Find the surface with a downward `workspace:Raycast`. Pick a
believable size: a character is about 5 studs tall, 1 stud is about 0.28 m.

### 2. Model in Blender

```
python {{RBXKIT}}/rbx.py blender --model Lamp --script lamp.py
```

Each call starts Blender headless, opens `work/models/Lamp/Lamp.blend` (new if
missing), runs the script with `bpy` and the `rbx` helpers, and saves. The
scene persists between calls; Python variables do not. Set `result = {...}`
to get values back.

- 1 Blender unit = 1 stud. **+Z is up and the front faces -Y.** Height goes
  on Z; a model built with its height on Y arrives lying on its side.
- One collection per model, named after it: `rbx.collection("Lamp")`,
  `rbx.add(obj, "Lamp")`.
- Separate objects for parts that move (wheels, doors, limbs), each limb's
  origin at its joint. Sink limbs 0.05-0.1 into the body; overlap every other
  joint by 0.01-0.05 so no seam opens.
- Place parts from their neighbours' real bounds (`rbx.bounds(obj)` returns
  world min and max), computed in code, never eyeballed.
- Thin parts get a Solidify modifier, hard edges a small Bevel.
- Materials: `rbx.color_material(name, (r, g, b), obj)` for a flat colour, or
  `rbx.smart_uv(obj)` then `rbx.pixel_material(name, pixels, w, h, obj)` for
  an image texture built from an RGBA float list. A plain Base Color arrives
  as a solid colour; an image arrives as a texture.
- `rbx.finish("Lamp")` merges doubles and fixes normals.

### 3. Check and look

`rbx.check_model("Lamp")` prints every piece's world bounds, flags floating
pieces with the move that closes each gap, and warns when the model is lying
on its side or not resting on z = 0. Run it before every render and fix what
it lists; a clean check says nothing about shape, so still look.

`rbx.shot("Lamp")` renders the model alone from `iso` and `iso_back` and
returns the PNG paths: read them. `views=` takes any of `iso`, `iso_back`,
`iso_left`, `front`, `back`, `left`, `right`, `top`, `low`; `objects=[...]`
frames a joint close up. Repeat after every edit. A group shot of several
models is not a check.

### 4. Export, upload, insert

`rbx.export_model("Lamp")` writes `work/models/Lamp/export/Lamp.fbx` and a
manifest whose `size_studs` is the size the model should have in Studio.

Upload with `open_cloud_invoke`:

- `method: "POST"`, `endpoint: "/assets/v1/assets"`,
  `scopes: ["asset:read", "asset:write"]`
- `body`: `{"request":{"assetType":"Model","displayName":"Lamp","description":"...","creationContext":{"creator":{"groupId":"<id>"}}}}`
  (or `"userId"`). Upload as whoever owns the place, so the game can use it:
  read `game.CreatorType` and `game.CreatorId` with `execute_luau`.
- `files`: `[{"name":"fileContent","path":"<absolute path to the .fbx>"}]`

It returns an operation; `GET /assets/v1/operations/<id>` until `done` is
true, and take `response.assetId`. Then `insert_asset` with that id.

Files can only be uploaded from folders the user approved in Studio's
Assistant Settings > MCP Permissions. If the call says the folder is not
approved, ask the user to approve it; do not move the file elsewhere.

### 5. Place and verify

Edit the CONFIG at the top of `studio/place_model.lua` (path of the inserted
model, name, `sizeStuds` from the manifest, a position above the spot, yaw)
and run it. It scales the model to the intended height, anchors and welds it,
puts the pivot at the base, sets `DoubleSided` on its MeshParts, rests it on
the surface below, and records one undo step. Then capture it front and back
in Studio. If it lies on its side it was built in the wrong axes: fix it in
Blender and re-export, don't rotate it in Studio.

If an import fails, tell the user the exact error. Don't swap in Parts or a
Toolbox asset on your own; the .blend is saved and the import can be retried.

## Animations

Check whether the game's characters are R6 or R15 first; an animation made
for one does not play on the other. Never create rigs in the game to animate.

Quick route: `generate_animation` makes an R15 animation from a text prompt
(pass the user's wording unchanged).

Hand-keyed route:

1. **Rig.** `rigs/R15.json` is the standard R15 dummy. For any other rig run
   `studio/export_rig.lua` (set `RIG` to its path, or `"R6"`) and save the
   JSON it returns under `work/rigs/`. It reads Motor6Ds and the
   AnimationConstraints newer avatars use.
2. **Animate** with `rbx.py blender --file work/anims/<Name>.blend`:
   `arm = rbx.import_rig(path)` builds an armature with one bone per joint,
   named after the part it moves, plus a box per part. Set
   `scene.frame_start` / `frame_end` (24 fps) and key poses with
   `rbx.pose(arm, "RightUpperArm", rotation=(x, y, z), frame=6)`; degrees,
   about the joint's own Roblox axes (X right, Y up, Z back). Move the body
   with `location=` on `LowerTorso`. For a loop make the last frame match
   the first.
3. **Look.** `rbx.shot("R15", views=("back", "iso_back"), frame=9)`. The
   character faces Blender +Y, so `back` shows its front. Check each key pose
   and fix wrong directions before moving on. With a reference, work one pose
   at a time and compare limb angles, stance and lean side by side.
4. **Bake and build.** `rbx.bake_animation(arm, "Wave", loop=False,
   priority="Action")` writes a JSON; `rbx.py anim <that json>` turns it into
   a `.rbxmx` KeyframeSequence.
5. **Upload** like a model but with `assetType: "Animation"` and the
   `.rbxmx`. Create an `Animation` with `AnimationId = "rbxassetid://<id>"`,
   report its path and id, and write the Animator-loading script if the user
   wants it to play. Loop idles and walks; priority Idle, Movement or Action
   to match.

## Icons and renders

For inventory or shop icons of models that are in the place, render them in
Studio:

1. Once per session, run `studio/icon_booth.lua` through `execute_luau`
   (Edit). It defines `shared.RbxIcon`.
2. Per model: `return shared.RbxIcon.show("Workspace.Sword", { view = "iso", ortho = true })`,
   then `screen_capture` with no camera arguments. The result names the saved
   image file.
3. `python {{RBXKIT}}/rbx.py icon "<that file>" --name Sword --folder icons/Weapons`
   cuts it into a transparent 512 px PNG and writes a preview: read the
   preview.
4. `return shared.RbxIcon.clear()` when the batch is done. Always.

Use one view and one set of options for a whole batch so the icons match.
Options: `view` (`iso`, `iso_right`, `front`, `back`, `left`, `right`, `top`),
`yaw` / `pitch`, `ortho`, `margin`, `ambient`, `light`, `lightDirection`.
Render one first and check it before doing the rest.

For models built in Blender, `rbx.render_icons([collections], out_dir)` does
the same there.

Upload icons with `open_cloud_invoke` (`assetType: "Image"`, the PNG), list
each id for the user (`Sword: rbxassetid://123`), and keep them in one table
or config module.

## Uploads and moderation

Roblox moderates every uploaded image, mesh and animation, and content that
breaks its Community Standards can get the account banned. Look at every
image or render before uploading it. If it shows blood, gore, graphic
violence or sexual content, stop, show the user, say why, and upload only
after an explicit yes in chat. Uploads are permanent: settle the look first,
upload a set once, and tell the user how many assets are going up. New
uploads can show blank for a few minutes during review: say so, don't
re-upload. Never invent an asset id.

## Video references

`rbx.py video info <file>` gives duration, size and fps. `rbx.py video frames
<file>` tiles frames from across the clip into one sheet to read;
`--start 1.2 --end 2.0 --count 12` looks at a stretch frame by frame. A GIF
attachment is one still: ask for an MP4 or separate frames.

## Lighting a new place

When building in a fresh Baseplate place, also set Lighting to suit the theme
and time of day. Leave an established game's lighting alone unless asked.

## When blocked

If a step needs something only the user or Roblox can change (asset
permissions, moderation, a group role, an unapproved upload folder, Studio API
access), stop, give the exact error and the one thing they need to do. Don't
build diagnostics around it. If Studio stops responding, retry a few times,
then ask the user to check Studio.

## Resetting player data

Only when a test needs a fresh player. `studio/reset_data.lua` removes one
DataStore key in edit mode. It deletes real saved data, so confirm the store
and key with the user first.

## Image generation

Textures can be generated with Gemini. With the user's API key it is automatic.
Without one, the tool shows the prompt to the user, who makes the image in
their own Gemini chat, and the call waits for them to paste or copy it; tell
the user a prompt is waiting before you call it. For plain tileable surfaces
on parts, prefer Studio's free `generate_material`.

The command:
`rbx.py gen "<what it shows>" --kind surface|particle|decal --name <Name>`
(the `rbx_generate_texture` tool). `surface` gives a seamless tileable
material, `particle` a white VFX sprite on transparency, `decal` an isolated
mark on transparency. Look at the review image it returns before using or
uploading anything.

That generator is for textures only. Never use it for UI, icons, logos,
typography or any graphic design, and never for an image with text in it.

Pictures that are not textures (the game's icon, thumbnail key art, UI art,
concept art) can be AI-generated with `rbx_image`
(`rbx.py image generate "<what it shows>" --size 512x512 --name <Name>`). It
makes the picture on the user's own Gemini plan, through Antigravity, at
exactly the size you ask for, and saves it in the game's project folder.
`rbx_image` with action `keep` brings in an image that already exists, such as
one a Gemini agent's own generate_image tool made. With `transparent: true`
the subject is cut out onto a transparent background (an item or shop icon):
hard-edged subjects cut out cleanly, soft ones (smoke, glow, fur, glass) less
so. Each generated picture costs some of the user's Gemini plan: make a few
considered ones, never batches.

Captured footage of the real game is still what shows gameplay. Thumbnails: at
least three, at least one a real capture of the game; generated key art shows
only what is really in the game (its characters, its places, its look) and
promises nothing the game does not keep. The game's icon may be fully
generated. Never another game's characters or art, and no lettering in a
generated picture. `rbx_image` uploads nothing: look at each image and name
its file; uploading it follows Uploads and moderation above, like any image.

For UI art you can also use the game's own asset ids, Creator Store images
(`search_asset`, `assetType: "Image"`), icon-booth renders, textures drawn
with `rbx.py tex`, or images the user supplies. Leave labelled slots and
report what is missing rather than faking art.
