"""The game's project folder: scripts on disk, checkpoints in git.

The project folder is named by the RBXKIT_PROJECT environment variable. The
scripts of the open Studio place are copied into <project>/src using Rojo-style
names, so they can be read, searched, diffed and committed like any code:

  src/ServerScriptService/Systems/Shop.server.luau     a Script
  src/StarterPlayer/StarterPlayerScripts/Hud.client.luau   a LocalScript
  src/ReplicatedStorage/Prices.luau                    a ModuleScript
  src/ServerScriptService/Round/init.server.luau       a Script that has children

  rbx.py project info | export_scripts | changes
  rbx.py project checkpoint -m "shop works"
  rbx.py project restore_scripts [--commit abc123] [--paths ...] [--confirm]
  rbx.py project save_scripts --from scripts.json --out folder   (scripts the Fridae plugin handed over)

A checkpoint covers SCRIPTS ONLY. Parts, UI, terrain and every other instance
are not in it: Studio's own undo and the place's version history cover those.
"""
import difflib, hashlib, json, os, re, subprocess

from rbxpaths import KitError, no_window, read_json, write_json
import rbxstudio

EXT = {"Script": ".server.luau", "LocalScript": ".client.luau", "ModuleScript": ".luau"}
RESERVED = {"con", "prn", "aux", "nul"} | {"com%d" % i for i in range(1, 10)} | {"lpt%d" % i for i in range(1, 10)}
MANIFEST = ".manifest.json"


class ProjectError(KitError):
    pass


def project(required=True):
    path = os.environ.get("RBXKIT_PROJECT") or ""
    if path and os.path.isdir(path):
        return os.path.abspath(path)
    if required:
        raise ProjectError("No project folder is set for this chat (RBXKIT_PROJECT), so there is nowhere to keep the scripts. "
                           "Choose a project folder first." if not path else "The project folder %s does not exist." % path)
    return None


# --------------------------------------------------------------------- git
def git(args, cwd, text=True):
    """Run git. Returns (exit code, output)."""
    try:
        # the game's own hooks and fsmonitor program never run: agents can write the game's .git folder
        p = subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"] + list(args), cwd=cwd, capture_output=True, creationflags=no_window(), timeout=120)
    except FileNotFoundError:
        raise ProjectError("git is not installed (or not on PATH), so there are no checkpoints. Install it from git-scm.com.")
    except subprocess.TimeoutExpired:
        raise ProjectError("git did not finish: git " + " ".join(args[:2]))
    out = p.stdout if p.returncode == 0 or not p.stderr else p.stdout + p.stderr
    return p.returncode, (out.decode("utf-8", "replace").strip() if text else p.stdout)


def is_repo(folder):
    """True when the folder is inside a git repository. Raises ProjectError when git is missing."""
    return git(["rev-parse", "--is-inside-work-tree"], folder) == (0, "true")


# -------------------------------------------------------- reading the place
SERVICES = ["Workspace", "ReplicatedFirst", "ReplicatedStorage", "ServerScriptService", "ServerStorage", "StarterGui",
            "StarterPack", "StarterPlayer", "Lighting", "SoundService", "Teams", "Chat", "TextChatService",
            "LocalizationService", "MaterialService", "TestService"]

