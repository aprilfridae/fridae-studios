"""Render a promo recipe into a finished folder, and write the card the app's chat shows.

  render("promo.json")  -> {"folder", "video", "cover", "caption", "sheet", "duration", "check"}

The folder (WORK/promos/<project-or-chat>/<YYYYMMDD-HHMMSS>-<slug>/) holds promo.json (a copy with every
path made absolute and the auto-filled focus/words/voice in it), promo.mp4, cover.jpg, caption.txt,
sheet.jpg, layout.json and, when the recipe has voice text, voice.wav. The card
WORK/promo-cards/<id>.json is written last, after the self-check ran.
"""
import copy, datetime, json, ntpath, os, re, shutil, subprocess, tempfile

import rbxpaths
from rbxpromo import engine
from rbxpromo.check import check
from rbxpromo.recipe import RecipeError, load_recipe, recipe_duration, scene_duration, validate, _resolve

TRACK_STEP = 0.25        # seconds between motion samples
MAX_PAN = 0.3            # widest allowed pan, as a fraction of the clip width per second
MIN_MOTION = 0.002       # mean frame difference below this counts as "nothing moved"
# progress budget: prepare 0-0.1, Remotion 0.1-0.9, cover/sheet/check 0.9-1.0
_PREP, _RENDER = 0.1, 0.8


def _slug(text, fallback="promo"):
    s = re.sub(r"[^a-z0-9]+", "-", str(text).lower()).strip("-")[:40].strip("-")
    return s or fallback


def promo_folder(project_or_chat, title):
    """A fresh (not yet created) folder path: WORK/promos/<project-or-chat>/<YYYYMMDD-HHMMSS>-<slug>."""
    who = re.sub(r'[\\/:*?"<>|\x00-\x1f]+', "_", str(project_or_chat)).strip(" ._") or "general"
    base = os.path.join(rbxpaths.WORK, "promos", who,
                        datetime.datetime.now().strftime("%Y%m%d-%H%M%S") + "-" + _slug(title))
    path, n = base, 2
    while os.path.exists(path):
        path, n = "%s-%d" % (base, n), n + 1
    return path


# ---------------------------------------------------------------- auto-track

