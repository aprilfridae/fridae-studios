"""rbxkit Blender helpers. Available as `rbx` inside code run by `rbx.py blender`.

Conventions: 1 Blender unit = 1 stud, +Z up, model front faces -Y.
One collection per model, named after the model.
"""
import bpy, bmesh, os, json, math
from mathutils import Vector

WORK = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "work")

VIEWS = {
    "iso": (0.72, -0.72, 0.5),
    "iso_back": (-0.72, 0.72, 0.5),
    "iso_left": (-0.72, -0.72, 0.5),
    "front": (0, -1, 0.12),
    "back": (0, 1, 0.12),
    "left": (-1, 0, 0.12),
    "right": (1, 0, 0.12),
    "top": (0, -0.05, 1),
    "low": (0.72, -0.72, 0.1),
}


# ---------------------------------------------------------------- selection
def collection(name):
    """Get or create a collection linked to the scene."""
    col = bpy.data.collections.get(name)
    if not col:
        col = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(col)
    return col


def add(obj, coll):
    """Link an object to a collection (by name), unlinking it elsewhere."""
    col = collection(coll) if isinstance(coll, str) else coll
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    col.objects.link(obj)
    return obj


def meshes(collection=None, objects=None):
    if objects:
        names = [o if isinstance(o, str) else o.name for o in objects]
        return [bpy.data.objects[n] for n in names if bpy.data.objects[n].type == "MESH"]
    if collection:
        col = bpy.data.collections[collection] if isinstance(collection, str) else collection
        return [o for o in col.all_objects if o.type == "MESH"]
    return [o for o in bpy.context.scene.objects if o.type == "MESH"]


# ------------------------------------------------------------------- bounds
def _eval_mesh_points(obj):
    dg = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(dg)
    me = ev.to_mesh()
    mw = obj.matrix_world
    pts = [mw @ v.co for v in me.vertices]
    ev.to_mesh_clear()
    return pts


def bounds(obj_or_list):
    """World-space (min, max) Vectors of one object or a list, modifiers applied."""
    objs = obj_or_list if isinstance(obj_or_list, (list, tuple)) else [obj_or_list]
    lo = Vector((1e18,) * 3)
    hi = Vector((-1e18,) * 3)
    for o in objs:
        for p in _eval_mesh_points(o):
            for i in range(3):
                lo[i] = min(lo[i], p[i])
                hi[i] = max(hi[i], p[i])
    return lo, hi


def _islands(obj, limit=60000):
    """World AABBs of each loose piece of a mesh object: [(label, lo, hi)]."""
    dg = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(dg)
    me = ev.to_mesh()
    mw = obj.matrix_world
    out = []
    if len(me.vertices) == 0:
        ev.to_mesh_clear()
        return out
    if len(me.vertices) > limit:
        pts = [mw @ v.co for v in me.vertices]
        ev.to_mesh_clear()
        lo = Vector([min(p[i] for p in pts) for i in range(3)])
        hi = Vector([max(p[i] for p in pts) for i in range(3)])
        return [(obj.name, lo, hi)]
    bm = bmesh.new()
    bm.from_mesh(me)
    ev.to_mesh_clear()
    seen = set()
    for v in bm.verts:
        if v.index in seen:
            continue
        stack, lo, hi = [v], Vector((1e18,) * 3), Vector((-1e18,) * 3)
        seen.add(v.index)
        count = 0
        while stack:
            cur = stack.pop()
            count += 1
            p = mw @ cur.co
            for i in range(3):
                lo[i] = min(lo[i], p[i])
                hi[i] = max(hi[i], p[i])
            for e in cur.link_edges:
                o2 = e.other_vert(cur)
                if o2.index not in seen:
                    seen.add(o2.index)
                    stack.append(o2)
        if count >= 3:
            out.append((obj.name, lo, hi))
    bm.free()
    if len(out) > 1:
        out = [("%s#%d" % (n, i + 1), lo, hi) for i, (n, lo, hi) in enumerate(out)]
    return out


