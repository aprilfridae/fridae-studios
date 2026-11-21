# Promo Engine Implementation Plan (plan 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn any gameplay clips into a finished, self-checked vertical promo (MP4, cover, post caption) from the chat, shown as a card with a player, Download and Show in folder.

**Architecture:** A Python package `rbxpromo/` (CLI `rbx promo …`, MCP tool `rbx_promo`) validates a JSON recipe and drives a Remotion project in `promo/` through a portable Node that Fridae downloads on first use. Finished renders drop a card file into `WORK/promo-cards/`; the Rust app watches that folder (same pattern as `team.rs`) and shows a promo card in the chat that started it.

**Tech Stack:** Python 3.9+ stdlib + numpy/pillow/opencv (already bundled); Node 22 LTS portable; Remotion 4.0.x (`remotion`, `@remotion/cli`, `@remotion/renderer`, `@remotion/install-whisper-cpp`), `kokoro-js`; Rust (tiny_http, rfd, open — already dependencies); `ui/index.html`.

**Spec:** `docs/superpowers/specs/2026-11-20-promo-maker-design.md`

**Later plans (not here):** 2 recorder + gameplay library; 3 posting (connected accounts, post buttons, auto-post). This plan leaves a slot for post buttons on the card but adds none.

## Global Constraints

- Output: 1080×1920, 30 or 60 fps, H.264 video + AAC audio, `.mp4`.
- Per promo folder: `WORK/promos/<project-or-chat>/<YYYYMMDD-HHMMSS>-<slug>/` holding `promo.json`, `promo.mp4`, `cover.jpg`, `caption.txt`, `sheet.jpg`, optional `voice.wav`.
- Engine folder: `Documents/Fridae Studios/engine/` when packaged (`paths::packaged()` / `rbxpaths.FROZEN`), `<KIT>/engine/` from source. Never inside `WORK`. Never system-wide.
- Engine downloads show their size before starting and verify checksums (Node: `SHASUMS256.txt` from nodejs.org).
- Clips are referenced by path, never copied; no 30 MB limit applies to them.
- Windows 10 2004+/11 and macOS 12+. No Linux work.
- Remotion licence note, verbatim meaning: free for individuals, non-profits and for-profit teams of up to 3 people; larger companies need a Remotion company licence for the promo maker.
- Captions/titles must stay out of platform UI safe zones (Task 7 constants).
- Nothing in this plan posts anywhere or uploads footage.
- Credit adapted code: Subtle (MIT), ideas from OpenMotion (Apache-2.0) and Short Video Maker (MIT) in `THIRD_PARTY.md`.

## Review Focus

1. **Paths with spaces, non-ASCII letters and Windows drive letters** (`C:\Users\José\Videos\clip 1.mp4`): must validate, probe and render. Test in Task 1 and Task 4.
2. **Real-world clips: no audio track, variable frame rate (OBS default), landscape/4:3/portrait sources**: must probe and render without crashing; silent clips render with music/voice or silence. Test in Task 3 and Task 4.
3. **Engine missing or half-installed** (download interrupted, npm step failed): every action says exactly what is missing and `install` resumes instead of starting over. Test in Task 2.
4. **Long renders vs. the MCP call timeout** (`run_rbx` default 600 s): `render` must report progress and not be killed mid-render. Test in Task 6.
5. **Audio shorter or longer than the video** (song shorter than cut, voice longer than cut): music loops or fades, voice longer than the scenes is an error the agent can act on. Test in Task 1 and Task 4.

---

### Task 1: Recipe schema and validator

**Files:**
- Create: `rbxpromo/__init__.py`, `rbxpromo/recipe.py`
- Create: `tests/__init__.py`, `tests/test_promo_recipe.py`

**Interfaces:**
- Produces: `load_recipe(path: str) -> dict` (raises `RecipeError` with a list of human-readable problems); `validate(recipe: dict, base_dir: str) -> list[str]` (empty list = valid); `RecipeError(Exception)` with `.problems: list[str]`; `STYLES: tuple[str, ...]` placeholder filled by Task 4; `recipe_duration(recipe: dict) -> float`.

