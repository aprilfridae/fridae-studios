"""Build Roblox XML files (.rbxmx) for rbxkit: the VFX module package and
KeyframeSequence animations."""
import json, os, sys
from xml.sax.saxutils import escape

from rbxpaths import KIT, WORK
HEAD = '<roblox xmlns:xmime="http://www.w3.org/2005/05/xmlmime" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:noNamespaceSchemaLocation="http://www.roblox.com/roblox.xsd" version="4">\n'
_ref = [0]


def _item(cls, name, props="", children=""):
    _ref[0] += 1
    return ('<Item class="%s" referent="RBX%d"><Properties><string name="Name">%s</string>%s</Properties>%s</Item>\n'
            % (cls, _ref[0], escape(name), props, children))


def _module(name, source):
    src = source.replace("]]>", "]]]]><![CDATA[>")
    return _item("ModuleScript", name, '<ProtectedString name="Source"><![CDATA[%s]]></ProtectedString>' % src)


def build_vfx_package(out=None):
    out = out or os.path.join(WORK, "package", "RbxKitVFX.rbxmx")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    mods = ""
    for name in ("Emit", "Lightning"):
        mods += _module(name, open(os.path.join(KIT, "studio", name + ".lua"), encoding="utf-8").read())
    with open(out, "w", encoding="utf-8") as f:
        f.write(HEAD + _item("Folder", "VFX", "", mods) + "</roblox>\n")
    return out


def _cframe(name, c):
    # c = [x, y, z, r00, r01, r02, r10, r11, r12, r20, r21, r22]
    tags = ["X", "Y", "Z", "R00", "R01", "R02", "R10", "R11", "R12", "R20", "R21", "R22"]
    return '<CoordinateFrame name="%s">%s</CoordinateFrame>' % (name, "".join("<%s>%.6g</%s>" % (t, v, t) for t, v in zip(tags, c)))


IDENTITY = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]
PRIORITY = {"Idle": 0, "Movement": 1, "Action": 2, "Action2": 3, "Action3": 4, "Action4": 5, "Core": 1000}


def build_animation(data, out):
    """data = {"name", "loop", "priority", "tree": {part: parent or None}, "frames": [{"time", "poses": {part: [12 floats]}}]}"""
    tree = data["tree"]
    kids = {}
    root = None
    for part, parent in tree.items():
        if parent is None:
            root = part
        else:
            kids.setdefault(parent, []).append(part)

    def pose(part, poses):
        inner = "".join(pose(k, poses) for k in kids.get(part, []))
        moved = part in poses
        props = _cframe("CFrame", poses.get(part, IDENTITY)) + '<float name="Weight">%d</float>' % (1 if moved else 0)
        props += '<token name="EasingStyle">0</token><token name="EasingDirection">0</token>'
        return _item("Pose", part, props, inner)

    frames = ""
    for i, fr in enumerate(data["frames"]):
        frames += _item("Keyframe", "Keyframe%d" % i, '<float name="Time">%.5f</float>' % fr["time"], pose(root, fr["poses"]))
    props = '<bool name="Loop">%s</bool><token name="Priority">%d</token>' % (
        "true" if data.get("loop") else "false", PRIORITY.get(data.get("priority", "Action"), 2))
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(HEAD + _item("KeyframeSequence", data.get("name", "Animation"), props, frames) + "</roblox>\n")
    return out


def stage(path):
    """Copy a .rbxmx into this computer's Studio content folder so Studio can
    load it locally with game:GetObjects (no upload). Returns the rbxasset url."""
    import glob, shutil
    base = os.path.expandvars(r"%LOCALAPPDATA%\Roblox\Versions")
    hits = [d for d in glob.glob(os.path.join(base, "*")) if os.path.exists(os.path.join(d, "RobloxStudioBeta.exe"))]
    if not hits:
        return None
    dest = os.path.join(max(hits, key=os.path.getmtime), "content", "rbxkit")
    os.makedirs(dest, exist_ok=True)
    shutil.copyfile(path, os.path.join(dest, os.path.basename(path)))
    return "rbxasset://rbxkit/" + os.path.basename(path)


def _cmd_package(a):
    path = build_vfx_package()
    out = {"package": path}
    url = stage(path)
    if url:
        out["local_url"] = url
        out["install_luau"] = ('local objs = game:GetObjects("%s") local vfx = game.ReplicatedStorage:FindFirstChild("VFX") '
                               'or Instance.new("Folder") vfx.Name = "VFX" vfx.Parent = game.ReplicatedStorage '
                               'local added = {} for _, m in ipairs(objs[1]:GetChildren()) do if not vfx:FindFirstChild(m.Name) then '
                               'm.Parent = vfx table.insert(added, m.Name) end end return "added: " .. table.concat(added, ", ")') % url
    print(json.dumps(out, indent=1))


def _cmd_anim(a):
    data = json.load(open(a.json, encoding="utf-8"))
    out = a.out or os.path.splitext(a.json)[0] + ".rbxmx"
    print(json.dumps({"animation": build_animation(data, out), "frames": len(data["frames"])}))


def register(sub):
    p = sub.add_parser("package", help="build work/package/RbxKitVFX.rbxmx (Emit + Lightning modules)")
    p.set_defaults(fn=_cmd_package)
    an = sub.add_parser("anim", help="turn an animation JSON baked by rbx.bake_animation into a .rbxmx KeyframeSequence")
    an.add_argument("json")
    an.add_argument("--out")
    an.set_defaults(fn=_cmd_anim)
