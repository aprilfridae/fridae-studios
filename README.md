# Fridae Studios

An AI helper for building Roblox games. You say what you want in plain words;
it works in your open Roblox Studio place and in Blender, and shows you what
it made.

## Download

Get **Fridae-Studios.zip** from the [latest release](../../releases/latest),
unzip it anywhere, and run `Fridae Studios.exe`. After that it updates itself.

Windows may warn the first time because the program is not signed: choose
"More info", then "Run anyway".

## You need

1. **Claude Code**, signed in with your own account. Fridae Studios has no AI
   of its own; it drives Claude Code for you.
2. **Blender** (any recent version).
3. **Roblox Studio** with its MCP server turned on (Assistant settings).

The dots at the top of the window show whether each one is found, and Setup
explains anything missing.

## What it does

- Models props in Blender, checks them for gaps and floating parts, and
  renders them before calling them done.
- Builds visual effects and captures them frame by frame in slow motion.
- Draws particle textures, and can generate textures with Gemini if you add
  your own key.
- Renders item icons, animates rigs, and runs real playtests with simulated
  clicks and key presses.
- Runs several chats at once, saves every chat, and can check its own work in
  rounds with the Self-test switch.

`fridae-cli.exe --mcp` is a standard MCP tool server, so the same tools work
in Cursor, Codex, Gemini CLI and other agents.

## Credits

Bundled guides and fonts keep their own licences; see `THIRD_PARTY.md` and
the `licenses` folder inside the app.

## Source

Everything the program is made of is in this repository: the window (Rust, in
`app-rs`), the page it shows (`ui/index.html`), the tool server and tools
(Python, `rbxkit_mcp.py` and the `rbx*.py` files), the agent guides (`skills`,
`guides`, `routing.txt`) and the Studio plugin (`studio/plugin`). See
[DEVELOPING.md](DEVELOPING.md) for how it fits together and how to build it.
