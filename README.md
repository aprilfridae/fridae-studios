# Fridae Studios

An AI helper for building Roblox games. You say what you want in plain words;
it works in your open Roblox Studio place and in Blender, and shows you what
it made.

## Download

Pick the download for your computer from the [latest release](../../releases/latest):

| Your computer | Download | Status |
|---|---|---|
| **Windows** 10 or 11 | `Fridae-Studios.zip` | Supported, updates itself |
| **macOS** 12+ on Apple Silicon (M1 or newer) | `Fridae-Studios-macOS.dmg` | New, no self-update yet |
| **Linux** | none | Not available: Roblox Studio has no Linux version |

### Windows

Unzip `Fridae-Studios.zip` anywhere and run `Fridae Studios.exe`. After that it
updates itself.

Windows may warn the first time because the program is not signed: choose
"More info", then "Run anyway".

### macOS

Open `Fridae-Studios-macOS.dmg` and drag **Fridae Studios** into Applications.

The app is not signed with an Apple Developer ID, so the first time you open it
macOS will refuse. Open **System Settings > Privacy & Security**, scroll down,
and click **Open Anyway** next to Fridae Studios. (Or run
`xattr -dr com.apple.quarantine "/Applications/Fridae Studios.app"` once in
Terminal.)

On a Mac:
- Updates are not automatic yet: download the new `.dmg` when a release comes out.
- Texture previews and local module loading in Studio are off, because Studio's
  content folder is inside its signed app.
- Intel Macs are not built yet.

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

`fridae-cli --mcp` (`fridae-cli.exe` on Windows) is a standard MCP tool server, so the same tools work
in Cursor, Codex, Gemini CLI and other agents.

## Promo maker

Ask a chat for a promo of your game and it cuts your gameplay clips into a
vertical 1080x1920 video for TikTok, YouTube Shorts and Instagram Reels: hook
title, word-by-word captions, an optional spoken voice-over, a cover image and
a post caption. It checks its own render (captions clear of the platforms'
buttons, a hook in the first seconds, readable text, sensible loudness) and
shows the finished video in the chat with buttons to save it or show it in its
folder. Nothing is posted or uploaded; your clips are read where they are and
never copied.

- **It downloads its engine on first use** (about 730 MB: a portable Node.js,
  the Remotion renderer with its headless browser, a speech recognition model
  and a voice model). Install it from Setup, Connections, Promo engine, or let
  the agent ask. It goes in `Documents/Fridae Studios/engine`; nothing is
  installed system-wide. An interrupted install continues where it stopped.
- **Licence:** rendering uses [Remotion](https://www.remotion.dev/docs/license),
  which is free for individuals, non-profits and for-profit teams of up to 3
  people; larger companies need a Remotion company licence to use the promo
  maker.
- **Music is your responsibility.** Use your own track or one that is
  free-licensed, or post the video silent and add a sound inside TikTok.
- **Reference TikToks must be saved video files** (the agent can study a saved
  video's shots, words and beats); links are not downloaded.

## Credits

Bundled guides and fonts keep their own licences; see `THIRD_PARTY.md` and
the `licenses` folder inside the app.

## Source

Everything the program is made of is in this repository: the window (Rust, in
`app-rs`), the page it shows (`ui/index.html`), the tool server and tools
(Python, `rbxkit_mcp.py` and the `rbx*.py` files), the agent guides (`skills`,
`guides`, `routing.txt`) and the Studio plugin (`studio/plugin`). See
[DEVELOPING.md](DEVELOPING.md) for how it fits together and how to build it.
