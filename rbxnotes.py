"""The user's notes: what agents learn when the user steers them, kept with the user's own data.

Two folders, same shape (one Markdown file per note, plus `House rules.md`):
  <home>/notes/                for everything   (home = rbxpaths.HOME_DIR, the folder with config.json and work/)
  <game>/.fridae/notes/        for one game
They are searched as rbx_vault shelves `yours` and `game` (indexed fresh each time), and the app appends both House
rules files to every agent's instructions. Nothing here reads or writes outside those two folders.
"""
import os
import re
import tempfile
import time
import unicodedata

import rbxpaths
from rbxpaths import KitError

NOTE_CAP = 1500         # characters in one note
RULE_CAP = 240          # characters in one house rule
RULES_FILE_CAP = 2000   # characters in one House rules.md
RULES_NAME = "House rules.md"
LESSONS_NAME = "studio-lessons.md"  # lessons for every game, from before notes (moved here from the work folder)
TRASH = ".trash"
TITLE_MAX = 80
_RESERVED = {"con", "prn", "aux", "nul"} | {"com%d" % i for i in range(1, 10)} | {"lpt%d" % i for i in range(1, 10)}
_KEPT_NAMES = {RULES_NAME[:-3].casefold(), LESSONS_NAME[:-3].casefold()}


def home_notes():
    return os.path.join(rbxpaths.HOME_DIR, "notes")


def game_notes(project):
    return os.path.join(project, ".fridae", "notes")


def project_dir():
    p = (os.environ.get("RBXKIT_PROJECT") or "").strip()
    return p if p and os.path.isdir(p) else None


def folder(scope, project):
    """The notes folder for a scope ('game' needs a project)."""
    if scope == "all":
        return home_notes()
    if not project:
        raise KitError("This chat has no game project, so there are no game notes. Use scope 'all'.")
    return game_notes(project)


def scope_arg(a, project, default=True):
    scope = str(a.get("scope") or "").strip().lower()
    if not scope:
        if not default:
            return None
        scope = "game" if project else "all"
    if scope not in ("game", "all"):
        raise KitError("scope must be 'game' (this game) or 'all' (every game).")
    return scope


# ------------------------------------------------------------------ titles and safe paths
def safe_title(title):
    """A title that is also a safe file name on every system: spaces and accents are fine; path separators, control
    characters and the characters Windows refuses are not; never a Windows device name or one of the kept files."""
    t = unicodedata.normalize("NFC", str(title or ""))
    t = "".join(" " if (unicodedata.category(c)[0] == "C" or c in '/\\:*?"<>|') else c for c in t)
    t = " ".join(t.split()).strip(" .")
    if len(t) > TITLE_MAX:
        t = t[:TITLE_MAX].rsplit(" ", 1)[0].strip(" .") or t[:TITLE_MAX].strip(" .")
    if not t:
        t = "Note"
    stem = t.split(".")[0].strip().casefold()
    if stem in _RESERVED or t.casefold() in _KEPT_NAMES:
        t += " note"
    return t


def title_from(text, words=8):
    w = " ".join(text.split()).split(" ")
    return " ".join(w[:words]) + ("…" if len(w) > words else "")


def _inside(root, path):
    """True when path is root or inside it (after resolving links)."""
    r, p = os.path.realpath(root), os.path.realpath(path)
    try:
        return os.path.commonpath([r, p]) == r
    except ValueError:
        return False


def _note_path(root, name):
    path = os.path.join(root, name)
    if not _inside(root, path) or os.path.dirname(os.path.realpath(path)) != os.path.realpath(root):
        raise KitError("That title cannot be used as a note name.")
    return path


def _atomic_write(path, text):
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".md", dir=d)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def _reserve(root, title):
    """Claim a free file name for a title ('Title.md', then 'Title 2.md', ...) without racing another writer."""
    os.makedirs(root, exist_ok=True)
    n = 1
    while True:
        t = title if n == 1 else "%s %d" % (title, n)
        path = _note_path(root, t + ".md")
        try:
            os.close(os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY))
            return t, path
        except FileExistsError:
            n += 1


# ------------------------------------------------------------------ reading
def _front(text):
    m = re.match(r"^---\r?\n(.*?)\r?\n---\r?\n?", text, re.S)
    fm = {}
    if m:
        for line in m.group(1).splitlines():
            k, sep, v = line.partition(":")
            if sep:
                fm[k.strip()] = v.strip().strip('"')
    return fm


