"""VFX texture drawing for rbxkit: white-on-transparent particle textures and
flipbook sheets drawn with numpy, checked from their pixels, and staged into
Studio for a free local preview.

In a script run by `rbx.py tex`, everything here is on the `rbx` object:

    a = rbx.tex_edge(rbx.tex_blob(*rbx.tex_coords(256), radius=0.6))
    rbx.draw_texture("Puff", a, size=256)
    rbx.draw_flipbook("Burst", lambda t, size: ..., grid=4)
"""
import glob, hashlib, json, math, os, shutil, sys
import numpy as np
from PIL import Image, ImageDraw

from rbxpaths import KIT, WORK
SHEET = 1024
_written = []
_folder = "vfx"


# ------------------------------------------------------------ building blocks
def tex_coords(size):
    """x, y arrays from -1 to 1, x right and y up."""
    lin = np.linspace(-1.0, 1.0, size)
    x, y = np.meshgrid(lin, -lin)
    return x, y


def tex_polar(x, y):
    """r, angle: distance from the centre and the angle (radians)."""
    return np.hypot(x, y), np.arctan2(y, x)


def tex_ease(t, power=3.0):
    """Ease out: fast start, slow finish."""
    return 1.0 - (1.0 - np.clip(t, 0.0, 1.0)) ** power


def tex_edge(value, at=0.0, soft=0.008):
    """The hard edge of drawn matter: 0 below `at`, 1 above, blended over `soft`."""
    return np.clip((np.asarray(value, dtype=float) - at) / max(soft, 1e-6) * 0.5 + 0.5, 0.0, 1.0)


def _smooth(f):
    return f * f * (3.0 - 2.0 * f)


def tex_noise(size, scale=4, octaves=4, seed=0):
    """Smooth value noise in 0..1 that tiles."""
    rng = np.random.default_rng(seed)
    out = np.zeros((size, size))
    amp, total = 1.0, 0.0
    lin = np.arange(size) / size
    for o in range(octaves):
        n = max(int(round(scale * 2 ** o)), 1)
        grid = rng.random((n, n))
        u = lin * n
        i0 = np.floor(u).astype(int) % n
        i1 = (i0 + 1) % n
        f = _smooth(u - np.floor(u))
        rows = grid[i0][:, i0] * (1 - f)[None, :] + grid[i0][:, i1] * f[None, :]
        rows2 = grid[i1][:, i0] * (1 - f)[None, :] + grid[i1][:, i1] * f[None, :]
        out += amp * (rows * (1 - f)[:, None] + rows2 * f[:, None])
        total += amp
        amp *= 0.5
    out /= total
    lo, hi = out.min(), out.max()
    return (out - lo) / max(hi - lo, 1e-9)


def tex_sample(image, u, v):
    """`image` looked up at 0..1 coordinates with wrapping (bilinear)."""
    h, w = image.shape[:2]
    fu = (np.asarray(u) % 1.0) * w
    fv = (np.asarray(v) % 1.0) * h
    x0 = np.floor(fu).astype(int) % w
    y0 = np.floor(fv).astype(int) % h
    x1, y1 = (x0 + 1) % w, (y0 + 1) % h
    ax, ay = fu - np.floor(fu), fv - np.floor(fv)
    top = image[y0, x0] * (1 - ax) + image[y0, x1] * ax
    bot = image[y1, x0] * (1 - ax) + image[y1, x1] * ax
    return top * (1 - ay) + bot * ay


def tex_warp(x, y, amount=0.1, scale=3, seed=0, t=0.0):
    """Coordinates pushed around by noise, so what is drawn with them is organic."""
    size = x.shape[0]
    n1 = tex_noise(size, scale, 3, seed)
    n2 = tex_noise(size, scale, 3, seed + 101)
    u = (x + 1) / 2 + t * 0.37
    v = (1 - y) / 2 + t * 0.21
    return x + (tex_sample(n1, u, v) - 0.5) * 2 * amount, y + (tex_sample(n2, u, v) - 0.5) * 2 * amount


