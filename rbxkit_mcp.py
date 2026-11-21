#!/usr/bin/env python
"""rbxkit MCP server: exposes the toolkit to any agent that speaks the Model
Context Protocol (Claude, Cursor, Codex, Gemini CLI, Windsurf, ...).

Start it with:  python rbxkit_mcp.py     (stdio transport, no dependencies)

It works beside Roblox's own Studio MCP server, which is what actually touches
Studio. This server supplies everything around it: Blender, textures, icons,
animation and video tools, the Luau snippets to run in Studio, and the guides
that say how to use it all.
"""
import base64, json, os, re, subprocess, sys, tempfile, threading, time, uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rbxpaths import KIT, VERSION, KitError, cli, no_window
MAX_IMAGE = 1_500_000
MAX_IMAGES = 4

# which guides to load for which kind of request lives in routing.txt (the app uses the same file)
INSTRUCTIONS = """rbxkit: Roblox Studio toolkit. It works beside the Roblox Studio MCP tools (execute_luau, screen_capture,
start_stop_play, ...). rbx_studio_snippet returns ready-made Luau to run in Studio. Tools that produce images return
them: always look before reporting a model, texture or icon as done.

{routing}- Generated textures: `rbx_generate_texture` makes surface materials, particle sprites and decals (through the user's Gemini
  key, or by asking the user to make the image in their Gemini chat). For plain tileable surfaces prefer Studio's generate_material.
  Use it only for textures. Never use it for UI, icons, logos, typography or any graphic design: build those natively.
To pick the style guide, look at the place first (its existing UI, materials and colours); if it is new or unclear,
choose the closest and say which you chose. Guides already read in this chat do not need loading again.

More tools, for when they fit:
- `rbx_project`: the game's project folder. `checkpoint` before and after a risky change to scripts (it saves every script to
  files and commits them; it does not cover parts or UI), `changes` to see which scripts differ, `restore_scripts` to put
  scripts back (a dry run unless `confirm` is true), `info` at the start of a task.
- `rbx_economy`: before settling prices, upgrade costs, rebirths or drop rates, simulate them: it shows how long each
  purchase takes and flags slow starts, walls, runaway inflation and content that runs out.
- `rbx_market`: live public numbers for Roblox experiences (players now, visits, like ratio) when comparing with existing games.
- `rbx_research`: Roblox's public charts summarised (genres, openings, rising new games, the top of each chart) when
  deciding what to make or how a genre is doing.
- `rbx_team`: inside the app, hand an independent piece of work to a worker, or finished work to a critic, then `wait`
  for their reports. Give each a full brief; they start knowing nothing.
- Studio snippets (rbx_studio_snippet): `vet_asset` before trusting an inserted model, `perf_capture` for frame time and
  instance budgets, `session_recorder` to record what a playtest was like for the player, `sound_probe` to measure a sound.
""".replace("{routing}", open(os.path.join(KIT, "routing.txt"), encoding="utf-8").read())

PREFACE = """> How to read this guide from an MCP client: tool names such as `execute_luau`, `screen_capture` or
> `open_cloud_invoke` belong to the Roblox Studio MCP server. Where it says to run `rbx.py <command>`, you can
> call the matching tool instead: blender -> rbx_blender, tex -> rbx_texture, sheet -> rbx_check_images,
> icon -> rbx_icon, video -> rbx_video, anim -> rbx_animation, package -> rbx_vfx_package. A file under
> `studio/` is fetched with rbx_studio_snippet. Another guide ("the roblox-vfx skill") is rbx_guide with
> that topic. Mentions of ToolSearch or skills are Claude Code details you can ignore.

"""

WEB_GUIDES = {"emil-design-eng", "motion-design", "redesign-skill", "reference-design-contract",
              "soft-skill", "minimalist-skill", "brutalist-skill"}
WEB_NOTE = ("> This guide was written for websites. For Roblox, take its judgement and translate its CSS with the "
            "`web-to-roblox` guide (rbx_guide topic web-to-roblox)." + chr(10) + chr(10))

