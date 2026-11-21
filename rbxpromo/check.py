"""Self-check of a rendered promo: platform safe zones, hook, loudness and text contrast.

  check(video, layout, recipe) -> {"passed": bool, "problems": [str], "notes": [str]}

`layout` is layout.json (a list of {"from","to","x","y","w","h","kind"}: seconds and pixels of the
1080x1920 frame, kind caption/title/end). It carries no colours, so the text colour and outline of each box
come from the recipe's style (promo/src/styles/<style>.json for titles and the end card, captions/presets.ts
for captions). Nothing is changed; this only reports. `video=None` skips the checks that read the file.

Loudness is measured here (the bundled ffmpeg has no ebur128 and no float PCM encoder, so the audio comes out as 24-bit WAV on a pipe): ITU-R BS.1770-4 K-weighting at 48 kHz,
400 ms blocks with 75 % overlap, -70 LUFS absolute gate and -10 LU relative gate. Contrast compares the
median luminance of the rendered frame inside a box (the text is a minority of those pixels, and boxes
drawn on a container such as the glass pill or ticket label see the container) with the text colour.
"""
import json
import math
import os
import re
import subprocess

import rbxpaths
from rbxpromo import engine
from rbxpromo.recipe import recipe_duration, scene_duration

# Fractions (x0, y0, x1, y1) of the 1080x1920 frame that platform UI covers; text must stay out of them.
SAFE_ZONES = {
    "tiktok": [(0, 0, 1, 0.09), (0.86, 0.35, 1, 0.86), (0, 0.80, 1, 1)],
    "reels": [(0, 0, 1, 0.08), (0.86, 0.45, 1, 0.88), (0, 0.82, 1, 1)],
    "shorts": [(0, 0, 1, 0.08), (0.86, 0.40, 1, 0.88), (0, 0.83, 1, 1)],
}
ZONE_NAMES = ("top bar", "right rail", "bottom caption area")   # order of each platform's zones
OVERLAP_LIMIT = 0.02         # share of a box's area that may sit in a zone
HOOK_SECONDS = 1.0
SHORTS_MAX = 60.0
LOUDNESS_RANGE = (-16.0, -12.0)
MIN_CONTRAST = 3.0           # WCAG ratio for large text
MAX_CONTRAST_BOXES = 24      # frames read for contrast: a handful, spread over the video
FRAME = (1080, 1920)

_PLATFORM_LABEL = {"tiktok": "TikTok", "reels": "Reels", "shorts": "Shorts"}
_KIND_LABEL = {"caption": "caption", "title": "title", "end": "end card text"}


def _t(x):
    return "%.1f s" % x


# ---------------------------------------------------------------- safe zones

def _overlap_share(box, zone):
    x0, y0, x1, y1 = zone[0] * FRAME[0], zone[1] * FRAME[1], zone[2] * FRAME[0], zone[3] * FRAME[1]
    w = min(box["x"] + box["w"], x1) - max(box["x"], x0)
    h = min(box["y"] + box["h"], y1) - max(box["y"], y0)
    area = box["w"] * box["h"]
    return (w * h / area) if w > 0 and h > 0 and area > 0 else 0.0


def _safe_zone_problems(layout):
    hits = {}    # (platform, zone, kind) -> [times of the boxes that hit it], in time order
    for box in sorted(layout, key=lambda b: b["from"]):
        for platform, zones in SAFE_ZONES.items():
            for name, zone in zip(ZONE_NAMES, zones):
                if _overlap_share(box, zone) > OVERLAP_LIMIT:
                    hits.setdefault((platform, name, box["kind"]), []).append(box["from"])
    out = []
    for (platform, name, kind), times in sorted(hits.items(), key=lambda kv: kv[1][0]):
        more = " (and %d more times)" % (len(times) - 1) if len(times) > 1 else ""
        out.append("%s at %s%s sits in the %s %s, where the app's buttons cover it."
                   % (_KIND_LABEL.get(kind, kind), _t(times[0]), more, _PLATFORM_LABEL[platform], name))
    return out


# ---------------------------------------------------------------- hook

def _hook_problems(layout, recipe):
    cuts, at = [], 0.0
    for s in recipe.get("scenes", []):
        at += scene_duration(s)
        cuts.append(at)
    cuts = cuts[:-1]       # the end of the last scene is not a cut
    starts = [b["from"] for b in layout if b["kind"] in ("title", "caption")]
    first = min(cuts + starts) if cuts + starts else None
    if first is not None and first <= HOOK_SECONDS + 1e-6:
        return []
    when = "the first one comes at %s" % _t(first) if first is not None else "there is none at all"
    return ["No hook: nothing cuts, shows a title or starts a caption within the first %.0f s (%s)." % (HOOK_SECONDS, when)]