def tex_cells(size, cells=8, seed=0):
    """Voronoi `near, edge` (both 0..1) that tiles: detail inside a shape."""
    rng = np.random.default_rng(seed)
    pts = rng.random((cells, 2))
    lin = (np.arange(size) + 0.5) / size
    gx, gy = np.meshgrid(lin, lin)
    d1 = np.full((size, size), 9.0)
    d2 = np.full((size, size), 9.0)
    for px, py in pts:
        dx = np.abs(gx - px)
        dy = np.abs(gy - py)
        d = np.hypot(np.minimum(dx, 1 - dx), np.minimum(dy, 1 - dy))
        closer = d < d1
        d2 = np.where(closer, d1, np.minimum(d2, d))
        d1 = np.where(closer, d, d1)
    norm = max(d1.max(), 1e-9)
    return d1 / norm, np.clip((d2 - d1) / norm, 0, 1)


def tex_curve(points, samples=64, closed=False):
    """A smooth path (Catmull-Rom) through control points; returns (samples, 2)."""
    p = np.asarray(points, dtype=float)
    if closed:
        p = np.vstack([p[-1], p, p[0], p[1]])
    else:
        p = np.vstack([p[0], p, p[-1]])
    segs = len(p) - 3
    out = []
    for s in np.linspace(0, segs, samples, endpoint=not closed):
        i = min(int(s), segs - 1)
        u = s - i
        p0, p1, p2, p3 = p[i], p[i + 1], p[i + 2], p[i + 3]
        out.append(0.5 * ((2 * p1) + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u * u
                          + (-p0 + 3 * p1 - 3 * p2 + p3) * u ** 3))
    return np.array(out)


def _width_fn(width):
    """width: a number (tapers to a point at the tail), a list of widths spread
    along the path, or a function of position 0..1 along the path."""
    if callable(width):
        def fn(u):
            try:
                out = np.asarray(width(u), dtype=float)
                if out.shape == np.shape(u):
                    return out
            except Exception:
                pass
            return np.vectorize(lambda v: float(width(float(v))))(u)
        return fn
    if isinstance(width, (list, tuple, np.ndarray)):
        vals = np.asarray(width, dtype=float)
        knots = np.linspace(0, 1, len(vals))
        return lambda u: np.interp(u, knots, vals)
    return lambda u: width * (1.0 - 0.95 * u)


def tex_stroke(x, y, path, width=0.08, start=0.0, end=1.0):
    """A brush stroke along a path as a field (positive inside). `width` is a
    number (tapers to a point at the tail), a list of widths along the path,
    or a function of position 0..1. `start`/`end` in 0..1 draw the stroke on
    (raise end) or erase it from its head (raise start)."""
    path = np.asarray(path, dtype=float)
    wfn = _width_fn(width)
    start, end = float(start), float(end)
    field = np.full(x.shape, -9.0)
    n = len(path) - 1
    if end <= start or n < 1:
        return field
    for i in range(n):
        s0, s1 = i / n, (i + 1) / n
        if s1 <= start or s0 >= end:
            continue
        a, b = path[i], path[i + 1]
        ab = b - a
        L2 = float(ab @ ab) or 1e-12
        lo = max((start - s0) / (s1 - s0), 0.0)
        hi = min((end - s0) / (s1 - s0), 1.0)
        tt = np.clip(((x - a[0]) * ab[0] + (y - a[1]) * ab[1]) / L2, lo, hi)
        d = np.hypot(x - (a[0] + ab[0] * tt), y - (a[1] + ab[1] * tt))
        field = np.maximum(field, wfn(s0 + (s1 - s0) * tt) - d)
    return field


def tex_blob(x, y, radius=0.6, lumps=5, roughness=0.5, seed=0):
    """A lumpy, lopsided blob in one piece as a field (positive inside)."""
    rng = np.random.default_rng(seed)
    cx, cy = (rng.random(2) - 0.5) * 0.12 * roughness
    r, ang = tex_polar(x - cx, y - cy)
    edge = np.ones_like(r)
    for k in range(1, lumps + 1):
        amp = roughness * 0.32 * rng.random() / (k ** 0.7)
        edge += amp * np.sin(ang * (k + 1) + rng.random() * math.tau)
    return radius * edge - r


def ring(x, y, radius=0.7, width=0.05):
    """A ring as a field (positive inside the band)."""
    return width - np.abs(np.hypot(x, y) - radius)


