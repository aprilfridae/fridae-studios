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
| `rbx_promo` | Vertical promo videos from gameplay clips: probe, reference, voice, render, check (needs the promo engine) |
| `rbx_doctor` | Says what is missing and how to fix it |

Claude Code users also get the guides as skills and a `roblox-tester` agent.

The same tools are a command line: `rbx <command>` (`rbx --help`).

## The promo engine

The promo maker renders with a separate engine that is downloaded on first use, never bundled: a portable
Node 22, a copy of `promo/` with its npm packages (Remotion, transformers.js, kokoro-js), Remotion's headless
browser, and the Whisper and Kokoro models. It lives in `engine/` in this folder when running from source, and
in `Documents/Fridae Studios/engine` in the packaged app (`rbxpromo.engine.engine_dir()`).

- `python3 -m rbxpromo.engine install` (or `rbx promo install`) installs it, about 730 MB. Every finished step is
  written to `engine/.state.json`, so running install again resumes instead of starting over. `rbx promo status`
  says what is missing. In the app, Setup, Connections, Promo engine runs `rbx promo install --json-progress`.
- The renderer is a copy: after editing anything in `promo/`, run `python3 -m rbxpromo.engine install` again to
  refresh `engine/renderer` (it re-runs `npm ci` only when `package-lock.json` changed).
- The packaged app ships `promo/` without `node_modules` (`rbxkit.spec`) so the install can copy it.
- onnxruntime (used by the Whisper and Kokoro scripts) can abort as Node exits, with exit code 134, after the
  work is done. The Python side judges those scripts by what they printed (a JSON line with `"ok"`, or a
  "model ready" line), not by the exit code; expect the 134 when you run `promo/scripts/voice.mjs` or
  `transcribe.mjs` by hand.

Tests: the promo tests need the engine installed, and the ones that read video need OpenCV, so run them with the
virtual environment's Python:

```
.venv/bin/python -m unittest discover tests -v
engine/node/bin/node --experimental-strip-types --test 'promo/src/captions/*.test.ts' 'promo/scripts/*.test.mjs'
```

Without the engine the engine-gated tests are skipped, not failed.

## Folders

- `rbx.py`, `rbx*.py`, `blender/` - the tools.
- `rbxkit_mcp.py` - the MCP server agents connect to.
- `studio/` - Luau that runs in Roblox Studio.
- `skills/`, `agents/` - the guides.
- `rbxpromo/`, `promo/` - the promo maker: the Python side, and the Remotion project the engine renders with.
- `engine/` - the promo engine when running from source (not in git).
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
