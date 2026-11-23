"""AI-generated images for a game: its icon, thumbnail key art, UI art, concept art.

  rbx.py image generate "a paper boat on a calm lake at dawn" --size 1920x1080 --name Thumb1 --folder store
  rbx.py image generate "a smooth skipping stone with a painted finish" --size 512x512 --name Stone --transparent
  rbx.py image keep <an image file> --size 512x512 --name Icon

generate  The picture is made by Google's Antigravity command-line tool (agy) on the person's own Gemini plan, with
          no key (rbxgen.antigravity_image). The model is asked for the nearest shape it offers; the result is then
          cropped about its middle and resized to exactly the size asked for. Each one costs some of that plan, so:
          only while the person has left "Agents may generate images with Gemini" on in Setup, and never more than
          PER_HOUR in an hour for one chat.
keep      Brings an image that already exists into the project at an exact size: one Gemini's own generate_image
          tool left in Antigravity's folder for a conversation, or one in the project, this chat's own folder, or
          the app's images, attachments and chat pictures. Nothing else is read.
transparent  (both) The subject on a transparent background. generate asks for the subject on one flat colour; the
          background is then cut out here (cut_out): its colour is measured from the border, each pixel's alpha comes
          from how far its colour is from that, and the background's share is taken out of the edge pixels. When the
          border is not one flat colour nothing is guessed: the picture is saved as it is and the answer says so.

Both save into the game's project folder (RBXKIT_PROJECT), in `folder` there, and never outside it; a chat with no
project saves under the app's work folder instead. An existing file is never replaced. Nothing is uploaded.
"""
import io, json, os, re, sys, time, warnings

import rbxgen
from rbxpaths import KitError, read_json, write_json

WORK = rbxgen.WORK
RATIOS = ("1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3")
MIN_SIDE, MAX_SIDE = 16, 4096
MAX_SOURCE = 60 * 1024 * 1024   # bytes of a file `keep` will open
MAX_PIXELS = 50_000_000         # pixels of an image either action will open
FORMATS = ("PNG", "JPEG", "WEBP")
ATTACHED_UP_TO = 1_400_000      # bytes: a saved file up to this size is itself the picture handed back
DEFAULT_FOLDER = "art"
NAME_MAX = 80                   # letters of a file name before its ending
PER_HOUR = 12                   # generations one chat may start in an hour
CHAT_FOLDERS = (("gemini",), ("chatgpt",), ("opencode",), ("cursor", "chats"))     # work/<these>/<chat id>
NOTE = "rbx_image uploads nothing. Look at the picture before you use or report it; uploads follow the guide's upload rules."
# the flat background a cut-out is asked for: the first whose colour the prompt does not seem to want in the subject
KEYS = (("magenta", "#FF00FF", ("magenta", "pink", "purple", "violet", "fuchsia", "lilac", "rose")),
        ("green", "#00FF00", ("green", "lime", "emerald", "leaf", "leaves", "grass", "moss", "plant", "tree", "forest")),
        ("blue", "#0000FF", ("blue", "cyan", "water", "ocean", "sea", "sky", "ice", "sapphire")))


class ImageError(KitError):
    pass


def _plain(e):
    """An operating system error in a few plain words ("File name too long"), never a traceback or a class name."""
    return (getattr(e, "strerror", None) or "it could not be written").rstrip(".")


# ------------------------------------------------------------------ sizes
def parse_size(size, default=None):
    """(width, height) from "1920x1080" (also "1920 x 1080", "1920*1080", or one number for a square)."""
    if size in (None, ""):
        if default is None:
            raise ImageError("Give 'size' as WIDTHxHEIGHT in pixels, for example 512x512 or 1920x1080.")
        return default
    m = re.fullmatch(r"\s*(\d{1,5})\s*(?:[xX*×]\s*(\d{1,5}))?\s*", str(size))
    if not m:
        raise ImageError("The size \"%s\" is not WIDTHxHEIGHT in pixels. Give 'size' like 512x512 or 1920x1080." % str(size)[:40])
    w, h = int(m.group(1)), int(m.group(2) or m.group(1))
    if not (MIN_SIDE <= w <= MAX_SIDE and MIN_SIDE <= h <= MAX_SIDE):
        raise ImageError("The size %dx%d is out of range: each side must be %d to %d pixels." % (w, h, MIN_SIDE, MAX_SIDE))
    return w, h


