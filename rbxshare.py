"""What of the user's skills and Knowledge may be shared with the Fridae vault, and packing a batch to send.
Of the library, only skills the user made are shareable: never an import, never one named like Fridae's, never a near
copy of any skill Fridae can see. Of Knowledge, guides the user changed or added and vault skills they edited
(knowledge_changes). Nothing here sends anything; the app does, on the user's yes."""
import hashlib
import os
import shutil
import time
import uuid

import rbxskills
from rbxskills import SHARED_FILE

LICENCE = "CC-BY-4.0 (text), MIT (scripts)"
COPY_MIN = 0.85          # the keyword cosine of two SKILL.md bodies from which one is a copy of the other
FILE_MAX = 200 * 1024    # bytes of one file
TOTAL_MAX = 1024 * 1024  # bytes of a whole skill
FILES_MAX = 40
DROPPED = ("owner", "core", "supersedes")  # pins that are the sender's own, not the vault's to carry


def _body(d):
    try:
        return rbxskills.front(rbxskills._read(os.path.join(d, "SKILL.md")))[1]
    except OSError:
        return ""


def _corpus(skip):
    """[(id, label, body text, folder)] of everything a skill could be a copy of: Fridae's own and shipped skills, the
    vault's, imported library copies and the import sources (_copies). skip: folders left out (the candidates)."""
    seen, out = {os.path.realpath(d) for d in skip}, []

    def add(sid, origin, d):
        real = os.path.realpath(d)
        if real not in seen:
            seen.add(real)
            out.append((sid, rbxskills.ORIGIN_LABEL.get(origin, origin), _body(d), d))

    for e in rbxskills.kit_skills():
        add(e["id"], e["origin"], e["dir"])
    for d in rbxskills._skill_dirs(rbxskills.vault_dir()):
        if not d.endswith(rbxskills.UPDATE_SUFFIX):
            add(rbxskills._entry(d, "vault", "community")["id"], "vault", d)
    for d in rbxskills._skill_dirs(rbxskills.user_dir()):
        if rbxskills._record(d):
            add(rbxskills._entry(d, "user", "user")["id"], "user", d)
    for c in rbxskills._copies():
        add(c["id"], c["origin"], c["dir"])
    return out


def best_copy(d, corpus=None, idf=None):
    """(score, id, origin label) of the skill a folder's SKILL.md body is most like, by keyword cosine with weights
    over everything compared; (0.0, "", "") when there is nothing to compare."""
    corpus = _corpus([d]) if corpus is None else corpus
    mine = _body(d)
    idf = idf or rbxskills._idf([mine] + [c[2] for c in corpus])
    best = (0.0, "", "")
    for sid, label, text, _dir in corpus:
        s = rbxskills._cosine(mine, text, idf)
        if s > best[0]:
            best = (s, sid, label)
    return best


def _sent(d):
    sent = rbxskills._read_json(os.path.join(d, SHARED_FILE), {}).get("sent")
    return [e for e in sent if isinstance(e, dict)] if isinstance(sent, list) else []


def _has_scripts(files):
    return any(rel.startswith("scripts/") or (os.name != "nt" and os.access(full, os.X_OK)) for rel, full in files)


def _file_problem(files):
    """The reason a skill's files cannot be shared, or None."""
    if len(files) > FILES_MAX or sum(os.path.getsize(f) for _r, f in files if os.path.isfile(f)) > TOTAL_MAX:
        return "Too big to share (over 1 MB or 40 files)."
    for rel, full in files:
        try:
            if os.path.islink(full) or not os.path.isfile(full):
                return "Has a file that is not text: %s." % rel
            with open(full, "rb") as f:
                data = f.read()
            if b"\0" in data:
                return "Has a file that is not text: %s." % rel
            data.decode("utf-8")
        except UnicodeDecodeError:
            return "Has a file that is not text: %s." % rel
        except OSError:
            return "Has a file that is not text: %s." % rel
        if len(data) > FILE_MAX:
            return "%s is over 200 KB." % rel
    return None


