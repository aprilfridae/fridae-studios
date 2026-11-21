"""Reference videos and the style list for the promo maker.

  reference(path, out_dir) -> {"skeleton", "shots", "words", "sheet", "beats"}
      Breaks a saved video (a TikTok the user downloaded themselves) into shot lengths, spoken words,
      music beats and a contact sheet. The skeleton is a recipe with one clip scene per shot
      (`clip` null, to be filled with the user's game clips) in the same shape recipe.py validates.
  styles(out_dir) -> [{"name", "about", "still"}]
      One rendered still per style, cached by style, preview version and promo sources hash. Each still is the
      style over a bright sample clip made here in code (sample_clip), so the styles can be told apart.

Fridae never downloads from a link: a URL is refused with a note to save the video file first.
"""
import glob, hashlib, json, os, re, subprocess, tempfile

import rbxpaths
from rbxpromo import engine
from rbxpromo.recipe import STYLES

MIN_SHOT = 0.2           # seconds; a shorter "shot" is a detection glitch and joins the previous one
HOP = 0.01               # beat analysis: seconds between energy frames
BEAT_GAP = 0.25          # no two beats closer than this
WIN = 0.03               # seconds of audio per energy frame
BASELINE = 0.5           # seconds of recent energy a beat has to rise above
MIN_LEVEL = 0.003        # quieter than this (full scale RMS) is silence: no beats
BEAT_FLOOR = 0.3         # smallest rise (fraction of the recent level) that can count as a beat
BEAT_RATE = 8000
PREVIEW_VERSION = 2      # bump when the preview recipe or the sample clip changes: every still is rendered again
PREVIEW_TITLE = "Sky Islands"
PREVIEW_CAPTION = "Only 1% reach the top"
PREVIEW_TITLE_SECONDS = 1.5
PREVIEW_TITLE_AT = 1.05  # seconds: the title card, its animation settled, before the cut
PREVIEW_CAPTION_AT = 2.2  # seconds: the footage with the caption page holding "1%" on screen
PREVIEW_TITLE_SHARE = 0.4  # the title band's share of the still's height; the footage band has the rest
SAMPLE_SIZE = (1080, 1920)
SAMPLE_FPS = 30
SAMPLE_SECONDS = 3.0
_URL = re.compile(r"^\s*([a-z][a-z0-9+.-]*://|www\.)", re.I)


def _refuse_url(path):
    if _URL.match(str(path)):
        raise ValueError(
            "Fridae does not download videos from links. Please save the video file first (for example with your "
            "phone or the TikTok app's Save video), then drop the file into the chat or give its path: "
            "%s" % path)


# ---------------------------------------------------------------- beats

def beats_from_samples(samples, rate):
    """Beat times in seconds. Onset strength is how far the short-window RMS rises above its own recent
    average (about the last 0.5 s), as a fraction of that average, so a steady music bed does not hide
    a kick. Peaks above an adaptive threshold count, at least BEAT_GAP apart. Silence gives none."""
    import numpy as np
    x = np.asarray(samples, dtype=np.float64)
    hop, win = max(1, int(rate * HOP)), max(2, int(rate * WIN))
    n = (len(x) - win) // hop + 1
    if n < 3:
        return []
    sq = np.concatenate([[0.0], np.cumsum(x * x)])
    starts = np.arange(n) * hop
    rms = np.sqrt(np.maximum(sq[starts + win] - sq[starts], 0.0) / win)
    back, skip = int(BASELINE / HOP), 3
    onset = np.zeros(n)
    for i in range(skip + 1, n):
        past = rms[max(0, i - back):i - skip]
        base = past.mean()
        if base > MIN_LEVEL:
            onset[i] = max(0.0, rms[i] - base) / base
    thr = max(BEAT_FLOOR, float(onset.mean() + 2 * onset.std()))
    out, strength = [], []
    for i in np.flatnonzero(onset > thr):
        if onset[i] < onset[max(0, i - 4):i + 5].max():
            continue                                    # not the local peak
        t = round(float(i * hop / rate + WIN / 2), 3)
        if not out or t - out[-1] >= BEAT_GAP:
            out.append(t)
            strength.append(onset[i])
        elif onset[i] > strength[-1]:
            out[-1], strength[-1] = t, onset[i]         # a stronger onset inside the gap replaces the weaker
    return out