def nearest_ratio(w, h):
    """The shape to ask the model for: the closest of the ratios image models offer."""
    import math
    want = math.log(w / h)

    def off(r):
        a, b = (int(x) for x in r.split(":"))
        return abs(math.log(a / b) - want)
    return min(RATIOS, key=off)


def fit(img, w, h):
    """Exactly w x h: cropped about the middle to that shape (never stretched), then resized."""
    from PIL import Image
    sw, sh = img.size
    if sw * h > sh * w:         # too wide: lose the sides
        cw = max(1, round(sh * w / h))
        left = (sw - cw) // 2
        img = img.crop((left, 0, left + cw, sh))
    elif sw * h < sh * w:       # too tall: lose top and bottom
        ch = max(1, round(sw * h / w))
        top = (sh - ch) // 2
        img = img.crop((0, top, sw, top + ch))
    return img if img.size == (w, h) else img.resize((w, h), Image.LANCZOS)


def within(size):
    """A size no larger than MAX_SIDE a side, keeping its shape (for an image kept at its own size)."""
    w, h = size
    scale = min(1.0, MAX_SIDE / max(w, h))
    return max(MIN_SIDE, round(w * scale)), max(MIN_SIDE, round(h * scale))


# -------------------------------------------------------- transparent cut-out
def key_for(prompt):
    """The flat background colour to ask for: (name, hex), one the prompt does not seem to want in the subject."""
    low = (prompt or "").lower()
    for name, code, words in KEYS:
        if not any(re.search(r"\b%s" % w, low) for w in words):
            return name, code
    return KEYS[0][:2]


def _grow(mask, steps):
    """A mask grown by `steps` pixels in every direction (numpy only)."""
    import numpy as np
    out = mask.copy()
    for _ in range(steps):
        p = np.pad(out, 1, mode="edge")
        out = p[1:-1, 1:-1] | p[:-2, 1:-1] | p[2:, 1:-1] | p[1:-1, :-2] | p[1:-1, 2:] | p[:-2, :-2] | p[:-2, 2:] | p[2:, :-2] | p[2:, 2:]
    return out