Recipe fields (version 1): `version` (=1), `size` ([1080,1920]), `fps` (30|60), `style` (str in STYLES), `audio` {`music`?, `music_start`?=0, `music_volume`?=0.8, `voice`?, `duck_music_under_voice`?=true}, `scenes` [ {`clip`, `in`, `out`, `crop`?{`x`:0..1,`track`:"auto"|"none"}, `speed`?0.25..4, `zoom`?str, `transition`?str} | {`title`, `duration`, `style`?} ], `captions`? {`source`:"voice"|"music"|"none"|"words", `style`, `highlight`:"auto"|"numbers"|"off", `words`?path}, `end_card`? {`title`, `cta`}, `post`? {`caption`, `hashtags`[]}, `final`? bool (used by plan 4).

- [ ] **Step 1: Write the failing tests** — `tests/test_promo_recipe.py` (stdlib `unittest`), fixtures written to a temp dir with tiny dummy files:
  - `test_minimal_valid`: one clip scene + style "hype" → `validate(...) == []`.
  - `test_relative_paths_resolve_against_recipe_dir`.
  - `test_path_with_spaces_and_unicode`: clip at `tmp/"Vidéos é"/"clip 1.mp4"` → valid.
  - `test_missing_clip_file`: problem text contains the path and "not found".
  - `test_in_after_out`: problem names the scene index ("scene 1: 'in' must be before 'out'").
  - `test_unknown_style`: problem lists allowed styles.
  - `test_voice_longer_than_scenes`: given `audio.voice` with known duration (pass `durations={"voice.wav": 9.0}` hook) and scenes totalling 6 s → problem "voice is 9.0 s but the video is 6.0 s".
  - `test_recipe_duration`: speeds and titles counted (clip 2 s at speed 2 → 1 s, title 1.2 s → 2.2 s total).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_recipe -v` → FAIL (module missing).
- [ ] **Step 3: Implement** `rbxpromo/recipe.py` with the interfaces above. `validate` accepts an optional `durations: dict[str, float] | None` for audio lengths (real lengths come from Task 3's `probe_file` when not given). Problems are full sentences the agent can act on.
- [ ] **Step 4: Run** the same command → PASS.
- [ ] **Step 5: Commit** `git add rbxpromo tests && git commit -m "promo: recipe schema and validator"`

### Task 2: Engine paths, status and installer

**Files:**
- Create: `rbxpromo/engine.py`, `promo/package.json`, `promo/package-lock.json` (generated)
- Test: `tests/test_promo_engine.py`

**Interfaces:**
- Produces: `engine_dir() -> str`; `status() -> dict` with keys `node` (path|None), `renderer` (bool: `promo/node_modules` installed into engine), `browser` (bool), `whisper` (bool), `voice` (bool), `ready` (bool), `missing` (list[str]), `download_mb` (int estimate); `install(progress: Callable[[str, float], None] | None = None) -> dict` (returns `status()`); `node_cmd(script: str, *args: str) -> list[str]`; `ffmpeg() -> str`, `ffprobe() -> str` (Remotion's bundled binaries via `@remotion/renderer`'s compositor package path).
- Layout inside `engine_dir()`: `node/` (portable Node 22 LTS), `renderer/` (copy of `promo/` sources + `node_modules`), `whisper/` (whisper.cpp + `ggml-base.en.bin`), `.state.json` (completed steps, for resume).

- [ ] **Step 1: Write the failing tests:**
  - `test_engine_dir_source_vs_packaged` (monkeypatch `rbxpaths.FROZEN`).
  - `test_node_download_url`: `node_url("win32","AMD64") ` ends `win-x64.zip`; `("darwin","arm64")` ends `darwin-arm64.tar.gz`; `("darwin","x86_64")` ends `darwin-x64.tar.gz`.
  - `test_checksum_mismatch_rejected`: `verify_sha256(file, wrong)` raises `EngineError` and deletes the file.
  - `test_status_reports_missing_parts`: empty temp engine dir → `ready is False`, `missing == ["node","renderer","browser","whisper","voice"]`.
  - `test_resume_skips_done_steps`: `.state.json` marking `node` done → `install(dry_run=True)` plan starts at `renderer`.
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_engine -v` → FAIL.
- [ ] **Step 3: Implement.** `install` steps, each recorded in `.state.json` when done: download+verify+extract Node; copy `promo/` to `renderer/` and `npm ci --omit=dev` with the engine's Node; `npx remotion browser ensure`; `@remotion/install-whisper-cpp` `installWhisperCpp` + `downloadWhisperModel("base.en")` via a small `promo/scripts/setup-whisper.mjs`; warm the Kokoro model with `promo/scripts/voice.mjs --warm`. Pin `remotion`/`@remotion/*` to one exact 4.0.x version in `promo/package.json`. `progress(step, fraction)` is called per step.
- [ ] **Step 4: Run** tests → PASS. Then manual: `python3 rbx.py promo install` on this Mac completes and `python3 rbx.py promo status` prints `ready: true`.
- [ ] **Step 5: Commit** `git commit -m "promo: engine download, verify and resume"`