GUIDES = {
    "workflow": ("skills/roblox-studio-workflow/SKILL.md", "Start here: tools, ground rules, 3D models, animations, icons, uploads"),
    "playtest": ("skills/roblox-playtest/SKILL.md", "Running a playtest and acting like a player"),
    "ui-design": ("skills/roblox-ui-design/SKILL.md", "Building and checking Roblox UI"),
    "ui-helpers": ("skills/roblox-ui-design/helpers.lua", "Luau UI builder functions to paste into a builder"),
    "ui-audit": ("skills/roblox-ui-design/audit.lua", "Luau UI defect check to run in Studio"),
    "sim-ui": ("skills/roblox-sim-ui/SKILL.md", "The chunky simulator / tycoon UI look"),
    "vfx": ("skills/roblox-vfx/SKILL.md", "Visual effects: modules, capture, textures, meshes"),
    "vfx-basics": ("skills/roblox-vfx/references/vfx-basics.md", "VFX craft notes: layers, timing, brightness, budgets"),
    "tester": ("agents/roblox-tester.md", "Instructions for a sub-agent that only runs playtests"),
    "web-to-roblox": ("skills/roblox-ui-design/web-to-roblox.md", "How to apply the design guides below in Roblox (CSS to Roblox table)"),
    "emil-design-eng": ("skills/emil-design-eng/SKILL.md", "Design: UI polish and when and how to animate"),
    "motion-design": ("skills/motion-design/SKILL.md", "Design: motion craft, beat maps, pacing"),
    "redesign-skill": ("skills/redesign-skill/SKILL.md", "Design: audit and upgrade existing UI"),
    "reference-design-contract": ("skills/reference-design-contract/SKILL.md", "Design: turn references into a design contract"),
    "unslop": ("skills/unslop/SKILL.md", "Design: turn sloppy, AI-looking or vibecoded UI into designed UI (audit, direction, nine passes)"),
    "soft-skill": ("skills/soft-skill/SKILL.md", "Style: soft premium look"),
    "minimalist-skill": ("skills/minimalist-skill/SKILL.md", "Style: clean minimal look"),
    "brutalist-skill": ("skills/brutalist-skill/SKILL.md", "Style: raw industrial look"),
    "game-brainstorm": ("skills/game-brainstorm/SKILL.md", "Game: interview the user about a new idea and write GAME.md"),
    "game-bible": ("skills/game-bible/SKILL.md", "Game: the GAME.md format and rules; writing one for an existing game"),
    "game-design": ("skills/game-design/SKILL.md", "Game: core loop, first five minutes, feedback, pacing, fair retention, failure modes"),
    "economy-design": ("skills/economy-design/SKILL.md", "Game: prices, rewards, cost curves, drop odds, checking numbers with rbx_economy"),
    "player-testing": ("skills/player-testing/SKILL.md", "Game: testing like a new player; feel, performance budgets, devices, multi-player"),
    "sound-design": ("skills/sound-design/SKILL.md", "Game: sounds and music by measurement; timing, levels, SoundGroups, rolloff"),
    "market-research": ("skills/market-research/SKILL.md", "Game: comparable live experiences with rbx_market and what to learn from them"),
    "critic": ("skills/critic/SKILL.md", "Instructions for an agent spawned as a reviewer of finished work"),
    "producer": ("skills/producer/SKILL.md", "Splitting a large request between parallel agents with rbx_team"),
    "promo": ("skills/roblox-promo/SKILL.md", "Promo videos: making a TikTok/Shorts/Reels promo for a game with rbx_promo"),
}


def fill(text):
    """Replace the install-location placeholders in guides."""
    return text.replace("{{RBXKIT}}", KIT.replace("\\", "/")).replace("{{RBXKIT_WIN}}", KIT)


def run_rbx(args, stdin=None, timeout=600):
    p = subprocess.run(cli(args), input=stdin, capture_output=True, text=True, creationflags=no_window(),
                       encoding="utf-8", errors="replace", timeout=timeout)
    out = (p.stdout or "").strip()
    err = (p.stderr or "").strip()
    text = out + (("\n" + err) if err else "")
    return p.returncode == 0, text or "(no output)"


def images_in(text):
    """Image files named in a tool's output, to hand back as pictures."""
    found = []
    for m in re.finditer(r'"((?:[A-Za-z]:)?[^"\n]+?\.(?:png|jpg|jpeg))"', text):
        path = m.group(1).replace("\\\\", "\\")
        if path not in found and os.path.isfile(path) and os.path.getsize(path) <= MAX_IMAGE:
            found.append(path)
    # prefer review images over raw outputs
    found.sort(key=lambda p: 0 if ("_check" in p or "_preview" in p or "shots" in p or "sheet_" in p) else 1)
    return found[:MAX_IMAGES]


def result(text, ok=True, with_images=True):
    content = [{"type": "text", "text": text}]
    if with_images and ok:
        for path in images_in(text):
            mime = "image/png" if path.lower().endswith(".png") else "image/jpeg"
            with open(path, "rb") as f:
                content.append({"type": "image", "data": base64.b64encode(f.read()).decode("ascii"), "mimeType": mime})
    return {"content": content, "isError": not ok}


# ------------------------------------------------------------------- tools
def t_doctor(a):
    ok, text = run_rbx(["doctor"], timeout=120)
    return result(text, True, False)


def _describe(path):
    """First sentence of a skill's description, from its front matter."""
    try:
        head = open(path, encoding="utf-8").read(1600)
    except OSError:
        return ""
    m = re.search(r"^description:\s*[>|]?-?\s*(.+?)(?=\n[A-Za-z_.-]+:|\n---)", head, re.S | re.M)
    text = " ".join(m.group(1).split()).strip("\"'") if m else ""
    return (text[:110] + "...") if len(text) > 110 else text


def _register_roblox_skills():
    base = os.path.join(KIT, "guides", "roblox-skills")
    if not os.path.isdir(base):
        return
    for name in sorted(os.listdir(base)):
        path = os.path.join(base, name, "SKILL.md")
        if os.path.exists(path) and name not in GUIDES:
            GUIDES[name] = ("guides/roblox-skills/%s/SKILL.md" % name, "Roblox: " + _describe(path))
    # LihFolk's guides (CC BY 4.0): ones whose subject rbxkit already covered were left out when they were copied in
    base = os.path.join(KIT, "guides", "lihfolk")
    for name in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        path = os.path.join(base, name, "SKILL.md")
        if os.path.exists(path) and name not in GUIDES:
            GUIDES[name] = ("guides/lihfolk/%s/SKILL.md" % name, "LihFolk: " + _describe(path))


_register_roblox_skills()


def _reference(topic):
    """A topic like 'roblox-luau/type-system' is a file in that guide's references folder."""
    if "/" not in topic:
        return None
    skill, ref = topic.split("/", 1)
    if skill not in GUIDES:
        return None
    folder = os.path.join(os.path.dirname(os.path.join(KIT, GUIDES[skill][0])), "references")
    ref = os.path.basename(ref)
    for cand in (ref, ref + ".md"):
        path = os.path.join(folder, cand)
        if os.path.isfile(path):
            return fill(open(path, encoding="utf-8").read())
    return None


