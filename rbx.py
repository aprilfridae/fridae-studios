#!/usr/bin/env python
"""rbxkit command line.

  doctor    check that everything the kit needs is installed
  blender   run Python in headless Blender against a .blend file
  tex       draw VFX textures and flipbooks (white on transparent)
  sheet     check PNGs and tile them on a dark background
  icon      cut a transparent icon out of an icon-booth screenshot
  video     video metadata and frame contact sheets
  anim      turn a baked animation into a Roblox .rbxmx
  package   build the VFX modules for local loading in Studio
  gen       generate a texture with Gemini (needs a Gemini API key)
  studio    run Luau in the open Roblox Studio and print the result
  project   export the place's scripts to the project folder, checkpoint and restore them
  economy   simulate progression numbers or a drop table from a JSON file
  market    look up live Roblox experiences (public data)
  promo     vertical promo videos: probe clips, render, check (needs the promo engine)
  rag       the knowledge index: its local search service, setup steps and check
  where     print the paths the kit uses
"""
import argparse, glob, importlib, json, os, shutil, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rbxpaths import KIT, WORK, CONFIG, FROZEN, KitError, no_window


def load_config():
    try:
        with open(CONFIG, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def set_blender(path):
    """Write only the Blender path, into the settings as they are on disk right now (read just before the file is
    replaced, so a change the app made meanwhile is kept). A settings file that exists but does not read is left
    alone: nothing in it is lost."""
    try:
        with open(CONFIG, encoding="utf-8") as f:
            cfg = json.load(f)
    except OSError:
        cfg = {}
    except ValueError:
        print("  config.json could not be read, so the Blender path was not saved in it: fix or remove it, then run rbx doctor again.")
        return
    if not isinstance(cfg, dict) or cfg.get("blender") == path:
        return
    cfg["blender"] = path
    save_config(cfg)


def save_config(cfg):
    """Replace config.json whole (a temporary file renamed over it), so the app never reads half a file."""
    os.makedirs(os.path.dirname(CONFIG), exist_ok=True)
    tmp = "%s.%d.tmp" % (CONFIG, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(cfg, f, indent=1)
    os.replace(tmp, CONFIG)


def _registry_blender():
    """Blender install folders recorded by its Windows installer."""
    if os.name != "nt":
        return []
    import winreg
    found = []
    roots = [(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
             (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
             (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall")]
    for hive, path in roots:
        try:
            key = winreg.OpenKey(hive, path)
        except OSError:
            continue
        for i in range(winreg.QueryInfoKey(key)[0]):
            try:
                sub = winreg.OpenKey(key, winreg.EnumKey(key, i))
                name = str(winreg.QueryValueEx(sub, "DisplayName")[0])
                if not name.lower().startswith("blender"):
                    continue
                loc = str(winreg.QueryValueEx(sub, "InstallLocation")[0])
                exe = os.path.join(loc, "blender.exe")
                if os.path.exists(exe):
                    found.append(exe)
            except OSError:
                continue
    return found


def find_blender(required=True):
    env = os.environ.get("RBXKIT_BLENDER")
    if env and os.path.exists(env):
        return env
    # run by the app: the Blender it checked is the one (above), never one named in the settings file
    from_app = os.environ.get("FRIDAE_SETTINGS_FROM_APP") == "1"
    cfg = None if from_app else load_config().get("blender")
    if cfg and os.path.exists(cfg):
        return cfg
    found = shutil.which("blender")
    if found:
        return found
    hits = _registry_blender()
    if not hits:
        pats = [
            r"C:\Program Files\Blender Foundation\Blender*\blender.exe",
            os.path.expandvars(r"%LOCALAPPDATA%\Programs\Blender*\blender.exe"),
            r"?:\Program Files (x86)\Steam\steamapps\common\Blender\blender.exe",
            r"?:\SteamLibrary\steamapps\common\Blender\blender.exe",
            "/Applications/Blender.app/Contents/MacOS/Blender",
            "/usr/bin/blender", "/snap/bin/blender",
        ]
        for p in pats:
            hits += sorted(glob.glob(p))
    if hits:
        return hits[-1]
    if required:
        sys.exit("Blender not found. Install it from blender.org, then run: rbx doctor\n"
                 "(or set it by hand: rbx doctor --blender \"C:\\path\\to\\blender.exe\")")
    return None


def model_file(name):
    return os.path.join(WORK, "models", name, name + ".blend")


def cmd_blender(a):
    blend = a.file or (model_file(a.model) if a.model else None)
    if blend:
        blend = os.path.abspath(blend)
    code = a.code
    if a.script == "-":
        code = sys.stdin.read()
    opts = {"file": blend, "code": code, "script": None if a.script == "-" else a.script,
            "nosave": a.nosave, "work": WORK}
    if not opts["code"] and not opts["script"]:
        sys.exit("give --code, --script FILE, or --script - to read stdin")
    cmd = [find_blender(), "--background", "--factory-startup"]
    if blend and os.path.exists(blend):
        cmd.append(blend)
    cmd += ["--python-exit-code", "1", "--python", os.path.join(KIT, "blender", "runner.py"), "--", json.dumps(opts)]
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", creationflags=no_window())
    res = None
    shown = []
    for line in p.stdout.splitlines():
        if line.startswith("RBXRESULT "):
            res = json.loads(line[10:])
        elif (line.strip() and not line.startswith(("Blender ", "Read blend:", "Fra:", "Info:", "Time:", " Time:", "FBX export"))
              and "| Saved: " not in line and "| Read blend: " not in line and "export finished" not in line):
            shown.append(line)
    if shown:
        print("\n".join(shown[-a.tail:]))
    if res is None:
        print(p.stdout[-3000:])
        print(p.stderr[-3000:], file=sys.stderr)
        sys.exit("Blender did not return a result (exit %s)" % p.returncode)
    if not res["ok"]:
        print(res["error"], file=sys.stderr)
        sys.exit(1)
    print(json.dumps({k: v for k, v in res.items() if k != "ok"}, indent=1))


def cmd_where(a):
    from rbxpaths import VERSION
    print(json.dumps({"version": VERSION, "kit": KIT, "work": WORK, "blender": find_blender(required=False)}, indent=1))


def cmd_doctor(a):
    """Check requirements and print what is missing, in plain words."""
    ok = True
    lines = []

    def row(good, name, detail, fix=None):
        nonlocal ok
        lines.append("  [%s] %-18s %s" % ("ok" if good else "!!", name, detail))
        if not good:
            ok = False
            if fix:
                lines.append("       fix: " + fix)

    if FROZEN:
        lines.append("  [ok] %-18s built in" % "Python")
    else:
        row(sys.version_info >= (3, 9), "Python", sys.version.split()[0], "install Python 3.9 or newer from python.org")
    for mod, pipname in (("numpy", "numpy"), ("PIL", "pillow"), ("cv2", "opencv-python")):
        try:
            m = __import__(mod)
            row(True, pipname, getattr(m, "__version__", "installed"))
        except ImportError:
            row(False, pipname, "missing", "python -m pip install " + pipname)

    if a.blender:
        if os.path.exists(a.blender):
            set_blender(os.path.abspath(a.blender))
        else:
            row(False, "Blender (given)", a.blender + " does not exist")
    exe = find_blender(required=False)
    if exe:
        try:
            out = subprocess.run([exe, "--version"], capture_output=True, text=True, timeout=60, creationflags=no_window()).stdout.splitlines()
            ver = next((l for l in out if l.startswith("Blender")), "found")
        except Exception as e:
            ver = "found but did not run: %s" % e
        row(True, "Blender", "%s  (%s)" % (ver, exe))
        set_blender(exe)
    else:
        row(False, "Blender", "not found", "install from blender.org, or: rbx doctor --blender <path to blender.exe>")

    base = os.path.expandvars(r"%LOCALAPPDATA%\Roblox\Versions")
    studio = [d for d in glob.glob(os.path.join(base, "*")) if os.path.exists(os.path.join(d, "RobloxStudioBeta.exe"))]
    if sys.platform == "darwin" and os.path.exists("/Applications/RobloxStudio.app"):
        studio = ["/Applications/RobloxStudio.app"]
    lines.append("  [%s] %-18s %s" % ("ok" if studio else "--", "Roblox Studio",
                 "found" if studio else "not found on this computer (needed for texture previews and local module loading)"))

    try:
        from rbxpromo import engine
        st = engine.status()
        detail = ("ready" if st["ready"] else "not installed (missing: %s; about %s MB to download; install it in the app's Setup, "
                  "or: rbx promo install)" % (", ".join(st["missing"]), st["download_mb"]))
        lines.append("  [%s] %-18s %s" % ("ok" if st["ready"] else "--", "Promo engine", detail))
    except Exception as e:  # never a download or a failure of doctor itself
        lines.append("  [--] %-18s could not be checked: %s" % ("Promo engine", e))

    skills = os.path.join(os.path.expanduser("~"), ".claude", "skills")
    have = [n for n in ("roblox-studio-workflow", "roblox-playtest", "roblox-vfx", "roblox-ui-design")
            if os.path.exists(os.path.join(skills, n, "SKILL.md"))]
    if not FROZEN:
        row(len(have) == 4, "Claude skills", "%d of 4 installed in %s" % (len(have), skills), "run install.bat (or: python install.py)")
    try:
        os.makedirs(WORK, exist_ok=True)
        probe = os.path.join(WORK, ".write_test")
        open(probe, "w").close()
        os.remove(probe)
        row(True, "Work folder", WORK)
    except OSError as e:
        row(False, "Work folder", "%s is not writable: %s" % (WORK, e))
    print("rbxkit doctor\n" + "\n".join(lines))
    print("\nEverything needed is in place." if ok else "\nSome things need fixing (marked !!).")
    print("In Claude, the Roblox Studio MCP connector must also be connected, with Studio open.")
    sys.exit(0 if ok else 1)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="rbx", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("doctor", help="check that everything the kit needs is installed")
    d.add_argument("--blender", help="path to blender.exe to remember")
    d.set_defaults(fn=cmd_doctor)

    b = sub.add_parser("blender", help="run Python in headless Blender")
    b.add_argument("--model", help="model name; uses work/models/<name>/<name>.blend")
    b.add_argument("--file", help="explicit .blend path (created if missing)")
    b.add_argument("--code", help="Python source to run")
    b.add_argument("--script", help="Python file to run, or - for stdin")
    b.add_argument("--nosave", action="store_true", help="do not save the .blend afterwards")
    b.add_argument("--tail", type=int, default=60, help="max printed output lines")
    b.set_defaults(fn=cmd_blender)

    w = sub.add_parser("where", help="print kit paths")
    w.set_defaults(fn=cmd_where)

    names = {"rbxtex": "tex", "rbximg": "icon", "rbxvideo": "video", "rbxpack": "package", "rbxgen": "gen", "rbximage": "image",
             "rbxstudio": "studio", "rbxproject": "project", "rbxeconomy": "economy", "rbxmarket": "market", "rbxresearch": "research",
             "rbxpromo.cli": "promo", "rbxrag.cli": "rag", "rbxstage": "stage"}
    for mod, name in names.items():
        try:
            importlib.import_module(mod).register(sub)
        except ImportError as e:
            # a missing library (numpy, pillow, opencv) only disables that command
            missing = sub.add_parser(name, help="unavailable: %s (run: rbx doctor)" % e)
            missing.add_argument("rest", nargs="*")
            missing.set_defaults(fn=lambda a, e=e: sys.exit("This command needs a missing library: %s\nRun: rbx doctor" % e))

    if sys.stdout is not None:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    a = ap.parse_args(argv)
    try:
        a.fn(a)
    except KitError as e:  # the studio, project, economy and market commands fail with a message to show as it is
        sys.exit(str(e))


if __name__ == "__main__":
    sys.path.insert(0, KIT)
    main()