def cut_out(img):
    """The picture with its flat background made transparent: (RGBA image, None), or (None, why not).

    The background's colour is the middle colour of a ring round the border; if that ring is not close to one flat
    colour, nothing is guessed. A pixel as close to that colour as the border's own noise is background. On a
    saturated background (a chroma colour asked for on purpose) a somewhat darker shade of the very same hue is
    background too: a soft shadow the model drew anyway, or the background showing through a dark outline's edge.
    The pixels next to the background are the soft edge. The subject's own colour there is carried outward from the
    pixels just further in. A pixel that lies between the background's colour and that colour is a blend of the
    two: its alpha is how far along it lies, and the background's share is taken out of its colour, so no rim of
    the background colour is left. A pixel of another colour (a thin outline, a detail) is the subject's and is
    kept, fading only where it is close to the background's colour. Everything further in is kept as it is.
    """
    import numpy as np
    from PIL import Image
    a = np.asarray(img.convert("RGB"), dtype=np.float32) / 255.0
    h, w = a.shape[:2]
    if min(h, w) < 24:
        return None, "the picture is too small to tell its background from its subject"
    ring = max(2, round(min(h, w) * 0.015))
    border = np.concatenate([a[:ring].reshape(-1, 3), a[-ring:].reshape(-1, 3), a[ring:-ring, :ring].reshape(-1, 3), a[ring:-ring, -ring:].reshape(-1, 3)])
    bg = np.median(border, axis=0)
    spread = np.sqrt(((border - bg) ** 2).sum(1) / 3)
    if float((spread < 0.06).mean()) < 0.97:
        return None, "its border is not one flat colour (a gradient, a scene, or the subject touches the edge)"
    lo = min(max(0.035, float(np.percentile(spread, 95)) * 2.5), 0.09)
    d = np.sqrt(((a - bg) ** 2).sum(2) / 3)
    is_bg = d <= lo
    if float(bg.max() - bg.min()) >= 0.5:
        # a shade of the same hue: the pixel's colour points the way the background's does, only darker
        norm = np.sqrt((a * a).sum(2)) * float(np.sqrt((bg * bg).sum()))
        same_hue = (a * bg).sum(2) / np.maximum(norm, 1e-6) >= 0.992
        shade = a.max(2) / max(float(bg.max()), 1e-6)
        is_bg |= same_hue & (shade >= 0.5) & (shade <= 1.05)
    covered = float(is_bg.mean())
    if covered > 0.995:
        return None, "nothing but background was found in it"
    if covered < 0.02:
        return None, "no flat background was found round the subject"
    k = max(2, round(min(h, w) / 400))      # how far the background's colour reaches into the subject's edge
    near = _grow(is_bg, k)
    band = near & ~is_bg
    # the subject's own colour for the edge pixels, carried outward from the pixels just further in
    known = ~near
    est = np.where(known[..., None], a, 0.0).astype(np.float32)
    for _ in range(k + 2):
        pe, pk = np.pad(est, ((1, 1), (1, 1), (0, 0))), np.pad(known, 1).astype(np.float32)
        total, count = np.zeros_like(est), np.zeros((h, w), dtype=np.float32)
        for dy in (0, 1, 2):
            for dx in (0, 1, 2):
                total += pe[dy:dy + h, dx:dx + w] * pk[dy:dy + h, dx:dx + w, None]
                count += pk[dy:dy + h, dx:dx + w]
        new = ~known & (count > 0)
        est[new] = total[new] / count[new][:, None]
        known |= new
    v, p = est - bg, a - bg
    vv, pv = (v * v).sum(2), (p * v).sum(2)
    along = np.clip(pv / np.maximum(vv, 1e-6), 0.0, 1.0)
    # how far the pixel's colour lies from the line between the background's colour and the subject's
    aside = np.sqrt(np.maximum((p * p).sum(2) - pv * pv / np.maximum(vv, 1e-6), 0.0) / 3)
    # a blend of the two: only where a colour could be carried, and the subject's is not too near the background's
    blend = known & (vv > (3 * lo) ** 2 * 3) & (aside <= 0.10)
    # any other colour is the subject's own, and fades only close to the background's colour
    hi = max(lo * 3, 0.22)
    ramp = np.clip((d - lo) / (hi - lo), 0.0, 1.0)
    alpha = np.ones((h, w), dtype=np.float32)
    alpha[band] = np.where(blend, along, ramp)[band]
    alpha[band & (alpha < 0.06)] = 0.0      # no faint haze round the subject (a compressed picture's edge noise)
    alpha[is_bg] = 0.0
    al = alpha[..., None]
    clean = np.clip((a - (1 - al) * bg) / np.maximum(al, 1e-3), 0.0, 1.0)
    rgb = np.where(band[..., None], clean, a)
    faint = band & blend & (alpha < 0.35)
    rgb[faint] = est[faint]
    rgb[is_bg] = 0.0
    out = np.dstack([rgb, alpha])
    return Image.fromarray((out * 255 + 0.5).astype(np.uint8), "RGBA"), None


def has_alpha(img):
    """Whether the image already has a transparent background of its own."""
    if img.mode not in ("RGBA", "LA", "PA") and not (img.mode == "P" and "transparency" in img.info):
        return False
    return img.convert("RGBA").getchannel("A").getextrema()[0] < 250


