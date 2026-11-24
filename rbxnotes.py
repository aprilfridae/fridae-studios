"""The user's notes: what agents learn when the user steers them, kept with the user's own data.

Two folders, same shape (one Markdown file per note, plus `House rules.md`):
  <home>/notes/                for everything   (home = rbxpaths.HOME_DIR, the folder with config.json and work/)
  <game>/.fridae/notes/        for one game
They are searched as rbx_vault shelves `yours` and `game` (indexed fresh each time), and the app appends both House
rules files to every agent's instructions. Nothing here reads or writes outside those two folders.
"""
import contextlib
import json
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
_RESERVED = ({"con", "prn", "aux", "nul"} | {"com%s" % i for i in "123456789¹²³"} |
             {"lpt%s" % i for i in "123456789¹²³"})
LOCK_WAIT, LOCK_STALE = 5.0, 10.0
SIDECAR = "superseded.json"  # {old title: {"by": new title, "date": YYYY-MM-DD, "shelf": the old note's shelf}} for
# shipped notes (read only); an entry without a shelf (written before it was kept) is a shipped note's
_KEPT_NAMES = {RULES_NAME[:-3].casefold(), LESSONS_NAME[:-3].casefold()}


def home_notes():
    return os.path.join(rbxpaths.HOME_DIR, "notes")


def game_notes(project):
    return os.path.join(project, ".fridae", "notes")


def project_dir():
    p = (os.environ.get("RBXKIT_PROJECT") or "").strip()
    return p if p and os.path.isdir(p) else None


def folder(scope, project):
    """The notes folder for a scope ('game' needs a project). Refused when the folder (or the game's .fridae) is a
    link that leads somewhere else: notes are read and written only where they belong."""
    if scope == "all":
        root, base = home_notes(), rbxpaths.HOME_DIR
    elif not project:
        raise KitError("This chat has no game project, so there are no game notes. Use scope 'all'.")
    else:
        root, base = game_notes(project), project
    if not _where_expected(root, base):
        raise KitError("The notes folder %s leads somewhere else (a link); notes are not read or written there." % root)
    return root


def _where_expected(root, base):
    """True when root, followed through any links, is still base's own sub-folder of the same name."""
    want = os.path.join(os.path.realpath(base), os.path.relpath(root, base))
    return os.path.realpath(root) == os.path.normpath(want)


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
    if stem in _RESERVED:
        t = "Note - " + t  # a Windows device name before the first dot, whatever follows it
    elif t.casefold() in _KEPT_NAMES:
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


@contextlib.contextmanager
def _locked(root):
    """One writer at a time for a notes folder's House rules: <root>/.lock created exclusively, retried for about
    5 seconds, taken over when older than 10 seconds."""
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
                raise KitError("Another agent is changing the house rules. Try again in a moment.")
            time.sleep(0.02)
    os.close(fd)
    try:
        yield
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


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
        if f.endswith(".md") and not f.startswith(".") and f != RULES_NAME and os.path.isfile(p) and not os.path.islink(p):
            out.append((f[:-3], p))
    return out


def shelf_files(root):
    """Everything a shelf indexes from a notes folder: the notes and House rules.md."""
    files = note_files(root)
    rules = os.path.join(root, RULES_NAME)
    if os.path.isfile(rules) and not os.path.islink(rules):
        files.append((RULES_NAME[:-3], rules))
    return files


def rules_text(root):
    if os.path.islink(os.path.join(root, RULES_NAME)):
        return ""
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


# ------------------------------------------------------------------ superseded notes
# A note that replaces others names them (front matter `supersedes`); the old ones are kept and marked
# (`superseded_by`, `superseded_on`), and search ranks them well below (rbxvault.SUPERSEDED_SCALE). The shipped
# notes (vault/Roblox, vault/Fridae) are never written: their marks go to the notes folder's superseded.json.
_SIDE_CACHE = {}  # path -> ((mtime_ns, size), entries)


def sidecar_read(root):
    """{old title: {"by", "date"}} from a notes folder's superseded.json ({} when there is none or it does not read)."""
    path = os.path.join(root, SIDECAR)
    try:
        st = os.stat(path)
    except OSError:
        return {}
    sig = (st.st_mtime_ns, st.st_size)
    hit = _SIDE_CACHE.get(path)
    if hit and hit[0] == sig:
        return hit[1]
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError, UnicodeDecodeError):
        data = {}
    data = {str(k): v for k, v in data.items() if isinstance(v, dict) and str(v.get("by") or "").strip()} \
        if isinstance(data, dict) else {}
    _SIDE_CACHE[path] = (sig, data)
    return data


