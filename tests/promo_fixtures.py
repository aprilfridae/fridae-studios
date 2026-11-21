"""Synthetic clips for promo tests.

Remotion's bundled ffmpeg is a stripped build (no testsrc2, drawbox, select, setpts), so
frames are drawn with numpy/OpenCV and piped to the engine's ffmpeg as PNGs; only `sine`
(lavfi) is used from ffmpeg itself.
"""
import os
import subprocess

from rbxpromo import engine

_COLOURS = [(0, 0, 255), (255, 0, 0), (0, 200, 0), (0, 220, 220)]  # BGR


def _ffmpeg(args, stdin=None):
    r = subprocess.run([engine.ffmpeg(), "-y", "-hide_banner", "-loglevel", "error"] + args,
                       input=stdin, env=engine.tool_env(), capture_output=True)
    if r.returncode != 0:
        raise RuntimeError("ffmpeg failed: " + r.stderr.decode("utf-8", "replace")[-800:])


def _frame(i, t, size, cuts):
    import cv2, numpy as np
    w, h = size
    seg = sum(1 for c in cuts if t >= c)
    if seg % 2 == 1:
        img = np.empty((h, w, 3), np.uint8)
        img[:] = _COLOURS[(seg // 2) % len(_COLOURS)]
        return img
    x = np.linspace(0, 255, w, dtype=np.uint8)
    img = np.empty((h, w, 3), np.uint8)
    img[:, :, 0] = x[None, :]
    img[:, :, 1] = np.linspace(40, 200, h, dtype=np.uint8)[:, None]
    img[:, :, 2] = 90
    bw = max(8, w // 8)
    # the box moves faster in the second half of each second so motion varies over time
    pos = (t * w * (0.2 + 0.8 * (t % 1.0))) % max(1, w - bw)
    cv2.rectangle(img, (int(pos), h // 3), (int(pos) + bw, h // 3 + bw), (255, 255, 255), -1)
    return img


def _png(img):
    import cv2
    return cv2.imencode(".png", img)[1].tobytes()


def make_clip(path, seconds, size=(1280, 720), fps=30, audio=True, cuts=()):
    """Write an H.264 (+AAC sine) clip. `cuts` are times where the picture switches to a
    flat colour and back, giving hard scene changes."""
    cuts = sorted(cuts)
    n = int(round(seconds * fps))
    data = b"".join(_png(_frame(i, i / fps, size, cuts)) for i in range(n))
    args = ["-f", "image2pipe", "-framerate", str(fps), "-c:v", "png", "-i", "-"]
    if audio:
        args += ["-f", "lavfi", "-i", "sine=frequency=440:duration=%s" % seconds]
    args += ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]
    args += ["-c:a", "aac", "-shortest"] if audio else ["-an"]
    _ffmpeg(args + [path], stdin=data)
    return path


def make_vfr(path, seconds, size=(640, 360)):
    """A silent variable-frame-rate clip (like OBS output): frame durations alternate
    between 1/60 s and 1/15 s, via the concat demuxer and -fps_mode vfr."""
    import cv2
    folder = path + ".frames"
    os.makedirs(folder, exist_ok=True)
    lines, t, i = ["ffconcat version 1.0"], 0.0, 0
    while t < seconds:
        d = 1 / 60 if i % 2 == 0 else 1 / 15
        fp = os.path.join(folder, "f%05d.png" % i)
        cv2.imwrite(fp, _frame(i, t, size, ()))
        lines += ["file '%s'" % fp.replace("'", "'\\''"), "duration %.6f" % d]
        t, i = t + d, i + 1
    lines.append("file '%s'" % fp.replace("'", "'\\''"))
    lst = os.path.join(folder, "list.txt")
    with open(lst, "w") as f:
        f.write("\n".join(lines) + "\n")
    _ffmpeg(["-f", "concat", "-safe", "0", "-i", lst, "-fps_mode", "vfr", "-c:v", "libx264",
             "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-an", path])
    return path
