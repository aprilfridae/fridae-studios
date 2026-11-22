"""Texture generation with Google's Gemini image models.

Only textures: tileable surface materials, particle sprites and decals.
It is deliberately not for UI, icons, logos or anything with lettering.

  rbx.py gen "weathered oak planks" --kind surface --name OakPlanks
  rbx.py gen "jagged impact star" --kind particle --name ImpactStar --folder Slam

Where the image comes from, in order:
1. Google's Antigravity command-line tool (agy), when it is installed and signed
   in. It makes the image on the person's own Google account, with no key.
2. The Gemini API, only when a key is saved AND the person switched it on in
   the app's Setup panel. Google charges for each image.
3. The person: the app shows the prompt, they make the image in their own
   Gemini chat, and the app picks it up when they copy or paste it.
A source that fails falls through to the next. The result is processed and
checked the same way whichever one made it.
"""
import base64, io, json, os, re, shutil, subprocess, sys, urllib.error, urllib.request

from rbxpaths import WORK, CONFIG

API = "https://generativelanguage.googleapis.com/v1beta"
DEFAULT_MODELS = ["gemini-nano-banana-2.1", "gemini-3.1-flash-image", "gemini-2.5-flash-image"]
NOT_TEXTURES = re.compile(r"\b(logo|logotype|wordmark|typograph\w*|font|lettering|letters?|text|caption|slogan|title screen|ui|hud|gui|"
                          r"button|menu|icon|badge|banner|poster|thumbnail|layout|mockup|wireframe|interface)\b", re.I)

KINDS = {
    "surface": ("Seamless tileable texture of {p}. Flat and evenly lit, seen straight on with no perspective, no cast shadows, "
                "no vignette, no border, no frame. The material fills the whole image edge to edge. "
                "No text, no letters, no numbers, no logos, no symbols, no objects."),
    "particle": ("{p}. A single white shape on a pure black background, for a game particle texture. Centred, with empty black "
                 "margin all round, greyscale only, no colour, no border, no frame, no text, no letters."),
    "decal": ("{p}. One isolated mark on a pure black background, for a game decal. Centred, with empty black margin all round, "
              "no border, no frame, no text, no letters, no logos."),
}