def _decode_mono(path):
    """Mono 8 kHz samples (float, -1..1) of the file's audio via a WAV pipe; None if it has no audio track."""
    import numpy as np
    r = subprocess.run([engine.ffmpeg(), "-nostdin", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(BEAT_RATE),
                        "-c:a", "pcm_s16le", "-f", "wav", "-"], env=engine.tool_env(),
                       stdin=subprocess.DEVNULL, capture_output=True)
    if r.returncode != 0:
        raise RuntimeError("ffmpeg could not decode the audio of %s: %s"
                           % (path, r.stderr.decode("utf-8", "replace").strip()[-300:]))
    data, pos = r.stdout, 12
    while pos + 8 <= len(data):          # a piped WAV has placeholder sizes on 'data', so stop there
        cid, size = data[pos:pos + 4], int.from_bytes(data[pos + 4:pos + 8], "little")
        if cid == b"data":
            pcm = data[pos + 8:]
            return np.frombuffer(pcm[:len(pcm) // 2 * 2], dtype="<i2").astype(np.float64) / 32768.0
        pos += 8 + size + (size & 1)
    raise RuntimeError("could not find the audio data in the WAV ffmpeg returned for " + path)


# ---------------------------------------------------------------- reference

def _spoken(words):
    """Drop the speech model's sound annotations on music or noise, like "(eerie music)" or "[BLANK_AUDIO]"."""
    out, depth = [], 0
    for w in words:
        t = w["text"].strip()
        opens = t.count("(") + t.count("[") - t.count(")") - t.count("]")
        note = depth > 0 or t[:1] in "([" or "\u266a" in t
        depth = max(0, depth + opens)
        if not note:
            out.append(w)
    return out


def _shots(cuts, duration):
    edges = [0.0] + [c for c in sorted(cuts) if 0 < c < duration] + [duration]
    shots = []
    for a, b in zip(edges, edges[1:]):
        if shots and b - a < MIN_SHOT:
            shots[-1][1] = b
        else:
            shots.append([a, b])
    if len(shots) > 1 and shots[-1][1] - shots[-1][0] < MIN_SHOT:
        last = shots.pop()
        shots[-1][1] = last[1]
    return [{"start": round(a, 3), "end": round(b, 3)} for a, b in shots]


def _skeleton(shots):
    mean = sum(s["end"] - s["start"] for s in shots) / len(shots)
    style = "hype" if mean < 2.0 else "cinematic"       # a starting guess: quick cuts or long shots
    return {"version": 1, "size": [1080, 1920], "fps": 30, "style": style,
            "scenes": [{"clip": None, "in": 0, "out": round(s["end"] - s["start"], 3)} for s in shots]}


def reference(path, out_dir):
    """Break a saved reference video into shots, words, beats and a contact sheet (see module doc)."""
    _refuse_url(path)
    if not os.path.isfile(path):
        raise ValueError("Reference video not found: %s. Save the video file and give its path." % path)
    engine.require_ready()
    from rbxpromo import audio
    from rbxpromo.probe import probe
    p = probe([path], out_dir)[path]
    duration = p["info"]["duration"]
    if duration <= 0:
        raise ValueError("Could not read the length of %s." % path)
    shots = _shots(p["cuts"], duration)
    words, beats = [], []
    if p["info"]["has_audio"]:
        words = _spoken(audio.transcribe(path)["words"])
        samples = _decode_mono(path)
        beats = beats_from_samples(samples, BEAT_RATE) if samples is not None else []
    return {"skeleton": _skeleton(shots), "shots": shots, "words": words, "sheet": p["sheet"],
            "beats": beats, "warnings": p["warnings"]}


# ---------------------------------------------------------------- styles

def _sample_frame(t):
    """One frame of the sample clip at `t` seconds, as a PIL image: a bright, blocky sky-island obby with no
    text. Clouds and islands drift, a cube hops between platforms and a coin spins, so a still looks like play.
    Pillow only, so the previews need nothing beyond what the styles sheet already uses."""
    import math
    from PIL import Image, ImageDraw
    w, h = SAMPLE_SIZE
    img = Image.new("RGB", (w, h))
    draw = ImageDraw.Draw(img)
    shade = lambda c, f: tuple(max(0, min(255, int(v * f))) for v in c)
    # sky: deep blue at the top to pale cyan at the horizon
    top, low = (45, 140, 235), (170, 236, 255)
    for y in range(h):
        k = y / (h - 1)
        draw.line([(0, y), (w, y)], fill=tuple(int(a * (1 - k) + b * k) for a, b in zip(top, low)))
    sx, sy = int(w * 0.78), int(h * 0.13)
    draw.ellipse([sx - 90, sy - 90, sx + 90, sy + 90], fill=(255, 245, 170))
    draw.ellipse([sx - 70, sy - 70, sx + 70, sy + 70], fill=(255, 255, 215))

    def cube(x, y, s, col, depth=0.45):
        """An axonometric cube: front face at (x, y) size s, a lighter top and a darker side."""
        d = int(s * depth)
        draw.polygon([(x + s, y), (x + s + d, y - d), (x + s + d, y + s - d), (x + s, y + s)], fill=shade(col, 0.72))
        draw.polygon([(x, y), (x + d, y - d), (x + s + d, y - d), (x + s, y)], fill=shade(col, 1.18))
        draw.rectangle([x, y, x + s, y + s], fill=col)

    def slab(x, y, sw, sh, grass, dirt):
        """A floating island platform: a grass top over a dirt block."""
        d = int(sh * 0.6)
        draw.rectangle([x, y, x + sw, y + sh], fill=dirt)
        draw.polygon([(x, y), (x + d, y - d), (x + sw + d, y - d), (x + sw, y)], fill=grass)
        draw.rectangle([x, y, x + sw, y + max(8, sh // 5)], fill=shade(grass, 0.85))
        draw.polygon([(x + sw, y), (x + sw + d, y - d), (x + sw + d, y + sh - d), (x + sw, y + sh)], fill=shade(dirt, 0.6))

    # clouds: blocky white, drifting slowly
    for i, (cx, cy, cw) in enumerate(((0.1, 0.08, 260), (0.55, 0.2, 200), (0.95, 0.3, 300), (0.3, 0.36, 180))):
        x = int((cx * w + t * (30 + 12 * i)) % (w + 400)) - 200
        y = int(cy * h)
        for dx, dy, f in ((0, 0, 1.0), (cw * 0.3, -40, 0.7), (cw * 0.55, 0, 0.8)):
            draw.rectangle([int(x + dx), int(y + dy), int(x + dx + cw * f), int(y + dy + 70)], fill=(248, 252, 255))
    # far islands (slow), then a path of platforms up the frame
    pan = t * 40
    for x0, y0, sw, col in ((0.05, 0.42, 150, (120, 200, 90)), (0.62, 0.38, 120, (140, 190, 80)), (0.84, 0.47, 170, (110, 210, 100))):
        slab(int(x0 * w - pan * 0.4), int(y0 * h), sw, 50, col, (160, 110, 70))
    for x0, y0, sw, col in ((0.08, 0.86, 380, (90, 200, 60)), (0.58, 0.74, 300, (240, 215, 70)), (0.12, 0.62, 260, (70, 120, 230)),
                            (0.56, 0.52, 230, (235, 90, 90))):
        slab(int(x0 * w - pan), int(y0 * h), sw, 110, col, (150, 105, 60))
    # lava below the path
    lava_y = int(h * 0.94)
    draw.rectangle([0, lava_y, w, h], fill=(245, 90, 30))
    for i in range(0, w + 120, 120):
        x = int(i - (t * 160) % 120)
        draw.rectangle([x, lava_y, x + 60, lava_y + 18], fill=(255, 170, 40))
    # coloured cubes on the platforms
    for x0, y0, sz, col in ((0.18, 0.86, 90, (235, 60, 40)), (0.30, 0.86, 70, (250, 210, 40)), (0.70, 0.74, 80, (40, 120, 220)),
                            (0.20, 0.62, 64, (200, 70, 170))):
        cube(int(x0 * w - pan), int(y0 * h - sz - 2), sz, col)
    # the player: an orange cube hopping from the green platform to the yellow one
    p = (t % 1.5) / 1.5
    cube(int(w * (0.25 + 0.42 * p) - pan), int(h * (0.86 - 0.12 * p) - 120 - 260 * 4 * p * (1 - p)), 110, (255, 140, 30))
    # a spinning coin above the red platform
    c = abs(math.cos(t * 4.0))
    cx, cy = int(w * 0.66 - pan), int(h * 0.44)
    r1, r2 = max(6, int(48 * c)), max(3, int(30 * c))
    draw.ellipse([cx - r1, cy - 48, cx + r1, cy + 48], fill=(255, 190, 20))
    draw.ellipse([cx - r2, cy - 30, cx + r2, cy + 30], fill=(255, 230, 90))
    return img


def sample_clip(out_dir):
    """The preview footage, made once per PREVIEW_VERSION and kept beside the stills: a silent H.264 clip
    (sample-p<version>.mp4), written under another name first so a half-written clip is never used."""
    clip = os.path.join(out_dir, "sample-p%d.mp4" % PREVIEW_VERSION)
    if os.path.isfile(clip) and os.path.getsize(clip) > 0:
        return clip
    fd, writing = tempfile.mkstemp(prefix="sample-", suffix=".writing.mp4", dir=out_dir)
    os.close(fd)
    try:
        with tempfile.TemporaryDirectory() as frames:
            # frames as files, not a pipe: every child gets stdin=DEVNULL (and the engine's ffmpeg has no rawvideo)
            for i in range(int(round(SAMPLE_SECONDS * SAMPLE_FPS))):
                _sample_frame(i / SAMPLE_FPS).save(os.path.join(frames, "f%04d.png" % i), compress_level=1)
            cmd = [engine.ffmpeg(), "-nostdin", "-y", "-hide_banner", "-loglevel", "error",
                   "-framerate", str(SAMPLE_FPS), "-i", os.path.join(frames, "f%04d.png"),
                   "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-an",
                   "-movflags", "+faststart", "-f", "mp4", writing]
            r = subprocess.run(cmd, env=engine.tool_env(), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, timeout=300)
        if r.returncode != 0:
            tail = r.stderr.decode("utf-8", "replace")[-400:].strip()
            raise engine.EngineError("Could not make the style preview clip: " + (tail or "ffmpeg exit code %s" % r.returncode))
        os.replace(writing, clip)
    finally:
        if os.path.exists(writing):
            os.remove(writing)
    for old in glob.glob(os.path.join(glob.escape(out_dir), "sample-p*.mp4")):
        if old != clip:
            os.remove(old)
    return clip


def _styles_dir():
    return os.path.join(rbxpaths.KIT, "promo", "src", "styles")


def _about(name):
    with open(os.path.join(_styles_dir(), name + ".json"), encoding="utf-8") as f:
        return str(json.load(f).get("about", ""))


def _readable_image(path):
    from PIL import Image
    try:
        with Image.open(path) as im:
            im.load()
        return True
    except (OSError, ValueError, SyntaxError):
        return False


def _preview_recipe(name, clip, words_path):
    """A title card, then the sample clip with one caption line: the two things a style changes most."""
    return {"version": 1, "size": [SAMPLE_SIZE[0], SAMPLE_SIZE[1]], "fps": SAMPLE_FPS, "style": name,
            "scenes": [{"title": PREVIEW_TITLE, "duration": PREVIEW_TITLE_SECONDS},
                       {"clip": clip, "in": 0, "out": SAMPLE_SECONDS}],
            "captions": {"source": "words", "words": words_path}}


def _preview_words():
    """The caption line, timed so its page is on screen at PREVIEW_CAPTION_AT."""
    start, step = PREVIEW_TITLE_SECONDS + 0.25, 0.3
    words = PREVIEW_CAPTION.split()
    return [{"text": w, "start": round(start + i * step, 3), "end": round(start + (i + 1) * step - 0.02, 3)}
            for i, w in enumerate(words)]


def _stack_preview(title_jpg, clip_jpg, style_name, out_jpg):
    """One 9:16 still from two rendered frames: the band of the title card around the title above the band of the
    footage around the caption. Both bands are cut from real frames, nothing is drawn on them."""
    from PIL import Image
    w, h = SAMPLE_SIZE
    frames = []
    for path in (title_jpg, clip_jpg):
        with Image.open(path) as im:
            im = im.convert("RGB")
            frames.append(im if im.size == (w, h) else im.resize((w, h), Image.LANCZOS))
    with open(os.path.join(_styles_dir(), style_name + ".json"), encoding="utf-8") as f:
        style = json.load(f)
    top_h = int(h * PREVIEW_TITLE_SHARE)

    def band(img, centre, size):
        y0 = int(min(max(0, centre * h - size / 2), h - size))
        return img.crop((0, y0, w, y0 + size))
    out = Image.new("RGB", (w, h))
    out.paste(band(frames[0], float(style.get("title", {}).get("y", 0.45)), top_h), (0, 0))
    out.paste(band(frames[1], _caption_y(style.get("captions")), h - top_h), (0, top_h))
    out.save(out_jpg, "JPEG", quality=86)


def _caption_y(preset):
    """Centre of the caption block for a caption preset (promo/src/captions/presets.ts), 0.65 if not found."""
    try:
        with open(os.path.join(rbxpaths.KIT, "promo", "src", "captions", "presets.ts"), encoding="utf-8") as f:
            text = f.read()
        m = re.search(r"name: '%s'.*?\by: ([0-9.]+)" % re.escape(str(preset)), text, re.S)
        return float(m.group(1)) if m else 0.65
    except OSError:
        return 0.65


def styles(out_dir):
    """[{"name","about","still"}] for every style; each still is rendered once per preview version and
    promo-source hash, over the sample clip (see sample_clip)."""
    from rbxpromo import render
    os.makedirs(out_dir, exist_ok=True)
    # 12 hex like before (the app serves `<name>-<12 hex>.jpg`), now of the preview version and the promo sources
    tag = hashlib.sha1(("preview %d %s" % (PREVIEW_VERSION, engine.promo_hash())).encode("utf-8")).hexdigest()[:12]
    items = []
    clip = None
    for name in STYLES:
        still = os.path.join(out_dir, "%s-%s.jpg" % (name, tag))
        if os.path.isfile(still) and not _readable_image(still):
            os.remove(still)                        # a broken cached still is rendered again
        if not os.path.isfile(still):
            clip = clip or sample_clip(out_dir)
            part = os.path.join(out_dir, "%s-%s.part.jpg" % (name, tag))   # never a half-written still under the real name
            try:
                with tempfile.TemporaryDirectory() as tmp:
                    words = os.path.join(tmp, "words.json")
                    rbxpaths.write_json(words, _preview_words())
                    rp = os.path.join(tmp, "recipe.json")
                    rbxpaths.write_json(rp, _preview_recipe(name, clip, words))
                    title_jpg, clip_jpg = os.path.join(tmp, "title.jpg"), os.path.join(tmp, "clip.jpg")
                    render.render_still(rp, title_jpg, PREVIEW_TITLE_AT)
                    render.render_still(rp, clip_jpg, PREVIEW_CAPTION_AT)
                    _stack_preview(title_jpg, clip_jpg, name, part)
                os.replace(part, still)
            finally:
                if os.path.exists(part):
                    os.remove(part)
            for old in glob.glob(os.path.join(glob.escape(out_dir), glob.escape(name) + "-*.jpg")):
                if old != still and re.fullmatch(re.escape(name) + r"-[0-9a-f]{12}\.jpg", os.path.basename(old)):
                    os.remove(old)
        items.append({"name": name, "about": _about(name), "still": still})
    return items
