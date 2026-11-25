"""Skills for every AI in Fridae: one library of skills, served to Claude, Gemini, Cursor, OpenCode and Codex.

A skill is a folder holding SKILL.md (front matter with `name`, `description` and the other skill fields, then
instructions) and any files it uses. The app's tool server serves them all (`rbx_skill`), and every agent's standing
instructions carry a short catalog of them (`catalog`) so it knows when one applies.

The library (`library`), one skill per id, a later root taking an earlier one's id:
  1. the app's own skills (owner app; no other skill can take their names) and the skills it ships: KIT_ROOTS
  2. the user's library:   user_dir()/<skill>
  3. the project's:        <project>/.agents/skills, then <project>/.claude/skills (for that project only)

Skills in other places are import sources, not a second library (`import_external`, run by `catalog` and `index` at
most once a minute). Each is copied into the user's library once, with its import link in <skill>/.fridae-source.json:
  - Claude Code's own:    ~/.claude/skills/<skill>, and the skills of the enabled plugins in installed_plugins.json
  - synced by Claude:     ~/.claude/plugins/synced/<account>/<plugin>/skills/<skill> and
                          ~/.claude/skills/synced/<account>/<skill> (every account's; the newest copy of a skill)
  - other tools' folders: ~/.agents/skills, ~/.codex/skills, ~/.cursor/skills, ~/.config/opencode/skill(s),
                          ~/.gemini/skills
A plugin's skill is imported under its skill name (`superpowers:brainstorming` becomes `brainstorming`; `find` still
takes the plugin:skill form). An unedited copy follows its source; an edited one is never overwritten. A newcomer
that overlaps a library skill (`similar`) waits in <user_dir>/.incoming for the user's decision (`pending`, `decide`).
The APP_ONLY skills are never imported or listed. Those folders are only ever read.
"""
import contextlib
import hashlib
import json
import os
import re
import shutil
import time
import uuid

import rbxpaths

KIT = rbxpaths.KIT
HOME = os.path.expanduser("~")
DESC_MAX = 170       # characters of a description in the catalog
READ_MAX = 100_000   # bytes of a file handed out by `read`
FILES_MAX = 80       # files listed by `load`

# the skills the app ships: (folder under KIT, owner of a skill that does not say)
KIT_ROOTS = [("vault/Fridae/Guides/Core", "shipped"), ("vault/Fridae/Guides/Roblox subjects", "shipped"),
             ("vault/Fridae/Guides/LihFolk", "shipped")]
OWNERS = ("app", "shipped", "community", "user")
APPLIES = ("game", "plain", "both")

# Skills the Claude desktop app syncs that only work with the desktop app's own tools (its browser pane, Chrome
# extension, computer use, docs and Google connectors, artifacts, memory). No agent in Fridae has those tools.
APP_ONLY = {"built-in-browser", "chrome-browser", "computer-use", "docs", "google-workspace", "morning", "import-memory"}

