# Promo maker: design

Status: approved by the user, 2026-11-20, 2026-11-20. Research behind it: `docs/research/2026-11-20-tiktok-promo-prior-art.md`.

## Intent

Fridae Studios is a free, open-source AI helper for building Roblox games, used first by a three-person team
making their own games and given away to everyone. The team wants what RoAutomation sells as its paid "TikTok
promo maker": turn a game into short vertical promo videos for TikTok, YouTube Shorts and Instagram Reels, and
post them. In Fridae it is free, runs on the user's computer, and is driven by the same chat agent (Claude Code)
that builds the game.

Success: a team member drops in a few minutes of their own gameplay recordings, says "make a 20-second hype
promo with this song", and gets back a finished vertical video, a cover image and a post caption, which they can
download, open in their folder, or post with one click.

## Goals

1. Cut promos from **gameplay the user recorded**: mainly with Fridae's own **Record** button (goal 10), or any
   clip they add (OBS, phone capture). Output 9:16, 1080×1920, 30 or 60 fps.
2. **Styles**: at least 8 named presets, each a combination of edit rhythm, transitions, caption style and
   title/end cards (e.g. hype, cinematic trailer, meme, top-5 countdown, update log, lyric edit, calm showcase,
   "POV" story). Exact list is the builder's call.
3. **"Like this TikTok"**: the user drops a reference video; the agent copies its structure (shot lengths,
   pacing, where text appears, caption look) and fills it with game footage.
4. **Captions** animated in sync with the voice or the song's lyrics (word-level timing), with the caption rules
   from Subtle: one highlighted word per page, numbers as one token, no page across a hard cut, legibility halo
   only where needed, kept out of platform UI areas.
5. **Voiceover** from a free local voice; **lyric edits** timed from the user's own audio track.
6. Output per promo: `promo.mp4`, `cover.jpg`, `caption.txt` (post text + hashtags), and `sheet.jpg` (contact
   sheet the agent uses to check its own work).
7. In the chat, a finished promo shows as a card with a **player**, **Download**, **Show in folder**, and a
   **Post** button per platform (TikTok, YouTube, Instagram).
8. **Connected accounts** in Setup via each platform's official OAuth sign-in, using the team's own developer
   apps. Posting works now in whatever mode each platform allows an unreviewed app (private / draft) and fully
   once that team's developer apps are approved, with no code change.
10. **Record in Studio**: a Record button in Fridae's Studio plugin toolbar (and one in the app) records **only
    Studio's game view** (not Explorer/Properties/other panels) at 30 or 60 fps, **with Studio's own sound and
    nothing else** (no Discord, music players, system sounds, microphone). Stop saves it to the gameplay library.
11. **Gameplay library**: every recording lands in `Documents/Fridae Studios/gameplay/<YYYY-MM-DD>/`, with the
    game's name, length and a thumbnail. A **Gameplay** tab lists all recordings grouped by date (Today, Yesterday,
    dates), with play, rename, delete, show in folder, and multi-select → **Make promo**. The agent can list and
    search the library, so "make a video with yesterday's Pet Tycoon gameplay" works in chat.
9. Works on **Windows and macOS** (the team runs Windows). Linux is not supported by Fridae at all.

## Non-goals (for this project)

