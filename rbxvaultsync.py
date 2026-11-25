"""The shared Fridae vault: what was downloaded from it, as the base record remembers it, and applying an update
without overriding the user (file by file: replace what is untouched, merge what both changed, otherwise keep the
user's file and save the new one beside it)."""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import uuid

import rbxpaths

BASE_NAME = "base.json"
ALLOWED_TOP = ("Skills/",)
BESIDE = " (Fridae update)"
BAD = "This vault update looked wrong, so nothing was changed."
MERGE_MAX = 1 << 20


def state_dir():
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
    for seg in p.split("/"):
        if not seg or seg.startswith(".") or len(seg.encode("utf-8", "surrogatepass")) > 255:
            return False
    return True


def newcomer(name, staged_dir):
    """What to do with a skill that is new to this computer (no base record, no local folder): "install" now.
    The seam for Task 4's checks of newcomers, which will return other answers too."""
    return "install"


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _read(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def _inside(path, root):
    root = os.path.realpath(root)
    real = os.path.realpath(path)
    return real == root or real.startswith(root + os.sep)


def _target_ok(path, root):
    """The file itself is no symlink and its folder really lies inside root."""
    return not os.path.islink(path) and _inside(os.path.dirname(path), root)


def _write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.%s.tmp" % (path, uuid.uuid4().hex[:8])
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
        if not isinstance(files, list):
            raise ValueError
        seen = set()
        vault = rbxskills.vault_dir()
        root = os.path.realpath(staged)
        for f in files:
            if not isinstance(f, dict):
                raise ValueError
            p, h, size = f.get("path"), f.get("sha256"), f.get("size")
            if not check_path(p) or p in seen or not isinstance(h, str) or isinstance(size, bool) \
                    or not isinstance(size, int):
                raise ValueError
            seen.add(p)
            lp = _local_path(p, renamed)
            if not check_path(lp):
                raise ValueError
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
        _write(os.path.join(bdir, *p.split("/")), data)
        base["files"][p] = _sha(data)

    def forget(p):
        base["files"].pop(p, None)
        try:
            os.remove(os.path.join(bdir, *p.split("/")))
        except OSError:
            pass

    decided = {}  # upstream skill name -> install or not
    for p in sorted(new):
        name = _skill(p)
        if name in declined:
            if name not in res["declined"]:
                res["declined"].append(name)
            continue
        if p in base["deleted"]:
            res["skipped_deleted"].append(p)
            continue
        if name and name not in decided:
            known = any(_skill(b) == name for b in base["files"])
            local_dir = os.path.join(vault, renamed.get(name, name))
            if known or os.path.lexists(local_dir):
                decided[name] = True
            else:
                answer = newcomer(name, os.path.join(staged, "Skills", name))
                decided[name] = answer == "install"
                if answer in ("incoming", "settled"):
                    res[answer].append(name)
        if name and not decided[name]:
            continue
        try:
            data = _read(os.path.join(staged, *p.split("/")))
            target = lp(p)
            if not _target_ok(target, vault):
                raise OSError("outside the vault folder")
            local = _read(target) if os.path.lexists(target) else None
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
                        _write(_beside(target), data)
                        res["kept"].append(p)
            elif local is None:
                _write(target, data)
                record(p, data)
                res["added"].append(p)
            elif _sha(local) == nh:
                record(p, data)
            else:
                _write(_beside(target), data)
                res["kept"].append(p)
        except OSError as e:
            res["errors"].append("%s: %s" % (p, e))
        if name and (p == max(q for q in new if _skill(q) == name)):
            _save_base(base)

    for p in sorted(b for b in base["files"] if b not in new and _skill(b) not in declined):
        try:
            if not check_path(p):
                forget(p)
                continue
            target = lp(p)
            if not _target_ok(target, vault):
                raise OSError("outside the vault folder")
            local = _read(target) if os.path.lexists(target) else None
            if local is not None and _sha(local) == base["files"][p]:
                dest = os.path.join(state_dir(), "removed", release, *_local_path(p, renamed).split("/"))
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
