#!/usr/bin/env python
"""Publish a new version of Fridae so every installed copy updates itself.

    python publish.py 1.9.2 "What changed, in a line or two"

It sets the version, builds the program (build_app.py), zips it, and creates a GitHub release
in rbxpaths.UPDATE_REPO with the zip attached. Needs Rust, PyInstaller and the
GitHub CLI (`gh`) signed in to the account that owns the repository.
Everything in a release is public.
"""
import os, re, subprocess, sys, zipfile

KIT = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, KIT)
from build_app import BUILD

OLD_EXE_NAME = "Fridae Studios.exe"


def main():
    if len(sys.argv) < 3 or not re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
        sys.exit(__doc__)
    version, notes = sys.argv[1], sys.argv[2]
    path = os.path.join(KIT, "rbxpaths.py")
    src = open(path, encoding="utf-8").read()
    src, n = re.subn(r'^VERSION = "[^"]+"', 'VERSION = "%s"' % version, src, count=1, flags=re.M)
    if n != 1:
        sys.exit("could not find VERSION in rbxpaths.py")
    open(path, "w", encoding="utf-8").write(src)
    repo = re.search(r'^UPDATE_REPO = "([^"]+)"', src, flags=re.M).group(1)

    print("Building %s ..." % version)
    import build_app
    app = build_app.build(quiet=True)
    dist = os.path.dirname(app)
    zpath = os.path.join(BUILD, "Fridae.zip")
    count = 0
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for root, _dirs, files in os.walk(app):
            for f in files:
                full = os.path.join(root, f)
                z.write(full, os.path.relpath(full, dist))
                count += 1
        # copies from before the rename look for "Fridae Studios.exe" in the update: the same program under that
        # name lets them update; it installs and opens Fridae.exe
        z.write(os.path.join(app, build_app.EXE_NAME), os.path.join(os.path.basename(app), OLD_EXE_NAME))
    print("Zipped %d files (%.0f MB)" % (count, os.path.getsize(zpath) / 1e6))
    print("Publishing to %s ..." % repo)
    subprocess.run(["gh", "release", "create", "v" + version, zpath, "--repo", repo,
                    "--title", "Fridae " + version, "--notes", notes], check=True)
    print("Published. Installed copies pick it up within a few hours, or at once from Setup > Check now.")


if __name__ == "__main__":
    main()
