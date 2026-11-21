"""The promo engine: a portable Node, the Remotion project, a headless browser, whisper.cpp and the
voice model, all kept in one folder and installed on first use.

The install is a list of steps. Each finished step is written to .state.json, so an install that
was interrupted (or whose npm step failed) picks up at the step it stopped at instead of starting over.
"""
import hashlib, json, os, platform, shutil, subprocess, sys, tarfile, urllib.request, zipfile

import rbxpaths

NODE_MAJOR = "v22."
NODE_FALLBACK = "v22.12.0"  # only used when no version is given
NODE_DIST = "https://nodejs.org/dist"
STEPS = ["node", "renderer", "browser", "whisper", "voice"]
# rough download sizes in MB, shown before anything is fetched
_SIZE_MB = {"node": 45, "renderer": 330, "browser": 90, "whisper": 160, "voice": 90}
WHISPER_MODEL = "base.en"


class EngineError(rbxpaths.KitError):
    """The engine is missing a part, or a download or install step failed."""


def engine_dir():
    """Documents/Fridae Studios/engine when packaged, <program folder>/engine from source."""
    base = rbxpaths.HOME_DIR if rbxpaths.FROZEN else rbxpaths.KIT
    return os.path.join(base, "engine")


def _p(*parts):
    return os.path.join(engine_dir(), *parts)


def _source_dir():
    return os.path.join(rbxpaths.KIT, "promo")


# ---------------------------------------------------------------- platform / downloads