### Task 3: Probe clips

**Files:**
- Create: `rbxpromo/probe.py`
- Test: `tests/test_promo_probe.py` (skips with a clear reason when `engine.status()["ready"]` is false)
- Test fixture helper: `tests/promo_fixtures.py` — `make_clip(path, seconds, size=(1280,720), fps=30, audio=True, cuts=())` using engine ffmpeg `lavfi` (`testsrc2`, `sine`; cuts = colour switches).

**Interfaces:**
- Consumes: `engine.ffmpeg()`, `engine.ffprobe()`, `rbxvideo.frames(path, count=…, out_dir=…)`.
- Produces: `probe_file(path: str) -> dict` (`duration`, `width`, `height`, `fps`, `vfr` bool, `has_audio` bool); `probe(paths: list[str], out_dir: str) -> dict` with per-clip `info`, `cuts` (seconds, ffmpeg `select='gt(scene,0.35)'`), `loud` (top 10 one-second windows by RMS, `[start, end, db]`), `motion` (top 10 windows by mean frame difference), `sheet` (path to contact sheet).

- [ ] **Step 1: Write the failing tests:** `test_probe_basic` (5 s 1280×720 clip → duration≈5±0.1, has_audio True); `test_probe_no_audio` (has_audio False, `loud == []`, no exception); `test_probe_detects_cuts` (cuts at 2 and 4 s found within ±0.2 s); `test_probe_portrait_and_4x3` (900×1600 and 640×480 probe fine); `test_probe_vfr_flagged` (fixture re-encoded with `-fps_mode vfr` and dropped frames → `vfr` True).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_probe -v` → FAIL.
- [ ] **Step 3: Implement** with ffprobe JSON (`-show_streams -show_format`), one ffmpeg pass for scene cuts + `astats`/`ebur128` per second, motion from OpenCV at 4 fps samples. Never read a whole clip into memory.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "promo: probe clips for cuts, loud and busy moments"`

### Task 4: Remotion renderer and styles

