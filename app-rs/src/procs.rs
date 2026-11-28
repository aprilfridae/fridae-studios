//! A small ledger of the processes the app starts for the AIs (the per-turn command-line tools, OpenCode's server,
//! sign-ins), so one left behind by an app that crashed or was killed is ended the next time the app starts.
//!
//! Each copy of the app keeps its own file, `work/processes/<app pid>.json` (so two copies sharing a work folder never
//! drop each other's entries): the app's own pid, start time and command line, and per process its pid, its process
//! group, its start time and its command line as `ps` reports them. At start-up the files of apps that are gone are
//! read: an entry is acted on only when a process with that pid is still there with exactly the same start time and
//! command line (a pid the system has since given to anything else is never touched); this run's own file, and the
//! file of another copy of the app still running, are left alone. Unix only; on Windows nothing is recorded (the
//! processes there are ended with `taskkill /T` as before).

use crate::paths::Start;
use std::{fs, path::Path, path::PathBuf, process::Command, sync::Mutex};

use serde_json::{Value, json};

static LOCK: Mutex<()> = Mutex::new(());

fn folder() -> PathBuf {
    #[cfg(test)]
    return crate::paths::test_dir("procs");
    #[allow(unreachable_code)]
    crate::paths::PATHS.work.join("processes")
}

/// This run's file, in a ledger folder.
fn mine_in(dir: &std::path::Path) -> PathBuf {
    dir.join(format!("{}.json", std::process::id()))
}

/// What `ps` says about a process: (process group, start time, command line). None when it is gone.
pub fn describe(pid: u32) -> Option<(u32, String, String)> {
    if cfg!(windows) || pid <= 1 {
        return None;
    }
    let out = Command::new("ps").args(["-o", "pgid=", "-o", "lstart=", "-o", "command=", "-p", &pid.to_string()]).env("LC_ALL", "C").run_output().ok()?;
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

/// A ledger file: {"app": {pid, started, command}, "processes": [...]}.
fn read_in(file: &Path) -> Value {
    fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({"processes": []}))
}

fn write_in(file: &Path, v: &Value) {
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = file.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_string_pretty(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, file);
    }
}

/// This app's own pid, start time and command line (read once).
fn me() -> Value {
    static ME: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    ME.get_or_init(|| {
        let pid = std::process::id();
        let (started, command) = describe(pid).map(|(_, s, c)| (s, c)).unwrap_or_default();
        json!({"pid": pid, "started": started, "command": command})
    })
    .clone()
}

/// Note a process the app started (by `owner`, such as "opencode" or "claude"), once it is running.
pub fn record(owner: &str, pid: u32) {
    record_in(&folder(), owner, pid)
}

/// The same, in a ledger folder (the app has one; a test of the ledger itself uses its own).
fn record_in(dir: &std::path::Path, owner: &str, pid: u32) {
    if cfg!(windows) {
        return;
    }
    let Some((pgid, started, command)) = describe(pid) else { return };
    let app = me();
    // an app that cannot describe itself writes no ledger: another copy could not tell it from a crashed run
    if app["started"].as_str().is_none_or(str::is_empty) {
        return;
    }
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = mine_in(dir);
    let mut v = read_in(&file);
    v["app"] = app;
    let mut list = v["processes"].as_array().cloned().unwrap_or_default();
    list.retain(|e| e["pid"].as_u64() != Some(pid as u64));
    list.push(json!({"owner": owner, "pid": pid, "pgid": pgid, "started": started, "command": command}));
    v["processes"] = Value::Array(list);
    write_in(&file, &v);
}

/// Whether a process is in this run's ledger (for tests of what must be noted there, and taken out again).
#[cfg(test)]
pub fn recorded(pid: u32) -> bool {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_in(&mine_in(&folder()))["processes"].as_array().is_some_and(|l| l.iter().any(|e| e["pid"].as_u64() == Some(pid as u64)))
}

/// Drop a process from the ledger once it has ended (or the app has ended it).
pub fn forget(pid: u32) {
    forget_in(&folder(), pid)
}

fn forget_in(dir: &std::path::Path, pid: u32) {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = mine_in(dir);
    let mut v = read_in(&file);
    let Some(list) = v["processes"].as_array_mut() else { return };
    let before = list.len();
    list.retain(|e| e["pid"].as_u64() != Some(pid as u64));
    if list.len() != before {
        if list.is_empty() {
            let _ = fs::remove_file(&file);
        } else {
            write_in(&file, &v);
        }
    }
}

/// Whether a ledger entry is still exactly the process it was: same pid, start time and command line.
fn same(entry: &Value, now: Option<&(u32, String, String)>) -> bool {
    match now {
        Some((_, started, command)) => entry["started"].as_str() == Some(started.as_str()) && entry["command"].as_str() == Some(command.as_str()),
        None => false,
    }
}

/// At start-up: for every ledger file of an app that is gone (not this run, not another copy still running), end
/// each recorded process still running exactly as recorded (with its process group), and remove the file. Returns
/// how many were ended.
pub fn reap_orphans() -> usize {
    reap_in(&folder())
}

