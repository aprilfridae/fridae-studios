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


def antigravity_exe():
    """Antigravity's command-line tool (agy), or None when it is not installed."""
    local = os.path.join(os.environ.get("LOCALAPPDATA", ""), "agy", "bin", "agy.exe")
    return os.environ.get("FRIDAE_AGY") or shutil.which("agy") or (local if os.path.exists(local) else None)


def find_antigravity():
    """Antigravity's command-line tool as a source of textures, or None. Switch this source off with
    "textures_from_antigravity": false in config.json."""
    if _config().get("textures_from_antigravity") is False:
        return None
    return antigravity_exe()


IMAGE_PATH = re.compile(r"[A-Za-z]:[\\/][^\r\n\"<>|?*]+?\.(?:png|jpe?g|webp)|/[^\r\n\"<>|?*]+?\.(?:png|jpe?g|webp)", re.I)
IMAGE_ENDINGS = (".png", ".jpg", ".jpeg", ".webp")
MODELS_KEPT = 24 * 3600     # seconds the cheapest model's name is remembered before agy is asked again
LEVELS = ("-minimal", "-low", "", "-medium", "-high", "-xhigh", "-max")
MODEL_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")      # never anything agy could read as a flag
CONVERSATION_ID = re.compile(r"^[A-Za-z0-9-]{1,80}$")
MAX_MADE = 40 * 1024 * 1024  # bytes: the largest file taken as a generated picture (agy's are under a megabyte)
# What the wrapper turn may not do, whatever the prompt says: run a command, or write a file with its file tools.
# Its image tool needs neither (measured, agy 1.5.15: a picture was made under both rules).
WRAPPER_DENY = ["command(*)", "write_file(*)"]
# How many pictures agents may start on the person's Gemini plan in an hour: one chat, and one game's whole team (its
# Lead and every expert it started work in the same project folder). Textures and images count alike.
PER_HOUR = 12
PER_HOUR_PROJECT = 30
STALE = 30 * 60             # seconds after which what a call left behind (it was ended mid-way) is cleared away


class SpendLimit(RuntimeError):
    """No more pictures may be started for now: the message says who has started how many, and to ask the user."""


def brain_dir():
    """Where Antigravity keeps a folder per conversation; its generate_image tool leaves each picture there
    (measured, agy 1.5.15: <home>/.gemini/antigravity-cli/brain/<conversation>/<name>_<time>.jpg)."""
    return os.path.join(os.path.expanduser("~"), ".gemini", "antigravity-cli", "brain")


def turns_dir():
    """Where the wrapper turns run: a folder of the app's own that no agent's file tools may write (the app's deny
    rules name work/antigravity, `paths::app_protected`), since agy starts the tool servers a folder's
    .agents/mcp_config.json names."""
    return os.path.join(WORK, "antigravity")


# ------------------------------------------------- ending cleanly, and sweeping
class cleanly:
    """While this is open, being told to end (SIGTERM: a cancelled job, a turn that is over) unwinds the call like an
    error would, so what it was holding is given back (`finally`): the wrapper's folder, a file name taken for the
    picture. Only on the main thread, and only where nothing else handles the signal already."""

    def __enter__(self):
        import signal, threading
        self.was = None
        try:
            if threading.current_thread() is threading.main_thread() and signal.getsignal(signal.SIGTERM) == signal.SIG_DFL:
                def end(signum, frame):
                    raise SystemExit(128 + signum)
                self.was = signal.signal(signal.SIGTERM, end)
        except (ValueError, OSError, AttributeError):
            self.was = None
        return self

    def __exit__(self, *exc):
        import signal
        if self.was is not None:
            try:
                signal.signal(signal.SIGTERM, self.was)
            except (ValueError, OSError):
                pass
        return False


def reserved_dir():
    return os.path.join(turns_dir(), "reserved")


