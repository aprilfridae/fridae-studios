//! Projects: one folder each, usually a git repository, with its notes in PROJECT.md. Any kind of software project
//! works the same way. A Roblox project (kind "game", the name it had when the app was for Roblox games only)
//! also gets the Roblox tools: Studio, the place's exported scripts, the Roblox guides and Promos. The user marks it
//! in the project's settings, or the app sees it in the folder (`looks_roblox`); every other project is "plain".

use crate::paths::Start;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{Value, json};

use crate::paths::{self, PATHS};

/// Settings every git the app runs starts with: no hooks, no fsmonitor program.
pub const GIT_SAFE: [&str; 6] = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "commit.gpgsign=false"];

pub fn git_installed() -> bool {
    paths::which("git").is_some()
}

/// Run git in a folder. Returns whether it succeeded and what it printed. The game's own hooks and fsmonitor program
/// are never run: an agent can write the game's `.git` folder, and these checkpoints run outside any agent's rules.
fn run_git(dir: &Path, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.args(GIT_SAFE).args(args).current_dir(dir).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    paths::quiet(&mut cmd);
    match cmd.run_output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if !err.is_empty() {
                text = if text.is_empty() { err } else { format!("{text}\n{err}") };
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("Could not run git: {e}")),
    }
}

/// A project's own notes file, the project bible (what it is, its decisions and their log).
pub const BIBLE: &str = "PROJECT.md";
/// Its name before the rename: read when a project has only this one, and renamed to `BIBLE` when the project opens.
pub const OLD_BIBLE: &str = "GAME.md";

/// The project's PROJECT.md, else its GAME.md from before the rename (PROJECT.md first when both are there).
pub fn bible_path(dir: &Path) -> Option<PathBuf> {
    [BIBLE, OLD_BIBLE].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Rename a project's GAME.md to PROJECT.md, content and all, when it has only the old one. When both are there
/// both stay (PROJECT.md is the one read). Returns whether it renamed.
pub fn adopt_bible(dir: &Path) -> bool {
    let (old, new) = (dir.join(OLD_BIBLE), dir.join(BIBLE));
    // the exact name: on a disk that ignores case, game.md is the same file and stays as it is
    let exact = fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.file_name() == OLD_BIBLE);
    exact && old.is_file() && !new.exists() && fs::rename(&old, &new).is_ok()
}

/// Where new game folders go unless the user picks somewhere else.
pub fn default_parent() -> PathBuf {
    if paths::packaged() { PATHS.home.join("games") } else { paths::work("games") }
}

/// Where new plain (not game) project folders go unless the user picks somewhere else.
pub fn default_parent_plain() -> PathBuf {
    if paths::packaged() { PATHS.home.join("projects") } else { paths::work("projects") }
}

/// A project's kind as asked for: "game" for a Roblox project ("roblox" means the same), "plain" for any other; None
/// for anything else ("auto": let the folder decide, `looks_roblox`).
pub fn kind_word(asked: Option<&str>) -> Option<&'static str> {
    match asked.map(|k| k.trim().to_ascii_lowercase()).as_deref() {
        Some("game" | "roblox") => Some("game"),
        Some("plain") => Some("plain"),
        _ => None,
    }
}

/// A saved project's kind: the one the user set, else what its folder looks like.
fn kind_of(entry: &Value) -> &'static str {
    kind_word(entry["kind"].as_str()).unwrap_or_else(|| detected_kind(Path::new(entry["path"].as_str().unwrap_or(""))))
}

/// "game" when the folder looks like a Roblox project, else "plain".
fn detected_kind(dir: &Path) -> &'static str {
    if looks_roblox(dir) { "game" } else { "plain" }
}

/// Whether a folder looks like a Roblox project: a place or model file (.rbxl, .rbxlx), a Rojo project
/// (default.project.json or any *.project.json), Wally, Rokit or Aftman (wally.toml, rokit.toml, aftman.toml), Luau
/// sources in `src` (a few folders down), or the place's scripts the app exported there (src/.manifest.json).
pub fn looks_roblox(dir: &Path) -> bool {
    const MARKERS: [&str; 4] = ["default.project.json", "wally.toml", "rokit.toml", "aftman.toml"];
    let place = |name: &str| {
        let n = name.to_ascii_lowercase();
        n.ends_with(".rbxl") || n.ends_with(".rbxlx")
    };
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if MARKERS.contains(&name.as_str()) || name.ends_with(".project.json") || place(&name) {
            return true;
        }
    }
    let src = dir.join("src");
    src.join(".manifest.json").is_file() || has_luau(&src, 3, &mut 0)
}

