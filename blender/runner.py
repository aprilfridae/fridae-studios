"""Runs inside Blender (started by rbx.py). Executes user code with the `rbx`
helper module available, then saves the .blend and prints a JSON result line."""
import bpy, sys, os, json, traceback

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
opts = json.loads(argv[0]) if argv else {}
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rbxb as rbx  # noqa: E402
if opts.get("work"):
    rbx.WORK = opts["work"]

blend = opts.get("file")
if blend and not os.path.exists(blend):
    # new file: start from an empty scene
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    for c in list(bpy.data.collections):
        bpy.data.collections.remove(c)

scope = {"bpy": bpy, "rbx": rbx, "result": None, "WORK": opts.get("work", "")}
out = {"ok": True}
try:
    src = opts.get("code") or open(opts["script"], encoding="utf-8").read()
    exec(compile(src, opts.get("script") or "<code>", "exec"), scope)
    out["result"] = scope.get("result")
except Exception:
    out = {"ok": False, "error": traceback.format_exc()}

if blend and out["ok"] and not opts.get("nosave"):
    os.makedirs(os.path.dirname(blend), exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=blend, check_existing=False)
    out["saved"] = blend

print("RBXRESULT " + json.dumps(out, default=str))
sys.stdout.flush()
if not out["ok"]:
    sys.exit(1)
