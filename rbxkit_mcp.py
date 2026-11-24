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
from rbxpaths import KIT, VERSION, KitError, cli, mcp_command, no_window
MAX_IMAGE = 1_500_000
MAX_IMAGES = 4

# which guides to load for which kind of request lives in routing.txt (the app uses the same file)
INSTRUCTIONS = """rbxkit: Roblox Studio toolkit. It works beside the Roblox Studio MCP tools (execute_luau, screen_capture,
start_stop_play, ...). rbx_studio_snippet returns ready-made Luau to run in Studio. Tools that produce images return
them: always look before reporting a model, texture or icon as done.

{routing}- Generated textures: `rbx_generate_texture` makes surface materials, particle sprites and decals (through the user's Gemini
  key, or by asking the user to make the image in their Gemini chat). For plain tileable surfaces prefer Studio's generate_material.
  Use it only for textures, never for UI, icons, logos or typography.
- Generated images: `rbx_image` makes a picture from a prompt at an exact size and saves it in the game's project folder
  (on the user's own Gemini plan, through Antigravity; `transparent: true` cuts the subject out). Good for the game
  icon, thumbnail key art, UI art and concept art. Each picture costs some of the user's plan: make a few considered
  ones, never batches. Captured footage of the real game is still what shows gameplay; never another game's characters
  or art. Lettering and layout are still built natively with real fonts. rbx_image uploads nothing; uploads follow
  the guide's upload rules.
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
> icon -> rbx_icon, image -> rbx_image, video -> rbx_video, anim -> rbx_animation, package -> rbx_vfx_package. A file under
> `studio/` is fetched with rbx_studio_snippet. Another guide ("the roblox-vfx skill") is rbx_guide with
> that topic. Mentions of ToolSearch or skills are Claude Code details you can ignore.

"""

WEB_GUIDES = {"emil-design-eng", "motion-design", "redesign-skill", "reference-design-contract",
              "soft-skill", "minimalist-skill", "brutalist-skill"}
WEB_NOTE = ("> This guide was written for websites. For Roblox, take its judgement and translate its CSS with the "
            "`web-to-roblox` guide (rbx_guide topic web-to-roblox)." + chr(10) + chr(10))

