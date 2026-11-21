---
name: roblox-promo
description: Making a vertical promo video (TikTok, YouTube Shorts, Instagram Reels) for a Roblox game from the user's gameplay clips - what makes one work, picking moments with probe, writing the promo.json recipe, rendering with rbx_promo, checking the result, the post caption and hashtags, music and voiceover rules. Use for any promo, trailer, TikTok, Short, Reel or "make a video of my game" request.
---

# Roblox promo videos

You make a vertical video (1080x1920, H.264 and AAC, `.mp4`) from clips the
user has recorded, with the `rbx_promo` tool (CLI: `rbx promo ...`). You plan
it, render it, check it, look at it, fix it, and only then show it. Nothing is
posted or uploaded: the user downloads the file or uses the app's post
buttons.

## What makes a promo work

- **Hook in the first second.** The first frame must already be the best
  thing in the clip or a bold line of text. No logo, no fade in, no
  "hey guys". People swipe away in about a second.
- **15 to 30 seconds.** Under 15 is fine when the idea is simple. Never
  more than 60 (Shorts limit); `check` notes it when you go over.
- **Show the core loop and the best moment early.** The viewer should see
  what the player does and why it feels good within the first five seconds.
  Put the biggest moment in the first third, not at the end.
- **One idea per promo.** One pet, one boss, one update, one funny moment.
  A second idea is a second promo.
- **Cut on action.** Keep each shot only as long as something happens in
  it. Cut dead running, loading and menus.
- **Show the game, not the pitch.** Real gameplay beats text that says it
  is fun.

## Captions

- Short: one to four words per line, readable in under a second.
- Big and high contrast; the engine checks contrast and fails text that
  blends into the picture.
