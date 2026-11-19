//! Game projects: one folder per game, holding GAME.md (the game bible), the
//! place's exported scripts, and usually a git repository.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{Value, json};

use crate::paths::{self, PATHS};

pub fn git_installed() -> bool {
    paths::which("git").is_some()
}

/// Run git in a folder. Returns whether it succeeded and what it printed.
fn run_git(dir: &Path, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    paths::quiet(&mut cmd);
    match cmd.output() {
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

fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

fn saved() -> Vec<Value> {
    paths::load_config().get("projects").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn save(list: Vec<Value>) {
    let mut cfg = paths::load_config();
    cfg.insert("projects".into(), json!(list));
    paths::save_config(&cfg);
}

fn known(path: &str) -> bool {
    saved().iter().any(|p| same(p["path"].as_str().unwrap_or(""), path))
}

/// What the window shows about a project.
pub fn describe(path: &str) -> Value {
    let dir = PathBuf::from(path);
    let name = saved()
        .iter()
        .find(|p| same(p["path"].as_str().unwrap_or(""), path))
        .and_then(|p| p["name"].as_str().map(str::to_string))
        .unwrap_or_else(|| paths::base_name(path));
    json!({"name": name, "path": path, "exists": dir.is_dir(), "git": dir.join(".git").exists(), "bible": dir.join("GAME.md").is_file()})
}

/// Known projects, most recently used first.
pub fn list() -> Vec<Value> {
    saved().iter().filter_map(|p| p["path"].as_str()).map(describe).filter(|p| p["exists"] == true).collect()
}

fn remember(name: &str, path: &str) {
    let mut list: Vec<Value> = saved().into_iter().filter(|p| !same(p["path"].as_str().unwrap_or(""), path)).collect();
    list.insert(0, json!({"name": name, "path": path}));
    save(list);
}

/// Move a project to the top of the list.
pub fn touch(path: &str) {
    if let Some(name) = describe(path)["name"].as_str() {
        remember(name, path);
    }
}

pub fn forget(path: &str) {
    save(saved().into_iter().filter(|p| !same(p["path"].as_str().unwrap_or(""), path)).collect());
}

/// Make a new folder for a game, optionally as a git repository.
pub fn create(name: &str, parent: Option<&str>, git: bool) -> Result<Value, String> {
    let name = name.trim();
    let folder: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let folder = folder.trim().to_string();
    if folder.is_empty() {
        return Err("Give the game a name first.".into());
    }
    let parent = parent.map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(default_parent);
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
    remember(name, &path);
    let mut p = describe(&path);
    p["note"] = json!(note);
    Ok(p)
}

/// Start using a folder that already exists.
pub fn add(path: &str) -> Result<Value, String> {
    let dir = PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Err("That folder does not exist.".into());
    }
    let path = dir.display().to_string();
    if !known(&path) {
        remember(&paths::base_name(&path), &path);
    } else {
        touch(&path);
    }
    Ok(describe(&path))
}

/// Ask the user for a folder with the system's own dialog.
pub fn pick_folder() -> Option<String> {
    rfd::FileDialog::new().set_title("Choose the game's folder").pick_folder().map(|p| p.display().to_string())
}

pub fn open(path: &str, what: &str) -> bool {
    if !known(path) {
        return false;
    }
    let dir = PathBuf::from(path);
    let target = if what == "bible" { dir.join("GAME.md") } else { dir };
    target.exists() && open::that(target).is_ok()
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
        args.extend(["-c", "user.name=Fridae Studios", "-c", "user.email=fridae-studios@localhost"]);
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

/// A checkpoint of the project's files before an overnight run. None when the folder has no git repository.
pub fn night_checkpoint(path: &str) -> Option<String> {
    commit_all(Path::new(path), "Before the overnight run (Fridae Studios checkpoint)")
}

/// Put the project's files back as they were at `commit`, as a new commit, so the night's work stays in the history
/// and this can itself be undone. The board (.fridae) is left as it is, so the night's cards can still be read.
pub fn restore_to(path: &str, commit: &str) -> Result<String, String> {
    let dir = Path::new(path);
    if !commit.chars().all(|c| c.is_ascii_hexdigit()) || commit.len() < 7 {
        return Err("The checkpoint id is not valid.".to_string());
    }
    let end = commit_all(dir, "End of the overnight run (kept in history)").ok_or("Could not record the night's files before undoing them.")?;
    if end == commit {
        return Ok("The project's files had not changed.".to_string());
    }
    let (changed, list) = run_git(dir, &["diff", "--name-only", commit, &end, "--", ".", ":(exclude).fridae"]);
    let count = if changed { list.lines().filter(|l| !l.trim().is_empty()).count() } else { 0 };
    let (ok, out) = run_git(dir, &["restore", "--source", commit, "--staged", "--worktree", "--", ".", ":(exclude).fridae"]);
    if !ok {
        return Err(format!("Git could not put the files back: {out}"));
    }
    commit_all(dir, "Undo the overnight run (Fridae Studios)").ok_or("The files were put back, but git could not record it.")?;
    Ok(match count {
        0 => "The project's files had not changed.".to_string(),
        1 => "1 file in the project was put back. The night's version is still in git history.".to_string(),
        n => format!("{n} files in the project were put back. The night's versions are still in git history."),
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
