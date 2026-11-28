"""A stand-in for the app for the toolbox's page pieces: serves ui/index.html and the /api calls the page makes, with
two chats in one project. The harness pushes events (appcard, apptool, projects_changed, theme_set ...) through
/mock/push and reads what the page sent through /mock/posts and /mock/gets. 127.0.0.1 only."""
import copy, json, os, sys, threading, time
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

UI = os.environ.get("MOCK_UI") or "H:/fridae-app-t4/ui"
TYPES = {".png": "image/png", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".woff2": "font/woff2", ".ttf": "font/ttf"}
A = "H:/games/obby-rush"
PID = "p0123456789ab"

STATUS = {"ok": True, "version": "1.2.3", "studio": True, "blender": True, "swarm_ceiling": 4, "commands": ["git"], "max_agents": 8,
          "plugin": {"connected": False}, "update": {"status": "", "auto": True},
          "ai": {"offered": ["claude", "opencode"], "installed": {"claude": True, "opencode": True}, "default": "claude",
                 "models": {"opencode": [["opencode/big-pickle", "OpenCode Zen · big-pickle"]]}, "default_models": {"opencode": "opencode/big-pickle"},
                 "sign_in": {"claude": {"offered": True, "state": "in"}}}}

def state0():
    return {"projects": [{"path": A, "name": "Obby Rush", "type": "game", "bible": True, "id": PID}],
            "events": [{"id": 1, "kind": "agent_new", "agent": "a2", "name": "Lava stage notes", "ai": "claude"},
                       {"id": 2, "kind": "project", "agent": "a2", "path": A, "name": "Obby Rush", "type": "game", "bible": True},
                       {"id": 3, "kind": "user", "agent": "a2", "text": "Write the lava notes"},
                       {"id": 4, "kind": "text", "agent": "a2", "text": "Notes written."},
                       {"id": 5, "kind": "done", "agent": "a2"},
                       {"id": 6, "kind": "agent_new", "agent": "a1", "name": "Rename things", "ai": "claude"},
                       {"id": 7, "kind": "project", "agent": "a1", "path": A, "name": "Obby Rush", "type": "game", "bible": True},
                       {"id": 8, "kind": "user", "agent": "a1", "text": "Rename this project to Lava Run"}],
            "history": {"chats": [], "deleted": []}}

COND = threading.Condition()
S = {}
POSTS = []
GETS = {}
def reset():
    with COND:
        S.clear(); S.update(state0()); del POSTS[:]; GETS.clear(); COND.notify_all()
reset()

def push(e):
    e["id"] = (S["events"][-1]["id"] if S["events"] else 0) + 1; S["events"].append(e)

def wait_events(after):
    with COND:
        ev = [e for e in S["events"] if e["id"] > after]
        end = time.time() + 15
        while not ev and time.time() < end:
            COND.wait(end - time.time()); ev = [e for e in S["events"] if e["id"] > after]
    return ev

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, obj, code=200, ctype="application/json"):
        b = obj if isinstance(obj, bytes) else json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(b))); self.send_header("Cache-Control", "no-store"); self.end_headers()
        try: self.wfile.write(b)
        except Exception: pass
    def do_GET(self): self.route(None)
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0); raw = self.rfile.read(n) if n else b""
        try: body = json.loads(raw or b"{}")
        except Exception: body = {}
        self.route(body)
    def route(self, body):
        u = urlparse(self.path); q = parse_qs(u.query); p = u.path
        if p in ("/", "/index.html"): return self.send(open(UI + "/index.html", "rb").read(), ctype="text/html; charset=utf-8")
        if p == "/mock/reset": reset(); return self.send({"ok": True})
        if p == "/mock/posts": return self.send(POSTS)
        if p == "/mock/gets": return self.send(GETS)
        if p == "/mock/push":
            with COND: push(body); COND.notify_all()
            return self.send({"ok": True})
        if p == "/mock/projects":   # {"projects": [...]}: what /api/projects answers from now on
            S["projects"] = body["projects"]; return self.send({"ok": True})
        if not p.startswith("/api/"):
            f = UI + p
            if os.path.isfile(f) and ".." not in p: return self.send(open(f, "rb").read(), ctype=TYPES.get(os.path.splitext(f)[1], "application/octet-stream"))
            return self.send(b"", 404, "text/plain")
        if body is None: GETS[p] = GETS.get(p, 0) + 1
        else: POSTS.append([p, body])
        if p == "/api/events":
            after = int(q.get("after", ["0"])[0])
            if q.get("wait", ["1"])[0] == "0":
                with COND: return self.send({"events": [e for e in S["events"] if e["id"] > after]})
            return self.send({"events": wait_events(after)})
        if p == "/api/status": return self.send(STATUS)
        if p == "/api/projects": return self.send({"projects": S["projects"], "default_parent": "H:/games", "default_parent_plain": "H:/work", "git": True})
        if p == "/api/history": return self.send(S["history"])
        if p == "/api/remote/machines": return self.send({"this": {"id": "me", "name": "Laptop"}, "machines": []})
        if p == "/api/review":
            path = q.get("path", [""])[0] if body is None else body.get("path")
            return self.send({"ok": True, "project": path, "key": "k", "run": {"on": False}, "items": [], "needs": []})
        if p == "/api/app-card": return self.send({"ok": True})
        if p == "/api/theme-now": return self.send({"ok": True})
        return self.send({"ok": True, "events": [], "projects": [], "cards": [], "records": [], "chats": [], "items": [], "skills": [], "accounts": []})

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8799), H).serve_forever()
