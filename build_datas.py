"""What of vault/ the build bundles (rbxkit.spec): Fridae's own guides, team and code data, file by file. Dot-folders
and dot-files (Obsidian's .obsidian/ and .trash/, .DS_Store) stay out, and so does the whole of vault/Roblox: the
Roblox guides come from the Fridae vault into <home>/Knowledge/Roblox, never with the app."""
import os


def vault_datas(here):
    """[(source file, destination folder relative to here)] for every file the app carries from here/vault."""
    out = []
    root = os.path.join(here, "vault")
    for folder, dirs, files in os.walk(root):
        rel = os.path.relpath(folder, here)
        dirs[:] = sorted(d for d in dirs if not d.startswith(".") and not (folder == root and d == "Roblox"))
        out += [(os.path.join(folder, f), rel) for f in sorted(files) if not f.startswith(".")]
    return out
