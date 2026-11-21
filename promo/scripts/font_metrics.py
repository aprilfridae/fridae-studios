"""Regenerate promo/src/captions/metrics.json: per-character advance widths (in em) of every
bundled font in promo/public/fonts, so captions can be laid out without a browser.

Run with a Python that has Pillow:  python promo/scripts/font_metrics.py
"""
import json, os, sys
from PIL import ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.join(HERE, "..", "public", "fonts")
OUT = os.path.join(HERE, "..", "src", "captions", "metrics.json")
SIZE = 1000
# Basic Latin, Latin-1, Latin Extended-A, and symbols captions use
CHARS = [chr(c) for c in range(0x20, 0x7F)] + [chr(c) for c in range(0xA0, 0x180)] + list("€£₹—–…‘’“”•✓→")
WEIGHTS = {"LibreBaskerville-wght.ttf": 500}  # variable fonts: the instance the styles use


def metrics(path, name):
    font = ImageFont.truetype(path, SIZE, layout_engine=ImageFont.Layout.BASIC)
    if name in WEIGHTS:
        font.set_variation_by_axes([WEIGHTS[name]])
    adv = {}
    for ch in CHARS:
        w = font.getlength(ch)
        adv[ch] = round(w / SIZE, 4)
    return {"default": round(font.getlength("n") / SIZE, 4), "adv": adv}


def main():
    out = {}
    for name in sorted(os.listdir(FONTS)):
        if name.endswith(".ttf"):
            out[name] = metrics(os.path.join(FONTS, name), name)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        f.write("\n")
    print("wrote", OUT, len(out), "fonts", file=sys.stderr)


if __name__ == "__main__":
    main()