def note_files(root):
    """[(title, path)] of the notes in a folder (not House rules, not the trash, not hidden or temporary files)."""
    if not os.path.isdir(root):
        return []
    out = []
    for f in sorted(os.listdir(root)):
        p = os.path.join(root, f)
        if f.endswith(".md") and not f.startswith(".") and f != RULES_NAME and os.path.isfile(p):
            out.append((f[:-3], p))
    return out


def shelf_files(root):
    """Everything a shelf indexes from a notes folder: the notes and House rules.md."""
    files = note_files(root)
    rules = os.path.join(root, RULES_NAME)
    if os.path.isfile(rules):
        files.append((RULES_NAME[:-3], rules))
    return files


def rules_text(root):
    try:
        with open(os.path.join(root, RULES_NAME), encoding="utf-8") as f:
            return f.read()
    except (OSError, UnicodeDecodeError):
        return ""


def rule_lines(text):
    return [l for l in text.splitlines() if l.lstrip().startswith(("- ", "* "))]


def migrate_lessons():
    """Move the old work-folder studio-lessons.md into the notes folder once (both are still read if this fails)."""
    old = os.path.join(rbxpaths.WORK, LESSONS_NAME)
    new = os.path.join(home_notes(), LESSONS_NAME)
    if os.path.isfile(old) and not os.path.exists(new):
        try:
            os.makedirs(home_notes(), exist_ok=True)
            os.replace(old, new)
        except OSError:
            pass


def yours_files():
    """[(title, path)] for the yours shelf: the for-everything notes, House rules and lessons (old place too)."""
    migrate_lessons()
    files = shelf_files(home_notes())
    old = os.path.join(rbxpaths.WORK, LESSONS_NAME)
    if os.path.isfile(old) and not any(t == LESSONS_NAME[:-3] for t, _ in files):
        files.append((LESSONS_NAME[:-3], old))
    return files


# ------------------------------------------------------------------ writing
def _by():
    return " ".join((os.environ.get("RBXKIT_AGENT_NAME") or "agent").split())[:60]


def write_note(text, title=None, scope="all", project=None, by=None, kind="note"):
    """Save one note; returns (title, path)."""
    text = (text or "").strip()
    if not text:
        raise KitError("Give the note in 'text': what you were doing, what the user wants instead and why.")
    if len(text) > NOTE_CAP:
        raise KitError("A note holds at most %d characters (this one has %d). Keep the point and the user's words; "
                       "drop the rest." % (NOTE_CAP, len(text)))
    root = folder(scope, project)
    title, path = _reserve(root, safe_title(title or title_from(text)))
    body = ("---\ntitle: %s\nwhen: %s\nscope: %s\nby: %s\nkind: %s\n---\n\n# %s\n\n%s\n" % (
        title, time.strftime("%Y-%m-%d %H:%M"), scope, by or _by(), kind, title, text))
    _atomic_write(path, body)
    return title, path


def add_rule(text, scope="all", project=None):
    rule = " ".join((text or "").split())
    if not rule:
        raise KitError("Give the rule in 'text', as one short sentence.")
    if len(rule) > RULE_CAP:
        raise KitError("A house rule holds at most %d characters (this one has %d). Say it shorter; put the reasons "
                       "in a note." % (RULE_CAP, len(rule)))
    root = folder(scope, project)
    path = _note_path(root, RULES_NAME)
    old = rules_text(root).rstrip("\n")
    new = (old + "\n" if old else "# House rules\n\n") + "- " + rule + "\n"
    if len(new) > RULES_FILE_CAP:
        raise KitError("The %s house rules are full (%d characters at most). Merge or replace an existing rule with "
                       "action 'rule_edit' (old: a fragment of the rule, text: the new wording, or empty to remove)." % (
                           "game's" if scope == "game" else "for-everything", RULES_FILE_CAP))
    _atomic_write(path, new)
    return path


def rule_edit(old, new, scope, project=None):
    old = " ".join((old or "").split()).casefold()
    new = " ".join((new or "").split())
    if not old:
        raise KitError("rule_edit needs 'old': a few words of the rule to change.")
    if len(new) > RULE_CAP:
        raise KitError("A house rule holds at most %d characters." % RULE_CAP)
    root = folder(scope, project)
    path = _note_path(root, RULES_NAME)
    text = rules_text(root)
    lines = text.splitlines()
    hits = [i for i, l in enumerate(lines) if l.lstrip().startswith(("- ", "* ")) and old in " ".join(l.split()).casefold()]
    if not hits:
        raise KitError("No house rule contains '%s'. The rules:\n%s" % (old, "\n".join(rule_lines(text)) or "(none)"))
    if len(hits) > 1:
        raise KitError("Several house rules contain '%s'; give more of the words:\n%s" % (
            old, "\n".join(lines[i] for i in hits)))
    if new:
        lines[hits[0]] = "- " + new
    else:
        del lines[hits[0]]
    out = "\n".join(lines).rstrip("\n") + "\n"
    if len(out) > RULES_FILE_CAP:
        raise KitError("That would make the house rules longer than %d characters; say it shorter." % RULES_FILE_CAP)
    _atomic_write(path, out)
    return "changed" if new else "removed"


