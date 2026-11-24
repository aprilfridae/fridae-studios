//! The user's skills for every AI: the catalog part of an agent's standing instructions.
//!
//! A skill is a folder with a SKILL.md. Claude Code loads its own (its skills folder, its enabled plugins, what it
//! synced for the account it runs on); Gemini, Cursor, OpenCode and Codex load none of them. So the tool server
//! serves every skill to every AI (`rbx_skill`, `rbxskills.py`), and each agent's instructions carry a short list of
//! them, made by the same Python (`rbx skills catalog`), so the list and the tool always agree. For Claude the list
//! leaves out what Claude Code already lists itself.
//!
//! Making the list costs a Python start (about 0.05 s, 0.5 s through the bundled program), so it is kept and made
//! again only when a folder skills live in changes, or after `KEEP`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::paths::{self, PATHS};

/// How long a made list is used while none of the watched folders changes (a SKILL.md edited in place changes none).
const KEEP: Duration = Duration::from_secs(600);
/// How long making the list may take before the turn goes without it.
const LIMIT: Duration = Duration::from_secs(8);

struct Kept {
    made: Instant,
    print: Vec<Option<SystemTime>>,
    text: String,
}

static CACHE: Mutex<Option<HashMap<(String, String, String), Kept>>> = Mutex::new(None);

/// The user's own Claude Code folder (`CLAUDE_CONFIG_DIR` when the app was started with one), as `accounts` reads it.
fn users_claude() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| paths::home_dir().join(".claude"))
}

/// The folders whose change means a skill came or went (each synced account's folder too), and the files that
/// decide which plugins are on.
fn watched(project: Option<&str>) -> Vec<PathBuf> {
    let claude = users_claude();
    let home = paths::home_dir();
    let mut out = vec![
        claude.join("skills"),
        claude.join("settings.json"),
        claude.join("plugins").join("installed_plugins.json"),
        home.join(".claude.json"),
    ];
    for base in [claude.join("plugins").join("synced"), claude.join("skills").join("synced")] {
        out.push(base.clone());
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            subs.sort();
            out.extend(subs);
        }
    }
    for rel in [".agents/skills", ".codex/skills", ".cursor/skills", ".config/opencode/skills", ".config/opencode/skill", ".gemini/skills"] {
        out.push(home.join(rel));
    }
    if let Some(p) = project {
        out.push(Path::new(p).join(".claude").join("skills"));
        out.push(Path::new(p).join(".agents").join("skills"));
    }
    out
}

fn fingerprint(paths: &[PathBuf]) -> Vec<Option<SystemTime>> {
    paths.iter().map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).collect()
}

/// The command that prints the catalog: the module itself when the kit runs from source, else the bundled program.
fn command(args: &[&str]) -> std::process::Command {
    match &PATHS.cli_exe {
        Some(_) => {
            let mut all = vec!["skills"];
            all.extend_from_slice(args);
            paths::cli(&all)
        }
        None => {
            let mut c = std::process::Command::new(paths::python());
            c.arg(PATHS.kit.join("rbxskills.py")).arg("skills").args(args);
            paths::quiet(&mut c);
            c
        }
    }
}

fn make(ai: &str, project: Option<&str>, claude_config: Option<&Path>) -> Option<String> {
    let config = claude_config.map(|p| p.display().to_string()).unwrap_or_default();
    let mut args = vec!["catalog", "--ai", ai, "--project", project.unwrap_or("")];
    if !config.is_empty() {
        args.extend_from_slice(&["--claude-config", &config]);
    }
    let mut cmd = command(&args);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if began.elapsed() < LIMIT => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The skills part of an agent's standing instructions for this AI ("claude" leaves out Claude Code's own) and
/// project; `claude_config`: a second Claude account's folder. Empty when there are no skills to list or the list
/// could not be made (the turn goes on without it; `rbx_skill` still lists them).
pub fn catalog(ai: &str, project: Option<&str>, claude_config: Option<&Path>) -> String {
    // every AI but Claude gets the same list
    let who = if ai == "claude" { "claude" } else { "other" };
    let key = (who.to_string(), project.unwrap_or("").to_string(), claude_config.map(|p| p.display().to_string()).unwrap_or_default());
    let print = fingerprint(&watched(project));
    if let Some(k) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        if k.print == print && k.made.elapsed() < KEEP {
            return k.text.clone();
        }
    }
    let Some(text) = make(who, project, claude_config) else {
        return String::new();
    };
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, Kept { made: Instant::now(), print, text: text.clone() });
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_change_shows_in_the_fingerprint() {
        let dir = std::env::temp_dir().join(format!("fridae-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let watched = vec![dir.clone()];
        let before = fingerprint(&watched);
        assert_eq!(before, vec![None], "a missing folder is part of the fingerprint as missing");
        std::fs::create_dir_all(&dir).unwrap();
        let made = fingerprint(&watched);
        assert_ne!(before, made, "a skills folder that appears is noticed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_adds_its_own_skill_folders() {
        let w = watched(Some("/tmp/some game"));
        assert!(w.contains(&PathBuf::from("/tmp/some game/.claude/skills")) && w.contains(&PathBuf::from("/tmp/some game/.agents/skills")));
        assert!(!watched(None).iter().any(|p| p.starts_with("/tmp/some game")));
    }
}