/// Whether a folder holds a .luau file within `depth` folders, looking at no more than 2000 entries in all.
fn has_luau(dir: &Path, depth: u32, seen: &mut u32) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    let mut subs = Vec::new();
    for e in entries.flatten() {
        *seen += 1;
        if *seen > 2000 {
            return false;
        }
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".luau") {
            return true;
        }
        if depth > 0 && !name.starts_with('.') && name != "node_modules" && e.file_type().is_ok_and(|t| t.is_dir()) {
            subs.push(e.path());
        }
    }
    subs.iter().any(|s| has_luau(s, depth - 1, seen))
}

/// Whether a project is a Roblox project: as the user set it, else as its folder looks (also for a folder that is not
/// on the list).
pub fn is_game(path: &str) -> bool {
    match saved().iter().find(|p| same(p["path"].as_str().unwrap_or(""), path)) {
        Some(p) => kind_of(p) == "game",
        None => looks_roblox(Path::new(path)),
    }
}

pub fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

fn saved() -> Vec<Value> {
    paths::setting("projects").as_array().cloned().unwrap_or_default()
}

fn save(list: Vec<Value>) {
    paths::update_config(|cfg| cfg.insert("projects".into(), json!(list)));
}

pub fn known(path: &str) -> bool {
    saved().iter().any(|p| same(p["path"].as_str().unwrap_or(""), path))
}

/// What the window shows about a project.
pub fn describe(path: &str) -> Value {
    let dir = PathBuf::from(path);
    let entry = saved().into_iter().find(|p| same(p["path"].as_str().unwrap_or(""), path));
    let name = entry.as_ref().and_then(|p| p["name"].as_str().map(str::to_string)).unwrap_or_else(|| paths::base_name(path));
    let set = entry.as_ref().and_then(|p| kind_word(p["kind"].as_str()));
    let kind = set.unwrap_or_else(|| detected_kind(&dir));
    // `type` too: an agent's "project" event puts its own `kind` over this one. `roblox`: the same as kind "game";
    // `kind_set`: the user chose it (else the folder decided)
    json!({"name": name, "path": path, "kind": kind, "type": kind, "roblox": kind == "game", "kind_set": set.is_some(),
           "exists": dir.is_dir(), "git": dir.join(".git").exists(), "bible": bible_path(&dir).is_some(),
           // the id the AIs' toolbox names it by (`fridae://project/<id>` links); as `projectId` too, because an event
           // (a chat's "project" event is this) has its own number put over `id`
           "id": crate::app_mcp::project_id(path), "projectId": crate::app_mcp::project_id(path)})
}

/// Known projects, most recently used first.
pub fn list() -> Vec<Value> {
    saved().iter().filter_map(|p| p["path"].as_str()).map(describe).filter(|p| p["exists"] == true).collect()
}

/// Put a project at the top of the list. Without a `kind`, a project already on the list keeps its own, and a new one
/// has none: its folder decides (`looks_roblox`).
fn remember(name: &str, path: &str, kind: Option<&str>) {
    save(remembered(saved(), name, path, kind));
}

fn remembered(list: Vec<Value>, name: &str, path: &str, kind: Option<&str>) -> Vec<Value> {
    let was = list.iter().find(|p| same(p["path"].as_str().unwrap_or(""), path)).and_then(|p| kind_word(p["kind"].as_str()));
    let kind = kind_word(kind).or(was);
    let mut list: Vec<Value> = list.into_iter().filter(|p| !same(p["path"].as_str().unwrap_or(""), path)).collect();
    let mut entry = json!({"name": name, "path": path});
    if let Some(k) = kind {
        entry["kind"] = json!(k);
    }
    list.insert(0, entry);
    list
}

/// Move a project to the top of the list.
pub fn touch(path: &str) {
    adopt_bible(Path::new(path));
    if let Some(name) = describe(path)["name"].as_str() {
        remember(name, path, None);
    }
}