def glow(x, y, radius=1.0, power=2.0):
    """Soft light: a smooth falloff from a hot core, 0..1 (not a field)."""
    return np.clip(1.0 - np.hypot(x, y) / radius, 0, 1) ** power


def flare(x, y, arms=4, length=0.95, thickness=0.035, core=0.22):
    """A symmetric star flare with thinning arms and a soft core, 0..1."""
    r, ang = tex_polar(x, y)
    a = np.abs(((ang / math.tau * arms + 0.5) % 1.0) - 0.5) * math.tau / arms
    across = r * np.sin(a)
    along = r * np.cos(a)
    arm = np.exp(-(across / (thickness * (1 - np.clip(along / length, 0, 1)) + 1e-4)) ** 2)
    arm *= np.clip(1 - along / length, 0, 1) ** 1.5
    return np.clip(arm + glow(x, y, core, 1.5), 0, 1)


# ------------------------------------------------------------------- writing
def _out_dir(folder):
    f = folder or _folder
    path = f if os.path.isabs(f) else os.path.join(WORK, f)
    os.makedirs(path, exist_ok=True)
    return path


def _to_rgba(result, size, mode):
    if callable(result):
        result = result(size)
    value = 1.0
    if isinstance(result, tuple):
        result, value = result
    arr = np.asarray(result, dtype=float)
    if arr.ndim == 3:
        rgb = arr[..., :3]
        alpha = arr[..., 3] if arr.shape[2] == 4 else np.ones(arr.shape[:2])
    else:
        alpha = np.clip(arr, 0, 1)
        v = np.broadcast_to(np.clip(np.asarray(value, dtype=float), 0, 1), alpha.shape)
        rgb = np.stack([v, v, v], axis=-1)
    if mode == "additive":
        rgb = rgb * alpha[..., None]
        alpha = np.ones_like(alpha)
    rgba = np.dstack([np.clip(rgb, 0, 1), np.clip(alpha, 0, 1)])
    return (rgba * 255 + 0.5).astype(np.uint8)


def draw_texture(name, image, size=512, mode="alpha", folder=None):
    """Write one texture as <name>.png from an array or a function of `size`."""
    px = _to_rgba(image, size, mode)
    path = os.path.join(_out_dir(folder), name + ".png")
    Image.fromarray(px, "RGBA").save(path)
    _written.append(path)
    return path


def draw_flipbook(name, frame, grid=4, loop=False, fps=None, mode="alpha", padding=4, folder=None):
    """Call frame(t, size) once per cell and pack a 1024 sheet as
    <name>.flipbook.png with a .flipbook.json beside it. t runs 0..1 (stopping
    just below 1 when loop=True)."""
    if grid not in (2, 4, 8):
        raise ValueError("grid must be 2, 4 or 8")
    cell = SHEET // grid
    inner = cell - padding * 2
    n = grid * grid
    sheet = np.zeros((SHEET, SHEET, 4), dtype=np.uint8)
    for i in range(n):
        t = i / n if loop else (i / (n - 1) if n > 1 else 0.0)
        px = _to_rgba(frame(t, inner), inner, mode)
        if px.shape[0] != inner:
            px = np.asarray(Image.fromarray(px, "RGBA").resize((inner, inner), Image.LANCZOS))
        r, c = divmod(i, grid)
        sheet[r * cell + padding:r * cell + padding + inner, c * cell + padding:c * cell + padding + inner] = px
    out = _out_dir(folder)
    path = os.path.join(out, name + ".flipbook.png")
    Image.fromarray(sheet, "RGBA").save(path)
    with open(os.path.join(out, name + ".flipbook.json"), "w", encoding="utf-8") as f:
        json.dump({"grid": grid, "loop": loop, "fps": fps, "padding": padding, "mode": mode}, f)
    _written.append(path)
    return path


# ------------------------------------------------------------------ checking
def _ink(px):
    """Per-pixel ink 0..1: alpha, or brightness for an opaque (additive) image."""
    a = px[..., 3].astype(float) / 255
    if _is_additive(px):
        return px[..., :3].max(axis=-1).astype(float) / 255 * a
    return a


