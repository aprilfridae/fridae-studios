"""The shared Fridae vault: what was downloaded from it, as the base record remembers it, and applying an update
without overriding the user (file by file: replace what is untouched, merge what both changed, otherwise keep the
user's file and save the new one beside it)."""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unicodedata
import uuid

import rbxpaths

BASE_NAME = "base.json"
ALLOWED_TOP = ("Skills/",)
BESIDE = " (Fridae update)"
BAD = "This vault update looked wrong, so nothing was changed."
MERGE_MAX = 1 << 20
# the longest file name accepted: room for " (Fridae update 99)" in the same folder (temp files have short names)
NAME_MAX = 255 - len(" (Fridae update 99)")
MAX_FILES, MAX_FILE, MAX_TOTAL = 5000, 1 << 20, 100 << 20


def state_dir():
    """The one place the vault's base record and moved-aside files live."""
    return os.path.join(rbxpaths.KNOWLEDGE_DIR, ".fridae-vault")


def base_path():
    return os.path.join(state_dir(), BASE_NAME)


def read_base():
    """The base record (<Knowledge>/.fridae-vault/base.json), every field filled: a missing or broken file, or a
    field of the wrong type, gives that field's empty value."""
    base = {"format": 1, "release": "", "files": {}, "deleted": [], "declined": [], "renamed": {}, "waiting": {},
            "patch": {}}
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


def check_path(p):
    """True when p is a safe vault-relative path: under an allowed top folder, with no empty, dot or dot-started
    segment, no backslash, NUL or leading slash, and every segment at most 255 bytes."""
    if not isinstance(p, str) or not p.startswith(ALLOWED_TOP) or "\\" in p or "\0" in p or p.startswith("/"):
        return False
    parts = p.split("/")
    if len(parts) < 3:  # skills live in folders: Skills/<skill>/<file>
        return False
    for seg in parts:
        if not seg or seg.startswith(".") or len(seg.encode("utf-8", "surrogatepass")) > 255:
            return False
    return True


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
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = os.path.join(os.path.dirname(path), ".%s.tmp" % uuid.uuid4().hex[:8])
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def _save_base(base):
    os.makedirs(state_dir(), exist_ok=True)
    tmp = base_path() + ".%s.tmp" % uuid.uuid4().hex[:8]
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(json.dumps(base, ensure_ascii=False, indent=1) + "\n")
    os.replace(tmp, base_path())


def _beside(path):
    stem, ext = os.path.splitext(path)
    return stem + BESIDE + ext


def _merge(local, base, new):
    """The clean three-way merge of three byte strings, or None (conflicts, binary, too big, no base, no git)."""
    if base is None or local is None:
        return None
    for d in (local, base, new):
        if len(d) > MERGE_MAX or b"\0" in d:
            return None
        try:
            d.decode("utf-8")
        except UnicodeDecodeError:
            return None
    with tempfile.TemporaryDirectory(prefix="vaultmerge") as tmp:
        names = []
        for n, d in (("local", local), ("base", base), ("new", new)):
            names.append(os.path.join(tmp, n))
            with open(names[-1], "wb") as f:
                f.write(d)
        try:
            r = subprocess.run(["git", "merge-file", "-p"] + names, capture_output=True, timeout=30)
        except (OSError, subprocess.SubprocessError):
            return None
    return r.stdout if r.returncode == 0 else None


def _skill(path):
    parts = path.split("/")
    return parts[1] if len(parts) > 2 else ""


def _local_path(path, renamed):
    parts = path.split("/")
    if len(parts) > 2 and parts[1] in renamed:
        parts[1] = renamed[parts[1]]
    return "/".join(parts)


def _no_symlinks():
    """Knowledge/Skills and Knowledge/.fridae-vault are real folders (when they exist)."""
    import rbxskills
    for d in (rbxskills.vault_dir(), state_dir()):  # Knowledge itself may be a link; what is below it may not
        if os.path.islink(d):
            raise ValueError