GUIDES = {
    "workflow": ("vault/Fridae/Guides/Core/workflow/workflow.md", "Start here: tools, ground rules, 3D models, animations, icons, uploads"),
    "playtest": ("vault/Fridae/Guides/Core/playtest/playtest.md", "Running a playtest and acting like a player"),
    "ui-design": ("vault/Fridae/Guides/Core/ui-design/ui-design.md", "Building and checking Roblox UI"),
    "ui-helpers": ("vault/Fridae/Guides/Core/ui-design/helpers.lua", "Luau UI builder functions to paste into a builder"),
    "ui-audit": ("vault/Fridae/Guides/Core/ui-design/audit.lua", "Luau UI defect check to run in Studio"),
    "sim-ui": ("vault/Fridae/Guides/Core/sim-ui/sim-ui.md", "The chunky simulator / tycoon UI look"),
    "vfx": ("vault/Fridae/Guides/Core/vfx/vfx.md", "Visual effects: modules, capture, textures, meshes"),
    "vfx-basics": ("vault/Fridae/Guides/Core/vfx/references/vfx-basics.md", "VFX craft notes: layers, timing, brightness, budgets"),
    "tester": ("agents/roblox-tester.md", "Instructions for a sub-agent that only runs playtests"),
    "web-to-roblox": ("vault/Fridae/Guides/Core/ui-design/web-to-roblox.md", "How to apply the design guides below in Roblox (CSS to Roblox table)"),
    "emil-design-eng": ("vault/Fridae/Guides/Core/emil-design-eng/emil-design-eng.md", "Design: UI polish and when and how to animate"),
    "motion-design": ("vault/Fridae/Guides/Core/motion-design/motion-design.md", "Design: motion craft, beat maps, pacing"),
    "redesign-skill": ("vault/Fridae/Guides/Core/redesign-skill/redesign-skill.md", "Design: audit and upgrade existing UI"),
    "reference-design-contract": ("vault/Fridae/Guides/Core/reference-design-contract/reference-design-contract.md", "Design: turn references into a design contract"),
    "unslop": ("vault/Fridae/Guides/Core/unslop/unslop.md", "Design: turn sloppy, AI-looking or vibecoded UI into designed UI (audit, direction, nine passes)"),
    "soft-skill": ("vault/Fridae/Guides/Core/soft-skill/soft-skill.md", "Style: soft premium look"),
    "minimalist-skill": ("vault/Fridae/Guides/Core/minimalist-skill/minimalist-skill.md", "Style: clean minimal look"),
    "brutalist-skill": ("vault/Fridae/Guides/Core/brutalist-skill/brutalist-skill.md", "Style: raw industrial look"),
    "game-brainstorm": ("vault/Fridae/Guides/Core/game-brainstorm/game-brainstorm.md", "Game: interview the user about a new idea and write GAME.md"),
    "game-bible": ("vault/Fridae/Guides/Core/game-bible/game-bible.md", "Game: the GAME.md format and rules; writing one for an existing game"),
    "game-design": ("vault/Fridae/Guides/Core/game-design/game-design.md", "Game: core loop, first five minutes, feedback, pacing, fair retention, failure modes"),
    "economy-design": ("vault/Fridae/Guides/Core/economy-design/economy-design.md", "Game: prices, rewards, cost curves, drop odds, checking numbers with rbx_economy"),
    "player-testing": ("vault/Fridae/Guides/Core/player-testing/player-testing.md", "Game: testing like a new player; feel, performance budgets, devices, multi-player"),
    "sound-design": ("vault/Fridae/Guides/Core/sound-design/sound-design.md", "Game: sounds and music by measurement; timing, levels, SoundGroups, rolloff"),
    "market-research": ("vault/Fridae/Guides/Core/market-research/market-research.md", "Game: comparable live experiences with rbx_market and what to learn from them"),
    "critic": ("vault/Fridae/Guides/Core/critic/critic.md", "Instructions for an agent spawned as a reviewer of finished work"),
    "producer": ("vault/Fridae/Guides/Core/producer/producer.md", "Splitting a large request between parallel agents with rbx_team"),
    "studio": ("vault/Fridae/Guides/Core/studio/studio.md", "Studio mode, for the Lead: stages, gates, briefing experts, review items, decisions, waiting"),
    "promo": ("vault/Fridae/Guides/Core/promo/promo.md", "Promo videos: making a TikTok/Shorts/Reels promo for a game with rbx_promo"),
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


_CAPTURED = None


def note_pictures(paths):
    """Tell the app which image files this result names. An AI kept open over ACP (Gemini) hands the app no tool
    results, so the app reads them here: one JSON line per result in the file it names (FRIDAE_TOOL_PICTURES).
    Never in the way of the result itself."""
    if _CAPTURED is not None:     # inside a job (see "long calls"): kept with the result, noted when it is given
        if paths:
            _CAPTURED.append(list(paths))
        return
    where = os.environ.get("FRIDAE_TOOL_PICTURES")
    if not where or not paths:
        return
    try:
        with open(where, "a", encoding="utf-8") as f:
            f.write(json.dumps({"images": list(paths)}) + "\n")
    except OSError:
        pass


def result(text, ok=True, with_images=True):
    content = [{"type": "text", "text": text}]
    if with_images and ok:
        note_pictures(images_in(text))
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


GUIDE_GROUPS = (("vault/Fridae/Guides/Roblox subjects", "Roblox: "), ("vault/Fridae/Guides/LihFolk", "LihFolk: "))
LIHFOLK_GUIDES = "vault/Fridae/Guides/LihFolk/"


def _register_roblox_skills():
    """The vault's subject guides: each folder <name>/ holding <name>.md registers itself as a guide."""
    # LihFolk's guides (CC BY 4.0): ones whose subject rbxkit already covered were left out when they were copied in
    for rel, prefix in GUIDE_GROUPS:
        base = os.path.join(KIT, rel)
        for name in sorted(os.listdir(base)) if os.path.isdir(base) else []:
            path = os.path.join(base, name, name + ".md")
            if os.path.exists(path) and name not in GUIDES:
                GUIDES[name] = ("%s/%s/%s.md" % (rel, name, name), prefix + _describe(path))


_register_roblox_skills()


def _guide_path(topic):
    """The file behind a guide topic or a reference topic like 'roblox-luau/type-system', or None."""
    if topic in GUIDES:
        return os.path.join(KIT, GUIDES[topic][0])
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
            return path
    return None


def _reference(topic):
    """A topic like 'roblox-luau/type-system' is a file in that guide's references folder."""
    if "/" not in topic:
        return None
    path = _guide_path(topic)
    if not path:
        return None
    with open(path, encoding="utf-8") as f:
        return fill(f.read())


def _guide_extras(topic):
    """What follows a guide's own text: where its reference files and other files are, and its credit."""
    if topic not in GUIDES:
        return ""
    path = os.path.join(KIT, GUIDES[topic][0])
    text = ""
    refs = os.path.join(os.path.dirname(path), "references")
    if topic != "vfx" and os.path.isdir(refs):
        files = sorted(f[:-3] for f in os.listdir(refs) if f.endswith(".md"))
        if files:
            text += "\n\nReference files for this guide (load with rbx_guide topic `%s/<name>`): %s" % (topic, ", ".join(files))
    if topic == "vfx" and os.path.isdir(refs):
        text += "\n\nReference files present: " + ", ".join(sorted(f for f in os.listdir(refs) if f.endswith(".md")))
    if GUIDES[topic][0].startswith(LIHFOLK_GUIDES):
        # its code (Luau modules, examples, Blender scripts, rubrics) is read straight from the folder
        folder = os.path.dirname(path)
        files = sorted(os.path.relpath(os.path.join(r, f), folder).replace("\\", "/")
                       for r, _, fs in os.walk(folder) for f in fs if os.path.join(r, f) != path)
        text += "\n\nThis guide's folder is %s" % folder.replace("\\", "/")
        if files:
            text += "; read its files from there: " + ", ".join(files)
        text += ("\n\n(From LihFolk Vault V2 by LihFolk, CC BY 4.0. Search the full vault with rbx_vault. Where a step "
                 "names a slash command or agent from the LihFolk plugin, do that step yourself.)")
    return text


def _one_guide(topic):
    path = os.path.join(KIT, GUIDES[topic][0])
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as f:
        text = fill(f.read())
    if text.startswith("---"):
        text = text.split("---", 2)[2].lstrip()  # drop the skill front matter
    if topic in WEB_GUIDES:
        text = WEB_NOTE + text
    return text + _guide_extras(topic)


# Claude Code skill names (the workflow guide's table uses them) -> rbx_guide names. install.py builds the skill
# folders with the first six names; the last four are those skills' front-matter names.
GUIDE_ALIASES = {"roblox-studio-workflow": "workflow", "roblox-playtest": "playtest", "roblox-ui-design": "ui-design",
                 "roblox-sim-ui": "sim-ui", "roblox-vfx": "vfx", "roblox-promo": "promo",
                 "industrial-brutalist-ui": "brutalist-skill", "minimalist-ui": "minimalist-skill",
                 "redesign-existing-projects": "redesign-skill", "high-end-visual-design": "soft-skill"}


def resolve_alias(topic):
    """A guide name as rbx_guide knows it, also for 'alias/reference' topics."""
    head, sep, rest = topic.partition("/")
    return GUIDE_ALIASES.get(head, head) + sep + rest


# Guides an agent always gets whole: they hold rules that must be followed, not looked up (see DEVELOPING.md,
# "How agents find knowledge"). Any guide under OUTLINE_OVER bytes is also always whole.
ALWAYS_WHOLE = ("workflow", "studio", "critic", "producer", "game-bible")
OUTLINE_OVER = 12 * 1024


# An outline always carries a guide's rules in full: sections whose heading reads like a rule, plus named ones.
RULES_HEADING = re.compile(r"\b(rules?|do not|don'?t|never|must|always|checklist)\b", re.I)
ALWAYS_SECTIONS = {  # rules-like sections the heading pattern misses
    "roblox-blender-modeling": ["Pipeline (do every step)", "Known traps"],
    "motion-design": ["Quality while composing"],
}
RULES_CAP = 8000     # bytes of rules sections an outline carries; past that they are listed by name
OPENING_CAP = 1500   # bytes of a guide's opening an outline carries


def _rules_sections(topic, p):
    """The sections an outline must show in full (outermost only: a rules section inside another is already there)."""
    named = [n.casefold() for n in ALWAYS_SECTIONS.get(topic, [])]
    out = []
    for s in p["sections"]:
        if not s["level"]:
            continue
        h = s["heading"].casefold()
        if RULES_HEADING.search(s["heading"]) or any(h.startswith(n) for n in named):
            if not any(o["start"] <= s["start"] < o["end"] for o in out):
                out.append(s)
    return out


def _guide_outline(topic, raw):
    """A guide's description, its opening (cut at OPENING_CAP), its rules sections in full and its section table."""
    import rbxvault
    p, lines = rbxvault.sections_listing(raw, "guides", topic)
    first = next((s["start"] for s in p["sections"] if s["level"]), None)
    if first is None:
        return None  # no headings to choose from: the guide comes whole
    opening = p["body"][:first].strip()
    if len(opening.encode("utf-8")) > OPENING_CAP:
        cut = opening.encode("utf-8")[:OPENING_CAP].decode("utf-8", "ignore")
        opening = cut[:cut.rfind("\n")].rstrip() if "\n" in cut else cut
        opening += "\n\n(Opening cut; read it whole with `section` \"opening\".)"
    head = ("> " + p["description"] + "\n\n") if p["description"] else ""
    if topic in WEB_GUIDES:
        head = WEB_NOTE + head
    rules, listed, used = [], [], 0
    for s in _rules_sections(topic, p):
        text = p["body"][s["start"]:s["end"]].strip()
        n = len(text.encode("utf-8"))
        if used + n > RULES_CAP:
            listed.append(s["heading"])
            continue
        rules.append(text)
        used += n
    body = head + opening
    if rules or listed:
        body += "\n\n## Rules from this guide (always shown)\n\n" + "\n\n".join(rules)
        if listed:
            body += "\n\nRules sections you must read (rbx_guide `section`): " + " · ".join(listed)
    size = len(raw.encode("utf-8"))
    return (body + "\n\n## Sections of this guide\n\n" + "\n".join("- " + l for l in lines) +
            "\n\n(Outline of a %s guide: its opening, its rules and its sections. Read the sections the task needs: "
            "rbx_guide topic `%s` with `section` set to heading words, or search with rbx_vault (shelf guides). "
            "`whole: true` returns all of it.)" % (rbxvault.size_text(size), topic))


def _guide_section(topic, heading):
    """One section of one guide (or reference file)."""
    import rbxvault
    path = _guide_path(topic)
    if not path or not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as f:
        raw = fill(f.read())
    p, lines = rbxvault.sections_listing(raw, "guides", topic)
    s = rbxvault.find_section(p, heading)
    if s is None:
        raise KitError("No section like '%s' in the %s guide. Its sections:\n%s" % (heading, topic, "\n".join(lines)))
    body = p["body"][s["start"]:s["end"]].rstrip()
    if not s["level"] and p["description"]:
        body = "> " + p["description"] + "\n\n" + body.lstrip()
    sub = [(x["level"], x["heading"]) for x in p["sections"] if s["start"] < x["start"] < s["end"]]
    return "Section '%s' (%s)\n\n%s" % (s["heading"], rbxvault.size_text(len(body.encode("utf-8"))),
                                        rbxvault.cap_text(body, sub, topic, "guides"))


def guide_text(topic, whole=False, outline=False):
    """What rbx_guide returns for one topic: the whole guide, or its outline when asked for one or when it is large
    and not on ALWAYS_WHOLE."""
    text = _one_guide(topic) if topic in GUIDES else _reference(topic)
    if text is None or whole:
        return text
    if not outline and (topic in ALWAYS_WHOLE or len(text.encode("utf-8")) <= OUTLINE_OVER):
        return text
    path = _guide_path(topic)
    if not path.endswith(".md"):
        return text
    with open(path, encoding="utf-8") as f:
        raw = fill(f.read())
    outline = _guide_outline(topic, raw)
    return text if outline is None else outline + _guide_extras(topic)


GUIDE_CAP = 30000         # bytes one rbx_guide call returns at most to a helper agent (past the first guide)
GUIDE_CAP_CHAT = 120000   # and to an ordinary chat, which loads its guides in one call (routing.txt)


def _guide_cap():
    return GUIDE_CAP if (os.environ.get("RBXKIT_AGENT_ROLE") or "").strip() else GUIDE_CAP_CHAT


def t_guide(a):
    """One or several guides in a single call."""
    wanted = [str(t).strip().lower() for t in (a.get("topics") or [])]
    if a.get("topic"):
        wanted.insert(0, str(a["topic"]).strip().lower())
    wanted = list(dict.fromkeys(resolve_alias(t) for t in wanted if t))
    if not wanted:
        lines = ["Guide topics (call rbx_guide with one, or several in `topics`):"] + ["- %s: %s" % (k, v[1]) for k, v in GUIDES.items()]
        return result("\n".join(lines), True, False)
    section = str(a.get("section") or "").strip()
    if section:
        topic = wanted[0]
        try:
            text = _guide_section(topic, section)
        except KitError as e:
            return result(str(e), False)
        if text is None:
            return result("No such guide: %s. Call rbx_guide with no arguments to list the topics." % topic, False)
        note = ("\n\n(`section` reads one guide; not loaded: %s.)" % ", ".join(wanted[1:])) if len(wanted) > 1 else ""
        return result(PREFACE + "=" * 20 + " GUIDE: " + topic + " " + "=" * 20 + "\n\n" + text + note, True, False)
    whole = a.get("whole") is True or str(a.get("whole")).lower() == "true"
    outline = a.get("outline") is True or str(a.get("outline")).lower() == "true"
    cap = _guide_cap()
    out, unknown, left, size = [], [], [], len(PREFACE.encode("utf-8"))
    for topic in wanted:
        # a rules guide bundled with others comes whole; only a call for that one guide alone gets its outline
        text = guide_text(topic, whole, outline and not (topic in ALWAYS_WHOLE and len(wanted) > 1))
        if text is None:
            unknown.append(topic)
            continue
        part = "=" * 20 + " GUIDE: " + topic + " " + "=" * 20 + "\n\n" + text
        n = len(part.encode("utf-8")) + 2
        if out and (left or size + n > cap):
            left.append(topic)  # the first guide always comes whole; the rest only while the answer stays small
            continue
        out.append(part)
        size += n
    if not out:
        return result("No such guide: " + ", ".join(unknown) + ". Call rbx_guide with no arguments to list the topics.", False)
    body = PREFACE + "\n\n".join(out)
    if left:
        body += ("\n\n(Left out to keep this answer under %d KB: %s. Ask for them in another rbx_guide call, two topics "
                 "at a time.)" % (cap // 1000, ", ".join(left)))
    if unknown:
        body += "\n\n(No such guide: " + ", ".join(unknown) + ")"
    return result(body, True, False)


def t_snippet(a):
    import rbxstudio
    if rbxstudio.studio_off():
        return result(rbxstudio.STUDIO_OFF, False)
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


def t_image(a):
    """An AI-generated image, or an existing one, saved in the game's project at an exact size (rbximage). The
    answer names the saved file and carries the picture, as the other picture tools' do."""
    import rbximage
    try:
        out = rbximage.run(a.get("action"), a)
    except KitError as e:
        return result(str(e), False)
    return result(json.dumps(out, indent=1, ensure_ascii=False))     # paths as they are, so the picture is found


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


def t_stage(a):
    import rbxstage
    return _direct(rbxstage.run, a.get("action") or "status", a)


def t_vault(a):
    import rbxvault
    return _direct(rbxvault.run, a.get("action") or "search", a)


# ------------------------------------------------------------ promo tool
# A render can outlast the client's call timeout, so it runs in a thread and the call returns a job id once it
# would take longer than PROMO_SYNC_SECONDS. The child processes rbxpromo starts never read our stdin (the
# protocol channel) and never write our stdout.
PROMO_SYNC_SECONDS = 60
PROMO_JOB_WAIT_SECONDS = 45     # action 'job' waits this long for a running job before replying "running"
PROMO_JOB_READ_TTL = 300        # a finished job is forgotten this long after its result was read
PROMO_JOB_KEEP = 3600           # ... or this long after it finished if nobody read it
_JOBS = {}
_JOBS_LOCK = threading.Lock()


def _promo_wait(seconds):
    """How long a promo call waits for a render: as ever, or less where the AI cuts a call off sooner (see "long calls")."""
    budget = _budget()
    return seconds if budget is None else max(0.0, min(seconds, budget - BUDGET_SLACK))


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
                                   "next": "Still running. Call rbx_promo again with action 'job' and this id (each call waits up to %d s)." % _promo_wait(PROMO_JOB_WAIT_SECONDS)}), True, False)
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
        job["thread"].join(_promo_wait(PROMO_JOB_WAIT_SECONDS))      # well under the client's 600 s call timeout
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
            job["thread"].join(_promo_wait(PROMO_SYNC_SECONDS))
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