def _gap(a, b):
    """Per-axis separation vector between two AABBs (0 on overlapping axes)."""
    d = Vector((0, 0, 0))
    for i in range(3):
        if a[1][i] < b[0][i]:
            d[i] = b[0][i] - a[1][i]
        elif b[1][i] < a[0][i]:
            d[i] = -(a[0][i] - b[1][i])
    return d


def check_model(collection=None, objects=None, tol=0.002):
    """Find floating or detached pieces. Prints and returns a report.

    A piece is flagged when its bounding box does not touch or overlap any
    other piece. The report gives the move that closes the gap. A clean report
    says nothing about shape: still look at renders.
    """
    objs = meshes(collection, objects)
    pieces = []
    for o in objs:
        pieces.extend(_islands(o))
    lines, problems = [], []
    for name, lo, hi in pieces:
        size = hi - lo
        lines.append("%-24s min(%.2f, %.2f, %.2f) max(%.2f, %.2f, %.2f) size(%.2f, %.2f, %.2f)"
                     % (name, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, size.x, size.y, size.z))
    if len(pieces) > 1:
        for i, a in enumerate(pieces):
            best, best_d, best_n = None, 1e18, None
            for j, b in enumerate(pieces):
                if i == j:
                    continue
                g = _gap((a[1], a[2]), (b[1], b[2]))
                if g.length < best_d:
                    best, best_d, best_n = g, g.length, b[0]
            if best_d > tol:
                problems.append("FLOATING %s: %.3f from %s. Move it by (%.3f, %.3f, %.3f) plus a small overlap."
                                % (a[0], best_d, best_n, best.x, best.y, best.z))
    if pieces:
        lo = Vector([min(p[1][i] for p in pieces) for i in range(3)])
        hi = Vector([max(p[2][i] for p in pieces) for i in range(3)])
        size = hi - lo
        lines.append("WHOLE min(%.2f, %.2f, %.2f) max(%.2f, %.2f, %.2f) size(%.2f, %.2f, %.2f) studs"
                     % (lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, size.x, size.y, size.z))
        if size.y > size.z * 2.5 and size.y > size.x * 1.5:
            problems.append("AXES? The model is longest on Y. Height belongs on Z (+Z up, front faces -Y).")
        if abs(lo.z) > 0.05:
            problems.append("BASE: lowest point is z=%.2f. Rest the model on z=0 so it sits on the ground when placed." % lo.z)
    report = "\n".join(lines + (problems or ["nothing flagged (still look at the renders)"]))
    print(report)
    return {"pieces": len(pieces), "problems": problems, "report": report}


# ------------------------------------------------------------------ renders
def _camera(name="RbxCam"):
    cam = bpy.data.objects.get(name)
    if not cam:
        cam = bpy.data.objects.new(name, bpy.data.cameras.new(name))
        bpy.context.scene.collection.objects.link(cam)
    bpy.context.scene.camera = cam
    return cam


def _aim(cam, lo, hi, view, ortho=False, margin=1.15, aspect=1.0):
    d = Vector(VIEWS[view] if isinstance(view, str) else view).normalized()
    centre = (lo + hi) / 2
    radius = max((hi - lo).length / 2, 0.01)
    if ortho:
        cam.data.type = "ORTHO"
        cam.data.ortho_scale = radius * 2 * margin
        dist = radius * 4 + 1
    else:
        cam.data.type = "PERSP"
        cam.data.lens = 50
        fov = 2 * math.atan((cam.data.sensor_width / 2) / cam.data.lens)
        if aspect < 1:
            fov = 2 * math.atan(math.tan(fov / 2) * aspect)
        dist = radius * margin / math.sin(fov / 2)
    cam.location = centre + d * dist
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    cam.data.clip_start = max(dist / 1000, 0.001)
    cam.data.clip_end = dist * 10 + radius * 10