def _validate(staged, manifest, renamed):
    """Everything that can be checked before the first write; raises ValueError(BAD)."""
    import rbxskills
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
        _no_symlinks()
        seen, ups = set(), set()
        vault = rbxskills.vault_dir()
        root = os.path.realpath(staged)
        for f in files:
            if not isinstance(f, dict):
                raise ValueError
            p, h, size = f.get("path"), f.get("sha256"), f.get("size")
            if not check_path(p) or not isinstance(h, str):
                raise ValueError
            if os.path.splitext(p)[0].endswith(BESIDE):
                raise ValueError
            lp = _local_path(p, renamed)
            up = unicodedata.normalize("NFC", p).casefold()
            if up in ups or len(lp.rsplit("/", 1)[1].encode("utf-8", "surrogatepass")) > NAME_MAX \
                    or len(p.rsplit("/", 1)[1].encode("utf-8", "surrogatepass")) > NAME_MAX:
                raise ValueError
            ups.add(up)
            key = unicodedata.normalize("NFC", lp).casefold()
            if not check_path(lp) or not check_path(_beside(lp)) or key in seen:
                raise ValueError
            seen.add(key)
            src = os.path.join(staged, *p.split("/"))
            if os.path.islink(src) or not _inside(os.path.dirname(src), root):
                raise ValueError
            data = _read(src)
            if data is None or _sha(data) != h or len(data) != size:
                raise ValueError
            for q in (lp, _beside(lp)):
                if not _target_ok(os.path.join(rbxpaths.KNOWLEDGE_DIR, *q.split("/")), vault):
                    raise ValueError
    except (ValueError, TypeError, OSError):
        raise ValueError(BAD)


def apply(staged, manifest):
    """Apply a fetched vault release (files under staged/<path>, verified against the manifest) to
    <Knowledge>/Skills, file by file, never over what the user changed. Returns the lists in the result."""
    import rbxskills
    base = read_base()
    _validate(staged, manifest, base["renamed"])
    with rbxskills._locked(state_dir()):
        base = read_base()
        _validate(staged, manifest, base["renamed"])
        global _applying
        _applying = manifest["release"]
        try:
            return _apply(staged, manifest, base)
        finally:
            _applying = ""


def _record_base(base, p, data):
    """Remember what upstream's file p is: its copy under .fridae-vault/base and its hash in the base record."""
    copy = os.path.join(state_dir(), "base", *p.split("/"))
    if not _target_ok(copy, state_dir()):
        raise OSError("outside the vault record")
    _write(copy, data)
    base["files"][p] = _sha(data)


def _put_beside(target, data):
    """Save the new text beside the user's file as '<stem> (Fridae update)<ext>', never replacing a file already
    there: identical bytes are left, different ones get '(Fridae update 2)', '3'..."""
    stem, ext = os.path.splitext(target)
    n = 1
    while True:
        path = stem + (BESIDE if n == 1 else BESIDE[:-1] + " %d)" % n) + ext
        if not os.path.lexists(path):
            _write(path, data)
            return
        if _read_existing(path) == data:
            return
        n += 1
        if n > 99:
            raise OSError("too many saved copies beside it")


