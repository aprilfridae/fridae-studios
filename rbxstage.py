"""Studio mode, the agents' side: the run, review items for the user to sign off, decisions and lessons.

Everything is small JSON in <project>/.fridae/ (studio.json, review/<id>.json, review/<id>/ media), so it travels
with the game; lessons are the user's notes (rbxnotes.py, .fridae/notes/). The Fridae Studios app shows the review
items; agents use rbx_stage. Nothing here prints: functions return text, because the MCP server speaks JSON-RPC over
stdout.

A finished stage also asks the app for a checkpoint: WORK/stage-requests/<id>.json {"project", "stage", "lead"}. The
app takes it and appends {"stage", "commit", "studio", "at"} to studio.json "checkpoints" (see app-rs/src/studio_undo.rs).
"""
import json
import os
import shutil
import time
from contextlib import contextmanager
import uuid

import rbxpaths
import rbxroster
from rbxpaths import KitError

STAGES = ["idea", "design", "build", "test", "launch", "promote"]
DEFAULT_GATES = ["design", "launch"]
MEDIA_EXT = (".png", ".jpg", ".jpeg", ".webp", ".mp4", ".mov")
MAX_MEDIA = 8
MAX_MEDIA_BYTES = 200 * 1024 * 1024
OPEN = ("waiting", "changes")


def _dir(project):
    return os.path.join(project, ".fridae")


def _write_atomic(path, write):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.%d.%s.tmp" % (path, os.getpid(), uuid.uuid4().hex[:8])
    try:
        write(tmp)
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


LOCK_WAIT = 5.0
LOCK_STALE = 10.0


def lock_file(project):
    return os.path.join(_dir(project), ".lock")


@contextmanager
def locked(project):
    """One lock per project for every read-modify-write of studio.json and review items.

    The lock is <project>/.fridae/.lock, created exclusively and holding the pid and time. It is retried for about
    5 seconds, taken over when older than 10 seconds, and always removed on the way out. Not re-entrant.
    """
    path = lock_file(project)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    deadline = time.time() + LOCK_WAIT
    while True:
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except (FileExistsError, PermissionError) as e:
            # PermissionError: on Windows a lock file that is being deleted cannot be opened yet; wait like for a held one
            try:
                if time.time() - os.stat(path).st_mtime > LOCK_STALE:
                    os.remove(path)
                    continue
            except OSError:
                if isinstance(e, FileExistsError):
                    continue  # it vanished between the two calls: try again at once
                if time.time() > deadline:  # no lock file, and still denied: the folder cannot be written
                    raise KitError("Could not lock the game's Studio files: %s." % e)
                time.sleep(0.02)
                continue
            if time.time() > deadline:
                raise KitError("Another Fridae Studios process is updating this game's Studio files. Try again in a moment.")
            time.sleep(0.02)
    try:
        with os.fdopen(fd, "w") as f:
            f.write("%d %f" % (os.getpid(), time.time()))
        yield
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def _write_json(path, data):
    def w(tmp):
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1, ensure_ascii=False)
    _write_atomic(path, w)


def _read_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else None
    except (OSError, ValueError):
        return None


def _new_run():
    return {"on": False, "stage": "", "stages": [{"id": s, "status": "todo", "started": 0, "ended": 0} for s in STAGES],
            "gates": list(DEFAULT_GATES), "max_experts": 2, "lead": "", "idea": "", "started": 0, "paused": False,
            "read_decisions": []}


def run_file(project):
    return os.path.join(_dir(project), "studio.json")


def load_run(project):
    run = _new_run()
    data = _read_json(run_file(project))
    if data:
        run.update(data)
        have = {s.get("id") for s in run["stages"] if isinstance(s, dict)}
        run["stages"] = [s for s in run["stages"] if isinstance(s, dict)] + \
            [{"id": s, "status": "todo", "started": 0, "ended": 0} for s in STAGES if s not in have]
    return run


def save_run(project, run):
    _write_json(run_file(project), run)


def _review_dir(project):
    return os.path.join(_dir(project), "review")


def _check_id(item_id):
    item_id = (item_id or "").strip()
    if not item_id or item_id in (".", "..") or any(c in item_id for c in "/\\\0") or os.path.basename(item_id) != item_id:
        raise KitError("%r is not a review item id. Call with action 'status' to see the ids." % item_id)
    return item_id


def _item_path(project, item_id):
    return os.path.join(_review_dir(project), _check_id(item_id) + ".json")


