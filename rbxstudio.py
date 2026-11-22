"""Talk to Roblox Studio directly, through the MCP server that Studio installs.

Studio ships a small program (started by %LOCALAPPDATA%/Roblox/mcp.bat) that
speaks the Model Context Protocol over stdin/stdout. An agent normally calls it
itself, which puts every result through the agent's context. This client lets
the Python tools call Studio on their own, so large results (every script in a
place, say) never pass through a chat.

  with Studio() as s:
      print(s.run_luau("return #workspace:GetDescendants()"))
      text = s.fetch("return game:GetService('HttpService'):JSONEncode(bigTable)")

  rbx.py studio "return game.Name"
  rbx.py studio --measure          find how long a result Studio returns intact

run_luau returns whatever Studio sends back, as it is. fetch is for results
that must arrive complete and exact: the value stays in Studio and is collected
in pages, and its length and checksum are checked, so nothing is cut short
whatever limit execute_luau has. Measured on Studio: one execute_luau call
returns up to about 99,000 characters intact and cuts the result at 100,000.
PAGE below stays well inside that, and `rbx.py studio --measure` repeats the
measurement.

Set RBXKIT_STUDIO_MCP to a different launcher, or RBXKIT_STUDIO_ID to choose
between several open Studio windows.
"""
import json, os, queue, re, subprocess, sys, threading, time

from rbxpaths import VERSION, KitError, no_window

PAGE = 60000        # bytes of result collected per call by fetch(); Studio cuts one result at 100,000 characters
CODE_LIMIT = 60000  # bytes of Luau sent in one call by callers that send a lot; the limit on code sent is not measured


class StudioError(KitError):
    pass


STUDIO_OFF = "Studio is off for this run (FRIDAE_NO_STUDIO is set): nothing reads or changes a Roblox Studio place."


def studio_off():
    """True when the app was started with FRIDAE_NO_STUDIO: no tool may reach an open Studio place."""
    return bool(os.environ.get("FRIDAE_NO_STUDIO"))


def launcher():
    """The file that starts Studio's MCP server, or None (always None when Studio is off for this run)."""
    if studio_off():
        return None
    path = os.environ.get("RBXKIT_STUDIO_MCP")
    if not path and os.name == "nt":
        path = os.path.join(os.environ.get("LOCALAPPDATA", ""), "Roblox", "mcp.bat")
    if not path and sys.platform == "darwin":
        path = "/Applications/RobloxStudio.app/Contents/MacOS/StudioMCP"
    return path if path and os.path.exists(path) else None


def available():
    """True when Studio's MCP launcher exists on this computer. Studio itself may still be closed."""
    return launcher() is not None


_ESCAPES = {"\\": "\\\\", '"': '\\"', "\n": "\\n", "\r": "\\r"}


def lua_string(text):
    """A Luau string literal holding exactly this text."""
    return '"' + re.sub(r'[\\"\x00-\x1f\x7f]', lambda m: _ESCAPES.get(m.group()) or "\\x%02x" % ord(m.group()), text) + '"'


