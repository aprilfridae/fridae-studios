"""Ask the desktop app to start extra agents, and collect what they report.

The app watches the work folder. This module only writes requests and reads
results; the app does the rest.

  request   WORK/team-requests/<id>.json   written here (as .tmp, then renamed)
            {"id", "from", "role": "worker" | "critic", "name", "task", "created"}
            plus, when an expert from the Studio roster was asked for (rbxroster, filled in here, not by the app):
            "expert": id, "expert_name", "expert_text", "writes": bool; "role" is then the expert's kind
  result    WORK/team-results/<id>.json    written and updated by the app
            {"id", "agent", "name", "role", "status": "running" | "done" | "refused", "reason", "text"}
            and, from spawn, where it runs: "ai", "model" and "note" (when that is not what was asked for, and why)
  models    WORK/team-models.json          kept up to date by the app: what each AI offers
            {"lead_may_pick", "ais": [{"id", "name", "installed", "default", "models": [ids], "note"}]}

A worker does a piece of the job; a critic reviews work and reports problems.
Agents started this way cannot start more.
"""
import os, random, time

from rbxpaths import WORK, KitError, read_json, write_json

REQUESTS = os.path.join(WORK, "team-requests")
RESULTS = os.path.join(WORK, "team-results")
NUDGES = os.path.join(WORK, "team-nudge")  # WORK/team-nudge/<agent id>: the app has something waiting for that Lead
MODELS = os.path.join(WORK, "team-models.json")
MODELS_SHOWN = 40  # of one AI's models, when every AI is listed (Cursor offers hundreds)
ROLES = ("worker", "critic")
FINISHED = ("done", "refused")
PICKUP = 15       # seconds to wait for the app to answer a spawn
MAX_WAIT = 840
LEAD_WAIT = 120   # a studio run's Lead waits at most this long at a time, so messages and decisions reach it
RUN_ENDED = "This game's studio run was ended."


class TeamError(KitError):
    pass


def _result(rid):
    data = read_json(os.path.join(RESULTS, rid + ".json"))  # None while missing, or caught halfway through being written
    return data if isinstance(data, dict) else None


def _safe(rid):
    rid = str(rid or "").strip()
    if not rid or any(c in rid for c in "/\\:.*?\"<>|"):
        raise TeamError("\"%s\" is not a task id. Ids look like w1731850000000abcd and come from spawn." % rid)
    return rid


def _my_run():
    """The studio run this chat leads ({"on", "paused", "ended", ...} from <project>/.fridae/studio.json), or None
    when it leads none (no project, no run, or another chat's run)."""
    me = (os.environ.get("RBXKIT_AGENT_ID") or "").strip()
    project = (os.environ.get("RBXKIT_PROJECT") or "").strip()
    if not me or not project:
        return None
    run = read_json(os.path.join(project, ".fridae", "studio.json"))
    return run if isinstance(run, dict) and run.get("lead") == me else None


def _ended(run):
    return bool(run) and run.get("on") is not True and isinstance(run.get("ended"), (int, float)) \
        and not isinstance(run.get("ended"), bool)


def spawn(role, name, task, expert=None, ai=None, model=None):
    mine = (os.environ.get("RBXKIT_AGENT_ROLE") or "").strip().lower()
    if mine in ROLES:
        raise TeamError("You were started as a %s by another agent, and such agents may not start more. Do the task "
                        "yourself and say in your reply what else is needed." % mine)
    entry = None
    expert = str(expert or "").strip().lower()
    if expert and _ended(_my_run()):
        raise TeamError(RUN_ENDED + " Start no more experts: finish what you are doing and report.")
    if expert:
        import rbxroster  # lazy: only Studio mode needs the roster
        entry = rbxroster.expert(expert, os.environ.get("RBXKIT_PROJECT") or None)  # KitError lists the valid ids
        if entry["kind"] not in ROLES:
            raise TeamError("The %s is the game's own chat, not a helper, so it cannot be started with rbx_team. Pick another "
                            "expert." % entry["name"])
        role = entry["kind"]  # the roster decides what the helper is started as
    role = str(role or "").strip().lower()
    if role not in ROLES:
        raise TeamError("Give 'role': \"worker\" (does a piece of the job) or \"critic\" (reviews work and reports problems).")
    name = " ".join(str(name or "").split())
    if not name:
        raise TeamError("Give 'name': a short title for the agent, such as \"Shop UI\" (34 characters at most).")
    task = str(task or "").strip()
    if len(task) < 80:
        raise TeamError("The task is too short (%d characters; at least 80). The new agent starts knowing nothing: say what "
                        "to do, where (instance paths, file names), what is already done, what to leave alone, and what to "
                        "report back." % len(task))
    rid = "w%d%s" % (int(time.time() * 1000), "".join(random.choice("abcdefghjkmnpqrstuvwxyz23456789") for _ in range(4)))
    req = {"id": rid, "from": os.environ.get("RBXKIT_AGENT_ID") or "", "role": role, "name": name[:34], "task": task,
           "created": time.time()}
    # where it runs (optional): the app checks it, and the user's Setup > Experts table wins over it
    ai = str(ai or "").strip().lower()
    model = str(model or "").strip()
    if ai:
        req["ai"] = ai
    if model:
        req["model"] = model
    if entry:
        req.update({"expert": entry["id"], "expert_name": entry["name"], "expert_text": entry["text"],
                    "writes": bool(entry["writes"])})
    os.makedirs(REQUESTS, exist_ok=True)
    path = os.path.join(REQUESTS, rid + ".json")
    write_json(path, req)
    deadline = time.time() + PICKUP
    while time.time() < deadline:
        res = _result(rid)
        if res:
            return rid, res
        time.sleep(0.25)
    try:
        os.remove(path)  # nobody took it: do not leave it to start an agent later, unannounced
    except OSError:
        pass
    return rid, None


