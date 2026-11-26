# PyInstaller build for rbxkit. Run build_exe.bat.
# This builds fridae-cli.exe: the tool server and command line that agents start.
# Run build_app.py (or build_exe.bat), which also builds the window and puts the two together.
import os
import sys

HERE = os.path.abspath(os.getcwd())
sys.path.insert(0, HERE)
import build_datas  # noqa: E402

datas = [(os.path.join(HERE, d), d) for d in ("blender", "studio", "agents", "rigs", "licenses")]
# the window's page: index.html, the bundled screens (app.js, app.css), fonts and pictures; not the screens' source
# (ui/src), the libraries bundled into app.js (ui/vendor) or the bundling script
for folder, dirs, files in os.walk(os.path.join(HERE, "ui")):
    dirs[:] = [d for d in dirs if not (folder == os.path.join(HERE, "ui") and d in ("src", "vendor", "__pycache__"))]
    datas += [(os.path.join(folder, f), os.path.relpath(folder, HERE)) for f in files if f not in ("build.py", ".DS_Store")]
# the vault file by file, leaving out dot-folders, dot-files and vault/Roblox (the guides come from the Fridae vault)
datas += build_datas.vault_datas(HERE)
datas.append((os.path.join(HERE, "README.md"), "."))
datas.append((os.path.join(HERE, "routing.txt"), "."))
datas.append((os.path.join(HERE, "THIRD_PARTY.md"), "."))
# the promo engine's install copies this Remotion project into the engine folder; its node_modules are
# installed there by npm, never shipped (from source, `npm` may have left one here)
for folder, dirs, files in os.walk(os.path.join(HERE, "promo")):
    dirs[:] = [d for d in dirs if d != "node_modules"]
    datas += [(os.path.join(folder, f), os.path.relpath(folder, HERE)) for f in files if f != ".DS_Store"]
hidden = ["rbx", "rbxmerge", "rbxtex", "rbximg", "rbximage", "rbxvideo", "rbxpack", "rbxgen", "rbxkit_mcp", "rbxpaths",
          "rbxstudio", "rbxproject", "rbxeconomy", "rbxmarket", "rbxresearch", "rbxteam", "rbxboard", "rbxstage", "rbxroster", "rbxvault", "rbxnotes", "rbxupkeep", "rbxskills",
          "rbxpromo", "rbxpromo.cli", "rbxpromo.tool", "rbxpromo.engine", "rbxpromo.probe", "rbxpromo.reference",
          "rbxpromo.audio", "rbxpromo.render", "rbxpromo.check", "rbxpromo.recipe",
          "rbxrag", "rbxrag.chunks", "rbxrag.engine", "rbxrag.stores", "rbxrag.models", "rbxrag.net", "rbxrag.setup",
          "rbxrag.serve", "rbxrag.cli"]
# the knowledge index (rbxrag): fastembed's models run on onnxruntime, count tokens with tokenizers, stem with
# py_rust_stemmers and hash with mmh3 (bm25), and Heavy talks to Qdrant through qdrant_client. Each is imported only
# inside a function, so PyInstaller would not see them: their modules, native libraries and data go in whole.
# fastembed and qdrant_client read their own version from their package metadata, and huggingface_hub finds hf_xet
# (fast model downloads) by its metadata, so that goes in too.
from PyInstaller.utils.hooks import collect_all, collect_dynamic_libs, copy_metadata
binaries = []
for pkg in ("fastembed", "qdrant_client", "tokenizers", "py_rust_stemmers"):
    d, b, h = collect_all(pkg)
    datas += d
    binaries += b
    hidden += h
# onnxruntime: its own hook (pyinstaller-hooks-contrib) brings the package; only the runtime is needed, not its
# training, quantization and transformers tools (which pull in onnx, sympy and torch when present)
binaries += collect_dynamic_libs("onnxruntime")
hidden += ["onnxruntime", "onnxruntime.capi._pybind_state", "mmh3", "loguru", "huggingface_hub"]
for dist in ("fastembed", "qdrant-client", "onnxruntime", "tokenizers", "huggingface-hub", "hf-xet", "py-rust-stemmers"):
    datas += copy_metadata(dist)

a = Analysis(["cli_main.py"], pathex=[HERE], datas=datas, binaries=binaries, hiddenimports=hidden,
             excludes=["tkinter", "matplotlib", "scipy", "pandas", "IPython", "pytest", "torch", "tensorflow", "webview", "clr",
                       "onnxruntime.training", "onnxruntime.quantization", "onnxruntime.transformers", "onnxruntime.tools", "onnx", "sympy"])
pyz = PYZ(a.pure)

# the window (Fridae.exe) is the Rust program in app-rs; build_app.py puts it beside these tools
cli = EXE(pyz, a.scripts, [], exclude_binaries=True, name="fridae-cli", console=True, upx=False, icon=os.path.join(HERE, "ui", "fridae.ico"))
import shutil
coll = COLLECT(cli, a.binaries, a.datas, name="Fridae", upx=False)

shutil.copyfile(os.path.join(HERE, "START-HERE-app.txt"), os.path.join(DISTPATH, "Fridae", "START HERE.txt"))