def _one_guide(topic):
    path = os.path.join(KIT, GUIDES[topic][0])
    if not os.path.exists(path):
        return None
    text = fill(open(path, encoding="utf-8").read())
    refs = os.path.join(os.path.dirname(path), "references")
    if topic != "vfx" and os.path.isdir(refs):
        files = sorted(f[:-3] for f in os.listdir(refs) if f.endswith(".md"))
        if files:
            text += "\n\nReference files for this guide (load with rbx_guide topic `%s/<name>`): %s" % (topic, ", ".join(files))
    if text.startswith("---"):
        text = text.split("---", 2)[2].lstrip()  # drop the skill front matter
    if topic in WEB_GUIDES:
        text = WEB_NOTE + text
    if topic == "vfx":
        refs = os.path.join(os.path.dirname(path), "references")
        if os.path.isdir(refs):
            text += "\n\nReference files present: " + ", ".join(sorted(f for f in os.listdir(refs) if f.endswith(".md")))
    if GUIDES[topic][0].startswith("guides/lihfolk/"):
        # its code (Luau modules, examples, Blender scripts, rubrics) is read straight from the folder
        folder = os.path.dirname(path)
        files = sorted(os.path.relpath(os.path.join(r, f), folder).replace("\\", "/")
                       for r, _, fs in os.walk(folder) for f in fs if f != "SKILL.md")
        text += "\n\nThis guide's folder is %s" % folder.replace("\\", "/")
        if files:
            text += "; read its files from there: " + ", ".join(files)
        text += ("\n\n(From LihFolk Vault V2 by LihFolk, CC BY 4.0. Search the full vault with rbx_vault. Where a step "
                 "names a slash command or agent from the LihFolk plugin, do that step yourself.)")
    return text


def t_guide(a):
    """One or several guides in a single call."""
    wanted = [str(t).strip().lower() for t in (a.get("topics") or [])]
    if a.get("topic"):
        wanted.insert(0, str(a["topic"]).strip().lower())
    wanted = list(dict.fromkeys(t for t in wanted if t))
    if not wanted:
        lines = ["Guide topics (call rbx_guide with one, or several in `topics`):"] + ["- %s: %s" % (k, v[1]) for k, v in GUIDES.items()]
        return result("\n".join(lines), True, False)
    out, unknown = [], []
    for topic in wanted:
        text = _one_guide(topic) if topic in GUIDES else _reference(topic)
        if text is None:
            unknown.append(topic)
        else:
            out.append("=" * 20 + " GUIDE: " + topic + " " + "=" * 20 + "\n\n" + text)
    if not out:
        return result("No such guide: " + ", ".join(unknown) + ". Call rbx_guide with no arguments to list the topics.", False)
    body = PREFACE + "\n\n".join(out)
    if unknown:
        body += "\n\n(No such guide: " + ", ".join(unknown) + ")"
    return result(body, True, False)


def t_snippet(a):
    folder = os.path.join(KIT, "studio")
    names = sorted(f[:-4] for f in os.listdir(folder) if f.endswith(".lua"))
    name = (a.get("name") or "").replace(".lua", "")
    if name not in names:
        lines = []
        for n in names:  # each file's first comment line says what it is
            with open(os.path.join(folder, n + ".lua"), encoding="utf-8") as f:
                first = f.readline().strip()
            lines.append("- %s%s" % (n, ": " + first.lstrip("- ").rstrip(".") if first.startswith("--") else ""))
        return result("Studio snippets:\n" + "\n".join(lines) + "\nCall rbx_studio_snippet with one of these names.", True, False)
    src = open(os.path.join(folder, name + ".lua"), encoding="utf-8").read()
    return result(src, True, False)


def t_blender(a):
    code = a.get("code")
    if not code:
        return result("Give 'code': Python to run in Blender. `bpy` and the `rbx` helpers are available; set `result = {...}` to return values.", False)
    args = ["blender", "--script", "-"]
    if a.get("file"):
        args += ["--file", a["file"]]
    elif a.get("model"):
        args += ["--model", a["model"]]
    if a.get("nosave"):
        args.append("--nosave")
    ok, text = run_rbx(args, stdin=code, timeout=int(a.get("timeout") or 600))
    return result(text, ok)


def t_texture(a):
    code = a.get("code")
    if not code:
        return result("Give 'code': Python that calls rbx.draw_texture / rbx.draw_flipbook.", False)
    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False, encoding="utf-8") as f:
        f.write(code)
        script = f.name
    try:
        args = ["tex", "--script", script]
        if a.get("folder"):
            args += ["--folder", a["folder"]]
        if a.get("no_preview"):
            args.append("--nopreview")
        ok, text = run_rbx(args, timeout=600)
    finally:
        os.remove(script)
    return result(text, ok)


def t_check_images(a):
    paths = a.get("paths") or []
    if not paths:
        return result("Give 'paths': a list of PNG files to check.", False)
    ok, text = run_rbx(["sheet"] + list(paths) + (["--nopreview"] if a.get("no_preview") else []))
    return result(text, ok)


