//! Studio mode: a checkpoint at the start of a run and after each finished stage, and "Undo this run" back to any of
//! them. A checkpoint is the one Overnight takes when a night begins (Agent::undo_night uses the same `put_back`):
//!
//!   - the project's files: a git commit of the project folder, when it is a git repository;
//!   - Studio's scripts: a snapshot the Fridae plugin keeps in Studio's memory, pinned at the checkpoint's time (the
//!     plugin keeps the newest 12 pins); the agents' Studio steps since then are undone by time, whoever made them.
//!
//! What undo needs is in `<project>/.fridae/studio.json` "checkpoints": [{"stage", "commit", "studio", "at"}] ("start"
//! for the run's first), so it works after the app restarts. Studio's part needs Studio to have stayed open on the same
//! place (its undo history and the plugin's snapshots live there).
//!
//! A finished stage asks for its checkpoint through a request file, `WORK/stage-requests/<id>.json`
//! {"project", "stage", "lead"} (rbxstage.py); `watch` takes it and appends it.

use std::{
    fs,
    path::{Path, PathBuf},
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
    /// The agents' Studio steps since the checkpoint, when known (a studio run's undo log).
    pub steps: Option<usize>,
}

pub struct PutBack {
    pub studio_ok: bool,
    pub project_ok: bool,
    pub lines: Vec<String>,
}

/// Put Studio and the project's files back as they were at a checkpoint. Each part is tried whatever the other did.
pub fn put_back(cp: &Checkpoint) -> PutBack {
    let mut out = PutBack { studio_ok: true, project_ok: true, lines: Vec::new() };
    if cp.at > 0.0 && plugin::connected() {
        let r = plugin::revert_within(cp.at, 600);
        out.studio_ok = r["ok"] == true;
        out.lines.push(format!("Studio: {}", r["text"].as_str().unwrap_or("")));
        if cp.undoing.is_some() && !plugin::running_current() {
            out.lines.push("Studio: its Fridae plugin is an older copy that keeps scripts only from the latest checkpoint, so scripts may not have gone all the way back. Update the plugin in Setup and restart Studio.".to_string());
        }
    } else if cp.studio && cp.steps == Some(0) {
        out.lines.push("Studio: no agent of the run worked in Studio since then, so there was nothing there to undo.".to_string());
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

fn checkpoint_entry(stage: &str, commit: Option<&str>, studio: bool, at: f64) -> Value {
    json!({"stage": stage, "commit": commit, "studio": studio, "at": at})
}

/// Append a checkpoint to a run that is on. Returns whether it was written.
fn add_checkpoint(project: &Path, stage: &str, commit: Option<&str>, studio: bool, at: f64) -> Result<bool, String> {
    change_run(project, |run| {
        if !run.is_object() || run["on"] != true {
            return (false, false);
        }
        if !run["checkpoints"].is_array() {
            run["checkpoints"] = json!([]);
        }
        run["checkpoints"].as_array_mut().map(|list| list.push(checkpoint_entry(stage, commit, studio, at)));
        (true, true)
    })
}

/// The run's first checkpoint, "start": written by its Lead's night checkpoint, only while the run has none (a
/// Lead adopting the run later, or a message to it, keeps the first).
fn record_start_in(project: &Path, lead: &str, commit: Option<&str>, studio: bool, at: f64) -> Result<bool, String> {
    change_run(project, |run| {
        let empty = run["checkpoints"].as_array().is_none_or(|l| l.is_empty());
        if !run.is_object() || !running(run) || run["lead"] != lead || !empty {
            return (false, false);
        }
        run["checkpoints"] = json!([checkpoint_entry("start", commit, studio, at)]);
        (true, true)
    })
}

pub fn record_start(project: &str, lead: &str, commit: Option<&str>, studio: bool, at: f64) {
    let _ = record_start_in(Path::new(project), lead, commit, studio, at);
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

/// POST /api/studio-run {action: "undo", path, stage?}: the game back to a checkpoint of its run (the newest when no
/// stage is given). Refused while the Lead or an expert is working. The run is left paused.
pub fn undo(project: &str, stage: Option<&str>) -> Value {
    let dir = Path::new(project);
    let Some(run) = read_run(dir).filter(|r| r["on"] == true) else { return refuse(NO_RUN) };
    let (idx, cp) = match pick_checkpoint(&run, stage) {
        Ok(found) => found,
        Err(e) => return refuse(&e),
    };
    let lead = run["lead"].as_str().unwrap_or("").to_string();
    if run_busy(project, &lead) {
        return refuse(BUSY);
    }
    // paused before anything goes back, so no new work starts meanwhile
    let _ = change_run(dir, |r| {
        let go = r.is_object() && r["paused"] != true;
        if go {
            r["paused"] = json!(true);
        }
        (go, ())
    });
    let at = cp["at"].as_f64().unwrap_or(0.0);
    let name = checkpoint_name(cp["stage"].as_str().unwrap_or(""));
    let live = agents::get(&lead);
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
        steps: Some(since.len()),
    });
    let mut lines = vec![format!("Back to {name}.")];
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
            l.note(&format!("The user undid this studio run back to {name}: the game's files and Studio were put back, later stages are to do again, and review items filed since then were archived. Call rbx_stage status before going on."));
            l.put("run_undone", json!({"text": text}));
        } else {
            l.put("error", json!({"text": format!("Undo this run did not finish.\n{text}")}));
        }
    }
    if ok { json!({"ok": true, "text": text}) } else { json!({"ok": false, "text": text, "error": text}) }
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

