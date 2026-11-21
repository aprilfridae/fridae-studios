"""`rbx promo ...`: argument parsing and printing only; the work is in rbxpromo.tool and the engine modules.

Machine actions print JSON to stdout. Errors exit non-zero with the message on stderr.
"""
import json, os, sys


def _show(data):
    print(json.dumps(data, indent=1, ensure_ascii=False))


def _fail(e):
    from rbxpromo.tool import problems_text
    sys.exit(problems_text(e))


def _guarded(fn):
    """Run an action; any expected failure becomes a message on stderr and a non-zero exit."""
    def run(a):
        from rbxpaths import KitError
        from rbxpromo.recipe import RecipeError
        try:
            return fn(a)
        except (KitError, RecipeError, ValueError, FileNotFoundError, RuntimeError) as e:
            _fail(e)
    return run


def _status(a):
    from rbxpromo import engine
    st = engine.status()
    if a.json:
        return _show(st)
    for k in ("node", "renderer", "browser", "whisper", "voice", "missing", "download_mb"):
        print("%s: %s" % (k, st[k]))
    print("ready: " + str(st["ready"]).lower())


def _install(a):
    from rbxpromo import engine
    st = engine.status()
    if not st["ready"] and not a.json_progress:
        print("Downloading about %s MB into %s" % (st["download_mb"], engine.engine_dir()), flush=True)
    seen = {}

    def progress(step, fraction):
        if a.json_progress:
            print(json.dumps({"step": step, "fraction": round(fraction, 4)}), flush=True)
            return
        decile = int(fraction * 10)
        if seen.get(step) != decile:
            seen[step] = decile
            print("%s: %d%%" % (step, int(fraction * 100)), flush=True)

    done = engine.install(progress)
    if a.json_progress:
        print(json.dumps({"ready": done["ready"], "missing": done["missing"]}), flush=True)
    else:
        print("ready: " + str(done["ready"]).lower())


def _action(name, **fixed):
    def run(a):
        from rbxpromo import tool
        args = {k: getattr(a, k, None) for k in ("paths", "path", "recipe", "text", "voice", "out", "layout", "lang")}
        args.update(fixed)
        step = {"last": -1}

        def progress(x):
            tenth = int(x * 10)
            if tenth != step["last"]:
                step["last"] = tenth
                print("%s: %d%%" % (name, int(x * 100)), file=sys.stderr, flush=True)
        _show(tool.ACTIONS[name](args, progress))
    return _guarded(run)


def register(sub):
    p = sub.add_parser("promo", help="vertical promo videos for a game: probe clips, render, check")
    s = p.add_subparsers(dest="promo_cmd", required=True)

    x = s.add_parser("status", help="is the promo engine installed")
    x.add_argument("--json", action="store_true")
    x.set_defaults(fn=_guarded(_status))

    x = s.add_parser("install", help="download and install the promo engine (resumes where it stopped)")
    x.add_argument("--json-progress", action="store_true", help="one JSON line per progress step, for the app")
    x.set_defaults(fn=_guarded(_install))

    x = s.add_parser("probe", help="read clips: length, cuts, loud moments, a frame sheet each")
    x.add_argument("paths", nargs="+")
    x.add_argument("--out", help="folder for the frame sheets")
    x.set_defaults(fn=_action("probe"))

    x = s.add_parser("reference", help="break a saved reference video into shots, words and beats")
    x.add_argument("path")
    x.add_argument("--out", help="folder for the sheet")
    x.set_defaults(fn=_action("reference"))

    x = s.add_parser("voice", help="speak text with the local voice")
    x.add_argument("text")
    x.add_argument("--voice", help="voice name (default af_heart)")
    x.add_argument("--out", help="wav file to write")
    x.set_defaults(fn=_action("voice"))

    x = s.add_parser("transcribe", help="word timings of a voice or song file")
    x.add_argument("path")
    x.add_argument("--lang", default="en")
    x.set_defaults(fn=_action("transcribe"))

    x = s.add_parser("render", help="render a promo.json recipe, then check it")
    x.add_argument("recipe")
    x.add_argument("--out", help="output folder (default: a new folder under work/promos)")
    x.set_defaults(fn=_action("render"))

    x = s.add_parser("check", help="run the safe-area, hook, contrast and loudness checks on a rendered promo")
    x.add_argument("path", help="the promo.mp4")
    x.add_argument("--recipe", help="default: promo.json beside the video")
    x.add_argument("--layout", help="default: layout.json beside the video")
    x.set_defaults(fn=_action("check"))

    x = s.add_parser("styles", help="list the styles with a still of each")
    x.add_argument("--out", help="folder for the stills")
    x.set_defaults(fn=_action("styles"))