fn reap_in(dir: &std::path::Path) -> usize {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut groups: Vec<u32> = Vec::new();
    #[cfg_attr(test, allow(unused_mut))]
    let mut files: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    // the single ledger file of earlier versions
    #[cfg(not(test))]
    files.push(crate::paths::PATHS.work.join("processes.json"));
    let here = me();
    for file in files {
        let raw = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(raw) = raw else { continue };
        // earlier versions: a plain list, each entry with its own app
        let (app, list) = match &raw {
            Value::Array(l) => (l.first().map(|e| e["app"].clone()).unwrap_or(Value::Null), l.clone()),
            v => (v["app"].clone(), v["processes"].as_array().cloned().unwrap_or_default()),
        };
        let app_pid = app["pid"].as_u64().map(|p| p as u32).unwrap_or(0);
        if app["started"].as_str().is_none_or(str::is_empty) {
            continue; // not told apart from a running copy: left alone
        }
        let this_run = app_pid == std::process::id() && app["started"] == here["started"];
        if this_run || (app_pid != std::process::id() && same(&app, describe(app_pid).as_ref())) {
            continue; // this run's own, or a copy of the app still running: that app's to end
        }
        for e in &list {
            let Some(pid) = e["pid"].as_u64().map(|p| p as u32) else { continue };
            if !same(e, describe(pid).as_ref()) {
                continue; // gone, or its pid is someone else's now
            }
            // its own group (with the groups of what runs below it: a tool's shell may be in one of its own). Every
            // process the app records is started in a group of its own; one that is not is ended alone
            let pgid = e["pgid"].as_u64().map(|g| g as u32).unwrap_or(0);
            if pgid == pid {
                for g in tree_groups(pid) {
                    if !groups.contains(&g) {
                        groups.push(g);
                    }
                }
            } else {
                let _ = std::process::Command::new("kill").arg(pid.to_string()).stderr(std::process::Stdio::null()).run_status();
            }
        }
        let _ = fs::remove_file(&file);
    }
    let ended = groups.len();
    crate::paths::end_trees(&groups);
    ended
}

/// The process groups of `leader`'s whole tree: its own, and that of each process below it (Cursor runs each shell
/// command in a group of its own, so a signal to the turn's group alone leaves it running). Never the app's own
/// group. Unix; on Windows just the leader (taskkill /T follows the tree there).
pub fn tree_groups(leader: u32) -> Vec<u32> {
    if cfg!(windows) {
        return vec![leader];
    }
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=", "-o", "pgid=", "-o", "ppid="]).env("LC_ALL", "C").run_output() else { return vec![leader] };
    groups_below(&String::from_utf8_lossy(&out.stdout), leader, own_group())
}

/// The app's own process group.
fn own_group() -> u32 {
    describe(std::process::id()).map(|(g, _, _)| g).unwrap_or(0)
}

fn groups_below(text: &str, leader: u32, app_group: u32) -> Vec<u32> {
    let rows: Vec<(u32, u32, u32)> = text
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?, w.next()?.parse().ok()?))
        })
        .collect();
    let mut below = vec![leader];
    let mut i = 0;
    while i < below.len() {
        let parent = below[i];
        for &(pid, _, ppid) in &rows {
            if ppid == parent && !below.contains(&pid) {
                below.push(pid);
            }
        }
        i += 1;
    }
    let mut groups = vec![leader];
    for &(pid, pgid, _) in &rows {
        if below.contains(&pid) && pgid > 1 && pgid != app_group && !groups.contains(&pgid) {
            groups.push(pgid);
        }
    }
    groups
}

/// Every orphaned process (its parent gone) whose command line starts with `prefix` (an executable inside one of the
/// app's own folders, so it can only be the app's), as (pid, process group).
pub fn orphans_under(prefix: &str) -> Vec<(u32, u32)> {
    if cfg!(windows) || prefix.len() < 8 {
        return Vec::new();
    }
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=", "-o", "pgid=", "-o", "ppid=", "-o", "command="]).env("LC_ALL", "C").run_output() else { return Vec::new() };
    under(&String::from_utf8_lossy(&out.stdout), prefix)
}

