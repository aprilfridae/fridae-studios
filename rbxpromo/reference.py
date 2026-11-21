"""Reference videos and the style list for the promo maker.

  reference(path, out_dir) -> {"skeleton", "shots", "words", "sheet", "beats"}
      Breaks a saved video (a TikTok the user downloaded themselves) into shot lengths, spoken words,
      music beats and a contact sheet. The skeleton is a recipe with one clip scene per shot
      (`clip` null, to be filled with the user's game clips) in the same shape recipe.py validates.
  styles(out_dir) -> [{"name", "about", "still"}]
      One rendered still per style, cached by style + promo sources hash.

Fridae never downloads from a link: a URL is refused with a note to save the video file first.
"""
import glob, json, os, re, subprocess, tempfile

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
STILL_AT = 1.0
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
    r = subprocess.run([engine.ffmpeg(), "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(BEAT_RATE),
                        "-c:a", "pcm_s16le", "-f", "wav", "-"], env=engine.tool_env(), capture_output=True)
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

def _styles_dir():
    return os.path.join(rbxpaths.KIT, "promo", "src", "styles")


def _about(name):
    with open(os.path.join(_styles_dir(), name + ".json"), encoding="utf-8") as f:
        return str(json.load(f).get("about", ""))


def styles(out_dir):
    """[{"name","about","still"}] for every style; each still is rendered once per promo-source hash."""
    from rbxpromo import render
    os.makedirs(out_dir, exist_ok=True)
    tag = engine.promo_hash()[:12]
    items = []
    for name in STYLES:
        still = os.path.join(out_dir, "%s-%s.jpg" % (name, tag))
        if not os.path.isfile(still):
            recipe = {"version": 1, "size": [1080, 1920], "fps": 30, "style": name,
                      "scenes": [{"title": "YOUR GAME", "duration": 2.0}]}
            with tempfile.TemporaryDirectory() as tmp:
                rp = os.path.join(tmp, "recipe.json")
                rbxpaths.write_json(rp, recipe)
                render.render_still(rp, still, STILL_AT)
            for old in glob.glob(os.path.join(glob.escape(out_dir), glob.escape(name) + "-*.jpg")):
                if old != still and re.fullmatch(re.escape(name) + r"-[0-9a-f]{12}\.jpg", os.path.basename(old)):
                    os.remove(old)
        items.append({"name": name, "about": _about(name), "still": still})
    return items