def load_items(project):
    items = []
    try:
        names = sorted(os.listdir(_review_dir(project)))
    except OSError:
        return items
    for n in names:
        if n.endswith(".json"):
            d = _read_json(os.path.join(_review_dir(project), n))
            if d and d.get("id"):
                items.append(d)
    items.sort(key=lambda i: i.get("created") or 0)
    return items


def _find_project(a):
    project = (a.get("project") or os.environ.get("RBXKIT_PROJECT") or "").strip()
    if not project or not os.path.isdir(project):
        raise KitError("This chat has no game project, so there is no Studio run. Studio mode belongs to project folders.")
    return project


def _roster_names(project):
    return ["%s [%s, %s%s]" % (e["name"] or e["id"], e["id"], e["stage"], ", changes the place" if e["writes"] else "")
            for e in rbxroster.load(project)]


def _unread(project, run, items):
    read = set(run.get("read_decisions") or [])
    return [i for i in items if i.get("status") in ("approved", "changes") and "%s:%s" % (i["id"], i["status"]) not in read]


def _status(project):
    run = load_run(project)
    items = load_items(project)
    lines = []
    if not run.get("on") and not run.get("started"):
        lines.append("No run yet. Start the first stage with action 'stage'.")
    else:
        lines.append("Run: %s%s. Current stage: %s." % ("on" if run.get("on") else "off",
                                                      ", paused" if run.get("paused") else "", run.get("stage") or "none"))
    lines.append("Stages: " + "  ".join("%s (%s)" % (s["id"], s.get("status", "todo")) for s in run["stages"]))
    lines.append("Experts at once (max_experts): %s. Gates (the run waits for you to approve): %s" % (
        run.get("max_experts") or 2, ", ".join(run.get("gates") or []) or "none"))
    roster = _roster_names(project)
    if roster:
        lines.append("Team (name [id for rbx_team, stage]): " + ", ".join(roster))
    open_items = [i for i in items if i.get("status") in OPEN]
    lines.append("Open review items (%d):" % len(open_items) if open_items else "No open review items.")
    read = set(run.get("read_decisions") or [])
    for i in open_items:
        line = "  [%s] %s (%s%s, %s)" % (i["id"], i.get("title", ""), i.get("stage", ""),
                                         ", blocking" if i.get("blocking") else "", i.get("status"))
        if i.get("status") == "changes":
            # the user's note stays here until the redo is filed, so a turn that failed after 'decisions' loses nothing
            line += ". Note: %s%s" % (i.get("note") or "(no note)",
                                      " (already handed over by 'decisions')" if "%s:changes" % i["id"] in read else "")
        lines.append(line)
    unread = _unread(project, run, items)
    if unread:
        lines.append("%d decision%s not read yet (call 'decisions'):" % (len(unread), "" if len(unread) == 1 else "s"))
        lines += ["  [%s] %s: %s" % (i["id"], i.get("title", ""), i["status"]) for i in unread]
    return "\n".join(lines)


def _stage(project, a):
    sid = (a.get("id") or "").strip().lower()
    if sid not in STAGES:
        raise KitError("Stage id must be one of: " + ", ".join(STAGES) + ".")
    status = (a.get("status") or "").strip().lower()
    if status not in ("start", "done"):
        raise KitError("status must be 'start' or 'done'.")
    with locked(project):
        return _stage_locked(project, sid, status)


def _stage_locked(project, sid, status):
    run = load_run(project)
    if run.get("ended") and not run.get("on"):
        raise KitError("This studio run was ended in the app's Review tab, so its stages no longer change. Stop here; a "
                       "new run starts from the app.")
    entry = [s for s in run["stages"] if s["id"] == sid][0]
    now = time.time()
    if status == "start":
        run["on"] = True
        run["stage"] = sid
        run["started"] = run.get("started") or now
        entry.update(status="doing", started=now, ended=0)
        msg = "Stage %s started." % sid
    else:
        if sid in (run.get("gates") or []):
            mine = _supersede_old_changes(project, sid)
            if not any(i.get("status") == "approved" for i in mine) or any(i.get("status") in OPEN for i in mine):
                msg = ("The %s stage is a gate: the user has to approve its blocking review item first, and none of its "
                       "blocking items may still be waiting or need changes. File one with action 'review' "
                       "(blocking true), wait for the decision, then finish the stage." % sid)
                for i in mine:
                    if i.get("status") == "changes":
                        msg += (" [%s] %s still has changes wanted: file the redo with action 'review', blocking true and "
                                "replaces=%s." % (i["id"], i.get("title", ""), i["id"]))
                raise KitError(msg)
        entry.update(status="done", ended=now)
        msg = "Stage %s done." % sid
    save_run(project, run)
    if status == "done":
        _request_checkpoint(project, sid, run.get("lead") or os.environ.get("RBXKIT_AGENT_ID") or "")
    return msg


