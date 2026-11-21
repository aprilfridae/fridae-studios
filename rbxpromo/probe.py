"""Probe source clips for the promo maker: size, fps, VFR, audio, cuts, loud and busy moments.

Remotion's bundled ffmpeg is a stripped build (no `select`/scene filter, no astats/ebur128,
no raw PCM muxer), so instead of ffmpeg filters this module streams the data out and
measures it in Python, one small chunk at a time:
  - ffprobe JSON for the facts (incl. VFR detection),
  - ffmpeg -> WAV on a pipe (mono, 8 kHz) for per-second RMS,
  - OpenCV, one frame at a time, for cuts (every frame) and motion (4 fps samples).
Nothing reads a whole clip into memory. Every subprocess gets env=engine.tool_env().
"""
import json
import math
import os
import struct
import subprocess

from rbxpromo import engine

# Cut detection runs on the mean per-pixel change (0..1) between sampled frames shrunk to
# 64x36 BGR. That is NOT ffmpeg's scene score; it is calibrated on fixtures: a low-contrast
# hard cut is ~0.10, ordinary motion is well under 0.03. A cut needs a change above
# CUT_THRESHOLD that is also CUT_SPIKE times the recent average, so a steady fast pan
# (high but constant change) is not reported.
CUT_THRESHOLD = 0.06
CUT_SPIKE = 4.0
CUT_FPS = 12              # frames per second looked at for cuts (the rest are only grabbed)
MOTION_FPS = 4
WINDOW = 1.0
TOP = 10
AUDIO_RATE = 8000


def _run_json(args):
    r = subprocess.run([engine.ffprobe(), "-v", "error", "-of", "json"] + args,
                       env=engine.tool_env(), stdin=subprocess.DEVNULL, capture_output=True)
    if r.returncode != 0:
        raise ValueError("ffprobe could not read the clip: " + r.stderr.decode("utf-8", "replace").strip()[-300:])
    return json.loads(r.stdout.decode("utf-8", "replace") or "{}")


def _rate(s):
    try:
        a, b = str(s).split("/")
        return float(a) / float(b) if float(b) else 0.0
    except (ValueError, ZeroDivisionError):
        return 0.0