def t_icon(a):
    if not a.get("screenshot") or not a.get("name"):
        return result("Give 'screenshot' (the icon-booth capture file) and 'name'.", False)
    args = ["icon", a["screenshot"], "--name", a["name"]]
    if a.get("folder"):
        args += ["--folder", a["folder"]]
    if a.get("size"):
        args += ["--size", str(int(a["size"]))]
    ok, text = run_rbx(args)
    return result(text, ok)


def t_video(a):
    if not a.get("path"):
        return result("Give 'path' to a video file.", False)
    action = a.get("action") or "frames"
    args = ["video", action, a["path"]]
    for k in ("start", "end", "count"):
        if a.get(k) is not None and action == "frames":
            args += ["--" + k, str(a[k])]
    ok, text = run_rbx(args)
    return result(text, ok)


def t_animation(a):
    if not a.get("json"):
        return result("Give 'json': the file written by rbx.bake_animation.", False)
    ok, text = run_rbx(["anim", a["json"]])
    return result(text, ok, False)


def t_gen(a):
    if not a.get("prompt"):
        return result("Give 'prompt': what the texture shows.", False)
    args = ["gen", a["prompt"], "--kind", a.get("kind") or "surface"]
    for k in ("name", "folder", "reference", "model"):
        if a.get(k):
            args += ["--" + k, str(a[k])]
    if a.get("from_image"):
        args += ["--from-image", str(a["from_image"])]
    if a.get("size"):
        args += ["--size", str(int(a["size"]))]
    if a.get("seamless") is not None:
        args += ["--seamless", "yes" if a["seamless"] else "no"]
    ok, text = run_rbx(args, timeout=720)
    return result(text, ok)


def t_package(a):
    ok, text = run_rbx(["package"])
    return result(text, ok, False)


def t_lock(a):
    """A simple shared lock so several agents take turns with Studio's one playtest and camera."""
    import time
    from rbxpaths import WORK
    me = os.environ.get("RBXKIT_AGENT_ID") or "pid-%d" % os.getppid()
    name = os.environ.get("RBXKIT_AGENT_NAME") or me
    path = os.path.join(WORK, "studio.lock")
    os.makedirs(WORK, exist_ok=True)
    try:
        held = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        held = None
    if held and time.time() - held.get("since", 0) > 900:
        held = None  # stale: its owner stopped without releasing
    action = (a.get("action") or "status").lower()
    if action == "acquire":
        if held and held.get("owner") != me:
            return result("BUSY: held by \"%s\" for %d seconds. Do work that does not need Studio's playtest or camera, then try again."
                          % (held.get("name"), time.time() - held["since"]), True, False)
        json.dump({"owner": me, "name": name, "since": time.time()}, open(path, "w", encoding="utf-8"))
        return result("ACQUIRED. Release it as soon as you finish.", True, False)
    if action == "release":
        if held and held.get("owner") == me:
            os.remove(path)
            return result("RELEASED.", True, False)
        return result("You did not hold the lock.", True, False)
    return result("FREE." if not held else "Held by \"%s\" for %d seconds." % (held.get("name"), time.time() - held["since"]), True, False)


def _direct(run, *args):
    """Run a tool that lives in this process. Its own errors (KitError) are messages for the agent, shown as they are."""
    try:
        return result(run(*args), True, False)
    except KitError as e:
        return result(str(e), False)


def t_project(a):
    import rbxproject
    return _direct(rbxproject.run, a.get("action") or "info", a)


def t_economy(a):
    import rbxeconomy
    if a.get("example"):
        return result("Example inputs for `config`, one per mode:\n" + json.dumps(rbxeconomy.EXAMPLES, indent=1)
                      + "\n\nAll the fields are described here:\n" + rbxeconomy.__doc__.split("PROGRESSION", 1)[1], True, False)
    cfg = a.get("config")
    if cfg is None and ("upgrades" in a or "items" in a):
        cfg = a  # the description was given directly instead of inside `config`
    if cfg is None:
        return result("Give 'config': one object describing the economy (income, upgrades, ...) or the drop table "
                      "(mode \"drops\", items). Call with example true to see both shapes.", False)
    return _direct(rbxeconomy.run, cfg)


def t_market(a):
    import rbxmarket
    return _direct(rbxmarket.run, a.get("action") or ("details" if a.get("universe_ids") else "search"), a)


def t_research(a):
    import rbxresearch
    return _direct(rbxresearch.run, a.get("action") or "snapshot", a)


def t_team(a):
    import rbxteam
    return _direct(rbxteam.run, a.get("action") or "status", a)


def t_board(a):
    import rbxboard
    return _direct(rbxboard.run, a.get("action") or "list", a)


def t_vault(a):
    import rbxvault
    return _direct(rbxvault.run, a.get("action") or "search", a)


# ------------------------------------------------------------ promo tool
# A render can outlast the client's call timeout, so it runs in a thread and the call returns a job id once it
# would take longer than PROMO_SYNC_SECONDS. The child processes rbxpromo starts never read our stdin (the
# protocol channel) and never write our stdout.
PROMO_SYNC_SECONDS = 60
PROMO_JOB_READ_TTL = 300        # a finished job is forgotten this long after its result was read
PROMO_JOB_KEEP = 3600           # ... or this long after it finished if nobody read it
_JOBS = {}
_JOBS_LOCK = threading.Lock()


def _promo_text(data):
    return json.dumps(data, indent=1, ensure_ascii=False)


def _promo_failure(e):
    from rbxpromo.tool import problems_text
    return result(problems_text(e), False)


def _promo_expected_errors():
    from rbxpromo.recipe import RecipeError
    return (KitError, RecipeError, ValueError, FileNotFoundError, RuntimeError)


