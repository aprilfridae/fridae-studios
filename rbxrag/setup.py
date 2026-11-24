"""The knowledge index's setup steps, run one at a time by the app (or `rbx rag setup <step>`), each idempotent:

  models          the fastembed models into <data>/models (Light: dense, sparse, rerank; Heavy: sparse, rerank)
  ollama-install  Ollama, only when it is not already answering: Ollama.app into ~/Applications, then opened
  ollama-pull     the embedding model (mxbai-embed-large) through the Ollama API, with progress
  qdrant-install  the Qdrant binary (a pinned release) into <data>/bin/qdrant

Each reports progress as (step, fraction) and ends with {"ready", "missing", "error"}. A step whose part is already
there ends ready at once, without a download. Nothing here runs except when the user starts the step.
"""
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.request

from rbxrag import models
from rbxrag.net import LOOPBACK

STEPS = ("models", "ollama-install", "ollama-pull", "qdrant-install")
OLLAMA_ZIP = "https://ollama.com/download/Ollama-darwin.zip"
# Pinned Qdrant release (the native binary, not Docker). Bump deliberately; the asset names are Qdrant's own.
QDRANT_VERSION = "v1.16.0"
QDRANT_ASSET = {"arm64": "qdrant-aarch64-apple-darwin.tar.gz", "x86_64": "qdrant-x86_64-apple-darwin.tar.gz"}
USER_AGENT = "fridae-studios"


class SetupError(RuntimeError):
    pass


def models_dir(data):
    return os.path.join(data, "models")


def qdrant_binary(data):
    return os.path.join(data, "bin", "qdrant")


def qdrant_url():
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    return "https://github.com/qdrant/qdrant/releases/download/%s/%s" % (QDRANT_VERSION, QDRANT_ASSET[arch])


# ------------------------------------------------------------------ small helpers (each one can be replaced in tests)
def http_json(url, body=None, timeout=2.0):
    """GET (or POST a JSON body) to a local service; the parsed JSON answer, or None when it does not answer."""
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"} if data else {})
    try:
        with LOOPBACK.open(req, timeout=timeout) as r:
            return json.load(r)
    except (urllib.error.URLError, OSError, ValueError):
        return None


def download(url, dest, on_fraction=None, timeout=60):
    """Download url to dest (through dest.part, renamed only when complete)."""
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    part = dest + ".part"
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r, open(part, "wb") as f:
            total = int(r.headers.get("Content-Length") or 0)
            got = 0
            while True:
                chunk = r.read(1 << 20)
                if not chunk:
                    break
                f.write(chunk)
                got += len(chunk)
                if on_fraction and total:
                    on_fraction(min(1.0, got / total))
    except (urllib.error.URLError, OSError) as e:
        try:
            os.remove(part)
        except OSError:
            pass
        raise SetupError("Could not download %s: %s. Check the internet connection and try again." % (url, e))
    os.replace(part, dest)


def fastembed_download(kind, name, cache_dir):
    """Download one fastembed model's files into cache_dir (no ONNX session is opened)."""
    models.load_fastembed(kind, name, cache_dir, local_only=False, lazy=True)


# ------------------------------------------------------------------ checks
def ollama_app():
    for d in (os.path.expanduser("~/Applications"), "/Applications"):
        p = os.path.join(d, "Ollama.app")
        if os.path.isdir(p):
            return p
    return None


def ollama_state(url=models.OLLAMA_URL, get=http_json):
    if get(url.rstrip("/") + "/api/version") is not None:
        return "running"
    if ollama_app() or shutil.which("ollama"):
        return "stopped"
    return "missing"


def ollama_has_model(url=models.OLLAMA_URL, get=http_json, model=models.HEAVY["dense"]):
    tags = get(url.rstrip("/") + "/api/tags")
    names = [m.get("name") or m.get("model") or "" for m in (tags or {}).get("models", []) if isinstance(m, dict)]
    return any(n == model or n.split(":")[0] == model for n in names)


def has_qdrant(data):
    p = qdrant_binary(data)
    return os.path.isfile(p) and os.access(p, os.X_OK)


def missing_models(mode, data, present=models.model_present):
    return [name for kind, name in models.fastembed_models(mode) if not present(kind, name, models_dir(data))]


def check(mode, data, url=models.OLLAMA_URL, get=http_json, present=models.model_present):
    """What is set up: no network except the local Ollama."""
    state = ollama_state(url, get)
    return {"models": not missing_models(mode, data, present), "ollama": state,
            "ollama_model": state == "running" and ollama_has_model(url, get), "qdrant_binary": has_qdrant(data)}


# ------------------------------------------------------------------ steps
def step_models(mode, data, progress, downloader=fastembed_download, present=models.model_present):
    todo = missing_models(mode, data, present)
    names = dict((n, k) for k, n in models.fastembed_models(mode))
    for i, name in enumerate(todo):
        label = "models: " + name
        progress(label, 0.0)
        downloader(names[name], name, models_dir(data))
        progress(label, 1.0)
    return missing_models(mode, data, present)


def _wait_for_ollama(url, get, wait, sleep=time.sleep):
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        if get(url.rstrip("/") + "/api/version") is not None:
            return True
        sleep(1.0)
    return get(url.rstrip("/") + "/api/version") is not None