def _motion_centres(path, start, end, step=TRACK_STEP):
    """[(seconds into the window, x centre 0..1 or None)] of where the picture moves, every `step`
    seconds: the centroid of the difference between two frames a couple of frames apart."""
    import cv2
    import numpy as np
    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise ValueError("could not open video for tracking: " + path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30
    gap = 2
    out = []
    try:
        t = 0.0
        while start + t < end:
            cap.set(cv2.CAP_PROP_POS_FRAMES, int(round((start + t) * fps)))
            ok1, a = cap.read()
            ok2, b = cap.read() if ok1 else (False, None)
            for _ in range(gap - 1):
                ok2, b = cap.read() if ok2 else (False, None)
            if ok1 and ok2:
                a = cv2.cvtColor(cv2.resize(a, (160, 90), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2GRAY)
                b = cv2.cvtColor(cv2.resize(b, (160, 90), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2GRAY)
                d = cv2.absdiff(a, b).astype(np.float32) / 255.0
                d[d < 0.08] = 0          # compression noise
                colsum = d.sum(axis=0)
                x = None
                if d.mean() >= MIN_MOTION and colsum.sum() > 0:
                    x = float((colsum * (np.arange(len(colsum)) + 0.5)).sum() / colsum.sum() / len(colsum))
                out.append((t, x))
            t += step
    finally:
        cap.release()
    return out


def _smooth(points, x0, speed, step=TRACK_STEP):
    """Focus keyframes [{"t","x"}] (t in output seconds): gaps hold the last position, a 3-point moving
    average takes out jitter, and the pan never exceeds MAX_PAN of the width per output second."""
    xs, last = [], x0
    for _, x in points:
        last = last if x is None else x
        xs.append(last)
    avg = [sum(xs[max(0, i - 1):i + 2]) / len(xs[max(0, i - 1):i + 2]) for i in range(len(xs))]
    dt = step / speed
    keys, cur = [], None
    for i, x in enumerate(avg):
        if cur is None:
            cur = x
        else:
            lim = MAX_PAN * dt
            cur += max(-lim, min(lim, x - cur))
        keys.append({"t": round(points[i][0] / speed, 3), "x": round(min(1.0, max(0.0, cur)), 4)})
    return keys


def auto_track(scene):
    """Focus keyframes following the motion in a clip scene's `in`..`out` window (output timeline)."""
    speed = float(scene.get("speed", 1))
    pts = _motion_centres(scene["clip"], float(scene["in"]), float(scene["out"]), TRACK_STEP * speed)
    if not pts:
        return []
    x0 = float((scene.get("crop") or {}).get("x", 0.5))
    return _smooth(pts, x0, speed, TRACK_STEP * speed)


# ---------------------------------------------------------------- preparing the recipe

def _absolute(recipe, base_dir):
    """Copy of the recipe with every file path absolute (the copy lives in another folder)."""
    r = copy.deepcopy(recipe)
    for s in r["scenes"]:
        if "clip" in s:
            s["clip"] = _resolve(s["clip"], base_dir)
    audio = r.get("audio") or {}
    for k in ("music", "voice"):
        if audio.get(k):
            audio[k] = _resolve(audio[k], base_dir)
    caps = r.get("captions")
    if caps and isinstance(caps.get("words"), str):
        caps["words"] = _resolve(caps["words"], base_dir)
    return r


def _shift(words, offset):
    """Song words on the video's timeline: the song starts `offset` seconds into itself."""
    return [{"text": w["text"], "start": round(w["start"] - offset, 3), "end": round(w["end"] - offset, 3)}
            for w in words if w["end"] - offset > 0]


def _caption_problems(recipe):
    caps, audio = recipe.get("captions") or {}, recipe.get("audio") or {}
    src = caps.get("source")
    if caps.get("words") or src in (None, "none", "words"):
        return []
    if src == "voice" and not (audio.get("voice") or audio.get("voice_text")):
        return ["Captions come from the voice, but the recipe has no 'voice' file or 'voice_text'."]
    if src == "music" and not audio.get("music"):
        return ["Captions come from the music (lyrics), but the recipe has no 'music' file."]
    return []


def _prepare(recipe, folder, report):
    """Fill what the recipe leaves empty: the voice (from voice_text), caption words, auto-track focus."""
    from rbxpromo import audio as audio_mod
    if recipe.get("audio") is None:
        recipe["audio"] = {}
    audio, caps = recipe["audio"], recipe.get("captions")
    words = None
    text = audio.get("voice_text")
    if text and not audio.get("voice"):
        report(0.02)
        wav = os.path.join(folder, "voice.wav")
        v = audio_mod.voice(text, wav)
        audio["voice"], words = wav, v["words"]
        checked = copy.deepcopy(recipe)
        checked["audio"].pop("voice_text")      # now spoken: only the length is left to check
        problems = validate(checked, folder, durations={wav: v["duration"]})
        if problems:
            raise RecipeError(problems)
    if caps and caps.get("source") != "none" and not caps.get("words"):
        if caps.get("source") == "voice":
            if words is None:
                words = audio_mod.transcribe(audio["voice"])["words"]
                if text:
                    words = audio_mod.align_to_script(text, words)
            caps["words"] = words
        elif caps.get("source") == "music":
            caps["words"] = _shift(audio_mod.transcribe(audio["music"])["words"], float(audio.get("music_start", 0)))
    report(0.05)
    tracked = [s for s in recipe["scenes"] if "clip" in s and (s.get("crop") or {}).get("track") == "auto" and not s.get("focus")]
    for i, s in enumerate(tracked):
        s["focus"] = auto_track(s)
        report(0.05 + 0.05 * (i + 1) / len(tracked))


# ---------------------------------------------------------------- renderer

def _cover_time(recipe, duration):
    t = 0.5
    at = 0.0
    for s in recipe["scenes"]:
        if "title" in s:
            t = at + min(0.5, scene_duration(s) / 2)
            break
        at += scene_duration(s)
    return round(max(0.0, min(t, duration - 0.1)), 3)


def _run_renderer(recipe_path, video, layout, cover, cover_t, report):
    script = os.path.join(engine.engine_dir(), "renderer", "scripts", "render.mjs")
    cmd = engine.node_cmd(script, recipe_path, video, *(["--layout", layout] if layout else []),
                          "--still", str(cover_t), cover)
    with tempfile.TemporaryFile() as err:
        with subprocess.Popen(cmd, env=engine.node_env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=err,
                              text=True, encoding="utf-8", errors="replace") as proc:
            final = None
            for line in proc.stdout:    # no overall timeout: a long render reports progress until it ends
                if not line.startswith("{"):
                    continue
                try:
                    msg = json.loads(line)
                except ValueError:
                    continue
                if "progress" in msg:
                    report(_PREP + _RENDER * min(1.0, max(0.0, float(msg["progress"]))))
                elif "ok" in msg:
                    final = msg
            code = proc.wait()
        err.seek(0)
        tail = err.read().decode("utf-8", "replace")[-800:].strip()
    if not final or not final.get("ok"):
        why = (final or {}).get("error") or tail or "no result from the renderer (exit code %s)" % code
        raise engine.EngineError("The render failed: " + why)
    return final


def render_still(recipe_path, out_jpg, seconds):
    """Render only one frame of a recipe (at `seconds`) to a JPEG, without making a video."""
    engine.require_ready()
    _run_renderer(recipe_path, "-", None, out_jpg, seconds, lambda _x: None)
    return out_jpg


def _sheet(video, out_jpg):
    import cv2
    import rbxvideo
    with tempfile.TemporaryDirectory() as tmp:
        try:
            res = rbxvideo.frames(video, count=12, out_dir=tmp)
        except SystemExit as e:
            raise RuntimeError("Could not make the contact sheet: %s" % e)
        img = cv2.imread(res["look_at"][0])
        if img is None or not cv2.imwrite(out_jpg, img, [cv2.IMWRITE_JPEG_QUALITY, 88]):
            raise RuntimeError("Could not write the contact sheet " + out_jpg)


def _run_check(video, layout_path, recipe):
    with open(layout_path, encoding="utf-8") as f:
        layout = json.load(f)
    return check(video, layout, recipe)


def _caption_text(recipe):
    post = recipe.get("post") or {}
    tags = " ".join("#" + str(t).lstrip("#") for t in post.get("hashtags") or [] if str(t).strip("#"))
    return str(post.get("caption", "")) + "\n" + tags


def render(recipe_path, out_dir=None, on_progress=None):
    """Validate, prepare and render a recipe; returns the output paths, duration and self-check result."""
    last = [0.0]

    def report(x):
        x = min(1.0, x)
        if on_progress and x > last[0]:
            last[0] = x
            on_progress(round(x, 4))

    base = os.path.dirname(os.path.abspath(recipe_path))
    recipe = load_recipe(recipe_path)                       # RecipeError before anything is created
    problems = _caption_problems(recipe)
    if problems:
        raise RecipeError(problems)
    engine.require_ready()
    first_title = next((s["title"] for s in recipe["scenes"] if "title" in s), recipe.get("style", "promo"))
    agent = os.environ.get("RBXKIT_AGENT_ID", "")
    folder = os.path.abspath(out_dir) if out_dir else promo_folder(recipe.get("project") or agent or "general", first_title)
    existed = os.path.isdir(folder)
    os.makedirs(folder, exist_ok=True)
    try:
        recipe = _absolute(recipe, base)
        _prepare(recipe, folder, report)
        duration = recipe_duration(recipe)
        p = {n: os.path.join(folder, n) for n in ("promo.json", "promo.mp4", "cover.jpg", "caption.txt", "sheet.jpg", "layout.json")}
        rbxpaths.write_json(p["promo.json"], recipe, indent=2, ensure_ascii=False)
        done = _run_renderer(p["promo.json"], p["promo.mp4"], p["layout.json"], p["cover.jpg"],
                             _cover_time(recipe, duration), report)
        report(0.92)
        _sheet(p["promo.mp4"], p["sheet.jpg"])
        text = _caption_text(recipe)
        with open(p["caption.txt"], "w", encoding="utf-8") as f:
            f.write(text)
        report(0.95)
        result = _run_check(p["promo.mp4"], p["layout.json"], recipe)
    except Exception:
        if not existed:
            shutil.rmtree(folder, ignore_errors=True)
        raise
    duration = float(done.get("duration", duration))
    pid = os.path.basename(folder)
    cards = os.path.join(rbxpaths.WORK, "promo-cards")
    os.makedirs(cards, exist_ok=True)
    card = {"id": pid, "agent": agent, "folder": folder, "video": p["promo.mp4"], "cover": p["cover.jpg"],
            "caption_text": text, "style": recipe["style"], "duration": duration, "check": result,
            "created": datetime.datetime.now().astimezone().isoformat(timespec="seconds")}
    rbxpaths.write_json(os.path.join(cards, pid + ".json"), card, indent=2, ensure_ascii=False)
    report(1.0)
    return {"folder": folder, "video": p["promo.mp4"], "cover": p["cover.jpg"], "caption": p["caption.txt"],
            "sheet": p["sheet.jpg"], "duration": duration, "check": result}