def _apply(staged, manifest, base):
    import rbxskills
    vault = rbxskills.vault_dir()
    release = manifest["release"]
    res = {"release": release, "added": [], "updated": [], "merged": [], "kept": [], "removed": [],
           "skipped_deleted": [], "declined": [], "incoming": [], "settled": [], "errors": []}
    renamed, declined = base["renamed"], set(base["declined"])
    new = {f["path"]: f for f in manifest["files"]}
    bdir = os.path.join(state_dir(), "base")

    def lp(p):
        return os.path.join(rbxpaths.KNOWLEDGE_DIR, *_local_path(p, renamed).split("/"))

    def record(p, data):
        _record_base(base, p, data)

    def forget(p):
        base["files"].pop(p, None)
        copy = os.path.join(bdir, *p.split("/"))
        try:
            if _target_ok(copy, state_dir()):
                os.remove(copy)
        except OSError:
            pass

    def one(p, name):
        data = _read(os.path.join(staged, *p.split("/")))
        target = lp(p)
        if not _target_ok(target, vault):
            raise OSError("outside the vault folder")
        local = _read_existing(target)
        old = base["files"].get(p)
        nh = _sha(data)
        if old is not None:
            if local is None:
                base["deleted"].append(p)
                forget(p)
                res["skipped_deleted"].append(p)
            elif _sha(local) == nh:
                record(p, data)
            elif _sha(local) == old:
                _write(target, data)
                record(p, data)
                res["updated"].append(p)
            elif nh == old:
                pass  # the user's edit stands, upstream did not change
            else:
                merged = _merge(local, _read(os.path.join(bdir, *p.split("/"))), data)
                if merged is not None:
                    _write(target, merged)
                    record(p, data)
                    res["merged"].append(p)
                else:
                    _put_beside(target, data)
                    res["kept"].append(p)
        elif local is None:
            _write(target, data)
            record(p, data)
            res["added"].append(p)
        elif _sha(local) == nh:
            record(p, data)
        else:
            _put_beside(target, data)
            res["kept"].append(p)

    decided = {}  # upstream skill name -> install or not
    own, failed = [], set()  # names whose own earlier share comes back (the user's copy is set aside once installed)
    current = None
    for p in sorted(new):
        name = _skill(p)
        if current is not None and name != current:
            _save_base(base)  # the skill folder before this one is done
        current = name
        if name in declined:
            if name not in res["declined"]:
                res["declined"].append(name)
            continue
        if p in base["deleted"]:
            res["skipped_deleted"].append(p)
            continue
        if name not in decided:
            staged_dir = os.path.join(staged, "Skills", name)
            if name in base["waiting"]:  # its decision is still open: the newer copy replaces the waiting one
                try:
                    kept = _renew(name, staged_dir)
                except OSError as e:
                    res["errors"].append("%s: %s" % (p, e))
                    decided[name] = False
                    continue
                if kept:
                    base["waiting"][name] = release
                    res["incoming"].append(name)
                    decided[name] = False
                    continue
                del base["waiting"][name]  # nobody waits for it any more: it meets the library afresh
            known = any(_skill(b) == name for b in base["files"])
            if known or os.path.lexists(os.path.join(vault, renamed.get(name, name))):
                decided[name] = True
            else:
                try:
                    answer = newcomer(name, staged_dir)
                except (OSError, rbxpaths.KitError) as e:
                    res["errors"].append("%s: %s" % (p, e))
                    decided[name] = False
                    continue
                what, _, arg = answer.partition(":")
                decided[name] = what in ("install", "settled")
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
        if not decided[name]:
            continue
        try:
            one(p, name)
        except OSError as e:
            failed.add(name)
            res["errors"].append("%s: %s" % (p, e))
    for name in own:  # installed whole: only now is the user's own earlier share set aside
        if name in failed:
            continue
        try:
            with rbxskills._locked(rbxskills.user_dir()):  # the vault's lock is held: always vault first, then library
                d = _own_copy(name)
                if d:
                    rbxskills._aside(d, os.path.basename(d))
            res["settled"].append(name)
        except (OSError, ValueError, rbxpaths.KitError) as e:
            res["errors"].append("%s: %s" % (name, e))
    if current is not None:
        _save_base(base)

    for p in sorted(b for b in base["files"] if b not in new and _skill(b) not in declined):
        try:
            if not check_path(p):
                forget(p)
                continue
            target = lp(p)
            if not _target_ok(target, vault):
                raise OSError("outside the vault folder")
            local = _read_existing(target)
            if local is not None and _sha(local) == base["files"][p]:
                dest = os.path.join(state_dir(), "removed", release, *_local_path(p, renamed).split("/"))
                if not _target_ok(dest, state_dir()):
                    raise OSError("outside the vault record")
                os.makedirs(os.path.dirname(dest), exist_ok=True)
                if os.path.lexists(dest):
                    dest += "." + uuid.uuid4().hex[:6]
                shutil.move(target, dest)
                res["removed"].append(p)
            forget(p)
        except OSError as e:
            res["errors"].append("%s: %s" % (p, e))
    base["release"] = release
    _save_base(base)
    return res


