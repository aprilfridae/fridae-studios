#!/usr/bin/env python3
"""Build Fridae for Linux:  python3 build_linux.py
    ->  build-out/Fridae-linux-<arch>.AppImage   (when linuxdeploy and appimagetool are there)
    ->  build-out/Fridae-linux-<arch>.tar.gz     (always: the plain folder, for people who do not want an AppImage)

Needs Rust (cargo), the WebKitGTK and GTK development packages (Debian/Ubuntu: libwebkit2gtk-4.1-dev libgtk-3-dev
libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libssl-dev pkg-config build-essential), and PyInstaller, numpy, pillow,
opencv-python-headless, fastembed 0.9 and qdrant-client 1.20 in the Python running this.

The AppImage carries WebKitGTK and GTK with it (linuxdeploy with its GTK plugin, as Tauri's AppImages do), so it runs on
a distribution that does not have them. The .tar.gz does not: it needs the system's webkit2gtk-4.1 (Debian/Ubuntu
package libwebkit2gtk-4.1-0, Fedora webkit2gtk4.1).
Never a .zip: the Windows app updates itself from the first .zip in the latest release (update.rs, `pick_asset`).

Tools for the AppImage, from the environment or PATH:
    LINUXDEPLOY       linuxdeploy (its x86_64/aarch64 AppImage)
    LINUXDEPLOY_GTK   linuxdeploy-plugin-gtk.sh (found by linuxdeploy beside it or on PATH when not set)
    APPIMAGETOOL      appimagetool
Build on the oldest distribution you support (the workflow uses Ubuntu 22.04): the bundled Python and libraries then
run on newer ones.
"""
import os, platform, shutil, stat, subprocess, sys, tarfile

KIT = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, KIT)
from rbxpaths import VERSION

OUT = os.environ.get("FRIDAE_BUILD_DIR") or os.path.join(KIT, "build-out")
APP_NAME = "Fridae"
# the folder PyInstaller makes (rbxkit.spec's COLLECT name); the window program goes inside it, as on Windows, so
# the app finds fridae-cli and _internal/ui beside itself (paths.rs, PATHS)
TOOLS_NAME = "Fridae"
ARCH = {"amd64": "x86_64", "arm64": "aarch64"}.get(platform.machine().lower(), platform.machine().lower())
# a desktop launcher's file and icon name; StartupWMClass is GTK's default WM class, the program's own name
DESKTOP = """[Desktop Entry]
Type=Application
Name=Fridae
Comment=AI agents for your projects
Exec=Fridae %U
Icon=fridae
Terminal=false
Categories=Development;
StartupWMClass=Fridae
X-AppImage-Version={version}
"""
# the AppImage's start: GTK's and WebKit's hooks from linuxdeploy's plugins, then the program. The working folder is
# the AppImage's usr folder, where the patched WebKitGTK finds its helper programs (`patch_webkit`); the program
# itself only uses full paths, and the AI tools it starts get their project's folder.
APPRUN = """#!/bin/sh
APPDIR="${APPDIR:-$(dirname "$(readlink -f "$0")")}"
export APPDIR
for hook in "$APPDIR"/apprun-hooks/*.sh; do
    [ -f "$hook" ] && . "$hook"
done
cd "$APPDIR/usr" || exit 1
exec "$APPDIR/usr/lib/fridae/Fridae" "$@"
"""

# the .tar.gz's folder: adds Fridae to the desktop's app list, pointing at the folder wherever it was unpacked
INSTALL_LAUNCHER = """#!/bin/sh
# Adds Fridae to your desktop's list of apps (for this folder, wherever you unpacked it).
HERE="$(cd "$(dirname "$0")" && pwd)"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
mkdir -p "$apps" "$icons"
cp "$HERE/fridae.png" "$icons/fridae.png"
sed "s|^Exec=.*|Exec=\\"$HERE/Fridae\\" %U|" "$HERE/fridae.desktop" > "$apps/fridae.desktop"
echo "Fridae is in your apps now."
"""


def run(*cmd, **kw):
    print("+", " ".join(str(c) for c in cmd))
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def tool(env_name, *names):
    """A build tool from the environment, else PATH; None when it is not there."""
    set_to = os.environ.get(env_name)
    if set_to:
        return set_to if os.path.exists(set_to) else None
    return next((shutil.which(n) for n in names if shutil.which(n)), None)


def png_icon(dest, size=256):
    """ui/fridae.ico -> a PNG for the launcher."""
    from PIL import Image
    Image.open(os.path.join(KIT, "ui", "fridae.ico")).convert("RGBA").resize((size, size), Image.LANCZOS).save(dest)


def executable(path):
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def asset_name(ext):
    """The release file's name: Fridae-linux-x86_64.AppImage, Fridae-linux-x86_64.tar.gz (update.rs picks by these)."""
    assert ext in ("AppImage", "tar.gz"), ext
    return "Fridae-linux-%s.%s" % (ARCH, ext)