def shareable():
    """{"ready": [{id, dir, hash, files, scripts, changed}], "held": [{id, why}], "sent": [{id, pr, at}]}: the
    user's own skills (origin user) by whether they may be sent. Left out entirely: APP_ONLY names, and skills sent
    already with their present hash (listed under sent)."""
    ready, held, sent = [], [], []
    mine = []
    for d in rbxskills._skill_dirs(rbxskills.user_dir()):
        if d.endswith(rbxskills.UPDATE_SUFFIX):
            continue
        e = rbxskills._entry(d, "user", "user")
        if e["id"].casefold() not in rbxskills.APP_ONLY:
            mine.append(e)
    kit = {e["id"].casefold() for e in rbxskills.kit_skills()}
    todo = []
    for e in mine:
        d, name = e["dir"], e["id"]
        files = rbxskills._tree(d)
        h = rbxskills.folder_hash(d)
        marks = [m for m in _sent(d) if m.get("name") == name]
        if any(m.get("hash") == h for m in marks):
            last = [m for m in marks if m.get("hash") == h][-1]
            sent.append({"id": name, "pr": str(last.get("pr") or ""), "at": str(last.get("at") or "")})
            continue
        rec = rbxskills._record(d)
        if rec:
            why = "Imported from %s: it is credited there, not shared." % (rec.get("source") or "another tool")
        elif name.casefold() in kit:
            why = "Has the same name as one of Fridae's skills."
        elif e["licence"] and e["licence"] != LICENCE:
            why = ("Says its licence is %s; only skills you made, under CC BY 4.0 and MIT, can be shared."
                   % e["licence"])
        else:
            why = _file_problem(files)
        if why:
            held.append({"id": name, "why": why})
        else:
            todo.append((e, files, h, bool(marks)))
    if todo:
        corpus = _corpus([t[0]["dir"] for t in todo])
        idf = rbxskills._idf([_body(t[0]["dir"]) for t in todo] + [c[2] for c in corpus])
        for e, files, h, changed in todo:
            score, sid, label = best_copy(e["dir"], corpus, idf)
            if score >= COPY_MIN:
                held.append({"id": e["id"], "why": "Looks like a copy of %s (%s)." % (sid, label)})
            else:
                ready.append({"id": e["id"], "dir": e["dir"], "hash": h, "files": len(files),
                              "scripts": _has_scripts(files), "changed": changed})
    return {"ready": ready, "held": held, "sent": sent}


def _drop_front(text, keys):
    """text with these front matter keys (and their list items or folded lines) taken out."""
    import re
    m = re.match(r"^(﻿?---[ \t]*\r?\n)(.*?)(\r?\n---[ \t]*(?:\r?\n|$))", text, re.S)
    if not m:
        return text
    keep, skipping = [], False
    for line in m.group(2).splitlines():
        km = re.match(r"^([A-Za-z_][\w.-]*):", line)
        if km:
            skipping = km.group(1) in keys
            if skipping:
                continue
        elif skipping and (line[:1] in (" ", "\t") or re.match(r"^-[ \t]", line)):
            continue
        else:
            skipping = False
        keep.append(line)
    return m.group(1) + "\n".join(keep) + m.group(3) + text[m.end():]