def known():
    """Every task that has a result file, oldest first."""
    try:
        names = os.listdir(RESULTS)
    except OSError:
        return []
    out = []
    for n in names:
        res = _result(n[:-5]) if n.endswith(".json") else None
        if res:
            res.setdefault("id", n[:-5])
            out.append(res)
    return sorted(out, key=lambda r: str(r.get("id")))


def _take_nudge():
    """What the app has waiting for this chat ({"typed": n, "wakes": n}), taken so the next wait does not return
    for it again; None when there is nothing."""
    me = (os.environ.get("RBXKIT_AGENT_ID") or "").strip()
    if not me or any(c in me for c in "/\\:."):
        return None
    path = os.path.join(NUDGES, me)
    if not os.path.exists(path):
        return None
    data = read_json(path)
    try:
        os.remove(path)
    except OSError:
        pass
    return data if isinstance(data, dict) else {}


def _early(run, since):
    """Why a studio Lead's wait (begun at `since`) should return now, or None. A run that ended before the wait began is
    no run at all: the former Lead's chat then waits on its helpers like any chat."""
    if run is not None:
        if _ended(run):
            return "ended" if run["ended"] >= since else None
        if run.get("on") is True and run.get("paused") is True:
            return "paused"
    nudge = _take_nudge() if run is not None and run.get("on") is True else None
    if nudge is not None:
        return "message" if nudge.get("typed") else "decision"
    return None


def wait(ids, timeout=600):
    """Wait for the tasks to finish; returns ({id: result}, why) where `why` is None (all finished, or the time ran
    out) or why a studio run's Lead was handed back early: "ended", "paused", "message", "decision" or "slice"
    (LEAD_WAIT passed). Callers that lead no studio run wait as before."""
    ids = [_safe(i) for i in ids or []]
    if not ids:
        raise TeamError("Give 'ids': the task ids to wait for (spawn returns them).")
    timeout = max(1, min(MAX_WAIT, int(timeout or 600)))
    began = time.time()
    first = _my_run()
    lead = first is not None and not _ended(first)  # an ended run's former Lead waits as before
    slice_end = began + LEAD_WAIT if lead else None
    deadline = began + timeout
    while True:
        res = {i: _result(i) for i in ids}
        if all(r and r.get("status") in FINISHED for r in res.values()) or time.time() >= deadline:
            return res, None
        if lead:
            why = _early(_my_run(), began)
            if why:
                return res, why
            if time.time() >= slice_end:
                return res, "slice"
        time.sleep(0.5)


EARLY = {
    "ended": "the studio run was ended in Review. Start nothing new: finish up and report",
    "paused": "the studio run was paused in Review. Hand out nothing new; finish what you are doing and report",
    "message": "the user sent you a message. It reaches you when this turn ends, so end the turn with a short note "
               "(your experts keep working; wait for them again next turn)",
    "decision": "a decision or other news is waiting for you. Call rbx_stage action 'decisions', act on it, then wait again",
    "slice": "a studio Lead waits at most %d seconds at a time, so messages and decisions reach it. Call rbx_stage "
             "action 'decisions', act on anything new, then wait again" % LEAD_WAIT,
}


