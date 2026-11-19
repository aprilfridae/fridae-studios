"""VFX mesh shapes and rendered flipbooks for rbxkit (inside Blender).

Every shape is one open sheet around the origin, in studs, facing Roblox's
forward (Blender +Y). UVs: U along the sweep (0 start, 1 end), V across it
(0 inside/bottom, 1 outside/top). Set DoubleSided on the MeshPart in Studio.
"""
import bpy, bmesh, math, os, json
from mathutils import Vector


def _link(obj, coll=None):
    col = bpy.data.collections.get(coll or "VFX")
    if not col:
        col = bpy.data.collections.new(coll or "VFX")
        bpy.context.scene.collection.children.link(col)
    col.objects.link(obj)


def vfx_surface(name, point, columns=32, rows=1, rgba=None, collection=None):
    """Any sheet described as a function: point(u, v) -> position for u, v in
    0..1; that corner's UV is (u, v). Corners that meet are merged, so closed
    loops and poles need no special care. Replaces an object of the same name."""
    old = bpy.data.objects.get(name)
    if old:
        bpy.data.objects.remove(old, do_unlink=True)
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("UVMap")
    grid = [[bm.verts.new(Vector(point(c / columns, r / rows))) for c in range(columns + 1)] for r in range(rows + 1)]
    for r in range(rows):
        for c in range(columns):
            quad = [grid[r][c], grid[r][c + 1], grid[r + 1][c + 1], grid[r + 1][c]]
            if len(set(tuple(round(k, 6) for k in v.co) for v in quad)) < 3:
                continue
            try:
                face = bm.faces.new(quad)
            except ValueError:
                continue
            coords = [(c / columns, r / rows), ((c + 1) / columns, r / rows),
                      ((c + 1) / columns, (r + 1) / rows), (c / columns, (r + 1) / rows)]
            for loop, co in zip(face.loops, coords):
                loop[uv].uv = co
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=1e-6)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    obj = bpy.data.objects.new(name, me)
    _link(obj, collection)
    if rgba:
        mat = bpy.data.materials.new(name + "Mat")
        mat.use_nodes = True
        mat.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (rgba[0], rgba[1], rgba[2], 1)
        mat.diffuse_color = (rgba[0], rgba[1], rgba[2], 1)
        me.materials.append(mat)
    return obj


def vfx_arc(name, radius, width, sweep=160, segments=32, taper=True, **kw):
    """A flat crescent for a slash: sweeps `sweep` degrees centred on forward,
    from radius - width out to radius. With taper it is widest in the middle
    and pointed at both ends. U runs from the left end to the right end."""
    half = math.radians(sweep) / 2

    def point(u, v):
        ang = -half + u * 2 * half
        w = width * (math.sin(u * math.pi) if taper else 1.0)
        r = radius - w + v * w
        return (math.sin(ang) * r, math.cos(ang) * r, 0.0)
    return vfx_surface(name, point, columns=segments, rows=1, **kw)


def vfx_ring(name, radius, width=0.5, height=0, top_radius=None, segments=48, **kw):
    """height 0: a flat band on the ground (a shockwave). With a height: a wall
    from radius at the bottom to top_radius at the top (flared = a blast wave)."""
    top = radius if top_radius is None else top_radius

    def point(u, v):
        ang = u * math.tau
        if height == 0:
            r, z = radius - width + v * width, 0.0
        else:
            r, z = radius + (top - radius) * v, height * v
        return (math.sin(ang) * r, math.cos(ang) * r, z)
    return vfx_surface(name, point, columns=segments, rows=1, **kw)


def vfx_cone(name, radius, height, tip_radius=0, segments=32, **kw):
    """An open cone from its base at the origin up to its tip."""
    def point(u, v):
        ang = u * math.tau
        r = radius + (tip_radius - radius) * v
        return (math.sin(ang) * r, math.cos(ang) * r, height * v)
    return vfx_surface(name, point, columns=segments, rows=1, **kw)


def vfx_swirl(name, radius, height, width, turns=1.5, top_radius=None, segments=96, **kw):
    """A ribbon `width` tall spiralling up `turns` times to `height`."""
    top = radius if top_radius is None else top_radius

    def point(u, v):
        ang = u * math.tau * turns
        r = radius + (top - radius) * u
        return (math.sin(ang) * r, math.cos(ang) * r, (height - width) * u + width * v)
    return vfx_surface(name, point, columns=segments, rows=1, **kw)


