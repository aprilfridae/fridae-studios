//! Studio mode: a checkpoint at the start of a run and after each finished stage, and "Undo this run" back to any of
//! them. A checkpoint is the one Overnight takes when a night begins (Agent::undo_night uses the same `put_back`):
//!
//!   - the project's files: a git commit of the project folder, when it is a git repository;
//!   - Studio's scripts: a snapshot the Fridae plugin keeps in Studio's memory, pinned at the checkpoint's time (the
//!     plugin keeps the newest 24 pins); the agents' Studio steps since then are undone by time, whoever made them.
//!     Before an undo puts scripts back, their current text is pinned too ("before undo").
//!
//! What undo needs is in `<project>/.fridae/studio.json` "checkpoints": [{"stage", "commit", "studio", "at", "git"?}]
//! ("git": "failed" when the folder is a repository but the commit failed: such a checkpoint is refused) ("start"
//! for the run's first), so it works after the app restarts. Studio's part needs Studio to have stayed open on the same
//! place (its undo history and the plugin's snapshots live there).
//!
//! A finished stage asks for its checkpoint through a request file, `WORK/stage-requests/<id>.json`
//! {"project", "stage", "lead"} (rbxstage.py); `watch` takes it and appends it.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    paths, plugin, projects,
    studio_run::{self, STAGES, change_run, item_files, read_obj, read_run, running, write_json},
};

pub const NO_RUN: &str = "This game has no studio run to undo.";
pub const NO_CHECKPOINTS: &str = "This run has no checkpoints to go back to yet.";
pub const NO_SUCH_CHECKPOINT: &str = "This run has no checkpoint for that stage.";
const BUSY: &str = "Pause the run and let the experts finish, then undo.";
const STUDIO_CLOSED: &str = "Open the game in Roblox Studio, then undo \u{2014} or undo the files only.";
const ALREADY_UNDOING: &str = "An undo is already running.";
const GIT_FAILED: &str = "Git could not save the project's files at this checkpoint, so the run cannot go back to it. Pick an earlier checkpoint.";
const MISSING_COMMIT: &str = "The checkpoint's commit is not in the project's git history any more, so nothing was undone.";
/// What `.fridae/.gitignore` keeps out of the checkpoint commits: review pictures and clips (the item files stay
/// tracked), the archive, and the lock.
const GITIGNORE_LINES: [&str; 3] = ["review/*/", "review-archive/", ".lock"];

/// A checkpoint to go back to, and how the report names it.
pub struct Checkpoint<'a> {
    /// When it was taken (seconds since 1970): Studio steps from then on are undone, and scripts go back to the
    /// snapshot pinned then.
    pub at: f64,
    /// Studio was connected, so the plugin pinned a snapshot.
    pub studio: bool,
    pub project: Option<&'a str>,
    pub commit: Option<&'a str>,
    /// "the night began": the moment, for the report.
    pub began: &'a str,
    /// The words for the project's restore commits; None for the night's.
    pub undoing: Option<&'a projects::Undoing<'a>>,
    /// Studio is left alone, with this line in the report (files only, or nothing there to undo).
    pub skip_studio: Option<&'a str>,
}

pub struct PutBack {
    pub studio_ok: bool,
    pub project_ok: bool,
    pub lines: Vec<String>,
}

/// Put Studio and the project's files back as they were at a checkpoint. A checkpoint commit missing from git stops
/// it before anything is touched; otherwise each part is tried whatever the other did. Before Studio's scripts go
/// back, their text as it is now is pinned in Studio as "before undo", so it is not lost.
pub fn put_back(cp: &Checkpoint) -> PutBack {
    let mut out = PutBack { studio_ok: true, project_ok: true, lines: Vec::new() };
    if let (Some(p), Some(c)) = (cp.project, cp.commit) {
        if !projects::commit_exists(p, c) {
            out.project_ok = false;
            out.lines.push(format!("Project: {MISSING_COMMIT} Studio was left as it is too."));
            return out;
        }
    }
    if let Some(line) = cp.skip_studio {
        out.lines.push(line.to_string());
    } else if cp.at > 0.0 && plugin::connected() {
        // an older plugin keeps one pin only: pinning now would replace the checkpoint's own snapshot
        let current = plugin::running_current();
        if current {
            plugin::pin(paths::now_secs(), "before undo");
        }
        let r = plugin::revert_within(cp.at, 600);
        out.studio_ok = r["ok"] == true;
        out.lines.push(format!("Studio: {}", r["text"].as_str().unwrap_or("")));
        if current && out.studio_ok {
            out.lines.push("Studio: the scripts as they were just before this undo are kept in Studio as a snapshot named \"before undo\" until Studio closes.".to_string());
        }
        if cp.undoing.is_some() && !current {
            out.lines.push("Studio: its Fridae plugin is an older copy that keeps scripts only from the latest checkpoint, so scripts may not have gone all the way back. Update the plugin in Setup and restart Studio.".to_string());
        }
    } else if cp.studio {
        out.studio_ok = false;
        out.lines.push("Studio: not connected. Open the same place in Studio (without restarting it if you can, since a restart clears its undo history), then try again.".to_string());
    } else {
        out.lines.push(format!("Studio: it was not connected when {}, so there was nothing there to undo.", cp.began));
    }
    match (cp.project, cp.commit) {
        (Some(p), Some(c)) => match cp.undoing.map_or_else(|| projects::restore_to(p, c), |w| projects::restore_with(p, c, w)) {
            Ok(t) => out.lines.push(format!("Project: {t}")),
            Err(e) => {
                out.project_ok = false;
                out.lines.push(format!("Project: {e}"));
            }
        },
        (Some(_), None) => out.lines.push("Project: its folder has no git repository, so its files could not be checkpointed and were left as they are.".to_string()),
        _ => {}
    }
    out
}