def sweep(now=None):
    """Clear away what an earlier call left when it was ended outright: wrapper folders, and file names taken for a
    picture that never came (still empty files), once they are older than any call can be."""
    import time
    now = now or time.time()
    turns = os.path.join(turns_dir(), "turns")
    try:
        for name in os.listdir(turns):
            path = os.path.join(turns, name)
            if now - os.lstat(path).st_mtime > STALE and not os.path.islink(path):
                shutil.rmtree(path, ignore_errors=True)
    except OSError:
        pass
    try:
        notes = os.listdir(reserved_dir())
    except OSError:
        notes = []
    from rbxpaths import read_json
    for name in notes:
        note = os.path.join(reserved_dir(), name)
        try:
            if now - os.lstat(note).st_mtime <= STALE:
                continue
            had = read_json(note)
            taken = had.get("path") if isinstance(had, dict) else None
            if isinstance(taken, str) and os.path.isabs(taken):
                import stat
                st = os.lstat(taken)
                if stat.S_ISREG(st.st_mode) and st.st_size == 0:
                    os.remove(taken)
        except OSError:
            pass
        try:
            os.remove(note)
        except OSError:
            pass


# ---------------------------------------------------------- the hourly count
def chat_id():
    return re.sub(r"[^A-Za-z0-9_-]", "", os.environ.get("RBXKIT_AGENT_ID") or "")[:80]


def _counts():
    """The counts a picture started now belongs to: (who, the file its start times are kept in, how many an hour)."""
    import hashlib
    spent = os.path.join(turns_dir(), "spent")
    out = [("This chat", os.path.join(spent, (chat_id() or "command-line") + ".json"), PER_HOUR)]
    project = os.environ.get("RBXKIT_PROJECT") or ""
    if project and os.path.isdir(project):
        key = hashlib.sha256(os.path.normcase(os.path.realpath(project)).encode("utf-8")).hexdigest()[:20]
        out.append(("This game's team (its Lead and experts together)", os.path.join(spent, "project-" + key + ".json"), PER_HOUR_PROJECT))
    return out


def _starts(path, now):
    """The start times of the last hour kept in a file (one that does not read as that counts as none)."""
    from rbxpaths import read_json
    had = read_json(path)
    return [t for t in had if isinstance(t, (int, float)) and not isinstance(t, bool) and 0 <= now - t < 3600] if isinstance(had, list) else []


def started_this_hour(now=None):
    """How many pictures this chat has started in the last hour (made or not: a try that failed may have cost the plan too)."""
    import time
    return len(_starts(_counts()[0][1], now or time.time()))


def spend_refusal(now=None):
    """Why no picture may be started now, in words for the agent, or None when one may."""
    import time
    now = now or time.time()
    for who, path, most in _counts():
        if len(_starts(path, now)) >= most:
            return ("%s has started %d image generations in the last hour, the most allowed without the user's say (each one "
                    "uses their Gemini plan, whether or not a picture came of it). No image was made. Stop generating: tell "
                    "the user what you still want to make and why, and ask them." % (who, most))
    return None


def _note_start(now):
    from rbxpaths import write_json
    for _, path, _ in _counts():
        try:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            write_json(path, _starts(path, now) + [now])
        except OSError:
            pass


# ------------------------------------------------------------------ the model
def flash_model(model):
    """Whether this is a model id the wrapper turn may be given: a plain id, and a Flash one."""
    return isinstance(model, str) and len(model) <= 80 and bool(MODEL_ID.match(model)) and "flash" in model.lower()


def cheapest_flash(ids):
    """The cheapest Flash model among agy's model ids: a "lite" one first, then the lowest effort level. None when
    there is no Flash model."""
    def rank(mid):
        low = mid.lower()
        level = next((i for i, end in enumerate(LEVELS) if end and low.endswith(end)), LEVELS.index(""))
        return ("lite" not in low, level, low)
    flash = [m for m in ids if flash_model(m)]
    return min(flash, key=rank) if flash else None


def _models_file():
    return os.path.join(turns_dir(), "models.json")


def _remember_model(exe, model):
    """Keep the model's name for a day ("" for: name none, agy's own default)."""
    import time
    from rbxpaths import write_json
    try:
        os.makedirs(turns_dir(), exist_ok=True)
        write_json(_models_file(), {"exe": exe, "model": model or "", "at": time.time()})
    except OSError:
        pass