/// Make a known project a Roblox project ("game" or "roblox") or a plain one ("plain"); anything else ("auto") lets
/// its folder decide again.
pub fn set_kind(path: &str, kind: &str) -> Result<Value, String> {
    if !known(path) {
        return Err("That is not one of your projects.".into());
    }
    let kind = kind_word(Some(kind));
    save(saved().into_iter().map(|mut p| {
        if same(p["path"].as_str().unwrap_or(""), path) {
            match (kind, p.as_object_mut()) {
                (Some(k), _) => p["kind"] = json!(k),
                (None, Some(o)) => {
                    o.remove("kind");
                }
                (None, None) => {}
            }
        }
        p
    }).collect());
    Ok(describe(path))
}

/// Give a known project another name. It is the name Fridae shows: the folder on disk keeps its own, so the project's
/// chats, its git history and every saved path still point at it.
pub fn rename(path: &str, name: &str) -> Result<Value, String> {
    if !known(path) {
        return Err("That is not one of your projects.".into());
    }
    save(renamed(saved(), path, name)?);
    Ok(describe(path))
}

fn renamed(list: Vec<Value>, path: &str, name: &str) -> Result<Vec<Value>, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the project a name.".into());
    }
    if name.chars().count() > 80 {
        return Err("That name is too long: keep it under 80 characters.".into());
    }
    Ok(list.into_iter().map(|mut p| {
        if same(p["path"].as_str().unwrap_or(""), path) {
            p["name"] = json!(name);
        }
        p
    }).collect())
}

pub fn forget(path: &str) {
    save(saved().into_iter().filter(|p| !same(p["path"].as_str().unwrap_or(""), path)).collect());
}

/// Why a folder may not go to the Trash, if it may not: it is (or holds) the user's home, the app's own folders or
/// the folder new games go in, or it holds another of the user's projects.
fn trash_refusal(dir: &Path, protected: &[PathBuf], others: &[PathBuf]) -> Option<String> {
    if dir.parent().is_none() || dir.components().count() < 3 {
        return Some("That folder is too close to the top of the disk to move to the Trash.".into());
    }
    if protected.iter().any(|p| p.starts_with(dir)) {
        return Some("That folder holds your home folder or Fridae's own folders, so it stays. Remove it from Fridae instead.".into());
    }
    if others.iter().any(|p| p != dir && p.starts_with(dir)) {
        return Some("That folder holds another of your projects, so it stays. Remove it from Fridae instead.".into());
    }
    None
}

/// Take a project off Fridae's list. With `trash`, its folder first goes to the Trash (macOS) or Recycle Bin
/// (Windows), where the user can put it back: nothing is ever deleted outright. The list is only changed once the
/// folder is gone, so a failed move leaves everything as it was.
pub fn delete(path: &str, trash: bool) -> Result<(), String> {
    if !known(path) {
        return Err("That is not one of your projects.".into());
    }
    if trash {
        let dir = fs::canonicalize(path).map_err(|_| "That folder is no longer there. Remove it from Fridae instead.".to_string())?;
        let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let mut protected = vec![canon(&PATHS.home), canon(&PATHS.kit), canon(&PATHS.work), canon(&PATHS.install), canon(&default_parent()), canon(&default_parent_plain())];
        if let Some(h) = std::env::var_os("HOME") {
            protected.push(canon(Path::new(&h)));
        }
        let others: Vec<PathBuf> = saved().iter().filter_map(|p| p["path"].as_str()).filter(|p| !same(p, path)).map(|p| canon(Path::new(p))).collect();
        if let Some(why) = trash_refusal(&dir, &protected, &others) {
            return Err(why);
        }
        to_trash(&dir)?;
    }
    forget(path);
    Ok(())
}

/// The programs that move a folder to the Trash, in the order they are tried, as (program, arguments): a Mac's
/// `/usr/bin/trash` (macOS 14 and later), else Finder through `osascript` (older macOS); Linux's `gio trash` (GLib,
/// there wherever GTK is), else KDE's `kioclient5`/`kioclient`. The folder is always an argument of its own, never
/// part of a script's text.
#[cfg_attr(windows, allow(dead_code))]
fn trash_commands(dir: &Path, mac: bool) -> Vec<(&'static str, Vec<std::ffi::OsString>)> {
    let d = dir.as_os_str().to_os_string();
    if mac {
        let finder = ["-e", "on run argv", "-e", "tell application \"Finder\" to delete (POSIX file (item 1 of argv) as alias)", "-e", "end run"];
        let mut osa: Vec<std::ffi::OsString> = finder.iter().map(|a| a.into()).collect();
        osa.push(d.clone());
        vec![("/usr/bin/trash", vec!["--".into(), d]), ("/usr/bin/osascript", osa)]
    } else {
        vec![
            ("gio", vec!["trash".into(), "--".into(), d.clone()]),
            ("kioclient5", vec!["move".into(), d.clone(), "trash:/".into()]),
            ("kioclient", vec!["move".into(), d, "trash:/".into()]),
        ]
    }
}