/// Keep review media, the archive and the lock out of the checkpoint commits: `<project>/.fridae/.gitignore` gets the
/// lines it is missing (a user's own lines are kept; the project's top-level .gitignore is never touched).
pub(crate) fn ensure_gitignore(project: &Path) {
    let file = project.join(".fridae").join(".gitignore");
    let have = fs::read_to_string(&file).unwrap_or_default();
    let lines: Vec<&str> = have.lines().map(str::trim).collect();
    let missing: Vec<&str> = GITIGNORE_LINES.iter().copied().filter(|l| !lines.contains(l)).collect();
    if missing.is_empty() || fs::create_dir_all(project.join(".fridae")).is_err() {
        return;
    }
    let mut text = have.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for l in missing {
        text += l;
        text.push('\n');
    }
    let _ = crate::promo::write_atomic(&file, text.as_bytes());
}

/// What a checkpoint saved: the commit (None without git, or when git failed: `git_failed`), whether Studio was
/// connected (so the plugin pinned the scripts), and when.
pub struct Saved {
    pub commit: Option<String>,
    pub git_failed: bool,
    pub studio: bool,
    pub at: f64,
}

fn checkpoint_entry(stage: &str, saved: &Saved) -> Value {
    let mut e = json!({"stage": stage, "commit": saved.commit, "studio": saved.studio, "at": saved.at});
    if saved.git_failed {
        e["git"] = json!("failed");
    }
    e
}

/// Append a finished stage's checkpoint to a run that is on, only while that stage is still done (a request taken
/// after an undo put the stage back must not add one). Returns whether it was written.
fn add_checkpoint(project: &Path, stage: &str, saved: &Saved) -> Result<bool, String> {
    change_run(project, |run| {
        let done = run["stages"].as_array().is_some_and(|l| l.iter().any(|s| s["id"] == stage && s["status"] == "done"));
        if !run.is_object() || run["on"] != true || !done {
            return (false, false);
        }
        if !run["checkpoints"].is_array() {
            run["checkpoints"] = json!([]);
        }
        run["checkpoints"].as_array_mut().map(|list| list.push(checkpoint_entry(stage, saved)));
        (true, true)
    })
}

/// The run's first checkpoint, "start": written by its Lead's night checkpoint, only while the run has none (a
/// Lead adopting the run later, or a message to it, keeps the first).
fn record_start_in(project: &Path, lead: &str, saved: &Saved) -> Result<bool, String> {
    change_run(project, |run| {
        let empty = run["checkpoints"].as_array().is_none_or(|l| l.is_empty());
        if !run.is_object() || !running(run) || run["lead"] != lead || !empty {
            return (false, false);
        }
        run["checkpoints"] = json!([checkpoint_entry("start", saved)]);
        (true, true)
    })
}

pub fn record_start(project: &str, lead: &str, saved: &Saved) {
    let _ = record_start_in(Path::new(project), lead, saved);
}

/// The checkpoint to go back to: the newest one of `stage` ("start" or a stage id), or the newest of all.
fn pick_checkpoint(run: &Value, stage: Option<&str>) -> Result<(usize, Value), String> {
    let list = run["checkpoints"].as_array().filter(|l| !l.is_empty()).ok_or(NO_CHECKPOINTS)?;
    let i = match stage.map(str::trim).filter(|s| !s.is_empty()) {
        None => list.len() - 1,
        Some(s) => list.iter().rposition(|c| c["stage"] == s).ok_or(NO_SUCH_CHECKPOINT)?,
    };
    Ok((i, list[i].clone()))
}