def _supersede_old_changes(project, sid):
    """The stage's blocking items that count, after a newer approved blocking item of the stage has superseded every
    older one still marked "changes" (a redo filed without `replaces`). Call under the lock."""
    mine = [i for i in load_items(project) if i.get("stage") == sid and i.get("blocking") and i.get("status") != "superseded"]
    approved = [i for i in mine if i.get("status") == "approved"]
    keep = []
    for i in mine:
        newer = [a for a in approved if (a.get("created") or 0) > (i.get("created") or 0)]
        if i.get("status") == "changes" and newer:
            i["status"] = "superseded"
            i["superseded_by"] = newer[-1]["id"]
            _write_json(_item_path(project, i["id"]), i)
            continue
        keep.append(i)
    return keep


def _request_checkpoint(project, stage, lead):
    """Ask the app for a checkpoint of the game as it is now, so "Undo this run" can come back here. The app takes it
    (git commit of the project, Studio's script snapshot) and adds it to studio.json "checkpoints". Nothing waits for
    it: when the app is not running, or the work folder cannot be written, the stage is done all the same."""
    folder = os.path.join(rbxpaths.WORK, "stage-requests")
    rid = "s%d%s" % (int(time.time() * 1000), uuid.uuid4().hex[:6])
    try:
        _write_json(os.path.join(folder, rid + ".json"), {"project": project, "stage": stage, "lead": lead})
        return True
    except OSError:
        return False


