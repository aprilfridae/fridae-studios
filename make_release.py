#!/usr/bin/env python
"""Zip rbxkit for sharing. Leaves out your work files and local settings."""
import os, sys, zipfile

KIT = os.path.dirname(os.path.abspath(__file__))
import sys
sys.path.insert(0, KIT)
from rbxpaths import VERSION
SKIP_DIRS = {"work", "__pycache__", ".git", "build", "dist"}
SKIP_FILES = {"config.json"}


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(KIT), "rbxkit-%s.zip" % VERSION)
    n = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for root, dirs, files in os.walk(KIT):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for f in files:
                if f in SKIP_FILES or f.endswith((".pyc", ".rbxkit-backup", ".zip")):
                    continue
                full = os.path.join(root, f)
                z.write(full, os.path.join("rbxkit", os.path.relpath(full, KIT)))
                n += 1
    print("%s  (%d files, %.0f KB)" % (out, n, os.path.getsize(out) / 1024))


if __name__ == "__main__":
    main()