/// The run as it was at checkpoint `idx`: stages finished after it are to do again (or under way, if they had begun
/// before it), stages begun after it are to do, later checkpoints go, and the run is paused. Returns the stages put back.
fn rewind(run: &mut Value, idx: usize) -> Vec<String> {
    let at = run["checkpoints"][idx]["at"].as_f64().unwrap_or(0.0);
    let mut undone = Vec::new();
    for s in run["stages"].as_array_mut().into_iter().flatten() {
        let (started, ended) = (s["started"].as_f64().unwrap_or(0.0), s["ended"].as_f64().unwrap_or(0.0));
        let back = match s["status"].as_str() {
            Some("done") if ended > at => Some(if started > 0.0 && started < at { "doing" } else { "todo" }),
            Some("doing") if started >= at => Some("todo"),
            _ => None,
        };
        if let Some(status) = back {
            s["status"] = json!(status);
            s["ended"] = json!(0);
            if status == "todo" {
                s["started"] = json!(0);
            }
            undone.push(s["id"].as_str().unwrap_or("").to_string());
        }
    }
    let stages = run["stages"].as_array().cloned().unwrap_or_default();
    let pick = |status: &str| stages.iter().rev().find(|s| s["status"] == status).and_then(|s| s["id"].as_str()).map(str::to_string);
    run["stage"] = json!(pick("doing").or_else(|| pick("done")).unwrap_or_default());
    if let Some(list) = run["checkpoints"].as_array_mut() {
        list.truncate(idx + 1);
    }
    run["paused"] = json!(true);
    undone
}

/// Move the review items filed after `at` (and their pictures) to `.fridae/review-archive/undo-<stamp>/`, so Review
/// does not show work that was undone. An item one of them replaced is open again ("changes"). Returns how many
/// moved, what could not be moved, and the folder. Call under the project's lock.
fn archive_after(project: &Path, at: f64, stamp: u64) -> (usize, Vec<String>, PathBuf) {
    let all: Vec<(PathBuf, Value)> = item_files(project).into_iter().filter_map(|(_, p)| read_obj(&p).map(|v| (p, v))).collect();
    let later: Vec<&(PathBuf, Value)> = all.iter().filter(|(_, i)| i["created"].as_f64().unwrap_or(0.0) > at).collect();
    let mut entries: Vec<PathBuf> = Vec::new();
    for (file, _) in &later {
        entries.push(file.clone());
        let folder = file.with_extension("");
        if folder.is_dir() {
            entries.push(folder);
        }
    }
    let (dest, failed) = studio_run::archive_entries(project, &format!("undo-{stamp}"), entries);
    let moved: Vec<&Value> = later.iter().filter(|(f, _)| !f.exists()).map(|(_, i)| i).collect();
    let replaced: Vec<&str> = moved.iter().filter_map(|i| i["replaces"].as_str()).filter(|r| !r.is_empty()).collect();
    for (file, item) in &all {
        if file.exists() && item["status"] == "superseded" && item["id"].as_str().is_some_and(|id| replaced.contains(&id)) {
            let mut item = item.clone();
            item["status"] = json!("changes");
            let _ = write_json(file, &item);
        }
    }
    (moved.len(), failed, dest)
}

/// The Lead or any helper working on this game: undo waits for them.
fn run_busy(project: &str, lead: &str) -> bool {
    agents::agents().iter().any(|a| a.busy() && (a.id == lead || (a.team().is_some() && a.project().is_some_and(|p| projects::same(&p, project)))))
}

/// How the report names a checkpoint.
fn checkpoint_name(stage: &str) -> String {
    if stage == "start" { "the start of the run".to_string() } else { format!("the end of the {stage} stage") }
}

fn refuse(why: &str) -> Value {
    json!({"ok": false, "error": why, "text": why})
}

/// Games with an undo going on in this app: a second one at the same time is refused.
static UNDOING: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Undoing(String);

impl Undoing {
    fn take(project: &str) -> Option<Undoing> {
        let mut busy = UNDOING.lock().unwrap();
        if busy.iter().any(|p| projects::same(p, project)) {
            return None;
        }
        busy.push(project.to_string());
        Some(Undoing(project.to_string()))
    }
}

impl Drop for Undoing {
    fn drop(&mut self) {
        UNDOING.lock().unwrap().retain(|p| p != &self.0);
    }
}

