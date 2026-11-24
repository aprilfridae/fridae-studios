"""Skills for every AI in Fridae: one list of the user's skills, served to Claude, Gemini, Cursor, OpenCode and Codex.

A skill is a folder holding SKILL.md (front matter with `name` and `description`, then instructions) and any files it
uses. Claude Code loads some of them by itself; the other AIs load none, and Claude Code itself does not load the
skills the Claude desktop app syncs. So the app's tool server serves them all (`rbx_skill`), and every agent's
standing instructions carry a short catalog of them (`catalog`) so it knows when one applies.

Where skills are looked for, first found wins for a name:
  1. the project:          <project>/.claude/skills, <project>/.agents/skills
  2. Claude Code's own:    ~/.claude/skills/<skill>, and the skills of the enabled plugins in installed_plugins.json
  3. synced by Claude:     ~/.claude/plugins/synced/<account>/<plugin>/skills/<skill> and
                           ~/.claude/skills/synced/<account>/<skill> (every account's; the newest copy of a skill)
  4. other tools' folders: ~/.agents/skills, ~/.codex/skills, ~/.cursor/skills, ~/.config/opencode/skill(s),
                           ~/.gemini/skills
A plugin's skill is named `<plugin>:<skill>`, as Claude Code names it.
"""
import json
import os
import re

HOME = os.path.expanduser("~")
DESC_MAX = 170       # characters of a description in the catalog
READ_MAX = 100_000   # bytes of a file handed out by `read`
FILES_MAX = 80       # files listed by `load`

# Skills the Claude desktop app syncs that only work with the desktop app's own tools (its browser pane, Chrome
# extension, computer use, docs and Google connectors, artifacts, memory). No agent in Fridae has those tools.
APP_ONLY = {"built-in-browser", "chrome-browser", "computer-use", "docs", "google-workspace", "morning", "import-memory"}

ORIGIN_LABEL = {"project": "this project", "claude": "Claude Code", "claude-plugin": "a Claude Code plugin",
                "synced": "synced from Claude", "synced-plugin": "a plugin synced from Claude", "other": "another AI tool"}
# the origins Claude Code loads by itself (its Skill tool already lists them), besides its own account's synced ones
CLAUDE_NATIVE = {"project", "claude", "claude-plugin"}


def claude_dir():
    d = os.environ.get("FRIDAE_USER_CLAUDE") or os.environ.get("CLAUDE_CONFIG_DIR")
    return d if d and os.path.isdir(d) else os.path.join(HOME, ".claude")


def _read(path, limit=None):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read(limit) if limit else f.read()


def front(text):
    """(front matter as {key: value} for simple and folded keys, the text after it)."""
    m = re.match(r"^﻿?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(\r?\n|$)", text, re.S)
    if not m:
        return {}, text
    fm, key = {}, None
    for line in m.group(1).splitlines():
        mm = re.match(r"^([A-Za-z_][\w.-]*):[ \t]*(.*)$", line)
        if mm:
            key, val = mm.group(1), mm.group(2).strip()
            fm[key] = "" if val in (">", "|", ">-", "|-", ">+", "|+") else _unquote(val)
        elif key and line[:1] in (" ", "\t"):
            fm[key] = (fm[key] + " " + line.strip()).strip()
    return fm, text[m.end():]


def _unquote(val):
    if len(val) > 1 and val[0] == val[-1] == '"':
        return val[1:-1].replace('\\"', '"').replace("\\\\", "\\")
    if len(val) > 1 and val[0] == val[-1] == "'":
        return val[1:-1].replace("''", "'")
    return val


def _json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def _skill_dirs(root):
    """Each <root>/<name>/ holding SKILL.md, sorted."""
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return []
    return [os.path.join(root, n) for n in names
            if not n.startswith(".") and os.path.isfile(os.path.join(root, n, "SKILL.md"))]


def _plugin_name(plugin_dir, fallback):
    return (_json(os.path.join(plugin_dir, ".claude-plugin", "plugin.json")).get("name") or fallback).strip()


def _roots(project=None):
    """[(origin, plugin name or "", skills folder, plugin root or "", synced account folder name or "")] in the order
    a name is looked for."""
    out = []
    if project:
        for sub in (".claude", ".agents"):
            out.append(("project", "", os.path.join(project, sub, "skills"), "", ""))
    cd = claude_dir()
    out.append(("claude", "", os.path.join(cd, "skills"), "", ""))
    enabled = _json(os.path.join(cd, "settings.json")).get("enabledPlugins") or {}
    for key, installs in sorted((_json(os.path.join(cd, "plugins", "installed_plugins.json")).get("plugins") or {}).items()):
        if not enabled.get(key):
            continue
        for inst in installs if isinstance(installs, list) else []:
            root = (inst or {}).get("installPath") or ""
            if root:
                out.append(("claude-plugin", _plugin_name(root, key.split("@")[0]), os.path.join(root, "skills"), root, ""))
    # synced: every account's folder, newest first, so the newest copy of a skill wins
    synced = []
    base = os.path.join(cd, "plugins", "synced")
    for acct in _subdirs(base):
        for plugin in _subdirs(acct):
            synced.append((_mtime(plugin), ("synced-plugin", _plugin_name(plugin, os.path.basename(plugin)),
                                            os.path.join(plugin, "skills"), plugin, os.path.basename(acct))))
    for acct in _subdirs(os.path.join(cd, "skills", "synced")):
        synced.append((_mtime(acct), ("synced", "", acct, "", os.path.basename(acct))))
    out += [r for _, r in sorted(synced, key=lambda x: -x[0])]
    for rel in (".agents/skills", ".codex/skills", ".cursor/skills", ".config/opencode/skills", ".config/opencode/skill",
                ".gemini/skills"):
        out.append(("other", "", os.path.join(HOME, *rel.split("/")), "", ""))
    return out