# -------------------------------------------------------------- long calls
# Some AIs cut every call to a tool server off after a fixed time, whatever the server asks for (measured: Cursor
# after 60 seconds, Gemini's kept process after three minutes). For those the app sets RBXKIT_CALL_BUDGET in this
# server's entry: the seconds a call may take before it must answer. With no budget nothing in this section runs and
# every tool behaves as ever.
#
# With a budget, a tool runs as a job: a child process of this server (this same program, started with JOB_ENV set)
# that runs the one tool exactly as this server would have and hands its result back whole. A call that is not done
# when the budget runs out answers "not finished yet" with the job's name, and `rbx_job` picks the result up later.
# Calls are then answered each on a thread of its own, so a wait never holds another call up.
#
# Jobs run one at a time, in the order asked for, as calls did on the one thread (BESIDE lists the few that need not).
#
# A job child is the leader of a process group of its own. When a job ends, for any reason (by itself, by its own
# time limit, cancelled, its turn over, the server ending), what is left of that group is ended too, also what
# ignores SIGTERM: always before the leader is reaped, so the group's number cannot have become another's. A job
# lives JOB_LIFE at most. Jobs never outlive this server: it ends them when its input closes or it is told to end,
# and a small watcher process (REAPER_ENV) ends them should the server be killed outright. Jobs live in this
# process's memory only, and each chat has its own server: one chat cannot see, wait on or cancel another's.
BUDGET_SLACK = 2.0           # a call that waits on purpose (rbx_team wait, a promo render) stops this long before the budget
JOB_ENV = "RBXKIT_JOB_CHILD"
REAPER_ENV = "RBXKIT_JOB_REAPER"
KILL_GRACE = 0.3             # seconds between SIGTERM and SIGKILL for a job's process group
JOB_LIFE = 900.0             # the longest a job runs once started: the 15 minutes a process per message gives a call
LIFE_MARGIN = 20.0           # past a tool's own time limit, so the tool can say so itself first
# the seconds each tool's own code gives its work (run_rbx's limit); rbx_blender's is its `timeout` argument
OWN_LIMIT = {"rbx_doctor": 120, "rbx_texture": 600, "rbx_check_images": 600, "rbx_icon": 600, "rbx_video": 600,
             "rbx_animation": 600, "rbx_vfx_package": 600, "rbx_generate_texture": 720, "rbx_image": 600}
