#!/usr/bin/env python
"""Bundle the front end's new screens:  python ui/build.py  ->  ui/app.js and ui/app.css

ui/src/main.tsx and what it imports (Preact components in TypeScript, each screen's CSS) become one small script
and one stylesheet, which ui/index.html loads. The bundler is the esbuild program in tools/ and Preact is the copy
in ui/vendor/preact: nothing is downloaded and no Node is needed. Types are stripped, not checked.

    python ui/build.py            one build
    python ui/build.py --watch    rebuild whenever a file under ui/src changes

Both outputs are committed, so a build on a computer without esbuild (the Mac build, until esbuild for macOS is in
tools/) still ships the screens.
"""
import os, shutil, subprocess, sys

UI = os.path.dirname(os.path.abspath(__file__))
KIT = os.path.dirname(UI)
OUT_JS, OUT_CSS = os.path.join(UI, "app.js"), os.path.join(UI, "app.css")


def esbuild():
    """The bundler: the one in tools/ on Windows, else one on PATH; None when there is neither."""
    own = os.path.join(KIT, "tools", "esbuild.exe")
    if os.name == "nt" and os.path.isfile(own):
        return own
    return shutil.which("esbuild")


def bundle(watch=False):
    """Write ui/app.js and ui/app.css. Exits with a message when the bundling fails."""
    exe = esbuild()
    if not exe:
        if os.path.isfile(OUT_JS) and os.path.isfile(OUT_CSS):
            print("esbuild was not found: using ui/app.js and ui/app.css as they are.")
            return
        sys.exit("esbuild was not found and ui/app.js is missing: the front end cannot be built")
    preact = os.path.join(UI, "vendor", "preact")
    cmd = [exe, os.path.join(UI, "src", "main.tsx"), "--bundle", "--format=esm", "--jsx=automatic", "--jsx-import-source=preact",
           "--alias:preact=" + preact, "--alias:preact/hooks=" + os.path.join(preact, "hooks"),
           "--alias:preact/jsx-runtime=" + os.path.join(preact, "jsx-runtime"),
           "--minify", "--target=es2020", "--legal-comments=none", "--outfile=" + OUT_JS]
    cmd += ["--watch", "--log-level=info"] if watch else ["--log-level=warning"]
    done = subprocess.run(cmd, cwd=KIT)
    if done.returncode:
        sys.exit("The front end did not bundle (ui/src): see the errors above")
    print("Bundled ui/app.js (%.1f KB) and ui/app.css (%.1f KB)." % (os.path.getsize(OUT_JS) / 1024, os.path.getsize(OUT_CSS) / 1024))


if __name__ == "__main__":
    bundle(watch="--watch" in sys.argv[1:])
