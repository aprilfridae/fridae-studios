# TikTok promo maker: what already exists on GitHub

Research notes, 2026-11-20. Goal: an open-source promo maker inside Fridae Studios that turns a Roblox game
into vertical TikTok / Shorts / Reels videos (styles, voiceover, lyric-synced captions, "in the style of a
TikTok you like"), the way RoAutomation's paid "TikTok promo maker" claims to.

Star counts and licences as of today. Nobody has built this for *games*; the pieces exist separately.

## The closest matches

### OpenMotion — `nexu-io/openmotion` (Apache-2.0, ~14k stars)
"Recreate a reference video with your own brand kit." The closest thing to "in the style of a TikTok you like".
- The agent (Claude Code, Codex, ...) breaks the reference down into shots and transitions, rebuilds them as
  HTML, generates missing assets, then renders MP4.
- **Motion.md**: a Markdown file describing shot order, timing and transitions. A shareable "recipe" for a video.
  Good idea to borrow: a reference TikTok becomes a Motion.md-style recipe the agent fills with game footage.
- Bring your own agent and keys; paid model providers (image, video, Suno music, ElevenLabs voice) are optional.
- Renders with HyperFrames (below).

### Subtle — `0xpratzyy/subtle` (MIT, small but polished)
Animated captions, 35 styles, runs locally, a Claude Code skill plus a CLI. The best reference for caption
*styles* and the rules that make captions look good.
- Whisper (`mlx-whisper`, large-v3-turbo) for word-level timing; transcript cached by file hash.
- Writes spoken numbers as one token ("five thousand dollars" → `$5,000`), highlights one word per page
  (numbers > names > punchline), never splits an amount across pages.
- Moves captions off faces and hands, never lets a page straddle a hard cut (ffmpeg scene detection), adds a
  halo or scrim only where the footage would hide the text, picks an accent colour from the footage.
- A style is one JSON file plus one small component; "recipe" styles need no code.
- Checks its own output (safe area, contrast) and makes a contact sheet.
- Renders with **Remotion** (see the licence warning below). Apple Silicon only for transcription.

### Short Video Maker — `gyoridavid/short-video-maker` (MIT, ~1.5k stars)
Text in, short video out, exposed as an **MCP server** and REST API — same shape as Fridae's tools.
- Pipeline: Kokoro TTS (free, local) → Whisper captions → Pexels stock footage → Remotion → MP4.
- A video is a list of **scenes**, each = narration text + search terms. Simple, good data model.
- English voice only (Kokoro); 3–4 GB RAM.

### /brag — `latent-spaces/brag` (MIT, ~22k stars)
"You built it. Now brag." A Claude Code skill that turns a project into a 20-second launch video with music,
motion and share copy. Closest in *spirit* (promote the thing you made). Has a lean variant that needs no
renderer, built for the current Claude models.

### HyperFrames — `heygen-com/hyperframes` (Apache-2.0, ~87k stars)
Write HTML/CSS/animations, render a deterministic MP4. Built for agents; ships Claude Code skills; desktop app
for macOS, Windows and Linux. **The renderer to use** (see licences).

## Long video → clips (useful if we cut from recorded playtests)
- `zhouxiaoka/autoclip` (MIT, ~12k): desktop + CLI + MCP. One link → vertical clip, cover image and post caption
  per platform (TikTok, Reels, Shorts...). Speaker framing, caption paging. Cloud models pay-per-use.
- `Anil-matcha/AI-Youtube-Shorts-Generator` (MIT, ~6k), `ClipsAI/clipsai` (MIT), `modelscope/FunClip` (MIT):
  transcribe → LLM picks the best moments → crop to 9:16 → captions.
- `WyattBlue/auto-editor` (Unlicense, ~6k): cuts dead air / silence automatically. Handy for trimming playtests.
- Gameplay-specific highlight detection exists only as tiny projects (`mahdi-alkak-1/HighlightIQ`, MIT).

## Topic → whole video (faceless channels)
- `harry0703/MoneyPrinterTurbo` (MIT, ~145k): topic → LLM script → search terms → stock clips → TTS → subtitles
  → background music → MP4. Web UI + API. Huge, but built around stock footage and paid model sponsors.
- `RayVentura/ShortGPT` (MIT), `FujiwaraChoki/MoneyPrinter` (MIT), `SamurAIGPT/AI-Faceless-Video-Generator` (MIT).
- `jutsuhq/content-factory` (MIT): local faceless video generator for Apple Silicon Macs.
Less relevant: we have real game footage, so stock footage is the wrong ingredient.

## Agent skills for motion and captions
- `iart-ai/tiktok-video-skills` (MIT): caption-animation, countdown-video, lower-thirds, short-form-video.
- `iart-ai/kinetic-typography-skills` (MIT): word/line staggers, lyric and title cards synced to the beat.
- `ShadoW-Shinigami/lyric-video-director` (MIT): typography-led lyric videos with exact timing.
- `zhuyansen/awesome-claude-video-skills`: a list of 180+ video skills for coding agents.

## Licences: what this means for a free, open Fridae
- **Remotion is not open source.** Free for individuals, non-profits and companies of up to 3 people; bigger
  companies need a paid licence. Subtle and Short Video Maker depend on it. Avoid it in Fridae.
- **HyperFrames (Apache-2.0) and ffmpeg (LGPL/GPL)** are genuinely free. Use these.
- MIT projects (Subtle, Short Video Maker, autoclip, /brag, ClipsAI, the iart skills) can be read and adapted
  freely with attribution. Fridae already credits third parties in `THIRD_PARTY.md`.
- Free local voices: **Kokoro** (Apache-2.0, English, best quality) and **Piper** (MIT, many languages).
  Free local timing: **Whisper** (MIT) via whisper.cpp (any OS) or mlx-whisper (Apple Silicon).
- **Music**: no project solves licensing. All either use the user's file, stock "no copyright" tracks, or paid
  Suno. Our plan: user's own audio or free-licensed tracks, or post silent and add the sound inside TikTok.

## Ideas worth taking
1. A **recipe file** for a video (OpenMotion's Motion.md): shots, timing, transitions, caption style. A reference
   TikTok becomes a recipe; a game's footage fills it.
2. **Scenes** as the unit (Short Video Maker): each scene = a clip + caption text + optional voice line.
3. **Caption rules** from Subtle: one highlight per page, numbers as one token, avoid the subject, respect cuts,
   add legibility only where needed, check the result and show a contact sheet.
4. **Self-check before showing** (Subtle, and Fridae already does this for models and VFX): safe areas for
   TikTok's buttons and caption bar, contrast, length, first-second hook.
5. **Per-platform output** (autoclip): the video plus a cover image and post caption with hashtags.
6. Everything runs **locally and free** by default; paid AI video/voice/music models only as opt-in with the
   user's own key (same rule Fridae uses for Gemini textures).

## What nobody has
Capturing the footage *from the game itself*. Every project starts from an existing video, stock clips or a
website. Fridae can already drive Studio playtests and capture frames, which is the unique part.