def sidecars(project=None):
    """{old title casefolded: new title} from the for-everything notes folder's superseded.json and, with a project,
    the game's (which wins). By title only: the upkeep sweep's view (search uses sidecar_marks)."""
    roots = [home_notes()] + ([game_notes(project)] if project else [])
    out = {}
    for root in roots:
        for old, v in sidecar_read(root).items():
            out[" ".join(old.split()).casefold()] = " ".join(str(v["by"]).split())
    return out


def sidecar_marks(project=None):
    """[(old title, its shelf or None for a shipped note named before the shelf was kept, new title, the shelf the
    new note is on first: "yours" or "game")] from the for-everything notes folder's superseded.json and, with a
    project, the game's (later, so it wins)."""
    roots = [(home_notes(), "yours")] + ([(game_notes(project), "game")] if project else [])
    out = []
    for root, where in roots:
        for old, v in sidecar_read(root).items():
            shelf = str(v.get("shelf") or "").strip() or None
            out.append((" ".join(old.split()), shelf, " ".join(str(v["by"]).split()), where))
    return out


def supersedes_arg(v):
    """The titles a 'note' call supersedes: one title, a list, or several separated by ' | '."""
    if v in (None, "", []):
        return []
    vals = v if isinstance(v, list) else str(v).split("|")
    out = []
    for x in vals:
        t = " ".join(str(x).split())
        if t.casefold().endswith(".md"):
            t = t[:-3]
        if t and t.casefold() not in {o.casefold() for o in out}:
            out.append(t)
    return out


def _shipped_titles():
    """{title casefolded: (title, shelf)} of the shipped notes: the LihFolk vault notes, Fridae's guides and team
    files (in that order: the first of a title wins)."""
    import rbxvault
    files = []
    try:
        if os.path.isdir(rbxvault.KB):
            files += [(os.path.basename(p)[:-3], "vault") for p in rbxvault.note_files()]
        files += [(t, "guides") for t, _ in rbxvault.guide_files()]
        files += [(t, "team") for t, _ in rbxvault.team_files(rbxvault.kit_team_dir())]
    except Exception:  # the guide list needs the tool server; a missing part only narrows the lookup
        pass
    out = {}
    for t, shelf in files:
        out.setdefault(t.casefold(), (t, shelf))
    return out


def supersede_targets(olds, new_title, scope, project=None):
    """[(old title, path or None for a shipped note, its shelf)] for the titles a new note supersedes; refuses
    (before anything is written) an unknown title, the note's own title, and, for an expert, a note for every game.
    Looked for in this order: the scope's own notes, the other scope's, the shipped notes."""
    roots = [(scope, folder(scope, project))]
    other = "all" if scope == "game" else ("game" if project else None)
    if other:
        roots.append((other, folder(other, project)))
    shipped = None
    out = []
    for old in olds:
        want = old.casefold()
        if want == new_title.casefold():
            raise KitError("A note cannot supersede itself ('%s'). Name the older note it replaces." % old)
        hit = None
        for sc, root in roots:
            found = [(t, p) for t, p in note_files(root) if t.casefold() == want]
            if found:
                hit = (sc, found[0])
                break
        if hit:
            sc, (t, p) = hit
            if sc == "all" and is_helper():
                raise KitError("'%s' is a note for every game: an expert marks only this game's notes. Tell your "
                               "Lead it is out of date instead." % t)
            out.append((t, p, "game" if sc == "game" else "yours"))
            continue
        shipped = _shipped_titles() if shipped is None else shipped
        if want in shipped:
            out.append((shipped[want][0], None, shipped[want][1]))
            continue
        near = sorted({t for _, root in roots for t, _ in note_files(root) if want in t.casefold()})[:12]
        raise KitError("No note called '%s' to supersede.%s Give the old note's exact title (action 'notes' lists "
                       "them)." % (old, " Did you mean: %s?" % "; ".join(near) if near else ""))
    return out


