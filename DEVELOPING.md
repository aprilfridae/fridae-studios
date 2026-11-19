# rbxkit

A toolkit that lets an AI agent build Roblox games properly: model in Blender
and check the result, draw VFX textures, build and capture visual effects
frame by frame, render item icons, animate rigs, and run real playtests.

It works with **any agent that supports MCP servers** (Claude, Cursor, Codex,
Gemini CLI, Windsurf and others). Nothing here needs an account or a
subscription, and all of it is readable source.

## Set up (about two minutes)

You need:

1. **Python 3.9+** from python.org (tick "Add python.exe to PATH").
2. **Blender** from blender.org (any recent version).
3. **Roblox Studio**, with its MCP server turned on and connected to your
   agent (Studio > Assistant settings). That is what lets the agent see and
   edit your place; rbxkit adds everything around it.

Then unzip this folder somewhere permanent and double-click **`install.bat`**
(or run `python install.py`). It installs three Python libraries, finds
Blender, and connects rbxkit to the agents it finds on your computer. Restart
your agent afterwards.

For an agent it did not find, add this MCP server by hand (the installer
prints the exact lines for your computer; `python install.py --print` shows
them again):

```json
{ "mcpServers": { "rbxkit": { "command": "python", "args": ["<this folder>/rbxkit_mcp.py"] } } }
```

Check everything at any time with `rbx doctor`.

## The desktop app

The chat window is a Rust program in `app-rs` (a native WebView2 window showing
`ui/index.html`). It runs Claude Code with the rbxkit and Roblox Studio tools
attached and shows the images they return. The tools stay in Python: the window
starts them as `fridae-cli.exe --mcp` and `fridae-cli.exe --rbx`. `build_exe.bat`
(or `python build_app.py`) builds both and puts them in one folder; it needs
Rust and PyInstaller. On a Mac, `python3 build_mac.py` builds
`build-out/Fridae-Studios-macOS.dmg` the same way. The packaged app needs no Python or Rust installed; it keeps
its output in `Documents/Fridae Studios/work`.

## Using it

Just ask your agent for what you want: "model a street lamp and put it by the
spawn", "make a ground-slam effect", "render icons for everything in
ReplicatedStorage.Items", "playtest the shop and tell me what breaks". The
agent reads the built-in guides and uses the tools itself.

## What the agent gets

| Tool | Does |
|---|---|
| `rbx_guide` | The how-to guides: workflow, playtest, ui-design, sim-ui, vfx |
| `rbx_blender` | Runs Python in headless Blender: model, check for floating parts, render, export, animate |
| `rbx_texture` | Draws particle textures and flipbook sheets, checks them, previews them in Studio for free |
| `rbx_check_images` | Checks any texture or flipbook sheet from its pixels |
| `rbx_icon` | Cuts a transparent icon out of an icon-booth screenshot |
| `rbx_video` | Reads a reference video as a sheet of frames |
| `rbx_animation` | Turns a Blender animation into a Roblox animation file |
| `rbx_vfx_package` | Installs the VFX modules into a place without uploading |
| `rbx_studio_snippet` | Ready-made Luau for Studio: VFX player, lightning, capture, icon booth, model placement, rig export |
| `rbx_doctor` | Says what is missing and how to fix it |

Claude Code users also get the guides as skills and a `roblox-tester` agent.

The same tools are a command line: `rbx <command>` (`rbx --help`).

## Folders

- `rbx.py`, `rbx*.py`, `blender/` - the tools.
- `rbxkit_mcp.py` - the MCP server agents connect to.
- `studio/` - Luau that runs in Roblox Studio.
- `skills/`, `agents/` - the guides.
- `work/` - everything the tools produce (models, textures, icons).

## Good to know

- Uploading models, images and animations to Roblox goes through Studio's own
  Open Cloud access. Studio only uploads from folders you approve in
  Assistant Settings > MCP Permissions: approve this folder's `work` folder.
- Roblox moderates every upload. The guides tell the agent to look at each
  image first and to ask you before uploading anything risky.
- Textures (tileable materials, particle sprites, decals) can be generated
  with Google's Gemini if you add your own Gemini API key in Setup. It is
  deliberately not used for UI, icons, logos or lettering.
- Texture previews and local module loading copy small files into your
  Studio install's `content/rbxkit` folder. Studio updates wipe that folder;
  nothing depends on it lasting.

## Credits

The simulator UI look follows Check-UI Clean Roblox by diggofun /
TemporaryLooker22 (MIT, see `skills/roblox-sim-ui/LICENSE.md`).
