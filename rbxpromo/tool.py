"""The promo actions, one function per action, shared by the CLI (rbxpromo.cli) and the MCP tool (rbx_promo).

Each action returns a JSON-able dict, or raises ValueError (bad input), RecipeError or engine.EngineError.
Nothing here prints: both front ends decide how to show the result.
"""
import datetime, json, os

import rbxpaths
from rbxpromo import engine

# actions that need the whole engine installed
NEEDS_ENGINE = ("probe", "reference", "voice", "transcribe", "render", "check", "styles")


def scratch_dir(kind):
    """A fresh folder under WORK for sheets and stills that do not belong to a promo folder."""
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    return os.path.join(rbxpaths.WORK, "promo-work", "%s-%s" % (kind, stamp))


def _need(args, key, what):
    v = args.get(key)
    if not v:
        raise ValueError("Give '%s': %s." % (key, what))
    return v


def do_status(args, on_progress=None):
    return engine.status()


def do_probe(args, on_progress=None):
    from rbxpromo import probe
    paths = args.get("paths") or ([args["path"]] if args.get("path") else [])
    if not paths:
        raise ValueError("Give 'paths': the clips to probe.")
    for p in paths:
        if not os.path.isfile(p):
            raise ValueError("Clip not found: %s" % p)
    engine.require_ready()
    return probe.probe(paths, args.get("out") or scratch_dir("probe"), on_progress)


def do_reference(args, on_progress=None):
    from rbxpromo import reference
    return reference.reference(_need(args, "path", "the saved reference video file"), args.get("out") or scratch_dir("reference"))


def styles_sheet(items, out_jpg, cols=4, cell=(270, 480), label=34):
    """One contact sheet of every style's still, each labelled with its name."""
    from PIL import Image, ImageDraw
    rows = (len(items) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * cell[0], rows * (cell[1] + label)), (20, 20, 20))
    draw = ImageDraw.Draw(sheet)
    for i, it in enumerate(items):
        x, y = (i % cols) * cell[0], (i // cols) * (cell[1] + label)
        with Image.open(it["still"]) as im:
            sheet.paste(im.convert("RGB").resize(cell), (x, y + label))
        draw.text((x + 8, y + 10), it["name"], fill=(255, 255, 255))
    sheet.save(out_jpg, quality=88)
    return out_jpg


def do_styles(args, on_progress=None):
    """Every style with a still. The default folder is stable so reference.styles' per-hash cache works across calls."""
    from rbxpromo import reference
    engine.require_ready()
    out = args.get("out") or os.path.join(rbxpaths.WORK, "promo-work", "styles")
    items = reference.styles(out)
    return {"styles": items, "sheet": styles_sheet(items, os.path.join(out, "sheet_styles.jpg"))}


def do_voice(args, on_progress=None):
    from rbxpromo import audio
    text = _need(args, "text", "the words to speak")
    out = args.get("out") or os.path.join(scratch_dir("voice"), "voice.wav")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    kw = {"voice": args["voice"]} if args.get("voice") else {}
    return audio.voice(text, out, **kw)


def do_transcribe(args, on_progress=None):
    from rbxpromo import audio
    path = _need(args, "path", "the voice or music file")
    if not os.path.isfile(path):
        raise ValueError("File not found: %s" % path)
    engine.require_ready()
    return audio.transcribe(path, args.get("lang") or "en")


def do_render(args, on_progress=None):
    from rbxpromo import render
    recipe = _need(args, "recipe", "the path of the promo.json recipe")
    return render.render(recipe, args.get("out") or None, on_progress)


def do_check(args, on_progress=None):
    """Re-run the self-check on a rendered promo. The layout and recipe default to the files render wrote beside the video."""
    from rbxpromo import check
    from rbxpromo.recipe import load_recipe
    video = _need(args, "path", "the rendered promo.mp4")
    folder = os.path.dirname(os.path.abspath(video))
    layout = args.get("layout") or os.path.join(folder, "layout.json")
    recipe = args.get("recipe") or os.path.join(folder, "promo.json")
    for what, p in (("layout", layout), ("recipe", recipe)):
        if not os.path.isfile(p):
            raise ValueError("The %s file %s does not exist. Check a video that render made (its folder holds both), "
                             "or pass 'recipe'%s." % (what, p, " and 'layout'" if what == "layout" else ""))
    engine.require_ready()
    with open(layout, encoding="utf-8") as f:
        lay = json.load(f)
    return check.check(video, lay, load_recipe(recipe))


ACTIONS = {"status": do_status, "probe": do_probe, "reference": do_reference, "styles": do_styles, "voice": do_voice,
           "transcribe": do_transcribe, "render": do_render, "check": do_check}


def problems_text(e):
    """The message to show for an error: every recipe problem on its own line."""
    from rbxpromo.recipe import RecipeError
    if isinstance(e, RecipeError):
        return "The recipe has %d problem%s:\n" % (len(e.problems), "" if len(e.problems) == 1 else "s") + \
               "\n".join("- " + p for p in e.problems)
    return str(e)
