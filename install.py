#!/usr/bin/env python
"""rbxkit installer. Run install.bat (Windows) or: python install.py

It installs the Python libraries the kit needs, finds Blender, and connects
the kit to the AI agents it finds on this computer. Run it again any time;
it only adds or refreshes its own entries.

  python install.py              set up everything it can find
  python install.py --no-pip     skip installing Python libraries
  python install.py --print      only print the connection settings
"""
import argparse, json, os, re, shutil, subprocess, sys

KIT = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.expanduser("~")
SERVER = os.path.join(KIT, "rbxkit_mcp.py")
LIBS = (("numpy", "numpy"), ("PIL", "pillow"), ("cv2", "opencv-python"),
        # the knowledge index (rbxrag): local embeddings, sparse vectors and reranking, and the vector store
        ("fastembed", "fastembed==0.9.*"), ("qdrant_client", "qdrant-client==1.20.*"))


def say(msg=""):
    print(msg, flush=True)


def fill(text):
    return text.replace("{{RBXKIT}}", KIT.replace("\\", "/")).replace("{{RBXKIT_WIN}}", KIT)


def install_libs():
    missing = []
    for mod, pipname in LIBS:
        try:
            __import__(mod)
        except ImportError:
            missing.append(pipname)
    if not missing:
        say("  Python libraries: already installed")
        return True
    say("  Installing Python libraries: " + ", ".join(missing))
    r = subprocess.run([sys.executable, "-m", "pip", "install", "--quiet"] + missing)
    if r.returncode != 0:
        say("  !! pip failed. Run this yourself:  python -m pip install " + " ".join(missing))
        return False
    return True


def copy_filled(src, dst):
    """Copy a folder, filling the install-path placeholders in text files."""
    for root, _dirs, files in os.walk(src):
        rel = os.path.relpath(root, src)
        out = os.path.join(dst, rel) if rel != "." else dst
        os.makedirs(out, exist_ok=True)
        for name in files:
            s, d = os.path.join(root, name), os.path.join(out, name)
            if name.lower().endswith((".md", ".lua", ".txt")):
                with open(s, encoding="utf-8") as f:
                    text = fill(f.read())
                with open(d, "w", encoding="utf-8", newline="") as f:  # LF on Windows too, as in the vault
                    f.write(text)
            else:
                shutil.copyfile(s, d)


# Claude Code gets Fridae's core guides as skills. They live in the vault as skill folders
# (vault/Fridae/Guides/Core/<guide>/SKILL.md); the folders in ~/.claude/skills are built from them at install time,
# with the folder and skill names they had when they were skills/<skill>/ (CLAUDE_SKILL_NAMES, CLAUDE_FM_NAMES: in
# Fridae each skill's name is its folder's) and their placeholders filled.
CORE_GUIDES = os.path.join(KIT, "vault", "Fridae", "Guides", "Core")
CLAUDE_SKILL_NAMES = {"workflow": "roblox-studio-workflow", "playtest": "roblox-playtest", "ui-design": "roblox-ui-design",
                      "sim-ui": "roblox-sim-ui", "vfx": "roblox-vfx", "promo": "roblox-promo"}  # else the guide's name
# the front-matter `name` each built skill keeps for Claude Code (else the guide's name)
CLAUDE_FM_NAMES = dict(CLAUDE_SKILL_NAMES, **{
    "brutalist-skill": "industrial-brutalist-ui", "minimalist-skill": "minimalist-ui",
    "redesign-skill": "redesign-existing-projects", "soft-skill": "high-end-visual-design"})


def built_text(src, text):
    """A core guide's file as Claude Code gets it: placeholders filled, and in its SKILL.md the Claude Code name."""
    text = fill(text)
    guide = os.path.basename(os.path.dirname(src))
    if os.path.basename(src) == "SKILL.md" and guide in CLAUDE_FM_NAMES:
        text = re.sub(r"\A(---\r?\n(?:.*\r?\n)*?)name:[^\r\n]*", lambda m: m.group(1) + "name: " + CLAUDE_FM_NAMES[guide],
                      text, count=1)
    return text


def claude_skills(core=CORE_GUIDES):
    """{skill folder name: {relative path in the skill folder: source file}} for every core guide."""
    out = {}
    for guide in sorted(os.listdir(core)):
        folder = os.path.join(core, guide)
        main = os.path.join(folder, "SKILL.md")
        if not os.path.isfile(main):
            continue
        files = {}
        for root, dirs, names in os.walk(folder):
            dirs[:] = [d for d in dirs if not d.startswith(".")]
            for name in names:
                if name == ".DS_Store":
                    continue
                src = os.path.join(root, name)
                rel = "SKILL.md" if src == main else os.path.relpath(src, folder).replace(os.sep, "/")
                files[rel] = src
        out[CLAUDE_SKILL_NAMES.get(guide, guide)] = files
    return out


INSTALLED_MARKER = ".fridae-installed.json"  # {"skill": the guide's id, "hash": rbxskills.folder_hash of the folder}