def _forget_old_jobs():
    now = time.time()
    with _JOBS_LOCK:
        for jid, j in list(_JOBS.items()):
            if not j["finished"]:
                continue
            since, ttl = (j["read"], PROMO_JOB_READ_TTL) if j["read"] else (j["finished"], PROMO_JOB_KEEP)
            if now - since > ttl:
                del _JOBS[jid]


def _start_promo_job(fn, args):
    job = {"id": uuid.uuid4().hex, "progress": 0.0, "result": None, "error": None, "finished": None, "read": None}

    def work():
        try:
            job["result"] = fn(args, lambda x: job.__setitem__("progress", x))
        except _promo_expected_errors() as e:
            job["error"] = e
        except Exception as e:  # a bug inside a render is the job's error, not a crash of the server
            job["error"] = RuntimeError("%s: %s" % (type(e).__name__, e))
        job["finished"] = time.time()

    job["thread"] = threading.Thread(target=work, daemon=True)
    with _JOBS_LOCK:
        _JOBS[job["id"]] = job
    job["thread"].start()
    return job


def _job_reply(job):
    if job["finished"] is None:
        return result(_promo_text({"job": job["id"], "state": "running", "progress": round(job["progress"], 3),
                                   "next": "Call rbx_promo again with action 'job' and this id in a minute."}), True, False)
    job["read"] = time.time()
    if job["error"] is not None:
        return _promo_failure(job["error"])
    return result(_promo_text(job["result"]))


def t_promo(a):
    from rbxpromo import engine, tool
    action = a.get("action")
    if not action:
        return result("Give 'action': " + ", ".join(list(tool.ACTIONS) + ["job"]) + ". Read the guide first: rbx_guide topic `promo`.", False)
    _forget_old_jobs()
    if action == "job":
        with _JOBS_LOCK:
            job = _JOBS.get(a.get("id") or "")
        if not job:
            return result("No promo job with id '%s'. A job is forgotten after its result was read, or an hour after it "
                          "finished, and when the app restarts. Start the render again." % a.get("id", ""), False)
        return _job_reply(job)
    if action == "install":
        return result("The app installs the promo engine: ask the user to open Setup and press Install on the Promo engine row.", False)
    fn = tool.ACTIONS.get(action)
    if fn is None:
        return result("Unknown action '%s'. Actions: %s, job." % (action, ", ".join(tool.ACTIONS)), False)
    if action in tool.NEEDS_ENGINE:
        st = engine.status()
        if not st["ready"]:
            return result("The promo engine is not installed. Install the promo engine in Setup (about %s MB; it continues if "
                          "interrupted). Missing parts: %s." % (st["download_mb"], ", ".join(st["missing"])), False)
    args = {k: a.get(k) for k in ("paths", "path", "recipe", "text", "voice", "out", "layout", "lang")}
    try:
        if action == "render":
            job = _start_promo_job(fn, args)
            job["thread"].join(PROMO_SYNC_SECONDS)
            if job["finished"] is None:
                return result(_promo_text({"job": job["id"], "state": "running", "progress": round(job["progress"], 3),
                                           "next": "This render is still running. Call rbx_promo with action 'job' and this id "
                                                   "to see progress and, when done, the result."}), True, False)
            return _job_reply(job)
        data = fn(args, None)
        if action == "styles":      # one labelled sheet of all styles; the stills are in the list, not attached
            res = result(_promo_text(data), True, False)
            with open(data["sheet"], "rb") as f:
                res["content"].append({"type": "image", "data": base64.b64encode(f.read()).decode("ascii"),
                                       "mimeType": "image/jpeg"})
            return res
        return result(_promo_text(data))
    except _promo_expected_errors() as e:
        return _promo_failure(e)


def schema(props, required=()):
    return {"type": "object", "properties": props, "required": list(required)}