# ---- a new skill meets the library
# Two locks guard all this, always taken in this order: the vault's (state_dir()) first, then the user's library
# (rbxskills.user_dir(), where the decisions are kept). apply() holds the first when it calls newcomer(), which takes
# the second; decide_vault() takes both in the same order, and rbxskills.decide() hands over before taking either.
_applying = ""  # the release apply() is applying: newcomer() names it as the decision's source
NOT_THERE = "One of them is not where Fridae keeps it; check them again."
TAKEN = "That name is already used in the vault folder; check them again."


def incoming_dir():
    """Where a new vault skill waits while the user decides about it."""
    return os.path.join(state_dir(), "incoming")


def _body(text):
    """A SKILL.md's text after its front matter, trailing spaces off every line and blank ends trimmed."""
    import rbxskills
    return "\n".join(line.rstrip() for line in rbxskills.front(text)[1].splitlines()).strip()


def _own_copy(name):
    """The user's library folder of a skill they sent to the vault under this name and have not edited since (its
    .fridae-shared.json lists the name with the folder's present hash), or None."""
    import rbxskills
    for d in rbxskills._skill_dirs(rbxskills.user_dir()):
        if d.endswith(rbxskills.UPDATE_SUFFIX) or not rbxskills._mine(d):
            continue
        sent = rbxskills._read_json(os.path.join(d, rbxskills.SHARED_FILE), {}).get("sent")
        for e in sent if isinstance(sent, list) else []:
            if isinstance(e, dict) and str(e.get("name", "")).casefold() == name.casefold() \
                    and e.get("hash") == rbxskills.folder_hash(d):
                return d
    return None


def _stage_incoming(staged_dir, name):
    """Copy the new skill's files (regular files only, no dot-files) to incoming/<name>/ through a temporary folder
    and a rename, replacing a waiting copy of an older release."""
    parent, dst = incoming_dir(), os.path.join(incoming_dir(), name)
    if os.path.islink(state_dir()) or os.path.islink(parent) or not _target_ok(dst, state_dir()):
        raise OSError("outside the vault record")
    os.makedirs(parent, exist_ok=True)
    tmp = os.path.join(parent, ".tmp-" + uuid.uuid4().hex[:10])
    try:
        os.makedirs(tmp)
        for root, dirs, files in os.walk(staged_dir):
            dirs[:] = [d for d in dirs if not d.startswith(".") and not os.path.islink(os.path.join(root, d))]
            for f in files:
                src = os.path.join(root, f)
                if f.startswith(".") or os.path.islink(src):
                    continue
                data = _read(src)
                if data is None:
                    raise OSError("unreadable: " + src)
                _write(os.path.join(tmp, os.path.relpath(src, staged_dir)), data)
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


def _renew(name, staged_dir):
    """A newer release of a skill whose decision is open: its incoming copy is replaced and the decision stays, with
    its recommendation cleared (a side changed). The decision's id, or None when no decision is open for it."""
    import rbxskills
    with rbxskills._locked(rbxskills.user_dir()):
        decs = rbxskills._decisions()
        dec = next((d for d in decs if d.get("from") == "vault" and d.get("state") == "open"
                    and d.get("kind", "skill") == "skill" and d.get("b") == name), None)
        if dec is None:
            return None
        _stage_incoming(staged_dir, name)
        dec["recommendation"] = None
        rbxskills._save_decisions(decs)
        return dec["id"]