# ---------------------------------------------------------------- loudness

# BS.1770-4 K-weighting at 48 kHz: a high shelf, then a high-pass.
_SHELF = ([1.53512485958697, -2.69169618940638, 1.19839281085285], [1.0, -1.69065929318241, 0.73248077421585])
_HIPASS = ([1.0, -2.0, 1.0], [1.0, -1.99004745483398, 0.99007225036621])
_RATE = 48000
_IR_LEN = 1 << 14     # the poles sit at radius 0.99, so the response is gone long before this


def _impulse_response(n):
    import numpy as np
    x = np.zeros(n)
    x[0] = 1.0
    for b, a in (_SHELF, _HIPASS):
        y = np.zeros(n)
        for i in range(n):
            acc = b[0] * x[i] + (b[1] * x[i - 1] if i > 0 else 0) + (b[2] * x[i - 2] if i > 1 else 0)
            acc -= (a[1] * y[i - 1] if i > 0 else 0) + (a[2] * y[i - 2] if i > 1 else 0)
            y[i] = acc
        x = y
    return x


_IR = None


def _k_weight(x):
    """The K-weighted signal (numpy has no IIR filter and a Python loop over minutes of audio is slow, so the
    cascade is applied as a convolution with its impulse response, by FFT overlap-add)."""
    import numpy as np
    global _IR
    if _IR is None:
        _IR = _impulse_response(_IR_LEN)
    n_fft = 1 << 18
    step = n_fft - _IR_LEN
    h = np.fft.rfft(_IR, n_fft)
    out = np.zeros(len(x) + _IR_LEN)
    for i in range(0, len(x), step):
        seg = x[i:i + step]
        out[i:i + len(seg) + _IR_LEN - 1] += np.fft.irfft(np.fft.rfft(seg, n_fft) * h, n_fft)[:len(seg) + _IR_LEN - 1]
    return out[:len(x)]


def integrated_loudness(samples, rate=_RATE):
    """LUFS of float samples shaped (frames, channels), -inf when nothing is above the gate.
    Channels count with weight 1 each (mono and stereo; no surround weights)."""
    import numpy as np
    if rate != _RATE:
        raise ValueError("loudness is only implemented for %d Hz" % _RATE)
    block, hop = int(0.4 * rate), int(0.1 * rate)
    x = np.asarray(samples, dtype=np.float64)
    if x.ndim == 1:
        x = x[:, None]
    if len(x) < block:
        x = np.vstack([x, np.zeros((block - len(x), x.shape[1]))])     # a short clip is one padded block
    power = sum(_k_weight(np.ascontiguousarray(x[:, c])) ** 2 for c in range(x.shape[1]))
    csum = np.concatenate([[0.0], np.cumsum(power)])
    starts = np.arange(0, len(power) - block + 1, hop)
    z = (csum[starts + block] - csum[starts]) / block
    with np.errstate(divide="ignore"):
        lk = -0.691 + 10 * np.log10(z)
    z = z[lk > -70.0]
    if not len(z):
        return float("-inf")
    rel = -0.691 + 10 * np.log10(z.mean()) - 10.0
    z = z[-0.691 + 10 * np.log10(z) > rel]
    return float(-0.691 + 10 * np.log10(z.mean())) if len(z) else float("-inf")


def _decode_audio(video):
    """(float samples in -1..1 shaped (frames, channels) at 48 kHz, or None when the file has no audio track)."""
    import numpy as np
    r = subprocess.run([engine.ffmpeg(), "-nostdin", "-v", "error", "-i", video, "-vn", "-ar", str(_RATE),
                        "-c:a", "pcm_s24le", "-f", "wav", "-"],
                       env=engine.tool_env(), stdin=subprocess.DEVNULL, capture_output=True)
    err = r.stderr.decode("utf-8", "replace").strip()
    if r.returncode != 0:
        if "does not contain any stream" in err or "Output file #0 does not contain" in err:
            return None
        raise RuntimeError("ffmpeg could not decode the audio of %s: %s" % (video, err[-300:]))
    data = r.stdout
    if data[:4] != b"RIFF":
        raise RuntimeError("ffmpeg did not return a WAV stream for " + video)
    channels, pos = None, 12
    while pos + 8 <= len(data):     # a piped WAV has placeholder sizes on 'data', so stop there
        cid, size = data[pos:pos + 4], int.from_bytes(data[pos + 4:pos + 8], "little")
        if cid == b"fmt ":
            channels = int.from_bytes(data[pos + 10:pos + 12], "little")
        elif cid == b"data":
            pcm = data[pos + 8:]
            if not channels:
                break
            n = len(pcm) // 3 // channels * channels        # whole frames of 24-bit samples
            b = np.frombuffer(pcm[:n * 3], dtype=np.uint8).reshape(-1, 3).astype(np.int32)
            x = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
            x = np.where(x >= 1 << 23, x - (1 << 24), x)
            return (x / float(1 << 23)).reshape(-1, channels)
        pos += 8 + size + (size & 1)
    raise RuntimeError("could not find the audio data in the WAV ffmpeg returned for " + video)