- Keep captions and titles out of the platform's UI areas. The safe zones
  are: a top bar (top 8-9% of the frame), a right rail (right 14% of the
  frame, from about 35-45% down to 86-88%) and the bottom caption area
  (bottom 17-20%, where the username and the user's own caption sit).
  Styles already place text safely; `check` fails any text that overlaps a
  zone, for TikTok, Reels and Shorts.
- Highlight the one word or number that matters (`"highlight": "auto"`).
- Caption sources: `voice` (spoken words), `music` (lyrics, only for the
  `lyric` style or when the user's song has words), `words` (a words file),
  `none`.

## The loop: probe, plan, render, look, fix

1. **Status.** `rbx_promo` action `status`. If the engine is not ready,
   tell the user to install the promo engine in Setup and stop. Do not try
   to install it yourself.
2. **Probe the clips.** `probe` with `paths` (all the clips). For each clip
   you get length, fps, size, whether it has audio, hard cuts, loud
   moments, motion, warnings and a frame sheet image. Look at every sheet.
   Pick moments from the cuts, loud windows and motion: a loud window
   usually means something happened. Clips are used where they are; they are
   never copied.
3. **Plan.** Decide the one idea, the hook, the order of 4-10 shots, the
   style, the caption words and the end card. Run `styles` once to see the
   stills if the user has no preference. Say the plan in two lines before
   rendering.
4. **Write `promo.json`.** In a folder you choose (for example next to your
   notes), then render it (see the recipe below).
5. **Render.** `render` with `recipe`. It validates, renders, writes the
   outputs and runs `check` itself. A render of more than a minute returns a
   `{"job": id}` at once: call `action: "job"` with `id` every minute or so
   until it reports the result. The progress is a number from 0 to 1. A job
   id is forgotten after its result was read; if you lose it, render again.
6. **Look at the sheet.** The result returns `sheet.jpg` (twelve frames)
   and `cover.jpg` as images. Look at them: is the hook visible in the first
   frame, is text readable and inside the safe zones, are the crops on the
   action, is anything black or empty?
7. **Read `check`.** `check.passed` must be true and `problems` empty. Fix
   every problem (move or shorten text, change the hook, change the music
   level) and render again. `notes` are not failures but read them: they say
   when a style preset was unresolved or a frame could not be read.
   `check` on its own re-runs the checks on an existing `promo.mp4`
   (`path`), using the `promo.json` and `layout.json` beside it.
8. **Only then show the user.** Give the folder, the video length, the
   style, and the post caption. Do not say a promo is done before you have
   looked at the sheet.

Errors tell you what to change. A recipe error lists every problem at once:
fix them all, then render again. If a clip is missing or not readable, say
which one and ask the user.

## The recipe (`promo.json`)

```json
{
  "version": 1, "size": [1080, 1920], "fps": 30,
  "style": "hype",
  "project": "pet-tycoon",
  "audio": {"music": "C:/Users/me/Music/song.mp3", "music_start": 12.4, "music_volume": 0.8,
            "voice_text": "This pet makes a million coins a minute."},
  "scenes": [
    {"clip": "C:/Users/me/Videos/run1.mp4", "in": 31.2, "out": 33.0,
     "crop": {"x": 0.55, "track": "auto"}, "speed": 1.0, "zoom": "punch@0.4", "transition": "whip"},
    {"title": "THIS PET IS INSANE", "duration": 1.2}
  ],
  "captions": {"source": "voice", "highlight": "auto"},
  "end_card": {"title": "Pet Tycoon", "cta": "Play now"},
  "post": {"caption": "This pet is broken #roblox", "hashtags": ["roblox", "robloxgames"]}
}
```

- `version` 1, `size` `[1080, 1920]`, `fps` 30 or 60. Always these.
- `style`: one of `hype`, `cinematic`, `meme`, `countdown`, `update-log`,
  `lyric`, `showcase`, `pov`. `styles` lists them with a still each.
  `hype` is the default for gameplay with action; `cinematic` for a trailer
  feel; `showcase` for calm building games; `update-log` for patch notes;
  `countdown` for top-5 lists; `lyric` only with the user's song.
- Scene with a clip: `clip` (a file path; spaces, non-ASCII and `C:\` paths
  are fine), `in` and `out` in seconds of the source, `speed` 0.25 to 4,
  `crop.x` (0 to 1, where the action is horizontally; the clip is cropped to
  9:16 around it) and `crop.track` `"auto"` to follow the motion, `zoom`
  (`punch@<seconds>`, `in`, `out`, comma-separated), `transition` into the
  scene (`cut`, `whip`, `flash`, `fade`, `zoom`, `slide`).
- Title scene: `title` and `duration` in seconds.
- Landscape, 4:3 and portrait clips all work; use `crop.x` to keep the
  action in frame. Clips without audio and clips with variable frame rate
  (OBS) work.
- `audio.voice` is a voice file you already have. `audio.voice_text` is text
  that `render` speaks itself (local voice, saved as `voice.wav` in the
  promo folder). Use one of them, never both. A voice that is longer than
  the scenes is an error: lengthen the scenes or cut words. `voice` and
  `transcribe` actions are there to test a line or time a song; for a normal
  promo `voice_text` is enough.
- `captions.source` `voice` needs `voice` or `voice_text`.
- `post.caption` and `post.hashtags` become `caption.txt` (see below).
- `project` is the folder name for the outputs. Use the game's name in
  lower case. If you leave it out, a general folder is used.
- Output goes to `work/promos/<project>/<YYYYMMDD-HHMMSS>-<slug>/` with
  `promo.json`, `promo.mp4`, `cover.jpg`, `caption.txt`, `sheet.jpg` (and
  `voice.wav` when the voice was spoken). Re-render a tweak by editing the
  `promo.json` in that folder and rendering it with `out` set to the same
  folder.

## Using a reference video

If the user likes a video and wants one like it, they must save the video
file and give you its path. You cannot fetch a TikTok or YouTube link, and
you must not try to download it (it is against the platforms' terms);
`reference` refuses URLs. If you get a link, say: "Save the video from the
app and give me the file."

`reference` with `path` returns the shots and their timings, the spoken
words, the music beats, a frame sheet and a starting recipe (`skeleton`).
Look at the sheet. Copy the structure: the length, how fast it cuts, where
the hook and the text are, how the music lands. Do not copy someone else's
footage, words or music. Fill the skeleton with the user's clips and pick a
style that matches the pacing. Say what you took from the reference.

## The post caption and hashtags

Write them with the promo, in `post`. The caption is one or two short lines
in the voice of a player, with a question or a reason to comment ("Would you
run this?"), not an advert. Keep it under 150 characters. Three to five
hashtags: `#roblox`, `#robloxgames`, the genre (`#tycoon`, `#obby`), and the
game's name. No hashtag spam. Show the caption to the user with the video;
it is in `caption.txt`.

## Music and voice

- Music is the user's own file (a song they made or have rights to), given as
  a path in `audio.music`. It loops and ducks under a voice, and it fades at
  the end. Never use music you found; never download any.
- If the user will add a trending TikTok sound in the app, render
  silent: leave out `audio.music` and `voice`. Say so: "Rendered without
  music so you can add a TikTok sound."
- Loudness is checked (it must land between -16 and -12 LUFS). A silent promo passes.
- **Ask before voiceover.** Do not write a voice line that claims
  something about the game (a number, "the best", "free", "new update",
  "rare") without asking the user to confirm it. Ask once, with the exact
  line. Captions made from `voice_text` carry the same claims.

## Checklist before you show the user

- First frame is a hook; nothing slow in the first second.
- 15 to 30 seconds, one idea, the best moment is early.
- Looked at `sheet.jpg` and `cover.jpg`; text readable and outside the safe
  zones; crops on the action.
- `check.passed` is true after the last render.
- Caption and hashtags written; voice claims confirmed with the user.
- Told the user where the folder is.