# answered by this process itself: instant, or tied to this process (the lock's owner, the promo renders it keeps)
INLINE = ("rbx_guide", "rbx_studio_snippet", "rbx_studio_lock", "rbx_board")
PROMO_INLINE = (None, "", "job", "render", "install")
# The tools that may run beside any other job, each with why. Everything else takes its turn: tools share files
# (two texture calls write one folder's _check.png; Blender, a package and an icon read and write one model's files;
# a checkpoint commits the project folder while another tool writes into it).
BESIDE = {
    "rbx_market": "reads Roblox's public web APIs and writes no file",
    "rbx_research": "reads the public charts and writes no file",
    "rbx_economy": "a calculation on its own arguments; writes no file",
    "rbx_doctor": "looks for installed programs; writes no file",
}
STILL = "NOT FINISHED YET"

_LONG = {}                   # job name -> _Job, in the order they were started
_LONG_LOCK = threading.RLock()
_LANE_TURN = threading.Condition(_LONG_LOCK)
_INLINE_LOCK = threading.Lock()
_NOTES_LOCK = threading.Lock()
_REAPER = None
_WATCHING = False
_GIVEN_UP = set()            # ids of calls the client cancelled: no answer is written, and no result is spent on them
_NO_ANSWER = object()


def _budget():
    """The seconds a call may take before it must answer (RBXKIT_CALL_BUDGET), or None: no limit, all as ever."""
    try:
        b = float(os.environ.get("RBXKIT_CALL_BUDGET") or 0)
    except ValueError:
        return None
    return b if b > 0 else None


def _job_life(tool, args):
    """The longest this job may run once started: the tool's own time limit where it has one (and a little more, so
    the tool reports it itself), otherwise JOB_LIFE; never more than JOB_LIFE."""
    own = OWN_LIMIT.get(tool)
    try:
        if tool == "rbx_blender":
            own = int(args.get("timeout") or 600)
        elif tool == "rbx_test_slow" and args.get("life"):
            return float(args["life"])
    except (TypeError, ValueError):
        own = 600
    return JOB_LIFE if own is None else max(1.0, min(own + LIFE_MARGIN, JOB_LIFE))


def _beside(tool, args):
    """Whether this call may run beside other jobs instead of taking its turn."""
    return tool in BESIDE or (tool == "rbx_test_slow" and bool(args.get("beside")))


class _Job:
    def __init__(self, tool, args):
        import secrets
        self.name = "job-" + secrets.token_hex(6)
        self.tool, self.args = tool, args
        self.queued = not _beside(tool, args)       # takes its turn behind the jobs before it
        self.life = _job_life(tool, args)
        self.began = time.monotonic()
        self.noted = time.time()                    # for the app's "turn ended" line, which carries its own time
        self.state = "queued" if self.queued else "running"
        self.proc = None
        self.started = None                         # when its process started (its life runs from here, not from the queue)
        self.closing = False                        # its output has ended: too late to call it expired
        self.result = None
        self.pictures = []
        self.cancelled = False                      # cancelled or dropped: it has no result
        self.expired = False                        # ended for running longer than its life
        self.gate = threading.Lock()                # held while its group is signalled, and while its leader is reaped
        self.done = threading.Event()


def _note_job(line):
    """One line for the app in the file it named (FRIDAE_TOOL_JOBS): which jobs were started and which were given
    back, so it can say when a turn ended with work thrown away. Never in the way of the work itself."""
    where = os.environ.get("FRIDAE_TOOL_JOBS")
    if not where:
        return
    try:
        with _NOTES_LOCK:
            os.makedirs(os.path.dirname(where), exist_ok=True)
            with open(where, "a", encoding="utf-8") as f:
                f.write(json.dumps(line) + "\n")
    except OSError:
        pass


def _turn_lines(path):
    """The app's lines in that file: [{"turn": "ended", "at": <its clock>}]. The file is small and read whole."""
    try:
        with open(path, "rb") as f:
            data = f.read(1 << 20)
    except OSError:
        return []
    out = []
    for raw in data[:data.rfind(b"\n") + 1].splitlines():
        try:
            line = json.loads(raw.decode("utf-8"))
        except ValueError:
            continue
        if isinstance(line, dict) and line.get("turn") == "ended" and isinstance(line.get("at"), (int, float)):
            out.append(line["at"])
    return out


def _watch_turns(path):
    """The app's side of that file: a line {"turn": "ended", "at": t} means a turn is over, so every job noted up to
    then goes, with its result (answered with {"dropped": [names], "at": t}). A job started after `t` belongs to a
    later turn and stays, however late the line is read. Lines already there when the first job began are old."""
    seen = set(_turn_lines(path))
    while True:
        time.sleep(0.2)
        for at in _turn_lines(path):
            if at in seen:
                continue
            seen.add(at)
            with _LONG_LOCK:
                jobs = [j for j in _LONG.values() if j.noted <= at]
            _note_job({"dropped": _end_jobs(jobs), "at": at})


def _reaper_tell(sign, pid):
    """Tell the watcher process which process groups are jobs ('+') and which are gone ('-')."""
    global _REAPER
    if os.name != "posix":
        return
    with _LONG_LOCK:
        try:
            if _REAPER is None:
                env = dict(os.environ)
                env[REAPER_ENV] = "1"
                # in this server's own process group (no new one: the app counts groups a turn leaves behind)
                _REAPER = subprocess.Popen(mcp_command(), stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, env=env)
            _REAPER.stdin.write(("%s%d\n" % (sign, pid)).encode("ascii"))
            _REAPER.stdin.flush()
        except OSError:
            pass


def _reaper_main():
    """The watcher: reads job process groups from the server; when the server is gone, however it went, kills them."""
    import signal
    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, signal.SIG_IGN)
    groups = set()
    for raw in sys.stdin.buffer:
        line = raw.decode("ascii", "replace").strip()
        if line[:1] == "+" and line[1:].isdigit():
            groups.add(int(line[1:]))
        elif line[:1] == "-" and line[1:].isdigit():
            groups.discard(int(line[1:]))
    for g in groups:
        try:
            os.killpg(g, signal.SIGKILL)
        except OSError:
            pass


def _signal_group(proc, sig):
    import signal
    try:
        if os.name == "posix":
            os.killpg(proc.pid, getattr(signal, sig))
        elif sig == "SIGKILL":
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True, creationflags=no_window())
    except OSError:
        pass