def _is_additive(px):
    """Opaque (or nearly all opaque) image drawn on black."""
    return (px[..., 3] > 250).mean() > 0.85


def _guess_grid(ink):
    for g in (8, 4, 2):
        cell = ink.shape[0] // g
        lines = [ink[k * cell - 1:k * cell + 1, :].mean() + ink[:, k * cell - 1:k * cell + 1].mean() for k in range(1, g)]
        if max(lines) < 0.004:
            return g
    return None


def check(path, grid=None):
    """Read a texture or flipbook from its pixels. Returns a report dict."""
    img = Image.open(path).convert("RGBA")
    px = np.asarray(img)
    ink = _ink(px)
    rep = {"file": path, "size": "%dx%d" % img.size, "problems": [], "notes": []}
    additive = _is_additive(px)
    rep["mode"] = "additive (on black)" if additive else "alpha"
    meta_path = path[:-4] + ".json"
    meta = json.load(open(meta_path, encoding="utf-8")) if os.path.exists(meta_path) else {}
    is_book = path.endswith(".flipbook.png") or grid
    if is_book:
        if img.size != (SHEET, SHEET):
            rep["problems"].append("flipbook sheet must be 1024x1024, this is %dx%d" % img.size)
        g = grid if isinstance(grid, int) else meta.get("grid") or _guess_grid(ink)
        if not g:
            rep["problems"].append("could not find a 2x2, 4x4 or 8x8 grid from the gutters")
            return rep
        cell = ink.shape[0] // g
        cover, prev = [], None
        for i in range(g * g):
            r, c = divmod(i, g)
            block = ink[r * cell:(r + 1) * cell, c * cell:(c + 1) * cell]
            cover.append(round(float((block > 0.05).mean()), 3))
            edge = max(block[0].max(), block[-1].max(), block[:, 0].max(), block[:, -1].max())
            if edge > 0.08:
                rep["problems"].append("frame %d is cut off at its cell edge (bleeds into a neighbour)" % (i + 1))
            if prev is not None and block.max() > 0.05 and np.abs(block - prev).mean() < 0.03 * max(float(block.mean()), 1e-4):
                rep["notes"].append("frame %d holds (same as frame %d)" % (i + 1, i))
            prev = block
        empties = [i + 1 for i, v in enumerate(cover) if v == 0]
        lead = [i for i in empties if i <= 2]
        if lead:
            rep["problems"].append("frame %s is empty: the first frame should read on its own" % lead)
        mid = [i for i in empties if any(cover[j] > 0 for j in range(i, len(cover)))]
        if [i for i in mid if i not in lead]:
            rep["problems"].append("empty frames in the middle: %s" % [i for i in mid if i not in lead])
        rep["grid"] = "%dx%d" % (g, g)
        rep["coverage"] = cover
        loop, fps = meta.get("loop"), meta.get("fps")
        use = {"FlipbookLayout": "Grid%dx%d" % (g, g), "FlipbookMode": "Loop" if loop else "OneShot",
               "LightEmission": 1 if additive else "0 for solid matter, 1 for light"}
        if fps:
            if loop:
                use["FlipbookFramerate"] = min(fps, 30)
            else:
                use["Lifetime"] = round(g * g / fps, 3)
        rep["use"] = use
    else:
        cov = float((ink > 0.05).mean())
        rep["coverage"] = round(cov, 3)
        if cov == 0:
            rep["problems"].append("the image is empty")
        if cov > 0.97:
            rep["problems"].append("the image is a filled square: it will show as a box")
        edge = max(ink[0].max(), ink[-1].max(), ink[:, 0].max(), ink[:, -1].max())
        if edge > 0.12 and cov <= 0.97:
            rep["notes"].append("the drawing touches the image edge: it may show a cut line")
        if max(img.size) > 512:
            rep["notes"].append("larger than 512 px: use 512 or 256 unless it is a flipbook sheet")
        if img.size[0] != img.size[1]:
            rep["notes"].append("not square")
    return rep