/// POST /api/studio-run {action: "undo", path, stage?, files_only?}: the game back to a checkpoint of its run (the
/// newest when no stage is given). Refused while the Lead or an expert is working, while another undo of the game
/// runs, and, when Studio is closed but the run used it since that checkpoint, unless `files_only` (Studio is then
/// left as it is). The run is left paused.
pub fn undo(project: &str, stage: Option<&str>, files_only: bool) -> Value {
    let Some(_one) = Undoing::take(project) else { return refuse(ALREADY_UNDOING) };
    let dir = Path::new(project);
    let Some(run) = read_run(dir).filter(|r| r["on"] == true) else { return refuse(NO_RUN) };
    let (idx, cp) = match pick_checkpoint(&run, stage) {
        Ok(found) => found,
        Err(e) => return refuse(&e),
    };
    if cp["git"] == "failed" {
        return refuse(GIT_FAILED);
    }
    if cp["commit"].as_str().is_some_and(|c| !projects::commit_exists(project, c)) {
        return refuse(MISSING_COMMIT);
    }
    let lead = run["lead"].as_str().unwrap_or("").to_string();
    if run_busy(project, &lead) {
        return refuse(BUSY);
    }
    // Studio closed: its part is never reported as done. Refused when the run had Studio open since this checkpoint.
    let connected = plugin::connected();
    let used_studio = run["checkpoints"].as_array().is_some_and(|l| l[idx..].iter().any(|c| c["studio"] == true));
    if !files_only && !connected && used_studio {
        return json!({"ok": false, "error": STUDIO_CLOSED, "text": STUDIO_CLOSED, "files_only": true});
    }
    let skip_studio = if files_only {
        Some("Studio: left as it is (you chose to undo the files only).")
    } else if !connected {
        Some("Studio: not connected now, and it was not connected at this checkpoint or any later one, so it was left as it is.")
    } else {
        None
    };
    // the Lead's next carry-on is dropped (the flag Stop uses), and the run paused, before anything goes back
    let live = agents::get(&lead);
    if let Some(l) = &live {
        l.halt();
    }
    let _ = change_run(dir, |r| {
        let go = r.is_object() && r["paused"] != true;
        if go {
            r["paused"] = json!(true);
        }
        (go, ())
    });
    let at = cp["at"].as_f64().unwrap_or(0.0);
    let name = checkpoint_name(cp["stage"].as_str().unwrap_or(""));
    let log = live.as_ref().map(|l| l.run_log()).unwrap_or_else(|| agents::saved_run_log(&lead));
    let since: Vec<&Value> = log.iter().filter(|s| s["t"].as_f64().unwrap_or(0.0) >= at).collect();
    let undo_commit = format!("Undo the studio run to {name} (Fridae Studios)");
    let words = projects::Undoing { whose: "run's", end_commit: "Before undoing the studio run (kept in history)", undo_commit: &undo_commit };
    let back = put_back(&Checkpoint {
        at,
        studio: cp["studio"] == true,
        project: Some(project),
        commit: cp["commit"].as_str(),
        began: "the checkpoint was taken",
        undoing: Some(&words),
        skip_studio,
    });
    let mut lines = vec![format!("Back to {name}{}.", if files_only { " (the files only)" } else { "" })];
    if !since.is_empty() {
        let mut who: Vec<&str> = Vec::new();
        for s in &since {
            let by = s["by"].as_str().unwrap_or("");
            if !who.contains(&by) {
                who.push(by);
            }
        }
        lines.push(format!("In Studio since then: {} step{} by {}.", since.len(), if since.len() == 1 { "" } else { "s" }, who.join(", ")));
    }
    lines.extend(back.lines);
    let mut ok = back.studio_ok && back.project_ok;
    // the run's own record goes back only when the files did, so a failed try can be repeated from the same place
    if back.project_ok {
        let stamp = paths::now_secs() as u64;
        let changed = change_run(dir, |r| {
            if !r.is_object() {
                return (false, None);
            }
            let undone = rewind(r, idx);
            (true, Some((undone, archive_after(dir, at, stamp))))
        });
        match changed {
            Ok(Some((undone, (moved, failed, dest)))) => {
                lines.push(if undone.is_empty() { "Run: no stage was finished after that checkpoint.".to_string() } else { format!("Run: to do again: {}.", undone.join(", ")) });
                if moved > 0 {
                    let rel = dest.strip_prefix(dir).map(|p| p.display().to_string()).unwrap_or_else(|_| dest.display().to_string());
                    lines.push(if moved == 1 { format!("Review: 1 item filed since then was moved to {rel}.") } else { format!("Review: {moved} items filed since then were moved to {rel}.") });
                }
                if !failed.is_empty() {
                    ok = false;
                    lines.push(format!("Review: could not move these into the archive: {}.", failed.join(", ")));
                }
            }
            Ok(None) => lines.push("Run: its file is gone, so its stages were left as they are.".to_string()),
            Err(e) => {
                ok = false;
                lines.push(format!("Run: {e}"));
            }
        }
    }
    lines.push("The run is paused. Press Resume in Review when it should carry on.".to_string());
    lines.push("The board's cards were kept so you can still read them.".to_string());
    let text = lines.join("\n");
    if let Some(l) = &live {
        if ok {
            let what = if skip_studio.is_some() { "the game's files were put back (Studio was left as it is)" } else { "the game's files and Studio were put back" };
            l.note(&format!("The user undid this studio run back to {name}: {what}, later stages are to do again, and review items filed since then were archived. Call rbx_stage status before going on."));
            l.put("run_undone", json!({"text": text}));
        } else {
            l.put("error", json!({"text": format!("Undo this run did not finish.\n{text}")}));
        }
    }
    if ok { json!({"ok": true, "text": text}) } else { json!({"ok": false, "text": text, "error": text}) }
}