def _isolate(objs):
    """Hide every other renderable object; returns a restore function."""
    keep = set(o.name for o in objs)
    changed = []
    for o in bpy.context.scene.objects:
        if o.type in {"MESH", "CURVE", "SURFACE", "FONT", "META"} and o.name not in keep and not o.hide_render:
            o.hide_render = True
            changed.append(o)

    def restore():
        for o in changed:
            o.hide_render = False
    return restore


def _workbench(scene, size, background, transparent):
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x, scene.render.resolution_y = size if isinstance(size, (tuple, list)) else (size, size)
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = transparent
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    sh = scene.display.shading
    sh.light = "STUDIO"
    sh.color_type = "TEXTURE"
    sh.show_cavity = True
    sh.cavity_type = "BOTH"
    sh.show_object_outline = True
    sh.object_outline_color = (0.03, 0.03, 0.04)
    sh.show_shadows = False
    try:
        scene.display.render_aa = "8"
    except Exception:
        pass
    if scene.world is None:
        scene.world = bpy.data.worlds.new("RbxWorld")
    scene.world.color = background
    try:
        scene.view_settings.view_transform = "Standard"
    except Exception:
        pass


def shot(collection=None, objects=None, views=("iso", "iso_back"), out_dir=None, name=None,
         size=768, background=(0.32, 0.34, 0.38), ortho=False, frame=None):
    """Render a model on its own from several views (fast Workbench shading with
    textures, cavity shading and a dark outline). Returns the PNG paths.

    views: names from rbx.VIEWS or direction tuples. objects: a list of object
    names to frame a joint close up instead of the whole collection.
    """
    scene = bpy.context.scene
    if frame is not None:
        scene.frame_set(frame)
    objs = meshes(collection, objects)
    if not objs:
        raise ValueError("nothing to render: no mesh objects in %r" % (collection or objects))
    show = meshes(collection) if (collection and objects) else objs
    restore = _isolate(show)
    lo, hi = bounds(objs)
    base = name or (collection if isinstance(collection, str) else "model")
    out_dir = out_dir or os.path.join(WORK, "models", collection if isinstance(collection, str) else base, "shots")
    os.makedirs(out_dir, exist_ok=True)
    _workbench(scene, size, background, False)
    cam = _camera()
    paths = []
    for v in views:
        _aim(cam, lo, hi, v, ortho=ortho)
        path = os.path.join(out_dir, "%s_%s.png" % (base, v if isinstance(v, str) else "view%d" % len(paths)))
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        paths.append(path)
    restore()
    return paths


def render_icons(collections, out_dir, size=512, view="iso", ortho=True, transparent=True,
                 background=(0.1, 0.1, 0.12), margin=1.1):
    """Render one icon per collection with the same camera style and lighting,
    each framed to fill the image. Writes renders.json listing the files."""
    scene = bpy.context.scene
    os.makedirs(out_dir, exist_ok=True)
    _workbench(scene, size, background, transparent)
    scene.display.shading.show_shadows = False
    cam = _camera()
    done = []
    for cname in collections:
        objs = meshes(cname)
        if not objs:
            continue
        restore = _isolate(objs)
        lo, hi = bounds(objs)
        _aim(cam, lo, hi, view, ortho=ortho, margin=margin)
        path = os.path.join(out_dir, "%s.png" % cname)
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        restore()
        done.append({"name": cname, "file": path})
    with open(os.path.join(out_dir, "renders.json"), "w", encoding="utf-8") as f:
        json.dump(done, f, indent=1)
    return done


# ------------------------------------------------------------------- export
def _select(objs):
    for o in bpy.context.view_layer.objects:
        o.select_set(False)
    for o in objs:
        o.hide_set(False)
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]