def models(ai=None):
    """What each AI offers, as text: whether it is installed, its default model and its model ids, with what the app
    knows about why an AI has few (OpenCode that is not signed in). `ai` lists that one in full."""
    data = read_json(MODELS)
    ais = data.get("ais") if isinstance(data, dict) else None
    if not isinstance(ais, list) or not ais:
        raise TeamError("The app has not listed its AIs' models. This only works when this chat runs inside the Fridae "
                        "Studios app with it open; if it is, try again in a few seconds.")
    # only entries shaped as the app writes them (the file is the app's, but nothing here trusts its shape)
    ais = [a for a in ais if isinstance(a, dict) and isinstance(a.get("id"), str) and a["id"]]
    if not ais:
        raise TeamError("The app's list of models could not be read. Try again in a few seconds.")
    known = ", ".join(a["id"] for a in ais)
    want = str(ai or "").strip().lower()
    if want:
        ais = [a for a in ais if a["id"] == want]
        if not ais:
            raise TeamError("\"%s\" is not an AI this app runs agents on. Use one of %s, or leave ai out to list them all."
                            % (want, known))
    out = ["Where an agent can run (rbx_team spawn's 'ai' and 'model'; leave both out and it runs like you). Model ids "
           "are given exactly as written here."]
    if data.get("lead_may_pick") is False:
        out.append("The user has switched off \"The Lead may pick an AI for a job\" in Setup, so leave ai and model out: "
                   "this list is for your information only.")
    for a in ais:
        head = "%s (%s): " % (a["id"], a["name"] if isinstance(a.get("name"), str) and a["name"] else a["id"])
        ids = [m for m in a["models"] if isinstance(m, str)] if isinstance(a.get("models"), list) else []
        if not a.get("installed"):
            out.append(head + "not installed")
        elif not ids:
            out.append(head + "its model list has not arrived yet (a model you name is passed on as it is)")
        else:
            shown = ids if want or len(ids) <= MODELS_SHOWN else ids[:MODELS_SHOWN]
            more = "" if len(shown) == len(ids) else " ... and %d more (give ai \"%s\" to see them all)" % (len(ids) - len(shown), a["id"])
            default = a["default"] if isinstance(a.get("default"), str) and a["default"] else "its own choice"
            out.append("%sdefault model %s. Models: %s%s" % (head, default, ", ".join(shown), more))
        if isinstance(a.get("note"), str) and a["note"]:
            out.append("  note: %s" % a["note"])
    return "\n".join(out)


def _line(rid, r):
    if not r:
        return "%s  unknown: the app has no record of this task" % rid
    return "%s  %-8s %-7s %s%s" % (rid, r.get("status") or "?", r.get("role") or "", r.get("name") or "",
                                   "  (%s)" % r["reason"] if r.get("status") == "refused" and r.get("reason") else "")


def run(action, a=None):
    """Do one action and return the text to show. Raises TeamError with a plain message."""
    a = a or {}
    action = (action or "status").lower()
    if action == "spawn":
        rid, res = spawn(a.get("role"), a.get("name"), a.get("task"), expert=a.get("expert"), ai=a.get("ai"),
                         model=a.get("model"))
        if res is None:
            return ("NOT STARTED. The app did not pick up the request within %d seconds, so it was withdrawn. Extra agents "
                    "only work when this chat runs inside the Fridae Studios app with it open. Carry on by yourself." % PICKUP)
        status = res.get("status")
        if status == "refused":
            return "REFUSED: %s\nThe agent was not started. Carry on by yourself." % (res.get("reason") or "the app gave no reason")
        where = ""
        if res.get("ai"):
            where = "\nruns on: %s%s" % (res["ai"], (" (model %s)" % res["model"]) if res.get("model") else " (its default model)")
        if res.get("note"):
            where += "\nnote: %s" % res["note"]
        return ("id: %s\nstatus: %s%s\n%s \"%s\" was started. Keep working on your own part, then collect its report with "
                "rbx_team action 'wait' and ids [\"%s\"]." % (rid, status, where, (res.get("role") or a.get("role") or "agent").capitalize(),
                                                            res.get("name") or a.get("name"), rid))
    if action == "status":
        ids = [_safe(i) for i in a.get("ids") or []]
        rows = [(i, _result(i)) for i in ids] if ids else [(str(r.get("id")), r) for r in known()]
        if not rows:
            return "No tasks yet. Start one with action 'spawn'."
        return "id                     status   role    name\n" + "\n".join(_line(i, r) for i, r in rows)
    if action == "wait":
        t0 = time.time()
        res, why = wait(a.get("ids"), a.get("timeout"))
        left = [i for i, r in res.items() if not r or r.get("status") not in FINISHED]
        out = []
        for i, r in res.items():
            out.append(_line(i, r))
            if r and r.get("status") == "done":
                out.append((r.get("text") or "(the agent finished without a reply)").strip())
            out.append("")
        if left and why:
            out.append("Returned early after %d seconds: %s. Still running: %s. They have not failed and keep working; "
                       "collect them with wait later." % (time.time() - t0, EARLY[why], ", ".join(left)))
        elif left:
            out.append("Still not finished after %d seconds: %s. Call wait again, or carry on and check later with 'status'."
                       % (time.time() - t0, ", ".join(left)))
        else:
            out.append("All finished. Check the work yourself before reporting it as done.")
        return "\n".join(out)
    if action == "models":
        return models(a.get("ai"))
    raise TeamError("Unknown action '%s'. Use spawn, status, wait or models." % action)