# Every script in the place: path (name and index among same-named siblings at each level), class, source.
COLLECT = """local SERVICES = { %s }
local SES
pcall(function() SES = game:GetService("ScriptEditorService") end)
local index = {}
local function sibling(inst)
	if not index[inst] then
		local n, name = 0, inst.Name
		for _, c in inst.Parent:GetChildren() do
			if c.Name == name then n += 1 index[c] = n end
		end
	end
	return index[inst]
end
local out = {}
for _, sname in SERVICES do
	local ok, svc = pcall(function() return game:FindService(sname) end)
	if ok and svc then
		for _, d in svc:GetDescendants() do
			if d:IsA("LuaSourceContainer") then
				local path, cur = {}, d
				while cur ~= svc do
					table.insert(path, 1, { cur.Name, sibling(cur) })
					cur = cur.Parent
				end
				table.insert(path, 1, { sname, 1 })
				local src
				if SES then
					local okS, s = pcall(function() return SES:GetEditorSource(d) end)
					if okS and type(s) == "string" then src = s end
				end
				if not src then
					local okS, s = pcall(function() return d.Source end)
					if okS and type(s) == "string" then src = s end
				end
				local item = { p = path, c = d.ClassName, k = #d:GetChildren() > 0 }
				if src then item.s = src else item.unreadable = true end
				if d.ClassName == "Script" then pcall(function() item.r = d.RunContext.Name end) end
				pcall(function() if d.Enabled == false then item.off = true end end)
				table.insert(out, item)
				if #out %% 300 == 0 then task.wait() end
			end
		end
	end
end
return game:GetService("HttpService"):JSONEncode({ place = { name = game.Name, id = game.PlaceId, universe = game.GameId }, scripts = out })
""" % ", ".join('"%s"' % s for s in SERVICES)

FIND = """local function find(path)
	local cur = game:FindService(path[1][1])
	for i = 2, #path do
		if not cur then return nil end
		local k, hit = 0, nil
		for _, c in cur:GetChildren() do
			if c.Name == path[i][1] then
				k += 1
				if k == path[i][2] then hit = c break end
			end
		end
		cur = hit
	end
	return cur
end
"""

APPLY = FIND + """local SES
pcall(function() SES = game:GetService("ScriptEditorService") end)
local items = { %s }
local res = {}
for _, it in items do
	local inst = find(it.p)
	if not inst or not inst:IsA("LuaSourceContainer") then
		table.insert(res, "missing")
	else
		local ok, err = false, "no ScriptEditorService"
		if SES then ok, err = pcall(function() SES:UpdateSourceAsync(inst, function() return it.s end) end) end
		if not ok then ok, err = pcall(function() inst.Source = it.s end) end
		table.insert(res, ok and "ok" or ("failed: " .. tostring(err)))
	end
end
return game:GetService("HttpService"):JSONEncode(res)
"""


def sha(text):
    return hashlib.sha1(text.encode("utf-8", "surrogatepass")).hexdigest()


def key(path):
    """A stable identity for an instance: its names and sibling indexes from the service down."""
    return json.dumps(path, ensure_ascii=False, separators=(",", ":"))


def dotted(path):
    return ".".join(n if n2 == 1 else "%s[%d]" % (n, n2) for n, n2 in path)


def safe_name(name):
    """A file or folder name Windows accepts."""
    s = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", name).rstrip(" .")
    if not s or s.split(".")[0].lower() in RESERVED:
        s = "_" + s
    return s[:120]


def read_place(studio):
    """(place info, scripts) from the open place. Each script: path, class, source, run_context, has_children."""
    return parse_place(json.loads(studio.fetch(COLLECT, timeout=300)))


def parse_place(data):
    """(place info, scripts) from what COLLECT returns (the Fridae plugin's "sources" answer has the same shape)."""
    scripts = [{"path": [[str(n), int(i)] for n, i in it["p"]], "class": it["c"], "source": it.get("s"),
                "run_context": it.get("r"), "has_children": bool(it.get("k")), "disabled": bool(it.get("off"))}
               for it in data.get("scripts") or []]
    return data.get("place") or {}, scripts


def layout(scripts):
    """Give each readable script its file under src. Returns {relative path: script}."""
    files, used = {}, set()
    for sc in scripts:
        if sc["source"] is None or sc["class"] not in EXT:
            continue
        parts = [safe_name(n) + ("~%d" % i if i > 1 else "") for n, i in sc["path"]]
        ext = EXT[sc["class"]]
        folder, stem = (parts, "init") if sc["has_children"] else (parts[:-1], parts[-1])
        rel = "/".join(folder + [stem + ext])
        n = 1
        while rel.lower() in used:  # names that differ only by case, or a child really called "init"
            n += 1
            rel = "/".join(folder + ["%s~%d%s" % (stem, n, ext)])
        used.add(rel.lower())
        files[rel] = sc
    return files