def _group_has_others(proc):
    """Whether the job's process group still has a running member besides its leader (Unix; `ps`)."""
    if os.name != "posix":
        return False
    try:
        out = subprocess.run(["ps", "-A", "-o", "pid=,pgid=,stat="], capture_output=True, text=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return True     # cannot tell: treat it as having some
    for line in out.splitlines():
        f = line.split()
        if len(f) >= 3 and f[1] == str(proc.pid) and f[0] != str(proc.pid) and not f[2].startswith("Z"):
            return True
    return False


def _end_group(job):
    """End the job's whole process group: SIGTERM, then SIGKILL a moment later. Only while the leader is unreaped
    (`returncode` is None under the gate: only `_run_job` reaps, and under the same gate), so the group's number is
    still this job's."""
    with job.gate:
        p = job.proc
        if p is None or p.returncode is not None:
            return
        _signal_group(p, "SIGTERM")
        time.sleep(KILL_GRACE)
        _signal_group(p, "SIGKILL")


def _end_jobs(jobs):
    """End these jobs with everything they started; they have no result. Returns their names."""
    with _LONG_LOCK:
        for j in jobs:
            j.cancelled = True
            _LONG.pop(j.name, None)
        _LANE_TURN.notify_all()
    for j in jobs:
        _end_group(j)
    return [j.name for j in jobs]


def _drop_all():
    """Every job goes, running or finished and not yet given back. Returns their names."""
    with _LONG_LOCK:
        jobs = list(_LONG.values())
    return _end_jobs(jobs)


def _lane_ahead(job):
    """The job a queued one waits behind (call with the lock held)."""
    before = None
    for j in _LONG.values():
        if j is job:
            return before
        if j.queued and not j.done.is_set():
            before = j
    return before


def _expire(job):
    """The job has run longer than its life: it is ended with all it started, and that is its result."""
    if job.done.is_set() or job.cancelled or job.closing:
        return
    job.expired = True
    _end_group(job)


def _run_job(job):
    timer = None
    try:
        with _LONG_LOCK:
            while job.queued and not job.cancelled and _lane_ahead(job) is not None:
                _LANE_TURN.wait()
            if job.cancelled:
                return
            env = {k: v for k, v in os.environ.items() if k not in ("RBXKIT_CALL_BUDGET", "FRIDAE_TOOL_PICTURES", "FRIDAE_TOOL_JOBS")}
            env[JOB_ENV] = "1"
            own = {"start_new_session": True} if os.name == "posix" else {"creationflags": no_window() | 0x00000200}
            job.proc = p = subprocess.Popen(mcp_command(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, **own)
            job.state = "running"
            job.started = time.monotonic()
        _reaper_tell("+", p.pid)
        timer = threading.Timer(job.life, _expire, (job,))
        timer.daemon = True
        timer.start()
        try:
            p.stdin.write(json.dumps({"name": job.tool, "arguments": job.args}).encode("utf-8"))
            p.stdin.close()
        except OSError:
            pass
        out = p.stdout.read()       # to its end: the child has written its result and gone, or was ended
        job.closing = not job.expired
        timer.cancel()
        with job.gate:
            # whatever the job started and left running goes with it, before the leader is reaped
            if job.cancelled or job.expired or _group_has_others(p):
                _signal_group(p, "SIGTERM")
                if _group_has_others(p):
                    time.sleep(KILL_GRACE)
                _signal_group(p, "SIGKILL")
            p.wait()
        p.stdout.close()
        _reaper_tell("-", p.pid)
        if job.expired:
            job.result = result("%s was ended: it ran for %d seconds, longer than a job may (%d seconds for this one). "
                                "Everything it started was ended with it and it has no result. Do not wait for it again; "
                                "if the work is still needed, start it again in smaller parts, or tell the user it "
                                "could not be finished." % (job.tool, time.monotonic() - job.started, job.life), False)
            return
        try:
            got = json.loads(out.decode("utf-8"))
            job.result, job.pictures = got["result"], got.get("pictures") or []
        except (ValueError, KeyError, TypeError):
            job.result = result("%s ended without a result (exit code %s)." % (job.tool, p.returncode), False)
    except Exception as e:  # the job could not be run at all: that is its result
        job.result = result("%s: %s" % (type(e).__name__, e), False)
    finally:
        if timer is not None:
            timer.cancel()
        with _LONG_LOCK:
            job.state = "finished"
            job.done.set()
            _LANE_TURN.notify_all()


def _start_job(tool, args):
    global _WATCHING
    job = _Job(tool, args)
    with _LONG_LOCK:
        notes = os.environ.get("FRIDAE_TOOL_JOBS")
        if notes and not _WATCHING:
            _WATCHING = True
            threading.Thread(target=_watch_turns, args=(notes,), daemon=True).start()
        _LONG[job.name] = job
    _note_job({"job": job.name, "tool": job.tool, "at": "started"})
    threading.Thread(target=_run_job, args=(job,), daemon=True).start()
    return job


def _still_going(job):
    """What a call answers when the job is not done in time. Worded so that no model takes it for the tool's result or
    for a failure (checked with real turns on Cursor and Gemini: see DEVELOPING.md, "Long tool calls")."""
    with _LONG_LOCK:
        ahead = _lane_ahead(job) if job.state == "queued" else None
    if ahead is not None:
        what = ('%s, as job "%s", has not started. It is queued behind %s (%s), because the rbx tools that make or '
                'change files run one at a time, and it starts by itself when that ends.' % (job.tool, job.name, ahead.name, ahead.tool))
    else:
        what = ('%s is still working as job "%s" (%d seconds so far) and nothing has failed.'
                % (job.tool, job.name, time.monotonic() - job.began))
    return result('%s. This is not the tool\'s result: %s\n'
                  'The result comes from one call only: rbx_job with action "wait" and job "%s". Call it now, and call it '
                  'again every time it answers %s. Until it returns the result, do not report this work as done or as '
                  'failed, and do not start it again.' % (STILL, what, job.name, STILL), True, False)


def _wait_job(job, deadline, call=None):
    """The job's result if it comes before `deadline`, given once and exactly as the tool returned it; else 'not
    finished yet'. A call the client has given up on (`call`, its request id) takes nothing: the result stays for
    the next wait."""
    while not job.done.wait(min(0.25, max(0.0, deadline - time.monotonic()))):
        if call is not None and call in _GIVEN_UP:
            return _NO_ANSWER
        if time.monotonic() >= deadline:
            return _still_going(job)
    if call is not None and call in _GIVEN_UP:
        return _NO_ANSWER
    with _LONG_LOCK:
        if _LONG.pop(job.name, None) is None:
            return _no_job(job.name)        # another call took it, or it was cancelled meanwhile
    for paths in job.pictures:              # noted for the app now, with the call that hands them over
        note_pictures(paths)
    _note_job({"job": job.name, "at": "collected"})
    return job.result


def _no_job(name):
    with _LONG_LOCK:
        mine = ", ".join(_LONG) or "none"
    return result('No job "%s" in this chat: do not wait for it again. A job\'s result is given once, and jobs end when '
                  'the turn they were started in ends; start the work again if it is still needed. This chat\'s jobs '
                  'now: %s.' % (name, mine), False)


def t_job(a, deadline, call=None):
    action = (a.get("action") or "wait").lower()
    name = str(a.get("job") or a.get("id") or "").strip()
    with _LONG_LOCK:
        jobs = list(_LONG.values())
        job = _LONG.get(name)
        if job is None and not name and len(jobs) == 1 and action in ("wait", "cancel"):
            job = jobs[0]
        if action == "list":
            if not jobs:
                return result("No jobs are running in this chat.", True, False)
            lines = []
            for j in jobs:
                ahead = _lane_ahead(j) if j.state == "queued" else None
                state = ("queued behind %s" % ahead.name if ahead is not None else
                         'finished: collect its result with action "wait"' if j.done.is_set() else
                         "working, %d seconds so far" % (time.monotonic() - j.began))
                lines.append("%s  %s  %s" % (j.name, j.tool, state))
            return result("\n".join(lines), True, False)
    if action not in ("wait", "cancel"):
        return result("Give 'action': wait, list or cancel.", False)
    if job is None:
        return _no_job(name) if name else result("Give 'job': the job name from the %s answer. This chat's jobs now: %s."
                                                 % (STILL, ", ".join(j.name for j in jobs) or "none"), False)
    if action == "cancel":
        _end_jobs([job])
        _note_job({"job": job.name, "at": "collected"})
        return result("Cancelled %s (%s). Everything it started was ended; it has no result." % (job.name, job.tool), True, False)
    return _wait_job(job, deadline, call)


def _run_tool(fn, args):
    """One tool, with its failures reported as tool errors, not protocol errors."""
    try:
        return fn(args)
    except subprocess.TimeoutExpired:
        return result("The command timed out.", False)
    except Exception as e:  # report tool failures as tool errors, not protocol errors
        return result("%s: %s" % (type(e).__name__, e), False)


def _team_in_time(fn, args, deadline):
    """rbx_team under a budget. Its `wait` is already a wait: it returns inside the budget saying who is still
    working and to wait again; no job."""
    if (args.get("action") or "").lower() in ("wait", "wait_board"):
        left = int(deadline - time.monotonic() - BUDGET_SLACK)
        args = dict(args, timeout=max(1, min(int(args.get("timeout") or 600), left)))
    return fn(args)


def _budgeted(name, fn, args, deadline, call=None, placed=None):
    """A call under a budget: answered by `deadline`, with the tool's result or with 'not finished yet'. `placed` is
    set once the call has its place in the order of jobs (or needs none), so calls sent together queue as sent."""
    as_job = not (name == "rbx_job" or name in INLINE or name == "rbx_team" or (name == "rbx_promo" and args.get("action") in PROMO_INLINE))
    if placed is not None and not as_job:
        placed.set()
    if name == "rbx_job":
        return _run_tool(lambda a: t_job(a, deadline, call), args)
    if name in INLINE:
        with _INLINE_LOCK:
            return _run_tool(fn, args)
    if name == "rbx_team":
        return _run_tool(lambda a: _team_in_time(fn, a, deadline), args)
    if name == "rbx_promo" and args.get("action") in PROMO_INLINE:
        return _run_tool(fn, args)      # its renders are this process's own and already answer in time (_promo_wait)
    job = _start_job(name, args)
    if placed is not None:
        placed.set()
    return _wait_job(job, deadline, call)


def _job_main():
    """A job child: one tool, run as the server runs it, its result written whole to the server."""
    global _CAPTURED
    os.environ.pop(JOB_ENV, None)       # nothing the tool starts is a job child
    out = os.fdopen(os.dup(1), "wb")
    sys.stdout.flush()
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    req = json.loads(sys.stdin.buffer.read().decode("utf-8"))
    _CAPTURED = []
    res = _run_tool(_handler(req["name"]), req.get("arguments") or {})
    out.write(json.dumps({"result": res, "pictures": _CAPTURED}).encode("utf-8"))
    out.flush()


def _tiny_png(rgb, size=16):
    import struct, zlib
    rows = b"".join(b"\x00" + bytes(rgb) * size for _ in range(size))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def t_test_slow(a):
    """A stand-in for a slow tool, offered only under RBXKIT_TEST_TOOLS=1 (tests and live trials; never to users):
    it works for `seconds`, then returns a line with a word in it and one small picture."""
    import hashlib
    from rbxpaths import WORK
    word = re.sub(r"[^a-z0-9]", "", str(a.get("word") or "done").lower()) or "done"
    seconds = float(a.get("seconds") or 0)

    def log(what):
        if a.get("log"):
            with open(a["log"], "a", encoding="utf-8") as f:
                f.write("%s:%s\n" % (what, word))
    log("start")
    if a.get("child_pidfile"):      # something the job starts that will not go when asked (a shell: also in a packaged build)
        p = subprocess.Popen(["/bin/sh", "-c", 'trap "" TERM; while :; do sleep 600; done'],
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with open(a["child_pidfile"] + ".tmp", "w") as f:
            f.write(str(p.pid))
        os.replace(a["child_pidfile"] + ".tmp", a["child_pidfile"])
    time.sleep(seconds)
    log("end")
    if a.get("fail"):
        return result("The stand-in tool failed on purpose (%s)." % word, False)
    folder = os.path.join(WORK, "test-tool")
    os.makedirs(folder, exist_ok=True)
    path = os.path.join(folder, word + "_check.png")
    with open(path, "wb") as f:
        f.write(_tiny_png(hashlib.sha256(word.encode()).digest()[:3]))
    return result('The stand-in tool worked for %g seconds and finished. Its result word is %s. It made "%s"'
                  % (seconds, word.upper(), path.replace("\\", "/")))


JOB_TOOL = ("rbx_job", "Pick up work that one of the rbx tools is still doing. A tool that needs longer than one call may take "
            "answers NOT FINISHED YET with a job name instead of its result. Action 'wait' (job) then returns that tool's "
            "real result, text and pictures, or NOT FINISHED YET again: call it again until the result comes. 'list' shows "
            "this chat's jobs; 'cancel' (job) ends one and everything it started. A result is given once, and jobs end "
            "when the turn ends.",
            {"type": "object", "properties": {"action": {"type": "string", "enum": ["wait", "list", "cancel"]},
                                              "job": {"type": "string", "description": "wait, cancel: the job name from the NOT FINISHED YET answer"}},
             "required": ["action"]}, None)
TEST_TOOL = ("rbx_test_slow", "A stand-in for a slow tool, present in test runs only: it works for `seconds`, then returns one "
             "line with a result word in it and one small picture.",
             {"type": "object", "properties": {"seconds": {"type": "number"}, "word": {"type": "string"}}, "required": ["seconds"]},
             t_test_slow)


def _extra_tools():
    """Tools beyond the fixed list: rbx_job where calls have a budget, the stand-in in test runs."""
    extra = [JOB_TOOL] if _budget() is not None else []
    if os.environ.get("RBXKIT_TEST_TOOLS") == "1":
        extra.append(TEST_TOOL)
    return extra


def _handler(name):
    fn = HANDLERS.get(name)
    if fn is None:
        fn = next((f or t_job for n, _, _, f in _extra_tools() if n == name), None)
    return fn


def schema(props, required=()):
    return {"type": "object", "properties": props, "required": list(required)}


S = {"type": "string"}
TOOLS = [
    ("rbx_guide", "Read rbxkit guides that must be read in full (always 'workflow'), or one section of a guide. A guide over 12 KB "
     "comes as its opening plus a list of its sections with sizes (except workflow, studio, critic, producer, game-bible, "
     "which always come whole): then ask for the sections the task needs with `section`. For anything else, look it up "
     "with rbx_vault (shelf guides) and read only the sections that apply. One answer holds about 30 KB at most for a "
     "helper agent (120 KB for a chat): topics left out are named. With no arguments, lists the topics.",
     schema({"topics": {"type": "array", "items": {"type": "string"},
                        "description": "Guide names: workflow, playtest, ui-design, ui-helpers, ui-audit, sim-ui, vfx, vfx-basics, tester, web-to-roblox, "
                                       "emil-design-eng, motion-design, redesign-skill, reference-design-contract, "
                                       "soft-skill, minimalist-skill, brutalist-skill, and the Roblox subject "
                                       "guides (roblox-luau, roblox-architecture, roblox-security, roblox-networking, roblox-data-stores, roblox-ui, "
                                       "roblox-world-building, ...). Call with no arguments for the full list"},
             "topic": {"type": "string", "description": "A single guide name (same as one entry in topics)"},
             "section": {"type": "string", "description": "Heading words: return only that section of the guide in `topic` "
                                                          "(or the first of `topics`)"},
             "whole": {"type": "boolean", "description": "true returns large guides in full instead of their outline"},
             "outline": {"type": "boolean", "description": "true returns each guide's outline (opening, rules sections in "
                                                          "full, section table) whatever its size"}}), t_guide),
    ("rbx_vault", "One lookup for Roblox knowledge (LihFolk Vault V2: 1,300+ notes dated October 2026), Fridae's own guides, "
     "the Studio team's expert notes and this game's own notes (GAME.md with its decision log, lessons). Search first, then "
     "read ONE section. 'search' (query: the task or bug in plain words; shelf: all, vault, guides, team or game; k: results, "
     "default 5) returns ranked sections labelled by shelf and by kind, source (or 'this game') and month, each with a "
     "snippet, its size and the exact arguments to read it; up to two may be notes the first results link to, marked "
     "'linked from'; vault results add matching symptom lines. Optional filters: game ('this' or 'general'), kind, "
     "source, since (a date). When the knowledge index is on, search finds sections by meaning as "
     "well as by words. 'read' (title, heading words, optional shelf) returns one section in "
     "full (about 12 KB at most). 'sections' (title) lists a file's headings with sizes, 'list' (folder) browses the vault, "
     "'api' (names, e.g. 'BadgeService.AwardBadge') says whether an API is current or deprecated. Use the vault for limits, "
     "quotas, prices, policy and anything changed in 2025-2026. Trust labels: 'Roblox docs' and 'Measured' beat 'Rule of "
     "thumb'. Cite note titles you used. The user's own notes: shelf 'yours' (how they like things, every game) and "
     "'game' (this game) hold what the user taught agents; search them before design or build work. 'note' (text, at "
     "most 1,500 characters; optional title; scope game or all; rule: true for a standing preference, one short "
     "sentence added to the House rules every agent gets) saves one; 'notes' lists them, 'forget' (title, scope) moves "
     "one to the trash, 'rule_edit' (scope, old: words of a rule, text: new wording or empty to remove) changes a rule. "
     "A note that makes older ones out of date names them in 'supersedes': they are kept, marked, ranked below it "
     "and shown as '(replaced by ...)'. 'note' refuses a near-duplicate (it shows the existing note) unless force is "
     "true, and lists notes the new one may contradict for you to resolve. 'upkeep' writes a report of duplicates, "
     "contradictions, stale and broken links in the notes folder.",
     schema({"action": {"type": "string", "enum": ["search", "read", "sections", "list", "api", "note", "notes",
                                                   "forget", "rule_edit", "upkeep"]},
             "query": {"type": "string", "description": "search: the task or bug in plain words"},
             "shelf": {"type": "string", "enum": ["all", "vault", "guides", "team", "game", "yours"],
                       "description": "search/read/sections: where to look; default all (game only when the chat has a game project)"},
             "text": {"type": "string", "description": "note: what you were doing, what the user wants instead and why "
                      "(their words where you can); rule_edit: the rule's new wording, empty to remove it"},
             "scope": {"type": "string", "enum": ["game", "all"], "description": "note/notes/forget/rule_edit: this "
                       "game, or everything (default game when the chat has a game project)"},
             "rule": {"type": "boolean", "description": "note: true adds one House rule (only for a standing "
                      "preference: 'always', 'never', 'from now on')"},
             "replace": {"type": "boolean", "description": "note: true updates the note with exactly this title instead of adding another"},
             "supersedes": {"type": "string", "description": "note: the exact title of an older note this one "
                            "replaces (several: separate with ' | '); it is kept and marked, and ranks below"},
             "force": {"type": "boolean", "description": "note: true files it even when a near-duplicate exists"},
             "old": {"type": "string", "description": "rule_edit: a few words of the rule to change"},
             "k": {"type": "integer", "description": "search: results to return, default 5 (3 for shelf vault), at most 8"},
             "game": {"type": "string", "enum": ["this", "general", "all"], "description": "search: 'this' = only "
                      "this game's own notes, 'general' = only notes for every game; default all"},
             "kind": {"type": "string", "description": "search: only these kinds (comma-separated): rule, lesson, "
                      "taste, reference, guide"},
             "source": {"type": "string", "description": "search: only these sources (comma-separated): lihfolk, "
                        "user, agent, community, game, fridae"},
             "since": {"type": "string", "description": "search: only notes dated on or after this (YYYY, YYYY-MM or "
                       "YYYY-MM-DD); only notes with a date in their front matter match, undated notes are left out"},
             "count": {"type": "integer", "description": "search: same as k (older name)"},
             "title": {"type": "string", "description": "read/sections: the title from a search result; note: optional "
                       "title; forget: the note's title"},
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
    ("rbx_image", "Make an AI-generated image, or keep one that already exists, saved in the game's project folder at an exact size. "
     "Good for the game icon, thumbnail key art, UI art and concept art. Action 'generate' makes a new picture from `prompt` "
     "(through Google's Antigravity on the user's own Gemini plan; it can take a minute or two). Each picture costs some of "
     "that plan: make a few considered ones, never batches; a chat may start 12 in an hour and a game's whole team 30 "
     "(generated textures count too), and the user can switch this off in Setup. Action 'keep' brings an existing image "
     "file in instead and costs nothing: `source` is its full path (the file your own image tool made, or an image in the "
     "project, this chat's own folder, or the app's images, icons, textures and shots). Either "
     "way the picture is cropped about its middle to the shape of `size` and resized to exactly that; it is never "
     "stretched, and an existing file is never replaced. With `transparent: true` the subject is cut out onto a "
     "transparent background and placed whole inside `size` (an item or shop icon): generate asks for it on a flat "
     "colour and removes that; hard-edged subjects cut out cleanly, soft ones (smoke, glow, fur, glass) less so. Anything "
     "in the background's own colour is cleared, also inside the subject (a white highlight on a white background): the "
     "answer says when that happened. When the background is not flat the picture is saved as it is and the answer says "
     "so. The answer names the saved file "
     "and carries the picture: look at it before you use or report it. Captured footage of the real game is still what "
     "shows gameplay; never another game's characters or art, and no lettering (add text natively with real fonts). "
     "rbx_image uploads nothing; uploads follow the guide's upload rules.",
     schema({"action": {"type": "string", "enum": ["generate", "keep"]},
             "transparent": {"type": "boolean", "description": "Cut the subject out onto a transparent background (PNG). Default false"},
             "prompt": {"type": "string", "description": "generate: what the image shows, in plain words: subject, setting, style, light, mood"},
             "source": {"type": "string", "description": "keep: full path of the image file to bring in. Read only from the project, this chat's own "
                                                         "folder, the app's images, icons, textures, shots, attachments and chat pictures, or the "
                                                         "file your own image tool made"},
             "size": {"type": "string", "description": "WIDTHxHEIGHT in pixels, e.g. 512x512 (icon) or 1920x1080 (thumbnail). "
                                                       "generate: default 1024x1024; keep: default the image's own size"},
             "name": {"type": "string", "description": "File name; saved as .png unless it ends in .jpg"},
             "folder": {"type": "string", "description": "Folder inside the game's project folder, e.g. store/thumbnails. Default: art"}},
            ["action"]), t_image),
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
    ("rbx_board", "The game's board (the user sees its needs cards in the game's Review tab), with columns todo (planned), doing (being worked on now), "
     "needs (something only the user can do or decide) and done. Only for chats that belong to a game project. Keep it true: "
     "'add' a card in doing when you start a real piece of work, 'move' it to done when it is finished and checked, add one in "
     "needs for each thing you need from the user (one clear action per card), and add planned work in todo. Action 'list' "
     "shows every card with its id. Titles are short and plain, like a task on a sticky note.",
     schema({"action": {"type": "string", "enum": ["list", "add", "move", "update"]},
             "title": {"type": "string", "description": "add/update: a few words, e.g. 'Shop menu' or 'Upload the coin sound'"},
             "note": {"type": "string", "description": "add/update: optional detail, one or two sentences"},
             "column": {"type": "string", "enum": ["todo", "doing", "needs", "done"], "description": "add (default doing) / move"},
             "id": {"type": "string", "description": "move/update: the card's id from list or add"}}, ["action"]), t_board),
    ("rbx_stage", "Studio mode: move the game through its stages (idea, design, build, test, launch, promote), file review items "
     "with pictures or clips for the user to approve in the app, read their decisions, and keep lessons. Load the `studio` guide "
     "before using it.",
     schema({"action": {"type": "string", "enum": ["status", "stage", "review", "decisions", "lesson"]},
             "id": {"type": "string", "enum": ["idea", "design", "build", "test", "launch", "promote"], "description": "stage: which stage"},
             "status": {"type": "string", "enum": ["start", "done"], "description": "stage: start or finish it"},
             "title": {"type": "string", "description": "review: short title"},
             "summary": {"type": "string", "description": "review: what was done and what the user should check"},
             "plain": {"type": "string", "description": "review: the expert's \"In plain words\" line (from its result), kept on the item for the user"},
             "media": {"type": "array", "items": {"type": "string"}, "description": "review: picture or video file paths (png, jpg, webp, mp4, mov; 8 at most)"},
             "expert": {"type": "string", "description": "review: the expert that made the work"},
             "blocking": {"type": "boolean", "description": "review: true if the run must wait for the user's yes"},
             "replaces": {"type": "string", "description": "review: id of the older item this one replaces"},
             "force": {"type": "boolean", "description": "review: replace even an item the user already approved (only when the plan itself must change)"},
             "text": {"type": "string", "description": "lesson: one short sentence"},
             "scope": {"type": "string", "enum": ["game", "all"], "description": "lesson: this game or every game (default game)"}},
            ["action"]), t_stage),
    ("rbx_team", "Start extra agents through the Fridae Studios app and collect their reports. Only works when this chat runs inside the "
     "app. Action 'spawn' (role, name, task): role 'worker' does an independent piece of the job, 'critic' reviews work and reports "
     "problems; the task must be a full brief (at least 80 characters: what to do, where, what is done already, what to leave alone, "
     "what to report), because the new agent knows nothing about this chat. Returns an id and status, or says the app did not pick "
     "it up. Optional 'ai' and 'model' run it on another AI (see the studio guide; the user's Setup > Experts table wins); the answer "
     "says which AI and model it runs on, with a note whenever that is not what you asked for. 'models' (optional ai) lists what each "
     "AI offers: its default model and its model ids. The swarm: the experts you start work at once, each writer in its own copy "
     "of the project, on a shared board. 'post' (to: [\"@name\"], [\"@role\"], [\"@all\"] or [\"@lead\"]; text; kind msg, done "
     "or verify) posts on it; 'board' (optional since, limit) reads the recent posts; 'wait_board' (timeout) waits for a post "
     "that mentions you; 'studio_turn' (op take, give or status) takes turns with the one Studio place. 'status' (optional ids) lists tasks. 'wait' (ids, timeout seconds, default 600, at most 840) blocks until they are done "
     "or refused and returns each agent's final reply (a studio run's Lead is handed back after 120 seconds at most, or sooner when a "
     "message or decision is waiting or the run was paused or ended; it says why). Agents started this way cannot start more. They share the one Studio: use "
     "rbx_studio_lock around playtests.",
     schema({"action": {"type": "string", "enum": ["spawn", "status", "wait", "models", "post", "board", "wait_board", "studio_turn"]},
             "member": {"type": "string", "description": "spawn (optional): the member's handle on the board, such as \"shop-ui\" (default: from name)"},
             "to": {"type": "array", "items": S, "description": "post: who it is for: \"@name\", \"@role\", \"@all\" or \"@lead\""},
             "text": {"type": "string", "description": "post: the message (for kind done, a one-line summary of your part)"},
             "kind": {"type": "string", "enum": ["msg", "done", "verify"], "description": "post: msg (default); done when your part is finished; verify for a verifier's PASS or FAIL"},
             "since": {"type": "number", "description": "board, wait_board: only posts after this time (a [t] from the board)"},
             "limit": {"type": "integer", "description": "board: posts to show, at most 40"},
             "op": {"type": "string", "enum": ["take", "give", "status"], "description": "studio_turn"},
             "role": {"type": "string", "enum": ["worker", "critic"], "description": "spawn"},
             "name": {"type": "string", "description": "spawn: short title shown in the app, 34 characters at most"},
             "task": {"type": "string", "description": "spawn: the full instructions for the new agent"},
             "ids": {"type": "array", "items": S, "description": "status (optional) and wait: task ids returned by spawn"},
             "timeout": {"type": "integer", "description": "wait: seconds, default 600, at most 840; wait_board: default 300"},
             "expert": {"type": "string", "description": "spawn (Studio mode): a roster expert id such as coder or security; its role comes from the roster and the task is added to the expert's own instructions"},
             "ai": {"type": "string", "enum": ["claude", "gemini", "cursor", "opencode"], "description": "spawn (optional, use sparingly): run the new agent on this AI instead of your own. The user's Setup > Experts choice for that expert wins over it; an AI that is not installed is refused with the reason. models (optional): list this AI's models in full"},
             "model": {"type": "string", "description": "spawn (optional): a model id of that AI (or of yours when ai is left out), exactly as action 'models' lists it; one it does not offer runs on the nearest it has or on its default, and the result says which and why"}}, ["action"]), t_team),
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
def handle(msg, began=None, placed=None):
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
        return {"tools": [{"name": n, "description": d, "inputSchema": s} for n, d, s, _ in TOOLS + _extra_tools()]}
    if method == "tools/call":
        fn = _handler(params.get("name"))
        if not fn:
            raise KeyError("unknown tool: %s" % params.get("name"))
        budget = _budget()
        if budget is not None:      # an AI that cuts calls off: answered in time (see "long calls")
            return _budgeted(params.get("name"), fn, params.get("arguments") or {}, (began or time.monotonic()) + budget, mid, placed)
        return _run_tool(fn, params.get("arguments") or {})
    if mid is None:
        return None  # a notification we do not act on
    raise NotImplementedError(method)


def _reply(msg, began=None, placed=None):
    """The line that answers one message, or None for a notification."""
    mid = msg.get("id")
    try:
        res = handle(msg, began, placed)
        if mid is None or res is _NO_ANSWER:    # a notification, or a call the client gave up on
            return None
        return {"jsonrpc": "2.0", "id": mid, "result": res}
    except NotImplementedError as e:
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "method not found: %s" % e}}
    except Exception as e:
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": str(e)}}


def _end_with_jobs(signum, frame):
    """Told to end while jobs may run (only where calls have a budget): they go first, then the server as it would have."""
    import signal
    _drop_all()
    signal.signal(signum, signal.SIG_DFL)
    os.kill(os.getpid(), signum)


def main():
    if os.environ.get(REAPER_ENV):
        return _reaper_main()
    if os.environ.get(JOB_ENV):
        return _job_main()
    stdin = sys.stdin.buffer
    # The protocol goes out on a private copy of fd 1. Fd 1 and sys.stdout then point at stderr before any thread
    # starts, so a stray print, a library or a child process writing to "stdout" can never corrupt the protocol,
    # and tools that capture stdout with redirect_stdout still work.
    out = os.fdopen(os.dup(1), "wb")
    sys.stdout.flush()
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    writing = threading.Lock()

    def answer(msg, began=None, after=None, placed=None):
        if after is not None:
            after.wait(5)       # the call read before this one has taken its place in line first
        try:
            reply = _reply(msg, began, placed)
        finally:
            if placed is not None:
                placed.set()
        if reply is not None:
            with writing:
                out.write((json.dumps(reply) + "\n").encode("utf-8"))
                out.flush()

    budgeted = _budget() is not None
    before = None
    if budgeted and os.name == "posix":
        import signal
        for s in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            signal.signal(s, _end_with_jobs)
    for raw in stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw.decode("utf-8"))
        except ValueError:
            continue
        if budgeted and msg.get("method") == "notifications/cancelled":
            _GIVEN_UP.add((msg.get("params") or {}).get("requestId"))   # its waiting call answers nothing and takes no result
            continue
        if budgeted and msg.get("method") == "tools/call" and msg.get("id") is not None:
            # each call on a thread of its own, timed from here: a wait never holds another call up
            placed = threading.Event()
            threading.Thread(target=answer, args=(msg, time.monotonic(), before, placed), daemon=True).start()
            before = placed
            continue
        answer(msg)
    if budgeted:
        _drop_all()     # the client is gone: nothing of this server's runs on


if __name__ == "__main__":
    main()