def _subdirs(path):
    try:
        return [os.path.join(path, n) for n in sorted(os.listdir(path))
                if not n.startswith(".") and os.path.isdir(os.path.join(path, n))]
    except OSError:
        return []


def _mtime(path):
    try:
        return os.path.getmtime(path)
    except OSError:
        return 0.0


def _copies(project=None):
    """Every skill folder found, in the order a name is looked for (a name may be found more than once)."""
    for origin, plugin, root, plugin_root, bucket in _roots(project):
        for d in _skill_dirs(root):
            try:
                fm, _ = front(_read(os.path.join(d, "SKILL.md"), 8000))
            except OSError:
                continue
            name = (fm.get("name") or os.path.basename(d)).strip()
            if origin == "synced" and name in APP_ONLY:
                continue
            yield {"id": "%s:%s" % (plugin, name) if plugin else name, "name": name,
                   "description": " ".join((fm.get("description") or "").split()), "dir": d, "origin": origin,
                   "plugin": plugin, "plugin_root": plugin_root, "bucket": bucket}


def skills(project=None):
    """[{id, name, description, dir, origin, plugin, plugin_root, bucket}] for every skill, first found wins."""
    seen, out = set(), []
    for s in _copies(project):
        if s["id"].casefold() not in seen:
            seen.add(s["id"].casefold())
            out.append(s)
    return out


def claude_account(config=None):
    """The account id Claude Code is signed in with in this config folder (None: unknown). Claude Code names the
    folder it syncs an account's skills and plugins into `<...>_<account id>`. Only the id is read."""
    if config:
        path = os.path.join(config, ".claude.json")
    elif os.environ.get("CLAUDE_CONFIG_DIR"):
        path = os.path.join(os.environ["CLAUDE_CONFIG_DIR"], ".claude.json")
    else:
        path = os.path.join(HOME, ".claude.json")
    acct = (_json(path).get("oauthAccount") or {}).get("accountUuid")
    return acct if isinstance(acct, str) and acct else None


def claude_native(project=None, config=None):
    """The ids of the skills Claude Code loads by itself: the project's, its own folder's, its enabled plugins' and
    what it synced for the account it is signed in with."""
    acct = claude_account(config)
    return {s["id"].casefold() for s in _copies(project)
            if s["origin"] in CLAUDE_NATIVE or (acct and s["bucket"].endswith("_" + acct))}


def short(desc, limit=DESC_MAX):
    if len(desc) <= limit:
        return desc
    cut = desc[:limit].rsplit(" ", 1)[0].rstrip(",;:.-(")
    return cut + "..."


def catalog(ai="", project=None, config=None):
    """The skills part of an agent's standing instructions: how to use rbx_skill and one line per skill. For Claude,
    only the skills Claude Code does not load by itself (config: its config folder for a second account). Empty when
    there is nothing to list."""
    claude = ai == "claude"
    native = claude_native(project, config) if claude else set()
    items = [s for s in skills(project) if s["id"].casefold() not in native]
    if not items:
        return ""
    lines = ["", "", "## Skills", ""]
    if claude:
        lines.append("Besides the skills your Skill tool lists, these skills are installed. They are not in your Skill "
                     "tool: load one with the `rbx_skill` tool (action `load`, name) and follow it as you would a skill.")
    else:
        lines.append("Skills are packaged instructions for particular kinds of work, the same for every AI in Fridae. "
                     "Before you start a task, look down this list. When a skill fits the task, even partly, load it "
                     "with the `rbx_skill` tool (action `load`, name) and follow it. `rbx_skill` is a tool of Fridae's "
                     "tool server; its name may carry the server's prefix, such as `rbxkit_rbx_skill`. A built-in "
                     "skill tool of your own, if you have one, cannot load these: use `rbx_skill`. The skills were "
                     "written for Claude Code: where one says to invoke a skill or use the Skill tool, call `rbx_skill`; "
                     "where it names a tool you do not have (TodoWrite, Task, AskUserQuestion and the like), use your "
                     "nearest equivalent or skip that step.")
    lines.append("")
    lines += ["- %s: %s" % (s["id"], short(s["description"]) or "(no description)") for s in items]
    return "\n".join(lines) + "\n"


