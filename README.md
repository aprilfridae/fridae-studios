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

The agents look things up instead of reading whole manuals: one search covers the bundled Roblox
notes, Fridae's own how-to guides, the Studio team's notes and your game's own notes (GAME.md and the lessons a run
keeps), and the agent reads only the part it needs. The Roblox notes are a trimmed copy of
[LihFolk Vault V2](https://github.com/lihfolk-cy/v2-vault) (CC BY 4.0). The rules every agent must follow are always given in full.

`fridae-cli --mcp` (`fridae-cli.exe` on Windows) is a standard MCP tool server, so the same tools work
in Cursor, Codex, Gemini CLI and other agents.

## Studio mode

Studio mode lets a whole team of AI helpers work on one game while you get on
with other things. You talk to one **Lead**. It moves the game through six
stages (idea, design, build, test, launch, promote) and hands each part to an
expert with a plain job name: Scout, Designer, Coder, UI builder, World
builder, Effects and sound, Playtester, Device tester, Security reviewer,
Economy and shop, Icon and thumbnails, Promo maker. You sign off on what they
make now and then, and the run keeps going.

**Before the first run.**
- Sign in to the AI first: run `claude auth login` in a terminal once. Setup
  can show Claude Code as ready while it is signed out, and a signed-out run
  stops after three failed turns.
- Keep Roblox Studio open on the game, with the Fridae plugin connected (the
  Studio plugin dot at the top), for as long as the run builds.
- Update the plugin to 2.8.0: in Setup, press **Update** next to the Studio
  plugin, then restart Studio. An older plugin now blocks the Studio part of undo, for Overnight's
  Undo the night as well as for Undo this run (the files can still go back).

**Starting a run.** Open a chat for a game (a chat needs a game folder; in a
chat without one a message sent with Studio chosen goes as Ask me), pick
**Studio** in the menu under the message box, and send your idea in one line,
for example "A small obby with three checkpoints and a coin shop". Or press
**Start a studio run** on one of the Research tab's ideas. Whatever is chosen
in the menu, what you send in the Lead's own chat goes to the run.

**When it stops for you.** By default only twice: to approve the **design
plan**, and at **ready to publish**, which is a checklist of what you do
yourself (publish, make it public, set prices live). Fridae never publishes,
spends money or Robux, or deletes your work. You can change where it stops in
the Review tab's Run settings.

**The Review tab.** Everything else the experts finish lands in **To look
at**, with its pictures or clips, while the run carries on. What does stop the
run is under **Needs your answer**. On each item press **Approve**, or
**Change** and write a short note: a fresh expert of the same kind redoes the
work with your note, and the new version replaces the old one in Review.
**Approve all** signs off everything in To look at at once.

**Pause and Resume.** Pause stops the Lead handing out new work; experts
already working finish. While a run is paused, your messages to the Lead wait
until you press Resume. If you quit the app, the run comes back as Paused.

**End this run.** In Review's Run settings, End this run stops the run for
good: the Lead starts nothing new, and the game's files, review items and
checkpoints stay. You can then start a new run for the game.

**Undo this run.** The run takes a checkpoint when it starts and after each
finished stage. Undo this run (in Review's Run settings, or at the top of the
Lead's chat) goes back to the checkpoint you pick and leaves the run paused.
- Your project's files can always go back, as long as the game's folder is a
  git repository (a new game folder made in the app is one; git must be
  installed). What they were just before the undo is saved in the game's git
  history first, so nothing is lost.
- Scripts in Roblox Studio can go back only if Studio has stayed open since
  that checkpoint and the Fridae Studios plugin is 2.8.0 or newer. The copies
  live in Studio's memory, and it keeps the newest 24. Before putting them
  back, the scripts as they are now are saved under `.fridae/before-undo/` in
  the game's folder.
- If Studio is closed or was restarted since then, undo offers to put back the
  files only.

**It runs on your own AI plan**, and a team uses more of it than one chat.
In Run settings, **Experts at once** is 2 by default (1 to 4); set it to 1 to
use less, or Pause the run. Only one expert changes the game in Studio at a
time, and at most eight chats and helpers are open at once in the whole app
(an expert that has finished stays open to read but no longer counts).

**Experts ask before running programs.** Experts are not in Full auto: before
one runs a shell program you have not allowed, it asks, and the question waits
in its chat (allow the program there, or in Setup).

**Make the team your own.** The team list and each expert's instructions are
plain files in the app's `team` folder: `team/roster.json` and
`team/experts/<name>.md`. To change them for one game, put a file with the
same name in that game's `.fridae/team/` folder (for example
`.fridae/team/experts/designer.md`).

**It learns.** At the end of each stage the Lead writes down one short lesson:
for this game in `.fridae/lessons.md`, or for every game in
`studio-lessons.md` in your work folder. It reads them at the start of every
run. Edit or delete lines whenever you like.

Review pictures and clips stay out of the game's git history (the run writes
`.fridae/.gitignore`), so the repository stays small enough to push.

Not included yet: live player numbers after launch, a phone remote, and a
built-in asset library.

## Promo maker

The Promos tab cuts your gameplay clips into a vertical 1080x1920 video for
TikTok, YouTube Shorts and Instagram Reels: hook title, word-by-word captions,
an optional spoken voice-over, a cover image and a post caption. Fill in the
New promo form (clips, optional music, an optional saved TikTok to copy the
pacing of, style, length, voice-over script, what it should show, project) and
press Make promo; an agent makes it in a chat of its own (you can also just ask
any chat for a promo). With no music the game sound stays; tick "Mute game
sound" to add a TikTok sound later. The agent checks its own render (captions
clear of the platforms' buttons, a hook in the first seconds, readable text,
sensible loudness), and the finished video lands in the tab's library with its
check result, the post caption, Download and Show in folder. Nothing is posted
or uploaded; your clips are read where they are and never copied.

- **It downloads its engine on first use** (about 730 MB: a portable Node.js,
  the Remotion renderer with its headless browser, a speech recognition model
  and a voice model). Install it from Setup (Connections, Promo engine) or from
  the Promos tab. It goes in your own app data folder (macOS:
  `~/Library/Application Support/Fridae Studios/engine`; Windows:
  `%LOCALAPPDATA%\Fridae Studios\engine`), not in Documents, so iCloud or
  OneDrive do not sync it; an engine an earlier version put in
  `Documents/Fridae Studios/engine` is moved there. Nothing is installed
  system-wide. An interrupted install continues where it stopped, and only one
  install runs at a time.
- **Licence:** rendering uses [Remotion](https://www.remotion.dev/docs/license),
  which is free for individuals, non-profits and for-profit teams of up to 3
  people; larger companies need a Remotion company licence to use the promo
  maker.
- **Music is your responsibility.** Use your own track or one that is
  free-licensed, or leave music out (and mute the game sound) and add a sound
  inside TikTok.
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