def _is_vfr(path, stream, warnings=None):
    r_fps, avg = _rate(stream.get("r_frame_rate")), _rate(stream.get("avg_frame_rate"))
    if r_fps and avg and abs(r_fps - avg) / max(r_fps, avg) > 0.02:
        return True
    # Packet timestamps: a constant-rate clip has near-equal gaps between frames.
    try:
        pk = _run_json(["-select_streams", "v:0", "-show_entries", "packet=pts_time", path]).get("packets", [])
    except ValueError as e:
        if warnings is not None:
            warnings.append("variable frame rate could not be checked: %s" % e)
        return False
    ts = sorted(float(p["pts_time"]) for p in pk if p.get("pts_time") not in (None, "N/A"))
    gaps = [b - a for a, b in zip(ts, ts[1:]) if b > a]
    if len(gaps) < 8:
        return False
    med = sorted(gaps)[len(gaps) // 2]
    off = sum(1 for g in gaps if abs(g - med) > 0.25 * med)
    return off > 0.05 * len(gaps)


def probe_file(path, warnings=None):
    """Facts about one clip: duration, width, height, fps, vfr, has_audio.
    Problems that do not stop probing are appended to `warnings` when a list is given."""
    data = _run_json(["-show_streams", "-show_format", path])
    streams = data.get("streams", [])
    video = next((s for s in streams if s.get("codec_type") == "video"), None)
    if video is None:
        raise ValueError("no video stream in " + path)
    dur = None
    for src in (data.get("format", {}), video):
        try:
            dur = float(src.get("duration"))
            break
        except (TypeError, ValueError):
            pass
    fps = _rate(video.get("avg_frame_rate")) or _rate(video.get("r_frame_rate"))
    return {
        "duration": dur if dur is not None else 0.0,
        "width": int(video.get("width", 0)),
        "height": int(video.get("height", 0)),
        "fps": round(fps, 3),
        "vfr": _is_vfr(path, video, warnings),
        "has_audio": any(s.get("codec_type") == "audio" for s in streams),
    }


# ---------------------------------------------------------------- audio

def _loud_windows(path, warnings):
    """Per-second RMS in dB (full scale), streamed from ffmpeg as WAV on a pipe."""
    import tempfile
    err = tempfile.TemporaryFile()
    proc = subprocess.Popen(
        [engine.ffmpeg(), "-nostdin", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(AUDIO_RATE),
         "-c:a", "pcm_s16le", "-f", "wav", "-"],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=err, env=engine.tool_env())
    out, size, finished = [], AUDIO_RATE * 2, False
    try:
        head = proc.stdout.read(12)                     # RIFF....WAVE
        data_ok = False
        while len(head) == 12:
            hdr = proc.stdout.read(8)                   # chunk id + size
            if len(hdr) < 8:
                break
            if hdr[:4] == b"data":
                data_ok = True
                break
            proc.stdout.read(struct.unpack("<I", hdr[4:])[0])
        if data_ok:
            i = 0
            while True:
                buf = proc.stdout.read(size)
                while buf and len(buf) < size:
                    more = proc.stdout.read(size - len(buf))
                    if not more:
                        break
                    buf += more
                if len(buf) < 2:
                    break
                n = len(buf) // 2
                s = struct.unpack("<%dh" % n, buf[:n * 2])
                rms = math.sqrt(sum(v * v for v in s) / n) / 32768.0
                db = 20 * math.log10(rms) if rms > 0 else -90.0
                out.append([round(i * WINDOW, 3), round(i * WINDOW + n / AUDIO_RATE, 3), round(max(db, -90.0), 2)])
                i += 1
        finished = True
    finally:
        proc.stdout.close()
        if not finished:
            proc.kill()
        code = proc.wait()
        err.seek(0)
        tail = err.read().decode("utf-8", "replace").strip()[-300:]
        err.close()
    if code != 0 or not out:
        warnings.append("audio could not be decoded: %s" % (tail or "ffmpeg exited with code %s" % code))
        return []
    return sorted(out, key=lambda w: -w[2])[:TOP]


# ---------------------------------------------------------------- video

def _scan_video(path, on_progress=None, duration=0.0):
    """One pass with OpenCV: cut times and busiest windows. Returns (cuts, motion, last_time).

    Every frame is grabbed (decoded) but only ~CUT_FPS per second are converted and
    compared, which is where most of the time goes on 1080p60 clips."""
    import cv2
    import numpy as np
    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise ValueError("could not open video: " + path)
    step = max(1, int(round((cap.get(cv2.CAP_PROP_FPS) or 30) / CUT_FPS)))
    cuts, recent, prev_small = [], [], None
    prev_sample, next_sample, diffs = None, 0.0, []
    idx, last_t, seen = 0, 0.0, 0
    try:
        while cap.grab():
            idx += 1
            if (idx - 1) % step:
                continue
            ok, img = cap.retrieve()
            if not ok:
                continue
            seen += 1
            t = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000.0
            last_t = max(last_t, t)
            small = cv2.resize(img, (64, 36), interpolation=cv2.INTER_AREA).astype(np.int16)
            if prev_small is not None:
                d = float(np.abs(small - prev_small).mean()) / 255.0
                avg = sum(recent) / len(recent) if recent else 0.0
                if len(recent) >= 2 and d > CUT_THRESHOLD and d > CUT_SPIKE * max(avg, 1e-3):
                    cuts.append(round(t, 3))
                    recent = []
                else:
                    recent = (recent + [d])[-8:]
            prev_small = small
            if t >= next_sample:
                g = cv2.cvtColor(cv2.resize(img, (160, 90), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2GRAY)
                if prev_sample is not None:
                    diffs.append((t, float(cv2.absdiff(g, prev_sample).mean()) / 255.0))
                prev_sample = g
                next_sample = t + 1.0 / MOTION_FPS
            if on_progress and duration and seen % 12 == 0:
                on_progress(min(1.0, t / duration))
    finally:
        cap.release()
    if not seen:
        raise ValueError("no frames could be decoded from " + path)
    wins = {}
    for t, d in diffs:
        wins.setdefault(int(t // WINDOW), []).append(d)
    motion = [[round(k * WINDOW, 3), round((k + 1) * WINDOW, 3), round(sum(v) / len(v), 5)]
              for k, v in wins.items()]
    return cuts, sorted(motion, key=lambda w: -w[2])[:TOP], last_t


def _sheet(path, out_dir, name):
    import rbxvideo
    try:
        res = rbxvideo.frames(path, count=12, out_dir=os.path.join(out_dir, name))
    except SystemExit as e:          # rbxvideo is CLI-first and exits on failure
        raise ValueError(str(e.code) if e.code else "could not make a contact sheet for " + path)
    return res["look_at"][0] if res.get("look_at") else None


def probe(paths, out_dir, on_progress=None):
    """Probe each clip: {path: {info, cuts, loud, motion, sheet, warnings}}.
    Sheets go under out_dir. `on_progress(fraction)` is called as clips are scanned (0..1)."""
    os.makedirs(out_dir, exist_ok=True)
    result, total = {}, max(1, len(paths))
    for n, path in enumerate(paths):
        warnings = []
        info = probe_file(path, warnings)

        def prog(f, n=n):
            if on_progress:
                on_progress((n + f) / total)
        cuts, motion, last_t = _scan_video(path, prog, info["duration"])
        if info["duration"] and info["duration"] - last_t > max(1.0, 0.05 * info["duration"]):
            warnings.append("only %.1f s of %.1f s could be read" % (last_t, info["duration"]))
        stem = "%02d-%s" % (n + 1, os.path.splitext(os.path.basename(path))[0])
        result[path] = {
            "info": info,
            "cuts": cuts,
            "loud": _loud_windows(path, warnings) if info["has_audio"] else [],
            "motion": motion,
            "sheet": _sheet(path, out_dir, stem),
            "warnings": warnings,
        }
        prog(1.0)
    return result
