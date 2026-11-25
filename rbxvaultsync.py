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
    """True when p is a safe vault-relative path: under an allowed top folder, inside a skill's folder, with no
    leading slash and every segment a safe name (_seg_ok). The same rule as vault_sync::check_path."""
    if not isinstance(p, str) or not p.startswith(ALLOWED_TOP) or p.startswith("/"):
        return False
    parts = p.split("/")
    if len(parts) < 3:  # skills live in folders: Skills/<skill>/<file>
        return False
    return all(_seg_ok(seg) for seg in parts)


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
            r = subprocess.run(["git", "merge-file", "-p"] + names, capture_output=True, timeout=30,
                               creationflags=rbxpaths.no_window())
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
            raise ValueError(BAD)


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
        return _apply(staged, manifest, base)


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

    def record(p, data, upstream=None):
        _record_base(base, p, data, upstream)

    def forget(p):
        _forget_base(base, p)

    def one(p, name):
        data = _read(os.path.join(staged, *p.split("/")))
        up = data
        if name in renamed and p == "Skills/%s/SKILL.md" % name:  # compared and merged as written locally
            data = rename_front(data, renamed[name])
        record_ = lambda p, d: record(p, d, up if up != d else None)
        target = lp(p)
        if not _target_ok(target, vault):
            raise OSError("outside the vault folder")
        local = _read_existing(target)
        old = _cmp_hash(base, p) if p in base["files"] else None
        nh = _sha(data)
        if old is not None:
            if local is None:
                base["deleted"].append(p)
                forget(p)
                res["skipped_deleted"].append(p)
            elif _sha(local) == nh:
                record_(p, data)
            elif _sha(local) == old:
                _write(target, data)
                record_(p, data)
                res["updated"].append(p)
            elif nh == old:
                pass  # the user's edit stands, upstream did not change
            else:
                merged = _merge(local, _read(os.path.join(bdir, *p.split("/"))), data)
                if merged is not None:
                    _write(target, merged)
                    record_(p, data)
                    res["merged"].append(p)
                else:
                    _put_beside(target, data)
                    res["kept"].append(p)
        elif local is None:
            _write(target, data)
            record_(p, data)
            res["added"].append(p)
        elif _sha(local) == nh:
            record_(p, data)
        else:
            _put_beside(target, data)
            res["kept"].append(p)

    # what upstream dropped goes first, so a skill it renamed is not compared with its old self below
    for p in sorted(b for b in base["files"] if b not in new and _skill(b) not in declined):
        try:
            if not check_path(p):
                forget(p)
                continue
            target = lp(p)
            if not _target_ok(target, vault):
                raise OSError("outside the vault folder")
            local = _read_existing(target)
            if local is not None and _sha(local) == _cmp_hash(base, p):
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
    import rbxvaultnew
    ctx = rbxvaultnew.Ctx(release)  # the library is read once, when the first newcomer needs it
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
                    kept = rbxvaultnew._renew(name, staged_dir)
                except (OSError, rbxpaths.KitError) as e:
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
                    answer = newcomer(name, staged_dir, ctx=ctx)
                except (OSError, rbxpaths.KitError) as e:
                    res["errors"].append("%s: %s" % (p, e))
                    decided[name] = False
                    continue
                what, _, arg = answer.partition(":")
                decided[name] = what in ("install", "settled")
                if decided[name]:
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
                d = rbxvaultnew._own_copy(name)
                if d:
                    rbxskills._aside(d, os.path.basename(d))
            res["settled"].append(name)
        except (OSError, ValueError, rbxpaths.KitError) as e:
            res["errors"].append("%s: %s" % (name, e))
    if current is not None:
        _save_base(base)

    base["release"] = release
    _save_base(base)
    return res