S = {"type": "string"}
TOOLS = [
    ("rbx_guide", "Read rbxkit guides. Before a Roblox task, load every relevant guide in one call by passing several names in "
     "`topics` (always include 'workflow'; add the style guide for modelling, UI and effects). With no arguments, lists the topics.",
     schema({"topics": {"type": "array", "items": {"type": "string"},
                        "description": "Guide names: workflow, playtest, ui-design, ui-helpers, ui-audit, sim-ui, vfx, vfx-basics, tester, web-to-roblox, "
                                       "emil-design-eng, motion-design, redesign-skill, reference-design-contract, "
                                       "soft-skill, minimalist-skill, brutalist-skill, and the Roblox subject "
                                       "guides (roblox-luau, roblox-architecture, roblox-security, roblox-networking, roblox-data-stores, roblox-ui, "
                                       "roblox-world-building, ...). Call with no arguments for the full list"},
             "topic": {"type": "string", "description": "A single guide name (same as one entry in topics)"}}), t_guide),
    ("rbx_vault", "Search LihFolk Vault V2: 1,300+ Roblox notes (Luau, remotes, data, UI, building, VFX, animation, rigs, game "
     "design, monetization, performance, publishing, case studies), dated October 2026. Use it for limits, quotas, defaults, "
     "prices, policy and anything changed in 2025-2026, and when a bug's cause is unclear. 'search' (query: the task or bug in "
     "plain words) returns the best notes' summaries plus matching symptom lines; then 'read' (title, heading words) for ONE "
     "section, never a whole note. 'sections' (title) lists a note's headings, 'list' (folder) browses, 'api' (names, e.g. "
     "'BadgeService.AwardBadge') says whether an API is current or deprecated. Trust labels: 'Roblox docs' and 'Measured' beat "
     "'Rule of thumb'. Cite note titles you used.",
     schema({"action": {"type": "string", "enum": ["search", "read", "sections", "list", "api"]},
             "query": {"type": "string", "description": "search: the task or bug in plain words"},
             "count": {"type": "integer", "description": "search: notes to return, default 3, at most 8"},
             "title": {"type": "string", "description": "read/sections: the note title from a search"},
             "heading": {"type": "string", "description": "read: words from the heading of the one section to read"},
             "folder": {"type": "string", "description": "list: a folder like '24 Worked Builds'; empty for the top level"},
             "names": {"type": "string", "description": "api: API names separated by spaces"}}, ["action"]), t_vault),
    ("rbx_doctor", "Check that Python libraries, Blender and Roblox Studio are found, and say how to fix what is missing.",
     schema({}), t_doctor),
    ("rbx_studio_snippet", "Get ready-made Luau to run in Roblox Studio through the Studio MCP's execute_luau: Emit and Lightning VFX "
     "modules, vfx_capture, icon_booth, place_model, export_rig, reset_data, vet_asset (check an inserted model's scripts), "
     "perf_capture (frame time, memory, instance budgets), session_recorder (what a playtest was like for the player), "
     "sound_probe (length and loudness of a sound). Each file's header says where to run it. With no name, lists them.",
     schema({"name": S}), t_snippet),
    ("rbx_blender", "Run Python in headless Blender against a saved .blend (created if missing). The scene persists between calls; "
     "variables do not. `bpy` and the `rbx` helpers are available (check_model, shot, export_model, import_rig, pose, bake_animation, "
     "vfx_arc, vfx_ring, flipbook, ...). Set `result = {...}` to return values. Renders named in the result come back as images.",
     schema({"code": {"type": "string", "description": "Python source"},
             "model": {"type": "string", "description": "Model name; uses work/models/<name>/<name>.blend"},
             "file": {"type": "string", "description": "Explicit .blend path instead of model"},
             "nosave": {"type": "boolean"}, "timeout": {"type": "integer", "description": "Seconds, default 600"}}, ["code"]), t_blender),
    ("rbx_texture", "Draw VFX particle textures and flipbook sheets with numpy. The code calls rbx.draw_texture / rbx.draw_flipbook and the "
     "helpers: tex_coords(size), tex_polar(x, y), tex_noise(size, scale, octaves, seed), tex_cells, tex_sample, tex_curve(points), "
     "tex_stroke(x, y, path, width, start, end), tex_blob(x, y, radius, lumps, roughness, seed), tex_warp, tex_edge(value, at, soft), "
     "tex_ease, ring(x, y, radius, width), glow(x, y, radius, power), flare(x, y, arms). numpy is available as np. "
     "Returns a pixel-level check of each file, a contact sheet image, and a free local Studio preview id.",
     schema({"code": S, "folder": {"type": "string", "description": "Subfolder of work/vfx, e.g. the effect name"},
             "no_preview": {"type": "boolean"}}, ["code"]), t_texture),
    ("rbx_check_images", "Check PNG textures or flipbook sheets made another way: grid, per-frame coverage, problems, and a contact sheet.",
     schema({"paths": {"type": "array", "items": S}, "no_preview": {"type": "boolean"}}, ["paths"]), t_check_images),
    ("rbx_icon", "Cut a transparent icon out of a screenshot of the Studio icon booth (see the 'workflow' guide, Icons).",
     schema({"screenshot": S, "name": S, "folder": S, "size": {"type": "integer"}}, ["screenshot", "name"]), t_icon),
    ("rbx_video", "Look at a video: action 'info' for metadata, or 'frames' for frames tiled into a sheet (optionally start/end seconds and count).",
     schema({"path": S, "action": {"type": "string", "enum": ["info", "frames"]}, "start": {"type": "number"},
             "end": {"type": "number"}, "count": {"type": "integer"}}, ["path"]), t_video),
    ("rbx_animation", "Turn an animation JSON baked in Blender (rbx.bake_animation) into a Roblox .rbxmx KeyframeSequence to upload.",
     schema({"json": S}, ["json"]), t_animation),
    ("rbx_studio_lock", "When several agents share one Studio place: take turns with its single playtest and camera. Call with action "
     "'acquire' before start_stop_play, a VFX capture, the icon booth or a bulk change, and 'release' as soon as you finish. 'status' shows who holds it.",
     schema({"action": {"type": "string", "enum": ["acquire", "release", "status"]}}, ["action"]), t_lock),
    ("rbx_generate_texture", "Get a generated TEXTURE from Google's Gemini. With the user's Gemini API key it is automatic. Without one, this call shows the "
     "prompt to the user, who makes the image in their own Gemini chat, and WAITS (up to 10 minutes) for them to paste or copy it; "
     "it then returns as normal, so just call it and tell the user first that a prompt is waiting for them. For plain tileable "
     "surfaces on parts, prefer Roblox Studio's free generate_material tool. "
     "kind 'surface' = a seamless tileable material (wood, stone, metal, fabric, ground); 'particle' = one white VFX sprite, returned on "
     "transparency; 'decal' = one isolated mark (scorch, crack, stain) on transparency. ONLY for textures: never for UI, icons, logos, "
     "typography or any graphic design, and never for images containing text. Returns the file, a review image to look at, and a free "
     "local Studio preview id. Nothing is uploaded.",
     schema({"prompt": {"type": "string", "description": "What the texture shows, e.g. 'weathered oak planks' or 'jagged impact star'"},
             "kind": {"type": "string", "enum": ["surface", "particle", "decal"]},
             "name": {"type": "string", "description": "File name without extension"},
             "folder": {"type": "string", "description": "Subfolder of work/textures"},
             "size": {"type": "integer", "description": "Pixels per side; default 1024 for surface, 512 otherwise"},
             "seamless": {"type": "boolean", "description": "surface only; default true"},
             "reference": {"type": "string", "description": "Path to an image whose style to match (API mode only)"},
             "from_image": {"type": "string", "description": "Path to an image the user already made: process it instead of generating"}},
            ["prompt"]), t_gen),
    ("rbx_vfx_package", "Build the Emit and Lightning modules into one file staged for Studio, and return the one-line Luau that installs "
     "them into ReplicatedStorage.VFX without uploading or pasting source.", schema({}), t_package),
    ("rbx_project", "The game's project folder: the scripts of the open Studio place as files, with checkpoints in git. Talks to Studio "
     "itself, so script sources never pass through this chat. Actions: 'info' (project folder, GAME.md, git branch and recent "
     "checkpoints, the open place); 'export_scripts' (write every Script, LocalScript and ModuleScript to <project>/src with Rojo-style "
     "names; returns counts written, unchanged, removed); 'checkpoint' (export, then commit with `message` if the project is a git "
     "repository, and leave a marker in Studio's undo history; returns the commit and changed files); 'changes' (which scripts in "
     "the place differ from the last export, with line counts; writes nothing); 'restore_scripts' (write script sources from src, "
     "or from `commit`, back into the place, only where they differ; it only REPORTS what would change unless `confirm` is true). "
     "Checkpoints cover SCRIPTS ONLY: parts, UI, terrain and other instances are not saved by this tool (Studio's undo and the "
     "place's version history cover those), and a restored script is not recreated if its instance was deleted. Needs Studio open.",
     schema({"action": {"type": "string", "enum": ["info", "export_scripts", "checkpoint", "changes", "restore_scripts"]},
             "message": {"type": "string", "description": "checkpoint: a few words saying what this checkpoint is"},
             "commit": {"type": "string", "description": "restore_scripts: commit to restore from; default is the files now in src"},
             "paths": {"type": "array", "items": S, "description": "restore_scripts: only these scripts. Files or folders under src "
                       "(src/ServerScriptService/Shop.server.luau) or instance paths (ServerScriptService.Shop)"},
             "confirm": {"type": "boolean", "description": "restore_scripts: true to really overwrite scripts in the place. Default "
                         "false: a dry run that lists what would change"}}, ["action"]), t_project),
    ("rbx_economy", "Simulate a game's numbers before building them. Mode 'progression': give income (per second, or per action with "
     "actions per minute), upgrades (base_cost, growth flat / linear / polynomial / exponential or a list of costs, max_level, effect "
     "add or multiply, optional requires), a buying policy (cheapest, payback, in_order), and optionally a rebirth rule, a session "
     "model and milestones. Returns a purchase timeline, income over time, time to first purchase and to finish, the longest wait and "
     "where it is, and flags: first purchase over a minute, walls (a wait over 3x any before it), runaway inflation, content gone in "
     "under an hour. Mode 'drops': give items with weights or probabilities and optional pity; returns exact odds, expected "
     "attempts per item and for the full set, and p50/p90/p99. It models one player who buys the moment they can afford it, earning "
     "only while playing: a guide to the shape of the curve, not a forecast. Call with example true for the input format.",
     schema({"config": {"type": "object", "description": "The economy or drop table. Progression: {income: {per_second | per_action, "
                        "actions_per_minute}, upgrades: [{name, base_cost, growth: {type, rate | step | power}, max_level, effect: "
                        "{type: add | multiply, amount | factor}}], policy, rebirth, session: {minutes, per_day}, milestones, max_hours}. "
                        "Drops: {mode: 'drops', items: [{name, weight | probability, pity}], set, trials, seed}"},
             "example": {"type": "boolean", "description": "true: return example inputs and the full field list instead of simulating"}}),
     t_economy),
    ("rbx_research", "What is working on Roblox right now, from the public charts the website shows a signed-out visitor (nothing is "
     "sent but a random session id). Action 'snapshot': a genre table (players, like ratios, how much of each genre is young games, how "
     "many are in Top Earning and Up-and-Coming), observations worth a second look, new games already charting, and the top 15 of "
     "each main chart. Action 'chart' (chart id, e.g. 'top-earning' or 'trending-in-survival'): one chart in full. Takes about a "
     "minute. Players are a count at this moment; only about the top hundred of each chart are visible.",
     schema({"action": {"type": "string", "enum": ["snapshot", "chart"]},
             "chart": {"type": "string", "description": "chart: the chart's id, as listed at the end of a snapshot"}}, ["action"]), t_research),
    ("rbx_market", "Look up live Roblox experiences from Roblox's public web APIs (no login; only the search words or ids are sent). "
     "Action 'search' (query, limit up to 20) or 'details' (universe_ids). Each result: name, universe id, place id, players now, total "
     "visits, like ratio, created and updated dates, genre, first 200 characters of the description. Sorted by players now. Search "
     "also returns loosely related popular experiences, so read the names. Numbers are a snapshot at the time of the call.",
     schema({"action": {"type": "string", "enum": ["search", "details"]},
             "query": {"type": "string", "description": "search: the words to look for, e.g. 'pet simulator'"},
             "limit": {"type": "integer", "description": "search: how many to return, 1 to 20 (default 10)"},
             "universe_ids": {"type": "array", "items": {"type": "integer"}, "description": "details: universe ids (not place ids), up to 50"}},
            ["action"]), t_market),
    ("rbx_board", "The game's board, shown to the user in the app's Board tab: columns todo (planned), doing (being worked on now), "
     "needs (something only the user can do or decide) and done. Only for chats that belong to a game project. Keep it true: "
     "'add' a card in doing when you start a real piece of work, 'move' it to done when it is finished and checked, add one in "
     "needs for each thing you need from the user (one clear action per card), and add planned work in todo. Action 'list' "
     "shows every card with its id. Titles are short and plain, like a task on a sticky note.",
     schema({"action": {"type": "string", "enum": ["list", "add", "move", "update"]},
             "title": {"type": "string", "description": "add/update: a few words, e.g. 'Shop menu' or 'Upload the coin sound'"},
             "note": {"type": "string", "description": "add/update: optional detail, one or two sentences"},
             "column": {"type": "string", "enum": ["todo", "doing", "needs", "done"], "description": "add (default doing) / move"},
             "id": {"type": "string", "description": "move/update: the card's id from list or add"}}, ["action"]), t_board),
    ("rbx_team", "Start extra agents through the Fridae Studios app and collect their reports. Only works when this chat runs inside the "
     "app. Action 'spawn' (role, name, task): role 'worker' does an independent piece of the job, 'critic' reviews work and reports "
     "problems; the task must be a full brief (at least 80 characters: what to do, where, what is done already, what to leave alone, "
     "what to report), because the new agent knows nothing about this chat. Returns an id and status, or says the app did not pick "
     "it up. 'status' (optional ids) lists tasks. 'wait' (ids, timeout seconds, default 600, at most 840) blocks until they are done "
     "or refused and returns each agent's final reply. Agents started this way cannot start more. They share the one Studio: use "
     "rbx_studio_lock around playtests.",
     schema({"action": {"type": "string", "enum": ["spawn", "status", "wait"]},
             "role": {"type": "string", "enum": ["worker", "critic"], "description": "spawn"},
             "name": {"type": "string", "description": "spawn: short title shown in the app, 34 characters at most"},
             "task": {"type": "string", "description": "spawn: the full instructions for the new agent"},
             "ids": {"type": "array", "items": S, "description": "status (optional) and wait: task ids returned by spawn"},
             "timeout": {"type": "integer", "description": "wait: seconds, default 600, at most 840"}}, ["action"]), t_team),
    ("rbx_promo", "Make vertical promo videos for a game (TikTok, Shorts, Reels): probe clips, render a recipe, check it. "
     "Read the guide first: rbx_guide topic `promo`. A long render returns a job id; poll it with action 'job'.",
     schema({"action": {"type": "string", "enum": ["status", "probe", "reference", "voice", "transcribe", "render", "check", "styles", "job"]},
             "paths": {"type": "array", "items": S, "description": "probe: clip files"},
             "path": {"type": "string", "description": "reference, transcribe: the media file; check: the rendered promo.mp4"},
             "recipe": {"type": "string", "description": "render, check: path of promo.json"},
             "text": {"type": "string", "description": "voice: the words to speak"},
             "voice": {"type": "string", "description": "voice: voice name, default af_heart"},
             "out": {"type": "string", "description": "output folder (probe, reference, styles, render) or wav file (voice)"},
             "id": {"type": "string", "description": "job: the id render returned"}}, ["action"]), t_promo),
]
HANDLERS = {name: fn for name, _, _, fn in TOOLS}


