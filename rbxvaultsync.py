"""The shared Fridae vault: what was downloaded from it, as the base record remembers it, and applying an update
without overriding the user (replace what is untouched, combine what both changed, otherwise keep the user's file and
open a clash that waits for their choice). A skill is decided as one folder, any other file on its own."""
import difflib
import hashlib
import json
import os
import re
import shutil
import time
import unicodedata
import uuid

import rbxmerge
import rbxpaths

BASE_NAME = "base.json"
# the code's own data under Roblox/: it stays in the app and never travels (vault_sync::CODE_DATA)
CODE_DATA = ("Roblox/_tools/", "Roblox/_data/")
BAD = "This vault update looked wrong, so nothing was changed."
NOT_WAITING = "That choice is not waiting any more."
SHOW_MAX = 300  # the most lines of each side a clash card shows
SAVE_EVERY = 50  # the base record is saved after this many files outside Skills/ (and after each skill folder)
# at most 20,000 files, 8 MiB each, 256 MiB in all (vault_sync's limits: the guides are 1,352 files and 21 MB)
MAX_FILES, MAX_FILE, MAX_TOTAL = 20_000, 8 << 20, 256 << 20


def state_dir():
    """The one place the vault's base record and moved-aside files live."""
    return os.path.join(rbxpaths.KNOWLEDGE_DIR, ".fridae-vault")


def base_path():
    return os.path.join(state_dir(), BASE_NAME)


def read_base():
    """The base record (<Knowledge>/.fridae-vault/base.json), every field filled: a missing or broken file, or a
    field of the wrong type, gives that field's empty value."""
    base = {"format": 1, "release": "", "files": {}, "deleted": [], "declined": [], "renamed": {}, "waiting": {},
            "patch": {}, "clashes": {}}
    data = rbxpaths.read_json(base_path())
    if not isinstance(data, dict):
        return base
    for key, empty in base.items():
        val = data.get(key)
        if key == "format":
            if isinstance(val, int) and not isinstance(val, bool):
                base[key] = val
        elif key == "release":
            if isinstance(val, str):
                base[key] = val
        elif isinstance(val, type(empty)):
            base[key] = val
    return base


_DEVICES = ("con", "prn", "aux", "nul", "conin$", "conout$")
_DEVICE_DIGITS = "0123456789¹²³"
_NOT_IN_NAMES = '<>:"|?*\\'


def _seg_ok(seg):
    """vault_sync::seg_ok: whether one part of a path may be a file's or folder's name on every system. Not empty, not
    dot-started, at most 255 bytes, and nothing Windows would read as a drive, a stream or a device, or would change:
    no ':' or '<>"|?*\\', no control character, no trailing dot or space and no leading space, and none of CON PRN AUX
    NUL CONIN$ CONOUT$ COM0-9 LPT0-9 (also with the superscript 1, 2, 3), in any case, with or without an extension."""
    if not seg or seg.startswith(".") or len(seg.encode("utf-8", "surrogatepass")) > 255:
        return False
    if any(c in _NOT_IN_NAMES or unicodedata.category(c) == "Cc" for c in seg):
        return False
    if seg.endswith((".", " ")) or seg.startswith(" "):
        return False
    stem = seg.split(".")[0].rstrip(" ").lower()  # lower(), as Rust's to_lowercase, so both sides agree
    if stem in _DEVICES:
        return False
    return not (len(stem) == 4 and stem[:3] in ("com", "lpt") and stem[3] in _DEVICE_DIGITS)


def check_path(p):
    """True when p travels with the vault: everything in Knowledge but top-level files (House rules.md), dot-folders
    and the code's data (CODE_DATA); under Skills/, only inside a skill's folder. No leading slash, and every segment
    a safe name (_seg_ok). The same rule as vault_sync::check_path."""
    if not isinstance(p, str) or p.startswith("/") or p.startswith(CODE_DATA):
        return False
    parts = p.split("/")
    if len(parts) < (3 if parts[0] == "Skills" else 2):  # skills live in folders: Skills/<skill>/<file>
        return False
    return all(_seg_ok(seg) for seg in parts)


