//! A small ledger of the long-lived helper processes the app starts (today: OpenCode's server), so one left behind by
//! an app that crashed or was killed is ended the next time the app starts.
//!
//! Each entry holds the process's pid, its process group, its start time and its command line as `ps` reports them,
//! and the same for the copy of the app that started it. At start-up an entry is only acted on when a process with
//! that pid is still there with exactly the same start time and command line (a pid the system has since given to
//! anything else is never touched), and the app that started it is gone (another copy of the app still running keeps
//! its own). Unix only; on Windows the
//! ledger is kept empty (no helper that needs it runs there yet).
//!
//! Kept general on purpose, so the other AIs' long-lived helpers can be recorded the same way.

use std::{fs, path::Path, path::PathBuf, process::Command, sync::Mutex};

use serde_json::{Value, json};

static LOCK: Mutex<()> = Mutex::new(());

fn ledger() -> PathBuf {
    #[cfg(test)]
    return crate::paths::test_dir("procs").join("processes.json");
    #[allow(unreachable_code)]
    crate::paths::PATHS.work.join("processes.json")
}

/// What `ps` says about a process: (process group, start time, command line). None when it is gone.
pub fn describe(pid: u32) -> Option<(u32, String, String)> {
    if cfg!(windows) || pid <= 1 {
        return None;
    }
    let out = Command::new("ps").args(["-o", "pgid=", "-o", "lstart=", "-o", "command=", "-p", &pid.to_string()]).env("LC_ALL", "C").output().ok()?;
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

/// One line of `ps -o pgid= -o lstart= -o command=`: the group, the start time (always five words, as
/// "Sun Nov 22 04:13:36 2026") and the command line.
fn parse_ps(text: &str) -> Option<(u32, String, String)> {
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut words = line.split_whitespace();
    let pgid: u32 = words.next()?.parse().ok()?;
    let started: Vec<&str> = words.by_ref().take(5).collect();
    if started.len() < 5 {
        return None;
    }
    let command = words.collect::<Vec<_>>().join(" ");
    (!command.is_empty()).then(|| (pgid, started.join(" "), command))
}

fn read_in(file: &Path) -> Vec<Value> {
    fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

fn write_in(file: &Path, list: &[Value]) {
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(file, serde_json::to_string_pretty(&Value::Array(list.to_vec())).unwrap_or_default());
}

/// Note a process the app started (by `owner`, such as "opencode"), once it is running.
pub fn record(owner: &str, pid: u32) {
    let Some((pgid, started, command)) = describe(pid) else { return };
    let app = std::process::id();
    let (app_started, app_command) = describe(app).map(|(_, s, c)| (s, c)).unwrap_or_default();
    let _l = LOCK.lock().unwrap();
    let file = ledger();
    let mut list = read_in(&file);
    list.retain(|e| e["pid"].as_u64() != Some(pid as u64));
    list.push(json!({"owner": owner, "pid": pid, "pgid": pgid, "started": started, "command": command,
                     "app": {"pid": app, "started": app_started, "command": app_command}}));
    write_in(&file, &list);
}

/// Drop a process from the ledger once the app has ended it.
pub fn forget(pid: u32) {
    let _l = LOCK.lock().unwrap();
    let file = ledger();
    let mut list = read_in(&file);
    let before = list.len();
    list.retain(|e| e["pid"].as_u64() != Some(pid as u64));
    if list.len() != before {
        write_in(&file, &list);
    }
}

/// Whether a ledger entry is still exactly the process it was: same pid, start time and command line.
fn same(entry: &Value, now: Option<&(u32, String, String)>) -> bool {
    match now {
        Some((_, started, command)) => entry["started"].as_str() == Some(started.as_str()) && entry["command"].as_str() == Some(command.as_str()),
        None => false,
    }
}

/// At start-up: end every recorded process still running exactly as recorded (with its process group) whose app is
/// gone, and keep only the entries of a copy of the app still running. Returns how many were ended.
pub fn reap_orphans() -> usize {
    let _l = LOCK.lock().unwrap();
    let file = ledger();
    let list = read_in(&file);
    if list.is_empty() {
        return 0;
    }
    let mut groups: Vec<u32> = Vec::new();
    let mut kept: Vec<Value> = Vec::new();
    for e in &list {
        let Some(pid) = e["pid"].as_u64().map(|p| p as u32) else { continue };
        let now = describe(pid);
        if !same(e, now.as_ref()) {
            continue; // gone, or its pid is someone else's now
        }
        let app = e["app"]["pid"].as_u64().map(|p| p as u32).unwrap_or(0);
        if app != std::process::id() && same(&e["app"], describe(app).as_ref()) {
            kept.push(e.clone()); // its app still runs: that app's to end
            continue;
        }
        {
            // its own group when it was started in one, else the process alone
            let pgid = e["pgid"].as_u64().map(|g| g as u32).unwrap_or(0);
            groups.push(if pgid == pid { pgid } else { pid });
        }
    }
    let ended = groups.len();
    crate::paths::end_trees(&groups);
    write_in(&file, &kept);
    ended
}

/// Every orphaned process (its parent gone) whose command line starts with `prefix` (an executable inside one of the
/// app's own folders, so it can only be the app's), as (pid, process group).
pub fn orphans_under(prefix: &str) -> Vec<(u32, u32)> {
    if cfg!(windows) || prefix.len() < 8 {
        return Vec::new();
    }
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=", "-o", "pgid=", "-o", "ppid=", "-o", "command="]).env("LC_ALL", "C").output() else { return Vec::new() };
    under(&String::from_utf8_lossy(&out.stdout), prefix)
}

fn under(text: &str, prefix: &str) -> Vec<(u32, u32)> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (pid, pgid, ppid): (u32, u32, u32) = (w.next()?.parse().ok()?, w.next()?.parse().ok()?, w.next()?.parse().ok()?);
            let command = w.collect::<Vec<_>>().join(" ");
            (ppid == 1 && command.starts_with(prefix)).then_some((pid, pgid))
        })
        .filter(|&(pid, _)| pid != std::process::id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_lines_are_read_with_their_five_word_start_time() {
        let (g, s, c) = parse_ps("48292 Sun Nov 22 04:13:36 2026     /Users/a/.opencode/bin/opencode serve --hostname 127.0.0.1 --port 0\n").unwrap();
        assert_eq!((g, s.as_str()), (48292, "Sun Nov 22 04:13:36 2026"));
        assert_eq!(c, "/Users/a/.opencode/bin/opencode serve --hostname 127.0.0.1 --port 0");
        assert!(parse_ps("").is_none() && parse_ps("12 Sun Nov 22").is_none());
    }

    #[test]
    fn only_an_exact_match_is_the_same_process() {
        let e = json!({"pid": 5, "started": "Sun Nov 22 04:13:36 2026", "command": "opencode serve"});
        assert!(same(&e, Some(&(5, "Sun Nov 22 04:13:36 2026".into(), "opencode serve".into()))));
        assert!(!same(&e, Some(&(5, "Sun Nov 22 04:13:37 2026".into(), "opencode serve".into()))), "a reused pid starts at another time");
        assert!(!same(&e, Some(&(5, "Sun Nov 22 04:13:36 2026".into(), "vim".into()))));
        assert!(!same(&e, None));
    }

    #[test]
    fn processes_under_a_folder_are_found_by_their_command() {
        let ps = "  101   101     1 /store/cache/opencode/bin/opencode-pty/0.6/opencode-pty daemon --runtime-dir /store/state\n  103   103   900 /store/cache/opencode/bin/opencode-pty/0.6/opencode-pty daemon\n  102   7     1 /usr/bin/opencode-pty daemon\n  bad line\n";
        assert_eq!(under(ps, "/store/cache/opencode/bin/opencode-pty/"), [(101, 101)], "only an orphan: one whose server still runs is the server's");
    }

    #[test]
    fn a_recorded_process_is_ended_at_start_up_and_a_changed_one_is_left() {
        let file = ledger();
        let _ = fs::remove_file(&file);
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        crate::paths::own_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        record("test", child.id());
        let list = read_in(&file);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["command"], "sleep 30");
        // an entry whose start time no longer matches is someone else's process now: left alone
        let mut other = Command::new("sleep");
        other.arg("31");
        let mut other = other.spawn().unwrap();
        let mut list = read_in(&file);
        list.push(json!({"owner": "test", "pid": other.id(), "pgid": other.id(), "started": "Mon Jan  1 00:00:00 2001", "command": "sleep 31"}));
        write_in(&file, &list);
        assert_eq!(reap_orphans(), 1);
        assert!(child.wait().is_ok());
        assert!(other.try_wait().unwrap().is_none(), "not an exact match: still running");
        let _ = other.kill();
        let _ = other.wait();
        assert!(read_in(&file).is_empty());
        forget(1);
    }
}