**Files:**
- Create: `promo/src/index.ts`, `promo/src/Root.tsx`, `promo/src/Promo.tsx`, `promo/src/scenes/ClipScene.tsx`, `promo/src/scenes/TitleScene.tsx`, `promo/src/EndCard.tsx`, `promo/src/captions/Captions.tsx`, `promo/src/captions/paginate.ts`, `promo/src/styles/*.json` + `promo/src/styles/index.ts`, `promo/scripts/render.mjs`, `promo/tsconfig.json`
- Modify: `rbxpromo/recipe.py` (fill `STYLES` from `promo/src/styles/*.json` names)
- Test: `promo/src/captions/paginate.test.ts` (run with `node --test` via tsx-free compiled JS or Remotion's bundled esbuild), `tests/test_promo_render.py` (skips if engine not ready)

**Interfaces:**
- Consumes: recipe dict (Task 1), clip paths passed as `file://` URLs through Remotion `staticFile`-free `OffthreadVideo src`.
- Produces: `node render.mjs <recipe.json> <out.mp4> --layout <layout.json> [--still <sec> <out.jpg>]` printing JSON progress lines `{"progress":0.42}` and a final `{"ok":true,"duration":…}`; `layout.json` = list of `{from, to, x, y, w, h, kind:"caption"|"title"|"end"}` in pixels for every text box (consumed by Task 7).
- Styles (at least 8, one JSON each = edit rhythm + transition set + caption style + title style): `hype`, `cinematic`, `meme`, `countdown`, `update-log`, `lyric`, `showcase`, `pov`. Caption styles adapted from Subtle (MIT; keep its copyright line in a header comment): `punch`, `highlighter`, `karaoke`, `bottom`, `impact`, `glass`, `ticket`, `cinema`.

- [ ] **Step 1: Write the failing tests:**
  - `paginate.test.ts`: numbers merged into one token (`["five","thousand","dollars"]` → `$5,000`); no page crosses a cut at 2.0 s; one highlight per page; a page never exceeds 3 lines at 1080 px width with the style's font size.
  - `test_render_two_scenes`: two 3 s fixtures (one silent, one 4:3) + title + end card + music fixture shorter than the video → `promo.mp4` exists, ffprobe says 1080×1920, 30 fps, h264/aac, duration = `recipe_duration` ±0.1 s, audio stream present.
  - `test_render_unicode_path`: same with clips under `tmp/Vidéos é/`.
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_render -v` and `node --test promo/src/captions` → FAIL.
- [ ] **Step 3: Implement.** `Promo.tsx` sequences scenes with `<Series>`; `ClipScene` crops to 9:16 using `crop.x` (and, for `track:"auto"`, a per-scene focus x computed by Task 6 and passed in props as `focus` keyframes), applies `speed` via `playbackRate`, `zoom` punches as scale springs, transitions from the style. Music loops when shorter than the video and fades out over the last 0.5 s; ducks to 25% under voice. `render.mjs` uses `@remotion/renderer` `bundle` + `selectComposition` + `renderMedia` (codec h264, audioCodec aac, crf 18) and writes `layout.json` from the same pagination code.
- [ ] **Step 4: Run** both → PASS. Look at one rendered still per style (`--still 1.0`) yourself.
- [ ] **Step 5: Commit** `git commit -m "promo: Remotion renderer with 8 styles"`

### Task 5: Voice and word timing

**Files:**
- Create: `promo/scripts/voice.mjs`, `promo/scripts/transcribe.mjs`, `rbxpromo/audio.py`
- Test: `tests/test_promo_audio.py` (skips if engine not ready)

**Interfaces:**
- Produces: `voice(text: str, out_wav: str, voice: str = "af_heart") -> dict` (`file`, `duration`, `words` [{`text`,`start`,`end`}]); `transcribe(path: str, lang: str = "en") -> dict` (`words`, `duration`); both cache by SHA-256 of input (+ text/voice) in `engine_dir()/cache/`.

- [ ] **Step 1: Write the failing tests:** `test_voice_hello` ("Hello world, play now" → wav 0.8–4 s, words include "hello" and "play"); `test_transcribe_voice_roundtrip` (transcribe that wav → ≥3 words, starts increasing); `test_transcribe_cached` (second call returns in <1 s and same result); `test_transcribe_silent_file` (silent 3 s wav → `words == []`, no exception).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_audio -v` → FAIL.
- [ ] **Step 3: Implement** `voice.mjs` with `kokoro-js` (`KokoroTTS.from_pretrained("onnx-community/Kokoro-82M-v1.0-ONNX", {dtype:"q8"})`) writing 24 kHz wav, then word timings via `transcribe.mjs` (`@remotion/install-whisper-cpp` `transcribe` with `tokenLevelTimestamps`, converted with `toCaptions`). Song lyrics use the same `transcribe`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "promo: local voiceover and word timing"`

### Task 6: Render pipeline, outputs and the card file

**Files:**
- Create: `rbxpromo/render.py`
- Test: `tests/test_promo_pipeline.py` (skips if engine not ready)

**Interfaces:**
- Consumes: `load_recipe`, `validate` (Task 1), `probe_file` (Task 3), `render.mjs` (Task 4), `transcribe` (Task 5), `check` (Task 7, called last; until Task 7 lands, a stub returning `{"passed": true, "problems": []}`).
- Produces: `render(recipe_path: str, out_dir: str | None = None, on_progress=None) -> dict` with `folder`, `video`, `cover`, `caption`, `sheet`, `duration`, `check`; writes `WORK/promo-cards/<id>.json` = `{"id","agent","folder","video","cover","caption_text","style","duration","check","created"}` where `agent = os.environ.get("RBXKIT_AGENT_ID","")`; `promo_folder(project_or_chat: str, title: str) -> str`.

- [ ] **Step 1: Write the failing tests:** `test_render_writes_all_outputs` (folder has the 5 files + `promo.json` copy; `caption.txt` = post caption + newline + `#tags`); `test_card_file_written` (card JSON has the env agent id and existing paths); `test_auto_track_focus` (fixture with a moving white square → `focus` keyframes follow it within 10% of width); `test_progress_reported` (on_progress receives increasing values ending at 1.0); `test_invalid_recipe_no_render` (RecipeError, no folder created).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_pipeline -v` → FAIL.
- [ ] **Step 3: Implement.** Auto-track: OpenCV motion centroid per 0.25 s, smoothed (moving average, max pan 0.3 width/s). Cover = still at the first title or 0.5 s. Sheet = `rbxvideo.frames(video, count=12)`. Card written last, only after `check` ran.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "promo: render pipeline, outputs and chat card file"`

### Task 7: Self-check

**Files:**
- Create: `rbxpromo/check.py`
- Modify: `rbxpromo/render.py` (replace stub)
- Test: `tests/test_promo_check.py`

**Interfaces:**
- Produces: `SAFE_ZONES: dict[str, list[tuple[float,float,float,float]]]` (fractions x0,y0,x1,y1 of 1080×1920 that text must avoid): `tiktok` = top bar (0,0,1,0.09), right rail (0.86,0.35,1,0.86), bottom caption (0,0.80,1,1); `reels` = (0,0,1,0.08), (0.86,0.45,1,0.88), (0,0.82,1,1); `shorts` = (0,0,1,0.08), (0.86,0.40,1,0.88), (0,0.83,1,1). `check(video: str, layout: list[dict], recipe: dict) -> dict` = `{"passed": bool, "problems": [str], "notes": [str]}`.
- Rules: any text box overlapping any zone of any platform by >2% of its area → problem naming the time and platform; hook: a cut, title or caption must start by 1.0 s; length > 60 s → note (Shorts); integrated loudness outside −16…−12 LUFS → problem with the measured value; text contrast: sampled luminance behind each box vs. text colour, WCAG ratio < 3 → problem.

- [ ] **Step 1: Write the failing tests** with hand-made layouts (no engine needed for these): `test_caption_in_right_rail_fails`, `test_centered_caption_passes`, `test_no_hook_fails`, `test_long_video_note`; plus `test_loudness_measured` (engine-gated, −30 LUFS fixture → problem mentions "-30").
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_check -v` → FAIL.
- [ ] **Step 3: Implement** and wire into `render`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "promo: safe-zone, hook, loudness and contrast checks"`

### Task 8: Reference videos and style list

**Files:**
- Create: `rbxpromo/reference.py`
- Test: `tests/test_promo_reference.py` (engine-gated)

**Interfaces:**
- Consumes: `probe` (Task 3), `transcribe` (Task 5), `rbxvideo.frames`.
- Produces: `reference(path: str, out_dir: str) -> dict` = `{"skeleton": <recipe with scenes whose clip is null and in/out = shot length>, "shots": [{start,end}], "words": […], "sheet": path, "beats": [seconds]}` (beats from onset strength via numpy on the audio RMS); `styles(out_dir: str) -> list[{"name","about","still"}]` rendering one still per style.

- [ ] **Step 1: Tests:** `test_reference_shot_lengths` (fixture with cuts at 1.5, 3.0, 4.2 s → shots match ±0.15 s; skeleton validates once clips are filled); `test_reference_rejects_url` (`"https://www.tiktok.com/..."` → error telling the user to save the video file first, per spec); `test_styles_lists_all` (names == STYLES, each still exists).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "promo: reference breakdown and style stills"`

### Task 9: CLI, MCP tool, setup check and agent guide

**Files:**
- Create: `rbxpromo/cli.py`, `skills/roblox-promo/SKILL.md`
- Modify: `rbx.py` (register `rbxpromo.cli` in `names`; `doctor` row "Promo engine"), `rbxkit_mcp.py` (`t_promo`, TOOLS entry, guide topic `promo`), `routing.txt` (route promo/TikTok/trailer requests to the guide)
- Test: `tests/test_promo_mcp.py`

**Interfaces:**
- Consumes: everything above.
- Produces: `rbx promo {status,install,probe,reference,voice,transcribe,render,check,styles}`; MCP tool `rbx_promo` with `action` enum of the same names (`install` excluded: the app owns installs, Task 11) and fields `paths` (array), `path`, `recipe`, `text`, `voice`, `out`. `render` runs as a background job: the tool returns `{"job": id}` at once if it would exceed 60 s, and `action:"job"` with `id` returns progress or the result, so the 600 s MCP timeout never kills a render (Review Focus 4).

- [ ] **Step 1: Tests:** `test_tools_list_has_promo` (calls `rbxkit_mcp.handle({"method":"tools/list"})`); `test_promo_needs_action`; `test_render_returns_job_when_slow` (monkeypatched render sleeping 2 s with threshold lowered to 1 s → `job` id, then `job` action reports done); `test_engine_missing_message` (status not ready → text says "Install the promo engine in Setup" and lists missing parts).
- [ ] **Step 2: Run** `python3 -m unittest tests.test_promo_mcp -v` → FAIL.
- [ ] **Step 3: Implement** and write the guide: hook in the first second, 15–30 s, core loop and best moment early, one idea per promo, caption rules, safe zones, probe → plan → render → look at the sheet → fix before showing, using `reference`, post caption and hashtags, music = user's own file or silent for a TikTok sound, reference videos are saved files only, ask before voiceover claims about the game.
- [ ] **Step 4: Run** → PASS; `python3 rbx.py doctor` shows the Promo engine row.
- [ ] **Step 5: Commit** `git commit -m "promo: CLI, rbx_promo tool and promo guide"`

### Task 10: App — clips by path, media serving, promo card

**Files:**
- Create: `app-rs/src/promo.rs` (card watcher, reveal, save-copy), `app-rs/src/media.rs` (range-request file serving)
- Modify: `app-rs/src/main.rs` (spawn watcher; wry drag-drop handler for video/audio files → paths), `app-rs/src/server.rs` (routes below), `ui/index.html` (Add clips button, card UI)
- Test: `cargo test` unit tests in `media.rs` and `promo.rs`

**Interfaces:**
- Consumes: card files from Task 6.
- Produces (routes, all token-protected like the rest): `GET /media/promo/<card id>/<video|cover>` with `Range` support (206, `Content-Range`, `Accept-Ranges: bytes`, `video/mp4`); `POST /api/pick-files {kind:"clips"|"music"|"reference"}` → `{paths:[…]}` via `rfd::FileDialog` (filters mp4/mov/mkv/webm; mp3/wav/m4a/ogg); `POST /api/promo {action:"reveal"|"save", id}` (save → `rfd` save dialog, copies mp4, and cover/caption beside it if ticked); BUS event `promo` = card JSON, routed to the agent id on the card (unknown/empty id → the chat in front).
- `promo::reveal_cmd(path) -> (String, Vec<String>)`: macOS `open -R <path>`, Windows `explorer /select,<path>`.
- Dropped or picked files are inserted into the composer as `[clip: <path>]` lines the agent reads as references; they never go through `/api/attach`.

- [ ] **Step 1: Write the failing Rust tests:** `parse_range("bytes=0-99", 1000) == Some((0,99))`, `("bytes=900-", 1000) == Some((900,999))`, `("bytes=2000-", 1000) == None` (→ 416); `reveal_cmd` per OS (`#[cfg]`-gated asserts); `card_route_target` picks the card's agent, falls back when empty.
- [ ] **Step 2: Run** `cargo test --manifest-path app-rs/Cargo.toml` → FAIL.
- [ ] **Step 3: Implement.** Card UI: `<video controls playsinline>` 9:16 max-height 70vh, style + length, editable post caption (saved back to `caption.txt`), check result line ("Checked: safe areas, hook, loudness" or the problems), buttons **Download** and **Show in folder**, and an empty `div.promo-post` slot for plan 3.
- [ ] **Step 4: Run** `cargo test` → PASS. Manual on macOS: pick two clips with Add clips, ask for a promo, card appears, video seeks, Download saves a copy, Show in folder opens Finder on the file. Drag a 200 MB clip onto the window → path inserted, nothing copied.
- [ ] **Step 5: Commit** `git commit -m "app: clip picker, promo card, download and show in folder"`

### Task 11: Engine install in Setup, packaging and docs

**Files:**
- Modify: `app-rs/src/server.rs` (`POST /api/promo-engine {action:"status"|"install"}`; install runs `paths::cli(["promo","install","--json-progress"])` in a thread and streams BUS `promo_engine` events), `ui/index.html` (Setup row: state, size, Install button, progress bar), `rbxkit.spec` (bundle `promo/` sources except `node_modules`), `build_mac.py` and `build_app.py` (nothing else needed; confirm `promo/` lands in `_internal`/`tools`), `README.md` (Promo maker section), `THIRD_PARTY.md`, `DEVELOPING.md`
- Test: manual verification steps below (UI + packaging)

- [ ] **Step 1: Implement** the Setup row: "Promo engine — not installed (about N MB) [Install]" → progress per step → "ready". A failed step shows its error and a Retry that resumes (Task 2 state).
- [ ] **Step 2: README** "Promo maker" section: what it does; downloads its engine on first use; the Remotion licence sentence from Global Constraints; music is the user's responsibility (own track, free-licensed, or post silent and add a sound in TikTok); reference TikToks must be saved video files. `THIRD_PARTY.md`: Remotion (its licence), Subtle (MIT), kokoro-js + Kokoro-82M (Apache-2.0), whisper.cpp (MIT), ideas from OpenMotion (Apache-2.0) and Short Video Maker (MIT).
- [ ] **Step 3: Verify packaging:** `.venv/bin/python build_mac.py` → open the built app with a bare environment (`env -i HOME=$HOME PATH=/usr/bin:/bin`), Setup shows the row, Install completes into `~/Documents/Fridae Studios/engine/`, render a promo from two fixture clips, card shows. Interrupt an install halfway (quit the app), reopen, Retry resumes.
- [ ] **Step 4: Run all tests:** `python3 -m unittest discover tests -v` and `cargo test --manifest-path app-rs/Cargo.toml` → all PASS (engine-gated tests run, not skipped, on the dev Mac).
- [ ] **Step 5: Commit** `git commit -m "promo: engine install in Setup, packaging and docs"`