#[cfg(unix)]
fn to_trash(dir: &Path) -> Result<(), String> {
    let mac = cfg!(target_os = "macos");
    let mut last = String::new();
    for (program, args) in trash_commands(dir, mac) {
        let found = if program.starts_with('/') { Some(PathBuf::from(program)).filter(|p| p.is_file()) } else { paths::which(program) };
        let Some(found) = found else { continue };
        let mut cmd = Command::new(found);
        cmd.args(&args).stdin(Stdio::null());
        match cmd.run_output() {
            Ok(o) if o.status.success() && !dir.exists() => return Ok(()),
            Ok(o) => last = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => last = e.to_string(),
        }
    }
    if last.is_empty() {
        let how = if mac { "drag its folder to the Trash in Finder" } else { "move its folder to the Trash in your file manager" };
        return Err(format!("Fridae found no way to move folders to the Trash on this computer. Remove the project from Fridae, then {how}."));
    }
    Err(format!("The folder could not go to the Trash: {last}"))
}

#[cfg(windows)]
fn to_trash(dir: &Path) -> Result<(), String> {
    // the path goes in through the environment, never into the script's text
    let script = "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:FRIDAE_TRASH, 'OnlyErrorDialogs', 'SendToRecycleBin')";
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]).env("FRIDAE_TRASH", dir).stdin(Stdio::null());
    paths::quiet(&mut cmd);
    match cmd.run_output() {
        Ok(o) if o.status.success() && !dir.exists() => Ok(()),
        Ok(o) => Err(format!("The folder could not go to the Recycle Bin: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("The folder could not go to the Recycle Bin: {e}")),
    }
}

#[cfg(not(any(unix, windows)))]
fn to_trash(_dir: &Path) -> Result<(), String> {
    Err("Fridae cannot move folders to the Trash here. Remove the project from Fridae, then delete its folder yourself.".into())
}

/// Make a new folder for a project, optionally as a git repository. A new project is plain unless `kind` makes it a
/// Roblox project ("game" or "roblox").
pub fn create(name: &str, parent: Option<&str>, git: bool, kind: Option<&str>) -> Result<Value, String> {
    // kept only when asked for: otherwise the folder decides later too (a Rojo project made in it later is Roblox)
    let asked = kind_word(kind);
    let kind = asked.unwrap_or("plain");
    let name = name.trim();
    let folder: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let folder = folder.trim().to_string();
    if folder.is_empty() {
        return Err("Give the project a name first.".into());
    }
    let parent = parent.map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| if kind == "plain" { default_parent_plain() } else { default_parent() });
    let dir = parent.join(&folder);
    if dir.exists() && fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} already exists and is not empty. Use \"Existing folder\" to work in it.", dir.display()));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let path = dir.display().to_string();
    let mut note = String::new();
    if git {
        let (ok, text) = run_git(&dir, &["init"]);
        if !ok {
            note = format!("The folder was made, but git could not start a repository: {text}");
        }
    }
    remember(name, &path, asked);
    let mut p = describe(&path);
    p["note"] = json!(note);
    Ok(p)
}

/// Start using a folder that already exists. A `kind` makes it a Roblox or a plain project; without one, a project
/// already on the list keeps its kind and a new one is what its folder looks like (`looks_roblox`).
pub fn add(path: &str, kind: Option<&str>) -> Result<Value, String> {
    let dir = PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Err("That folder does not exist.".into());
    }
    adopt_bible(&dir);
    let path = dir.display().to_string();
    let name = describe(&path)["name"].as_str().map(str::to_string).unwrap_or_else(|| paths::base_name(&path));
    remember(&name, &path, kind);
    Ok(describe(&path))
}

/// Ask the user for a folder with the system's own dialog.
pub fn pick_folder() -> Option<String> {
    crate::paths::dialog::Dialog::new().title("Choose the project's folder").pick_folder().map(|p| p.display().to_string())
}

pub fn open(path: &str, what: &str) -> bool {
    if !known(path) {
        return false;
    }
    let dir = PathBuf::from(path);
    let target = if what == "bible" { adopt_bible(&dir); bible_path(&dir).unwrap_or_else(|| dir.join(BIBLE)) } else { dir };
    target.exists() && crate::paths::open_that(target).is_ok()
}

