"""Where rbxkit keeps things, whether it runs from source or as a packaged .exe.

KIT   read-only program files (blender/, studio/, vault/, rigs/)
WORK  everything the tools produce; always user-writable
"""
import json, os, sys

VERSION = "2.7.3"
UPDATE_REPO = "aprilfridae/fridae-studios"  # where published releases live

FROZEN = bool(getattr(sys, "frozen", False))

if FROZEN:
    KIT = getattr(sys, "_MEIPASS", os.path.dirname(sys.executable))
    HOME_DIR = os.path.join(os.path.expanduser("~"), "Documents", "Fridae Studios")
else:
    KIT = os.path.dirname(os.path.abspath(__file__))
    HOME_DIR = KIT

WORK = os.environ.get("RBXKIT_WORK") or os.path.join(HOME_DIR, "work")
CONFIG = os.path.join(HOME_DIR, "config.json")
KNOWLEDGE_DIR = os.path.join(HOME_DIR, "Knowledge")  # the shared Fridae vault's files (Skills/) and its base record


def _cli_exe():
    """The console program beside the window program in a packaged build."""
    folder = os.path.dirname(sys.executable)
    for name in ("fridae-cli.exe", "fridae-cli", "rbxkit-cli.exe"):
        if os.path.exists(os.path.join(folder, name)):
            return os.path.join(folder, name)
    return sys.executable


def cli(args):
    """Command line that runs the rbx tool with these arguments."""
    if FROZEN:
        return [_cli_exe(), "--rbx"] + list(args)
    return [sys.executable, os.path.join(KIT, "rbx.py")] + list(args)


def mcp_command():
    """Command line that starts the MCP server."""
    if FROZEN:
        return [_cli_exe(), "--mcp"]
    return [sys.executable, os.path.join(KIT, "rbxkit_mcp.py")]


def no_window():
    """subprocess flags that stop console windows flashing up on Windows."""
    return 0x08000000 if os.name == "nt" else 0


class KitError(Exception):
    """A failure whose message is written for the person or agent to read as it is, with no traceback."""


def read_json(path):
    """The JSON in a file, or None when the file is missing or is not whole, valid JSON."""
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def write_json(path, data, **options):
    """Write JSON beside the file and rename it into place, so a reader never finds half a file."""
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        json.dump(data, f, **options)
    os.replace(path + ".tmp", path)