def measure_loudness(video):
    """Integrated LUFS of the video's audio, or None when it has no audio track."""
    samples = _decode_audio(video)
    return None if samples is None or not len(samples) else integrated_loudness(samples)


def _loudness_result(video):
    lufs = measure_loudness(video)
    if lufs is None:
        return [], ["The video has no audio track, so loudness was not checked."]
    lo, hi = LOUDNESS_RANGE
    if lufs == float("-inf"):
        # a promo with no music or voice renders silent on purpose: say so, do not fail it
        return [], ["The audio is silent (below -70 LUFS), so loudness was not checked."]
    if lo <= lufs <= hi:
        return [], []
    # plain words first; the measured value keeps one decimal and an ASCII minus, like the guide quotes it
    return ["%s: %.1f LUFS, aim for %d to %d" % ("Too quiet" if lufs < lo else "Too loud", lufs, lo, hi)], []


# ---------------------------------------------------------------- contrast

def _hex(c):
    m = re.fullmatch(r"#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})", str(c or "").strip())
    if not m:
        return None
    h = m.group(1)
    h = "".join(ch * 2 for ch in h) if len(h) == 3 else h
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))     # RGB


def _lin(v):
    v = v / 255.0
    return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4


def _luminance(rgb):
    return 0.2126 * _lin(rgb[0]) + 0.7152 * _lin(rgb[1]) + 0.0722 * _lin(rgb[2])


def contrast_ratio(a, b):
    """WCAG contrast ratio of two RGB tuples or two relative luminances."""
    la = a if isinstance(a, float) else _luminance(a)
    lb = b if isinstance(b, float) else _luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def _src_dir():
    """promo/src of the installed renderer copy (what actually renders), else the program's own promo/src."""
    installed = os.path.join(engine.engine_dir(), "renderer", "src")
    return installed if os.path.isdir(installed) else os.path.join(rbxpaths.KIT, "promo", "src")


def _style_json(name):
    path = os.path.join(_src_dir(), "styles", "%s.json" % name)
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _caption_presets():
    """{name: {"color", "stroke", "strokeColor"}} read from promo/src/captions/presets.ts."""
    with open(os.path.join(_src_dir(), "captions", "presets.ts"), encoding="utf-8") as f:
        src = f.read()
    out = {}
    for block in src.split("preset({")[1:]:
        block = block.split("}),")[0]
        name = re.search(r"\bname:\s*'([\w-]+)'", block)
        colour = re.search(r"\bcolor:\s*'(#[0-9A-Fa-f]{3,6})'", block)
        if name and colour:
            stroke = re.search(r"\bstroke:\s*([\d.]+)", block)
            sc = re.search(r"\bstrokeColor:\s*'(#[0-9A-Fa-f]{3,6})'", block)
            out[name.group(1)] = {"color": colour.group(1), "stroke": float(stroke.group(1)) if stroke else 0.0,
                                  "strokeColor": sc.group(1) if sc else None}
    if not out:
        raise ValueError("no caption presets found in presets.ts")
    return out


def _scene_style(recipe, t):
    at = 0.0
    for s in recipe.get("scenes", []):
        at += scene_duration(s)
        if t < at:
            return s.get("style") or recipe["style"]
    return recipe["style"]