def vfx_shell(name, radius, segments=32, rings=16, dome=False, **kw):
    """A sphere, or a dome standing on the ground."""
    def point(u, v):
        ang = u * math.tau
        lat = (v * (math.pi / 2)) if dome else (v * math.pi - math.pi / 2)
        return (math.sin(ang) * math.cos(lat) * radius, math.cos(ang) * math.cos(lat) * radius, math.sin(lat) * radius)
    return vfx_surface(name, point, columns=segments, rows=rings, **kw)


def flipbook(name, grid=4, mode="alpha", start=None, end=None, loop=False, padding=4, folder=None, work=None):
    """Render the scene's animation through scene.camera into a 1024 sheet,
    <name>.flipbook.png (+ .flipbook.json). "alpha" renders on a transparent
    film, "additive" on black. Puts back the render settings it changes."""
    import numpy as np
    scene = bpy.context.scene
    if scene.camera is None:
        raise ValueError("flipbook needs scene.camera")
    cells = grid * grid
    cell = 1024 // grid
    inner = cell - padding * 2
    start = scene.frame_start if start is None else start
    end = scene.frame_end if end is None else end
    r = scene.render
    saved = (r.resolution_x, r.resolution_y, r.resolution_percentage, r.film_transparent, r.filepath,
             r.image_settings.file_format, r.image_settings.color_mode, scene.frame_current, scene.view_settings.view_transform)
    base = folder or "vfx"
    out_dir = base if os.path.isabs(base) else os.path.join(work or os.path.dirname(bpy.data.filepath) or os.getcwd(), base)
    os.makedirs(out_dir, exist_ok=True)
    tmp = os.path.join(out_dir, "_frame.png")
    sheet = np.zeros((1024, 1024, 4), dtype=np.float32)
    if mode == "additive":
        sheet[..., 3] = 1.0  # opaque black between the frames too
    used, repeats = [], 0
    try:
        r.resolution_x = r.resolution_y = inner
        r.resolution_percentage = 100
        r.film_transparent = True  # additive is composited onto black below
        r.image_settings.file_format = "PNG"
        r.image_settings.color_mode = "RGBA"
        scene.view_settings.view_transform = "Standard"
        r.filepath = tmp
        for i in range(cells):
            t = i / cells if loop else (i / (cells - 1) if cells > 1 else 0)
            frame = int(round(start + (end - start) * t))
            if used and used[-1] == frame:
                repeats += 1
            used.append(frame)
            scene.frame_set(frame)
            bpy.ops.render.render(write_still=True)
            img = bpy.data.images.load(tmp, check_existing=False)
            px = np.array(img.pixels[:], dtype=np.float32).reshape(inner, inner, 4)[::-1]
            bpy.data.images.remove(img)
            if mode == "additive":
                px[..., :3] *= px[..., 3:4]
                px[..., 3] = 1.0
            row, col = divmod(i, grid)
            sheet[row * cell + padding:row * cell + padding + inner, col * cell + padding:col * cell + padding + inner] = px
    finally:
        (r.resolution_x, r.resolution_y, r.resolution_percentage, r.film_transparent, r.filepath,
         r.image_settings.file_format, r.image_settings.color_mode) = saved[:7]
        scene.frame_set(saved[7])
        scene.view_settings.view_transform = saved[8]
        if os.path.exists(tmp):
            os.remove(tmp)
    out = bpy.data.images.new(name + "_sheet", 1024, 1024, alpha=True)
    out.pixels = sheet[::-1].ravel()
    path = os.path.join(out_dir, name + ".flipbook.png")
    out.filepath_raw = path
    out.file_format = "PNG"
    out.save()
    bpy.data.images.remove(out)
    with open(os.path.join(out_dir, name + ".flipbook.json"), "w", encoding="utf-8") as f:
        json.dump({"grid": grid, "loop": loop, "fps": None, "padding": padding, "mode": mode}, f)
    return {"sheet": path, "frames": used, "held": repeats}