class Studio:
    """One connection to Studio's MCP server. Use it in a `with` block."""

    def __init__(self, datamodel="Edit"):
        self.studio_id = os.environ.get("RBXKIT_STUDIO_ID") or None
        self.datamodel = datamodel
        self.info = {}
        self.page = PAGE
        self.proc = None
        self._q = queue.Queue()
        self._n = 0
        self._schemas = None

    # ------------------------------------------------------- connection
    def __enter__(self):
        if studio_off():
            raise StudioError(STUDIO_OFF)
        path = launcher()
        if not path:
            raise StudioError("Roblox Studio's MCP server was not found (no mcp.bat in %LOCALAPPDATA%\\Roblox). Install or "
                              "update Roblox Studio and switch on the MCP server in its Assistant settings.")
        cmd = ["cmd.exe", "/c", path] if path.lower().endswith((".bat", ".cmd")) else (
            [sys.executable, path] if path.lower().endswith(".py") else
            [path, "--stdio"] if os.path.basename(path) == "StudioMCP" else [path])
        try:
            self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                         creationflags=no_window())
        except OSError as e:
            raise StudioError("Could not start Roblox Studio's MCP server: %s" % e)
        threading.Thread(target=self._read, daemon=True).start()
        try:
            self._request("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                         "clientInfo": {"name": "rbxkit", "version": VERSION}}, 20)
            self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        except Exception:
            self.close()
            raise
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        p, self.proc = self.proc, None
        if not p:
            return
        try:
            p.stdin.close()
        except OSError:
            pass
        try:
            p.wait(timeout=1.5)
        except subprocess.TimeoutExpired:
            # cmd.exe started the real server: end the whole tree
            if os.name == "nt":
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)], capture_output=True, creationflags=no_window())
            else:
                p.kill()

    def _read(self):
        try:
            for line in self.proc.stdout:
                self._q.put(line)
        except (OSError, ValueError, AttributeError):
            pass
        self._q.put(None)

    def _send(self, msg):
        try:
            self.proc.stdin.write((json.dumps(msg) + "\n").encode("utf-8"))
            self.proc.stdin.flush()
        except (OSError, AttributeError):
            raise StudioError("The connection to Roblox Studio's MCP server closed.")

    def _request(self, method, params, timeout):
        if not self.proc:
            raise StudioError("Not connected: use `with Studio() as s:`.")
        self._n += 1
        mid = self._n
        self._send({"jsonrpc": "2.0", "id": mid, "method": method, "params": params})
        deadline = time.time() + timeout
        while True:
            left = deadline - time.time()
            if left <= 0:
                raise StudioError("Roblox Studio did not answer within %d seconds (%s)." % (timeout, method))
            try:
                line = self._q.get(timeout=left)
            except queue.Empty:
                continue
            if line is None:
                raise StudioError("Roblox Studio's MCP server stopped.")
            try:
                msg = json.loads(line.decode("utf-8", "replace"))
            except ValueError:
                continue
            if not isinstance(msg, dict) or msg.get("id") != mid:
                continue  # a notification, or the late answer to a call that timed out
            if msg.get("error"):
                err = msg["error"]
                raise StudioError("Roblox Studio refused %s: %s" % (method, err.get("message") if isinstance(err, dict) else err))
            return msg.get("result") or {}

    # ------------------------------------------------------------ tools
    def tools(self):
        """{name: input schema} for every tool Studio offers."""
        if self._schemas is None:
            res = self._request("tools/list", {}, 30)
            self._schemas = {t["name"]: (t.get("inputSchema") or {}) for t in res.get("tools") or []}
        return self._schemas

    def studios(self):
        """The open Studio windows, as Studio lists them (each has an id and a name)."""
        text = self._call("list_roblox_studios", {}, 30)
        try:
            data = json.loads(text)
        except ValueError:
            raise StudioError("Could not read Studio's list of windows: %s" % text[:200])
        found = data.get("studios") if isinstance(data, dict) else data
        return [s for s in (found or []) if isinstance(s, dict)]

    def pick(self):
        """The id of the Studio window to use: the only one, or the one marked active."""
        if self.studio_id:
            return self.studio_id
        found = self.studios()
        if not found:
            raise StudioError("Roblox Studio is not connected. Open Studio with the place loaded and check that the MCP "
                              "server is switched on in its Assistant settings.")
        if len(found) > 1:
            active = [s for s in found if s.get("active") or s.get("isActive") or s.get("focused")]
            if len(active) != 1:
                names = "; ".join("%s = %s" % (s.get("id"), s.get("name")) for s in found)
                raise StudioError("Several Studio windows are open (%s). Close the others, or set RBXKIT_STUDIO_ID to the id "
                                  "of the one to use." % names)
            found = active
        self.studio_id = str(found[0].get("id"))
        self.info = found[0]
        return self.studio_id

    def _call(self, tool, args, timeout):
        res = self._request("tools/call", {"name": tool, "arguments": args}, timeout)
        text = "\n".join(c.get("text", "") for c in res.get("content") or [] if c.get("type") == "text")
        if res.get("isError"):
            raise StudioError(text.strip() or "Roblox Studio reported an error from %s." % tool)
        return text

    def call(self, tool, args=None, timeout=120):
        """Call one Studio tool and return its text. The studio id is filled in when the tool wants one."""
        args = dict(args or {})
        schema = self.tools().get(tool)
        if schema is None:
            raise StudioError("Roblox Studio has no tool called %s." % tool)
        if "studio_id" in (schema.get("properties") or {}) and "studio_id" not in args:
            args["studio_id"] = self.pick()
        return self._call(tool, args, timeout)

    def state(self):
        return self.call("get_studio_state", {}, 30)

    def run_luau(self, code, timeout=120):
        """Run Luau in Studio and return what Studio sends back, unchanged. Long results may be cut: use fetch()."""
        return self.call("execute_luau", {"code": code, "datamodel_type": self.datamodel}, timeout)

    def fetch(self, code, timeout=180):
        """Run Luau that returns a string (or a table, sent as JSON) and return all of it, exactly.

        The value is kept in Studio (in `shared`) and collected a page at a
        time, then checked against its length and checksum. If plain pages do
        not arrive intact, they are collected again as hex, then in smaller pages.
        """
        page = self.page
        token = "p%d_%d" % (int(time.time() * 1000), self._n)
        head = self.run_luau(_STORE % (code, token), timeout)
        m = re.search(r"RBXKIT<(\d+),(\d+)>", head)
        if not m:
            raise StudioError(head.strip()[:2000] or "Studio returned nothing.")
        size, check = int(m.group(1)), int(m.group(2))
        try:
            while page >= 200:
                for as_hex in (False, True):
                    data = self._pages(token, size, page // 2 if as_hex else page, as_hex)
                    if len(data) == size and sum(data) % 4294967296 == check:
                        self.page = page  # remember what worked for the next fetch
                        return data.decode("utf-8", "replace")
                page //= 4  # Studio cut the pages: ask for smaller ones
            raise StudioError("A result of %d bytes did not come back from Studio intact." % size)
        finally:
            try:
                self.run_luau("if shared.__rbxkit then shared.__rbxkit[%s] = nil end return 'ok'" % lua_string(token), 30)
            except StudioError:
                pass

    def _pages(self, token, size, page, as_hex):
        data, at = b"", 1
        while at <= size:
            text = self.run_luau(_PAGE % (lua_string(token), at, page, "true" if as_hex else "false"), 60)
            m = re.search(r"RBXKIT<(\d+)>", text)
            end = text.rfind("<END>")
            if not m or end < m.end():
                return b""  # cut short or reshaped: the caller tries the other way
            body = text[m.end():end]
            try:
                data += bytes.fromhex(body) if as_hex else body.encode("utf-8")
            except ValueError:
                return b""
            nxt = int(m.group(1)) + 1
            if nxt <= at:
                return b""
            at = nxt
        return data

    def measure(self, most=4_000_000):
        """Find how long a result execute_luau returns intact. Returns (longest that arrived whole, first that did not)."""
        def whole(n):
            try:
                text = self.run_luau("return string.rep('x', %d) .. '<END>'" % n)
            except StudioError:
                return False
            return text.count("x") == n and "<END>" in text

        good, bad, n = 0, None, 1000
        while n <= most and bad is None:  # double until a result is cut
            if whole(n):
                good, n = n, n * 2
            else:
                bad = n
        while bad and bad - good > max(100, good // 50):  # then narrow down between the two
            mid = (good + bad) // 2
            good, bad = (mid, bad) if whole(mid) else (good, mid)
        return good, bad


# Keep the value in Studio and report its size and a checksum (sum of bytes).
_STORE = """local __v = (function()
%s
end)()
if type(__v) ~= "string" then
	local ok, enc = pcall(function() return game:GetService("HttpService"):JSONEncode(__v) end)
	__v = ok and enc or tostring(__v)
end
shared.__rbxkit = shared.__rbxkit or {}
shared.__rbxkit["%s"] = __v
local sum = 0
for i = 1, #__v, 4000 do
	local b = { string.byte(__v, i, math.min(#__v, i + 3999)) }
	for k = 1, #b do sum += b[k] end
	sum %%= 4294967296
end
return "RBXKIT<" .. #__v .. "," .. sum .. ">"
"""

# One page, ending on a whole UTF-8 character.
_PAGE = """local s = (shared.__rbxkit or {})[%s]
if not s then error("rbxkit: the stored result is gone") end
local i, n, hex = %d, %d, %s
local j = math.min(#s, i + n - 1)
while j < #s and j > i do
	local b = string.byte(s, j + 1)
	if b >= 128 and b < 192 then j -= 1 else break end
end
local part = string.sub(s, i, j)
if hex then part = string.gsub(part, ".", function(c) return string.format("%%02x", string.byte(c)) end) end
return "RBXKIT<" .. j .. ">" .. part .. "<END>"
"""


def _cmd(a):
    with Studio(datamodel=a.datamodel) as s:
        if a.list:
            print(json.dumps(s.studios(), indent=1))
        elif a.tools:
            names = a.tools.split(",")
            print(json.dumps({k: v for k, v in s.tools().items() if a.tools == "all" or k in names}, indent=1))
        elif a.measure:
            good, bad = s.measure()
            print(json.dumps({"longest_intact_result": good, "first_cut_or_failed": bad,
                              "note": "characters returned whole by one execute_luau call" if bad else
                                      "no limit found up to this length"}, indent=1))
        else:
            code = sys.stdin.read() if a.code == "-" else a.code
            if not code:
                sys.exit("give the Luau to run, or - to read it from stdin")
            if os.path.isfile(code):
                code = open(code, encoding="utf-8").read()
            print(s.fetch(code, timeout=a.timeout) if a.whole else s.run_luau(code, timeout=a.timeout))


def register(sub):
    g = sub.add_parser("studio", help="run Luau in the open Roblox Studio and print the result")
    g.add_argument("code", nargs="?", help="Luau source, a .lua file, or - for stdin")
    g.add_argument("--datamodel", choices=["Edit", "Client", "Server"], default="Edit")
    g.add_argument("--whole", action="store_true", help="collect a long result in pages so none of it is cut")
    g.add_argument("--timeout", type=int, default=120)
    g.add_argument("--list", action="store_true", help="list the open Studio windows")
    g.add_argument("--tools", help="print Studio's tool schemas: 'all' or names separated by commas")
    g.add_argument("--measure", action="store_true", help="find how long a result Studio returns intact")
    g.set_defaults(fn=_cmd)