# --------------------------------------------------------------- protocol
def handle(msg):
    method = msg.get("method")
    mid = msg.get("id")
    params = msg.get("params") or {}
    if method == "initialize":
        return {"protocolVersion": params.get("protocolVersion", "2024-11-05"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "rbxkit", "version": VERSION},
                "instructions": INSTRUCTIONS}
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": [{"name": n, "description": d, "inputSchema": s} for n, d, s, _ in TOOLS]}
    if method == "tools/call":
        fn = HANDLERS.get(params.get("name"))
        if not fn:
            raise KeyError("unknown tool: %s" % params.get("name"))
        try:
            return fn(params.get("arguments") or {})
        except subprocess.TimeoutExpired:
            return result("The command timed out.", False)
        except Exception as e:  # report tool failures as tool errors, not protocol errors
            return result("%s: %s" % (type(e).__name__, e), False)
    if mid is None:
        return None  # a notification we do not act on
    raise NotImplementedError(method)


def main():
    stdin = sys.stdin.buffer
    # The protocol goes out on a private copy of fd 1. Fd 1 and sys.stdout then point at stderr before any thread
    # starts, so a stray print, a library or a child process writing to "stdout" can never corrupt the protocol,
    # and tools that capture stdout with redirect_stdout still work.
    out = os.fdopen(os.dup(1), "wb")
    sys.stdout.flush()
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    for raw in stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw.decode("utf-8"))
        except ValueError:
            continue
        mid = msg.get("id")
        try:
            res = handle(msg)
            if mid is None:
                continue
            reply = {"jsonrpc": "2.0", "id": mid, "result": res}
        except NotImplementedError as e:
            reply = {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "method not found: %s" % e}}
        except Exception as e:
            reply = {"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": str(e)}}
        out.write((json.dumps(reply) + "\n").encode("utf-8"))
        out.flush()


if __name__ == "__main__":
    main()