def _config():
    try:
        return json.load(open(CONFIG, encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def api_key():
    return os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY") or _config().get("gemini_key") or ""


def have_key():
    return bool(api_key())


def api_enabled():
    """The paid API is only used when a key exists AND the user switched it on.
    A saved key alone does nothing: the default is always the free hand-off."""
    if os.environ.get("FRIDAE_SETTINGS_FROM_APP") == "1":
        # run by the app: its own value, never the settings file (an AI's file tool may have changed that)
        return have_key() and os.environ.get("FRIDAE_GEMINI_USE_API") == "1"
    return have_key() and bool(_config().get("gemini_use_api"))


def find_antigravity():
    """Antigravity's command-line tool, or None. Switch this source off with "textures_from_antigravity": false in config.json."""
    if _config().get("textures_from_antigravity") is False:
        return None
    local = os.path.join(os.environ.get("LOCALAPPDATA", ""), "agy", "bin", "agy.exe")
    return os.environ.get("FRIDAE_AGY") or shutil.which("agy") or (local if os.path.exists(local) else None)


IMAGE_PATH = re.compile(r"[A-Za-z]:[\\/][^\r\n\"<>|?*]+?\.(?:png|jpe?g|webp)|/[^\r\n\"<>|?*]+?\.(?:png|jpe?g|webp)", re.I)


def antigravity_image(exe, prompt, timeout=420):
    """Have Antigravity generate one image and return its bytes.

    It is run without its own interface and given no permissions beyond its defaults, so it can make the
    image and nothing else. It answers with the path of the file it wrote.
    """
    from rbxpaths import no_window
    ask = ("Use your generate_image tool exactly once to make this image, then reply with only the full path of the image file "
           "it produced. Do nothing else and do not edit the image. Image: square (1:1). " + prompt)
    line = json.dumps({"event": "user", "message": {"content": ask}}) + "\n"
    folder = os.path.join(WORK, "antigravity", "textures")
    os.makedirs(folder, exist_ok=True)
    try:
        p = subprocess.run([exe, "--input-format", "stream-json", "--output-format", "stream-json"], input=line.encode("utf-8"),
                           capture_output=True, timeout=timeout, cwd=folder, creationflags=no_window())
    except subprocess.TimeoutExpired:
        raise RuntimeError("Antigravity took longer than %d minutes to make the image." % (timeout // 60))
    except OSError as e:
        raise RuntimeError("Antigravity could not be started (%s)." % e)
    reply, conversation = "", ""
    for raw in p.stdout.decode("utf-8", "replace").splitlines():
        try:
            e = json.loads(raw)
        except ValueError:
            continue
        conversation = e.get("conversation_id") or conversation
        if e.get("event") == "result":
            reply = (e.get("result") or {}).get("response") or ""
    paths = [m.group(0).strip() for m in IMAGE_PATH.finditer(reply)]
    # if the answer did not name the file, it is the newest image in this conversation's own folder
    brain = os.path.join(os.path.expanduser("~"), ".gemini", "antigravity-cli", "brain", conversation)
    if conversation and os.path.isdir(brain):
        made = [os.path.join(brain, f) for f in os.listdir(brain) if f.lower().endswith((".png", ".jpg", ".jpeg", ".webp"))]
        paths += sorted(made, key=os.path.getmtime, reverse=True)
    for path in paths:
        if os.path.isfile(path):
            with open(path, "rb") as f:
                return f.read()
    err = p.stderr.decode("utf-8", "replace").strip()
    if "authentication" in err.lower():
        raise RuntimeError("Antigravity is not signed in (run agy once in a terminal and sign in).")
    raise RuntimeError("Antigravity did not produce an image%s" % ((": " + (err or reply)[:200]) if (err or reply) else "."))


def _post(path, body, key, timeout=180):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode("utf-8"), method="POST",
                                 headers={"x-goog-api-key": key, "Content-Type": "application/json", "Api-Revision": "2026-05-20"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(raw)
        except ValueError:
            return e.code, {"error": {"message": raw[:400]}}


def _images_in(obj, out=None):
    """Find base64 image payloads anywhere in a response, whatever its exact shape."""
    out = [] if out is None else out
    if isinstance(obj, dict):
        data = obj.get("data")
        if isinstance(data, str) and len(data) > 200 and (obj.get("type") == "image" or "mime" in "".join(obj.keys()).lower()):
            out.append(data)
        for v in obj.values():
            _images_in(v, out)
    elif isinstance(obj, list):
        for v in obj:
            _images_in(v, out)
    return out


def _message(resp):
    err = resp.get("error") if isinstance(resp, dict) else None
    if isinstance(err, dict):
        return str(err.get("message") or err)
    return json.dumps(resp)[:300]


def request_image(prompt, key, model=None, reference=None):
    """Ask Gemini for one image. Returns (png_or_jpeg_bytes, model_used)."""
    models = [model] if model else ([_config().get("gemini_model")] if _config().get("gemini_model") else []) + DEFAULT_MODELS
    parts = [{"type": "text", "text": prompt}]
    if reference:
        with open(reference, "rb") as f:
            mime = "image/png" if reference.lower().endswith(".png") else "image/jpeg"
            parts.append({"type": "image", "mime_type": mime, "data": base64.b64encode(f.read()).decode("ascii")})
    last = "no model tried"
    for m in dict.fromkeys(x for x in models if x):
        # current API first
        code, resp = _post("/interactions", {"model": m, "input": parts}, key)
        imgs = _images_in(resp) if code == 200 else []
        if not imgs and code in (400, 404):
            # older API shape, for models that still use it
            old = [{"text": prompt}]
            if reference:
                old.append({"inlineData": {"mimeType": parts[1]["mime_type"], "data": parts[1]["data"]}})
            code2, resp2 = _post("/models/%s:generateContent" % m, {"contents": [{"parts": old}],
                                                                   "generationConfig": {"responseModalities": ["IMAGE"]}}, key)
            if code2 == 200:
                code, resp = code2, resp2
                imgs = _images_in(resp)
        if imgs:
            return base64.b64decode(imgs[-1]), m
        last = "%s: HTTP %s: %s" % (m, code, _message(resp))
        if code in (401, 403):
            break  # a key problem will not be fixed by another model
    raise RuntimeError("Gemini did not return an image (%s)" % last)


REQUESTS = os.path.join(WORK, "texture-requests")


def ask_user(prompt, kind, name, wait=600, agent=None):
    """No API key: hand the prompt to the person, who makes the image in their own Gemini chat.

    Writes a request the app shows as a card, then waits for the image to be
    pasted or copied. Returns the image bytes, or raises with what to do next.
    """
    import time
    os.makedirs(REQUESTS, exist_ok=True)
    rid = "t%d" % int(time.time() * 1000)
    base = os.path.join(REQUESTS, rid)
    chat_prompt = "Generate an image, square (1:1). " + prompt
    with open(base + ".json", "w", encoding="utf-8") as f:
        json.dump({"id": rid, "prompt": chat_prompt, "kind": kind, "name": name or "Texture",
                   "agent": agent or os.environ.get("RBXKIT_AGENT_ID") or "", "created": time.time()}, f)
    deadline = time.time() + wait
    try:
        while time.time() < deadline:
            if os.path.exists(base + ".png"):
                time.sleep(0.3)  # let the writer finish
                with open(base + ".png", "rb") as f:
                    return f.read()
            if os.path.exists(base + ".skip"):
                raise RuntimeError("The user skipped this texture. Carry on without it: use a built-in texture or draw one in "
                                   "code, and say which texture is missing.")
            time.sleep(0.5)
    finally:
        for ext in (".json", ".png", ".skip"):
            try:
                os.remove(base + ext)
            except OSError:
                pass
    raise RuntimeError("No image arrived within %d minutes. Tell the user the prompt below, ask them to make it in their Gemini "
                       "chat and save or paste the image, then call this again with from_image set to the file.\n\nPROMPT: %s"
                       % (wait // 60, chat_prompt))


def make_seamless(img):
    """Blend an image with its half-offset copy so opposite edges match."""
    import numpy as np
    from PIL import Image
    a = np.asarray(img.convert("RGB")).astype(np.float32)
    h, w = a.shape[:2]
    b = np.roll(np.roll(a, h // 2, axis=0), w // 2, axis=1)
    y = np.abs(np.linspace(-1, 1, h))[:, None]
    x = np.abs(np.linspace(-1, 1, w))[None, :]
    edge = np.clip((np.maximum(x, y) - 0.6) / 0.4, 0, 1) ** 1.5  # 0 in the middle, 1 at the borders
    out = a * (1 - edge[..., None]) + b * edge[..., None]
    return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "RGB")


def to_alpha(img, keep_colour):
    """Turn a shape drawn on black into a transparent PNG."""
    import numpy as np
    from PIL import Image
    a = np.asarray(img.convert("RGB")).astype(np.float32) / 255
    lum = a.max(axis=-1)
    alpha = np.clip((lum - 0.03) / 0.97, 0, 1)
    if keep_colour:
        rgb = np.where(alpha[..., None] > 0.02, a / np.maximum(alpha[..., None], 0.02), 0)
    else:
        rgb = np.ones_like(a)
    rgba = np.dstack([np.clip(rgb, 0, 1), alpha])
    return Image.fromarray((rgba * 255 + 0.5).astype(np.uint8), "RGBA")


def generate(prompt, kind="surface", name=None, folder=None, size=None, seamless=None, reference=None, model=None,
             from_image=None, wait=600, agent=None, use_api=None):
    from PIL import Image
    if kind not in KINDS:
        raise ValueError("kind must be one of: " + ", ".join(KINDS))
    hit = NOT_TEXTURES.search(prompt)
    if hit:
        raise ValueError("This tool only makes textures (surface materials, particle sprites, decals). \"%s\" reads as UI, "
                         "lettering or graphic design, which it is not for: build those with native UI and real fonts." % hit.group(0))
    key = api_key()
    full = KINDS[kind].format(p=prompt.strip().rstrip("."))
    if from_image:
        with open(from_image, "rb") as f:
            raw, used = f.read(), "supplied image"
    else:
        # each source that fails says why and hands over to the next; the person is always the last resort
        sources, agy = [], find_antigravity()
        if agy and not reference:  # it cannot be shown a reference image this way
            sources.append(("Antigravity", lambda: antigravity_image(agy, full)))
        if key and (api_enabled() if use_api is None else use_api):
            sources.append(("the Gemini API", lambda: request_image(full, key, model=model, reference=reference)[0]))
        raw, used, notes = None, "", []
        for label, make in sources:
            try:
                raw, used = make(), "generated by " + label
                break
            except Exception as e:
                notes.append("%s did not work (%s)" % (label, str(e)[:160]))
        if raw is None:
            raw, used = ask_user(full, kind, name, wait, agent), "made by the user in Gemini chat"
        if notes:
            used += "; " + "; ".join(notes)
    img = Image.open(io.BytesIO(raw))
    img.load()
    size = int(size or (1024 if kind == "surface" else 512))
    side = min(img.size)
    left, top = (img.size[0] - side) // 2, (img.size[1] - side) // 2
    img = img.crop((left, top, left + side, top + side)).resize((size, size), Image.LANCZOS)
    if kind == "surface":
        if seamless is None or seamless:
            img = make_seamless(img)
        else:
            img = img.convert("RGB")
    else:
        img = to_alpha(img, keep_colour=(kind == "decal"))
    safe = re.sub(r"[^A-Za-z0-9_-]+", "", name or "") or "Texture"
    out_dir = os.path.join(WORK, "textures", folder) if folder else os.path.join(WORK, "textures")
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, safe + ".png")
    img.save(path)
    result = {"file": path, "kind": kind, "size": "%dx%d" % img.size, "model": used}
    try:
        import rbxtex
        if kind == "surface":
            # show it tiled 2x2 so seams can be judged
            tiled = Image.new("RGB", (size * 2, size * 2))
            for dx in (0, size):
                for dy in (0, size):
                    tiled.paste(img, (dx, dy))
            look = os.path.join(out_dir, safe + "_tiled_check.jpg")
            tiled.resize((768, 768), Image.LANCZOS).save(look, quality=88)
            result["look_at"] = look
            result["note"] = "look_at shows the texture tiled 2x2: check for visible seams and repeats before using it."
        else:
            rep = rbxtex.review([path], preview=False)
            result["check"] = {k: v for k, v in rep["textures"][0].items() if k in ("coverage", "problems", "notes", "mode")}
            result["look_at"] = rep["look_at"]
        result["preview_id"] = rbxtex.stage_preview(path)
    except Exception as e:  # the texture is still usable without the review
        result["review_error"] = str(e)
    result["upload"] = "Not uploaded. Look at it first; upload with open_cloud_invoke (assetType Image) once it is right."
    return result


def _cmd(a):
    try:
        print(json.dumps(generate(a.prompt, a.kind, a.name, a.folder, a.size, None if a.seamless is None else a.seamless == "yes",
                                  a.reference, a.model, a.from_image, a.wait, agent=a.agent,
                                  use_api=False if a.no_api else None), indent=1))
    except (ValueError, RuntimeError) as e:
        sys.exit(str(e))


def register(sub):
    g = sub.add_parser("gen", help="generate a texture with Gemini (surface, particle or decal; not UI or lettering)")
    g.add_argument("prompt")
    g.add_argument("--kind", choices=sorted(KINDS), default="surface")
    g.add_argument("--name")
    g.add_argument("--folder")
    g.add_argument("--size", type=int)
    g.add_argument("--seamless", choices=["yes", "no"])
    g.add_argument("--reference", help="an image to match in style")
    g.add_argument("--model")
    g.add_argument("--from-image", dest="from_image", help="process an image you already have instead of generating one")
    g.add_argument("--wait", type=int, default=600, help="seconds to wait for the user's image when there is no API key")
    g.add_argument("--agent", help="which chat in the app the request belongs to")
    g.add_argument("--no-api", dest="no_api", action="store_true", help="always ask the user; never use the paid API")
    g.set_defaults(fn=_cmd)