def _text_looks(layout, recipe, notes):
    """[(box, text colour, outline colour or None)] for every box whose colours can be worked out;
    a caption whose preset cannot be found adds a note (once per preset) instead of vanishing."""
    style_cache, presets = {}, None

    def style(name):
        if name not in style_cache:
            style_cache[name] = _style_json(name)
        return style_cache[name]

    end_boxes = sorted((b for b in layout if b["kind"] == "end"), key=lambda b: (b["from"], b["y"]))
    ec = recipe.get("end_card") or {}
    end_kinds = [k for k in ("title", "cta") if ec.get(k)]
    out = []
    for b in layout:
        mid = (b["from"] + b["to"]) / 2
        if b["kind"] == "caption":
            if presets is None:
                presets = _caption_presets()
            name = (recipe.get("captions") or {}).get("style") or style(recipe["style"])["captions"]
            p = presets.get(name)
            if p:
                out.append((b, _hex(p["color"]), _hex(p["strokeColor"]) if p["stroke"] > 0 else None))
            else:
                msg = "Contrast not checked for captions (first at %s): style preset '%s' not found." % (_t(b["from"]), name)
                if not any("preset '%s' not found" % name in n for n in notes):
                    notes.append(msg)
        elif b["kind"] == "title":
            t = style(_scene_style(recipe, mid))["title"]
            out.append((b, _hex(t["color"]), _hex(t["strokeColor"]) if t.get("stroke", 0) > 0 else None))
        elif b["kind"] == "end":
            st = style(recipe["style"])
            i = end_boxes.index(b)
            kind = end_kinds[i] if i < len(end_kinds) else "title"
            if kind == "cta":
                out.append((b, _hex(st["end"]["ctaColor"]), None))
            else:
                t = st["title"]
                out.append((b, _hex(t["color"]), _hex(t["strokeColor"]) if t.get("stroke", 0) > 0 else None))
    return [o for o in out if o[1]]


def _box_luminance(cap, box, t):
    """Median relative luminance of the rendered frame inside `box` at second `t`, or None if unreadable."""
    import cv2
    import numpy as np
    cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000.0)
    ok, img = cap.read()
    if not ok:
        return None
    h, w = img.shape[:2]
    sx, sy = w / FRAME[0], h / FRAME[1]
    x0, x1 = int(max(0, box["x"] * sx)), int(min(w, (box["x"] + box["w"]) * sx))
    y0, y1 = int(max(0, box["y"] * sy)), int(min(h, (box["y"] + box["h"]) * sy))
    if x1 <= x0 or y1 <= y0:
        return None
    px = img[y0:y1, x0:x1].reshape(-1, 3)[:, ::-1].astype(np.float64) / 255.0       # RGB
    lin = np.where(px <= 0.03928, px / 12.92, ((px + 0.055) / 1.055) ** 2.4)
    return float(np.median(lin @ np.array([0.2126, 0.7152, 0.0722])))


def _contrast_problems(video, layout, recipe):
    notes = []
    try:
        looks = _text_looks(layout, recipe, notes)
    except (OSError, ValueError, KeyError) as e:
        return [], ["Text contrast was not checked: could not read the style files (%s)." % e]
    if not looks:
        return [], notes
    if len(looks) > MAX_CONTRAST_BOXES:
        looks = [looks[int(i * len(looks) / MAX_CONTRAST_BOXES)] for i in range(MAX_CONTRAST_BOXES)]
    import rbxvideo
    try:
        cap = rbxvideo.open_capture(video)
    except ValueError as e:
        raise RuntimeError("could not check contrast: %s" % e)
    problems, unread = [], []
    try:
        for box, colour, outline in looks:
            t = (box["from"] + box["to"]) / 2
            lum = _box_luminance(cap, box, t)
            if lum is None:
                unread.append(t)
                continue
            ratio = contrast_ratio(colour, lum)
            if outline:     # an outline carries the text where the picture would not
                ratio = max(ratio, contrast_ratio(colour, outline))
            if ratio < MIN_CONTRAST:
                problems.append("The %s at %s is hard to read: contrast %.1f:1 against the picture behind it (needs %.0f:1)."
                                % (_KIND_LABEL.get(box["kind"], box["kind"]), _t(t), ratio, MIN_CONTRAST))
    finally:
        cap.release()
    if unread:
        notes.append("Contrast not checked for %d text box%s (first at %s): no readable picture there."
                     % (len(unread), "" if len(unread) == 1 else "es", _t(unread[0])))
    return problems, notes


# ---------------------------------------------------------------- the check

def check(video, layout, recipe):
    problems = _safe_zone_problems(layout) + _hook_problems(layout, recipe)
    notes = []
    if recipe_duration(recipe) > SHORTS_MAX:
        notes.append("The video is %.0f s; YouTube Shorts only takes up to %.0f s." % (recipe_duration(recipe), SHORTS_MAX))
    if video is not None:
        if not os.path.isfile(video):
            raise FileNotFoundError("the video to check does not exist: " + str(video))
        p, n = _loudness_result(video)
        problems += p
        notes += n
        p, n = _contrast_problems(video, layout, recipe)
        problems += p
        notes += n
    return {"passed": not problems, "problems": problems, "notes": notes}