def manifest_of(files, place):
    out = {}
    for rel, sc in sorted(files.items()):
        src = sc["source"]
        e = {"instance": dotted(sc["path"]), "path": sc["path"], "class": sc["class"], "sha1": sha(src),
             "lines": src.count("\n") + (1 if src and not src.endswith("\n") else 0)}
        if sc.get("run_context"):
            e["run_context"] = sc["run_context"]
        if sc.get("disabled"):
            e["disabled"] = True
        out[rel] = e
    return {"about": "Written by rbxkit (rbx_project export_scripts). Maps each file to its script in the Roblox place.",
            "place": place, "files": out}


def load_manifest(src):
    m = read_json(os.path.join(src, MANIFEST))
    return m if isinstance(m, dict) and isinstance(m.get("files"), dict) else None


def _inside(src, rel):
    """The real path of a manifest entry, or None when it would fall outside src."""
    path = os.path.abspath(os.path.join(src, *rel.split("/")))
    return path if path.startswith(os.path.abspath(src) + os.sep) else None


def _bytes(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def _read(path):
    data = _bytes(path)
    return None if data is None else data.decode("utf-8", "replace")


def _waypoint(studio, name):
    """Leave a named marker in Studio's undo history."""
    studio.fetch('game:GetService("ChangeHistoryService"):SetWaypoint(%s) return "ok"' % rbxstudio.lua_string(name))


# ------------------------------------------------------------------ actions
def export_scripts(studio, root=None):
    root = root or project()
    place, scripts = read_place(studio)
    return write_scripts(os.path.join(root, "src"), place, scripts)


def save_scripts(data_file, out):
    """Write scripts the Fridae plugin handed over (a JSON file in COLLECT's shape) into the folder `out`, in the same
    layout as export_scripts. The app uses it to keep the scripts as they were before an undo puts them back."""
    data = read_json(data_file)
    if not isinstance(data, dict) or not isinstance(data.get("scripts"), list):
        raise ProjectError("The scripts to save could not be read from %s." % data_file)
    if os.path.isdir(out) and os.listdir(out):
        raise ProjectError("The folder %s is not empty, so the scripts were not saved there." % out)
    place, scripts = parse_place(data)
    return write_scripts(out, place, scripts)


def write_scripts(src, place, scripts):
    """Write the scripts under src (Rojo-style names and .manifest.json); files of scripts no longer there go."""
    files = layout(scripts)
    old = load_manifest(src) or {"files": {}}
    os.makedirs(src, exist_ok=True)
    written, unchanged, removed = [], 0, []
    for rel, sc in files.items():
        path = _inside(src, rel)
        data = sc["source"].encode("utf-8", "replace")
        if _bytes(path) == data:
            unchanged += 1
            continue
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
        written.append(rel)
    for rel in old["files"]:
        path = _inside(src, rel)
        if rel not in files and path and os.path.isfile(path):
            os.remove(path)
            removed.append(rel)
            folder = os.path.dirname(path)
            while folder != src and not os.listdir(folder):  # tidy folders left empty
                os.rmdir(folder)
                folder = os.path.dirname(folder)
    attrs = os.path.join(src, ".gitattributes")
    if not os.path.exists(attrs):
        with open(attrs, "w", encoding="utf-8") as f:
            f.write("# keep script sources byte for byte, so restoring gives back exactly what was exported\n* -text\n")
    write_json(os.path.join(src, MANIFEST), manifest_of(files, place), indent=1, ensure_ascii=False)
    unreadable = [dotted(s["path"]) for s in scripts if s["source"] is None]
    return {"folder": src, "place": place, "scripts": len(files), "written": len(written), "unchanged": unchanged,
            "removed": len(removed), "written_files": written, "removed_files": removed, "unreadable": unreadable}


def _line_delta(old, new):
    plus = minus = 0
    for line in difflib.ndiff(old.splitlines(), new.splitlines()):
        plus += line.startswith("+ ")
        minus += line.startswith("- ")
    return plus, minus


def _compare(src, files):
    """Scripts in the place (files: layout output) against what is on disk under src."""
    man = load_manifest(src) or {"files": {}}
    disk = {key(e["path"]): (rel, e) for rel, e in man["files"].items()}
    live = {key(sc["path"]): sc for sc in files.values()}
    added, removed, modified = [], [], []
    for k, sc in live.items():
        if k not in disk:
            added.append({"script": dotted(sc["path"]), "lines": len(sc["source"].splitlines())})
            continue
        rel = disk[k][0]
        old = _read(_inside(src, rel) or "")
        if old is None:
            added.append({"script": dotted(sc["path"]), "lines": len(sc["source"].splitlines()), "note": "its file is missing"})
        elif old != sc["source"]:
            plus, minus = _line_delta(old, sc["source"])
            modified.append({"script": dotted(sc["path"]), "file": "src/" + rel, "added_lines": plus, "removed_lines": minus})
    for k, (rel, e) in disk.items():
        if k not in live:
            removed.append({"script": e.get("instance"), "file": "src/" + rel, "lines": e.get("lines")})
    return added, removed, modified


def changes(studio, root=None):
    root = root or project()
    src = os.path.join(root, "src")
    if not load_manifest(src):
        raise ProjectError("Nothing has been exported yet, so there is nothing to compare with. Run export_scripts or checkpoint first.")
    place, scripts = read_place(studio)
    added, removed, modified = _compare(src, layout(scripts))
    return {"place": place, "added": added, "removed": removed, "modified": modified}


def checkpoint(studio, message, root=None):
    root = root or project()
    message = " ".join((message or "").split())
    if not message:
        raise ProjectError("Give 'message': a few words saying what this checkpoint is.")
    exp = export_scripts(studio, root)
    out = {"export": {k: exp[k] for k in ("scripts", "written", "unchanged", "removed", "unreadable")}}
    try:
        _waypoint(studio, "checkpoint: " + message)
        out["studio_waypoint"] = "checkpoint: " + message
    except rbxstudio.StudioError as e:
        out["studio_waypoint_error"] = str(e)[:200]
    try:
        repo = is_repo(root)
    except ProjectError as e:
        out["note"] = "Scripts were exported only. " + str(e)
        return out
    if not repo:
        out["note"] = ("Scripts were exported to src, but the project folder is not a git repository, so no checkpoint was "
                       "recorded. Run `git init` in %s to start keeping checkpoints." % root)
        return out
    code, err = git(["add", "-A", "--", "."], root)
    if code != 0:
        raise ProjectError("git add failed: " + err[:400])
    code, staged = git(["diff", "--cached", "--name-status", "--", "."], root)
    changed = [l.replace("\t", " ") for l in staged.splitlines() if l.strip()]
    if not changed:
        out["note"] = "Nothing changed since the last checkpoint, so no commit was made."
        code, head = git(["rev-parse", "--short", "HEAD"], root)
        out["commit"] = head if code == 0 else None
        return out
    code, err = git(["commit", "-m", message, "--", "."], root)
    if code != 0:
        raise ProjectError("git commit failed: " + err[:600])
    out["commit"] = git(["rev-parse", "--short", "HEAD"], root)[1]
    out["changed_files"] = changed
    return out


def _saved(root, commit):
    """({relative file: manifest entry}, read(rel) -> source) for the working tree or a commit."""
    src = os.path.join(root, "src")
    if not commit:
        man = load_manifest(src)
        if not man:
            raise ProjectError("There is no export in src to restore from.")
        return man["files"], lambda rel: _read(_inside(src, rel) or "")
    if not is_repo(root):
        raise ProjectError("The project folder is not a git repository, so there are no commits to restore from.")
    code, raw = git(["show", "%s:./src/%s" % (commit, MANIFEST)], root, text=False)
    if code != 0:
        raise ProjectError("Commit %s has no exported scripts (no src/%s in it), or does not exist." % (commit, MANIFEST))
    try:
        files = json.loads(raw.decode("utf-8"))["files"]
    except (ValueError, KeyError):
        raise ProjectError("The script list saved in commit %s could not be read." % commit)

    def read(rel):
        code, raw = git(["show", "%s:./src/%s" % (commit, rel)], root, text=False)
        return raw.decode("utf-8", "replace") if code == 0 else None
    return files, read


def restore_scripts(studio, commit=None, paths=None, confirm=False, root=None):
    root = root or project()
    files, read = _saved(root, commit)
    wanted = [p.replace("\\", "/").strip() for p in (paths or []) if p and p.strip()]

    def chosen(rel, e):
        if not wanted:
            return True
        names = ("src/" + rel, rel, e.get("instance") or "")
        return any(n == w or n.startswith(w.rstrip("/.") + "/") or n.startswith(w.rstrip("/.") + ".") for w in wanted for n in names)

    place, scripts = read_place(studio)
    live = {key(s["path"]): s for s in scripts}
    todo, missing, same, unreadable = [], [], 0, []
    for rel, e in sorted(files.items()):
        if not chosen(rel, e):
            continue
        saved = read(rel)
        if saved is None:
            unreadable.append("src/" + rel)
            continue
        now = live.get(key(e["path"]))
        if now is None or now["source"] is None:
            missing.append(e.get("instance"))
        elif now["source"] == saved:
            same += 1
        else:
            plus, minus = _line_delta(now["source"], saved)
            todo.append({"script": e.get("instance"), "file": "src/" + rel, "path": e["path"], "source": saved,
                         "added_lines": plus, "removed_lines": minus})
    out = {"place": place, "from": commit or "the files in src", "already_the_same": same,
           "not_in_place": missing, "unreadable_files": unreadable,
           "scripts": [{k: t[k] for k in ("script", "file", "added_lines", "removed_lines")} for t in todo]}
    if wanted and not (todo or same or missing or unreadable):
        raise ProjectError("No exported script matches: " + ", ".join(wanted))
    if not confirm:
        out["applied"] = False
        return out
    for t, r in zip(out["scripts"], _apply(studio, todo)):
        t["result"] = r
    if todo:
        try:
            _waypoint(studio, "restore scripts" + (" from " + commit if commit else ""))
        except rbxstudio.StudioError:
            pass
        after = {key(s["path"]): s for s in read_place(studio)[1]}
        out["verified"] = sum(1 for t in todo if (after.get(key(t["path"])) or {}).get("source") == t["source"])
    out["applied"] = True
    return out


def _apply(studio, todo):
    """Write these sources into the place, as many scripts per call as fit. Returns each script's result, in order."""
    batches, size = [], 0
    for t in todo:
        n = len(t["source"].encode("utf-8", "replace")) + 200
        if not batches or size + n > rbxstudio.CODE_LIMIT:
            batches.append([])
            size = 0
        batches[-1].append(t)
        size += n
    results = []
    for batch in batches:
        body = ", ".join("{ p = %s, s = %s }" % (_lua_path(t["path"]), rbxstudio.lua_string(t["source"])) for t in batch)
        results.extend(json.loads(studio.fetch(APPLY.replace("%s", body, 1), timeout=300)))
    return results


def _lua_path(path):
    return "{ " + ", ".join("{ %s, %d }" % (rbxstudio.lua_string(n), i) for n, i in path) + " }"


def info():
    root = project(required=False)
    out = {"project": root}
    if not root:
        out["note"] = ("No project folder is set for this chat (RBXKIT_PROJECT), so there is nowhere to export scripts or "
                       "keep checkpoints.")
    else:
        text = _read(os.path.join(root, "GAME.md"))
        out["game_md"] = text is not None
        if text is not None:
            out["game_md_head"] = "\n".join(text.splitlines()[:40])
        man = load_manifest(os.path.join(root, "src"))
        out["exported_scripts"] = len(man["files"]) if man else 0
        try:
            out["git"] = is_repo(root)
            if out["git"]:
                out["branch"] = git(["symbolic-ref", "--short", "-q", "HEAD"], root)[1] or "(detached)"
                out["uncommitted_changes"] = len([l for l in git(["status", "--porcelain", "--", "."], root)[1].splitlines() if l.strip()])
                code, log = git(["log", "-5", "--pretty=format:%h %ad %s", "--date=format:%Y-%m-%d %H:%M"], root)
                out["last_commits"] = log.splitlines() if code == 0 else []
        except ProjectError as e:
            out["git"] = False
            out["git_note"] = str(e)
    try:
        with rbxstudio.Studio() as s:
            s.pick()
            out["studio"] = {"window": s.info.get("name"), "mode": s.info.get("mode")}
            out["studio"].update(json.loads(s.fetch("return { name = game.Name, place_id = game.PlaceId, universe_id = game.GameId }")))
    except (rbxstudio.StudioError, ValueError) as e:
        out["studio"] = None
        out["studio_note"] = str(e)[:300]
    return out


# ------------------------------------------------------------------- output
SCOPE = ("Checkpoints cover scripts only. Parts, UI, terrain and other instances are not in them: Studio's undo and the "
         "place's version history cover those.")


def _list(title, rows, fmt=str):
    if not rows:
        return []
    out = ["%s (%d):" % (title, len(rows))] + ["  " + fmt(r) for r in rows[:40]]
    if len(rows) > 40:
        out.append("  ... and %d more" % (len(rows) - 40))
    return out


def _info_text(r):
    lines = ["Project: %s" % (r["project"] or "(none set)")]
    if r.get("note"):
        lines.append(r["note"])
    if r["project"]:
        lines.append("GAME.md: %s" % ("present" if r["game_md"] else "missing"))
        lines.append("Scripts exported in src: %d" % r["exported_scripts"])
        if r.get("git"):
            lines.append("Git: branch %s, %d uncommitted change(s)" % (r["branch"], r["uncommitted_changes"]))
            lines += ["  " + c for c in r["last_commits"]] or ["  (no commits yet)"]
        else:
            lines.append("Git: " + (r.get("git_note") or "not a repository (run `git init` in the project folder to keep checkpoints)"))
    st = r.get("studio")
    lines.append("Studio: %s (place id %s, universe id %s)%s" % (st["name"], st["place_id"], st["universe_id"],
                 ", mode %s" % st["mode"] if st.get("mode") else "") if st else "Studio: not reachable. " + r.get("studio_note", ""))
    if r.get("game_md_head"):
        lines += ["", "GAME.md (first 40 lines):", r["game_md_head"]]
    return "\n".join(lines)


def _export_text(r):
    lines = ["Exported %d script(s) from \"%s\" to %s" % (r["scripts"], r["place"].get("name"), r["folder"]),
             "written %d, unchanged %d, removed %d" % (r["written"], r["unchanged"], r["removed"])]
    lines += _list("Written", r["written_files"]) + _list("Removed", r["removed_files"])
    lines += _list("Could not be read (skipped)", r["unreadable"])
    return "\n".join(lines)


def _checkpoint_text(r):
    e = r["export"]
    lines = ["Exported %d script(s): written %d, unchanged %d, removed %d" % (e["scripts"], e["written"], e["unchanged"], e["removed"])]
    if r.get("changed_files"):
        lines.append("Checkpoint %s recorded." % r["commit"])
        lines += _list("Changed files", r["changed_files"])
    if r.get("note"):
        lines.append(r["note"] + (" Last checkpoint: %s." % r["commit"] if r.get("commit") else ""))
    lines.append("Studio undo marker: " + (r.get("studio_waypoint") or "not set (%s)" % r.get("studio_waypoint_error")))
    lines += _list("Could not be read (skipped)", e["unreadable"])
    lines.append(SCOPE)
    return "\n".join(lines)


def _changes_text(r):
    if not (r["added"] or r["removed"] or r["modified"]):
        return "No script has changed since the last export.\n" + SCOPE
    lines = ["Script changes in \"%s\" since the last export:" % r["place"].get("name")]
    lines += _list("Modified", r["modified"], lambda x: "%s  +%d -%d lines" % (x["script"], x["added_lines"], x["removed_lines"]))
    lines += _list("Added", r["added"], lambda x: "%s  %d lines%s" % (x["script"], x["lines"], "  (%s)" % x["note"] if x.get("note") else ""))
    lines += _list("Removed", r["removed"], lambda x: "%s  %s lines" % (x["script"], x.get("lines")))
    lines.append(SCOPE)
    return "\n".join(lines)


def _restore_text(r):
    did = r["applied"]
    lines = ["%s from %s into \"%s\":" % ("Restored" if did else "DRY RUN. This would restore", r["from"], r["place"].get("name"))]
    lines += _list("Scripts written" if did else "Scripts that would be overwritten", r["scripts"],
                   lambda x: "%s  +%d -%d lines%s" % (x["script"], x["added_lines"], x["removed_lines"],
                                                     "  [%s]" % x["result"] if x.get("result") else ""))
    if not r["scripts"]:
        lines.append("  nothing to do: no chosen script differs.")
    lines.append("Already the same: %d" % r["already_the_same"])
    lines += _list("In the export but no longer in the place (not recreated)", r["not_in_place"])
    lines += _list("Files that could not be read", r["unreadable_files"])
    if did and r["scripts"]:
        lines.append("Checked afterwards: %d of %d now match." % (r.get("verified", 0), len(r["scripts"])))
    elif r["scripts"]:
        lines.append("Nothing was changed. Call again with confirm true to write these scripts into the place "
                     "(take a checkpoint first if the current scripts are worth keeping).")
    return "\n".join(lines)


def run(action, a=None):
    """Do one action and return the text to show. Raises ProjectError or StudioError with a plain message."""
    a = a or {}
    action = (action or "info").lower()
    if action == "info":
        return _info_text(info())
    if action == "save_scripts":
        if not a.get("source") or not a.get("out"):
            raise ProjectError("save_scripts needs --from (the plugin's scripts, as JSON) and --out (the folder).")
        return _export_text(save_scripts(a["source"], a["out"]))
    root = project()
    with rbxstudio.Studio() as s:
        if action == "export_scripts":
            return _export_text(export_scripts(s, root))
        if action == "checkpoint":
            return _checkpoint_text(checkpoint(s, a.get("message"), root))
        if action == "changes":
            return _changes_text(changes(s, root))
        if action == "restore_scripts":
            return _restore_text(restore_scripts(s, a.get("commit") or None, a.get("paths") or None, bool(a.get("confirm")), root))
    raise ProjectError("Unknown action '%s'. Use info, export_scripts, checkpoint, changes or restore_scripts." % action)


def _cmd(a):
    print(run(a.action, {"message": a.message, "commit": a.commit, "paths": a.paths, "confirm": a.confirm,
                         "source": a.source, "out": a.out}))


def register(sub):
    g = sub.add_parser("project", help="export the place's scripts to the project folder, checkpoint them in git, restore them")
    g.add_argument("action", choices=["info", "export_scripts", "checkpoint", "changes", "restore_scripts", "save_scripts"])
    g.add_argument("-m", "--message", help="checkpoint: what this checkpoint is")
    g.add_argument("--commit", help="restore_scripts: the commit to restore from (default: the files in src)")
    g.add_argument("--paths", nargs="*", help="restore_scripts: only these files, folders or instance paths")
    g.add_argument("--confirm", action="store_true", help="restore_scripts: really write the scripts into the place")
    g.add_argument("--from", dest="source", help="save_scripts: a JSON file of scripts from the Fridae plugin")
    g.add_argument("--out", help="save_scripts: the folder to write them into")
    g.set_defaults(fn=_cmd)