def build_claude_skills(dest, core=CORE_GUIDES):
    """Write each core guide as a Claude Code skill folder under dest; returns how many. Each folder gets
    INSTALLED_MARKER, so Fridae's skill import knows the copy is its own guide and whether it was edited since
    (folder_hash leaves dot-files out, so the marker is not part of the hash it records)."""
    from rbxskills import folder_hash
    skills = claude_skills(core)
    for name, files in skills.items():
        for rel, src in files.items():
            out = os.path.join(dest, name, *rel.split("/"))
            os.makedirs(os.path.dirname(out), exist_ok=True)
            if src.lower().endswith((".md", ".lua", ".txt")):
                with open(src, encoding="utf-8") as f:
                    text = built_text(src, f.read())
                with open(out, "w", encoding="utf-8", newline="") as f:  # LF on Windows too, as in the vault
                    f.write(text)
            else:
                shutil.copyfile(src, out)
        folder = os.path.join(dest, name)
        guide = os.path.basename(os.path.dirname(files["SKILL.md"]))
        with open(os.path.join(folder, INSTALLED_MARKER), "w", encoding="utf-8") as f:
            json.dump({"skill": guide, "hash": folder_hash(folder)}, f)
    return len(skills)


def install_claude():
    """Claude Code reads skills and agents from ~/.claude."""
    base = os.path.join(HOME, ".claude")
    if not os.path.isdir(base):
        return False
    n = build_claude_skills(os.path.join(base, "skills"))
    copy_filled(os.path.join(KIT, "agents"), os.path.join(base, "agents"))
    say("  Claude Code: %d skills and the roblox-tester agent installed in %s" % (n, base))
    return True


def server_entry():
    return {"command": sys.executable, "args": [SERVER]}


def add_json_server(path, label, key="mcpServers"):
    """Add or refresh the rbxkit entry in a JSON MCP config, keeping the rest."""
    cfg = {}
    if os.path.exists(path):
        try:
            with open(path, encoding="utf-8") as f:
                cfg = json.load(f)
        except ValueError:
            say("  %s: %s is not valid JSON, left alone. Add the entry by hand." % (label, path))
            return False
        shutil.copyfile(path, path + ".rbxkit-backup")
    cfg.setdefault(key, {})["rbxkit"] = server_entry()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(cfg, f, indent=2)
    say("  %s: connected (%s)" % (label, path))
    return True


def add_codex():
    path = os.path.join(HOME, ".codex", "config.toml")
    if not os.path.isdir(os.path.dirname(path)):
        return False
    text = open(path, encoding="utf-8").read() if os.path.exists(path) else ""
    if "[mcp_servers.rbxkit]" in text:
        say("  Codex: already connected (%s)" % path)
        return True
    block = '\n[mcp_servers.rbxkit]\ncommand = %s\nargs = [%s]\n' % (json.dumps(sys.executable), json.dumps(SERVER))
    with open(path, "a", encoding="utf-8") as f:
        f.write(block)
    say("  Codex: connected (%s)" % path)
    return True


def print_settings():
    say("\nTo connect any other agent that supports MCP servers, add this server:")
    say(json.dumps({"mcpServers": {"rbxkit": server_entry()}}, indent=2))
    say("\nCommand form:  %s %s" % (json.dumps(sys.executable), json.dumps(SERVER)))
    say("Claude Code:   claude mcp add rbxkit -- %s %s" % (json.dumps(sys.executable), json.dumps(SERVER)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--no-pip", action="store_true")
    ap.add_argument("--print", action="store_true", dest="only_print")
    a = ap.parse_args()
    if a.only_print:
        print_settings()
        return
    say("rbxkit setup\n")
    say("1. Requirements")
    if sys.version_info < (3, 9):
        say("  !! Python 3.9 or newer is needed (this is %s)." % sys.version.split()[0])
        sys.exit(1)
    if not a.no_pip:
        install_libs()
    say("\n2. Connecting to your AI agents")
    found = False
    found |= install_claude()
    if os.path.isdir(os.path.join(HOME, ".cursor")):
        found |= add_json_server(os.path.join(HOME, ".cursor", "mcp.json"), "Cursor")
    if os.path.isdir(os.path.join(HOME, ".codeium", "windsurf")):
        found |= add_json_server(os.path.join(HOME, ".codeium", "windsurf", "mcp_config.json"), "Windsurf")
    if os.path.isdir(os.path.join(HOME, ".gemini")):
        found |= add_json_server(os.path.join(HOME, ".gemini", "settings.json"), "Gemini CLI")
    desktop = os.path.join(os.environ.get("APPDATA", ""), "Claude")
    if os.environ.get("APPDATA") and os.path.isdir(desktop):
        found |= add_json_server(os.path.join(desktop, "claude_desktop_config.json"), "Claude desktop app")
    found |= add_codex()
    if not found:
        say("  No known agent found on this computer.")
    print_settings()
    say("\n3. Check")
    subprocess.run([sys.executable, os.path.join(KIT, "rbx.py"), "doctor"])
    say("\nLast step, in Roblox Studio: turn on the Studio MCP server (Assistant settings) and connect it to")
    say("your agent too. rbxkit supplies the tools around Studio; the Studio MCP is what edits your place.")
    say("Restart your agent so it picks up the new tools.")


if __name__ == "__main__":
    main()