def cheapest_model(exe, timeout=30):
    """The model the wrapper turn runs on, so it costs the user's plan as little as possible: the cheapest Flash
    model `agy models` lists (one "id<tab>name" per line), remembered for a day. None when agy does not say, or
    would not take it earlier today; the turn then runs on agy's own default. (The turn only passes the request on:
    the picture itself is made by agy's image helper, on whatever model agy gives that.) A remembered name is used
    only if it is a plain Flash id; a file that does not read as expected is ignored."""
    import time
    from rbxpaths import no_window, read_json
    had = read_json(_models_file())
    try:
        fresh = isinstance(had, dict) and had.get("exe") == exe and 0 <= time.time() - float(had.get("at")) < MODELS_KEPT
    except (TypeError, ValueError):
        fresh = False
    if fresh and had.get("model") == "":
        return None
    if fresh and flash_model(had.get("model")):
        return had["model"]
    try:
        p = subprocess.run([exe, "models"], capture_output=True, timeout=max(1, timeout), creationflags=no_window())
    except (OSError, subprocess.TimeoutExpired):
        return None
    if p.returncode != 0:
        return None
    ids = [line.split("\t")[0].strip() for line in p.stdout.decode("utf-8", "replace").splitlines() if "\t" in line]
    model = cheapest_flash(ids)
    if model:
        _remember_model(exe, model)
    return model


def _shape_words(aspect):
    """How the image's shape is asked for: the ratio, and a plain word for it."""
    try:
        w, h = (int(x) for x in aspect.split(":"))
    except ValueError:
        w = h = 1
    if w == h:
        return "square (1:1)"
    return "%s (%d:%d aspect ratio)" % ("landscape" if w > h else "portrait", w, h)


def _why_no_image(errors):
    """What went wrong, in plain words, from Antigravity's own error output (never the model's words); None when it
    did not say. Nothing Antigravity wrote is passed on as it is: only how long until the quota resets."""
    if re.search(r"\bquota\b|RESOURCE_EXHAUSTED", errors, re.I):
        resets = re.search(r"Resets in ((?:\d{1,3}[dhms]){1,4})", errors)
        return ("Your Gemini plan's quota is used up for now, so no image was made (Antigravity reported that its quota was "
                "reached%s). Do not try again before it resets: tell the user, and carry on with work that needs no "
                "generated image." % ((", and that it resets in " + resets.group(1)) if resets else ""))
    if re.search(r"authentication required|not authenticated|please sign in|login required|not logged in|unauthenticated", errors, re.I):
        return ("Antigravity is not signed in, so no image was made. The user opens a terminal, runs `agy`, signs in with "
                "their Google account and closes it; then ask again.")
    return None


def _made_in(conversation, named, started):
    """The picture this turn made: an image file inside Antigravity's own folder for the conversation the turn
    opened, written since the turn began, of a size a picture has. A file the answer names counts only if it is one
    of those (the answer is the model's words, and the model read a prompt an agent wrote). Returns its bytes."""
    if not CONVERSATION_ID.match(conversation or ""):
        return None
    home = os.path.realpath(os.path.join(brain_dir(), conversation))
    if not os.path.isdir(home):
        return None

    def ours(path):
        try:
            real = os.path.realpath(path)
            st = os.lstat(real)
        except OSError:
            return None
        import stat
        inside = os.path.dirname(real) == home
        if not (inside and stat.S_ISREG(st.st_mode) and real.lower().endswith(IMAGE_ENDINGS)):
            return None
        return (st.st_mtime, real) if st.st_mtime >= started - 2 and 0 < st.st_size <= MAX_MADE else None

    told = [m for m in (ours(p) for p in named) if m]
    found = sorted((m for m in (ours(os.path.join(home, f)) for f in os.listdir(home)) if m), reverse=True)
    for _, real in told + found:
        try:
            with open(real, "rb") as f:
                return f.read(MAX_MADE + 1)[:MAX_MADE]
        except OSError:
            continue
    return None


