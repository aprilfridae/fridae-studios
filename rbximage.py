"""AI-generated images for a game: its icon, thumbnail key art, UI art, concept art.

  rbx.py image generate "a paper boat on a calm lake at dawn" --size 1920x1080 --name Thumb1 --folder store
  rbx.py image keep <an image file> --size 512x512 --name Icon

generate  The picture is made by Google's Antigravity command-line tool (agy) on the person's own Gemini plan, with
          no key (rbxgen.antigravity_image). The model is asked for the nearest shape it offers; the result is then
          cropped about its middle and resized to exactly the size asked for.
keep      Brings an image that already exists into the project at an exact size: one Gemini's own generate_image
          tool left in Antigravity's folder, or one in the project or the app's work folder. Nothing else is read.

Both save into the game's project folder (RBXKIT_PROJECT), in `folder` there, and never outside it; a chat with no
project saves under the app's work folder instead. An existing file is never replaced. Nothing is uploaded.
"""
import io, json, os, re, sys

import rbxgen
from rbxpaths import KitError

WORK = rbxgen.WORK
RATIOS = ("1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3")
MIN_SIDE, MAX_SIDE = 16, 4096
MAX_SOURCE = 60 * 1024 * 1024
ATTACHED_UP_TO = 1_400_000      # bytes: a saved file up to this size is itself the picture handed back
DEFAULT_FOLDER = "art"
NOTE = "Not uploaded. Look at it before you use it or report it; the user uploads images themselves."


class ImageError(KitError):
    pass


def parse_size(size, default=None):
    """(width, height) from "1920x1080" (also "1920 x 1080", "1920*1080", or one number for a square)."""
    if size in (None, ""):
        if default is None:
            raise ImageError("Give 'size' as WIDTHxHEIGHT in pixels, for example 512x512 or 1920x1080.")
        return default
    m = re.fullmatch(r"\s*(\d{1,5})\s*(?:[xX*×]\s*(\d{1,5}))?\s*", str(size))
    if not m:
        raise ImageError("The size \"%s\" is not WIDTHxHEIGHT in pixels. Give 'size' like 512x512 or 1920x1080." % size)
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


def _project():
    p = os.environ.get("RBXKIT_PROJECT") or ""
    return os.path.realpath(p) if p and os.path.isdir(p) else None


def _inside(path, folder):
    path, folder = os.path.normcase(path), os.path.normcase(folder)
    return path == folder or path.startswith(folder.rstrip(os.sep) + os.sep)


def out_dir(folder):
    """The folder the image is saved in: `folder` inside the game's project folder (or, in a chat with no project,
    inside the app's work folder), never anywhere else, and never a hidden folder (.git, .fridae, .agents)."""
    project = _project()
    root = project or os.path.join(os.path.realpath(WORK), "images")
    where = "the game's project folder" if project else "the app's work folder (this chat has no game project)"
    refuse = ImageError("The folder \"%s\" is not a place images are saved: give a plain folder inside %s, for example "
                        "\"art\" or \"store/thumbnails\" (%s)." % (folder, where, root))
    raw = str(folder or DEFAULT_FOLDER).strip().replace("\\", "/")
    if os.path.isabs(raw) or re.match(r"^[A-Za-z]:", raw):
        full = os.path.realpath(raw)
        if not _inside(full, root):
            raise refuse
        raw = os.path.relpath(full, root).replace("\\", "/")
        if raw == ".":
            raw = ""
    parts = [p for p in raw.split("/") if p not in ("", ".")]
    if any(p == ".." or p.startswith(".") or p.startswith("~") or not re.fullmatch(r"[\w .()+,&'-]+", p) for p in parts):
        raise refuse
    full = os.path.join(root, *parts)
    # a link inside the project may point anywhere: the folder is judged by where it really is
    if not _inside(os.path.realpath(full), root):
        raise refuse
    return full


def out_file(folder, name, fallback="Image"):
    """The file to write: <folder>/<name>.png (or .jpg when the name asks for it); a number is added instead of
    replacing a file that is already there."""
    directory = out_dir(folder)
    base = os.path.basename(str(name or fallback).replace("\\", "/").strip())
    stem, ext = os.path.splitext(base)
    ext = ext.lower()
    if ext not in (".png", ".jpg", ".jpeg"):
        stem, ext = (stem if ext in (".webp",) else base), ".png"
    stem = re.sub(r"\s+", " ", re.sub(r"[^\w -]+", "", stem)).strip(" .") or fallback
    path, n = os.path.join(directory, stem + ext), 1
    while os.path.exists(path):
        n += 1
        path = os.path.join(directory, "%s-%d%s" % (stem, n, ext))
    return path