- The agent *playing* the game to film it. The user plays; Fridae records. (Possible later.)
- A timeline editor. Changes are asked for in the chat. Revisit if that proves clumsy.
- AI-generated video, images or music. (May come later as opt-in with the user's own key, like Gemini textures.)
- Bundled or downloaded music. The user brings audio, uses free-licensed tracks, or posts silent and adds a
  sound inside TikTok.
- The Lead + specialist agent team. Separate project, next. The promo guide written here becomes the "promo"
  specialist's instructions later.
- Scheduling posts, analytics after posting.

## Anti-goals (would make it a failure even if it works)

- Posting anything publicly without the user's click, unless they turned on "post automatically" in Setup.
- Any posting method that breaks platform rules: no password logins, no browser automation clicking through
  websites, no unofficial APIs. These get accounts banned.
- Paid third-party posting services or anything that makes the feature cost money.
- Captions, titles or the hook hidden under TikTok/Reels/Shorts buttons and caption bars.
- Uploading the user's footage anywhere except the platform they chose to post to.

## Constraints

- **Licence:** rendering uses **Remotion**. Remotion is source-available, not open source: free for individuals,
  non-profits and for-profit teams of up to 3 people; larger companies need a Remotion company licence. Decided
  by the user (their team is exactly 3, not a company). The README must state this plainly next to the feature.
  Everything else stays MIT/Apache/LGPL. Code adapted from Subtle (MIT) and ideas from OpenMotion (Apache-2.0)
  and Short Video Maker (MIT) are credited in `THIRD_PARTY.md`.
- **Packaging:** the packaged app ships no Node.js today. Remotion needs Node and a headless Chrome, so the
  "promo engine" (portable Node, Remotion packages, Chrome Headless Shell, ffmpeg, Whisper model, Kokoro
  voice model) downloads on first use into Fridae's own data folder, with a progress display and a size shown
  before download (expect several hundred MB). Nothing system-wide is installed. Setup's checks show its state.
- **Large files:** chat attachments are base64 over HTTP, capped at 30 MB and copied into `work/attachments`.
  Gameplay clips are bigger. Clips are added with a native file picker (`rfd`, already a dependency) or
  drag-drop that yields file paths; Fridae references them in place and never copies them.
- **Platforms** (verify current rules while building; they change):
  - TikTok Content Posting API: OAuth (desktop/PKCE). Until TikTok audits the app, posts are limited to private
    visibility. Direct Post vs upload-to-drafts both exist; use whichever an unaudited app may use.
  - YouTube Data API `videos.insert`: OAuth installed-app flow. Uploads from unverified projects are forced
    private until Google's audit.
  - Instagram Reels via the Graph API: needs a Business/Creator account linked to a Facebook Page, Meta app
    review, and the video at a **public URL** when publishing.
  - **Each team registers its own developer apps** (user's decision). Fridae ships no app secrets and runs no
    server. Setup → Connected accounts asks for the team's own client ID and secret per platform (TikTok, Meta),
    stores them in the OS keychain, and links to a step-by-step guide (`docs/posting-setup.md`, written in this
    build) for registering each developer app, its redirect URI, scopes, and the approval forms. YouTube uses the
    same fields (Google allows a desktop client's secret in the app, but per-team keeps one consistent flow and
    each team's own quota). Approval paperwork stays with each team.
- **Why recording is done by the app, not Studio:** neither Studio's MCP server nor the plugin API can record
  video. Studio's `screen_capture` returns one still per call (about 1–3 a second), and Fridae's slow-motion VFX
  capture works only by freezing effect time between stills, which live gameplay can't do. So the desktop app
  records the screen region of the game view and Studio's audio at the OS level.
- **Recording on each OS:**
  - Windows 10 2004+ / 11: video via Windows.Graphics.Capture or Desktop Duplication (or ffmpeg `ddagrab`),
    cropped to the game view; audio via WASAPI **process loopback** for the Roblox Studio process only. The audio
    part is native Rust (ffmpeg cannot do per-process audio). No permission prompt.
  - macOS 13+: ScreenCaptureKit, filtered to the Roblox Studio app, for both video and that app's audio. macOS asks
    once for Screen Recording permission. Recording needs macOS 13+ (app audio); the rest of Fridae still runs
    on 12.
  - Encoded to H.264/AAC MP4 with the engine's ffmpeg.
- **Finding the game view:** the plugin tells Fridae when recording starts and the game view's size; Fridae finds
  Studio's window and locates the view inside it (e.g. by matching a `screen_capture` still, or brief corner
  markers drawn by the plugin; method is the builder's call). Re-detected at every recording start, so moving or
  resizing Studio is fine. Recording keeps going if the view moves mid-recording only if cheap; otherwise the user
  is told to keep Studio still while recording.
- **Credentials:** OAuth tokens are stored in the OS keychain (Windows Credential Manager / macOS Keychain),
  not in `config.json`. Fridae never sees passwords; sign-in happens in the user's browser.
- The agent never posts on its own initiative: posting is an app action triggered by the user's click (or the
  auto-post setting), not an MCP tool the agent can call.

## Approaches considered

- **A. A promo tool the agent drives in chat, plus a result card** *(chosen)*. New Python tool behind the MCP
  server like Blender/VFX; a Remotion project renders a JSON recipe; the agent plans, renders, self-checks and
  revises through conversation. Fits Fridae's existing pattern, fastest to good results.
- **B. A plus a timeline tab.** More control, but means building a video editor first. Deferred.
- **C. Install OpenMotion and Subtle side by side and call them.** Least code, but two extra installs with their
  own setup, built for talking-head and brand videos, not gameplay; breaks Fridae's one-download setup. Rejected.

Renderer: Remotion (user's decision) over HyperFrames (fully open, but caption styles would be rebuilt from
scratch). Remotion lets us adapt Subtle's caption styles directly.

## Design

### The recipe (`promo.json`)

The unit of work, written by the agent, rendered by the engine, kept beside the output so any promo can be
re-rendered after a tweak. Inspired by OpenMotion's Motion.md and Short Video Maker's scenes.

```json
{
  "version": 1,
  "size": [1080, 1920], "fps": 30,
  "style": "hype",
  "audio": {"music": "C:/…/song.mp3", "music_start": 12.4, "voice": "voice.wav", "duck_music_under_voice": true},
  "scenes": [
    {"clip": "C:/…/run1.mp4", "in": 31.2, "out": 33.0, "crop": {"x": 0.55, "track": "auto"},
     "speed": 1.0, "zoom": "punch@0.4", "transition": "whip"},
    {"title": "THIS PET IS INSANE", "duration": 1.2, "style": "impact"}
  ],
  "captions": {"source": "voice" , "style": "punch", "highlight": "auto"},
  "end_card": {"title": "Pet Tycoon", "cta": "Play now"},
  "post": {"caption": "…", "hashtags": ["roblox", "robloxgames"]}
}
```

Field set is indicative; the builder finalises the schema and validates it before rendering, with error
messages the agent can act on.

### Engine (Python, `rbxpromo.py` + `promo/` Remotion project)

CLI under `rbx promo …`, exposed to the agent as one MCP tool `rbx_promo` with actions:

| Action | Does | Returns |
|---|---|---|
| `probe` | Reads clips: length, fps, size, audio; hard cuts (ffmpeg scene detection); loud/action moments; a frame sheet per clip | JSON + sheet images |
| `reference` | Breaks a reference TikTok into shot list, timings, on-screen text positions, caption look, music beats | JSON skeleton recipe + sheet |
| `voice` | Script text → speech (Kokoro, local) | `voice.wav` + word timings |
| `transcribe` | Word-level timing for a voice or a song (Whisper via transformers.js) | JSON words |
| `render` | Validates `promo.json`, renders via Remotion, encodes H.264/AAC | `promo.mp4`, `cover.jpg`, `caption.txt`, `sheet.jpg` |
| `check` | Safe-area overlap for each platform's UI, text contrast, hook within the first second, length limits, loudness | pass/fail list |
| `styles` | Lists styles with a still for each | text + images |

`check` runs automatically after `render`; the agent fixes failures before showing the user, the same way
Fridae already checks models and VFX before calling them done.

The Remotion project (TypeScript) contains one composition driven by the recipe: clip scenes with 9:16
cropping (static focus point or auto-tracking of motion), speed ramps, zoom punches, transitions, title cards,
the caption layer (styles adapted from Subtle), end card, and audio mix with ducking. Styles are JSON presets
plus small components, so adding one does not touch the engine.

### Agent guide (`skills/roblox-promo/SKILL.md`)

How to make a promo that works for a Roblox game: hook in the first second, 15–30 s length, show the core loop
and the best moment early, one idea per promo, caption rules, safe areas, picking moments from `probe`, using a
reference with `reference`, writing the post caption and hashtags, music rules (user's audio only; or silent for
adding a TikTok sound). Also: always render, check and look at the sheet before presenting; ask before
voiceover lines that make claims about the game.

### App changes (Rust + `ui/index.html`)

- **Adding clips:** an "Add clips" button and drag-drop for video files; both pass file paths to the chat as
  references (no 30 MB limit, no copy). Same for a reference video and a music file.
- **Promo card:** when a render finishes, the chat shows a card: video player (served from the work folder over
  the existing token-protected local server, with range requests so it can seek), style name, length, the post
  caption (editable), and buttons:
  - **Download**: native save dialog, copies the MP4 (and optionally cover + caption) where the user picks.
  - **Show in folder**: reveals the file (macOS `open -R`, Windows `explorer /select,`).
  - **Post to TikTok / YouTube / Instagram**: one button each. States: *Connect first* (opens Setup),
    *Will post as private until approved*, *Posting…* with progress, *Posted* with a link, *Failed* with the
    platform's reason. A confirm step shows exactly what will be posted where.
- **Setup → Connected accounts:** connect/disconnect per platform via browser OAuth with a local loopback
  redirect; shows the account name and whether the app is in limited (private) or full mode. Optional fields
  for the user's own client ID/secret. A **"Post automatically when a promo is done"** switch, off by default,
  with chosen platforms.
- **Setup checks:** promo engine installed / not installed (with Install button and size), each account's state.

### Recorder and gameplay library

- **Plugin:** a Record / Stop toggle on the existing Fridae Studios toolbar, with a red indicator while recording.
  It sends start/stop to the app over the plugin's existing local connection (token-protected), with the place
  name and whether a playtest is running. Recording works in edit mode and playtests; the guide recommends
  recording during a playtest.
- **App:** the recorder (above) plus a Record button in the window; a small "Recording 0:42" indicator; a
  maximum length setting (default 10 minutes) so a forgotten recording doesn't fill the disk.
- **Library files:** `gameplay/<YYYY-MM-DD>/<HHMMSS>-<place>.mp4` plus `<same>.json` (place name, place id if
  known, duration, resolution, fps, recorded-at, user title) and `<same>.jpg` thumbnail. The folder is plain
  files, so clips dropped in by hand show up too.
- **Gameplay tab:** grouped by date, newest first; each row shows thumbnail, title (editable), game, length;
  actions: play, rename, delete (to the OS trash), show in folder, select several → Make promo (starts a chat
  with those clips attached as references).
- **Agent access:** `rbx_promo` gains a `library` action (list/search by date, game, title) so the agent can pick
  clips from a description.

### Where files go

`<work>/promos/<project or chat>/<timestamp>-<slug>/` holding `promo.json`, `promo.mp4`, `cover.jpg`,
`caption.txt`, `sheet.jpg`, and any `voice.wav`. Engine downloads go in Fridae's data folder (`Documents/Fridae
Studios/engine/` packaged, or the work folder from source), not the work folder.

### README

Add a "Promo maker" section: what it does, that it downloads its engine on first use, the Remotion licence
terms above, that posting is private until each platform approves Fridae, and that music is the user's
responsibility.

## Decided by the user

- Promo maker comes before the Lead + specialist team. (Assumed from the user's priority; confirm.)
- User records footage; the agent does not film.
- A Record button in Studio (plugin) that records only the game view, with game sound only, saving to a gameplay
  library listed by date, which the agent can use by description.
- Remotion is the renderer, with the licence note in the README.
- Post buttons for TikTok, YouTube and Instagram, plus Download and Show in folder; connect accounts now so
  posting works once approved.
- Free and open: no paid services, user's own Claude plan.
- Each team registers its own TikTok / Meta / Google developer apps and pastes the keys into Setup; Fridae ships
  no secrets and runs no server.

## Calls made by me (correct any)

- Approach A (chat-driven tool + result card), timeline editor deferred.
- Posting needs a click unless "post automatically" is switched on (off by default).
- Kokoro for voice (English only); Piper as a fallback for other languages is left to the builder.
- Whisper run with @huggingface/transformers in the engine's Node (works on Windows and Mac) rather than Subtle's
  Apple-only mlx-whisper. (Planned as whisper.cpp; replaced during the build.)
- Tokens in the OS keychain rather than `config.json`.
- Engine downloaded on first use rather than bundled, to keep the main download small.
- **Auto-post trigger:** only a render the agent marks `final` that also passes `check`; never during overnight
  mode, Self-test or any run with nobody watching (those leave the card waiting for a click). Drafts never post.
- **Reference videos are files only.** No TikTok/YouTube link downloading (yt-dlp and similar use unofficial
  endpoints against platform terms). The guide and error text tell users to save the video with the platform's
  own Save button or a screen recording.
- **Definition of done for posting:** code complete, and posting verified end to end (private posts) against
  developer apps the team registers during the build. Public posting is unlocked later by platform approval,
  with no code change.

## Left to the builder

- Final recipe schema, style list and names, transition set.
- How "action moments" are detected in `probe` (motion, audio loudness, scene cuts; any mix).
- Auto-tracking crop method.
- Whisper model size and Kokoro voice choice.
- Exact Remotion and Node versions, and how the engine download is verified (checksums).
- Instagram's public-URL requirement: pick a free, privacy-respecting way or ship Instagram as "coming soon"
  behind the button until solved. TikTok and YouTube must work.