ORIGIN_LABEL = {"app": "built into Fridae", "shipped": "shipped with Fridae", "user": "your library",
                "project": "this project", "claude": "Claude Code", "claude-plugin": "a Claude Code plugin",
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
    """(front matter as {key: value}: simple and folded keys are text; `key:` followed by `- item` lines, or
    `[a, b]`, is a list; the text after it)."""
    m = re.match(r"^﻿?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(\r?\n|$)", text, re.S)
    if not m:
        return {}, text
    fm, key, block = {}, None, False
    for line in m.group(1).splitlines():
        mm = re.match(r"^([A-Za-z_][\w.-]*):[ \t]*(.*)$", line)
        item = re.match(r"^[ \t]*-[ \t]+(.*)$", line)
        if mm:
            key, val = mm.group(1), mm.group(2).strip()
            block = val in (">", "|", ">-", "|-", ">+", "|+")
            if block:
                fm[key] = ""
            elif len(val) > 1 and val[0] == "[" and val[-1] == "]":
                fm[key] = [_unquote(x.strip()) for x in val[1:-1].split(",") if x.strip()]
            else:
                fm[key] = _unquote(val)
        elif key and not block and item and (fm[key] == "" or isinstance(fm[key], list)):
            fm[key] = (fm[key] or []) + [_unquote(item.group(1).strip())]
        elif key and line[:1] in (" ", "\t") and isinstance(fm[key], str):
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
            name = _fm_name(fm, d)
            if origin == "synced" and name in APP_ONLY:
                continue
            yield {"id": "%s:%s" % (plugin, name) if plugin else name, "name": name,
                   "description": " ".join((fm.get("description") or "").split()), "dir": d, "origin": origin,
                   "plugin": plugin, "plugin_root": plugin_root, "bucket": bucket}


def user_dir():
    """The user's own skill library."""
    return os.path.join(rbxpaths.HOME_DIR, "skills")


def _list(val):
    if isinstance(val, list):
        return [str(x).strip() for x in val if str(x).strip()]
    return [val.strip()] if isinstance(val, str) and val.strip() else []


# a name YAML would not read as plain text (an unclosed list or quote, an anchor, a tag...)
_BAD_NAME = re.compile(r"""^[\[\]{}"'&*!|>%@`#,]""")


def _fm_name(fm, folder):
    """The front matter's name, else (missing, empty, a list or unparseable) the folder's name."""
    name = fm.get("name")
    if isinstance(name, str) and name.strip() and not _BAD_NAME.match(name.strip()):
        return name.strip()
    return os.path.basename(folder.rstrip("/\\"))


def meta(fm, folder, default_owner):
    """Every skill field, filled: what the front matter says, else the default. Never raises on odd values."""
    name = _fm_name(fm, folder)
    desc = fm.get("description")
    owner, applies = fm.get("owner"), fm.get("applies")
    text = lambda k: fm[k].strip() if isinstance(fm.get(k), str) else ""
    return {"name": name, "description": " ".join(desc.split()) if isinstance(desc, str) else "",
            "examples": _list(fm.get("examples")), "applies": applies if applies in APPLIES else "both",
            "core": str(fm.get("core")).strip().lower() in ("true", "yes", "1"),
            "owner": owner if owner in OWNERS else default_owner, "source": text("source"), "licence": text("licence"),
            "supersedes": _list(fm.get("supersedes")), "updated": text("updated")}


def _entry(d, origin, default_owner, kit=False):
    try:
        fm, _ = front(_read(os.path.join(d, "SKILL.md"), 8000))
    except (OSError, ValueError):
        fm = {}
    m = meta(fm, d, default_owner)
    if m["owner"] == "app" and not kit:
        m["owner"] = default_owner  # only the app's own skills are app-owned
    m.update(id=m["name"], dir=d, origin=origin, plugin="", plugin_root="", bucket="")
    if origin == "user":  # an imported plugin skill keeps its plugin's folder for ${CLAUDE_PLUGIN_ROOT}
        m["plugin_root"] = str((_record(d) or {}).get("plugin_root") or "")
    return m


def kit_skills():
    """The skills the app ships (origin app or shipped), whatever the user's and the project's libraries hold."""
    kit = []
    for rel, owner in KIT_ROOTS:
        for d in _skill_dirs(os.path.join(KIT, *rel.split("/"))):
            e = _entry(d, "shipped", owner, kit=True)
            if e["owner"] == "app":
                e["origin"] = "app"
            kit.append(e)
    return kit


def library(project=None):
    """[{id, name, description, examples, applies, core, owner, source, licence, supersedes, updated, dir, origin}]:
    the app's skills, those it ships, the user's, then the project's. One per name; a later one takes an earlier
    name, except an app skill's, which nothing can take. A shipped skill a user skill supersedes is left out, and so
    are the APP_ONLY names and an update waiting for a decision (<name> (update))."""
    kit = kit_skills()
    roots = [(os.path.join(project, sub, "skills"), "project") for sub in (".claude", ".agents")] if project else []
    found = sorted(kit, key=lambda e: e["origin"] != "app") + \
        [_entry(d, "user", "user") for d in _skill_dirs(user_dir()) if not d.endswith(UPDATE_SUFFIX)] + \
        [_entry(d, o, "user") for r, o in roots[::-1] for d in _skill_dirs(r)]  # .agents, then .claude: .claude wins
    out = {}
    for e in found:
        key = e["id"].casefold()
        if e["origin"] in ("user", "project") and key in APP_ONLY:
            continue
        if key not in out or out[key]["owner"] != "app":
            out[key] = e
    gone = {n.casefold() for e in out.values() if e["origin"] == "user" for n in e["supersedes"]}
    return [e for e in out.values() if not (e["origin"] == "shipped" and e["id"].casefold() in gone)]


def summary(s):
    """The one entry a skill has on the skills search shelf: "<name>\n<description>\nFor: <examples joined by '; '>"
    (no For line when it has no examples). s: a library entry, or its meta fields."""
    lines = [s["name"], s["description"]]
    if s.get("examples"):
        lines.append("For: " + "; ".join(s["examples"]))
    return "\n".join(lines)


WHY_MAX = 60  # characters of "why it matched"
SEARCH_FIELDS = {"title": 2.0, "desc": 1.5, "examples": 3.0}


def _fits(s, kind):
    return kind not in ("game", "plain") or s["applies"] in ("both", kind)


def _cut(text, n=WHY_MAX):
    text = " ".join(text.split())
    return text if len(text) <= n else text[:n - 3].rstrip() + "..."


def _why(s, terms):
    """The example (else the description) that shares most words with the query, at most 60 characters."""
    import rbxvault
    best, hit = 0, ""
    for text in list(s["examples"]) + [s["description"]]:
        n = sum(1 for t in set(rbxvault.ptokens(text)) if t in terms)
        if n > best:
            best, hit = n, text
    return _cut(hit or s["description"])


def keyword_search(q, kind="", k=5, project=None):
    """[{id, description, why, score}] best first: BM25F over each skill's name, description and examples (the same
    words as its summary). score is the share of the query's words the skill holds (0 to 1); only skills holding at
    least one of them are returned."""
    import collections

    import rbxvault
    terms = rbxvault.pquery_terms(q)
    pool = [s for s in library(project) if _fits(s, kind)]
    if not terms or not pool:
        return []
    docs = []
    for s in pool:
        f = {"title": rbxvault.ptokens(s["id"].replace("-", " ").replace(":", " ") + " " + s["name"]),
             "desc": rbxvault.ptokens(s["description"]), "examples": rbxvault.ptokens(" ".join(s["examples"]))}
        tf = {key: collections.Counter(v) for key, v in f.items()}
        docs.append({"tf": tf, "len": {key: sum(c.values()) for key, c in tf.items()}})
    total = sum(terms.values()) or 1
    out = []
    for score, s, d in zip(rbxvault.bm25f(terms, docs, SEARCH_FIELDS), pool, docs):
        if score <= 0:
            continue
        cover = sum(w for t, w in terms.items() if any(d["tf"][f].get(t) for f in SEARCH_FIELDS)) / total
        out.append((score, {"id": s["id"], "description": s["description"], "why": _why(s, terms),
                            "score": round(min(cover, 1.0), 3)}))
    out.sort(key=lambda x: -x[0])
    return [h for _, h in out[:k]]


def search(q, kind="", k=5, project=None):
    """[{id, description, why, score}] the skills that fit q, best first: the knowledge index's skills shelf when the
    app runs it, else the keyword search over the skills' summaries. kind: "game" or "plain" drops the skills that
    apply only to the other. The score is the index's own (higher is better) or, for the keyword search, the share of
    the query's words the skill holds."""
    import rbxvault
    q = (q or "").strip()
    if not q:
        return []
    kind = kind if kind in ("game", "plain") else ""
    end = rbxvault.rag_endpoint()
    if end:
        try:
            hits = rbxvault._rag_hits(end, q, ["skills"], max(k * 3, 10))
            lib = {s["id"].casefold(): s for s in library(project)}
            terms = rbxvault.pquery_terms(q)
            out, seen = [], set()
            for h in hits or []:
                s = lib.get(str(h.get("title") or "").casefold()) if isinstance(h, dict) and h.get("shelf") == "skills" else None
                if s is None or s["id"] in seen or not _fits(s, kind):
                    continue
                seen.add(s["id"])
                out.append({"id": s["id"], "description": s["description"], "why": _why(s, terms),
                            "score": round(float(h.get("score") or 0), 4)})
            if out:
                return out[:k]
        except Exception as e:  # never an error from here: the keyword search answers
            rbxvault._close_quietly(e)
    return keyword_search(q, kind, k, project)


def pointer(h):
    """One search result as a line an agent can act on."""
    return "- %s: %s (load with rbx_skill load %s)" % (h["id"], short(h["description"]), h["id"])


def skills(project=None):
    """Every skill in the library, for the project."""
    return library(project)


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
    only the skills Claude Code does not load by itself (native_ids; config: its config folder for a second account).
    Empty when there is nothing to list. Imports other tools' skills first (at most once a minute)."""
    maybe_import()
    claude = ai == "claude"
    native = native_ids(project, config) if claude else set()
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
    if ":" in want and want.rsplit(":", 1)[1].strip():  # Claude Code's plugin:skill name for an imported skill
        return find(want.rsplit(":", 1)[1], project)
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
    if s.get("plugin_root"):
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


# ------------------------------------------------------------------ import, the overlap check, the decision store
OVERLAP = 0.55          # similar() at or above this: a newcomer waits for the user's decision (Task 9 sets it)
IMPORT_EVERY = 60       # seconds: catalog() and index() import at most this often
LOCK_WAIT, LOCK_STALE = 5.0, 120.0
SOURCE_FILE = ".fridae-source.json"   # in an imported copy: its import link
DECISIONS = ".decisions.json"
INCOMING, REPLACED = ".incoming", ".replaced"
STAMP = ".imported"                   # touched after each import
UPDATE_SUFFIX = " (update)"           # <name> (update): a source's new version beside the user's edited copy
CHOICES = ("keep", "replace", "merge", "keep_both")
APP_SKILL = "That is one of Fridae's own skills."
SKIP_DIRS = ("__pycache__", "node_modules")
TEXT_EXT = (".md", ".lua", ".txt")    # the files install.py fills placeholders in


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def _tree(d):
    """[(relative path, full path)] of a skill folder's files, sorted, leaving out dot-files and dot-folders."""
    out = []
    for root, dirs, files in os.walk(d):
        dirs[:] = [x for x in dirs if not x.startswith(".") and x not in SKIP_DIRS]
        for f in files:
            if not f.startswith("."):
                full = os.path.join(root, f)
                out.append((os.path.relpath(full, d).replace(os.sep, "/"), full))
    return sorted(out)


def _digest(items):
    """sha256 over (relative path, bytes) pairs, in the order given."""
    h = hashlib.sha256()
    for rel, data in items:
        h.update(rel.encode("utf-8") + b"\0" + hashlib.sha256(data).digest())
    return h.hexdigest()


def _bytes(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return b""


def folder_hash(d):
    """sha256 over a skill folder's sorted relative paths and contents, ignoring .DS_Store and other dot-files (the
    import link among them), __pycache__ and node_modules."""
    return _digest((rel, _bytes(full)) for rel, full in _tree(d))


def _ignore(_dir, names):
    return [n for n in names if n.startswith(".") or n in SKIP_DIRS]


def _read_json(path, default):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError, UnicodeDecodeError):
        return default
    return data if isinstance(data, type(default)) else default


def _write_text(path, text):
    """Write through a temporary file and os.replace, so a reader never sees half a file."""
    tmp = "%s.%s.tmp" % (path, uuid.uuid4().hex[:8])
    with open(tmp, "w", encoding="utf-8", newline="") as f:
        f.write(text)
    os.replace(tmp, path)


def _write_json(path, data):
    _write_text(path, json.dumps(data, ensure_ascii=False, indent=1) + "\n")


def _record(d):
    """A library copy's import link: {source, key, hash, upstream, imported, gone, ...}, or None.
    hash: the copy's folder_hash when Fridae last wrote it (the copy is unedited while it still matches);
    upstream: the source's folder_hash Fridae last took or decided on (the source changed when it differs)."""
    rec = _read_json(os.path.join(d, SOURCE_FILE), {})
    return rec or None


@contextlib.contextmanager
def _locked(root):
    """One writer at a time for the library: <root>/.lock created exclusively, retried for about 5 seconds, taken
    over when older than LOCK_STALE (as rbxnotes._locked does for the house rules)."""
    os.makedirs(root, exist_ok=True)
    path = os.path.join(root, ".lock")
    deadline = time.time() + LOCK_WAIT
    while True:
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except (FileExistsError, PermissionError):
            try:
                if time.time() - os.stat(path).st_mtime > LOCK_STALE:
                    os.remove(path)
                    continue
            except OSError:
                pass
            if time.time() > deadline:
                raise rbxpaths.KitError("Another task is changing the skills library. Try again in a moment.")
            time.sleep(0.02)
    os.close(fd)
    try:
        yield
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def _safe(name):
    """A folder name for a skill name."""
    return re.sub(r'[\\/:*?"<>|\x00-\x1f]', "-", name).strip(" .") or "skill"


def _free(parent, name, sep):
    """<parent>/<name>, or <name><sep>2, <name><sep>3... when that is taken."""
    path, n = os.path.join(parent, name), 2
    while os.path.lexists(path):
        path, n = os.path.join(parent, "%s%s%d" % (name, sep, n)), n + 1
    return path


def _set_front(path, updates):
    """Set front matter keys of a SKILL.md (a list as [a, b]), replacing each key's old lines; adds front matter
    when it has none."""
    text = _read(path)
    lines = []
    for k, v in updates.items():
        if isinstance(v, list):
            lines.append("%s: [%s]" % (k, ", ".join(v)))
        else:
            v = str(v)
            lines.append("%s: %s" % (k, json.dumps(v) if _BAD_NAME.match(v) or ": " in v or v != v.strip() else v))
    m = re.match(r"^(﻿?---[ \t]*\r?\n)(.*?)(\r?\n---[ \t]*(?:\r?\n|$))", text, re.S)
    if not m:
        _write_text(path, "---\n%s\n---\n\n%s" % ("\n".join(lines), text))
        return
    keep, skipping = [], False
    for line in m.group(2).splitlines():
        km = re.match(r"^([A-Za-z_][\w.-]*):", line)
        if km:
            skipping = km.group(1) in updates
            if skipping:
                continue
        elif skipping and (line[:1] in (" ", "\t") or re.match(r"^-[ \t]", line)):
            continue  # the replaced key's list items or folded lines
        else:
            skipping = False
        keep.append(line)
    _write_text(path, m.group(1) + "\n".join(keep + lines) + m.group(3) + text[m.end():])


def _place(src, dst, rec=None, patch=None):
    """Copy skill folder src to dst whole: into a temporary folder beside it first, then renamed (an existing dst is
    swapped out only once the copy is complete). patch: front matter keys to set; rec: the import link to write,
    its hash set to the copy's."""
    parent = os.path.dirname(dst)
    os.makedirs(parent, exist_ok=True)
    tmp = os.path.join(parent, ".tmp-" + uuid.uuid4().hex[:10])
    try:
        shutil.copytree(src, tmp, ignore=_ignore)
        if patch:
            _set_front(os.path.join(tmp, "SKILL.md"), patch)
        if rec is not None:
            rec["hash"] = folder_hash(tmp)
            _write_json(os.path.join(tmp, SOURCE_FILE), rec)
        if os.path.lexists(dst):
            old = os.path.join(parent, ".tmp-" + uuid.uuid4().hex[:10])
            os.rename(dst, old)
            os.rename(tmp, dst)
            shutil.rmtree(old, ignore_errors=True)
        else:
            os.rename(tmp, dst)
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise


def _key(c):
    """The stable identity of an external skill (one of _copies): a plugin's skill by plugin and name (its install
    folder changes with each version), a synced skill by name (the newest account's copy), any other by its folder."""
    if c["plugin"]:
        return "plugin:" + c["id"]
    if c["origin"] == "synced":
        return "synced:" + c["name"]
    return "path:" + os.path.normpath(c["dir"])


def _installed_copies():
    """{the folder install.py gives a core guide in ~/.claude/skills: (the guide's id, {folder_hash of exactly what
    install.py writes there})}. Such a copy is Fridae's own guide, not one of the user's skills (unless edited)."""
    try:
        import install
        built = install.claude_skills(os.path.join(KIT, "vault", "Fridae", "Guides", "Core"))
    except Exception:
        return {}
    out = {}
    for name, files in built.items():
        guide = os.path.basename(os.path.dirname(files["SKILL.md"]))
        hashes = set()
        for kit in {KIT, install.KIT}:
            items = []
            for rel, src in files.items():
                if any(p.startswith(".") or p in SKIP_DIRS for p in rel.split("/")):
                    continue
                try:
                    if src.lower().endswith(TEXT_EXT):
                        with open(src, encoding="utf-8") as f:
                            text = f.read()
                        text = text.replace("{{RBXKIT}}", kit.replace("\\", "/")).replace("{{RBXKIT_WIN}}", kit)
                        data = install.built_text(src, text).replace("\n", os.linesep).encode("utf-8")
                    else:
                        with open(src, "rb") as f:
                            data = f.read()
                except (OSError, UnicodeDecodeError):
                    break
                items.append((rel, data))
            else:
                hashes.add(_digest(sorted(items)))
        out[name] = (guide, hashes)
    return out


INSTALLED_FILE = ".fridae-installed.json"  # install.py's marker in each guide it writes: {"skill", "hash"}
OLD_COPY_MIN = 0.9   # an unmarked copy under a guide's name at least this like the guide is an older install of it


def _unfill(text, kit_text):
    """text with the kit folder install.py filled in put back as {{RBXKIT}}: the folder is whatever stands before the
    path that follows a placeholder in the kit's text (e.g. <folder>/rbx.py)."""
    found = set()
    for m in re.finditer(r"\{\{RBXKIT(?:_WIN)?\}\}([^\s`'\"()<>\[\]]+)", kit_text):
        for mm in re.finditer(r"([^\s`'\"()<>\[\]]+?)" + re.escape(m.group(1)), text):
            found.add(mm.group(1))
    for path in sorted(found, key=len, reverse=True):
        text = text.replace(path, "{{RBXKIT}}")
    return text


class _FridaeCopies:
    """Recognises Fridae's own core guides as install.py put them in Claude Code's skills folder, so the import links
    them to the kit's guide instead of copying them in (rulings 9 and 11). A copy is one when:
      (a) it is byte for byte what the current install.py writes;
      (b) its INSTALLED_FILE marker names a kit guide and its folder_hash still equals the hash recorded there;
      (c) it has no marker (installed before markers), its folder or front-matter name is a core guide's id or its
          Claude Code name, and its text (kit folder put back as the placeholder) is OLD_COPY_MIN like the guide's.
    A marked copy whose hash no longer matches was edited: it is the user's skill."""

    def __init__(self):
        self.installed = _installed_copies()
        self.guides = {os.path.basename(d): d for d in _skill_dirs(os.path.join(KIT, "vault", "Fridae", "Guides", "Core"))}
        try:
            import install
            folders, fm_names = install.CLAUDE_SKILL_NAMES, install.CLAUDE_FM_NAMES
        except Exception:
            folders, fm_names = {}, {}
        self.names = {}
        for g in self.guides:
            for n in (g, folders.get(g, g), fm_names.get(g, g)):
                self.names.setdefault(n.casefold(), g)

    def guide(self, d, h):
        """The kit guide's id when skill folder d (folder_hash h) is Fridae's installed copy of it, else None."""
        mark = _read_json(os.path.join(d, INSTALLED_FILE), {})
        if mark.get("skill") in self.guides and mark.get("hash") == h:
            return mark["skill"]
        hit = self.installed.get(os.path.basename(d))
        if hit and h in hit[1]:
            return hit[0]
        if mark:
            return None  # Fridae wrote it, and it was edited since
        try:
            fm, body = front(_read(os.path.join(d, "SKILL.md")))
        except OSError:
            return None
        g = self.names.get(os.path.basename(d).casefold()) or self.names.get(_fm_name(fm, d).casefold())
        if not g:
            return None
        try:
            kfm, kbody = front(_read(os.path.join(self.guides[g], "SKILL.md")))
        except OSError:
            return None
        kit_text = ("%s\n%s" % (kfm.get("description") or "", kbody)).replace("{{RBXKIT_WIN}}", "{{RBXKIT}}")
        text = _unfill("%s\n%s" % (fm.get("description") or "", body), kit_text)
        return g if _cosine(text, kit_text) >= OLD_COPY_MIN else None


# ---- the overlap check
_SERVICE = {}   # one import's knowledge index answers: {"down": True} after a failure, {summary text: hits}


def _idf(texts):
    """Word weights over the library's summaries (smoothed idf): a word most skills share counts for little."""
    import collections
    import math

    import rbxvault
    df = collections.Counter()
    for t in texts:
        df.update(set(rbxvault.ptokens(t)))
    n = len(texts)
    return lambda w: math.log((n + 1) / (df.get(w, 0) + 1)) + 1


def _cosine(a, b, idf=None):
    import collections
    import math

    import rbxvault
    wa, wb = collections.Counter(rbxvault.ptokens(a)), collections.Counter(rbxvault.ptokens(b))
    w = idf or (lambda _t: 1.0)
    dot = sum(c * wb[t] * w(t) ** 2 for t, c in wa.items() if t in wb)
    na = math.sqrt(sum((c * w(t)) ** 2 for t, c in wa.items()))
    nb = math.sqrt(sum((c * w(t)) ** 2 for t, c in wb.items()))
    return dot / (na * nb) if na and nb else 0.0


def _name_key(s):
    name = str(s.get("name") or s.get("id") or "").rsplit(":", 1)[-1]
    return re.sub(r"[-_\s]+", "-", name.strip().casefold())


def _service_score(a, b):
    """The knowledge index's score for a among the skills like b (0 to 1), 0.0 when a is not among them, None when the
    index is not running or fails (then once per import)."""
    import rbxvault
    if _SERVICE.get("down"):
        return None
    end = rbxvault.rag_endpoint()
    if not end:
        return None
    q = summary(b)
    hits = _SERVICE.get(q)
    if hits is None:
        try:
            hits = rbxvault._rag_hits(end, q, ["skills"], 10) or []
        except Exception as e:
            rbxvault._close_quietly(e)
            _SERVICE["down"] = True
            return None
        _SERVICE[q] = hits
    want = str(a.get("id") or a.get("name") or "").casefold()
    for h in hits:
        if isinstance(h, dict) and h.get("shelf") == "skills" and str(h.get("title") or "").casefold() == want:
            try:
                sc = float(h.get("score") or 0)
            except (TypeError, ValueError):
                return 0.0
            return sc if 0.0 <= sc <= 1.0 else 0.0  # another scale (fused ranks, reranker logits) says nothing here
    return 0.0


def similar(a, b, idf=None):
    """How much two skills (library entries or their meta fields) overlap, 0 to 1: the larger of their names' likeness
    (the same name is 1; else the share of name words they share) and the keyword cosine of their skill_docs text
    (summary), or the knowledge index's score for the pair when it runs and scores higher. idf: word weights
    (_idf) over the library."""
    import rbxvault
    if _name_key(a) == _name_key(b):
        return 1.0
    wa = set(rbxvault.ptokens(_name_key(a).replace("-", " ")))
    wb = set(rbxvault.ptokens(_name_key(b).replace("-", " ")))
    name = len(wa & wb) / len(wa | wb) if wa and wb else 0.0
    text = _cosine(summary(a), summary(b), idf)
    service = _service_score(a, b)
    return round(min(1.0, max(name, text, service or 0.0)), 4)


# ---- the decision store: <user_dir>/.decisions.json
def _decisions():
    return [d for d in _read_json(os.path.join(user_dir(), DECISIONS), []) if isinstance(d, dict)]


def _save_decisions(decs):
    _write_json(os.path.join(user_dir(), DECISIONS), decs)


def pending():
    """The open decisions: [{id, kind, a, b, a_dir, b_dir, made, recommendation, state, reason, source}]. a is the
    skill in use, b the newcomer waiting (in .incoming, or beside a as "<a> (update)")."""
    return [d for d in _decisions() if d.get("state") == "open"]


# ---- import
class _Importer:
    def __init__(self, root, out):
        self.root, self.out = root, out
        self.decisions = _decisions()
        self.changed = False
        self.open = {d.get("source_key"): d for d in self.decisions if d.get("state") == "open" and d.get("source_key")}
        self.live = {}       # key -> the library copy linked to it
        for d in _skill_dirs(root):
            rec = None if d.endswith(UPDATE_SUFFIX) else _record(d)
            if rec and rec.get("key"):
                self.live.setdefault(rec["key"], d)
        self.replaced = {}   # key -> the newest import link of a copy the user set aside
        for d in _subdirs(os.path.join(root, REPLACED)):
            rec = _record(d)
            if rec and rec.get("key"):
                old = self.replaced.get(rec["key"])
                if old is None or str(rec.get("imported", "")) >= str(old.get("imported", "")):
                    self.replaced[rec["key"]] = rec
        self.fridae = _FridaeCopies()
        self.lib = library()
        self.idf = _idf([summary(e) for e in self.lib])
        self.heads = {}      # library dir -> its SKILL.md's bytes hash (the identical check reads few folders whole)
        _SERVICE.clear()

    def run(self):
        seen = set()
        for c in _copies(None):
            key = _key(c)
            if key in seen:
                continue
            seen.add(key)
            if c["name"].casefold() in APP_ONLY or os.path.basename(c["dir"]).casefold() in APP_ONLY:
                continue
            try:
                self.one(c, key)
            except (OSError, ValueError, shutil.Error):
                continue  # one unreadable skill never stops the rest
        for key, d in self.live.items():
            rec = _record(d)
            if rec is not None and bool(rec.get("gone")) != (key not in seen):
                rec["gone"] = key not in seen  # the copy stays and is still served
                _write_json(os.path.join(d, SOURCE_FILE), rec)
        if self.changed:
            _save_decisions(self.decisions)

    def link(self, c, key, h):
        return {"source": c["dir"], "key": key, "hash": h, "upstream": h, "imported": _now(), "gone": False,
                "origin": c["origin"], "plugin": c["plugin"], "plugin_root": c["plugin_root"]}

    def ask(self, a, b, a_dir, b_dir, key, src, reason):
        d = {"id": uuid.uuid4().hex[:12], "kind": "skill", "a": a, "b": b, "a_dir": a_dir, "b_dir": b_dir,
             "made": _now(), "recommendation": None, "state": "open", "reason": reason, "source": src,
             "source_key": key}
        self.decisions.append(d)
        self.open[key] = d
        self.changed = True
        self.out["pending"].append(b)

    def identical(self, src, h):
        want = hashlib.sha256(_bytes(os.path.join(src, "SKILL.md"))).hexdigest()
        for e in self.lib:
            if e["dir"] not in self.heads:
                self.heads[e["dir"]] = hashlib.sha256(_bytes(os.path.join(e["dir"], "SKILL.md"))).hexdigest()
            if self.heads[e["dir"]] == want and folder_hash(e["dir"]) == h:
                return e
        return None

    def one(self, c, key):
        src, name = c["dir"], c["name"]
        h = folder_hash(src)
        dec = self.open.get(key)
        if dec:  # waiting for the user: keep the newcomer current, ask nothing more
            bd = dec.get("b_dir") or ""
            rec = _record(bd) if os.path.isdir(bd) else None
            if rec is not None and rec.get("upstream") != h:
                _place(src, bd, dict(rec, upstream=h, source=src))
            return
        d = self.live.get(key)
        if d:
            rec = _record(d) or {}
            if rec.get("upstream") == h:
                return
            if folder_hash(d) == rec.get("hash"):  # unedited: it follows its source
                rec.update(upstream=h, source=src, refreshed=_now(), gone=False, plugin_root=c["plugin_root"])
                _place(src, d, rec, rec.get("patch"))
                self.out["refreshed"].append(name)
            else:  # both changed: the new one waits beside the user's
                ud = _free(self.root, os.path.basename(d) + UPDATE_SUFFIX, " ")
                _place(src, ud, self.link(c, key, h))
                self.ask(_entry(d, "user", "user")["id"], name, d, ud, key, src, "update")
            return
        old = self.replaced.get(key)
        if old and old.get("upstream") == h:
            return  # the user set this one aside
        if self.fridae.guide(src, h):
            self.out["linked"].append(name)  # Fridae's own core guide, as install.py put it in Claude Code's folder
            return
        same = self.identical(src, h)
        if same is not None:
            if same["origin"] == "user" and _record(same["dir"]) is None:
                _write_json(os.path.join(same["dir"], SOURCE_FILE), self.link(c, key, h))
                self.live[key] = same["dir"]
            self.out["linked"].append(name)
            return
        fm, _ = front(_read(os.path.join(src, "SKILL.md"), 8000))
        m = meta(fm, src, "user")
        new = dict(m, id=m["name"])
        best, score = None, 0.0
        for e in self.lib:
            s = similar(e, new, self.idf)
            if s > score:
                best, score = e, s
        if best is not None and score >= OVERLAP:
            inc = _free(os.path.join(self.root, INCOMING), _safe(m["name"]), " ")
            _place(src, inc, self.link(c, key, h))
            self.ask(best["id"], m["name"], best["dir"], inc, key, src, "overlap")
            return
        target = _free(self.root, _safe(m["name"]), "-")
        rec = self.link(c, key, h)
        patch = {"name": os.path.basename(target)} if os.path.basename(target) != m["name"] else None
        if patch:
            rec["patch"] = patch
        _place(src, target, rec, patch)
        self.live[key] = target
        self.lib.append(_entry(target, "user", "user"))
        self.out["added"].append(name)


def import_external():
    """Bring other tools' skills (every external source but the project's, never an APP_ONLY one) into the user's
    library, holding <user_dir>/.lock. For each:
      - linked (its copy's .fridae-source.json): a changed source refreshes an unedited copy; when both changed, the
        new one is saved beside it as "<name> (update)" and a decision added;
      - identical to a library skill, or Fridae's own core guide as install.py put it there (_FridaeCopies):
        linked only;
      - new and overlapping a library skill (similar() >= OVERLAP): a decision, the newcomer kept in .incoming;
      - otherwise copied in.
    A source that disappeared marks its link gone and leaves the copy. Only ever reads the sources.
    Returns {"added": [...], "linked": [...], "refreshed": [...], "pending": [...]} (skill names)."""
    root = user_dir()
    out = {"added": [], "linked": [], "refreshed": [], "pending": []}
    with _locked(root):
        try:
            _Importer(root, out).run()
        finally:
            _touch(os.path.join(root, STAMP))
    return out


def _touch(path):
    try:
        with open(path, "a"):
            pass
        os.utime(path, None)
    except OSError:
        pass


def maybe_import():
    """import_external when the last import is more than IMPORT_EVERY seconds old; never raises."""
    if time.time() - _mtime(os.path.join(user_dir(), STAMP)) < IMPORT_EVERY:
        return None
    try:
        return import_external()
    except Exception:
        _touch(os.path.join(user_dir(), STAMP))  # retried a minute later, not on every call
        return None


# ---- deciding
def _kit_owner(d):
    """(owner, True) for one of the app's own or shipped skills (kit_skills), else ("user", False). Matched by the
    skill's folder, not by being somewhere under KIT: running from source, the user's library is <KIT>/skills."""
    real = os.path.realpath(d)
    for e in kit_skills():
        if os.path.realpath(e["dir"]) == real:
            return e["owner"], True
    return "user", False


def _aside(d, name):
    """Move a losing skill folder to <user_dir>/.replaced/<name>/ (never deleted)."""
    if os.path.isdir(d):
        dst = _free(os.path.join(user_dir(), REPLACED), _safe(name), " ")
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        os.rename(d, dst)


def _mark(d, updates):
    """Set front matter keys on a library skill (supersedes added to what it has). An imported copy remembers them,
    to set again when it is refreshed, and stays unedited if it was."""
    path = os.path.join(d, "SKILL.md")
    fm, _ = front(_read(path))
    if "supersedes" in updates:
        have = _list(fm.get("supersedes"))
        updates = dict(updates, supersedes=have + [n for n in updates["supersedes"] if n not in have])
    rec = _record(d)
    unedited = rec is not None and folder_hash(d) == rec.get("hash")
    _set_front(path, updates)
    if rec is not None:
        rec["patch"] = dict(rec.get("patch") or {}, **updates)
        if unedited:
            rec["hash"] = folder_hash(d)
        _write_json(os.path.join(d, SOURCE_FILE), rec)


def decide(id, choice, merged_text=None):
    """Settle an open decision. choice:
      keep       the skill in use stays; the newcomer goes to .replaced/ and the winner gains supersedes;
      replace    the newcomer takes over; the old one goes to .replaced/ (a shipped one stays in the app, superseded);
      merge      merged_text becomes the winner's SKILL.md (the one in use keeps its name); the old SKILL.md's folder
                 and the newcomer go to .replaced/;
      keep_both  the newcomer joins too, renamed <name>-2 when its name is taken.
    Refuses (ValueError) a choice that would change an owner: app skill. Returns the decision, now state "done"."""
    if choice not in CHOICES:
        raise ValueError("Choose keep, replace, merge or keep_both.")
    root = user_dir()
    with _locked(root):
        decs = _decisions()
        dec = next((d for d in decs if d.get("id") == id and d.get("state") == "open"), None)
        if dec is None:
            raise LookupError("No open decision '%s'." % id)
        if dec.get("kind", "skill") != "skill":
            raise ValueError("That decision is about notes, not skills.")
        a, b, a_dir, b_dir = dec["a"], dec["b"], dec["a_dir"], dec["b_dir"]
        owner, a_kit = _kit_owner(a_dir)
        if owner == "app" and choice in ("replace", "merge"):
            raise ValueError(APP_SKILL)
        if choice == "merge" and not (merged_text or "").strip():
            raise ValueError("A merge needs the merged SKILL.md.")
        if not os.path.isdir(b_dir):
            raise LookupError("The waiting skill's folder is missing: %s" % b_dir)
        update = dec.get("reason") == "update"
        upstream = (_record(b_dir) or {}).get("upstream")
        differ = a.casefold() != b.casefold()
        if choice == "keep":
            _aside(b_dir, b)
            rec = _record(a_dir) if update else None
            if rec is not None:
                rec["upstream"] = upstream  # this version of the source is settled: not asked about again
                _write_json(os.path.join(a_dir, SOURCE_FILE), rec)
            if not a_kit and differ and os.path.isdir(a_dir):
                _mark(a_dir, {"supersedes": [b]})
        elif choice == "replace":
            if not a_kit:
                _aside(a_dir, a)
            target = _free(root, _safe(b), "-")
            os.rename(b_dir, target)
            patch = {"name": os.path.basename(target)} if os.path.basename(target) != b else {}
            if differ:
                patch["supersedes"] = [a]
            if patch:
                _mark(target, patch)
        elif choice == "merge":
            if a_kit:
                target = _free(root, _safe(a), "-")
                shutil.copytree(a_dir, target, ignore=_ignore)
            else:
                target = a_dir
                shutil.copytree(a_dir, _free(os.path.join(root, REPLACED), _safe(a), " "), ignore=_ignore)
            path = os.path.join(target, "SKILL.md")
            _write_text(path, merged_text)
            fm, _ = front(merged_text)
            patch = {} if _fm_name(fm, target) == os.path.basename(target) else {"name": os.path.basename(target)}
            if differ:
                have = _list(fm.get("supersedes"))
                patch["supersedes"] = have + ([b] if b not in have else [])
            if patch:
                _set_front(path, patch)
            rec = _record(target)
            if rec is not None and update:  # merged: an edited copy now, settled for this version of the source
                rec["upstream"] = upstream
                _write_json(os.path.join(target, SOURCE_FILE), rec)
            _aside(b_dir, b)
        else:  # keep_both
            taken = {e["id"].casefold() for e in library()}
            base = n = _safe(b)
            i = 2
            while n.casefold() in taken or os.path.lexists(os.path.join(root, n)):
                n, i = "%s-%d" % (base, i), i + 1
            target = os.path.join(root, n)
            os.rename(b_dir, target)
            if n != b:
                _mark(target, {"name": n})
            if update and os.path.isfile(os.path.join(a_dir, SOURCE_FILE)):
                os.remove(os.path.join(a_dir, SOURCE_FILE))  # the user's edited copy is theirs now; the new one follows the source
        dec.update(state="done", choice=choice, decided=_now())
        _save_decisions(decs)
        return dec


ADVICE = ("keep", "replace", "merge")


def advise(id, rec):
    """Store an AI's recommendation on an open decision, and nothing else: no folder moves, the decision stays open
    for the user. rec: {choice: keep|replace|merge, why, merged (the merged SKILL.md), by (who wrote it)}. An unknown
    choice, or a merge without its text, is stored with no choice (the why still shows); a recommendation to replace
    or merge one of Fridae's own skills is stored as keep. Returns the decision."""
    rec = rec if isinstance(rec, dict) else {}
    text = lambda k: rec[k] if isinstance(rec.get(k), str) else ""
    choice = rec.get("choice") if rec.get("choice") in ADVICE else None
    merged = text("merged") if choice == "merge" else ""
    if choice == "merge" and not merged.strip():
        choice, merged = None, ""
    with _locked(user_dir()):
        decs = _decisions()
        dec = next((d for d in decs if d.get("id") == id and d.get("state") == "open"), None)
        if dec is None:
            raise LookupError("No open decision '%s'." % id)
        if choice in ("replace", "merge") and _kit_owner(dec.get("a_dir") or "")[0] == "app":
            choice, merged = "keep", ""
        dec["recommendation"] = {"choice": choice, "why": text("why").strip(), "merged": merged,
                                 "by": text("by").strip(), "made": _now()}
        _save_decisions(decs)
        return dec


# ---- what Claude Code lists itself, and the index the app reads
INDEX_FIELDS = ("id", "name", "description", "examples", "applies", "core", "owner", "source", "licence",
                "supersedes", "updated", "dir", "origin")


def native_ids(project=None, config=None):
    """The ids (casefolded) of the library skills Claude Code lists by itself for the account in config (its config
    folder; None: the main one): the project's, the unedited imports of a skill it loads (its own folder, its enabled
    plugins, what it synced for that account), and Fridae's core guides as install.py put them in its folder."""
    acct = claude_account(config)
    keys, fridae, guides = set(), None, set()
    for c in _copies(project):
        if c["origin"] in CLAUDE_NATIVE or (acct and c["bucket"].endswith("_" + acct)):
            keys.add(_key(c))
            if c["origin"] == "claude":
                fridae = _FridaeCopies() if fridae is None else fridae
                g = fridae.guide(c["dir"], folder_hash(c["dir"]))
                if g:
                    guides.add(g.casefold())
    out = set()
    for s in library(project):
        sid = s["id"].casefold()
        if s["origin"] == "project" or (s["origin"] in ("app", "shipped") and sid in guides):
            out.add(sid)
        elif s["origin"] == "user":
            rec = _record(s["dir"])
            if rec and rec.get("key") in keys and folder_hash(s["dir"]) == rec.get("hash"):
                out.add(sid)
    return out


def index(project=None, config=None):
    """Every library skill's fields (INDEX_FIELDS) plus claude_native: whether Claude Code lists it itself. Imports
    other tools' skills first (at most once a minute)."""
    maybe_import()
    native = native_ids(project, config)
    return [dict({k: s[k] for k in INDEX_FIELDS}, claude_native=s["id"].casefold() in native) for s in library(project)]


# ------------------------------------------------------------------ rbx skills (the app asks for the catalog)
def register(sub):
    p = sub.add_parser("skills", help="the user's skills, for every AI: catalog, list, load, read, import, pending, "
                                      "decide, index")
    p.add_argument("action", choices=["catalog", "list", "load", "read", "import", "pending", "decide", "index"])
    p.add_argument("name", nargs="?", default="", help="load, read: the skill; decide: the decision's id")
    p.add_argument("path", nargs="?", default="", help="read: the file; decide: keep, replace, merge or keep_both "
                                                       "(advise: only store an AI's recommendation, from --json-file)")
    p.add_argument("--ai", default="", help="catalog: the AI it is for (claude leaves out what Claude Code loads itself)")
    p.add_argument("--project", default="", help="the project folder, for its own skills")
    p.add_argument("--claude-config", default="", help="catalog, index: the Claude config folder of a second account")
    p.add_argument("--json", action="store_true", help="pending, decide: answer in JSON (import and index always do)")
    p.add_argument("--merged-file", default="", help="decide merge: the merged SKILL.md")
    p.add_argument("--json-file", default="", help="decide advise: the recommendation {choice, why, merged, by}")
    p.set_defaults(fn=cmd_skills)


def _decide_cli(a):
    try:
        if a.path == "advise":
            dec = advise(a.name, _read_json(a.json_file, {}) if a.json_file else {})
            return json.dumps({"ok": True, "decision": dec}, ensure_ascii=False) if a.json else \
                "Recommendation stored for %s / %s." % (dec["a"], dec["b"])
        merged = _read(a.merged_file) if a.merged_file else None
        dec = decide(a.name, a.path, merged)
    except (LookupError, ValueError, OSError, rbxpaths.KitError) as e:
        if a.json:
            return json.dumps({"ok": False, "error": str(e)}, ensure_ascii=False)
        raise SystemExit(str(e))
    if a.json:
        return json.dumps({"ok": True, "decision": dec}, ensure_ascii=False)
    return "Decided: %s (%s / %s)." % (dec["choice"], dec["a"], dec["b"])


def cmd_skills(a):
    project = a.project or None
    try:
        if a.action == "catalog":
            out = catalog(a.ai, project, a.claude_config or None)
        elif a.action == "list":
            out = listing(project)
        elif a.action == "load":
            out = load(a.name, project)
        elif a.action == "read":
            out = read_file(a.name, a.path, project)
        elif a.action == "import":
            out = json.dumps(import_external(), ensure_ascii=False)
        elif a.action == "pending":
            items = pending()
            out = json.dumps(items, ensure_ascii=False) if a.json else (
                "\n".join("%s: %s / %s (%s)" % (d["id"], d["a"], d["b"], d.get("reason", "")) for d in items)
                or "No skill decisions are waiting.")
        elif a.action == "index":
            out = json.dumps(index(project, a.claude_config or None), ensure_ascii=False)
        else:
            out = _decide_cli(a)
    except LookupError as e:
        raise SystemExit(str(e))
    print(out, end="" if a.action == "catalog" else "\n")


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(prog="rbxskills")
    register(ap.add_subparsers(dest="cmd", required=True))
    args = ap.parse_args()
    args.fn(args)