def _front_set(text, pairs):
    """text with these front matter keys set (each value written as a JSON string), other lines kept."""
    m = re.match(r"^\ufeff?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n|$)", text, re.S)
    lines = m.group(1).splitlines() if m else []
    rest = text[m.end():] if m else text
    for key, val in pairs:
        line = "%s: %s" % (key, json.dumps(val, ensure_ascii=False))
        at = next((i for i, l in enumerate(lines) if re.match(r"^%s[ \t]*:" % re.escape(key), l)), None)
        if at is None:
            lines.append(line)
            continue
        end = at + 1
        while end < len(lines) and re.match(r"^[ \t]+-|^[ \t]+\S", lines[end]):  # its block list or folded lines
            end += 1
        lines[at:end] = [line]
    return "---\n" + "\n".join(lines) + "\n---\n" + rest


def mark_superseded(targets, new_title, scope, project=None):
    """Mark each old note as replaced by new_title: its front matter for the user's own notes, the scope's
    superseded.json for a shipped one. Returns the titles marked."""
    day = time.strftime("%Y-%m-%d")
    side = []
    for t, path, shelf in targets:
        if path is None:
            side.append((t, shelf))
            continue
        try:
            with open(path, encoding="utf-8") as f:
                old = f.read()
        except (OSError, UnicodeDecodeError):
            continue
        _atomic_write(path, _front_set(old, [("superseded_by", new_title), ("superseded_on", day)]))
    if side:
        root = folder(scope, project)
        with _locked(root):
            path = _note_path(root, SIDECAR)
            try:
                with open(path, encoding="utf-8") as f:
                    data = json.load(f)
                data = data if isinstance(data, dict) else {}
            except (OSError, ValueError, UnicodeDecodeError):
                data = {}
            for t, shelf in side:
                data[t] = {"by": new_title, "date": day, "shelf": shelf}
            _atomic_write(path, json.dumps(data, ensure_ascii=False, indent=1) + "\n")
    return [t[0] for t in targets]


# ------------------------------------------------------------------ writing
def _by():
    return " ".join((os.environ.get("RBXKIT_AGENT_NAME") or "agent").split())[:60]


def target_title(text, title, scope, project=None, replace=False):
    """(the title a note will be saved under, the path of the note it updates or None)."""
    want = safe_title(title or title_from((text or "").strip()))
    if replace and title:
        hit = [(t, p) for t, p in note_files(folder(scope, project)) if t.casefold() == want.casefold()]
        if len(hit) == 1 and os.path.basename(hit[0][1]) not in (RULES_NAME, LESSONS_NAME):
            return hit[0]
    return want, None


def write_note(text, title=None, scope="all", project=None, by=None, kind="note", replace=False, supersedes=None):
    """Save one note; returns (title, path). `replace`: a note with exactly this title is updated, not duplicated.
    `supersedes`: titles of older notes this one replaces (checked before anything is written, then marked)."""
    text = (text or "").strip()
    if not text:
        raise KitError("Give the note in 'text': what you were doing, what the user wants instead and why.")
    if len(text) > NOTE_CAP:
        raise KitError("A note holds at most %d characters (this one has %d). Keep the point and the user's words; "
                       "drop the rest." % (NOTE_CAP, len(text)))
    root = folder(scope, project)
    want, path = target_title(text, title, scope, project, replace)
    olds = supersedes_arg(supersedes)
    targets = supersede_targets(olds, want, scope, project) if olds else []
    if path:
        title = want
    else:
        title, path = _reserve(root, want)
    sup = ""
    if targets:
        names = [t[0] for t in targets]
        sup = "supersedes: %s\n" % json.dumps(names[0] if len(names) == 1 else names, ensure_ascii=False)
    body = ("---\ntitle: %s\nwhen: %s\nscope: %s\nby: %s\nkind: %s\n%s---\n\n# %s\n\n%s\n" % (
        title, time.strftime("%Y-%m-%d %H:%M"), scope, by or _by(), kind, sup, title, text))
    _atomic_write(path, body)
    if targets:
        mark_superseded([t for t in targets if t[1] != path], title, scope, project)
    return title, path