def _one_turn(exe, ask, model, timeout):
    """One headless Antigravity turn, in an empty folder made for it and removed afterwards (also when the call is
    told to end mid-way). Returns (image bytes or None, the conversation it opened, Antigravity's own error output)."""
    import secrets, time
    from rbxpaths import no_window
    line = json.dumps({"event": "user", "message": {"content": ask}}) + "\n"
    folder = os.path.join(turns_dir(), "turns", secrets.token_hex(8))
    args = [exe, "--input-format", "stream-json", "--output-format", "stream-json", "--settings", json.dumps({"permissions": {"deny": WRAPPER_DENY}})]
    if flash_model(model):
        args += ["--model", model]
    started = time.time()
    with cleanly():
        try:
            os.makedirs(folder)
            p = subprocess.run(args, input=line.encode("utf-8"), capture_output=True, timeout=max(1, timeout), cwd=folder, creationflags=no_window())
        except subprocess.TimeoutExpired:
            raise RuntimeError("Antigravity took too long to make the image, so it was ended. Try once more later.")
        except OSError as e:
            raise RuntimeError("Antigravity could not be started (%s)." % (e.strerror or type(e).__name__))
        finally:
            shutil.rmtree(folder, ignore_errors=True)
    reply, conversation, errors = "", "", []
    for raw in p.stdout.decode("utf-8", "replace").splitlines():
        try:
            e = json.loads(raw)
        except ValueError:
            continue
        if not isinstance(e, dict):
            continue
        # the conversation this turn opened: the one its first line names
        conversation = conversation or str(e.get("conversation_id") or "")
        if e.get("event") == "result":
            r = e.get("result") if isinstance(e.get("result"), dict) else {}
            reply = str(r.get("response") or "")
            err = r.get("error")
            errors.append(str(err.get("message") or "") if isinstance(err, dict) else str(err or ""))
    raw = _made_in(conversation, [m.group(0).strip() for m in IMAGE_PATH.finditer(reply)], started)
    if raw is not None:
        return raw, conversation, ""
    return None, conversation, "\n".join(x for x in [p.stderr.decode("utf-8", "replace").strip()] + errors if x)


def antigravity_image(exe, prompt, timeout=420, aspect="1:1", model=None):
    """Have Antigravity generate one image and return its bytes.

    It is run without its own interface, in an empty folder of the app's own, and told it may run no command and
    write no file, so it can make the image and nothing else. The picture is taken only from Antigravity's own
    folder for the conversation the turn opened. `aspect` is the shape asked for ("1:1", "16:9", ...); the model
    keeps to it only roughly, so the caller crops. The turn runs on `model`, or on the cheapest Flash model agy
    lists. `timeout` is for the whole call, the question about models and a second try included. Every call is one
    start on the hourly counts (a texture's and an image's alike); past them, SpendLimit says to ask the user.
    """
    import time
    deadline = time.monotonic() + timeout
    left = lambda: deadline - time.monotonic()
    sweep()
    refusal = spend_refusal()
    if refusal:
        raise SpendLimit(refusal)
    # (measured: agy's image helper looks at its own picture and may make a second one unasked, which costs the plan
    # twice; whether these words stop it was not measured)
    ask = ("Use your generate_image tool exactly once to make this image, then reply with only the full path of the image file "
           "it produced. Do nothing else and do not edit the image. One image only: whoever makes it does not judge it, "
           "redo it or make a second version. Image: %s. %s" % (_shape_words(aspect), prompt))
    model = model if flash_model(model) else cheapest_model(exe, timeout=min(30, left()))
    _note_start(time.time())
    raw, conversation, errors = _one_turn(exe, ask, model, left())
    if raw is None and model and not conversation and not _why_no_image(errors) and left() > 60:
        # agy would not start a turn on that model (it never opened a conversation): once more on its own default,
        # and no model is named for the rest of the day
        _remember_model(exe, "")
        raw, conversation, errors = _one_turn(exe, ask, None, left())
    if raw is not None:
        return raw
    raise RuntimeError(_why_no_image(errors) or "Antigravity did not produce an image this time. Try once more; if it "
                                               "fails again, tell the user and carry on without it.")


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
            except SpendLimit:
                # agents have started as many pictures this hour as they may: nothing more is generated by any
                # source (never the paid API instead), and the person is asked, as when there is no source
                notes.append("agents have started as many generated pictures this hour as they may, so the user was asked")
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