// ---------------------------------------------------------------------- git
/// The repository's state: branch, changed files, recent commits and the remote.
pub fn git_state(path: &str) -> Value {
    let dir = PathBuf::from(path);
    if !git_installed() {
        return json!({"installed": false, "repo": false});
    }
    if !dir.join(".git").exists() {
        return json!({"installed": true, "repo": false});
    }
    let (_, status) = run_git(&dir, &["status", "--porcelain=v1", "-b"]);
    let mut lines = status.lines();
    let head = lines.next().unwrap_or("").trim_start_matches("## ").to_string();
    let branch = head.split("...").next().unwrap_or("").trim_start_matches("No commits yet on ").to_string();
    let changes: Vec<String> = lines.map(str::to_string).collect();
    let (ok, log) = run_git(&dir, &["log", "-12", "--pretty=format:%h\t%ar\t%s"]);
    let commits: Vec<Value> = if ok {
        log.lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                Some(json!({"hash": p.next()?, "when": p.next()?, "subject": p.next().unwrap_or("")}))
            })
            .collect()
    } else {
        Vec::new()
    };
    let (has_remote, remote) = run_git(&dir, &["remote", "get-url", "origin"]);
    let (has_name, _) = run_git(&dir, &["config", "user.name"]);
    json!({
        "installed": true, "repo": true, "branch": branch, "ahead": head.contains("ahead"), "changes": changes, "commits": commits,
        "remote": if has_remote { remote } else { String::new() }, "identity": has_name,
    })
}

/// Commit everything in the project as it is now and return the commit id. Uses the project's own git identity,
/// or a neutral one when the user has not set one (these commits never leave the computer unless the user pushes).
fn commit_all(dir: &Path, message: &str) -> Option<String> {
    if !git_installed() || !dir.join(".git").exists() {
        return None;
    }
    let mut args: Vec<&str> = Vec::new();
    if !run_git(dir, &["config", "user.name"]).0 {
        args.extend(["-c", "user.name=Fridae", "-c", "user.email=fridae-studios@localhost"]);
    }
    if !run_git(dir, &["add", "-A"]).0 {
        return None;
    }
    let mut commit = args.clone();
    commit.extend(["commit", "--allow-empty", "--no-verify", "-q", "-m", message]);
    if !run_git(dir, &commit).0 {
        return None;
    }
    let (ok, id) = run_git(dir, &["rev-parse", "HEAD"]);
    ok.then(|| id.trim().to_string()).filter(|id| !id.is_empty())
}

/// A checkpoint of the project's files before an overnight run, or before a studio run. None when the folder has no
/// git repository.
pub fn night_checkpoint(path: &str, studio: bool) -> Option<String> {
    checkpoint(path, night_message(studio))
}

/// The start commit's message: a studio run says so; an ordinary Overnight keeps its wording.
pub fn night_message(studio: bool) -> &'static str {
    if studio { "Before the studio run (Fridae checkpoint)" } else { "Before the overnight run (Fridae checkpoint)" }
}

/// A checkpoint of the project's files with this commit message. None when the folder has no git repository.
pub fn checkpoint(path: &str, message: &str) -> Option<String> {
    commit_all(Path::new(path), message)
}

/// The folder is a git repository and git is installed: a checkpoint of it can be committed.
pub fn is_repo(path: &str) -> bool {
    git_installed() && Path::new(path).join(".git").exists()
}

/// The commit is in the project's history (checked before anything is undone).
pub fn commit_exists(path: &str, commit: &str) -> bool {
    commit.chars().all(|c| c.is_ascii_hexdigit()) && commit.len() >= 7 && run_git(Path::new(path), &["cat-file", "-e", &format!("{commit}^{{commit}}")]).0
}

/// How a restore names what it undoes: in the text ("night's", "run's") and in its two commits.
pub struct Undoing<'a> {
    pub whose: &'a str,
    pub end_commit: &'a str,
    pub undo_commit: &'a str,
}

const NIGHT: Undoing<'static> = Undoing { whose: "night's", end_commit: "End of the overnight run (kept in history)", undo_commit: "Undo the overnight run (Fridae)" };

/// Put the project's files back as they were at `commit`, as a new commit, so the night's work stays in the history
/// and this can itself be undone. The board (.fridae) is left as it is, so the night's cards can still be read.
pub fn restore_to(path: &str, commit: &str) -> Result<String, String> {
    restore_with(path, commit, &NIGHT)
}