def contain(img, w, h, margin=0.04):
    """A cut-out placed whole inside w x h: trimmed to its subject, scaled to fit with a small margin (its shape
    kept, never stretched), centred, the rest transparent."""
    from PIL import Image
    img = img.convert("RGBA")
    box = img.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    if box:
        img = img.crop(box)
    room_w, room_h = max(1, round(w * (1 - 2 * margin))), max(1, round(h * (1 - 2 * margin)))
    scale = min(room_w / img.size[0], room_h / img.size[1])
    size = (max(1, round(img.size[0] * scale)), max(1, round(img.size[1] * scale)))
    # resized with its colours weighted by alpha, so transparent pixels never darken the edge
    small = img.convert("RGBa").resize(size, Image.LANCZOS).convert("RGBA")
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    out.paste(small, ((w - size[0]) // 2, (h - size[1]) // 2))
    return out


def check_sheet(img, path):
    """A cut-out shown over a dark and a light backdrop side by side, to judge its edges by."""
    from PIL import Image
    img = img.convert("RGBA")
    img.thumbnail((640, 640), Image.LANCZOS)
    w, h = img.size
    sheet = Image.new("RGB", (w * 2, h), (27, 27, 31))
    sheet.paste((238, 238, 240), (w, 0, w * 2, h))
    sheet.paste(img, (0, 0), img)
    sheet.paste(img, (w, 0), img)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    sheet.save(path, quality=90)
    return path


# ------------------------------------------------------------ where it goes
def _project():
    p = os.environ.get("RBXKIT_PROJECT") or ""
    return os.path.realpath(p) if p and os.path.isdir(p) else None


def _inside(path, folder):
    path, folder = os.path.normcase(path), os.path.normcase(folder)
    return path == folder or path.startswith(folder.rstrip(os.sep) + os.sep)


def _hidden(parts):
    return any(p == ".." or p.startswith(".") or p.startswith("~") or not re.fullmatch(r"[\w .()+,&'-]+", p) for p in parts)


def out_dir(folder):
    """The folder the image is saved in: `folder` inside the game's project folder (or, in a chat with no project,
    inside the app's work folder), never anywhere else, and never a hidden folder (.git, .fridae, .agents), also not
    one reached through a link."""
    project = _project()
    root = project or os.path.join(os.path.realpath(WORK), "images")
    where = "the game's project folder" if project else "the app's work folder (this chat has no game project)"
    refuse = ImageError("The folder \"%s\" is not a place images are saved: give a plain folder inside %s, for example "
                        "\"art\" or \"store/thumbnails\" (%s)." % (str(folder)[:120], where, root))
    raw = str(folder or DEFAULT_FOLDER).strip().replace("\\", "/")
    if "\0" in raw:
        raise refuse
    if os.path.isabs(raw) or re.match(r"^[A-Za-z]:", raw):
        full = os.path.realpath(raw)
        if not _inside(full, root):
            raise refuse
        raw = os.path.relpath(full, root).replace("\\", "/")
        if raw == ".":
            raw = ""
    parts = [p for p in raw.split("/") if p not in ("", ".")]
    if _hidden(parts):
        raise refuse
    full = os.path.join(root, *parts)
    # a link inside the project may point anywhere: the folder is judged by where it really is, hidden names too
    real = os.path.realpath(full)
    if not _inside(real, root) or _hidden([p for p in os.path.relpath(real, root).replace("\\", "/").split("/") if p not in ("", ".")]):
        raise refuse
    return full


def _name(name, fallback, ending=None):
    """(stem, ending) of the file: a plain name of NAME_MAX letters at most; .png unless the name asks for .jpg."""
    base = os.path.basename(str(name or fallback).replace("\\", "/").strip())
    stem, ext = os.path.splitext(base)
    ext = ext.lower()
    if ext not in (".png", ".jpg", ".jpeg"):
        stem, ext = (stem if ext == ".webp" else base), ".png"
    stem = re.sub(r"\s+", " ", re.sub(r"[^\w -]+", "", stem)).strip(" .")[:NAME_MAX].strip() or fallback
    return stem, ending or ext


def _unmake(made):
    for d in made:          # deepest first; only folders this call made, and only while they are empty
        try:
            os.rmdir(d)
        except OSError:
            break


def reserve(folder, name, fallback="Image", ending=None):
    """Settle where the image goes before anything is spent: the folder is made and an empty file of a free name
    is put there (a number is added rather than taking a name that is in use; a link is never written through).
    Returns (path, the folders made for it). Fails in plain words when the place cannot be written."""
    directory = out_dir(folder)
    stem, ext = _name(name, fallback, ending)
    made, d = [], directory
    while d and not os.path.isdir(d) and d != os.path.dirname(d):
        made.append(d)
        d = os.path.dirname(d)
    try:
        os.makedirs(directory, exist_ok=True)
        for n in range(1, 1000):
            path = os.path.join(directory, stem + ("" if n == 1 else "-%d" % n) + ext)
            try:
                with open(path, "xb"):
                    return path, made
            except FileExistsError:
                continue
        raise ImageError("There are too many images named %s in that folder already. Give another name." % stem)
    except OSError as e:
        _unmake(made)
        raise ImageError("An image cannot be saved in the folder \"%s\" (%s). Give another folder." % (str(folder or DEFAULT_FOLDER)[:120], _plain(e)))


def release(path, made):
    """Give a reserved place back when no picture came for it."""
    try:
        if os.path.getsize(path) == 0:
            os.remove(path)
    except OSError:
        pass
    _unmake(made)


def _write(img, path):
    """The image written beside the file and renamed over it, so a link put there meanwhile is replaced, not followed."""
    tmp = "%s.%d.part" % (path, os.getpid())
    try:
        if path.lower().endswith((".jpg", ".jpeg")):
            img.convert("RGB").save(tmp, format="JPEG", quality=92)
        else:
            img.save(tmp, format="PNG")
        os.replace(tmp, path)
    except OSError:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def store(img, path, made):
    """Save the picture in its reserved place. Should that fail after all, it is saved in the app's work folder
    instead and the answer says where, so a picture that was paid for is never thrown away. Returns (path, a note)."""
    try:
        _write(img, path)
        return path, None
    except OSError as e:
        release(path, made)
        spare = os.path.join(os.path.realpath(WORK), "images", "unsaved")
        try:
            os.makedirs(spare, exist_ok=True)
            base, ext = os.path.splitext(os.path.basename(path))
            for n in range(1, 1000):
                other = os.path.join(spare, base + ("" if n == 1 else "-%d" % n) + ext)
                if not os.path.lexists(other):
                    _write(img, other)
                    return other, ("The picture could not be saved in %s (%s), so it was saved in the app's work folder instead. "
                                   "Tell the user where it is." % (os.path.dirname(path), _plain(e)))
        except OSError:
            pass
        raise ImageError("The picture was made but could not be saved, neither in %s (%s) nor in the app's work folder. "
                         "Tell the user: the disk may be full or read-only." % (os.path.dirname(path), _plain(e)))


def _result(img, path, extra, sheet=False):
    """What the tool answers: the saved file, its size, and a picture to look at (the file itself when it is small
    enough to hand back, else a smaller copy in the app's work folder; for a cut-out, also a sheet of it over a dark
    and a light backdrop)."""
    out = {"file": path, "size": "%dx%d" % img.size}
    out.update(extra)
    looks = os.path.join(WORK, "images", "previews")
    base = os.path.splitext(os.path.basename(path))[0]
    try:
        if sheet:
            out["look_at"] = check_sheet(img, os.path.join(looks, base + "_cutout_check.jpg"))
        elif os.path.getsize(path) > ATTACHED_UP_TO:
            from PIL import Image
            os.makedirs(looks, exist_ok=True)
            small = img.convert("RGB")
            small.thumbnail((1280, 1280), Image.LANCZOS)
            small.save(os.path.join(looks, base + "_preview.jpg"), quality=85)
            out["look_at"] = os.path.join(looks, base + "_preview.jpg")
    except OSError:
        pass                    # the picture is saved; only the copy to look at is missing
    out["note"] = NOTE
    return out


def _open(source):
    """An image opened from bytes or a path: PNG, JPEG or WebP only, and never one so large it would fill memory."""
    from PIL import Image
    was = Image.MAX_IMAGE_PIXELS
    Image.MAX_IMAGE_PIXELS = MAX_PIXELS
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("error", Image.DecompressionBombWarning)
            img = Image.open(io.BytesIO(source) if isinstance(source, bytes) else source, formats=FORMATS)
            if img.size[0] * img.size[1] > MAX_PIXELS:
                raise Image.DecompressionBombError("too large")
            img.load()
            return img
    except (Image.DecompressionBombError, Image.DecompressionBombWarning):
        raise ImageError("That picture is too large to open (over %d megapixels). Use a smaller one." % (MAX_PIXELS // 1_000_000))
    finally:
        Image.MAX_IMAGE_PIXELS = was


def _colours(img):
    return img.convert("RGBA") if img.mode in ("P", "LA", "PA", "RGBA") else img.convert("RGB")


def _finish(img, w, h, transparent, path, made, extra):
    """Crop or cut out, save, and answer."""
    problem = None
    if transparent:
        cut, why = (img.convert("RGBA"), None) if has_alpha(img) else cut_out(img)
        if cut is None:
            problem = ("The background could not be removed: %s. Nothing was guessed: this file still has its background. "
                       "For a cut-out, the subject must sit whole on one flat colour with no shadow or gradient." % why)
        else:
            img = contain(cut, w, h)
            extra = dict(extra, transparent=True)
    if not transparent or problem:
        img = fit(_colours(img), w, h)
    path, note = store(img, path, made)
    out = _result(img, path, extra, sheet=bool(transparent and not problem))
    if problem:
        out["transparent"] = False
        out["problem"] = problem
    if note:
        out["saved_elsewhere"] = note
    return out


# ------------------------------------------------- what generating may cost
def images_allowed():
    """Whether agents may generate images on the person's Gemini plan: the switch in Setup (Textures), on unless they
    turned it off. Run by the app, the app's own value is used, never the settings file (an AI's file tool may have
    changed that)."""
    if os.environ.get("FRIDAE_SETTINGS_FROM_APP") == "1":
        return os.environ.get("FRIDAE_GEMINI_IMAGES", "1") != "0"
    return rbxgen._config().get("gemini_images") is not False


def _chat():
    return re.sub(r"[^A-Za-z0-9_-]", "", os.environ.get("RBXKIT_AGENT_ID") or "")[:80]


def _spent_file():
    return os.path.join(rbxgen.turns_dir(), "spent", (_chat() or "command-line") + ".json")


def _spent(now):
    """When this chat started its generations of the last hour (a file that does not read as that counts as none)."""
    had = read_json(_spent_file())
    return [t for t in had if isinstance(t, (int, float)) and not isinstance(t, bool) and 0 <= now - t < 3600] if isinstance(had, list) else []


def _note_spent(now):
    try:
        os.makedirs(os.path.dirname(_spent_file()), exist_ok=True)
        write_json(_spent_file(), _spent(now) + [now])
    except OSError:
        pass


# ----------------------------------------------------------------- generate
def generate(prompt, size=None, name=None, folder=None, transparent=False):
    if not (prompt or "").strip():
        raise ImageError("Give 'prompt': what the image shows, in plain words (subject, setting, style, mood).")
    prompt = str(prompt).strip()
    w, h = parse_size(size, default=(1024, 1024))
    if not images_allowed():
        raise ImageError("The user has switched off image generation for agents (Setup, Textures: \"Agents may generate images "
                         "with Gemini\"), so no image was made. Ask the user first: they can switch it on there, or make the "
                         "picture themselves, and you bring it in with action keep.")
    exe = rbxgen.antigravity_exe()
    if not exe:
        raise ImageError("Images are made by Google's Antigravity tool (agy) on the user's own Gemini plan, and it is not "
                         "installed on this computer, so no image was made. The user installs it from "
                         "https://antigravity.google, runs `agy` once in a terminal and signs in; then ask again.")
    if not rbxgen.find_antigravity():
        raise ImageError("Antigravity is switched off as a source of images in this app's settings "
                         "(\"textures_from_antigravity\": false in config.json), so no image was made. Tell the user.")
    now = time.time()
    if len(_spent(now)) >= PER_HOUR:
        raise ImageError("This chat has generated %d images in the last hour, the most it may without the user's say. No image "
                         "was made. Stop generating: tell the user what you still want to make and why, and ask them." % PER_HOUR)
    if transparent:
        try:
            import numpy  # noqa: F401
        except ImportError:
            raise ImageError("A transparent background needs the numpy library, which is missing here. Run rbx_doctor to see what to install.")
    path, made = reserve(folder, name, ending=".png" if transparent else None)   # where it goes is settled, and writable, before anything is spent
    ratio = nearest_ratio(w, h)
    ask = prompt
    if transparent:
        colour, code = key_for(prompt)
        ask = ("%s. The subject alone, whole and centred, with empty margin all round, on one perfectly flat solid pure %s "
               "background (%s) that fills the rest of the picture edge to edge: no shadow, no reflection, no floor, no "
               "gradient, no vignette, no border, no text. Do not use %s anywhere in the subject." % (prompt.rstrip("."), colour, code, colour))
    _note_spent(now)
    try:
        raw = rbxgen.antigravity_image(exe, ask, aspect=ratio)
        img = _open(raw)
    except RuntimeError as e:
        release(path, made)
        raise ImageError(str(e))
    except ImageError:
        release(path, made)
        raise
    except Exception:
        release(path, made)
        raise ImageError("Antigravity returned a file that does not open as an image. Try once more.")
    extra = {"made_at": "%dx%d" % img.size, "asked_for": ratio, "from": "generated by Antigravity on the user's Gemini plan",
             "generated_this_hour": "%d of %d" % (len(_spent(time.time())), PER_HOUR)}
    return _finish(img, w, h, transparent, path, made, extra)


# --------------------------------------------------------------------- keep
def _own_folders():
    """The folders `keep` reads besides Antigravity's: the game's project, and in the app's work folder this chat's own
    folder, the images this tool made, the user's attachments and the chat pictures (what the app lets a chat read)."""
    work = os.path.realpath(WORK)
    roots = [os.path.join(work, d) for d in ("images", "attachments", "chat-images")]
    if _chat():
        roots += [os.path.join(work, *parts, _chat()) for parts in CHAT_FOLDERS]
    project = _project()
    return ([project] if project else []) + roots


def _generated_by_agy(real):
    """A picture Antigravity's image tool left: a file at the top of one conversation's folder (never in its
    .system_generated logs or any other folder below)."""
    brain = os.path.realpath(rbxgen.brain_dir())
    conversation = os.path.dirname(real)
    return (os.path.dirname(conversation) == brain and bool(rbxgen.CONVERSATION_ID.match(os.path.basename(conversation)))
            and not os.path.basename(real).startswith("."))


def source_file(source):
    """The image to keep, as a real path: only an image file, and only from the places above."""
    raw = str(source or "").strip().strip('"')
    if not raw:
        raise ImageError("Give 'source': the full path of the image file to keep (for example the file your generate_image tool made).")
    if raw.lower().startswith("file://"):
        from urllib.parse import unquote, urlparse
        raw = unquote(urlparse(raw).path)
        if re.match(r"^/[A-Za-z]:", raw):
            raw = raw[1:]
    elif re.match(r"^[a-z][a-z0-9+.-]*://", raw, re.I):
        raise ImageError("The source must be an image file on this computer, not a web address.")
    if "\0" in raw:
        raise ImageError("The source is not a file path.")
    raw = os.path.expanduser(raw)
    if not os.path.isabs(raw):
        raw = os.path.join(_project() or os.getcwd(), raw)
    if not raw.lower().endswith(rbxgen.IMAGE_ENDINGS):
        raise ImageError("Only an image file can be kept (.png, .jpg, .jpeg or .webp); \"%s\" is not one." % os.path.basename(raw)[:80])
    real = os.path.realpath(raw)
    if not (any(_inside(real, root) for root in _own_folders()) or _generated_by_agy(real)):
        raise ImageError("The source must be an image inside the game's project folder, this chat's own folder, the app's "
                         "images, attachments or chat pictures, or the file your own generate_image tool made; this file is "
                         "somewhere else, so it was not read.")
    if not os.path.isfile(real):
        raise ImageError("There is no such file: %s" % raw[:300])
    if os.path.getsize(real) > MAX_SOURCE:
        raise ImageError("That file is over %d MB, which is too large to keep as an image." % (MAX_SOURCE // (1024 * 1024)))
    return real


def keep(source, size=None, name=None, folder=None, transparent=False):
    real = source_file(source)
    try:
        img = _open(real)
    except ImageError:
        raise
    except Exception:
        raise ImageError("\"%s\" does not open as a PNG, JPEG or WebP image, so it was not kept." % os.path.basename(real)[:80])
    w, h = parse_size(size, default=within(img.size))
    if transparent and not has_alpha(img):
        try:
            import numpy  # noqa: F401
        except ImportError:
            raise ImageError("A transparent background needs the numpy library, which is missing here. Run rbx_doctor to see what to install.")
    was = "%dx%d" % img.size
    path, made = reserve(folder, name, fallback=os.path.splitext(os.path.basename(real))[0] or "Image", ending=".png" if transparent else None)
    extra = {"made_at": was, "from": "kept from " + real}
    if size in (None, "") and (w, h) != img.size:
        extra["made_smaller"] = "The source is larger than %d pixels a side, so it was made smaller." % MAX_SIDE
    try:
        return _finish(img, w, h, transparent, path, made, extra)
    except ImageError:
        release(path, made)
        raise


def run(action, a):
    """The tool's answer as a dict. Its failures are KitErrors, written to be read as they are; nothing else gets out."""
    action = str(action or "").strip().lower()
    try:
        from PIL import Image  # noqa: F401
    except ImportError:
        raise ImageError("This tool needs the Pillow library, which is missing here. Run rbx_doctor to see what to install.")
    see_through = a.get("transparent") is True or str(a.get("transparent")).lower() == "true"
    try:
        if action == "generate":
            return generate(a.get("prompt"), a.get("size"), a.get("name"), a.get("folder"), see_through)
        if action == "keep":
            return keep(a.get("source"), a.get("size"), a.get("name"), a.get("folder"), see_through)
    except KitError:
        raise
    except Exception as e:      # never a traceback or a raw error to the agent
        raise ImageError("The image tool ran into a problem of its own (%s), so nothing was made. Try once more; if it "
                         "happens again, tell the user." % (_plain(e) if isinstance(e, OSError) else "an unexpected error"))
    raise ImageError("Give 'action': \"generate\" (make a new image from a prompt) or \"keep\" (bring an existing image file into the project).")


def _cmd(a):
    args = {"prompt": a.what, "source": a.what, "size": a.size, "name": a.name, "folder": a.folder, "transparent": a.transparent}
    print(json.dumps(run(a.action, args), indent=1, ensure_ascii=False))


def register(sub):
    p = sub.add_parser("image", help="make an AI-generated image (icon, thumbnail art, UI art) or keep an existing one, at an exact size")
    p.add_argument("action", choices=["generate", "keep"])
    p.add_argument("what", help="generate: the prompt; keep: the image file")
    p.add_argument("--size", help="WIDTHxHEIGHT in pixels, e.g. 512x512 or 1920x1080")
    p.add_argument("--name", help="file name, without or with .png / .jpg")
    p.add_argument("--folder", help="folder inside the game's project folder (default: art)")
    p.add_argument("--transparent", action="store_true", help="cut the subject out onto a transparent background")
    p.set_defaults(fn=_cmd)


if __name__ == "__main__":
    sys.exit("Run this through rbx.py: rbx.py image generate \"<prompt>\" --size 512x512")
