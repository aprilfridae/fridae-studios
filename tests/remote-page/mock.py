"""A stand-in for the app with remote control: serves ui/index.html, this computer's /api, the client's
/api/remote/* (machines, discover, pair, forget) and /m/<machine>/... for fake paired computers, as the contract in
.superpowers/sdd/2026-11-26-remote-control says. 127.0.0.1 only."""
import copy, json, os, sys, threading, time
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

UI = os.environ.get("MOCK_UI") or "H:/fridae-remote-page/ui"
TYPES = {".png": "image/png", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".woff2": "font/woff2", ".ttf": "font/ttf"}
NOW = float(os.environ.get("MOCK_NOW") or time.time())
A = "H:/games/obby-rush"
CASTLE = "D:/games/castle"
PNG = bytes.fromhex("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360f8cfc0f01f0005000201a5f6c9a40000000049454e44ae426082")

STATUS = {"ok": True, "version": "1.2.3", "studio": True, "blender": True, "swarm_ceiling": 4, "commands": ["git"], "max_agents": 8,
          "plugin": {"connected": False}, "update": {"status": "", "auto": True},
          "ai": {"offered": ["claude", "gemini", "opencode"], "installed": {"claude": True, "gemini": True, "opencode": True}, "default": "claude",
                 "models": {"gemini": [["gemini-4-carbon", "Gemini 4 Carbon"]], "opencode": [["oc/big", "OpenCode Zen · big-pickle"]]},
                 "default_models": {"gemini": "gemini-4-carbon", "opencode": "oc/big"},
                 "sign_in": {"claude": {"offered": True, "state": "in"}}}}
# the other computer offers fewer AIs and starts new chats on OpenCode: the message box's AI chip must read its status
RSTATUS = {"ok": True, "version": "1.2.3", "studio": False, "blender": False, "max_agents": 8, "plugin": {"connected": False}, "update": {},
           "ai": {"offered": ["claude", "opencode"], "installed": {"claude": True, "opencode": True}, "default": "opencode",
                  "models": {"opencode": [["oc/big", "OpenCode Zen · big-pickle"]]}, "default_models": {"opencode": "oc/big"}}}

def local0():
    return {"projects": [{"path": A, "name": "Obby Rush", "type": "game", "bible": True}],
            "events": [{"id": 1, "kind": "agent_new", "agent": "a1", "name": "Lava stage", "ai": "claude"},
                       {"id": 2, "kind": "project", "agent": "a1", "path": A, "name": "Obby Rush", "type": "game", "bible": True},
                       {"id": 3, "kind": "user", "agent": "a1", "text": "Build the lava stage"}],
            "history": {"chats": [], "deleted": []}, "next": 1}

def remote0(mid):
    if mid != "m1": return {"projects": [], "events": [], "history": {"chats": [], "deleted": []}, "off": False, "next": 1, "boot": "boot0"}
    return {"projects": [{"path": CASTLE, "name": "Castle", "type": "game", "bible": True}],
            "events": [{"id": 1, "kind": "agent_new", "agent": "r1", "name": "Castle gates", "ai": "claude"},
                       {"id": 2, "kind": "project", "agent": "r1", "path": CASTLE, "name": "Castle", "type": "game", "bible": True},
                       {"id": 3, "kind": "user", "agent": "r1", "text": "Build the gates"},
                       {"id": 4, "kind": "text", "agent": "r1", "text": "The gates are up."},
                       {"id": 5, "kind": "done", "agent": "r1"},
                       {"id": 6, "kind": "agent_new", "agent": "r2", "name": "Desk question", "ai": "claude"},
                       {"id": 7, "kind": "user", "agent": "r2", "text": "What is on this computer?"}],
            "history": {"chats": [{"id": "s9", "name": "Old castle chat", "project": CASTLE, "ai": "claude", "updated": NOW - 9000, "messages": 4}], "deleted": []},
            "off": False, "next": 1, "boot": "boot0"}

FP1, FP2 = "a1" * 32, "b2" * 32
def machines0():
    return {"m1": {"id": "m1", "name": "Desktop", "addr": "192.168.1.20", "port": 47530, "fp": FP1, "online": True, "last_seen": NOW}}