/// `restore_to`, with its own words for what is undone.
pub fn restore_with(path: &str, commit: &str, w: &Undoing) -> Result<String, String> {
    let dir = Path::new(path);
    if !commit.chars().all(|c| c.is_ascii_hexdigit()) || commit.len() < 7 {
        return Err("The checkpoint id is not valid.".to_string());
    }
    let end = commit_all(dir, w.end_commit).ok_or_else(|| format!("Could not record the {} files before undoing them.", w.whose))?;
    if end == commit {
        return Ok("The project's files had not changed.".to_string());
    }
    let (changed, list) = run_git(dir, &["diff", "--name-only", commit, &end, "--", ".", ":(exclude).fridae"]);
    let count = if changed { list.lines().filter(|l| !l.trim().is_empty()).count() } else { 0 };
    let (ok, out) = run_git(dir, &["restore", "--source", commit, "--staged", "--worktree", "--", ".", ":(exclude).fridae"]);
    if !ok {
        return Err(format!("Git could not put the files back: {out}"));
    }
    commit_all(dir, w.undo_commit).ok_or("The files were put back, but git could not record it.")?;
    Ok(match count {
        0 => "The project's files had not changed.".to_string(),
        1 => format!("1 file in the project was put back. The {} version is still in git history.", w.whose),
        n => format!("{n} files in the project were put back. The {} versions are still in git history.", w.whose),
    })
}