def step_ollama_install(mode, data, progress, url=models.OLLAMA_URL, get=http_json, fetch=download,
                        run=subprocess.run, wait=60):
    if get(url.rstrip("/") + "/api/version") is not None:
        return []
    if sys.platform != "darwin":
        raise SetupError("Fridae can install Ollama on a Mac only for now. Install it from ollama.com, then try again.")
    app = ollama_app()
    if not app:
        dest = os.path.join(data, "downloads", "Ollama-darwin.zip")
        progress("ollama-install: download", 0.0)
        fetch(OLLAMA_ZIP, dest, lambda f: progress("ollama-install: download", f))
        apps = os.path.expanduser("~/Applications")
        os.makedirs(apps, exist_ok=True)
        progress("ollama-install: unpack", 0.0)
        r = run(["ditto", "-x", "-k", dest, apps], capture_output=True, text=True)
        if r.returncode != 0:
            raise SetupError("Could not unpack Ollama: %s" % (r.stderr or "").strip()[:300])
        try:
            os.remove(dest)
        except OSError:
            pass
        app = os.path.join(apps, "Ollama.app")
        if not os.path.isdir(app):
            raise SetupError("The Ollama download did not hold Ollama.app.")
        progress("ollama-install: unpack", 1.0)
    progress("ollama-install: start", 0.0)
    run(["open", "-a", app], capture_output=True, text=True)
    if not _wait_for_ollama(url, get, wait):
        raise SetupError("Ollama was opened but is not answering yet. If it asks for anything, finish that in Ollama, "
                         "then press Try again.")
    progress("ollama-install: start", 1.0)
    return []


def _pull_stream(url, model, timeout=600):
    body = json.dumps({"model": model, "stream": True}).encode()
    req = urllib.request.Request(url.rstrip("/") + "/api/pull", data=body, headers={"Content-Type": "application/json"})
    try:
        with LOOPBACK.open(req, timeout=timeout) as r:
            for line in r:
                line = line.strip()
                if line:
                    try:
                        yield json.loads(line)
                    except ValueError:
                        continue
    except (urllib.error.URLError, OSError) as e:
        raise SetupError("Ollama stopped answering while downloading the model: %s" % e)


def step_ollama_pull(mode, data, progress, url=models.OLLAMA_URL, get=http_json, pull=_pull_stream):
    model = models.HEAVY["dense"]
    if get(url.rstrip("/") + "/api/version") is None:
        raise SetupError("Ollama is not running. Do step 1 (Install Ollama) first, or open Ollama.")
    if ollama_has_model(url, get, model):
        return []
    label = "ollama-pull: " + model
    progress(label, 0.0)
    done = False
    for ev in pull(url, model):
        if ev.get("error"):
            raise SetupError("Ollama could not download %s: %s" % (model, ev["error"]))
        total, got = ev.get("total"), ev.get("completed")
        if total and got is not None:
            progress(label, min(1.0, got / total))
        if ev.get("status") == "success":
            done = True
    if not done and not ollama_has_model(url, get, model):
        raise SetupError("Ollama did not finish downloading %s. Try again." % model)
    progress(label, 1.0)
    return []


def step_qdrant_install(mode, data, progress, fetch=download, url=None):
    if has_qdrant(data):
        return []
    url = url or qdrant_url()
    dest = os.path.join(data, "downloads", os.path.basename(url))
    progress("qdrant-install: download", 0.0)
    fetch(url, dest, lambda f: progress("qdrant-install: download", f))
    out = qdrant_binary(data)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    try:
        with tarfile.open(dest, "r:gz") as tar:
            member = next((m for m in tar.getmembers() if m.isfile() and os.path.basename(m.name) == "qdrant"), None)
            if member is None:
                raise SetupError("The Qdrant download did not hold the qdrant program.")
            src = tar.extractfile(member)
            tmp = out + ".part"
            with open(tmp, "wb") as f:
                shutil.copyfileobj(src, f)
        os.chmod(tmp, 0o755)
        os.replace(tmp, out)
    except tarfile.TarError as e:
        raise SetupError("The Qdrant download is damaged (%s). Try again." % e)
    finally:
        try:
            os.remove(dest)
        except OSError:
            pass
    progress("qdrant-install: download", 1.0)
    return []


def run_step(step, mode, data, progress=None, **hooks):
    """Run one step: {"ready", "missing", "error"}; never raises for an expected failure."""
    progress = progress or (lambda step, fraction: None)
    funcs = {"models": step_models, "ollama-install": step_ollama_install, "ollama-pull": step_ollama_pull,
             "qdrant-install": step_qdrant_install}
    if step not in funcs:
        return {"ready": False, "missing": [], "error": "Unknown step %r; one of: %s." % (step, ", ".join(STEPS))}
    if mode not in ("light", "heavy"):
        return {"ready": False, "missing": [], "error": "mode must be light or heavy."}
    fn = funcs[step]
    import inspect
    accepted = inspect.signature(fn).parameters
    kwargs = {k: v for k, v in hooks.items() if k in accepted}
    try:
        missing = fn(mode, data, progress, **kwargs)
        return {"ready": not missing, "missing": list(missing), "error": ""}
    except Exception as e:  # any failure is reported in the final line, in plain words
        missing = []
        if step == "models":
            try:
                missing = missing_models(mode, data, hooks.get("present", models.model_present))
            except Exception:
                pass
        return {"ready": False, "missing": missing, "error": str(e) or type(e).__name__}


def run_cli(step, mode, data, out=print, **hooks):
    """`rag setup <step> --json-progress`: one {"step", "fraction"} line per progress update, then the final line."""
    def progress(name, fraction):
        out(json.dumps({"step": name, "fraction": round(float(fraction), 4)}))
    done = run_step(step, mode, data, progress, **hooks)
    out(json.dumps(done))
    return done
