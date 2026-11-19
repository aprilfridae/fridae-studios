"""Image helpers: cut an icon out of an icon-booth screenshot.

  rbx.py icon <screenshot.jpg> --name Sword [--size 512] [--folder icons/Weapons]
"""
import json, os, sys
import numpy as np
from PIL import Image

from rbxpaths import KIT, WORK


def cut_icon(src, out, size=512, pad=0.04):
    img = np.asarray(Image.open(src).convert("RGB")).astype(float)
    r, g, b = img[..., 0], img[..., 1], img[..., 2]
    # magenta key: red and blue high, green low
    keyness = np.clip((np.minimum(r, b) - g) / 255.0, 0, 1)
    key = keyness > 0.55
    if key.mean() < 0.02:
        sys.exit("no key colour found: is the icon booth showing in this screenshot?")
    ys, xs = np.where(key)
    # the booth is the bounding box of the key colour
    y0, y1, x0, x1 = ys.min(), ys.max() + 1, xs.min(), xs.max() + 1
    crop = img[y0:y1, x0:x1]
    k = keyness[y0:y1, x0:x1]
    alpha = np.clip(1.0 - (k - 0.12) / 0.38, 0, 1)
    # remove the magenta spill on edge pixels
    rgb = crop.copy()
    spill = np.clip(np.minimum(rgb[..., 0], rgb[..., 2]) - rgb[..., 1], 0, None) * (alpha < 1)
    rgb[..., 0] -= spill
    rgb[..., 2] -= spill
    ay, ax = np.where(alpha > 0.05)
    if len(ay) == 0:
        sys.exit("the booth is empty: nothing but the key colour")
    rgba = np.dstack([np.clip(rgb, 0, 255), alpha * 255]).astype(np.uint8)
    rgba = rgba[ay.min():ay.max() + 1, ax.min():ax.max() + 1]
    obj = Image.fromarray(rgba, "RGBA")
    inner = int(size * (1 - pad * 2))
    scale = inner / max(obj.size)
    obj = obj.resize((max(1, int(obj.size[0] * scale)), max(1, int(obj.size[1] * scale))), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(obj, ((size - obj.size[0]) // 2, (size - obj.size[1]) // 2), obj)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    canvas.save(out)
    # a preview on a checker so transparency can be judged
    prev = Image.new("RGB", (size, size), (60, 64, 76))
    prev.paste(canvas, (0, 0), canvas)
    prev_path = out[:-4] + "_preview.jpg"
    prev.save(prev_path, quality=85)
    return {"icon": out, "look_at": prev_path, "source_booth": [int(x1 - x0), int(y1 - y0)]}


def _cmd(a):
    out = os.path.join(WORK, a.folder, a.name + ".png")
    print(json.dumps(cut_icon(a.screenshot, out, a.size), indent=1))


def register(sub):
    p = sub.add_parser("icon", help="cut a transparent icon out of an icon-booth screenshot")
    p.add_argument("screenshot")
    p.add_argument("--name", required=True)
    p.add_argument("--size", type=int, default=512)
    p.add_argument("--folder", default="icons")
    p.set_defaults(fn=_cmd)