def newcomer(name, staged_dir):
    """What to do with a skill that is new to this computer (no base record, no local folder):
      "declined:app"   its name is one of Fridae's own skills'; never installed
      "settled:own"    the user's own earlier share, unedited since: install it, then set theirs aside
      "declined:same"  its SKILL.md says what a library skill's does already
      "incoming:<id>"  it overlaps a library skill: it waits in incoming/<name>/ for the user's decision <id>
      "install"        otherwise."""
    import rbxskills
    key = name.casefold()
    if key in rbxskills.APP_ONLY or any(e["origin"] == "app" and e["id"].casefold() == key
                                        for e in rbxskills.kit_skills()):
        return "declined:app"
    waiting = _renew(name, staged_dir)  # an earlier run opened its decision but did not record the wait
    if waiting:
        return "incoming:" + waiting
    if _own_copy(name):
        return "settled:own"
    try:
        text = rbxskills._read(os.path.join(staged_dir, "SKILL.md"))
    except OSError:
        return "install"  # not a skill the library lists: nothing to compare
    lib = rbxskills.library()  # no project: a project's skills are not the user's
    body = _body(text)
    for e in lib:
        try:
            if _body(rbxskills._read(os.path.join(e["dir"], "SKILL.md"))) == body:
                return "declined:same"
        except OSError:
            continue
    fm, _ = rbxskills.front(text)
    new = dict(rbxskills.meta(fm, staged_dir, "community"), id=name)
    idf = rbxskills._idf([rbxskills.summary(e) for e in lib])
    best, score = None, 0.0
    for e in lib:
        sc = rbxskills.similar(e, new, idf)
        if sc > score:
            best, score = e, sc
    if best is None or score < rbxskills.OVERLAP:
        return "install"
    _stage_incoming(staged_dir, name)
    dec = {"id": uuid.uuid4().hex[:12], "kind": "skill", "from": "vault", "a": best["id"], "b": name,
           "a_dir": best["dir"], "b_dir": os.path.join(incoming_dir(), name), "made": rbxskills._now(),
           "recommendation": None, "state": "open", "reason": "overlap",
           "source": "Fridae vault " + (_applying or read_base()["release"])}
    with rbxskills._locked(rbxskills.user_dir()):
        decs = rbxskills._decisions()
        decs.append(dec)
        rbxskills._save_decisions(decs)
    return "incoming:" + dec["id"]


def _install_folder(src, name, local, base):
    """Move the folder src (a waiting skill) to Knowledge/Skills/<local> and remember each of its files as upstream's
    <name> file (_record_base), as apply() does for one it writes."""
    import rbxskills
    vault = rbxskills.vault_dir()
    dst = os.path.join(vault, local)
    if not _target_ok(dst, vault):
        raise ValueError(NOT_THERE)
    if os.path.lexists(dst):
        raise ValueError(TAKEN)
    rels = []
    for root, dirs, files in os.walk(src):
        for n in dirs + files:
            if os.path.islink(os.path.join(root, n)):
                raise ValueError(NOT_THERE)
        rels += [os.path.relpath(os.path.join(root, f), src).replace(os.sep, "/") for f in files]
    paths = {r: "Skills/%s/%s" % (name, r) for r in rels}
    if not all(check_path(p) for p in paths.values()):
        raise ValueError(NOT_THERE)
    os.makedirs(vault, exist_ok=True)
    shutil.move(src, dst)
    for r, p in paths.items():
        _record_base(base, p, _read(os.path.join(dst, *r.split("/"))))


def _set_aside(src, name, release):
    """Move a waiting copy to .fridae-vault/removed/<release>/Skills/<name> (never deleted)."""
    import rbxskills
    parent = os.path.join(state_dir(), "removed", release or "declined", "Skills")
    dest = rbxskills._free(parent, name, " ")
    if not _target_ok(dest, state_dir()):
        raise OSError("outside the vault record")
    os.makedirs(parent, exist_ok=True)
    shutil.move(src, dest)