def add_rule(text, scope="all", project=None):
    rule = " ".join((text or "").split())
    if not rule:
        raise KitError("Give the rule in 'text', as one short sentence.")
    if len(rule) > RULE_CAP:
        raise KitError("A house rule holds at most %d characters (this one has %d). Shorten it, keeping the user's key "
                       "words, and put the reasons in a note." % (RULE_CAP, len(rule)))
    root = folder(scope, project)
    path = _note_path(root, RULES_NAME)
    with _locked(root):
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
        raise KitError("A house rule holds at most %d characters. Shorten it, keeping the user's key words." % RULE_CAP)
    root = folder(scope, project)
    path = _note_path(root, RULES_NAME)
    with _locked(root):
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
        # the cap only stops a file from growing: an overfull file (edited by hand) can always shrink
        if len(out) > RULES_FILE_CAP and len(out) > len(text):
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
    hit = [p for t, p in files if t.casefold() == want]
    if len(hit) != 1:  # only an exact title moves a note; a partial one only lists the candidates
        near = [t for t, _ in files if want in t.casefold()]
        if near:
            raise KitError("No note is called exactly '%s'. Did you mean: %s? Give the exact title." % (
                title, "; ".join(near[:20])))
        raise KitError("No note called '%s'. Notes: %s" % (title, ", ".join(t for t, _ in files[:30])))
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
def is_helper():
    """An expert or helper started by another agent (it does not talk to the user directly)."""
    return bool((os.environ.get("RBXKIT_AGENT_ROLE") or "").strip())


def check_helper(scope, rule=False):
    """Helpers keep what they learn to this game: no notes or lessons for every game, and no house rules."""
    if not is_helper():
        return
    if rule:
        raise KitError("Only a chat the user is talking to can add a house rule. Report the user's preference to "
                       "your Lead instead.")
    if scope == "all":
        raise KitError("An expert saves notes and lessons for this game only (scope game): what holds for every game "
                       "is the user's to say, through the Lead or their own chat.")


def _flag(v):
    return v is True or str(v).lower() == "true"


def run(action, a):
    project = project_dir()
    if action == "note":
        scope = scope_arg(a, project)
        rule = a.get("rule") is True or str(a.get("rule")).lower() == "true"
        check_helper(scope, rule)
        if rule:
            add_rule(a.get("text"), scope, project)
            return ("House rule saved (%s). Every agent gets it from the next turn on." %
                    ("this game" if scope == "game" else "everything"))
        replace = _flag(a.get("replace"))
        olds = supersedes_arg(a.get("supersedes"))
        import rbxupkeep  # the checks when a note is filed: no model, only the notes' words
        want, updating = target_title(a.get("text"), a.get("title"), scope, project, replace)
        if not _flag(a.get("force")) and (a.get("text") or "").strip():
            titled = bool(str(a.get("title") or "").strip())  # else its title is the text's first words
            dup = rbxupkeep.find_duplicate(a.get("text"), want, scope, project, skip=olds, exclude=updating,
                                           titled=titled)
            if dup:
                return rbxupkeep.duplicate_text(dup, scope, want)
        title, path = write_note(a.get("text"), a.get("title"), scope, project, replace=replace, supersedes=olds)
        out = ("Note saved as '%s' (%s). Search finds it: rbx_vault action 'search' with shelf '%s'." %
               (title, "this game" if scope == "game" else "everything", "game" if scope == "game" else "yours"))
        if olds:
            out += " It replaces %s: kept, marked superseded, and ranked below it in search." % ", ".join(
                "'%s'" % t for t in olds)
        found = rbxupkeep.find_conflicts(a.get("text"), title, scope, project, skip=olds, exclude=path)
        return out + rbxupkeep.conflicts_text(found, title) if found else out
    if action == "notes":
        rows = list_notes(scope_arg(a, project, default=False), project)
        if not rows:
            return "No notes yet."
        return "Notes, newest first:\n" + "\n".join("- %s  (%s, %s)" % (t, sc, w) for w, t, sc in rows[:60])
    if action == "forget":
        scope = scope_arg(a, project)
        check_helper(scope)
        name = forget(a.get("title"), scope, project)
        return "Moved '%s' to the notes folder's .trash (not deleted)." % name
    if action == "upkeep":
        import rbxupkeep
        return rbxupkeep.summary_text(rbxupkeep.sweep(projects=[project] if project else []))
    if action == "rule_edit":
        scope = scope_arg(a, project)
        check_helper(scope, rule=True)
        done = rule_edit(a.get("old"), a.get("text") if a.get("text") is not None else a.get("new"), scope, project)
        return "House rule %s." % done
    raise KitError("Unknown notes action: %s." % action)


def lesson(text, scope, project):
    """rbx_stage lesson: what an agent learned, kept apart from what the user said (front matter kind: lesson, shown
    as [lesson] in search, never a house rule). Helpers may only save lessons for this game."""
    check_helper(scope)
    text = " ".join((text or "").split())
    if not text:
        raise KitError("Give the lesson as one short sentence in 'text'.")
    return write_note(text, "Lesson - " + title_from(text, 6), scope, project, kind="lesson")