def unit_of(p):
    """What a clash is decided for: a skill whole ("Skills/<name>"), anything else per file (vault_sync::unit_of)."""
    parts = p.split("/")
    return "Skills/" + parts[1] if len(parts) > 2 and parts[0] == "Skills" else p


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _read(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def _read_existing(path):
    """The bytes of a target that may be there: None when nothing is, OSError when something is but is not a
    readable regular file (never mistaken for missing)."""
    if not os.path.lexists(path):
        return None
    if os.path.islink(path) or not os.path.isfile(path):
        raise OSError("not an ordinary file")
    with open(path, "rb") as f:
        return f.read()


def _inside(path, root):
    root = os.path.realpath(root)
    real = os.path.realpath(path)
    return real == root or real.startswith(root + os.sep)


def _target_ok(path, root):
    """The file itself is no symlink and its folder really lies inside root."""
    return not os.path.islink(path) and _inside(os.path.dirname(path), root)


def _write(path, data):
    """Write through a temporary file and rename it into place (rbxpaths.replace waits out a short Windows "Access is
    denied"). When the rename fails, the temporary file is removed and the error raised."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = os.path.join(os.path.dirname(path), ".%s.tmp" % uuid.uuid4().hex[:8])
    try:
        with open(tmp, "wb") as f:
            f.write(data)
        rbxpaths.replace(tmp, path)
    except OSError:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def _save_base(base):
    os.makedirs(state_dir(), exist_ok=True)
    tmp = base_path() + ".%s.tmp" % uuid.uuid4().hex[:8]
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(json.dumps(base, ensure_ascii=False, indent=1) + "\n")
    os.replace(tmp, base_path())


def _merge(local, base, new):
    """The clean three-way merge of three byte strings (rbxmerge.merge3, no git), or None (a clash, binary, too big,
    no base)."""
    if base is None or local is None:
        return None
    return rbxmerge.merge3(local, base, new)


def _skill(path):
    """The skill a path belongs to, or "" for anything outside Skills/ (declined and renamed name only skills)."""
    parts = path.split("/")
    return parts[1] if len(parts) > 2 and parts[0] == "Skills" else ""


def _local_path(path, renamed):
    parts = path.split("/")
    if len(parts) > 2 and parts[0] == "Skills" and parts[1] in renamed:
        parts[1] = renamed[parts[1]]
    return "/".join(parts)


def _top(p):
    """The real folder a vault path lands under: <Knowledge>/<its first part>."""
    return os.path.join(rbxpaths.KNOWLEDGE_DIR, p.split("/")[0])


def _no_symlinks(tops=("Skills",)):
    """Knowledge/<top> for each top folder and Knowledge/.fridae-vault are real folders (when they exist)."""
    for d in [os.path.join(rbxpaths.KNOWLEDGE_DIR, t) for t in tops] + [state_dir()]:
        if os.path.islink(d):  # Knowledge itself may be a link; what is below it may not
            raise ValueError(BAD)


def _validate(staged, manifest, renamed):
    """Everything that can be checked before the first write; raises ValueError(BAD)."""
    try:
        if not isinstance(manifest, dict) or manifest.get("format") != 1:
            raise ValueError
        rel = manifest.get("release")
        if not isinstance(rel, str) or not rel or "/" in rel or "\\" in rel or "\0" in rel or rel.startswith("."):
            raise ValueError
        files = manifest.get("files")
        if not isinstance(files, list) or not files or len(files) > MAX_FILES:
            raise ValueError
        total = 0
        for f in files:  # the limits, from the manifest alone, before any staged file is read
            size = f.get("size") if isinstance(f, dict) else None
            if isinstance(size, bool) or not isinstance(size, int) or size < 0 or size > MAX_FILE:
                raise ValueError
            total += size
        if total > MAX_TOTAL:
            raise ValueError
        if not all(isinstance(f, dict) and check_path(f.get("path")) for f in files):
            raise ValueError
        _no_symlinks(sorted({f["path"].split("/")[0] for f in files}))
        seen, ups = set(), set()
        root = os.path.realpath(staged)
        for f in files:
            p, h, size = f.get("path"), f.get("sha256"), f.get("size")
            if not isinstance(h, str):
                raise ValueError
            lp = _local_path(p, renamed)
            up = unicodedata.normalize("NFC", p).casefold()
            if up in ups:
                raise ValueError
            ups.add(up)
            key = unicodedata.normalize("NFC", lp).casefold()
            if not check_path(lp) or key in seen:
                raise ValueError
            seen.add(key)
            src = os.path.join(staged, *p.split("/"))
            if os.path.islink(src) or not _inside(os.path.dirname(src), root):
                raise ValueError
            data = _read(src)
            if data is None or _sha(data) != h or len(data) != size:
                raise ValueError
            if not _target_ok(os.path.join(rbxpaths.KNOWLEDGE_DIR, *lp.split("/")), _top(lp)):
                raise ValueError
    except (ValueError, TypeError, OSError):
        raise ValueError(BAD)


def apply(staged, manifest):
    """Apply a fetched vault release (files under staged/<path>, verified against the manifest) to <Knowledge>, never
    over what the user changed: a skill folder as one unit, any other file on its own. Returns the lists in the
    result; `clashes` holds the units waiting for the user's choice after this run."""
    import rbxskills
    base = read_base()
    _validate(staged, manifest, base["renamed"])
    with rbxskills._locked(state_dir()):
        base = read_base()
        _validate(staged, manifest, base["renamed"])
        return _apply(staged, manifest, base)


def clash_dir():
    """Where the vault's copy of each unit waiting for the user's choice is kept (clash/<upstream path>)."""
    return os.path.join(state_dir(), "clash")


def replaced_dir():
    """Where the user's files go when they choose Keep the vault's (replaced/<time>/<local path>; never deleted)."""
    return os.path.join(state_dir(), "replaced")


def _copy_of(p):
    """The vault's waiting copy of p (a file, or a skill unit's folder) under clash/."""
    path = os.path.join(clash_dir(), *p.split("/"))
    if not _target_ok(path, clash_dir()):
        raise OSError("outside the vault record")
    return path


def _prune(d, root):
    """Remove d and its parents while they are empty and inside root (never root itself)."""
    top = os.path.realpath(root)
    while os.path.realpath(d).startswith(top + os.sep):
        try:
            os.rmdir(d)  # only an empty folder goes
        except OSError:
            break
        d = os.path.dirname(d)


def _remove_copy(unit):
    """The vault's waiting copy of a unit removed, and the folders it leaves empty."""
    path = _copy_of(unit)
    if os.path.isdir(path) and not os.path.islink(path):
        shutil.rmtree(path)
    elif os.path.lexists(path):
        os.remove(path)
    _prune(os.path.dirname(path), clash_dir())


def incoming_dir():
    """Where a new vault skill waits while the user decides about it."""
    return os.path.join(state_dir(), "incoming")


def _base_copy(p):
    return os.path.join(state_dir(), "base", *p.split("/"))


def _record_base(base, p, data, upstream=None):
    """Remember what upstream's file p is: its copy under .fridae-vault/base (data: the bytes as written locally, which
    for a renamed skill's SKILL.md are the upstream bytes with the local name) and upstream's own hash in the base
    record (upstream: those bytes, when they differ from data)."""
    copy = _base_copy(p)
    if not _target_ok(copy, state_dir()):
        raise OSError("outside the vault record")
    _write(copy, data)
    base["files"][p] = _sha(data if upstream is None else upstream)


def _forget_base(base, p):
    base["files"].pop(p, None)
    copy = _base_copy(p)
    try:
        if _target_ok(copy, state_dir()):
            os.remove(copy)
    except OSError:
        pass


def _cmp_hash(base, p):
    """The hash local files are compared with: that of the base copy (what was written locally), which is not
    upstream's hash for a renamed skill's SKILL.md."""
    data = _read(_base_copy(p))
    return _sha(data) if data is not None else base["files"].get(p)


def rename_front(data, local):
    """A renamed skill's upstream SKILL.md bytes with the front matter name set to its local name."""
    import rbxskills
    text = data.decode("utf-8", "surrogateescape")
    return rbxskills.front_with(text, {"name": local}).encode("utf-8", "surrogateescape")


def newcomer(name, staged_dir, ctx=None):
    """A skill new to this computer: see rbxvaultnew.newcomer. (Here so apply and its callers have one place.)"""
    import rbxvaultnew
    return rbxvaultnew.newcomer(name, staged_dir, ctx)


def decide_vault(dec, choice, merged_text):
    """Settle a decision about a vault skill: see rbxvaultnew.decide_vault."""
    import rbxvaultnew
    return rbxvaultnew.decide_vault(dec, choice, merged_text)


def _plan(p, local, old, base_bytes, new):
    """What to do with one vault file: (action, merged bytes or None). local: the user's bytes (None: no file);
    old: the hash it is compared with when the base record knows p (else None); base_bytes: the base copy; new: the
    vault's bytes as they would be written. Actions: add, record, write, keep, deleted, merge (with the merged bytes)
    and clash. Pictures and models are never combined."""
    import rbxmerge
    nh = _sha(new)
    if old is not None:
        if local is None:
            return "deleted", None
        if _sha(local) == nh:
            return "record", None
        if _sha(local) == old:
            return "write", None
        if nh == old:
            return "keep", None  # the user's edit stands, upstream did not change
        if rbxmerge.is_binary(p, local) or rbxmerge.is_binary(p, new):
            return "clash", None
        merged = _merge(local, base_bytes, new)
        return ("merge", merged) if merged is not None else ("clash", None)
    if local is None:
        return "add", None
    if _sha(local) == nh:
        return "record", None
    return "clash", None


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def _apply(staged, manifest, base):
    import rbxmerge
    import rbxskills
    import rbxvaultnew
    vault = rbxskills.vault_dir()
    release = manifest["release"]
    res = {"release": release, "added": [], "updated": [], "merged": [], "removed": [], "skipped_deleted": [],
           "declined": [], "incoming": [], "settled": [], "clashes": [], "errors": []}
    renamed, declined, clashes = base["renamed"], set(base["declined"]), base["clashes"]
    new = {f["path"]: f for f in manifest["files"]}
    units = {}  # each unit's manifest paths, in order: a skill's folder, or one other file
    for p in sorted(new):
        units.setdefault(unit_of(p), []).append(p)

    def failed_on(p, e):
        res["errors"].append("%s: %s" % (p, e))

    def target_of(p):
        local = _local_path(p, renamed)
        target = os.path.join(rbxpaths.KNOWLEDGE_DIR, *local.split("/"))
        if not _target_ok(target, _top(local)):
            raise OSError("outside the Knowledge folder")
        return target

    def set_aside(src, rel):
        """Move src to removed/<release>/<rel> (a name already there gets a suffix)."""
        dest = os.path.join(state_dir(), "removed", release, *rel.split("/"))
        if not _target_ok(dest, state_dir()):
            raise OSError("outside the vault record")
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        if os.path.lexists(dest):
            dest += "." + uuid.uuid4().hex[:6]
        shutil.move(src, dest)

    def drop(p):
        """Upstream no longer has p: the user's file is set aside only if they never changed it."""
        try:
            if not check_path(p):
                _forget_base(base, p)
                return
            target = target_of(p)
            local = _read_existing(target)
            if local is not None and _sha(local) == _cmp_hash(base, p):
                set_aside(target, _local_path(p, renamed))
                res["removed"].append(p)
            _forget_base(base, p)
        except OSError as e:
            failed_on(p, e)

    copy_of, remove_copy = _copy_of, _remove_copy

    def open_clash(unit, kind, paths):
        """The unit waits for the user's choice: the vault's copy of it under clash/, its entry in the record. The
        user's files and the unit's base record are left as they are."""
        remove_copy(unit)
        for q in paths:
            _write(copy_of(q), _read(os.path.join(staged, *q.split("/"))))
        made = clashes[unit].get("made") if isinstance(clashes.get(unit), dict) else None
        clashes[unit] = {"release": release, "kind": kind, "paths": list(paths), "made": made or _now()}

    def close_clash(unit):
        if unit in clashes:
            remove_copy(unit)
            del clashes[unit]

    def staged_bytes(p, name):
        """(the bytes as written locally, upstream's bytes): a renamed skill's SKILL.md carries its local name."""
        data = _read(os.path.join(staged, *p.split("/")))
        if name in renamed and p == "Skills/%s/SKILL.md" % name:  # compared and merged as written locally
            return rename_front(data, renamed[name]), data
        return data, data

    def plan(p, name=""):
        data, up = staged_bytes(p, name)
        target = target_of(p)
        local = _read_existing(target)
        old = _cmp_hash(base, p) if p in base["files"] else None
        action, merged = _plan(p, local, old, _read(_base_copy(p)) if old is not None else None, data)
        return {"p": p, "action": action, "merged": merged, "data": data, "up": up, "target": target,
                "local": local}

    def carry(pl):
        p, action, data, target = pl["p"], pl["action"], pl["data"], pl["target"]
        up = pl["up"] if pl["up"] != data else None
        if action == "deleted":
            base["deleted"].append(p)
            _forget_base(base, p)
            res["skipped_deleted"].append(p)
        elif action in ("add", "write", "merge"):
            _write(target, pl["merged"] if action == "merge" else data)
            _record_base(base, p, data, up)
            res[{"add": "added", "write": "updated", "merge": "merged"}[action]].append(p)
        elif action == "record":
            _record_base(base, p, data, up)
        # keep: the user's edit stands, upstream did not change

    # a Keep the vault's cut off part way is finished first, from its waiting copy (_settle), so its file is never
    # missing below and read as the user deleting it; one that cannot be finished stays waiting (skipped below)
    for unit in sorted(u for u, c in clashes.items() if isinstance(c, dict) and "keeping" in c):
        try:
            _settle(base, unit, "vault")
        except (OSError, LookupError, ValueError) as e:
            failed_on(unit, e)
    # what upstream dropped goes first, so a skill it renamed is not compared with its old self below; a dropped file
    # of a skill the vault still has goes with that skill, so a skill that clashes is left whole
    for p in sorted(b for b in base["files"] if unit_of(b) not in units and _skill(b) not in declined):
        drop(p)
    for unit in sorted(u for u in clashes if u not in units):  # a waiting unit the vault no longer has
        try:
            path = copy_of(unit)
            if os.path.lexists(path):
                set_aside(path, unit)
            remove_copy(unit)
            del clashes[unit]
        except OSError as e:
            failed_on(unit, e)

    ctx = rbxvaultnew.Ctx(release)  # the library is read once, when the first newcomer needs it
    own, failed = [], set()  # names whose own earlier share comes back (the user's copy is set aside once installed)

    def install(name, first):
        """Whether the vault's skill `name` goes into Knowledge/Skills: known or already there, or a newcomer the
        library agrees to (rbxvaultnew)."""
        staged_dir = os.path.join(staged, "Skills", name)
        if name in base["waiting"]:  # its decision is still open: the newer copy replaces the waiting one
            try:
                kept = rbxvaultnew._renew(name, staged_dir)
            except (OSError, rbxpaths.KitError) as e:
                failed_on(first, e)
                return False
            if kept:
                base["waiting"][name] = release
                res["incoming"].append(name)
                return False
            del base["waiting"][name]  # nobody waits for it any more: it meets the library afresh
        known = any(_skill(b) == name for b in base["files"])
        if known or os.path.lexists(os.path.join(vault, renamed.get(name, name))):
            return True
        try:
            answer = newcomer(name, staged_dir, ctx=ctx)
        except (OSError, rbxpaths.KitError) as e:
            failed_on(first, e)
            return False
        what, _, arg = answer.partition(":")
        if what in ("install", "settled"):
            ctx.installed.append(os.path.join(vault, name))
        if what == "settled":
            own.append(name)
        elif what == "declined":
            res["declined"].append(name)
            if arg == "app":  # never installed, whatever a later release brings
                base["declined"].append(name)
                declined.add(name)
        elif what == "incoming":
            base["waiting"][name] = release
            res["incoming"].append(name)
        return what in ("install", "settled")

    def skill_unit(name, unit, paths):
        """A skill folder as one unit: planned whole, and written only when none of its files clashes."""
        if name in declined:
            if name not in res["declined"]:
                res["declined"].append(name)
            return
        live = []
        for p in paths:
            if p in base["deleted"]:
                res["skipped_deleted"].append(p)
            else:
                live.append(p)
        if not live or not install(name, live[0]):
            return
        plans = []
        for p in live:
            try:
                plans.append(plan(p, name))
            except OSError as e:
                failed.add(name)
                failed_on(p, e)
        if any(pl["action"] == "clash" for pl in plans):
            try:
                open_clash(unit, "skill", paths)
            except OSError as e:
                failed.add(name)
                failed_on(unit, e)
            return
        for pl in plans:
            try:
                carry(pl)
            except OSError as e:
                failed.add(name)
                failed_on(pl["p"], e)
        for p in sorted(b for b in base["files"] if _skill(b) == name and b not in new):
            drop(p)
        try:
            close_clash(unit)
        except OSError as e:
            failed_on(unit, e)

    def file_unit(p):
        """Any file outside Skills/ is its own unit."""
        if p in base["deleted"]:
            res["skipped_deleted"].append(p)
            return
        try:
            pl = plan(p)
            if pl["action"] == "clash":
                binary = rbxmerge.is_binary(p, pl["data"]) or (pl["local"] is not None and
                                                               rbxmerge.is_binary(p, pl["local"]))
                open_clash(p, "binary" if binary else "file", [p])
            else:
                carry(pl)
                close_clash(p)
        except OSError as e:
            failed_on(p, e)

    others = 0
    for unit, paths in units.items():
        if isinstance(clashes.get(unit), dict) and "keeping" in clashes[unit]:
            continue  # a Keep the vault's that could not be finished: it stays waiting, never read as a deletion
        name = _skill(paths[0])
        if name:
            skill_unit(name, unit, paths)
            _save_base(base)  # the skill folder is done
        else:
            file_unit(unit)
            others += 1
            if others % SAVE_EVERY == 0:
                _save_base(base)
    for name in own:  # installed whole: only now is the user's own earlier share set aside
        if name in failed:
            continue
        try:
            with rbxskills._locked(rbxskills.user_dir()):  # the vault's lock is held: always vault first, then library
                d = rbxvaultnew._own_copy(name)
                if d:
                    rbxskills._aside(d, os.path.basename(d))
            res["settled"].append(name)
        except (OSError, ValueError, rbxpaths.KitError) as e:
            res["errors"].append("%s: %s" % (name, e))

    base["release"] = release
    res["clashes"] = sorted(clashes)
    _save_base(base)
    return res


# ---- the clashes waiting for the user's choice: Keep the vault's / Keep mine
def _cut(lines):
    if len(lines) > SHOW_MAX:
        lines = lines[:SHOW_MAX] + ["\u2026 (%d more lines)" % (len(lines) - SHOW_MAX)]
    return "\n".join(lines)


def _changed(before, after):
    """How many lines were added or removed between two texts."""
    return sum(1 for x in difflib.ndiff(before.splitlines(), after.splitlines()) if x.startswith(("+ ", "- ")))


def _safe_read(path):
    try:
        return _read_existing(path)
    except OSError:
        return None


def _unit_local(unit, renamed):
    """A unit's path as this machine keeps it (a renamed skill in its new folder)."""
    if unit.startswith("Skills/") and unit.count("/") == 1:
        return _local_path(unit + "/x", renamed).rsplit("/", 1)[0]
    return _local_path(unit, renamed)


def clashes():
    """The units waiting for the user's choice, each with what a card shows: the name, both sides' size, how many
    lines each changed against the version both started from, and a short view of both changes."""
    import rbxmerge
    base = read_base()
    out = []
    for unit, c in sorted(base["clashes"].items()):
        if not isinstance(c, dict):
            continue
        paths = [p for p in c.get("paths", []) if isinstance(p, str) and check_path(p)]
        if not paths:
            continue
        skill = unit != paths[0] or unit_of(paths[0]) != paths[0]
        local_unit = _unit_local(unit, base["renamed"])
        both_new = not any(p in base["files"] for p in paths)
        binary = c.get("kind") == "binary"
        mine = {"path": local_unit, "changed": None if both_new else 0, "size": 0, "modified": None}
        vault = {"changed": None if both_new else 0, "size": 0}
        show = {"mine": [], "vault": []}
        files = []
        for p in paths:
            lp = _local_path(p, base["renamed"])
            local = _safe_read(os.path.join(rbxpaths.KNOWLEDGE_DIR, *lp.split("/")))
            theirs = _read(os.path.join(clash_dir(), *p.split("/")))
            before = _read(_base_copy(p)) if p in base["files"] else None
            mine["size"] += len(local or b"")
            vault["size"] += len(theirs or b"")
            if skill and local != theirs:
                files.append(p.split("/", 2)[2])
            sides = [d for d in (local, theirs, before) if d is not None]
            if binary or any(rbxmerge.is_binary(p, d) for d in sides):
                mine["changed"] = vault["changed"] = None
                continue
            m = (local or b"").decode("utf-8")
            v = (theirs or b"").decode("utf-8")
            head = ["%s:" % p.split("/", 2)[2]] if skill and len(paths) > 1 else []
            if both_new:
                show["mine"] += head + m.splitlines()
                show["vault"] += head + v.splitlines()
                continue
            b = (before or b"").decode("utf-8")
            if mine["changed"] is not None:
                mine["changed"] += _changed(b, m)
                vault["changed"] += _changed(b, v)
            for side, text in (("mine", m), ("vault", v)):
                show[side] += list(difflib.unified_diff(b.splitlines(), text.splitlines(), p, p, lineterm=""))
        if binary:
            show = {"mine": [], "vault": []}
        try:
            mine["modified"] = os.path.getmtime(os.path.join(rbxpaths.KNOWLEDGE_DIR, *local_unit.split("/")))
        except OSError:
            pass
        name = local_unit.rsplit("/", 1)[-1]
        title = name if skill else (name[:-3] if name.lower().endswith(".md") else name)
        out.append({"unit": unit, "kind": c.get("kind", "file"), "title": title, "release": c.get("release", ""),
                    "made": c.get("made", ""), "both_new": both_new, "mine": mine, "vault": vault,
                    "files": files if skill else [], "show": {k: _cut(v) for k, v in show.items()}})
    return out


def decide_clash(unit, choice):
    """Settle one waiting unit. "vault": the user's files that differ from the vault's (for a skill, every file in its
    folder that is not the vault's) move to replaced/<time>/, and the vault's are written and recorded. "mine": the
    user's files stay, and the vault's version becomes the starting point (recorded as the base), so its later changes
    combine instead of asking again. Either way the waiting copy and the entry go."""
    import rbxskills
    if choice not in ("vault", "mine"):
        raise ValueError("Choose vault or mine.")
    with rbxskills._locked(state_dir()):
        base = read_base()
        replaced = _settle(base, unit, choice)
    return {"unit": unit, "choice": choice, "replaced": replaced}


_KEEPING_NAME = re.compile(r"^[0-9a-z-]{1,40}$")


def _settle(base, unit, choice):
    """decide_clash's work, with the vault's lock held; returns the replaced/ folder (Knowledge-relative) or None.
    Keep the vault's is made safe against being cut off part way (the app quits): before any of the user's files
    moves, the clash's entry gains "keeping" (the name of its replaced/ folder) and the record is saved. A unit left
    with "keeping" is finished, never read as the user deleting the file: by the next Pull newest (_apply), or by
    pressing Keep the vault's again (the same folder); Keep mine instead puts the user's files back from it."""
    c = base["clashes"].get(unit)
    paths = c.get("paths") if isinstance(c, dict) else None
    if not isinstance(paths, list) or not paths or not all(isinstance(p, str) and check_path(p) for p in paths) \
            or any(unit_of(p) != unit for p in paths):
        raise LookupError(NOT_WAITING)
    copies = {p: _read(_copy_of(p)) for p in paths}
    if any(d is None for d in copies.values()):
        raise LookupError(NOT_WAITING)
    _no_symlinks(sorted({p.split("/")[0] for p in paths}))
    renamed = base["renamed"]
    name = _skill(paths[0])

    def bytes_of(p):
        """(as written locally, upstream's): a renamed skill's SKILL.md carries its local name."""
        data = copies[p]
        if name in renamed and p == "Skills/%s/SKILL.md" % name:
            return rename_front(data, renamed[name]), data
        return data, data

    def target(lp):
        t = os.path.join(rbxpaths.KNOWLEDGE_DIR, *lp.split("/"))
        if not _target_ok(t, _top(lp)):
            raise OSError("outside the Knowledge folder")
        return t

    def rel_of(path):
        return os.path.relpath(path, rbxpaths.KNOWLEDGE_DIR).replace(os.sep, "/")

    wanted = {_local_path(p, renamed): bytes_of(p)[0] for p in paths}
    keeping = c.get("keeping")
    root = None
    if isinstance(keeping, str) and _KEEPING_NAME.match(keeping):
        root = os.path.join(replaced_dir(), keeping)
        if not _target_ok(root, state_dir()):
            raise OSError("outside the vault record")
    replaced = None
    if choice == "vault":
        if root is None:  # the intent first: a cut from here on is finished on the next run
            stamp = time.strftime("%Y%m%d-%H%M%S")
            if os.path.lexists(os.path.join(replaced_dir(), stamp)):
                stamp += "-" + uuid.uuid4().hex[:6]
            root = os.path.join(replaced_dir(), stamp)
            c["keeping"] = stamp
            _save_base(base)
        mine = []  # (Knowledge-relative path, full path) of the user's files to set aside
        if name:
            folder = target(_unit_local(unit, renamed) + "/x")[:-2]
            for r, ds, fs in os.walk(folder):
                for f in fs:
                    full = os.path.join(r, f)
                    mine.append((rel_of(full), full))
        else:
            lp = _local_path(unit, renamed)
            if os.path.lexists(target(lp)):
                mine.append((lp, target(lp)))
        aside = [(rel, full) for rel, full in mine if rel not in wanted or _safe_read(full) != wanted[rel]]
        for rel, full in aside:
            dest = os.path.join(root, *rel.split("/"))
            if not _target_ok(dest, state_dir()):
                raise OSError("outside the vault record")
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            if os.path.lexists(dest):
                dest += "." + uuid.uuid4().hex[:6]
            shutil.move(full, dest)
        if aside and name:  # folders left empty inside the skill go too
            folder = target(_unit_local(unit, renamed) + "/x")[:-2]
            for r, ds, fs in os.walk(folder, topdown=False):
                if r != folder and not os.listdir(r):
                    os.rmdir(r)
        if os.path.isdir(root):
            replaced = rel_of(root)
        for p in paths:
            data, up = bytes_of(p)
            _write(target(_local_path(p, renamed)), data)
            _record_base(base, p, data, up if up != data else None)
            if p in base["deleted"]:
                base["deleted"].remove(p)
    else:
        if root is not None and os.path.isdir(root):  # a cut-off Keep the vault's: the user's files go back
            for r, ds, fs in os.walk(root):
                for f in fs:
                    full = os.path.join(r, f)
                    lp = os.path.relpath(full, root).replace(os.sep, "/")
                    if "/" not in lp or lp.startswith("."):
                        continue
                    t = target(lp)
                    if not os.path.lexists(t) or _safe_read(t) == wanted.get(lp):
                        os.makedirs(os.path.dirname(t), exist_ok=True)
                        rbxpaths.replace(full, t)
            _prune_tree(root, replaced_dir())
        for p in paths:
            data, up = bytes_of(p)
            _record_base(base, p, data, up if up != data else None)
    _remove_copy(unit)
    del base["clashes"][unit]
    _save_base(base)
    return replaced


def _prune_tree(root, top):
    """Remove the empty folders of root, root included when it is left empty (never top)."""
    for r, ds, fs in os.walk(root, topdown=False):
        if not os.listdir(r):
            os.rmdir(r)
    _prune(os.path.dirname(root), top)
