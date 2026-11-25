"""A new vault skill meets the library: overlaps wait for the user's decision (Keep / Replace / Merge / Keep both).
Split from rbxvaultsync, which calls in here; this module imports rbxvaultsync, never the other way at import time."""
import os
import shutil
import uuid

import rbxvaultsync
from rbxvaultsync import (_inside, _read, _record_base, _forget_base, _save_base, _skill, _target_ok, _write,
                          _no_symlinks, check_path, incoming_dir, read_base, state_dir)

# ---- a new skill meets the library
# Two locks guard all this, always taken in this order: the vault's (state_dir()) first, then the user's library
# (rbxskills.user_dir(), where the decisions are kept). apply() holds the first when it calls newcomer(), which takes
# the second; decide_vault() takes both in the same order, and rbxskills.decide() hands over before taking either.
NOT_THERE = "One of them is not where Fridae keeps it; check them again."
TAKEN = "That name is already used in the vault folder; check them again."


def _body(text):
    """A SKILL.md's text after its front matter, trailing spaces off every line and blank ends trimmed."""
    import rbxskills
    return "\n".join(line.rstrip() for line in rbxskills.front(text)[1].splitlines()).strip()


def _own_index():
    """{casefolded name: folder} of the user's library skills they sent to the vault and have not edited since (their
    .fridae-shared.json lists the name with the folder's present hash)."""
    import rbxskills
    out = {}
    for d in rbxskills._skill_dirs(rbxskills.user_dir()):
        if d.endswith(rbxskills.UPDATE_SUFFIX) or not rbxskills._mine(d):
            continue
        sent = rbxskills._read_json(os.path.join(d, rbxskills.SHARED_FILE), {}).get("sent")
        h = None
        for e in sent if isinstance(sent, list) else []:
            if isinstance(e, dict) and isinstance(e.get("name"), str):
                h = h or rbxskills.folder_hash(d)
                if e.get("hash") == h:
                    out.setdefault(e["name"].casefold(), d)
    return out


def _own_copy(name):
    """The folder of the user's own earlier share of this name, unedited since, or None."""
    return _own_index().get(name.casefold())


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


class Ctx:
    """What one apply() learns once and every newcomer of it uses: the release, the library (read at the first use,
    then kept up to date with what this apply installs or sets aside), the word weights over it, the names of the app's
    own skills and each library skill's SKILL.md body."""

    def __init__(self, release):
        self.release = release
        self.installed = []  # folders of skills this apply installs: they join the library for the next newcomer
        self._lib = self._idf = self._apps = self._own = self._open = self._patch = None
        self._bodies = {}

    def own(self, name):
        if self._own is None:
            self._own = _own_index()
        return self._own.get(name.casefold())

    def has_open(self, name):
        """Whether a vault decision about this name is open (the decisions are read once)."""
        import rbxskills
        if self._open is None:
            self._open = {d.get("b") for d in rbxskills._decisions() if d.get("from") == "vault"
                          and d.get("state") == "open" and d.get("kind", "skill") == "skill"}
        return name in self._open

    def apps(self):
        import rbxskills
        if self._apps is None:
            self._apps = {e["id"].casefold() for e in rbxskills.kit_skills() if e["origin"] == "app"}
        return self._apps

    def library(self):
        import rbxskills
        if self._lib is None:
            self._lib = rbxskills.library()  # no project: a project's skills are not the user's
            self._idf = rbxskills._idf([rbxskills.summary(e) for e in self._lib])
        self._lib = [e for e in self._lib if os.path.isfile(os.path.join(e["dir"], "SKILL.md"))]
        have = {os.path.realpath(e["dir"]) for e in self._lib}
        if self._patch is None:
            self._patch = read_base()["patch"]
        patch = self._patch
        for d in self.installed:
            if os.path.realpath(d) not in have and os.path.isfile(os.path.join(d, "SKILL.md")):
                self._lib.append(rbxskills._entry(d, "vault", "community", patch=patch))
        self.installed = []
        return self._lib

    def idf(self):
        self.library()
        return self._idf

    def body(self, d):
        import rbxskills
        if d not in self._bodies:
            self._bodies[d] = _body(rbxskills._read(os.path.join(d, "SKILL.md")))
        return self._bodies[d]


def newcomer(name, staged_dir, ctx=None):
    """What to do with a skill that is new to this computer (no base record, no local folder):
      "declined:app"   its name is one of Fridae's own skills'; never installed
      "settled:own"    the user's own earlier share, unedited since: install it, then set theirs aside
      "declined:same"  its SKILL.md says what a library skill's does already
      "incoming:<id>"  it overlaps a library skill: it waits in incoming/<name>/ for the user's decision <id>
      "install"        otherwise.
    ctx: what the apply this is part of already learned (Ctx); alone, a new one is made."""
    import rbxskills
    ctx = ctx or Ctx(read_base()["release"])
    key = name.casefold()
    if key in rbxskills.APP_ONLY or key in ctx.apps():
        return "declined:app"
    waiting = _renew(name, staged_dir) if ctx.has_open(name) else None  # a run that opened it did not record the wait
    if waiting:
        return "incoming:" + waiting
    if ctx.own(name):
        return "settled:own"
    try:
        text = rbxskills._read(os.path.join(staged_dir, "SKILL.md"))
    except OSError:
        return "install"  # not a skill the library lists: nothing to compare
    lib = ctx.library()
    body = _body(text)
    for e in lib:
        try:
            if ctx.body(e["dir"]) == body:
                return "declined:same"
        except OSError:
            continue
    fm, _ = rbxskills.front(text)
    new = dict(rbxskills.meta(fm, staged_dir, "community"), id=name)
    best, score = None, 0.0
    for e in lib:
        sc = rbxskills.similar(e, new, ctx.idf())
        if sc > score:
            best, score = e, sc
    if best is None or score < rbxskills.OVERLAP:
        return "install"
    _stage_incoming(staged_dir, name)
    dec = {"id": uuid.uuid4().hex[:12], "kind": "skill", "from": "vault", "a": best["id"], "b": name,
           "a_dir": best["dir"], "b_dir": os.path.join(incoming_dir(), name), "made": rbxskills._now(),
           "recommendation": None, "state": "open", "reason": "overlap", "source": "Fridae vault " + ctx.release}
    with rbxskills._locked(rbxskills.user_dir()):
        decs = rbxskills._decisions()
        decs.append(dec)
        rbxskills._save_decisions(decs)
    return "incoming:" + dec["id"]


