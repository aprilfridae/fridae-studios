"""The game's board: what is being worked on, what is done, and what the user needs to do.

One board per game project, kept in <project>/.fridae/board.json so it travels with the game (and its git history).
The Fridae Studios app shows it as a board the user can move cards on; agents read and update it with rbx_board.
"""
import json
import os
import time
import uuid

from rbxpaths import KitError

COLUMNS = {"todo": "To do", "doing": "Working on", "needs": "Needs you", "done": "Done"}
ALIASES = {"to do": "todo", "backlog": "todo", "working": "doing", "in progress": "doing", "working on": "doing",
           "needs you": "needs", "needs_you": "needs", "blocked": "needs", "finished": "done"}


def _column(name, default):
    c = (name or default or "").strip().lower()
    c = ALIASES.get(c, c)
    if c not in COLUMNS:
        raise KitError("column must be one of: " + ", ".join(COLUMNS) + ".")
    return c


def board_file(project):
    return os.path.join(project, ".fridae", "board.json")


def load(project):
    try:
        with open(board_file(project), encoding="utf-8") as f:
            data = json.load(f)
        if isinstance(data, dict) and isinstance(data.get("cards"), list):
            return data
    except (OSError, ValueError):
        pass
    return {"cards": []}


def save(project, data):
    path = board_file(project)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=1)
    os.replace(tmp, path)


def _find(data, card_id):
    card_id = (card_id or "").strip()
    for c in data["cards"]:
        if c["id"] == card_id:
            return c
    raise KitError("No card with id %r. Call with action 'list' to see the ids." % card_id)


def _show(data):
    lines = []
    for col, title in COLUMNS.items():
        cards = [c for c in data["cards"] if c.get("column") == col]
        if col == "done":
            cards = cards[-12:]
        lines.append("%s (%d)" % (title, len([c for c in data["cards"] if c.get("column") == col])))
        for c in cards:
            lines.append("  [%s] %s%s%s" % (c["id"], c["title"], " - " + c["note"] if c.get("note") else "",
                                          "  (" + c["by"] + ")" if c.get("by") else ""))
    return "\n".join(lines)


def run(action, a):
    project = (a.get("project") or os.environ.get("RBXKIT_PROJECT") or "").strip()
    if not project or not os.path.isdir(project):
        raise KitError("This chat has no game project, so there is no board. Boards belong to project folders.")
    data = load(project)
    me = os.environ.get("RBXKIT_AGENT_NAME") or "agent"
    chat = os.environ.get("RBXKIT_AGENT_ID") or ""
    now = time.time()
    if action == "list":
        return _show(data)
    if action == "add":
        title = (a.get("title") or "").strip()
        if not title:
            raise KitError("Give the card a short title: what is being done, or what the user needs to do.")
        card = {"id": uuid.uuid4().hex[:6], "title": title[:120], "note": (a.get("note") or "").strip()[:600],
                "column": _column(a.get("column"), "doing"), "by": me, "chat": chat, "created": now, "updated": now}
        data["cards"].append(card)
        save(project, data)
        return "Added [%s] to %s.\n\n%s" % (card["id"], COLUMNS[card["column"]], _show(data))
    if action in ("move", "update"):
        card = _find(data, a.get("id"))
        if a.get("column"):
            card["column"] = _column(a.get("column"), None)
        if a.get("title"):
            card["title"] = a["title"].strip()[:120]
        if a.get("note") is not None:
            card["note"] = a["note"].strip()[:600]
        card["updated"] = now
        # a card moved to the end of the list keeps the columns in the order work happened
        data["cards"].remove(card)
        data["cards"].append(card)
        save(project, data)
        return "Card [%s] is now in %s.\n\n%s" % (card["id"], COLUMNS[card["column"]], _show(data))
    raise KitError("action must be list, add, move or update.")