def _platform_key(plat=None, machine=None):
    plat = plat or sys.platform
    machine = (machine or platform.machine()).lower()
    arch = {"amd64": "x64", "x86_64": "x64", "x64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(machine)
    if plat == "win32" and arch == "x64":
        return "win-x64", "zip"
    if plat == "darwin" and arch in ("arm64", "x64"):
        return "darwin-" + arch, "tar.gz"
    raise EngineError(f"The promo maker supports Windows 10/11 (64-bit) and macOS 12+, not {plat} on {machine}.")


def node_url(plat=None, machine=None, version=None):
    """Download URL of the Node archive for this platform. install resolves the newest 22.x LTS and passes it."""
    key, ext = _platform_key(plat, machine)
    version = version or NODE_FALLBACK
    return f"{NODE_DIST}/{version}/node-{version}-{key}.{ext}"


def _fetch(url, timeout=60):
    req = urllib.request.Request(url, headers={"User-Agent": "fridae-studios"})
    try:
        return urllib.request.urlopen(req, timeout=timeout)
    except Exception as e:
        raise EngineError(f"Could not download {url}: {e}. Check the internet connection and run install again.")


def _node_release():
    """The newest Node 22 LTS version string, e.g. v22.22.0."""
    with _fetch(f"{NODE_DIST}/index.json") as r:
        index = json.load(r)
    for entry in index:  # newest first
        if entry.get("lts") and str(entry.get("version", "")).startswith(NODE_MAJOR):
            return entry["version"]
    raise EngineError("nodejs.org lists no Node 22 LTS release.")


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def verify_sha256(path, expected):
    """Raise EngineError (and delete the file) unless the file's SHA-256 is the expected one."""
    actual = sha256_of(path)
    if actual.lower() != str(expected).strip().lower():
        try:
            os.remove(path)
        except OSError:
            pass
        raise EngineError(f"Checksum mismatch for {os.path.basename(path)}: the download is corrupt or was altered, "
                          "so it was deleted. Run install again.")


def _download(url, dest, on_fraction=None):
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    part = dest + ".part"
    with _fetch(url) as r, open(part, "wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        got = 0
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
            got += len(chunk)
            if on_fraction and total:
                on_fraction(got / total)
    os.replace(part, dest)


# ---------------------------------------------------------------- state

def _load_state():
    data = rbxpaths.read_json(_p(".state.json"))
    done = data.get("done") if isinstance(data, dict) else None
    return [s for s in STEPS if isinstance(done, list) and s in done]


def _mark_done(step):
    os.makedirs(engine_dir(), exist_ok=True)
    done = _load_state()
    if step not in done:
        done.append(step)
    rbxpaths.write_json(_p(".state.json"), {"done": done})


def _node_path():
    exe = _p("node", "node.exe") if os.name == "nt" else _p("node", "bin", "node")
    return exe if os.path.isfile(exe) else None


def _compositor_dir():
    base = _p("renderer", "node_modules", "@remotion")
    if not os.path.isdir(base):
        return None
    for name in sorted(os.listdir(base)):
        if name.startswith("compositor-") and os.path.isdir(os.path.join(base, name)):
            return os.path.join(base, name)
    return None


def _whisper_ok():
    return os.path.isfile(_p("whisper", f"ggml-{WHISPER_MODEL}.bin")) and os.path.isdir(_p("whisper", "whisper.cpp"))


def status():
    """What is installed: node path, one bool per part, ready, the missing parts in install order, and the download size left."""
    done = _load_state()
    node = _node_path()
    has = {
        "node": node is not None,
        "renderer": "renderer" in done and os.path.isdir(_p("renderer", "node_modules", "remotion")),
        "browser": "browser" in done,
        "whisper": "whisper" in done and _whisper_ok(),
        "voice": "voice" in done,
    }
    missing = [s for s in STEPS if not has[s]]
    return {"node": node, "renderer": has["renderer"], "browser": has["browser"], "whisper": has["whisper"],
            "voice": has["voice"], "ready": not missing, "missing": missing,
            "download_mb": sum(_SIZE_MB[s] for s in missing)}


def _require_ready():
    st = status()
    if not st["ready"]:
        raise EngineError("The promo engine is not fully installed (missing: " + ", ".join(st["missing"]) +
                          f"). Run `python3 -m rbxpromo.engine install` (about {st['download_mb']} MB left to download); "
                          "it continues where it stopped.")


# ---------------------------------------------------------------- running things

def _env():
    env = dict(os.environ)
    node_bin = os.path.dirname(_node_path() or "")
    env["PATH"] = node_bin + os.pathsep + env.get("PATH", "")
    env["npm_config_cache"] = _p("npm-cache")
    env["npm_config_update_notifier"] = "false"
    env["npm_config_fund"] = "false"
    env["npm_config_audit"] = "false"
    return env


def _run(cmd, cwd, what, ok_marker=None):
    try:
        r = subprocess.run(cmd, cwd=cwd, env=_env(), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           text=True, encoding="utf-8", errors="replace", creationflags=rbxpaths.no_window())
    except OSError as e:
        raise EngineError(f"{what} could not start: {e}")
    if r.returncode != 0 and not (ok_marker and ok_marker in (r.stdout or "")):
        tail = "\n".join((r.stdout or "").strip().splitlines()[-15:])
        raise EngineError(f"{what} failed (exit {r.returncode}). Run install again to retry.\n{tail}")
    return r.stdout


def _npm_cli(name):
    folder = _p("node", "node_modules") if os.name == "nt" else _p("node", "lib", "node_modules")
    return os.path.join(folder, "npm", "bin", name)


def node_cmd(script, *args):
    """Command line that runs a script with the engine's own Node."""
    node = _node_path()
    if not node:
        raise EngineError("The promo engine has no Node yet. Run `python3 -m rbxpromo.engine install`.")
    return [node, script] + list(args)


def _binary(name):
    _require_ready()
    folder = _compositor_dir()
    exe = os.path.join(folder, name + (".exe" if os.name == "nt" else "")) if folder else None
    if not exe or not os.path.isfile(exe):
        raise EngineError(f"{name} was not found in the engine's Remotion compositor package. "
                          "Delete the engine folder's renderer/ and run install again.")
    return exe


def tool_env():
    """Environment for running ffmpeg/ffprobe: on macOS they find their libav*.dylib files only via this path."""
    env = dict(os.environ)
    folder = _compositor_dir()
    if folder:
        env["DYLD_LIBRARY_PATH"] = folder + os.pathsep + env.get("DYLD_LIBRARY_PATH", "")
    return env


def ffmpeg():
    return _binary("ffmpeg")


def ffprobe():
    return _binary("ffprobe")


# ---------------------------------------------------------------- install steps

def _step_node(progress):
    version = _node_release()
    key, ext = _platform_key()
    name = f"node-{version}-{key}"
    base = f"{NODE_DIST}/{version}"
    with _fetch(base + "/SHASUMS256.txt") as r:
        sums = r.read().decode("utf-8", "replace")
    expected = next((ln.split()[0] for ln in sums.splitlines() if ln.strip().endswith(f"{name}.{ext}")), None)
    if not expected:
        raise EngineError(f"SHASUMS256.txt for Node {version} has no entry for {name}.{ext}.")
    archive = _p("downloads", f"{name}.{ext}")
    _download(f"{base}/{name}.{ext}", archive, lambda f: progress("node", 0.8 * f))
    verify_sha256(archive, expected)
    progress("node", 0.85)
    stage = _p("node.tmp")
    shutil.rmtree(stage, ignore_errors=True)
    if ext == "zip":
        with zipfile.ZipFile(archive) as z:
            z.extractall(stage)
    else:
        with tarfile.open(archive) as t:
            t.extractall(stage)
    shutil.rmtree(_p("node"), ignore_errors=True)
    os.replace(os.path.join(stage, name), _p("node"))
    shutil.rmtree(stage, ignore_errors=True)
    os.remove(archive)


def _copy_sources():
    src, dst = _source_dir(), _p("renderer")
    if not os.path.isdir(src):
        raise EngineError(f"The promo project is missing from the program files ({src}).")
    os.makedirs(dst, exist_ok=True)
    for entry in os.listdir(src):
        if entry == "node_modules":
            continue
        s, d = os.path.join(src, entry), os.path.join(dst, entry)
        if os.path.isdir(s):
            shutil.rmtree(d, ignore_errors=True)
            shutil.copytree(s, d)
        else:
            shutil.copy2(s, d)


def _step_renderer(progress):
    _copy_sources()
    progress("renderer", 0.1)
    _run(node_cmd(_npm_cli("npm-cli.js"), "ci", "--omit=dev"), _p("renderer"), "npm install of the Remotion project")


def _step_browser(progress):
    cli = os.path.join(_p("renderer"), "node_modules", "@remotion", "cli", "remotion-cli.js")
    _run(node_cmd(cli, "browser", "ensure"), _p("renderer"), "Remotion's headless browser download")


def _step_whisper(progress):
    _run(node_cmd(os.path.join("scripts", "setup-whisper.mjs"), _p("whisper"), WHISPER_MODEL),
         _p("renderer"), "whisper.cpp install")


def _step_voice(progress):
    _run(node_cmd(os.path.join("scripts", "voice.mjs"), "--warm"), _p("renderer"), "voice model download",
         ok_marker="voice model ready")  # onnxruntime may abort at exit after the model is cached


_STEP_FUNCS = {"node": _step_node, "renderer": _step_renderer, "browser": _step_browser,
               "whisper": _step_whisper, "voice": _step_voice}


def install(progress=None, dry_run=False):
    """Run the install steps that are not done yet. Returns status(), or just the planned step names for dry_run."""
    done = _load_state()
    plan = [s for s in STEPS if s not in done]
    if dry_run:
        return plan
    os.makedirs(engine_dir(), exist_ok=True)
    report = progress or (lambda step, fraction: None)
    for step in plan:
        report(step, 0.0)
        _STEP_FUNCS[step](report)
        _mark_done(step)
        report(step, 1.0)
    return status()


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    cmd = argv[0] if argv else "status"
    try:
        if cmd == "status":
            st = status()
            for k in ("node", "renderer", "browser", "whisper", "voice", "missing", "download_mb"):
                print(f"{k}: {st[k]}")
            print(f"ready: {str(st['ready']).lower()}")
        elif cmd == "install":
            st = status()
            if not st["ready"]:
                print(f"Downloading about {st['download_mb']} MB into {engine_dir()}", flush=True)
            install(lambda step, f: print(f"{step}: {int(f * 100)}%", flush=True) if f in (0.0, 1.0) else None)
            print(f"ready: {str(status()['ready']).lower()}")
        else:
            print("usage: python3 -m rbxpromo.engine status|install")
            return 2
    except EngineError as e:
        print(e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