def _install_folder(src, name, local, base):
    """Move the folder src (a waiting skill) to Knowledge/Skills/<local> and remember each of its files as upstream's
    <name> file (_record_base), as apply() does for one it writes. When local is not name, its SKILL.md is written, and
    its base copy kept, with the front matter name set to local (rbxvaultsync.rename_front)."""
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
    try:
        for r, p in paths.items():
            full = os.path.join(dst, *r.split("/"))
            data = _read(full)
            if local != name and r == "SKILL.md":
                mine = rbxvaultsync.rename_front(data, local)
                _write(full, mine)
                _record_base(base, p, mine, data)
            else:
                _record_base(base, p, data)
    except BaseException:  # all or nothing: the folder goes back where it waited and nothing stays recorded
        for p in paths.values():
            _forget_base(base, p)
        shutil.move(dst, src)
        raise


def _set_aside(src, name, release):
    """Move a waiting copy to .fridae-vault/removed/<release>/Skills/<name> (never deleted)."""
    import rbxskills
    parent = os.path.join(state_dir(), "removed", release or "declined", "Skills")
    dest = rbxskills._free(parent, name, " ")
    if not _target_ok(dest, state_dir()):
        raise OSError("outside the vault record")
    os.makedirs(parent, exist_ok=True)
    shutil.move(src, dest)
    return dest


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


def _forget_vault_skill(base, a_dir):
    """The vault skill whose folder a_dir was is gone from the vault folder: forget what the base record knew of it. Its
    upstream name is declined (not offered again) only when upstream had it."""
    local = os.path.basename(a_dir)
    up = next((u for u, l in base["renamed"].items() if l == local), local)
    had = [p for p in base["files"] if _skill(p) == up]
    for p in had:
        _forget_base(base, p)
    base["renamed"].pop(up, None)
    base["waiting"].pop(up, None)
    if had and up not in base["declined"]:
        base["declined"].append(up)


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
    if a_vault and os.path.realpath(os.path.dirname(a_dir)) != os.path.realpath(vault):
        a_vault = False  # only a skill folder directly in Knowledge/Skills
    if not (a_kit or a_mine or a_vault) and choice in ("replace", "merge"):
        raise ValueError(rbxskills.NOT_MINE)
    there = os.path.isdir(b_dir) and not os.path.islink(b_dir)
    if not there and choice in ("replace", "merge"):
        raise LookupError("The waiting skill's folder is missing: %s" % b_dir)
    differ = a.casefold() != name.casefold()
    if choice == "replace":
        asided = removed = None
        if a_vault:  # one of the vault's own: moved to removed/, never deleted, and not offered again
            removed = _set_aside(a_dir, os.path.basename(a_dir), base["waiting"].get(name, ""))
        if a_mine:
            asided = rbxskills._free(os.path.join(rbxskills.user_dir(), rbxskills.REPLACED), rbxskills._safe(a), " ")
            rbxskills._aside(a_dir, a)
        try:
            _install_folder(b_dir, name, name, base)
        except BaseException:
            if asided and os.path.isdir(asided):
                os.rename(asided, a_dir)  # the user's skill goes back
            if removed and os.path.isdir(removed):
                shutil.move(removed, a_dir)
            raise
        if removed:
            _forget_vault_skill(base, a_dir)
        if a_kit and differ:
            sup = base["patch"].setdefault(name, {}).setdefault("supersedes", [])
            if a not in sup:
                sup.append(a)
    elif choice == "keep_both":
        if there:
            taken = {e["id"].casefold() for e in rbxskills.library()}
            local, i = name + "-community", 1
            while os.path.lexists(os.path.join(vault, local)) or local.casefold() in taken:
                i += 1
                local = "%s-community-%d" % (name, i)
            _install_folder(b_dir, name, local, base)
            base["renamed"][name] = local
    else:  # keep, merge: the skill in use stays (merged), the waiting copy is set aside and never offered again
        if choice == "merge":
            # a vault skill in use: the merged text is the user's own new skill (a copy of it, as for a shipped one),
            # which wins over the vault copy and which updates never touch; the vault copy is moved to removed/
            target = rbxskills._merge_skill(rbxskills.user_dir(), a, a_dir, a_kit or a_vault, merged_text, name, differ)
            if a_vault:
                try:
                    _set_aside(a_dir, os.path.basename(a_dir), base["waiting"].get(name, ""))
                except BaseException:
                    shutil.rmtree(target, ignore_errors=True)
                    raise
                _forget_vault_skill(base, a_dir)
        if there:
            _set_aside(b_dir, name, base["waiting"].get(name, ""))
        if name not in base["declined"]:
            base["declined"].append(name)
    base["waiting"].pop(name, None)