def _copy_file(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    tmp = "%s.%s.tmp" % (dst, uuid.uuid4().hex[:8])
    shutil.copyfile(src, tmp)
    os.replace(tmp, dst)


def pack(names, out_dir, login):
    """Write out_dir/Skills/<name>/ for each named skill, ready to send: text files only, no dot-files or SKIP_DIRS,
    its SKILL.md front matter set to the shared licence, source, credit and today's date, and owner, core and
    supersedes taken out. Refuses any name not in shareable()["ready"]. Returns {name: folder_hash of the user's
    folder}."""
    ready = {e["id"]: e for e in shareable()["ready"]}
    bad = [n for n in names if n not in ready]
    if bad:
        raise ValueError("Not ready to share: %s." % ", ".join(bad))
    skills_dir = os.path.join(out_dir, "Skills")
    if os.path.islink(out_dir) or os.path.islink(skills_dir):
        raise OSError("The folder to pack into is a link.")
    os.makedirs(skills_dir, exist_ok=True)
    out = {}
    for name in names:
        e = ready[name]
        final = os.path.join(skills_dir, rbxskills._safe(name))
        tmp = os.path.join(skills_dir, ".tmp-" + uuid.uuid4().hex[:10])
        try:
            for rel, full in rbxskills._tree(e["dir"]):
                if not os.path.islink(full) and os.path.isfile(full):
                    _copy_file(full, os.path.join(tmp, *rel.split("/")))
            path = os.path.join(tmp, "SKILL.md")
            text = _drop_front(rbxskills._read(os.path.join(e["dir"], "SKILL.md")), DROPPED)
            rbxskills._write_text(path, rbxskills.front_with(text, {
                "licence": LICENCE, "source": "shared from Fridae by %s" % login, "credit": login,
                "updated": time.strftime("%Y-%m-%d")}))
            old = None
            if os.path.lexists(final):
                old = os.path.join(skills_dir, ".old-" + uuid.uuid4().hex[:10])
                os.replace(final, old)
            os.replace(tmp, final)
            if old:
                shutil.rmtree(old, ignore_errors=True)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
        out[name] = e["hash"]
    return out


def mark_shared(name, hash, pr_url):
    """Record that this skill, as it was at this hash, went out in this pull request."""
    for d in rbxskills._skill_dirs(rbxskills.user_dir()):
        if d.endswith(rbxskills.UPDATE_SUFFIX) or rbxskills._entry(d, "user", "user")["id"] != name:
            continue
        path = os.path.join(d, SHARED_FILE)
        if os.path.islink(path):
            raise OSError("%s is a link." % SHARED_FILE)
        rec = rbxskills._read_json(path, {})
        sent = [e for e in rec.get("sent", []) if isinstance(e, dict)] if isinstance(rec.get("sent"), list) else []
        sent.append({"name": name, "hash": hash, "pr": pr_url, "at": rbxskills._now()})
        rec["sent"] = sent
        rbxskills._write_json(path, rec)
        return
    raise LookupError("There is no skill called %s in your library." % name)


# ------------------------------------------------------------------ Knowledge: guides and the vault's skills
SHARED_UNITS = "shared.json"  # in rbxvaultsync.state_dir(): {unit: [{hash, pr, at}]}
HELD_BINARY = "Pictures and models cannot be shared yet."
HELD_BIG_NOTE = "Over 200 KB."
HELD_BIG_SKILL = "Too big to share (over 1 MB or 40 files)."
HELD_CLASH = "Decide the vault's change to it first, above."


def _shared_units():
    import rbxvaultsync
    data = rbxskills._read_json(os.path.join(rbxvaultsync.state_dir(), SHARED_UNITS), {})
    return {u: [m for m in v if isinstance(m, dict)] for u, v in data.items() if isinstance(v, list)}


def _note_title(path):
    name = path.rsplit("/", 1)[-1]
    return name[:-3] if name.lower().endswith(".md") and len(name) > 3 else name


def _regular(full):
    return not os.path.islink(full) and os.path.isfile(full)


def _knowledge_units():
    """[(unit, kind, title, local path or folder, new, [(vault path, full path)])] of what in Knowledge differs from
    what the vault gave (by the base record) or is not from the vault at all: notes per file (outside Skills/), and
    each folder of Knowledge/Skills as one skill under its vault (upstream) name."""
    import rbxpaths
    import rbxvaultsync
    know = rbxpaths.KNOWLEDGE_DIR
    base = rbxvaultsync.read_base()
    out = []
    if not os.path.isdir(know):
        return out, base

    def changed(p, full):
        data = rbxvaultsync._read(full)
        return data is None or p not in base["files"] or rbxvaultsync._sha(data) != rbxvaultsync._cmp_hash(base, p)

    for top in sorted(os.listdir(know)):
        top_dir = os.path.join(know, top)
        if top.startswith(".") or top == "Skills" or os.path.islink(top_dir) or not os.path.isdir(top_dir):
            continue
        for root, dirs, files in os.walk(top_dir):
            dirs[:] = sorted(d for d in dirs if not d.startswith(".") and not os.path.islink(os.path.join(root, d)))
            for f in sorted(files):
                full = os.path.join(root, f)
                p = os.path.relpath(full, know).replace(os.sep, "/")
                if not rbxvaultsync.check_path(p) or not _regular(full) or not changed(p, full):
                    continue
                out.append((p, "note", _note_title(p), full, p not in base["files"], [(p, full)]))
    skills_dir = os.path.join(know, "Skills")
    if os.path.isdir(skills_dir) and not os.path.islink(skills_dir):
        upstream_of = {v: k for k, v in base["renamed"].items() if isinstance(v, str)}
        for local in sorted(os.listdir(skills_dir)):
            d = os.path.join(skills_dir, local)
            if local.startswith(".") or os.path.islink(d) or not os.path.isdir(d):
                continue
            name = upstream_of.get(local, local)
            files = [("Skills/%s/%s" % (name, rel), full) for rel, full in rbxskills._tree(d)
                     if _regular(full)]
            files = [(p, full) for p, full in files if rbxvaultsync.check_path(p)]
            if not files or not any(changed(p, full) for p, full in files):
                continue
            new = not any(p.startswith("Skills/%s/" % name) for p in base["files"])
            out.append(("Skills/" + name, "skill", name, d, new, files))
    return out, base


def _unit_hash(kind, where):
    if kind == "skill":
        return rbxskills.folder_hash(where)
    with open(where, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def _held_why(unit, kind, files, base):
    import rbxmerge
    if unit in base["clashes"]:
        return HELD_CLASH
    sizes = [os.path.getsize(full) for _p, full in files]
    if kind == "skill" and (len(files) > FILES_MAX or sum(sizes) > TOTAL_MAX):
        return HELD_BIG_SKILL
    for (p, full), size in zip(files, sizes):
        with open(full, "rb") as f:
            data = f.read(FILE_MAX + 1)
        if rbxmerge.is_binary(p, data[:FILE_MAX]):
            return HELD_BINARY
        if size > FILE_MAX:
            return HELD_BIG_NOTE
    return None


def knowledge_changes():
    """{"ready": [{unit, kind: "note" | "skill", title, hash, new, files}], "held": [{unit, title, why}],
    "sent": [{unit, title, pr, at}]}: what of Knowledge may go to the vault. A note is a file outside Skills/ that
    travels and that the base record does not know or whose bytes differ from what the vault gave; a skill is a folder
    of Knowledge/Skills with a file changed or new, by its vault name (Skills/<upstream name>). Sent: its present hash
    went out already (shared.json). Nobody's own Knowledge is left out, the maintainer's included."""
    ready, held, sent = [], [], []
    units, base = _knowledge_units()
    shared = _shared_units()
    for unit, kind, title, where, new, files in units:
        try:
            h = _unit_hash(kind, where)
        except OSError:
            continue
        marks = [m for m in shared.get(unit, []) if m.get("hash") == h]
        if marks:
            sent.append({"unit": unit, "title": title, "pr": str(marks[-1].get("pr") or ""),
                         "at": str(marks[-1].get("at") or "")})
            continue
        try:
            why = _held_why(unit, kind, files, base)
        except OSError:
            continue
        if why:
            held.append({"unit": unit, "title": title, "why": why})
        else:
            ready.append({"unit": unit, "kind": kind, "title": title, "hash": h, "new": new, "files": len(files)})
    return {"ready": ready, "held": held, "sent": sent}


def pack_knowledge(units, out_dir):
    """Write each ready unit under out_dir as the vault would hold it: a note byte for byte at out_dir/<path>, a skill
    at out_dir/Skills/<upstream name>/ with its SKILL.md front matter name set to that name and nothing else changed.
    Refuses any unit not in knowledge_changes()["ready"]. Returns {unit: hash}."""
    found, _base = _knowledge_units()
    by_unit = {u[0]: u for u in found}
    ready = {e["unit"]: e for e in knowledge_changes()["ready"]}
    bad = [u for u in units if u not in ready]
    if bad:
        raise ValueError("Not ready to share: %s." % ", ".join(bad))
    if os.path.islink(out_dir):
        raise OSError("The folder to pack into is a link.")
    out = {}
    for unit in units:
        _u, kind, title, where, _new, files = by_unit[unit]
        for p, full in files:
            dst = os.path.join(out_dir, *p.split("/"))
            if kind == "skill" and p == unit + "/SKILL.md":
                with open(full, encoding="utf-8", newline="") as f:
                    text = f.read()
                if rbxskills.front(text)[0].get("name") != title:
                    text = _set_name(text, title)
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                rbxskills._write_text(dst, text)
            else:
                _copy_file(full, dst)
        out[unit] = ready[unit]["hash"]
    return out


def _set_name(text, name):
    """text with its front matter's name line set to name in place (every other line as it was); front_with when the
    front matter has no name line."""
    import re
    m = re.match(r"^(﻿?---[ \t]*\r?\n)(.*?)(\r?\n---[ \t]*(?:\r?\n|$))", text, re.S)
    if m:
        front, n = re.subn(r"(?m)^name:[^\r\n]*", lambda _m: "name: %s" % name, m.group(2), count=1)
        if n:
            return m.group(1) + front + m.group(3) + text[m.end():]
    return rbxskills.front_with(text, {"name": name})


def mark_shared_unit(unit, hash, pr_url):
    """Record that this unit of Knowledge, as it was at this hash, went out in this pull request."""
    import rbxvaultsync
    path = os.path.join(rbxvaultsync.state_dir(), SHARED_UNITS)
    if os.path.islink(path) or os.path.islink(rbxvaultsync.state_dir()):
        raise OSError("%s is a link." % SHARED_UNITS)
    rec = _shared_units()
    rec.setdefault(unit, []).append({"hash": hash, "pr": pr_url, "at": rbxskills._now()})
    os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
    rbxskills._write_json(path, rec)


def _folder_name(d):
    return os.path.basename(os.path.normpath(d))


def compare(new_dir, against):
    """How a submitted skill stands next to the vault's: {"same_name", "closest": {"id", "score", "dir"} | None,
    "diff"}. `against` is the folder of the vault's skills (<against>/<name>/SKILL.md). same_name: the vault has a skill
    of the same name; diff is then the unified diff of its SKILL.md against the submission's (vault first), else None.
    closest is the skill of `against` (the vault's, never the user's own or Fridae's) the submission's body is most
    like, by the keyword cosine used for copies; None when there is nothing to compare.

    A note: `new_dir` a file and `against` the vault's file of that path. same_name is whether the vault has it, closest
    is None, and diff is the unified diff of the vault's file against the submission's (line endings aside), else None."""
    import difflib
    if os.path.isfile(new_dir):
        name = os.path.basename(new_dir)
        same = os.path.isfile(against)
        diff = None
        if same:
            def note_lines(p):
                return rbxskills._read(p).replace("\r\n", "\n").splitlines(keepends=True)
            diff = "".join(difflib.unified_diff(note_lines(against), note_lines(new_dir), "vault/%s" % name,
                                                "submission/%s" % name))
        return {"same_name": same, "closest": None, "diff": diff}
    name = _folder_name(new_dir)
    key = name.casefold()
    theirs = rbxskills._skill_dirs(against)
    corpus = [(_folder_name(d), "vault", _body(d), d) for d in theirs
              if os.path.realpath(d) != os.path.realpath(new_dir)]
    mine = _body(new_dir)
    idf = rbxskills._idf([mine] + [c[2] for c in corpus])
    closest = None
    for sid, _label, text, d in corpus:
        s = rbxskills._cosine(mine, text, idf)
        if s > 0 and (closest is None or s > closest["score"]):
            closest = {"id": sid, "score": s, "dir": d}
    same = next((d for d in theirs if _folder_name(d).casefold() == key), None)
    diff = None
    if same:
        def lines(d):
            return rbxskills._read(os.path.join(d, "SKILL.md")).replace("\r\n", "\n").splitlines(keepends=True)
        diff = "".join(difflib.unified_diff(lines(same), lines(new_dir), "vault/%s/SKILL.md" % name,
                                            "submission/%s/SKILL.md" % name))
    return {"same_name": bool(same), "closest": closest, "diff": diff}
