#!/usr/bin/env python3
"""Build Fridae for macOS:  python3 build_mac.py  ->  build-out/Fridae-macOS.dmg

Needs Rust (cargo), and PyInstaller, numpy, pillow, opencv-python, fastembed 0.9 and qdrant-client 1.20 (the
knowledge index) in the Python running this (`python3 install.py` installs the libraries).
The download is a .dmg on purpose: the Windows app updates itself from the first .zip in the
latest release, so a Mac .zip there could be installed over a Windows copy.
The app is not signed with an Apple Developer ID, so the first open needs "Open Anyway".
"""
import os, shutil, subprocess, sys, tempfile

KIT = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, KIT)
from rbxpaths import VERSION

OUT = os.environ.get("FRIDAE_BUILD_DIR") or os.path.join(KIT, "build-out")
APP_NAME = "Fridae"
# the folder PyInstaller makes (rbxkit.spec's COLLECT name)
TOOLS_NAME = "Fridae"


def run(*cmd, **kw):
    subprocess.run(list(cmd), check=True, **kw)


def icns(dest):
    """ui/fridae.ico -> AppIcon.icns."""
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "AppIcon.iconset")
        os.makedirs(iconset)
        from PIL import Image
        img = Image.open(os.path.join(KIT, "ui", "fridae.ico"))
        img = img.convert("RGBA")
        for s in (16, 32, 128, 256, 512):
            img.resize((s, s), Image.LANCZOS).save(os.path.join(iconset, "icon_%dx%d.png" % (s, s)))
            img.resize((s * 2, s * 2), Image.LANCZOS).save(os.path.join(iconset, "icon_%dx%d@2x.png" % (s, s)))
        run("iconutil", "-c", "icns", iconset, "-o", dest)


def main():
    if sys.platform != "darwin":
        sys.exit("build_mac.py builds on a Mac only")
    os.makedirs(OUT, exist_ok=True)
    print("Building the window (Rust) ...")
    run("cargo", "build", "--release", cwd=os.path.join(KIT, "app-rs"))
    print("Building the tools (Python) ...")
    dist, work = os.path.join(OUT, "dist"), os.path.join(OUT, "build")
    run(sys.executable, "-m", "PyInstaller", os.path.join(KIT, "rbxkit.spec"), "--noconfirm", "--distpath", dist, "--workpath", work, cwd=KIT)

    app = os.path.join(OUT, APP_NAME + ".app")
    shutil.rmtree(app, ignore_errors=True)
    macos = os.path.join(app, "Contents", "MacOS")
    os.makedirs(macos)
    # the tools go in Resources: code signing allows only programs in Contents/MacOS
    shutil.copytree(os.path.join(dist, TOOLS_NAME), os.path.join(app, "Contents", "Resources", "tools"), symlinks=True)
    shutil.copyfile(os.path.join(KIT, "app-rs", "target", "release", "fridae"), os.path.join(macos, APP_NAME))
    os.chmod(os.path.join(macos, APP_NAME), 0o755)
    res = os.path.join(app, "Contents", "Resources")
    os.makedirs(res, exist_ok=True)
    icns(os.path.join(res, "AppIcon.icns"))
    with open(os.path.join(app, "Contents", "Info.plist"), "w") as f:
        f.write("""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>{n}</string>
<key>CFBundleDisplayName</key><string>{n}</string>
<key>CFBundleIdentifier</key><string>io.github.aprilfridae.fridae-studios</string><!-- kept: macOS ties permissions to it -->
<key>CFBundleVersion</key><string>{v}</string>
<key>CFBundleShortVersionString</key><string>{v}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleExecutable</key><string>{n}</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
""".format(n=APP_NAME, v=VERSION))
    run("codesign", "--force", "--deep", "--sign", "-", app)

    dmg = os.path.join(OUT, "Fridae-macOS.dmg")
    with tempfile.TemporaryDirectory() as stage:
        shutil.copytree(app, os.path.join(stage, APP_NAME + ".app"), symlinks=True)
        os.symlink("/Applications", os.path.join(stage, "Applications"))
        if os.path.exists(dmg):
            os.remove(dmg)
        run("hdiutil", "create", "-volname", APP_NAME, "-srcfolder", stage, "-format", "UDZO", "-ov", dmg)
    print("Built", dmg)


if __name__ == "__main__":
    main()