FOUND = [{"id": "m2", "name": "Studio laptop", "addr": "192.168.1.31", "port": 47530, "fp": FP2},
         {"id": "m1", "name": "Desktop", "addr": "192.168.1.20", "port": 47530, "fp": FP1}]
GOOD = "ABCDEFGHJK"

COND = threading.Condition()
S = {}
POSTS = []
def reset():
    with COND:
        S["local"] = local0(); S["machines"] = machines0(); S["remote"] = {"m1": remote0("m1")}; S["found"] = copy.deepcopy(FOUND)
        del POSTS[:]; COND.notify_all()
reset()

def push(events, e):
    e["id"] = (events[-1]["id"] if events else 0) + 1; events.append(e)

def wait_events(events, after, alive=lambda: True):
    with COND:
        ev = [e for e in events if e["id"] > after]
        end = time.time() + 15
        while not ev and alive() and time.time() < end:
            COND.wait(end - time.time()); ev = [e for e in events if e["id"] > after]
    return ev

def chat_api(st, p, q, body, prefix):
    """What both this computer and a fake remote answer for a chat: new, send, ai, project, history, review."""
    if p == "/api/new":
        with COND:
            aid = prefix + str(100 + st["next"]); st["next"] += 1
            push(st["events"], {"kind": "agent_new", "agent": aid, "name": "New chat", "ai": body.get("ai") or "claude"}); COND.notify_all()
        return {"ok": True, "agent": aid}
    if p == "/api/send":
        with COND:
            push(st["events"], {"kind": "user", "agent": body.get("agent"), "text": body.get("text")}); COND.notify_all()
        return {"ok": True}
    if p == "/api/project" and body and body.get("action") == "use":
        pr = next((x for x in st["projects"] if x["path"] == body.get("path")), None)
        if not pr: return {"ok": False, "error": "No such project."}
        with COND:
            push(st["events"], dict(pr, kind="project", agent=body.get("agent"))); COND.notify_all()
        return {"ok": True, "project": pr}
    if p == "/api/projects": return {"projects": st["projects"], "default_parent": "H:/games", "default_parent_plain": "H:/work", "git": True}
    if p == "/api/history": return st["history"]
    if p == "/api/review":
        path = q.get("path", [""])[0] if body is None else body.get("path")
        return {"ok": True, "project": path, "key": "k", "run": {"on": False}, "items": [], "needs": []}
    if p == "/api/chat-move":   # moving a chat between computers (agents.rs hold_for_move / move_out / move_in / moved_away)
        act, aid = body.get("action"), body.get("agent")
        if act == "hold":
            st.setdefault("held", {})[aid] = bool(body.get("on")); return {"ok": True}
        if act == "out":
            with COND: ev = [dict(e) for e in st["events"] if e.get("agent") == aid]
            if not any(e["kind"] == "user" for e in ev): return {"ok": False, "error": "Nothing has been said in this chat yet."}
            flags = [e for e in ev if e["kind"] in ("busy", "done")]
            if flags and flags[-1]["kind"] == "busy" and flags[-1].get("value"): return {"ok": False, "error": "It is still working. It moves when the reply is over."}
            name = next((e["name"] for e in ev if e["kind"] == "agent_new"), "Chat")
            evs = [{k: v for k, v in e.items() if k not in ("id", "agent")} for e in ev if e["kind"] != "agent_new"]
            return {"ok": True, "chat": {"id": aid, "name": name, "ai": "claude", "model": None, "events": evs, "handover": "[chat so far]", "queued": []}}
        if act == "in":
            chat = body.get("chat") or {}
            with COND:
                # the same chat keeps its id, unless this computer has a chat by that id
                taken = {e.get("agent") for e in st["events"] if e["kind"] == "agent_new"} - {e.get("agent") for e in st["events"] if e["kind"] == "agent_removed"}
                aid = chat.get("id") if chat.get("id") and chat.get("id") not in taken else prefix + "m" + str(st["next"])
                st["next"] += 1
                push(st["events"], {"kind": "agent_new", "agent": aid, "name": chat.get("name") or "Chat", "ai": chat.get("ai") or "claude", "restored": True})
                for e in chat.get("events") or []:
                    if e.get("kind") != "project": push(st["events"], dict(e, agent=aid))
                pr = next((x for x in st["projects"] if x["path"] == body.get("project")), None)
                if pr: push(st["events"], dict(pr, kind="project", agent=aid))
                push(st["events"], {"kind": "done", "agent": aid}); COND.notify_all()
            return {"ok": True, "agent": aid}
        if act == "away":
            with COND: push(st["events"], {"kind": "agent_removed", "agent": aid}); COND.notify_all()
            return {"ok": True}
        return {"ok": False, "error": "Unknown action."}
    if p == "/api/project-copy" and body.get("action") == "copy":   # project_copy.rs: run by the page's own app
        side = lambda m: S["local"] if not m else S["remote"][m]
        src = next((x for x in side(body.get("from"))["projects"] if x["path"] == body.get("path")), None)
        if not src: return {"ok": False, "error": "That project isn't on that computer's list."}
        dst = side(body.get("to"))
        with COND:
            there = next((x for x in dst["projects"] if x["name"].lower() == src["name"].lower()), None)
            if not there:
                there = dict(src, path="H:/games/" + src["name"]); dst["projects"].append(there)
        return {"ok": True, "path": there["path"], "files": 3, "bytes": 1200}
    if p == "/api/open-chat":
        with COND:
            aid = prefix + "o" + str(st["next"]); st["next"] += 1
            push(st["events"], {"kind": "agent_new", "agent": aid, "name": "Opened " + str(body.get("id")), "ai": "claude", "restored": True}); COND.notify_all()
        return {"ok": True, "agent": aid}
    return None

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, obj, code=200, ctype="application/json"):
        b = obj if isinstance(obj, bytes) else json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(b))); self.send_header("Cache-Control", "no-store"); self.end_headers()
        try: self.wfile.write(b)
        except Exception: pass
    def do_GET(self): self.route(None)
    def refuse(self, st):
        """What this computer's client answers for a computer it cannot use (the contract's three answers)."""
        if st["off"] == "unpaired": return self.send({"error": "not paired"}, 401)
        if st["off"] == "changed": return self.send({"error": "identity changed"}, 502)
        return self.send({"error": "offline"}, 502)
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0); raw = self.rfile.read(n) if n else b""
        try: body = json.loads(raw or b"{}")
        except Exception: body = {}
        self.route(body)
    def route(self, body):
        u = urlparse(self.path); q = parse_qs(u.query); p = u.path
        if p in ("/", "/index.html"): return self.send(open(UI + "/index.html", "rb").read(), ctype="text/html; charset=utf-8")
        # ---- the harness's own ----
        if p == "/mock/reset": reset(); return self.send({"ok": True})
        if p == "/mock/posts": return self.send(POSTS)
        if p == "/mock/push":
            with COND: push(S["local"]["events"], body); COND.notify_all()
            return self.send({"ok": True})
        if p == "/mock/rpush":   # {"m": id, "event": {...}}
            with COND: push(S["remote"][body["m"]]["events"], body["event"]); COND.notify_all()
            return self.send({"ok": True})
        if p == "/mock/found":   # {"found": [...]}: who finding hears from now on
            S["found"] = body["found"]; return self.send({"ok": True})
        if p == "/mock/off":     # {"m": id, "off": false | true | "unpaired" | "changed"}
            o = body["off"]
            with COND: S["remote"][body["m"]]["off"] = ("off" if o is True else o or False); COND.notify_all()
            return self.send({"ok": True})
        # {"m": id, "fill": 0 | "equal" | n}: its app started again (a new boot): its event ids begin at 1, its chats
        # put back; with fill, what it did on starting numbers up to its old newest id ("equal") or n past it
        if p == "/mock/restart":
            with COND:
                st = S["remote"][body["m"]]; old = st["events"][-1]["id"] if st["events"] else 0
                ev = [dict(e, id=i + 1) for i, e in enumerate(remote0(body["m"])["events"][:3])]
                fill = body.get("fill") or 0
                upto = old if fill == "equal" else (old + fill if fill else 0)
                while len(ev) < upto: ev.append({"id": len(ev) + 1, "kind": "text", "agent": "r1", "text": "Put back " + str(len(ev) + 1) + "."})
                st["events"] = ev; st["boot"] = "boot" + str(time.time()); COND.notify_all()
            return self.send({"ok": True, "old": old, "newest": len(ev)})
        # ---- another computer, through this one's client ----
        if p.startswith("/m/"):
            parts = p.split("/", 3); mid = parts[2]; rest = "/" + (parts[3] if len(parts) > 3 else "")
            if mid not in S["machines"]: return self.send({"error": "unknown computer"}, 404)
            st = S["remote"].setdefault(mid, remote0(mid))
            if st["off"]: return self.refuse(st)
            if body is not None: POSTS.append(["/m/" + mid + rest, body])
            if rest == "/api/events":
                after = int(q.get("after", ["0"])[0])
                if q.get("wait", ["1"])[0] == "0":
                    with COND: return self.send({"events": [e for e in st["events"] if e["id"] > after], "boot": st["boot"]})
                ev = wait_events(st["events"], after, lambda: not st["off"])
                if st["off"]: return self.refuse(st)
                return self.send({"events": ev, "boot": st["boot"]})
            if rest == "/api/status": return self.send(RSTATUS)
            if rest.startswith("/img/"): return self.send(PNG, ctype="image/png")
            r = chat_api(st, rest, q, body, "r")
            return self.send(r if r is not None else {"ok": True})
        if not p.startswith("/api/"):
            f = UI + p
            if os.path.isfile(f) and ".." not in p: return self.send(open(f, "rb").read(), ctype=TYPES.get(os.path.splitext(f)[1], "application/octet-stream"))
            return self.send(b"", 404, "text/plain")
        if body is not None: POSTS.append([p, body])
        # ---- the client's page API (Task B) ----
        if p == "/api/remote/machines":
            ms = [dict(m, online=not S["remote"].get(m["id"], {}).get("off"), paired=S["remote"].get(m["id"], {}).get("off") != "unpaired") for m in S["machines"].values()]
            return self.send({"this": {"id": "me", "name": "Laptop"}, "machines": ms})
        if p == "/api/remote/discover":
            time.sleep(0.8)
            return self.send({"found": [dict(f, paired=f["id"] in S["machines"] and S["machines"][f["id"]]["fp"] == f["fp"] and S["remote"].get(f["id"], {}).get("off") != "unpaired") for f in S["found"]]})
        if p == "/api/remote/pair":
            code = "".join(c for c in (body.get("code") or "") if c.isalnum()).upper()
            f = next((x for x in S["found"] if x["addr"] == body.get("addr") or x["name"].lower() == (body.get("addr") or "").lower()), None)
            if not f: return self.send({"ok": False, "error": "Could not reach that computer. Check the name, and that it lets other computers connect."})
            if code != GOOD: return self.send({"ok": False, "error": "That code is not right. Check the code on the other computer and try again."})
            m = {"id": f["id"], "name": f["name"], "addr": f["addr"], "port": f["port"], "fp": f["fp"], "online": True, "last_seen": time.time()}
            with COND:
                S["machines"][f["id"]] = m; S["remote"].setdefault(f["id"], remote0(f["id"]))
                if S["remote"][f["id"]]["off"] in ("unpaired", "changed"): S["remote"][f["id"]]["off"] = False
                COND.notify_all()
            return self.send({"ok": True, "machine": m})
        # ---- this computer letting others connect (Settings, Other computers) ----
        if p == "/api/remote/host":
            return self.send({"on": True, "port": 47530, "id": "me", "name": "Laptop", "fp": "c3" * 32, "devices": [], "code": None, "error": None})
        if p == "/api/notes": return self.send({"all": {"notes": 0, "rules": 0}})
        if p == "/api/remote/forget":
            S["machines"].pop(body.get("id"), None); return self.send({"ok": True})
        # ---- this computer ----
        if p == "/api/events":
            after = int(q.get("after", ["0"])[0])
            if q.get("wait", ["1"])[0] == "0":
                with COND: return self.send({"events": [e for e in S["local"]["events"] if e["id"] > after]})
            return self.send({"events": wait_events(S["local"]["events"], after)})
        if p == "/api/status": return self.send(STATUS)
        r = chat_api(S["local"], p, q, body, "a")
        if r is not None: return self.send(r)
        return self.send({"ok": True, "events": [], "projects": [], "cards": [], "records": [], "chats": [], "items": [], "skills": [], "accounts": []})

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8798), H).serve_forever()