def export_model(collection, out_dir=None, name=None):
    """Export a collection as FBX for Roblox, plus a manifest with the size the
    model should have in studs. Returns {"fbx", "manifest", "size", "parts"}.

    Roblox axes: X = Blender X, Y (up) = Blender Z, Z = -Blender Y.
    """
    objs = meshes(collection)
    if not objs:
        raise ValueError("no mesh objects in collection %r" % collection)
    name = name or collection
    out_dir = out_dir or os.path.join(WORK, "models", collection, "export")
    os.makedirs(out_dir, exist_ok=True)
    lo, hi = bounds(objs)
    size = hi - lo
    fbx = os.path.join(out_dir, name + ".fbx")
    _select(objs)
    bpy.ops.export_scene.fbx(
        filepath=fbx, use_selection=True, object_types={"MESH"},
        use_mesh_modifiers=True, use_triangles=True, mesh_smooth_type="FACE",
        path_mode="COPY", embed_textures=True, axis_forward="-Z", axis_up="Y",
        apply_scale_options="FBX_SCALE_ALL", bake_anim=False, add_leaf_bones=False,
    )
    manifest = {
        "name": name,
        "fbx": fbx,
        "size_studs": [round(size.x, 4), round(size.z, 4), round(size.y, 4)],
        "base_offset": round(-lo.z, 4),
        "parts": [o.name for o in objs],
        "triangles": sum(len(o.data.polygons) for o in objs),
    }
    mpath = os.path.join(out_dir, name + ".json")
    with open(mpath, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=1)
    return {"fbx": fbx, "manifest": mpath, "size": manifest["size_studs"], "parts": manifest["parts"]}


# ----------------------------------------------------------------- textures
def pixel_material(name, pixels, width, height, obj=None, nearest=True):
    """Make a material whose Base Color is an image built from a flat RGBA float
    list (bottom row first), and assign it to obj if given. The image is packed
    into the .blend and exported with the model."""
    img = bpy.data.images.new(name + "_tex", width, height, alpha=True)
    img.pixels = pixels
    img.pack()
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes.get("Principled BSDF")
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = img
    tex.interpolation = "Closest" if nearest else "Linear"
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    if obj is not None:
        obj.data.materials.clear()
        obj.data.materials.append(mat)
    return mat