/// Take a checkpoint now: `.fridae/.gitignore` made sure of, Studio's scripts pinned, the project's files committed.
pub fn take(project: &str, message: &str) -> Saved {
    ensure_gitignore(Path::new(project));
    let at = paths::now_secs();
    plugin::pin(at, "checkpoint");
    let studio = plugin::connected();
    let repo = projects::is_repo(project);
    let commit = projects::checkpoint(project, message);
    Saved { git_failed: repo && commit.is_none(), commit, studio, at }
}

/// Whether `id` leads the game's run and the run is paused.
pub fn paused_lead(project: &str, id: &str) -> bool {
    read_run(Path::new(project)).is_some_and(|r| running(&r) && r["paused"] == true && r["lead"] == id)
}

/// Stop pressed on a studio run's Lead: the run is paused (Review shows Paused, not Working).
pub fn pause_if_lead(agent: &Agent) {
    let Some(project) = agent.project() else { return };
    let dir = Path::new(&project);
    if !studio_run::run_file(dir).is_file() {
        return;
    }
    let _ = change_run(dir, |r| {
        let go = r.is_object() && running(r) && r["lead"] == agent.id.as_str() && r["paused"] != true;
        if go {
            r["paused"] = json!(true);
        }
        (go, ())
    });
}

/// Take a finished stage's checkpoint and append it (only while the stage is still done).
fn take_stage_checkpoint(req: &Value) {
    let Some(project) = studio_run::project_of(req["project"].as_str().unwrap_or("")) else { return };
    let stage = req["stage"].as_str().unwrap_or("");
    let dir = Path::new(&project);
    if !STAGES.contains(&stage) || !read_run(dir).is_some_and(|r| r["on"] == true) {
        return;
    }
    let saved = take(&project, &format!("After the {stage} stage of the studio run (Fridae Studios checkpoint)"));
    let _ = add_checkpoint(dir, stage, &saved);
}