def decide_vault(dec, choice, merged_text):
    """Settle an open decision about a skill from the vault (the decision dec, found again by its id under the locks).
      keep       the waiting copy goes to removed/, and the name to declined: later versions are not asked about;
      replace    the waiting copy moves into Knowledge/Skills and its files enter the base record; a skill of the
                 user's is set aside in .replaced/, a shipped one is superseded; one of Fridae's own is refused;
      merge      merged_text becomes the skill in use's SKILL.md (phase 1's merge); the waiting copy goes to removed/
                 and the name to declined;
      keep_both  it joins as <name>-community (or the next free name), its front matter name set, renamed recorded.
    Returns the decision, now state "done"."""
    import rbxskills
    if choice not in rbxskills.CHOICES:
        raise ValueError("Choose keep, replace, merge or keep_both.")
    with rbxskills._locked(state_dir()):  # the vault first, then the library: see the order above
        with rbxskills._locked(rbxskills.user_dir()):
            decs = rbxskills._decisions()
            cur = next((d for d in decs if d.get("id") == dec.get("id") and d.get("state") == "open"
                        and d.get("from") == "vault"), None)
            if cur is None:
                raise LookupError("No open decision '%s'." % dec.get("id"))
            rbxskills._refuse_stale(decs, cur, choice, merged_text)
            _no_symlinks()
            base = read_base()
            _settle(cur, choice, merged_text, base)
            _save_base(base)
            cur.update(state="done", choice=choice, decided=rbxskills._now())
            rbxskills._save_decisions(decs)
            return cur


def _settle(dec, choice, merged_text, base):
    import rbxskills
    name, a, a_dir, b_dir = dec["b"], dec["a"], dec["a_dir"], dec["b_dir"]
    if not (isinstance(b_dir, str) and b_dir and check_path("Skills/%s/x" % name) and
            os.path.realpath(b_dir) == os.path.join(os.path.realpath(incoming_dir()), name)):
        raise ValueError(NOT_THERE)  # the waiting copy is only ever incoming/<name>
    owner, a_kit = rbxskills._kit_owner(a_dir)
    a_mine = not a_kit and rbxskills._mine(a_dir)
    vault = rbxskills.vault_dir()
    a_vault = not (a_kit or a_mine) and isinstance(a_dir, str) and os.path.realpath(a_dir) != os.path.realpath(vault) \
        and _inside(a_dir, vault)
    if owner == "app" and choice in ("replace", "merge"):
        raise ValueError(rbxskills.APP_SKILL)
    if choice == "merge" and not (merged_text or "").strip():
        raise ValueError("A merge needs the merged SKILL.md.")
    if not (a_kit or a_mine or (a_vault and choice == "merge")) and choice in ("replace", "merge"):
        raise ValueError(rbxskills.NOT_MINE)
    there = os.path.isdir(b_dir) and not os.path.islink(b_dir)
    if not there and choice in ("replace", "merge"):
        raise LookupError("The waiting skill's folder is missing: %s" % b_dir)
    differ = a.casefold() != name.casefold()
    if choice == "replace":
        asided = None
        if a_mine:
            asided = rbxskills._free(os.path.join(rbxskills.user_dir(), rbxskills.REPLACED), rbxskills._safe(a), " ")
            rbxskills._aside(a_dir, a)
        try:
            _install_folder(b_dir, name, name, base)
        except BaseException:
            if asided and os.path.isdir(asided):
                os.rename(asided, a_dir)  # the user's skill goes back
            raise
        if a_kit and differ:
            sup = base["patch"].setdefault(name, {}).setdefault("supersedes", [])
            if a not in sup:
                sup.append(a)
    elif choice == "keep_both":
        if there:
            local = os.path.basename(rbxskills._free(vault, name + "-community", "-"))
            _install_folder(b_dir, name, local, base)
            rbxskills._set_front(os.path.join(vault, local, "SKILL.md"), {"name": local})
            base["renamed"][name] = local
    else:  # keep, merge: the skill in use stays (merged), the waiting copy is set aside and never offered again
        if choice == "merge":
            rbxskills._merge_skill(rbxskills.user_dir(), a, a_dir, a_kit, merged_text, name, differ)
        if there:
            _set_aside(b_dir, name, base["waiting"].get(name, ""))
        if name not in base["declined"]:
            base["declined"].append(name)
    base["waiting"].pop(name, None)