def color_material(name, rgb, obj=None):
    """A plain single-colour material (imports as a solid colour)."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (rgb[0], rgb[1], rgb[2], 1)
    mat.diffuse_color = (rgb[0], rgb[1], rgb[2], 1)
    if obj is not None:
        obj.data.materials.clear()
        obj.data.materials.append(mat)
    return mat


def smart_uv(obj):
    _select([obj])
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project()
    bpy.ops.object.mode_set(mode="OBJECT")


def finish(collection, merge_dist=0.0005):
    """Merge duplicate vertices and recalculate normals on every mesh."""
    for o in meshes(collection):
        bm = bmesh.new()
        bm.from_mesh(o.data)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=merge_dist)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(o.data)
        bm.free()


# --------------------------------------------------------------- animation
# Roblox (x, y up, z) -> Blender (x, -z, y up). A proper rotation, so a
# transform written in a joint's own axes is the same numbers in both.
from mathutils import Matrix

_R2B = Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1)))


def _cf_matrix(c):
    """12 CFrame components (x, y, z, R00..R22) -> 4x4 Matrix in Roblox axes."""
    return Matrix(((c[3], c[4], c[5], c[0]), (c[6], c[7], c[8], c[1]), (c[9], c[10], c[11], c[2]), (0, 0, 0, 1)))


def _matrix_cf(m):
    return [round(v, 5) for v in (m[0][3], m[1][3], m[2][3], m[0][0], m[0][1], m[0][2],
                                  m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2])]


def import_rig(path, lift=True):
    """Build an armature from a rig JSON (studio/export_rig.lua, or rigs/R15.json).

    One bone per joint, named after the part it moves, with the bone's own
    axes equal to the joint's Roblox axes (X right, Y up, Z back), so
    pose_bone.matrix_basis is the joint's Transform. Each part is a box
    parented to its bone. The character faces Blender +Y, so its front shows
    in the "back" and "iso_back" views. Returns the armature object.
    """
    rig = json.load(open(path, encoding="utf-8"))
    name = rig.get("name", "Rig")
    col = collection(name)
    parts = {p["name"]: p for p in rig["parts"]}
    by_child = {j["part1"]: j for j in rig["joints"]}
    root = rig.get("root") or rig["parts"][0]["name"]
    lowest = min(p["cf"][1] - p["size"][1] / 2 for p in rig["parts"])
    up = Matrix.Translation((0, 0, -lowest if lift else 0))

    arm_data = bpy.data.armatures.new(name + "Armature")
    arm = bpy.data.objects.new(name + "Armature", arm_data)
    col.objects.link(arm)
    arm["rbx_rig"] = json.dumps({"root": root, "tree": {j["part1"]: j["part0"] for j in rig["joints"]}})
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    made = {}

    def joint_world(part):
        j = by_child.get(part)
        if not j:
            return _cf_matrix(parts[part]["cf"])
        return _cf_matrix(parts[j["part0"]]["cf"]) @ _cf_matrix(j["c0"])

    def make(part):
        if part in made:
            return made[part]
        j = by_child.get(part)
        eb = arm_data.edit_bones.new(part)
        eb.head = (0, 0, 0)
        eb.tail = (0, 0.4, 0)
        eb.matrix = up @ _R2B @ joint_world(part)
        eb.length = 0.4
        if j and j["part0"] in parts:
            eb.parent = make(j["part0"])
        made[part] = eb
        return eb

    make(root)
    for p in parts:
        make(p)
    bpy.ops.object.mode_set(mode="OBJECT")

    for pname, p in parts.items():
        bpy.ops.mesh.primitive_cube_add(size=1)
        box = bpy.context.active_object
        box.name = pname + "_mesh"
        add(box, col)
        sx, sy, sz = p["size"]
        world = up @ _R2B @ _cf_matrix(p["cf"]) @ Matrix.Diagonal((sx, sy, sz, 1))
        if pname == root:
            box.display_type = "WIRE"
            box.hide_render = True
        box.parent = arm
        box.parent_type = "BONE"
        box.parent_bone = pname
        bpy.context.view_layer.update()
        box.matrix_world = world
    bpy.context.scene.render.fps = 24
    return arm


def pose(arm, part, rotation=None, location=None, frame=None):
    """Set a bone's pose and key it. rotation = (x, y, z) degrees about the
    joint's own Roblox axes (X right, Y up, Z back); location in studs."""
    arm = bpy.data.objects[arm] if isinstance(arm, str) else arm
    pb = arm.pose.bones[part]
    pb.rotation_mode = "XYZ"
    at = frame if frame is not None else bpy.context.scene.frame_current
    if rotation is not None:
        pb.rotation_euler = [math.radians(a) for a in rotation]
        pb.keyframe_insert("rotation_euler", frame=at)
    if location is not None:
        pb.location = location
        pb.keyframe_insert("location", frame=at)


def bake_animation(arm, name, loop=False, priority="Action", step=1, out_dir=None):
    """Sample the armature from scene.frame_start to frame_end into an
    animation JSON for `rbx.py anim` (which writes the .rbxmx to upload)."""
    arm = bpy.data.objects[arm] if isinstance(arm, str) else arm
    info = json.loads(arm["rbx_rig"])
    scene = bpy.context.scene
    fps = scene.render.fps / scene.render.fps_base
    tree = dict(info["tree"])
    tree[info["root"]] = None
    frames = []
    f = scene.frame_start
    last = scene.frame_end
    while True:
        scene.frame_set(f)
        poses = {}
        for pb in arm.pose.bones:
            if pb.name == info["root"]:
                continue
            poses[pb.name] = _matrix_cf(pb.matrix_basis)
        frames.append({"time": round((f - scene.frame_start) / fps, 5), "poses": poses})
        if f >= last:
            break
        f = min(f + step, last)
    out_dir = out_dir or os.path.join(WORK, "animations")
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, name + ".json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump({"name": name, "loop": loop, "priority": priority, "tree": tree, "frames": frames}, fh)
    return {"json": path, "frames": len(frames), "length": frames[-1]["time"]}


from rbxvfx import vfx_surface, vfx_arc, vfx_ring, vfx_cone, vfx_swirl, vfx_shell, flipbook  # noqa: E402,F401
