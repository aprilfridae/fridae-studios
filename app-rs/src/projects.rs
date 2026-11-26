//! Projects: one folder each, usually a git repository. A project is a game (GAME.md, the game bible, and the
//! place's exported scripts; the Roblox tools, Ideas and Promos) or a plain one: a regular program or anything else.

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

/// Where new game folders go unless the user picks somewhere else.
pub fn default_parent() -> PathBuf {
    if paths::packaged() { PATHS.home.join("games") } else { paths::work("games") }
}

/// Where new plain (not game) project folders go unless the user picks somewhere else.
pub fn default_parent_plain() -> PathBuf {
    if paths::packaged() { PATHS.home.join("projects") } else { paths::work("projects") }
}

/// A project's kind as asked for: "plain" for a plain project, "game" for anything else.
pub fn kind_word(asked: Option<&str>) -> &'static str {
    if asked.is_some_and(|k| k.trim().eq_ignore_ascii_case("plain")) { "plain" } else { "game" }
}

/// A saved project's kind. Projects saved before kinds existed are games.
fn kind_of(entry: &Value) -> &'static str {
    kind_word(entry["kind"].as_str())
}

/// Whether a known project is a game (and an unknown folder is taken as one, as before kinds existed).
pub fn is_game(path: &str) -> bool {
    saved().iter().find(|p| same(p["path"].as_str().unwrap_or(""), path)).is_none_or(|p| kind_of(p) == "game")
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
    let kind = entry.as_ref().map_or("game", kind_of);
    // `type` too: an agent's "project" event puts its own `kind` over this one
    json!({"name": name, "path": path, "kind": kind, "type": kind, "exists": dir.is_dir(), "git": dir.join(".git").exists(), "bible": dir.join("GAME.md").is_file()})
}

/// Known projects, most recently used first.
pub fn list() -> Vec<Value> {
    saved().iter().filter_map(|p| p["path"].as_str()).map(describe).filter(|p| p["exists"] == true).collect()
}

/// Put a project at the top of the list. Without a `kind`, a project already on the list keeps its own.
fn remember(name: &str, path: &str, kind: Option<&str>) {
    save(remembered(saved(), name, path, kind));
}

fn remembered(list: Vec<Value>, name: &str, path: &str, kind: Option<&str>) -> Vec<Value> {
    let was = list.iter().find(|p| same(p["path"].as_str().unwrap_or(""), path)).map(kind_of);
    let kind = kind.map(|k| kind_word(Some(k))).or(was).unwrap_or("game");
    let mut list: Vec<Value> = list.into_iter().filter(|p| !same(p["path"].as_str().unwrap_or(""), path)).collect();
    list.insert(0, json!({"name": name, "path": path, "kind": kind}));
    list
}

/// Move a project to the top of the list.
pub fn touch(path: &str) {
    if let Some(name) = describe(path)["name"].as_str() {
        remember(name, path, None);
    }
}

/// Make a known project a game or a plain project.
pub fn set_kind(path: &str, kind: &str) -> Result<Value, String> {
    if !known(path) {
        return Err("That is not one of your projects.".into());
    }
    let kind = kind_word(Some(kind));
    save(saved().into_iter().map(|mut p| {
        if same(p["path"].as_str().unwrap_or(""), path) {
            p["kind"] = json!(kind);
        }
        p
    }).collect());
    Ok(describe(path))
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

#[cfg(target_os = "macos")]
fn to_trash(dir: &Path) -> Result<(), String> {
    let tool = Path::new("/usr/bin/trash");
    if !tool.exists() {
        return Err("This Mac has no trash command. Remove the project from Fridae, then drag its folder to the Trash in Finder.".into());
    }
    let mut cmd = Command::new(tool);
    cmd.arg("--").arg(dir).stdin(Stdio::null());
    match cmd.run_output() {
        Ok(o) if o.status.success() && !dir.exists() => Ok(()),
        Ok(o) => Err(format!("The folder could not go to the Trash: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("The folder could not go to the Trash: {e}")),
    }
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

#[cfg(not(any(target_os = "macos", windows)))]
fn to_trash(_dir: &Path) -> Result<(), String> {
    Err("Fridae cannot move folders to the Trash here. Remove the project from Fridae, then delete its folder yourself.".into())
}

/// Make a new folder for a game or a plain project, optionally as a git repository.
pub fn create(name: &str, parent: Option<&str>, git: bool, kind: Option<&str>) -> Result<Value, String> {
    let kind = kind_word(kind);
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
    remember(name, &path, Some(kind));
    let mut p = describe(&path);
    p["note"] = json!(note);
    Ok(p)
}

/// Start using a folder that already exists. A `kind` makes it a game or a plain project; without one, a project
/// already on the list keeps its kind and a new one is a game.
pub fn add(path: &str, kind: Option<&str>) -> Result<Value, String> {
    let dir = PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Err("That folder does not exist.".into());
    }
    let path = dir.display().to_string();
    let name = describe(&path)["name"].as_str().map(str::to_string).unwrap_or_else(|| paths::base_name(&path));
    remember(&name, &path, kind);
    Ok(describe(&path))
}

/// Ask the user for a folder with the system's own dialog.
pub fn pick_folder() -> Option<String> {
    rfd::FileDialog::new().set_title("Choose the project's folder").pick_folder().map(|p| p.display().to_string())
}

pub fn open(path: &str, what: &str) -> bool {
    if !known(path) {
        return false;
    }
    let dir = PathBuf::from(path);
    let target = if what == "bible" { dir.join("GAME.md") } else { dir };
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
        assert_eq!(kind_of(&json!({"name": "Lake", "path": "/g/Lake"})), "game");
        assert_eq!(kind_of(&json!({"kind": "plain"})), "plain");
        assert_eq!(kind_word(Some("Plain")), "plain");
        assert_eq!(kind_word(Some("game")), "game");
        assert_eq!(kind_word(Some("anything")), "game");
        assert_eq!(kind_word(None), "game");
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
        assert_eq!(old[0]["kind"], "game");
        let new = remembered(list, "App", "/p/App", Some("plain"));
        assert_eq!((new[0]["kind"].as_str(), new.len()), (Some("plain"), 3));
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
    fn a_folder_holding_another_project_stays() {
        let protected = [PathBuf::from("/Users/me")];
        let why = trash_refusal(Path::new("/Users/me/Desktop/Games"), &protected, &[PathBuf::from("/Users/me/Desktop/Games/Lake")]);
        assert!(why.is_some_and(|w| w.contains("another of your projects")));
    }
}
