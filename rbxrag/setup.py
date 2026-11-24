"""The knowledge index's setup steps, run one at a time by the app (or `rbx rag setup <step>`), each idempotent:

  models          the fastembed models into <data>/models (Light: dense, sparse, rerank; Heavy: sparse, rerank)
  ollama-install  Ollama, only when it is not already answering: Ollama.app into ~/Applications, then opened
  ollama-pull     the embedding model (mxbai-embed-large) through the Ollama API, with progress
  qdrant-install  the Qdrant binary (a pinned release) into <data>/bin/qdrant

Each reports progress as (step, fraction) and ends with {"ready", "missing", "error"}. A step whose part is already
there ends ready at once, without a download. Nothing here runs except when the user starts the step.
"""
import hashlib
import http.client
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
# Pinned Qdrant release (the native binary, not Docker), with the SHA-256 of each asset from the GitHub release.
# Bump deliberately, both together; a binary of another version counts as not installed.
QDRANT_VERSION = "v1.20.2"
QDRANT_ASSET = {"arm64": "qdrant-aarch64-apple-darwin.tar.gz", "x86_64": "qdrant-x86_64-apple-darwin.tar.gz"}
QDRANT_SHA256 = {"arm64": "385b244e6283968b2be926358541b5bc19b9ada7ea9586e900684388bc912d5f",
                 "x86_64": "73a6538ccccb892ec2c6fa3431faf1e73fba7eb2179079adbd0c65ae4c55bf6a"}
USER_AGENT = "fridae-studios"


class SetupError(RuntimeError):
    pass


def models_dir(data):
    return os.path.join(data, "models")


def qdrant_binary(data):
    return os.path.join(data, "bin", "qdrant")


def _arch():
    return "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"


def qdrant_url(arch=None):
    return "https://github.com/qdrant/qdrant/releases/download/%s/%s" % (QDRANT_VERSION, QDRANT_ASSET[arch or _arch()])


def qdrant_version_file(data):
    return os.path.join(data, "bin", "qdrant.version")


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


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
    """Download url to dest (through dest.part, renamed only when complete). A short or broken download raises
    SetupError and leaves no .part file behind."""
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
        if total and got != total:
            raise SetupError("The download of %s stopped early (%d of %d bytes). Try again." % (url, got, total))
        os.replace(part, dest)
    except SetupError:
        _remove(part)
        raise
    except (urllib.error.URLError, http.client.HTTPException, OSError, ValueError) as e:
        _remove(part)
        raise SetupError("Could not download %s: %s. Check the internet connection and try again." % (url, e))
    except BaseException:
        _remove(part)
        raise


def _remove(path):
    try:
        os.remove(path)
    except OSError:
        pass


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
    """The pinned Qdrant is installed (a binary of another version, or of no recorded version, is not)."""
    p = qdrant_binary(data)
    if not (os.path.isfile(p) and os.access(p, os.X_OK)):
        return False
    try:
        with open(qdrant_version_file(data), encoding="utf-8") as f:
            return f.read().strip() == QDRANT_VERSION
    except OSError:
        return False


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


START_YOUR_OLLAMA = ("Ollama is installed but not running. Start your Ollama (open the Ollama app or run "
                     "`ollama serve`), then press Try again.")


def step_ollama_install(mode, data, progress, url=models.OLLAMA_URL, get=http_json, fetch=download,
                        run=subprocess.run, wait=60, which=shutil.which, find_app=ollama_app, apps_dir=None):
    if get(url.rstrip("/") + "/api/version") is not None:
        return []
    if sys.platform != "darwin":
        raise SetupError("Fridae can install Ollama on a Mac only for now. Install it from ollama.com, then try again.")
    if find_app() or which("ollama"):  # the user's own Ollama (the app, or a command such as Homebrew's): never a second one
        raise SetupError(START_YOUR_OLLAMA)
    apps = apps_dir or os.path.expanduser("~/Applications")
    downloads = os.path.join(data, "downloads")
    dest = os.path.join(downloads, "Ollama-darwin.zip")
    tmp = os.path.join(downloads, "tmp")
    try:
        progress("ollama-install: download", 0.0)
        fetch(OLLAMA_ZIP, dest, lambda f: progress("ollama-install: download", f))
        progress("ollama-install: unpack", 0.0)
        shutil.rmtree(tmp, ignore_errors=True)
        os.makedirs(tmp)
        r = run(["ditto", "-x", "-k", dest, tmp], capture_output=True, text=True)
        if r.returncode != 0:
            raise SetupError("Could not unpack Ollama: %s" % (r.stderr or "").strip()[:300])
        unpacked = os.path.join(tmp, "Ollama.app")
        if not os.path.isdir(os.path.join(unpacked, "Contents", "MacOS")):
            raise SetupError("The Ollama download did not hold a complete Ollama.app.")
        if which("codesign"):  # Apple's signature check; skipped only where the tool does not exist
            r = run(["codesign", "--verify", "--deep", "--strict", unpacked], capture_output=True, text=True)
            if r.returncode != 0:
                raise SetupError("The downloaded Ollama.app failed Apple's signature check, so it was not installed: "
                                 "%s" % (r.stderr or "").strip()[:300])
        os.makedirs(apps, exist_ok=True)
        app = os.path.join(apps, "Ollama.app")
        partial = os.path.join(apps, ".Ollama.app.partial")
        shutil.rmtree(partial, ignore_errors=True)
        try:  # copied in under a hidden name, then renamed in one step: never a half-copied app in Applications
            shutil.move(unpacked, partial)
            os.rename(partial, app)
        except BaseException:
            shutil.rmtree(partial, ignore_errors=True)
            raise
        progress("ollama-install: unpack", 1.0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        _remove(dest)
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


def step_qdrant_install(mode, data, progress, fetch=download, url=None, sha256=None, arch=None):
    if has_qdrant(data):
        return []
    arch = arch or _arch()
    url = url or qdrant_url(arch)
    want = (sha256 or QDRANT_SHA256[arch]).lower()
    dest = os.path.join(data, "downloads", os.path.basename(url))
    out = qdrant_binary(data)
    tmp = out + ".part"
    try:
        progress("qdrant-install: download", 0.0)
        fetch(url, dest, lambda f: progress("qdrant-install: download", f))
        got = sha256_of(dest)
        if got != want:
            raise SetupError("The Qdrant download does not match its published checksum, so it was deleted and not "
                             "installed. Try again.")
        os.makedirs(os.path.dirname(out), exist_ok=True)
        try:
            with tarfile.open(dest, "r:gz") as tar:
                member = next((m for m in tar.getmembers() if m.isfile() and os.path.basename(m.name) == "qdrant"),
                              None)
                if member is None:
                    raise SetupError("The Qdrant download did not hold the qdrant program.")
                with open(tmp, "wb") as f:
                    shutil.copyfileobj(tar.extractfile(member), f)
        except tarfile.TarError as e:
            raise SetupError("The Qdrant download is damaged (%s). Try again." % e)
        os.chmod(tmp, 0o755)
        os.replace(tmp, out)
        with open(qdrant_version_file(data), "w", encoding="utf-8") as f:
            f.write(QDRANT_VERSION + "\n")
    finally:
        _remove(dest)
        _remove(tmp)
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