def _copy_media(project, item_id, paths):
    kept, skipped = [], []
    folder = os.path.join(_review_dir(project), item_id)
    for p in paths or []:
        p = os.path.expanduser(str(p))
        name = os.path.basename(p)
        if os.path.splitext(name)[1].lower() not in MEDIA_EXT:
            skipped.append("%s (only %s files)" % (name or p, ", ".join(e[1:] for e in MEDIA_EXT)))
            continue
        try:
            st = os.stat(p)
        except OSError:
            skipped.append("%s (file not found)" % name)
            continue
        if not os.path.isfile(p):
            skipped.append("%s (not a file)" % name)
            continue
        if st.st_size > MAX_MEDIA_BYTES:
            skipped.append("%s (over %d MB)" % (name, MAX_MEDIA_BYTES // 1024 // 1024))
            continue
        if len(kept) >= MAX_MEDIA:
            skipped.append("%s (only %d files per item)" % (name, MAX_MEDIA))
            continue
        base, ext = os.path.splitext(name)
        final, n = name, 1
        while final in kept:
            n += 1
            final = "%s-%d%s" % (base, n, ext)
        dest = os.path.join(folder, final)
        try:
            _write_atomic(dest, lambda tmp, p=p: shutil.copyfile(p, tmp))
        except OSError as e:
            skipped.append("%s (could not copy: %s)" % (name, e.strerror or e))
            continue
        kept.append(final)
    return kept, skipped


def _refuse_approved(old, a):
    """Replacing an item the user already approved would make them approve twice: refused unless `force`."""
    if old.get("status") == "approved" and not a.get("force"):
        raise KitError("Not filed: [%s] %s was already approved by the user, so it is not replaced. Call 'decisions' and "
                       "carry on from the approval; record small additions in GAME.md's decision log. Give 'force' true "
                       "only when the plan itself must change." % (old.get("id") or "?", old.get("title") or ""))


def _review(project, a):
    title = (a.get("title") or "").strip()
    if not title:
        raise KitError("Give the review item a short title.")
    replaces = (a.get("replaces") or "").strip()
    old = None
    if replaces:
        old = _read_json(_item_path(project, replaces))
        if not old:
            raise KitError("No review item with id %r to replace." % replaces)
        _refuse_approved(old, a)
    run = load_run(project)
    item_id = uuid.uuid4().hex[:8]
    kept, skipped = _copy_media(project, item_id, a.get("media"))
    now = time.time()
    item = {"id": item_id, "stage": (a.get("stage") or run.get("stage") or "").strip(), "expert": (a.get("expert") or "").strip(),
            "title": title[:120], "summary": (a.get("summary") or "").strip()[:4000],
            # the app's plain-English line for the user ("In plain words:"), passed on from the expert's result
            "plain": (a.get("plain") or "").strip()[:4000], "media": kept,
            "blocking": bool(a.get("blocking")), "status": "waiting", "note": "", "created": now, "decided": 0,
            "replaces": replaces, "by_chat": os.environ.get("RBXKIT_AGENT_ID") or ""}
    note = ""
    with locked(project):
        if old:
            try:
                _refuse_approved(_read_json(_item_path(project, replaces)) or {}, a)  # approved while the media copied
            except KitError:
                shutil.rmtree(os.path.join(_review_dir(project), item_id), ignore_errors=True)
                raise
        _write_json(_item_path(project, item_id), item)
        if old:
            cur = _read_json(_item_path(project, replaces))  # re-read: the user may have decided it meanwhile
            if cur and cur.get("status") in OPEN:
                cur["status"] = "superseded"
                cur["superseded_by"] = item_id
                _write_json(_item_path(project, replaces), cur)
            elif cur:
                note = " The old item [%s] was already %s, so it was left as it is." % (replaces, cur.get("status"))
    msg = "Filed review item [%s] %s%s." % (item_id, title, " (blocking)" if item["blocking"] else "")
    if replaces:
        msg += " It replaces [%s]." % replaces + note
    if skipped:
        msg += "\nNot attached: " + "; ".join(skipped) + "."
    return msg


def _decisions(project):
    with locked(project):
        return _decisions_locked(project)


def _decisions_locked(project):
    run = load_run(project)
    new = _unread(project, run, load_items(project))
    if not new:
        return "No new decisions."
    lines = []
    for i in new:
        if i["status"] == "approved":
            lines.append("[%s] %s: approved.%s" % (i["id"], i.get("title", ""), " Note: " + i["note"] if i.get("note") else ""))
        else:
            lines.append("[%s] %s: changes wanted. %s" % (i["id"], i.get("title", ""), i.get("note") or "(no note)"))
    run["read_decisions"] = list(run.get("read_decisions") or []) + ["%s:%s" % (i["id"], i["status"]) for i in new]
    save_run(project, run)
    return "\n".join(lines)


def _lesson(project, a):
    """A lesson is one of the user's notes (rbxnotes): this game's .fridae/notes/, or the notes for everything."""
    import rbxnotes
    scope = (a.get("scope") or "game").strip().lower()
    if scope not in ("game", "all"):
        raise KitError("scope must be 'game' or 'all'.")
    title, _ = rbxnotes.lesson(a.get("text"), scope, project)
    return ("Lesson saved for %s as the note '%s'. Notes are searchable: rbx_vault action 'search' with shelf '%s'." % (
        "this game" if scope == "game" else "all games", title, "game" if scope == "game" else "yours"))


def run(action, a):
    project = _find_project(a)
    if action == "status":
        return _status(project)
    if action == "stage":
        return _stage(project, a)
    if action == "review":
        return _review(project, a)
    if action == "decisions":
        return _decisions(project)
    if action == "lesson":
        return _lesson(project, a)
    raise KitError("action must be status, stage, review, decisions or lesson.")


def _cmd(args):
    a = {"project": args.project, "id": args.id, "status": args.state, "title": args.title, "summary": args.summary,
         "plain": args.plain, "media": args.media, "expert": args.expert, "blocking": args.blocking,
         "replaces": args.replaces, "text": args.text, "scope": args.scope}
    print(run(args.action, a))


def register(sub):
    g = sub.add_parser("stage", help="Studio mode: the run, review items, decisions and lessons")
    g.add_argument("action", choices=["status", "stage", "review", "decisions", "lesson"])
    g.add_argument("--project", help="the game's project folder (default: RBXKIT_PROJECT)")
    g.add_argument("--id", help="stage: idea, design, build, test, launch or promote")
    g.add_argument("--state", choices=["start", "done"], help="stage: start or finish it")
    g.add_argument("--title")
    g.add_argument("--summary")
    g.add_argument("--plain", help="review: the plain-English line from the expert's result, for the user")
    g.add_argument("--media", nargs="*", help="review: picture or video files to attach")
    g.add_argument("--expert")
    g.add_argument("--blocking", action="store_true")
    g.add_argument("--replaces", help="review: id of the item this one replaces")
    g.add_argument("--text", help="lesson: the sentence to keep")
    g.add_argument("--scope", choices=["game", "all"], default="game")
    g.set_defaults(fn=_cmd)