def forget(title, scope, project=None):
    """Move a note into the folder's .trash (never deleted)."""
    want = " ".join((title or "").split()).casefold()
    if not want:
        raise KitError("forget needs the note's 'title' (action 'notes' lists them).")
    root = folder(scope, project)
    files = note_files(root)
    hit = [p for t, p in files if t.casefold() == want] or [p for t, p in files if want in t.casefold()]
    if len(hit) != 1:
        raise KitError(("Several notes match '%s'; give the exact title." if hit else "No note called '%s'.") % title +
                       " Notes: " + ", ".join(t for t, _ in files[:30]))
    src = hit[0]
    trash = os.path.join(root, TRASH)
    os.makedirs(trash, exist_ok=True)
    base = os.path.basename(src)[:-3]
    dest, n = os.path.join(trash, base + ".md"), 2
    while os.path.exists(dest):
        dest, n = os.path.join(trash, "%s %d.md" % (base, n)), n + 1
    if not (_inside(root, src) and _inside(root, dest)):
        raise KitError("That note is not in the notes folder.")
    os.replace(src, dest)
    return base


def list_notes(scope=None, project=None):
    """[(when, title, scope)] newest first."""
    scopes = [scope] if scope else (["game", "all"] if project else ["all"])
    out = []
    for sc in scopes:
        for t, p in note_files(folder(sc, project)):
            try:
                with open(p, encoding="utf-8") as f:
                    when = _front(f.read(600)).get("when", "")
            except (OSError, UnicodeDecodeError):
                when = ""
            if not when:
                when = time.strftime("%Y-%m-%d %H:%M", time.localtime(os.path.getmtime(p)))
            out.append((when, t, sc))
    out.sort(key=lambda x: x[0], reverse=True)
    return out


def counts(project=None):
    """{"all": {"notes", "rules", "folder"}, "game": {...} or None} for the app's Setup page."""
    def one(root):
        return {"notes": len(note_files(root)), "rules": len(rule_lines(rules_text(root))), "folder": root}
    return {"all": one(home_notes()), "game": one(game_notes(project)) if project else None}


# ------------------------------------------------------------------ rbx_vault actions
def run(action, a):
    project = project_dir()
    if action == "note":
        scope = scope_arg(a, project)
        if a.get("rule") is True or str(a.get("rule")).lower() == "true":
            add_rule(a.get("text"), scope, project)
            return ("House rule saved (%s). Every agent gets it from the next turn on." %
                    ("this game" if scope == "game" else "everything"))
        title, _ = write_note(a.get("text"), a.get("title"), scope, project)
        return ("Note saved as '%s' (%s). Search finds it: rbx_vault action 'search' with shelf '%s'." %
                (title, "this game" if scope == "game" else "everything", "game" if scope == "game" else "yours"))
    if action == "notes":
        rows = list_notes(scope_arg(a, project, default=False), project)
        if not rows:
            return "No notes yet."
        return "Notes, newest first:\n" + "\n".join("- %s  (%s, %s)" % (t, sc, w) for w, t, sc in rows[:60])
    if action == "forget":
        scope = scope_arg(a, project)
        name = forget(a.get("title"), scope, project)
        return "Moved '%s' to the notes folder's .trash (not deleted)." % name
    if action == "rule_edit":
        scope = scope_arg(a, project)
        done = rule_edit(a.get("old"), a.get("text") if a.get("text") is not None else a.get("new"), scope, project)
        return "House rule %s." % done
    raise KitError("Unknown notes action: %s." % action)


def lesson(text, scope, project):
    """rbx_stage lesson: a note like any other (the Lead's lessons are notes)."""
    text = " ".join((text or "").split())
    if not text:
        raise KitError("Give the lesson as one short sentence in 'text'.")
    return write_note(text, "Lesson - " + title_from(text, 6), scope, project, kind="lesson")