def build_folder():
    """The UI bundle, the Rust window and the PyInstaller tools, as one folder. Returns its path."""
    sys.path.insert(0, os.path.join(KIT, "ui"))
    import build as ui_build
    ui_build.bundle()
    print("Building the window (Rust) ...")
    app_rs = os.path.join(KIT, "app-rs")
    run("cargo", "build", "--release", cwd=app_rs)
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(app_rs, "target")
    window = os.path.join(target, "release", "fridae")
    print("Building the tools (Python) ...")
    dist, work = os.path.join(OUT, "dist"), os.path.join(OUT, "build")
    run(sys.executable, "-m", "PyInstaller", os.path.join(KIT, "rbxkit.spec"), "--noconfirm", "--distpath", dist, "--workpath", work, cwd=KIT)
    folder = os.path.join(dist, TOOLS_NAME)
    shutil.copyfile(window, os.path.join(folder, APP_NAME))
    executable(os.path.join(folder, APP_NAME))
    png_icon(os.path.join(folder, "fridae.png"))
    with open(os.path.join(folder, "fridae.desktop"), "w") as f:
        f.write(DESKTOP.format(version=VERSION))
    with open(os.path.join(folder, "install-launcher.sh"), "w") as f:
        f.write(INSTALL_LAUNCHER)
    executable(os.path.join(folder, "install-launcher.sh"))
    return folder


def make_tar(folder):
    out = os.path.join(OUT, asset_name("tar.gz"))
    if os.path.exists(out):
        os.remove(out)
    with tarfile.open(out, "w:gz") as t:
        t.add(folder, arcname=APP_NAME)
    print("Built", out)
    return out


def patch_webkit(appdir):
    """WebKitGTK starts its helper programs (WebKitWebProcess, WebKitNetworkProcess) from a folder written into the
    library at build time (/usr/lib/<triplet>/webkit2gtk-4.1). The AppImage carries that folder, and the path inside
    the library is made relative ("/usr" -> "././", the same length) so it is found from the AppImage's usr folder,
    where AppRun starts the program (the way Tauri's AppImages do it)."""
    triplet = "%s-linux-gnu" % ARCH
    system = "/usr/lib/%s/webkit2gtk-4.1" % triplet
    if not os.path.isdir(system):
        sys.exit("WebKitGTK's helper folder %s is missing: install libwebkit2gtk-4.1-dev" % system)
    inside = os.path.join(appdir, "usr", "lib", triplet, "webkit2gtk-4.1")
    shutil.rmtree(inside, ignore_errors=True)
    shutil.copytree(system, inside, symlinks=True)
    old, new = system.encode(), system.replace("/usr", "././", 1).encode()
    assert len(old) == len(new)
    for root, _dirs, files in os.walk(os.path.join(appdir, "usr", "lib")):
        for name in files:
            if name.startswith("libwebkit2gtk-4.1.so") or name.startswith("libjavascriptcoregtk-4.1.so"):
                path = os.path.join(root, name)
                if os.path.islink(path):
                    continue
                data = open(path, "rb").read()
                if old in data:
                    open(path, "wb").write(data.replace(old, new))
                    print("patched", path)


def make_appimage(folder):
    linuxdeploy, appimagetool = tool("LINUXDEPLOY", "linuxdeploy", "linuxdeploy-%s.AppImage" % ARCH), tool("APPIMAGETOOL", "appimagetool", "appimagetool-%s.AppImage" % ARCH)
    if not linuxdeploy or not appimagetool:
        print("No AppImage: linuxdeploy and appimagetool are needed (set LINUXDEPLOY and APPIMAGETOOL). The .tar.gz is built.")
        return None
    appdir = os.path.join(OUT, "AppDir")
    shutil.rmtree(appdir, ignore_errors=True)
    lib = os.path.join(appdir, "usr", "lib", "fridae")
    shutil.copytree(folder, lib, symlinks=True)
    os.makedirs(os.path.join(appdir, "usr", "bin"))
    os.symlink(os.path.join("..", "lib", "fridae", APP_NAME), os.path.join(appdir, "usr", "bin", APP_NAME))
    desktop, icon = os.path.join(OUT, "fridae.desktop"), os.path.join(OUT, "fridae.png")
    with open(desktop, "w") as f:
        f.write(DESKTOP.format(version=VERSION))
    png_icon(icon)
    env = dict(os.environ, ARCH=ARCH, DEPLOY_GTK_VERSION="3", NO_STRIP="1")
    gtk = os.environ.get("LINUXDEPLOY_GTK")
    if gtk:
        env["PATH"] = os.path.dirname(os.path.abspath(gtk)) + os.pathsep + env.get("PATH", "")
    # the libraries the window needs (GTK, WebKitGTK, libsoup, OpenSSL ...) go into usr/lib; the GTK plugin adds its
    # settings and the hook AppRun runs
    run(linuxdeploy, "--appdir", appdir, "--executable", os.path.join(lib, APP_NAME), "--desktop-file", desktop, "--icon-file", icon, "--plugin", "gtk", env=env)
    patch_webkit(appdir)
    for old in ("AppRun", "AppRun.wrapped"):
        p = os.path.join(appdir, old)
        if os.path.lexists(p):
            os.remove(p)
    with open(os.path.join(appdir, "AppRun"), "w") as f:
        f.write(APPRUN)
    executable(os.path.join(appdir, "AppRun"))
    out = os.path.join(OUT, asset_name("AppImage"))
    if os.path.exists(out):
        os.remove(out)
    run(appimagetool, appdir, out, env=env)
    print("Built", out)
    return out


def main():
    if not sys.platform.startswith("linux"):
        sys.exit("build_linux.py builds on Linux only")
    os.makedirs(OUT, exist_ok=True)
    folder = build_folder()
    make_tar(folder)
    make_appimage(folder)


if __name__ == "__main__":
    main()
