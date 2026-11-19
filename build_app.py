#!/usr/bin/env python
"""Build Fridae Studios: the Rust window plus the Python tools, in one folder.

    python build_app.py            ->  <build dir>/dist/Fridae Studios

Needs Rust (cargo), PyInstaller, numpy, pillow and opencv-python.

Where things go:
    FRIDAE_BUILD_DIR   build folder (default: H:\\rbxkit-build if H: exists, else ./build-out)
    FRIDAE_RUST_HOME   a self-contained Rust install to use instead of the one on PATH
                       (a folder holding rustup/, cargo/ and target/). With the GNU
                       toolchain it links with the bundled rust-lld, so no Visual
                       Studio or MinGW install is needed.
"""
import glob, os, shutil, subprocess, sys

KIT = os.path.dirname(os.path.abspath(__file__))
APP_RS = os.path.join(KIT, "app-rs")
BUILD = os.environ.get("FRIDAE_BUILD_DIR") or (r"H:\rbxkit-build" if os.path.isdir("H:\\") else os.path.join(KIT, "build-out"))
EXE_NAME = "Fridae Studios.exe"


def rust_env():
    """Environment for cargo, and the folder its output lands in."""
    env = dict(os.environ)
    home = os.environ.get("FRIDAE_RUST_HOME") or (r"H:\rust" if os.path.isdir(r"H:\rust\cargo") else "")
    if not home:
        return env, os.environ.get("CARGO_TARGET_DIR") or os.path.join(APP_RS, "target")
    target = os.path.join(home, "target")
    env.update(RUSTUP_HOME=os.path.join(home, "rustup"), CARGO_HOME=os.path.join(home, "cargo"), CARGO_TARGET_DIR=target)
    paths = [os.path.join(home, "cargo", "bin")]
    gnu = glob.glob(os.path.join(home, "rustup", "toolchains", "*-pc-windows-gnu", "lib", "rustlib", "*-pc-windows-gnu", "bin"))
    if gnu:
        # the GNU toolchain's own linker mishandles the import libraries rustc makes here; rust-lld does not
        lld = os.path.join(gnu[0], "rust-lld.exe").replace("\\", "/")
        env["RUSTFLAGS"] = "-C linker-flavor=ld.lld -C linker=%s -C link-self-contained=yes" % lld
        paths += [os.path.join(home, "shim"), os.path.join(gnu[0], "self-contained")]
    env["PATH"] = os.pathsep.join(paths + [env.get("PATH", "")])
    return env, target


def build_window():
    """Compile the Rust program. Returns the folder holding fridae.exe."""
    env, target = rust_env()
    cargo = shutil.which("cargo", path=env["PATH"])
    if not cargo:
        sys.exit("cargo was not found: install Rust from https://rustup.rs, or set FRIDAE_RUST_HOME")
    subprocess.run([cargo, "build", "--release"], cwd=APP_RS, env=env, check=True)
    out = os.path.join(target, "release")
    if not os.path.isfile(os.path.join(out, "fridae.exe")):
        sys.exit("cargo finished but %s is missing" % os.path.join(out, "fridae.exe"))
    return out, env


def webview_loader(env):
    """Microsoft's WebView2Loader.dll, which the window needs beside it (it ships inside the webview2-com-sys crate)."""
    cargo_home = env.get("CARGO_HOME") or os.path.join(os.path.expanduser("~"), ".cargo")
    hits = sorted(glob.glob(os.path.join(cargo_home, "registry", "src", "*", "webview2-com-sys-*", "x64", "WebView2Loader.dll")))
    if not hits:
        sys.exit("WebView2Loader.dll was not found in the cargo registry")
    return hits[-1]


def check_page():
    """Refuse to build if the window's page does not start cleanly: one bad line in its script leaves every
    button dead. The real page is loaded in a headless Edge or Chrome with the server calls stubbed out, and
    any error while it loads (a script that does not parse, or one that fails as it starts) stops the build."""
    import re, tempfile
    page = open(os.path.join(KIT, "ui", "index.html"), encoding="utf-8").read()
    browser = next((p for p in (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                                r"C:\Program Files\Google\Chrome\Application\chrome.exe") if os.path.exists(p)), None)
    if "<script>" not in page or not browser:
        sys.exit("Cannot check the window's page: %s" % ("no <script> in ui/index.html" if browser else "Edge or Chrome is needed"))
    trap = ("<script>window.__errors = []; window.onerror = function (m, s, l) { window.__errors.push(m + ' (line ' + l + ')'); };"
            "window.fetch = function () { return new Promise(function () {}); };</script>")
    verdict = ("<script>setTimeout(function () { document.title = window.__errors.length ? 'PAGE-BROKEN ' + window.__errors.join(' | ') : "
               "(typeof send === 'function' && document.getElementById('send').onclick ? 'PAGE-OK' : 'PAGE-BROKEN the script did not finish loading'); }, 300);</script>")
    probe_page = page.replace("<script>", trap + "<script>", 1).replace("</body>", verdict + "</body>", 1)
    with tempfile.TemporaryDirectory() as tmp:
        probe = os.path.join(tmp, "probe.html")
        with open(probe, "w", encoding="utf-8") as f:
            f.write(probe_page)
        out = subprocess.run([browser, "--headless=new", "--disable-gpu", "--user-data-dir=" + os.path.join(tmp, "profile"), "--virtual-time-budget=3000",
                              "--dump-dom", "file:///" + probe.replace("\\", "/")], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=90).stdout
    title = re.search(r"<title>(.*?)</title>", out or "", flags=re.S)
    verdict = title.group(1) if title else "no answer from the browser"
    if verdict != "PAGE-OK":
        sys.exit("ui/index.html is broken and was not built: %s" % verdict)
    print("The window's page starts cleanly.")


def build(quiet=False):
    check_page()
    dist = os.path.join(BUILD, "dist")
    app = os.path.join(dist, "Fridae Studios")
    print("Building the window (Rust) ...")
    out, env = build_window()
    print("Building the tools (Python) ...")
    pipe = subprocess.DEVNULL if quiet else None
    subprocess.run([sys.executable, "-m", "PyInstaller", os.path.join(KIT, "rbxkit.spec"), "--noconfirm",
                    "--distpath", dist, "--workpath", os.path.join(BUILD, "build")], cwd=KIT, check=True, stdout=pipe, stderr=pipe)
    exe = os.path.join(app, EXE_NAME)
    shutil.copyfile(os.path.join(out, "fridae.exe"), exe)
    shutil.copyfile(webview_loader(env), os.path.join(app, "WebView2Loader.dll"))
    from PyInstaller import config
    from PyInstaller.utils.win32 import icon
    config.CONF.setdefault("workpath", os.path.join(BUILD, "build"))
    icon.CopyIcons(exe, os.path.join(KIT, "ui", "fridae.ico"))
    print("Built", app)
    return app


if __name__ == "__main__":
    build()