def contact_sheet(paths, out, tile=384):
    """Tile PNGs on a dark ground, flipbook cells outlined, for looking at."""
    cols = min(len(paths), 3)
    rows = (len(paths) + cols - 1) // cols
    canvas = Image.new("RGB", (cols * tile, rows * (tile + 22)), (22, 24, 30))
    draw = ImageDraw.Draw(canvas)
    for i, p in enumerate(paths):
        img = Image.open(p).convert("RGBA").resize((tile, tile), Image.LANCZOS)
        x, y = (i % cols) * tile, (i // cols) * (tile + 22)
        ground = Image.new("RGB", (tile, tile), (38, 42, 54) if (i % 2 == 0) else (30, 34, 44))
        ground.paste(img, (0, 0), img)
        canvas.paste(ground, (x, y))
        rep_grid = None
        meta = p[:-4] + ".json"
        if os.path.exists(meta):
            rep_grid = json.load(open(meta, encoding="utf-8")).get("grid")
        if rep_grid:
            step = tile / rep_grid
            for k in range(1, rep_grid):
                draw.line([(x + k * step, y), (x + k * step, y + tile)], fill=(90, 60, 60))
                draw.line([(x, y + k * step), (x + tile, y + k * step)], fill=(90, 60, 60))
        draw.text((x + 4, y + tile + 4), os.path.basename(p)[:52], fill=(200, 200, 210))
    canvas.save(out)
    return out


# ------------------------------------------------------------------- preview
def studio_content_dir():
    """Studio's textures folder on this computer (Windows or Mac), or None."""
    import rbxpaths
    content = rbxpaths.studio_content_dir()
    return os.path.join(content, "textures") if content else None


def stage_preview(path):
    """Copy a PNG into Studio's content folder and return an rbxasset:// id that
    works on this computer only (a free preview before uploading). A redrawn
    texture gets a new id, because the id is named from its pixels."""
    content = studio_content_dir()
    if not content:
        return None
    digest = hashlib.sha1(open(path, "rb").read()).hexdigest()[:10]
    stem = os.path.basename(path).replace(".flipbook.png", "").replace(".png", "")
    stem = "".join(ch if ch.isalnum() else "_" for ch in stem)
    dest_dir = os.path.join(content, "rbxkit")
    try:
        os.makedirs(dest_dir, exist_ok=True)
        shutil.copyfile(path, os.path.join(dest_dir, "%s_%s.png" % (stem, digest)))
    except OSError:
        return None
    return "rbxasset://textures/rbxkit/%s_%s.png" % (stem, digest)


def review(paths, preview=True):
    reports = []
    for p in paths:
        rep = check(p)
        if preview:
            rep["preview_id"] = stage_preview(p)
        reports.append(rep)
    sheet = None
    if paths:
        sheet = contact_sheet(paths, os.path.join(os.path.dirname(paths[0]), "_check.png"))
    return {"textures": reports, "look_at": sheet}


# ----------------------------------------------------------------------- CLI
def _cmd_tex(a):
    global _folder
    _folder = os.path.join("vfx", a.folder) if a.folder else "vfx"
    src = sys.stdin.read() if a.script == "-" else open(a.script, encoding="utf-8").read()
    me = sys.modules[__name__]
    scope = {"rbx": me, "np": np, "numpy": np, "math": math}
    exec(compile(src, a.script, "exec"), scope)
    if not _written:
        print("the script wrote no textures (call rbx.draw_texture or rbx.draw_flipbook)")
        return
    print(json.dumps(review(_written, preview=not a.nopreview), indent=1))


def _cmd_sheet(a):
    paths = [os.path.abspath(p) for p in a.images]
    print(json.dumps(review(paths, preview=not a.nopreview), indent=1))


def register(sub):
    t = sub.add_parser("tex", help="draw VFX textures with numpy (script uses the rbx helpers)")
    t.add_argument("--script", required=True, help="Python file, or - for stdin")
    t.add_argument("--folder", help="subfolder of work/vfx to write into, e.g. Fireball")
    t.add_argument("--nopreview", action="store_true", help="do not stage previews into Studio")
    t.set_defaults(fn=_cmd_tex)
    s = sub.add_parser("sheet", help="check PNGs from their pixels and tile them for review")
    s.add_argument("images", nargs="+")
    s.add_argument("--nopreview", action="store_true")
    s.set_defaults(fn=_cmd_sheet)