/// One git action from the window. Only folders that are known projects are touched.
pub fn git(path: &str, action: &str, body: &Value) -> Value {
    if !known(path) || !PathBuf::from(path).is_dir() {
        return json!({"ok": false, "text": "That folder is not one of your projects."});
    }
    if !git_installed() {
        return json!({"ok": false, "text": "Git is not installed. Get it from https://git-scm.com and reopen the app.", "state": git_state(path)});
    }
    let dir = PathBuf::from(path);
    let text_of = |k: &str| body[k].as_str().unwrap_or("").trim().to_string();
    let (ok, text) = match action {
        "status" => (true, String::new()),
        "init" => run_git(&dir, &["init"]),
        "identity" => {
            let (name, email) = (text_of("name"), text_of("email"));
            if name.is_empty() || email.is_empty() {
                (false, "Enter both a name and an email. They are written into your commits.".to_string())
            } else {
                let a = run_git(&dir, &["config", "user.name", &name]);
                let b = run_git(&dir, &["config", "user.email", &email]);
                (a.0 && b.0, if a.0 && b.0 { "Saved for this project.".to_string() } else { format!("{}\n{}", a.1, b.1) })
            }
        }
        "commit" => {
            let message = text_of("message");
            if message.is_empty() {
                (false, "Write a short note about what changed first.".to_string())
            } else {
                let (added, out) = run_git(&dir, &["add", "-A"]);
                if added { run_git(&dir, &["commit", "-m", &message]) } else { (false, out) }
            }
        }
        "remote" => {
            let url = text_of("url");
            if url.is_empty() {
                run_git(&dir, &["remote", "remove", "origin"])
            } else if run_git(&dir, &["remote", "get-url", "origin"]).0 {
                run_git(&dir, &["remote", "set-url", "origin", &url])
            } else {
                run_git(&dir, &["remote", "add", "origin", &url])
            }
        }
        "push" => run_git(&dir, &["push", "-u", "origin", "HEAD"]),
        "pull" => run_git(&dir, &["pull", "--ff-only"]),
        _ => (false, "Unknown action.".to_string()),
    };
    let text = if !ok && text.contains("tell me who you are") {
        "Git does not know who you are yet. Fill in your name and email below, then commit again.".to_string()
    } else {
        text
    };
    json!({"ok": ok, "text": text, "state": git_state(path)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_saved_before_kinds_are_games_and_plain_is_kept() {
        assert_eq!(kind_of(&json!({"name": "Lake", "path": "/no/such/Lake"})), "plain", "no kind and nothing Roblox in it");
        assert_eq!(kind_of(&json!({"kind": "plain"})), "plain");
        assert_eq!(kind_of(&json!({"kind": "game", "path": "/no/such/Lake"})), "game", "the user's choice wins");
        assert_eq!(kind_word(Some("Plain")), Some("plain"));
        assert_eq!(kind_word(Some("game")), Some("game"));
        assert_eq!(kind_word(Some(" Roblox ")), Some("game"));
        assert_eq!(kind_word(Some("auto")), None);
        assert_eq!(kind_word(None), None);
    }

    #[test]
    fn remembering_keeps_a_projects_kind_unless_told_otherwise() {
        let list = vec![json!({"name": "Tool", "path": "/p/Tool", "kind": "plain"}), json!({"name": "Lake", "path": "/g/Lake"})];
        let touched = remembered(list.clone(), "Tool", "/p/Tool", None);
        assert_eq!(touched[0], json!({"name": "Tool", "path": "/p/Tool", "kind": "plain"}));
        assert_eq!(touched.len(), 2);
        let made_game = remembered(list.clone(), "Tool", "/p/Tool", Some("game"));
        assert_eq!(made_game[0]["kind"], "game");
        let old = remembered(list.clone(), "Lake", "/g/Lake", None);
        assert_eq!(old[0], json!({"name": "Lake", "path": "/g/Lake"}), "no kind set: the folder decides");
        let new = remembered(list.clone(), "App", "/p/App", Some("plain"));
        assert_eq!((new[0]["kind"].as_str(), new.len()), (Some("plain"), 3));
        let fresh = remembered(list, "Other", "/p/Other", None);
        assert!(fresh[0].get("kind").is_none());
    }

    #[test]
    fn renaming_a_project_changes_only_the_name_fridae_shows() {
        let list = vec![json!({"name": "Lake", "path": "/g/Lake", "kind": "game"}), json!({"name": "Tool", "path": "/p/Tool"})];
        let out = renamed(list.clone(), "/g/Lake/", "  Lake   of Fire ").unwrap();
        assert_eq!(out[0], json!({"name": "Lake of Fire", "path": "/g/Lake", "kind": "game"}), "the folder and the kind stay");
        assert_eq!(out[1], list[1], "other projects are untouched, and the order stays");
        assert!(renamed(list.clone(), "/g/Lake", "   ").is_err(), "no empty name");
        assert!(renamed(list.clone(), "/g/Lake", &"x".repeat(81)).is_err(), "no endless name");
        // the new name outlives the project being used again: only its place in the list moves
        let used = remembered(out, "Lake of Fire", "/g/Lake", None);
        assert_eq!(used[0], json!({"name": "Lake of Fire", "path": "/g/Lake", "kind": "game"}));
    }

    #[test]
    fn a_game_folder_may_go_to_the_trash() {
        let home = PathBuf::from("/Users/me");
        let protected = [home.clone(), home.join("Documents/Fridae Studios/games")];
        assert_eq!(trash_refusal(&home.join("Documents/Fridae Studios/games/Lake"), &protected, &[]), None);
        assert_eq!(trash_refusal(&home.join("Desktop/My Game"), &protected, &[home.join("Desktop/Other")]), None);
    }

    #[test]
    fn home_app_and_games_folders_and_their_parents_stay() {
        let home = PathBuf::from("/Users/me");
        let protected = [home.clone(), home.join("Documents/Fridae Studios/games")];
        for dir in ["/", "/Users", "/Users/me", "/Users/me/Documents", "/Users/me/Documents/Fridae Studios/games"] {
            assert!(trash_refusal(Path::new(dir), &protected, &[]).is_some(), "{dir} should stay");
        }
    }

    #[test]
    fn roblox_projects_are_seen_in_their_folder() {
        let root = paths::test_dir("looks-roblox");
        let make = |name: &str, files: &[&str]| {
            let d = root.join(name);
            fs::create_dir_all(&d).unwrap();
            for f in files {
                let p = d.join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, "").unwrap();
            }
            d
        };
        for (name, files) in [
            ("place", &["build/notes.txt", "Lake.rbxl"][..]),
            ("xml-place", &["Lake.RBXLX"][..]),
            ("rojo", &["default.project.json"][..]),
            ("rojo-named", &["lake.project.json"][..]),
            ("wally", &["wally.toml"][..]),
            ("rokit", &["rokit.toml"][..]),
            ("luau", &["src/server/game/Main.server.luau"][..]),
            ("exported", &["src/.manifest.json"][..]),
        ] {
            assert!(looks_roblox(&make(name, files)), "{name}");
        }
        for (name, files) in [
            ("web", &["package.json", "src/index.ts", "README.md"][..]),
            ("deep", &["src/a/b/c/d/far.luau"][..]),
            ("deps", &["src/node_modules/x/y.luau"][..]),
            ("rust", &["Cargo.toml", "src/main.rs", "GAME.md"][..]),
        ] {
            assert!(!looks_roblox(&make(name, files)), "{name}");
        }
        assert!(!looks_roblox(&root.join("not-there")));
        assert_eq!(detected_kind(&root.join("rojo")), "game");
        assert_eq!(detected_kind(&root.join("web")), "plain");
        assert!(is_game(&root.join("rojo").display().to_string()), "a folder not on the list: as it looks");
    }

    #[test]
    fn game_md_becomes_project_md_once() {
        let root = paths::test_dir("project-bible");
        let (old_only, both, neither) = (root.join("old"), root.join("both"), root.join("neither"));
        for d in [&old_only, &both, &neither] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(old_only.join("GAME.md"), "# Lake\n\nThe decisions.\n").unwrap();
        // read as it is until it is renamed
        assert_eq!(bible_path(&old_only), Some(old_only.join("GAME.md")));
        assert!(adopt_bible(&old_only));
        assert!(!old_only.join("GAME.md").exists());
        assert_eq!(fs::read_to_string(old_only.join("PROJECT.md")).unwrap(), "# Lake\n\nThe decisions.\n", "the content is kept");
        assert_eq!(bible_path(&old_only), Some(old_only.join("PROJECT.md")));
        assert!(!adopt_bible(&old_only), "nothing left to rename");
        // both: both stay, PROJECT.md is the one read
        fs::write(both.join("GAME.md"), "old").unwrap();
        fs::write(both.join("PROJECT.md"), "new").unwrap();
        assert!(!adopt_bible(&both));
        assert_eq!(fs::read_to_string(both.join("GAME.md")).unwrap(), "old");
        assert_eq!(bible_path(&both), Some(both.join("PROJECT.md")));
        assert!(!adopt_bible(&neither));
        assert_eq!(bible_path(&neither), None);
    }

    #[test]
    fn a_folder_holding_another_project_stays() {
        let protected = [PathBuf::from("/Users/me")];
        let why = trash_refusal(Path::new("/Users/me/Desktop/Games"), &protected, &[PathBuf::from("/Users/me/Desktop/Games/Lake")]);
        assert!(why.is_some_and(|w| w.contains("another of your projects")));
    }

    #[test]
    fn describe_has_an_id() {
        let d = describe("C:/No/Such/Shop/");
        assert_eq!(d["id"], json!(crate::app_mcp::project_id("c:\\no\\such\\shop")));
        assert_eq!(d["id"].as_str().map(str::len), Some(13));
    }

    #[test]
    fn a_project_event_keeps_the_project_id() {
        // the event bus numbers every event under "id", so the project's id travels as "projectId" too
        let path = "C:/No/Such/Castle";
        let n = crate::agents::BUS.after(0, 0.0).len();
        crate::agents::BUS.put("project", describe(path));
        let ev = crate::agents::BUS.after(n, 0.0).into_iter().find(|e| e["kind"] == "project" && e["path"] == path).unwrap();
        assert_eq!(ev["projectId"], json!(crate::app_mcp::project_id(path)));
    }

    #[test]
    fn a_folder_goes_to_each_systems_trash_as_an_argument_of_its_own() {
        let dir = Path::new("/home/a/Games/My \"Shop\"");
        let linux = trash_commands(dir, false);
        let programs: Vec<&str> = linux.iter().map(|(p, _)| *p).collect();
        assert_eq!(programs, ["gio", "kioclient5", "kioclient"]);
        assert_eq!(linux[0].1, ["trash", "--", dir.to_str().unwrap()].map(std::ffi::OsString::from));
        assert_eq!(linux[1].1, ["move", dir.to_str().unwrap(), "trash:/"].map(std::ffi::OsString::from));
        let mac = trash_commands(dir, true);
        assert_eq!(mac[0], ("/usr/bin/trash", ["--", dir.to_str().unwrap()].map(std::ffi::OsString::from).to_vec()));
        assert_eq!(mac[1].0, "/usr/bin/osascript", "older macOS: Finder");
        assert_eq!(mac[1].1.last().unwrap(), dir.as_os_str(), "the folder is the script's argument");
        assert!(mac[1].1.iter().all(|a| a == dir.as_os_str() || !a.to_string_lossy().contains("Shop")), "never in the script's text");
    }
}