/// Requests left from when the app was closed came too late to be their stage's checkpoint: cleared once, then watched.
pub fn watch() {
    let dir = paths::work("stage-requests");
    let _ = fs::remove_dir_all(&dir);
    loop {
        thread::sleep(Duration::from_millis(700));
        let mut found: Vec<PathBuf> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        found.sort(); // oldest first: the ids start with the time
        for path in found {
            let req = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let _ = fs::remove_file(&path);
            if let Some(req) = req {
                take_stage_checkpoint(&req);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio_run::{STAGES, change_run, read_run, run_file};
    use serde_json::json;
    use std::{fs, path::PathBuf};

    fn game(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fridae-studio-undo-{name}-{}", std::process::id())).join("My Gäme é");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".fridae").join("review")).unwrap();
        d
    }

    fn stages(done: &[(&str, f64, f64)]) -> Vec<serde_json::Value> {
        STAGES
            .iter()
            .map(|s| match done.iter().find(|d| d.0 == *s) {
                Some((_, start, end)) => json!({"id": s, "status": if *end > 0.0 { "done" } else { "doing" }, "started": start, "ended": end}),
                None => json!({"id": s, "status": "todo", "started": 0, "ended": 0}),
            })
            .collect()
    }

    fn saved(commit: Option<&str>, studio: bool, at: f64) -> Saved {
        Saved { commit: commit.map(str::to_string), git_failed: false, studio, at }
    }

    #[test]
    fn checkpoint_appended_per_stage() {
        let d = game("append");
        // no run, or a run that is off: nothing is written
        assert!(!add_checkpoint(&d, "build", &saved(Some("abc1234"), false, 10.0)).unwrap());
        assert!(!run_file(&d).exists());
        fs::write(run_file(&d), json!({"on": false}).to_string()).unwrap();
        assert!(!add_checkpoint(&d, "build", &saved(None, false, 10.0)).unwrap());
        fs::write(run_file(&d), json!({"on": true, "lead": "c1", "stages": stages(&[("design", 1.0, 2.0), ("build", 3.0, 4.0)]), "future": 1}).to_string()).unwrap();
        assert!(add_checkpoint(&d, "design", &saved(Some("abc1234"), true, 100.0)).unwrap());
        assert!(add_checkpoint(&d, "build", &Saved { commit: None, git_failed: true, studio: false, at: 200.0 }).unwrap());
        // a request taken after an undo put the stage back to do adds nothing
        assert!(!add_checkpoint(&d, "test", &saved(Some("abc1234"), false, 300.0)).unwrap());
        let run = read_run(&d).unwrap();
        assert_eq!(run["checkpoints"], json!([{"stage": "design", "commit": "abc1234", "studio": true, "at": 100.0}, {"stage": "build", "commit": null, "studio": false, "at": 200.0, "git": "failed"}]));
        assert_eq!(run["future"], 1, "other keys are kept");
        assert!(!crate::fridae_lock::lock_path(&d).exists(), "the lock is let go");
        // a checkpoint whose commit failed is refused by undo, with a clear reason
        assert_eq!(undo(&d.display().to_string(), Some("build"), false)["error"], GIT_FAILED);
        // the start of the run is recorded once, by its Lead, and only first
        assert!(!record_start_in(&d, "c1", &saved(Some("0000000"), false, 50.0)).unwrap(), "a run with checkpoints keeps them");
        change_run(&d, |r| {
            r["checkpoints"] = json!([]);
            (true, ())
        })
        .unwrap();
        assert!(!record_start_in(&d, "c-other", &saved(None, false, 50.0)).unwrap(), "not this run's Lead");
        assert!(record_start_in(&d, "c1", &saved(Some("0000000"), false, 50.0)).unwrap());
        assert_eq!(read_run(&d).unwrap()["checkpoints"][0]["stage"], "start");
    }

    #[test]
    fn gitignore_keeps_media_out_and_keeps_the_users_lines() {
        let d = game("ignore");
        let f = d.join(".fridae").join(".gitignore");
        ensure_gitignore(&d);
        assert_eq!(fs::read_to_string(&f).unwrap(), "review/*/\nreview-archive/\n.lock\n");
        fs::write(&f, "mine.txt\n.lock").unwrap();
        ensure_gitignore(&d);
        assert_eq!(fs::read_to_string(&f).unwrap(), "mine.txt\n.lock\nreview/*/\nreview-archive/\n");
        ensure_gitignore(&d);
        assert_eq!(fs::read_to_string(&f).unwrap(), "mine.txt\n.lock\nreview/*/\nreview-archive/\n", "nothing added twice");
        assert!(!d.join(".gitignore").exists(), "the project's own .gitignore is not touched");
    }

    fn git(dir: &Path, args: &[&str]) -> (bool, String) {
        let o = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string())
    }

    /// A git game with a space and "é" in its path: a.txt v1 at the "start" checkpoint, then a second commit adding
    /// new.txt, then an uncommitted edit and an untracked file; a review item filed after the checkpoint.
    fn git_game(name: &str, studio: bool) -> (PathBuf, String) {
        let d = game(name);
        assert!(git(&d, &["init", "-q"]).0);
        fs::write(d.join("a.txt"), "v1").unwrap();
        fs::write(run_file(&d), json!({"on": true, "lead": "c-none", "paused": false, "marker": "before",
            "stages": stages(&[("idea", 1.0, 0.0)])}).to_string()).unwrap();
        ensure_gitignore(&d);
        let first = projects::checkpoint(&d.display().to_string(), "first").unwrap();
        let at = paths::now_secs();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(d.join("a.txt"), "v2").unwrap();
        fs::write(d.join("new.txt"), "added later").unwrap();
        projects::checkpoint(&d.display().to_string(), "second").unwrap();
        fs::write(d.join("a.txt"), "v3 uncommitted").unwrap();
        fs::write(d.join("untracked.txt"), "loose").unwrap();
        let review = d.join(".fridae").join("review");
        fs::write(review.join("late.json"), json!({"id": "late", "status": "waiting", "created": at + 1.0}).to_string()).unwrap();
        fs::create_dir_all(review.join("late")).unwrap();
        fs::write(review.join("late").join("shot.png"), b"png").unwrap();
        let ended = at + 1.0;
        fs::write(run_file(&d), json!({"on": true, "lead": "c-none", "paused": false, "marker": "after",
            "stages": stages(&[("idea", at - 5.0, ended), ("design", ended, 0.0)]),
            "checkpoints": [{"stage": "start", "commit": first, "studio": studio, "at": at}]}).to_string()).unwrap();
        (d, first)
    }

    #[test]
    fn undo_puts_files_back_and_keeps_everything_in_history() {
        let (d, _) = git_game("git undo", false);
        let p = d.display().to_string();
        let r = undo(&p, None, false);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "v1");
        assert!(!d.join("new.txt").exists() && !d.join("untracked.txt").exists());
        // .fridae is not touched by git: the run file is the rewound one, not the checkpoint's
        let run = read_run(&d).unwrap();
        assert_eq!((run["marker"].as_str(), run["paused"].as_bool()), (Some("after"), Some(true)));
        assert_eq!(run["stages"][0]["status"], "doing");
        assert_eq!(run["stages"][1]["status"], "todo");
        assert!(d.join(".fridae").join("review-archive").read_dir().unwrap().next().is_some(), "the late item was archived");
        assert!(!d.join(".fridae").join("review").join("late.json").exists());
        // history keeps everything: the file added later, and the state just before the undo
        assert!(!git(&d, &["log", "--all", "--oneline", "--", "new.txt"]).1.trim().is_empty());
        let (_, log) = git(&d, &["log", "--format=%H %s"]);
        let before = log.lines().find(|l| l.contains("Before undoing the studio run")).expect("a commit before the undo").split(' ').next().unwrap().to_string();
        assert!(log.lines().next().unwrap().contains("Undo the studio run to the start of the run"));
        assert_eq!(git(&d, &["show", &format!("{before}:a.txt")]).1, "v3 uncommitted");
        assert_eq!(git(&d, &["show", &format!("{before}:untracked.txt")]).1, "loose");
        // review media are never in the checkpoint commits
        assert!(git(&d, &["show", &format!("{before}:.fridae/review/late/shot.png")]).1.is_empty());
        assert!(r["text"].as_str().unwrap().contains("Studio: not connected now"));
    }

    #[test]
    fn failed_restore_touches_nothing() {
        let (d, _) = git_game("git missing", true);
        let p = d.display().to_string();
        change_run(&d, |r| {
            r["checkpoints"][0]["commit"] = json!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
            (true, ())
        })
        .unwrap();
        let run_before = fs::read_to_string(run_file(&d)).unwrap();
        let head = git(&d, &["rev-parse", "HEAD"]).1;
        let r = undo(&p, None, true);
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some(MISSING_COMMIT)));
        assert_eq!(fs::read_to_string(run_file(&d)).unwrap(), run_before, "studio.json untouched");
        assert!(d.join(".fridae").join("review").join("late.json").exists(), "review items untouched");
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "v3 uncommitted");
        assert!(d.join("untracked.txt").exists());
        assert_eq!(git(&d, &["rev-parse", "HEAD"]).1, head, "no commit was made");
        // put_back itself stops before Studio or the files
        let back = put_back(&Checkpoint { at: 1.0, studio: true, project: Some(&p), commit: Some("deadbeefdeadbeef"), began: "x", undoing: None, skip_studio: None });
        assert!(!back.project_ok && back.studio_ok);
        assert_eq!(back.lines.len(), 1);
        assert!(back.lines[0].contains("Studio was left as it is"));
    }

    #[test]
    fn studio_closed_offers_files_only() {
        let (d, _) = git_game("git files only", true);
        let p = d.display().to_string();
        let run_before = fs::read_to_string(run_file(&d)).unwrap();
        let r = undo(&p, None, false);
        assert_eq!((r["ok"].as_bool(), r["error"].as_str(), r["files_only"].as_bool()), (Some(false), Some(STUDIO_CLOSED), Some(true)));
        assert_eq!(fs::read_to_string(run_file(&d)).unwrap(), run_before, "refused before anything changed");
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "v3 uncommitted");
        let r = undo(&p, None, true);
        assert_eq!(r["ok"], true, "{r}");
        let text = r["text"].as_str().unwrap();
        assert!(text.contains("Studio: left as it is (you chose to undo the files only)."), "{text}");
        assert!(!text.contains("Undid"), "no Studio undo is claimed");
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "v1");
    }

    #[test]
    fn undo_refused_while_an_expert_works_or_another_undo_runs() {
        let (d, _) = git_game("git busy", false);
        let p = d.display().to_string();
        let expert = agents::spawn_helper(Some("w-test"), "worker", "Coder: Busy", Some(p.clone()), None, Some("c-none")).unwrap();
        expert.set_running_for_test(true);
        let r = undo(&p, None, false);
        expert.set_running_for_test(false);
        agents::remove_agent(&expert.id);
        assert_eq!(r["error"], BUSY);
        let one = Undoing::take(&p).unwrap();
        assert_eq!(undo(&p, None, false)["error"], ALREADY_UNDOING);
        drop(one);
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "v3 uncommitted", "nothing was undone");
        assert_eq!(undo(&p, None, false)["ok"], true);
    }

    fn run_with_checkpoints() -> serde_json::Value {
        json!({"on": true, "checkpoints": [
            {"stage": "start", "commit": "a", "studio": false, "at": 10.0},
            {"stage": "design", "commit": "b", "studio": false, "at": 20.0},
            {"stage": "build", "commit": "c", "studio": false, "at": 30.0},
        ]})
    }

    #[test]
    fn undo_default_is_latest() {
        let run = run_with_checkpoints();
        let (i, cp) = pick_checkpoint(&run, None).unwrap();
        assert_eq!((i, cp["stage"].as_str()), (2, Some("build")));
        let (i, cp) = pick_checkpoint(&run, Some("")).unwrap();
        assert_eq!((i, cp["stage"].as_str()), (2, Some("build")), "an empty stage is the default");
        let (i, cp) = pick_checkpoint(&run, Some("start")).unwrap();
        assert_eq!((i, cp["commit"].as_str()), (0, Some("a")));
        // going back: stages finished after the checkpoint are to do again, the one going on at the time is going on
        // again, checkpoints after it go, and the run is paused
        let mut run = json!({"on": true, "stage": "test", "paused": false,
            "stages": stages(&[("idea", 5.0, 8.0), ("design", 11.0, 19.0), ("build", 21.0, 29.0), ("test", 31.0, 0.0)]),
            "checkpoints": run_with_checkpoints()["checkpoints"]});
        let undone = rewind(&mut run, 1);
        assert_eq!(undone, vec!["build".to_string(), "test".to_string()]);
        let st = |id: &str| run["stages"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone();
        assert_eq!(st("design")["status"], "done");
        assert_eq!((st("build")["status"].as_str(), st("build")["ended"].as_f64()), (Some("todo"), Some(0.0)));
        assert_eq!((st("test")["status"].as_str(), st("test")["started"].as_f64()), (Some("todo"), Some(0.0)));
        assert_eq!(run["stage"], "design");
        assert_eq!(run["paused"], true);
        assert_eq!(run["checkpoints"].as_array().unwrap().len(), 2, "the chosen checkpoint stays, so it can be gone back to again");
        // a stage that was under way at the checkpoint and finished later is under way again
        let mut run = json!({"on": true, "stages": stages(&[("idea", 5.0, 15.0)]), "checkpoints": [{"stage": "start", "at": 10.0}]});
        rewind(&mut run, 0);
        assert_eq!(run["stages"][0]["status"], "doing");
        assert_eq!(run["stages"][0]["started"], 5.0);
        assert_eq!(run["stage"], "idea");
    }

    #[test]
    fn undo_unknown_stage_refused() {
        let run = run_with_checkpoints();
        assert_eq!(pick_checkpoint(&run, Some("launch")).unwrap_err(), NO_SUCH_CHECKPOINT);
        assert_eq!(pick_checkpoint(&json!({"on": true}), None).unwrap_err(), NO_CHECKPOINTS);
        assert_eq!(pick_checkpoint(&json!({"on": true, "checkpoints": []}), Some("build")).unwrap_err(), NO_CHECKPOINTS);
        // the route refuses before touching anything
        let d = game("refuse");
        let p = d.display().to_string();
        fs::write(run_file(&d), run_with_checkpoints().to_string()).unwrap();
        let r = undo(&p, Some("promote"), false);
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some(NO_SUCH_CHECKPOINT)));
        assert_eq!(read_run(&d).unwrap(), run_with_checkpoints(), "nothing changed");
        assert_eq!(undo(&d.join("none").display().to_string(), None, false)["error"], NO_RUN);
    }

    #[test]
    fn undo_moves_later_review_items_to_the_archive() {
        let d = game("archive");
        let review = d.join(".fridae").join("review");
        let put = |id: &str, created: f64, extra: serde_json::Value| {
            let mut it = json!({"id": id, "stage": "build", "status": "waiting", "created": created, "media": []});
            for (k, v) in extra.as_object().unwrap() {
                it[k] = v.clone();
            }
            fs::write(review.join(format!("{id}.json")), it.to_string()).unwrap();
        };
        put("old", 5.0, json!({"status": "superseded"}));
        put("keep", 15.0, json!({"status": "approved"}));
        put("new", 25.0, json!({"replaces": "old", "media": ["a.png"]}));
        fs::create_dir_all(review.join("new")).unwrap();
        fs::write(review.join("new").join("a.png"), b"png").unwrap();
        let _lock = crate::fridae_lock::acquire(&d).unwrap();
        let (moved, failed, dest) = archive_after(&d, 20.0, 777);
        assert_eq!((moved, failed.len()), (1, 0));
        assert!(!review.join("new.json").exists() && !review.join("new").exists());
        let arch = d.join(".fridae").join("review-archive").join("undo-777");
        assert_eq!(dest, arch);
        assert_eq!(fs::read(arch.join("new").join("a.png")).unwrap(), b"png");
        assert!(arch.join("new.json").is_file());
        let read = |id: &str| -> serde_json::Value { serde_json::from_str(&fs::read_to_string(review.join(format!("{id}.json"))).unwrap()).unwrap() };
        assert_eq!(read("keep")["status"], "approved");
        assert_eq!(read("old")["status"], "changes", "the item a moved one replaced is open again");
        // nothing after the checkpoint: nothing moves and no folder is made
        let (moved, _, _) = archive_after(&d, 100.0, 778);
        assert_eq!(moved, 0);
        assert!(!d.join(".fridae").join("review-archive").join("undo-778").exists());
    }
}