/// An orphan's parent is pid 1, or (Linux desktops) the user's `systemd --user`, which takes in the orphans of the
/// session's programs instead of pid 1.
fn under(text: &str, prefix: &str) -> Vec<(u32, u32)> {
    let rows: Vec<(u32, u32, u32, String)> = text
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (pid, pgid, ppid): (u32, u32, u32) = (w.next()?.parse().ok()?, w.next()?.parse().ok()?, w.next()?.parse().ok()?);
            Some((pid, pgid, ppid, w.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    let reapers: Vec<u32> = rows.iter().filter(|(_, _, _, c)| is_user_systemd(c)).map(|r| r.0).chain([1]).collect();
    rows.iter()
        .filter(|(pid, _, ppid, command)| reapers.contains(ppid) && command.starts_with(prefix) && *pid != std::process::id())
        .map(|&(pid, pgid, _, _)| (pid, pgid))
        .collect()
}

/// `/usr/lib/systemd/systemd --user` (or `/lib/systemd/systemd --user`), as `ps` shows the user's service manager.
fn is_user_systemd(command: &str) -> bool {
    let mut w = command.split_whitespace();
    w.next().is_some_and(|p| p.rsplit('/').next() == Some("systemd")) && w.any(|a| a == "--user")
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
    fn a_turns_tree_includes_the_groups_its_shells_were_put_in() {
        // 10 the turn (its own group), 11 its helper in the group, 12 a shell Cursor put in group 12 with 13 under it,
        // 14 something in the app's group 1000 (signalled as part of 10's group never), 20 unrelated
        let ps = "  10 10 1000\n  11 10 10\n  12 12 10\n  13 12 12\n  14 1000 11\n  20 20 1\n  bad\n";
        assert_eq!(groups_below(ps, 10, 1000), [10, 12]);
        assert_eq!(groups_below("", 10, 1000), [10], "the turn's own group at least");
    }

    #[test]
    fn processes_under_a_folder_are_found_by_their_command() {
        let ps = "  101   101     1 /store/cache/opencode/bin/opencode-pty/0.6/opencode-pty daemon --runtime-dir /store/state\n  103   103   900 /store/cache/opencode/bin/opencode-pty/0.6/opencode-pty daemon\n  102   7     1 /usr/bin/opencode-pty daemon\n  bad line\n";
        assert_eq!(under(ps, "/store/cache/opencode/bin/opencode-pty/"), [(101, 101)], "only an orphan: one whose server still runs is the server's");
    }

    #[test]
    #[cfg_attr(windows, ignore = "the process ledger is kept on Unix only; Windows ends a turn's tree with taskkill /T (DEVELOPING.md)")]
    fn a_crashed_runs_processes_are_ended_at_start_up_and_this_runs_are_kept() {
        // a ledger folder of its own: every test that starts an AI records into the one the tests share
        let dir = crate::paths::test_dir("procs-ledger-test");
        let _ = fs::remove_dir_all(&dir);
        let spawn = |secs: &str| {
            let mut cmd = Command::new("sleep");
            cmd.arg(secs);
            crate::paths::own_group(&mut cmd);
            cmd.start().unwrap()
        };
        // this run's own process: recorded in its file, and left alone by the reap (M3: a turn may already be running)
        let mut own = spawn("30");
        record_in(&dir, "test", own.id());
        assert_eq!(read_in(&mine_in(&dir))["processes"][0]["command"], "sleep 30");
        // a crashed run's file: an app pid that is gone, a process still running exactly as recorded, and one whose
        // start time no longer matches (someone else's process now)
        let mut left = spawn("32");
        let mut other = spawn("31");
        let (g, started, command) = describe(left.id()).unwrap();
        let crashed = json!({"app": {"pid": 999_999, "started": "Mon Jan  1 00:00:00 2001", "command": "fridae"}, "processes": [
            {"owner": "test", "pid": left.id(), "pgid": g, "started": started, "command": command},
            {"owner": "test", "pid": other.id(), "pgid": other.id(), "started": "Mon Jan  1 00:00:00 2001", "command": "sleep 31"}]});
        write_in(&dir.join("999999.json"), &crashed);
        assert_eq!(reap_in(&dir), 1);
        assert!(left.wait().is_ok());
        assert!(!dir.join("999999.json").exists(), "the crashed run's file is gone");
        assert!(other.try_wait().unwrap().is_none(), "not an exact match: still running");
        assert!(own.try_wait().unwrap().is_none(), "this run's own process is never reaped");
        assert_eq!(read_in(&mine_in(&dir))["processes"].as_array().unwrap().len(), 1);
        for c in [&mut other, &mut own] {
            let _ = c.kill();
            let _ = c.wait();
        }
        forget_in(&dir, own.id());
        assert!(!mine_in(&dir).exists(), "an empty ledger leaves no file");
    }

    #[test]
    fn an_orphan_taken_in_by_the_users_systemd_is_still_found() {
        let ps = "  900   900     1 /usr/lib/systemd/systemd --user\n\
                   4001  4001   900 /home/a/Documents/Fridae Studios/work/opencode serve\n\
                   4002  4002     1 /home/a/Documents/Fridae Studios/work/opencode serve\n\
                   4003  4003  3000 /home/a/Documents/Fridae Studios/work/opencode serve\n\
                   4004  4004   900 /usr/bin/gnome-text-editor\n";
        assert_eq!(under(ps, "/home/a/Documents/Fridae Studios/work/"), [(4001, 4001), (4002, 4002)], "a parent still running keeps its child");
        assert!(is_user_systemd("/lib/systemd/systemd --user") && !is_user_systemd("/usr/lib/systemd/systemd-logind") && !is_user_systemd("/sbin/init splash"));
    }
}