def _save(img, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if path.lower().endswith((".jpg", ".jpeg")):
        img.convert("RGB").save(path, quality=92)
    else:
        img.save(path)


def _result(img, path, extra):
    """What the tool answers: the saved file, its size, and a picture to look at (the file itself when it is small
    enough to hand back, else a smaller copy in the app's work folder)."""
    out = {"file": path, "size": "%dx%d" % img.size}
    out.update(extra)
    if os.path.getsize(path) > ATTACHED_UP_TO:
        from PIL import Image
        look_dir = os.path.join(WORK, "images", "previews")
        os.makedirs(look_dir, exist_ok=True)
        look = os.path.join(look_dir, os.path.splitext(os.path.basename(path))[0] + "_preview.jpg")
        small = img.convert("RGB")
        small.thumbnail((1280, 1280), Image.LANCZOS)
        small.save(look, quality=85)
        out["look_at"] = look
    out["note"] = NOTE
    return out


def generate(prompt, size=None, name=None, folder=None):
    from PIL import Image
    if not (prompt or "").strip():
        raise ImageError("Give 'prompt': what the image shows, in plain words (subject, setting, style, mood).")
    w, h = parse_size(size, default=(1024, 1024))
    path = out_file(folder, name)        # where it will go is settled before anything is spent
    exe = rbxgen.antigravity_exe()
    if not exe:
        raise ImageError("Images are made by Google's Antigravity tool (agy) on the user's own Gemini plan, and it is not "
                         "installed on this computer, so no image was made. The user installs it from "
                         "https://antigravity.google, runs `agy` once in a terminal and signs in; then ask again.")
    ratio = nearest_ratio(w, h)
    try:
        raw = rbxgen.antigravity_image(exe, prompt.strip(), aspect=ratio)
    except RuntimeError as e:
        raise ImageError(str(e))
    try:
        img = Image.open(io.BytesIO(raw))
        img.load()
    except Exception:
        raise ImageError("Antigravity returned a file that does not open as an image. Try once more.")
    made = "%dx%d" % img.size
    img = fit(img.convert("RGBA") if img.mode in ("P", "LA", "RGBA") else img.convert("RGB"), w, h)
    path = out_file(folder, name)
    _save(img, path)
    return _result(img, path, {"made_at": made, "asked_for": ratio, "from": "generated by Antigravity on the user's Gemini plan"})


def _source_roots():
    roots = [("Gemini's own image folder", os.path.realpath(rbxgen.brain_dir())), ("the app's work folder", os.path.realpath(WORK))]
    project = _project()
    if project:
        roots.insert(0, ("the game's project folder", project))
    return roots


def source_file(source):
    """The image to keep, as a real path: only an image file, and only inside the project, the app's work folder or
    Antigravity's own folder of generated images."""
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
    raw = os.path.expanduser(raw)
    if not os.path.isabs(raw):
        raw = os.path.join(_project() or os.getcwd(), raw)
    if not raw.lower().endswith(rbxgen.IMAGE_ENDINGS):
        raise ImageError("Only an image file can be kept (.png, .jpg, .jpeg or .webp); \"%s\" is not one." % os.path.basename(raw))
    real = os.path.realpath(raw)
    roots = _source_roots()
    if not any(_inside(real, root) for _, root in roots):
        raise ImageError("The source must be inside %s or %s; this file is somewhere else, so it was not read."
                         % (", ".join(label for label, _ in roots[:-1]), roots[-1][0]))
    if not os.path.isfile(real):
        raise ImageError("There is no such file: %s" % raw)
    if os.path.getsize(real) > MAX_SOURCE:
        raise ImageError("That file is over %d MB, which is too large to keep as an image." % (MAX_SOURCE // (1024 * 1024)))
    return real


def keep(source, size=None, name=None, folder=None):
    from PIL import Image
    real = source_file(source)
    try:
        with Image.open(real) as probe:
            probe.verify()
        img = Image.open(real)
        img.load()
    except Exception:
        raise ImageError("\"%s\" does not open as an image, so it was not kept." % os.path.basename(real))
    w, h = parse_size(size, default=img.size)
    was = "%dx%d" % img.size
    img = fit(img.convert("RGBA") if img.mode in ("P", "LA", "RGBA", "PA") else img.convert("RGB"), w, h)
    path = out_file(folder, name, fallback=os.path.splitext(os.path.basename(real))[0] or "Image")
    _save(img, path)
    return _result(img, path, {"made_at": was, "from": "kept from " + real})


def run(action, a):
    """The tool's answer as a dict. Its failures are KitErrors, written to be read as they are."""
    action = (action or "").strip().lower()
    try:
        from PIL import Image  # noqa: F401
    except ImportError:
        raise ImageError("This tool needs the Pillow library, which is missing here. Run rbx_doctor to see what to install.")
    if action == "generate":
        return generate(a.get("prompt"), a.get("size"), a.get("name"), a.get("folder"))
    if action == "keep":
        return keep(a.get("source"), a.get("size"), a.get("name"), a.get("folder"))
    raise ImageError("Give 'action': \"generate\" (make a new image from a prompt) or \"keep\" (bring an existing image file into the project).")


def _cmd(a):
    args = {"prompt": a.what, "source": a.what, "size": a.size, "name": a.name, "folder": a.folder}
    print(json.dumps(run(a.action, args), indent=1))


def register(sub):
    p = sub.add_parser("image", help="make an AI-generated image (icon, thumbnail art, UI art) or keep an existing one, at an exact size")
    p.add_argument("action", choices=["generate", "keep"])
    p.add_argument("what", help="generate: the prompt; keep: the image file")
    p.add_argument("--size", help="WIDTHxHEIGHT in pixels, e.g. 512x512 or 1920x1080")
    p.add_argument("--name", help="file name, without or with .png / .jpg")
    p.add_argument("--folder", help="folder inside the game's project folder (default: art)")
    p.set_defaults(fn=_cmd)


if __name__ == "__main__":
    sys.exit("Run this through rbx.py: rbx.py image generate \"<prompt>\" --size 512x512")