def find(name, project=None):
    """The skill called `name`: its id exactly, ignoring case, or the bare skill name when only one has it."""
    want = (name or "").strip().lstrip("/")
    every = skills(project)
    for test in (lambda s: s["id"] == want, lambda s: s["id"].casefold() == want.casefold()):
        hit = [s for s in every if test(s)]
        if hit:
            return hit[0]
    bare = [s for s in every if s["name"].casefold() == want.casefold()]
    if len(bare) == 1:
        return bare[0]
    if len(bare) > 1:
        raise LookupError("Several skills are called '%s': %s. Give the full name." % (want, ", ".join(s["id"] for s in bare)))
    near = [s["id"] for s in every if want.casefold() in s["id"].casefold()][:8]
    raise LookupError("No skill called '%s'.%s Call rbx_skill with action 'list' for every skill." % (
        want, (" Did you mean: %s?" % ", ".join(near)) if near else ""))


def _files(d):
    out = []
    for root, dirs, fs in os.walk(d):
        dirs[:] = sorted(x for x in dirs if not x.startswith(".") and x not in ("__pycache__", "node_modules"))
        for f in sorted(fs):
            if not f.startswith("."):
                out.append(os.path.relpath(os.path.join(root, f), d).replace(os.sep, "/"))
            if len(out) > FILES_MAX:
                return out
    return out


def _fill(text, s):
    text = text.replace("${CLAUDE_SKILL_DIR}", s["dir"])
    if s["plugin_root"]:
        text = text.replace("${CLAUDE_PLUGIN_ROOT}", s["plugin_root"])
    return text


def load(name, project=None):
    s = find(name, project)
    _, body = front(_read(os.path.join(s["dir"], "SKILL.md")))
    files = [f for f in _files(s["dir"]) if f != "SKILL.md"]
    more = " (and more)" if len(files) >= FILES_MAX else ""
    head = ["# Skill %s (%s)" % (s["id"], ORIGIN_LABEL.get(s["origin"], s["origin"])),
            "Base directory: %s" % s["dir"]]
    if s["description"]:
        head.append("When to use: %s" % s["description"])
    if files:
        head.append("Its files%s: %s" % (more, ", ".join(files)))
    tail = ("\n\n---\nPaths in this skill are relative to its base directory. Read its files with rbx_skill action "
            "'read' (name, path) or your own file tools, and run its scripts from a shell by their full path.")
    return "\n".join(head) + "\n\n---\n\n" + _fill(body.strip(), s) + tail


def read_file(name, path, project=None):
    s = find(name, project)
    base = os.path.realpath(s["dir"])
    full = os.path.realpath(os.path.join(base, (path or "").replace("\\", "/").lstrip("/")))
    if os.path.commonpath([base, full]) != base:
        raise LookupError("'%s' is outside the skill's folder." % path)
    if not os.path.isfile(full):
        raise LookupError("No file '%s' in skill %s. Its files: %s" % (path, s["id"], ", ".join(_files(s["dir"])[:40])))
    with open(full, "rb") as f:
        raw = f.read(READ_MAX + 1)
    if b"\0" in raw[:4096]:
        raise LookupError("'%s' is not a text file; use it from a shell by its full path: %s" % (path, full))
    text = raw[:READ_MAX].decode("utf-8", errors="replace")
    cut = "\n\n[cut at %d KB; read the rest from %s]" % (READ_MAX // 1000, full) if len(raw) > READ_MAX else ""
    return "%s/%s\n\n%s%s" % (s["id"], path, _fill(text, s), cut)


def listing(project=None):
    every = skills(project)
    if not every:
        return "No skills are installed."
    lines = ["%d skills (rbx_skill action 'load' with a name):" % len(every)]
    lines += ["- %s [%s]: %s" % (s["id"], ORIGIN_LABEL.get(s["origin"], s["origin"]), short(s["description"], 240))
              for s in every]
    return "\n".join(lines)


# ------------------------------------------------------------------ rbx skills (the app asks for the catalog)
def register(sub):
    p = sub.add_parser("skills", help="the user's skills, for every AI: catalog, list, load, read")
    p.add_argument("action", choices=["catalog", "list", "load", "read"])
    p.add_argument("name", nargs="?", default="")
    p.add_argument("path", nargs="?", default="")
    p.add_argument("--ai", default="", help="catalog: the AI it is for (claude leaves out what Claude Code loads itself)")
    p.add_argument("--project", default="", help="the project folder, for its own skills")
    p.add_argument("--claude-config", default="", help="catalog for claude: the config folder of a second account")
    p.set_defaults(fn=cmd_skills)


def cmd_skills(a):
    project = a.project or None
    try:
        if a.action == "catalog":
            out = catalog(a.ai, project, a.claude_config or None)
        elif a.action == "list":
            out = listing(project)
        elif a.action == "load":
            out = load(a.name, project)
        else:
            out = read_file(a.name, a.path, project)
    except LookupError as e:
        raise SystemExit(str(e))
    print(out, end="" if a.action == "catalog" else "\n")


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(prog="rbxskills")
    register(ap.add_subparsers(dest="cmd", required=True))
    args = ap.parse_args()
    args.fn(args)
