"""Look at a video: metadata, and frames tiled into contact sheets.

  rbx.py video info clip.mp4
  rbx.py video frames clip.mp4                 overview: frames spread across the whole clip
  rbx.py video frames clip.mp4 --start 1.2 --end 2.0 --count 12     a stretch, frame by frame
"""
import json, os, sys

from rbxpaths import KIT, WORK


def imwrite(path, img, params=None):
    """cv2.imwrite that works for any path: on Windows cv2.imwrite only takes paths in the ANSI code page,
    so the image is encoded in memory and written with numpy. Raises OSError if it could not be written."""
    import cv2
    ext = os.path.splitext(path)[1] or ".png"
    ok, buf = cv2.imencode(ext, img, params or [])
    if not ok:
        raise OSError("could not encode the image for " + path)
    try:
        buf.tofile(path)
    except (OSError, ValueError) as e:
        raise OSError("could not write the image %s: %s" % (path, e))
    return path


def imread(path, flags=None):
    """cv2.imread that works for any path (see imwrite). Raises OSError if it could not be read."""
    import cv2
    import numpy as np
    try:
        data = np.fromfile(path, np.uint8)
    except (OSError, ValueError) as e:
        raise OSError("could not read the image %s: %s" % (path, e))
    img = cv2.imdecode(data, cv2.IMREAD_COLOR if flags is None else flags) if data.size else None
    if img is None:
        raise OSError("could not decode the image " + path)
    return img


def open_capture(path):
    """cv2.VideoCapture that never fails silently: ValueError naming the path when it does not open."""
    import cv2
    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        if os.path.isfile(path):
            raise ValueError("could not open the video %s: the file exists but OpenCV cannot read it (an unsupported "
                             "format, or on Windows letters in the path outside the system code page)" % path)
        raise ValueError("could not open the video %s: the file does not exist" % path)
    return cap


def _open(path):
    import cv2
    try:
        cap = open_capture(path)
    except ValueError as e:
        sys.exit(str(e))
    return cv2, cap


def info(path):
    cv2, cap = _open(path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 0
    n = int(cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0)
    fourcc = int(cap.get(cv2.CAP_PROP_FOURCC))
    out = {
        "file": path,
        "width": int(cap.get(cv2.CAP_PROP_FRAME_WIDTH)),
        "height": int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT)),
        "fps": round(fps, 3),
        "frames": n,
        "duration": round(n / fps, 3) if fps else None,
        "codec": "".join(chr((fourcc >> 8 * i) & 0xFF) for i in range(4)).strip(),
        "size_mb": round(os.path.getsize(path) / 1e6, 2),
    }
    cap.release()
    return out


def frames(path, start=None, end=None, count=12, cols=4, width=480, out_dir=None, sheet=True):
    cv2, cap = _open(path)
    meta = info(path)
    fps = meta["fps"] or 30
    dur = meta["duration"] or 0
    start = 0.0 if start is None else max(0.0, start)
    end = dur if end is None else min(end, dur)
    count = max(1, min(count, 48))
    times = [start + (end - start) * (i + 0.5) / count for i in range(count)] if count > 1 else [start]
    out_dir = out_dir or os.path.join(WORK, "video", os.path.splitext(os.path.basename(path))[0])
    os.makedirs(out_dir, exist_ok=True)
    tiles, saved = [], []
    for t in times:
        cap.set(cv2.CAP_PROP_POS_FRAMES, int(round(t * fps)))
        ok, img = cap.read()
        if not ok:
            continue
        h = int(img.shape[0] * width / img.shape[1])
        img = cv2.resize(img, (width, h), interpolation=cv2.INTER_AREA)
        cv2.rectangle(img, (0, 0), (118, 26), (0, 0, 0), -1)
        cv2.putText(img, "%.2fs" % t, (6, 20), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (255, 255, 255), 2)
        tiles.append(img)
        if not sheet:
            p = os.path.join(out_dir, "frame_%07.3f.png" % t)
            imwrite(p, img)
            saved.append(p)
    cap.release()
    if not tiles:
        sys.exit("no frames could be read")
    if sheet:
        import numpy as np
        rows = []
        for i in range(0, len(tiles), cols):
            row = tiles[i:i + cols]
            while len(row) < cols:
                row.append(np.zeros_like(tiles[0]))
            rows.append(np.hstack(row))
        p = os.path.join(out_dir, "sheet_%.2f-%.2f.png" % (start, end))
        imwrite(p, np.vstack(rows))
        saved.append(p)
    return {"look_at": saved, "times": [round(t, 3) for t in times], "video": meta}


def _cmd(a):
    if a.action == "info":
        print(json.dumps(info(a.path), indent=1))
    else:
        print(json.dumps(frames(a.path, a.start, a.end, a.count, a.cols, a.width, sheet=not a.separate), indent=1))


def register(sub):
    v = sub.add_parser("video", help="video metadata and frame contact sheets")
    v.add_argument("action", choices=["info", "frames"])
    v.add_argument("path")
    v.add_argument("--start", type=float)
    v.add_argument("--end", type=float)
    v.add_argument("--count", type=int, default=12)
    v.add_argument("--cols", type=int, default=4)
    v.add_argument("--width", type=int, default=480, help="width of each frame in the sheet")
    v.add_argument("--separate", action="store_true", help="write one PNG per frame instead of a sheet")
    v.set_defaults(fn=_cmd)