/// Take a finished stage's checkpoint: Studio's scripts pinned now, the project's files committed, then appended.
fn take_stage_checkpoint(req: &Value) {
    let Some(project) = studio_run::project_of(req["project"].as_str().unwrap_or("")) else { return };
    let stage = req["stage"].as_str().unwrap_or("");
    let dir = Path::new(&project);
    if !STAGES.contains(&stage) || !read_run(dir).is_some_and(|r| r["on"] == true) {
        return;
    }
    let at = paths::now_secs();
    plugin::pin(at);
    let studio = plugin::connected();
    let commit = projects::checkpoint(&project, &format!("After the {stage} stage of the studio run (Fridae Studios checkpoint)"));
    let _ = add_checkpoint(dir, stage, commit.as_deref(), studio, at);
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

    #[test]
    fn checkpoint_appended_per_stage() {
        let d = game("append");
        // no run, or a run that is off: nothing is written
        assert!(!add_checkpoint(&d, "build", Some("abc1234"), false, 10.0).unwrap());
        assert!(!run_file(&d).exists());
        fs::write(run_file(&d), json!({"on": false}).to_string()).unwrap();
        assert!(!add_checkpoint(&d, "build", None, false, 10.0).unwrap());
        fs::write(run_file(&d), json!({"on": true, "lead": "c1", "stages": stages(&[]), "future": 1}).to_string()).unwrap();
        assert!(add_checkpoint(&d, "design", Some("abc1234"), true, 100.0).unwrap());
        assert!(add_checkpoint(&d, "build", None, false, 200.0).unwrap());
        let run = read_run(&d).unwrap();
        assert_eq!(run["checkpoints"], json!([{"stage": "design", "commit": "abc1234", "studio": true, "at": 100.0}, {"stage": "build", "commit": null, "studio": false, "at": 200.0}]));
        assert_eq!(run["future"], 1, "other keys are kept");
        assert!(!crate::fridae_lock::lock_path(&d).exists(), "the lock is let go");
        // the start of the run is recorded once, by its Lead, and only first
        assert!(!record_start_in(&d, "c1", Some("0000000"), false, 50.0).unwrap(), "a run with checkpoints keeps them");
        change_run(&d, |r| {
            r["checkpoints"] = json!([]);
            (true, ())
        })
        .unwrap();
        assert!(!record_start_in(&d, "c-other", None, false, 50.0).unwrap(), "not this run's Lead");
        assert!(record_start_in(&d, "c1", Some("0000000"), false, 50.0).unwrap());
        assert_eq!(read_run(&d).unwrap()["checkpoints"][0]["stage"], "start");
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
        let r = undo(&p, Some("promote"));
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some(NO_SUCH_CHECKPOINT)));
        assert_eq!(read_run(&d).unwrap(), run_with_checkpoints(), "nothing changed");
        assert_eq!(undo(&d.join("none").display().to_string(), None)["error"], NO_RUN);
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
