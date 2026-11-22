//! Studio mode, the app's side: the Review tab's listing, the user's decisions on review items, and starting, pausing
//! and resuming a game's run. The files belong to rbxstage.py (the agents' `rbx_stage`), and are read and written here
//! exactly as it does, under the same per-project lock (fridae_lock), keeping keys this file does not know:
//!
//!   <project>/.fridae/studio.json        the run: {"on", "stage", "stages", "gates", "max_experts", "lead", "idea",
//!                                         "started", "paused", "read_decisions", "note"?, "checkpoints"?, "ended"?}
//!                                         ("checkpoints" belong to studio_undo.rs)
//!   <project>/.fridae/review/<id>.json   one review item: {"id", "stage", "expert", "title", "summary", "media",
//!                                         "blocking", "status", "note", "created", "decided", "replaces", "by_chat"}
//!   <project>/.fridae/review/<id>/       its pictures and clips (the names in "media")
//!
//! A decision only sets "status", "note" and "decided"; rbxstage hands it to the Lead as `id:status`. The app also
//! sends the Lead a short message so a run waiting at a gate goes on. The watcher (studio_watch.rs) tells the window
//! when items or runs change.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    thread,
    time::SystemTime,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent, Ask},
    board, fridae_lock, paths, projects,
    promo::{valid_id, write_atomic},
};

pub const STAGES: [&str; 6] = ["idea", "design", "build", "test", "launch", "promote"];
const DEFAULT_GATES: [&str; 2] = ["design", "launch"];
const NOTE_MAX: usize = 600;
const IDEA_MAX: usize = 4000;
const ALREADY: &str = "This game already has a studio run. Resume it from Review.";

/// Review item files the app wrote itself, and their times, shared with the watcher so it does not report them.
pub(crate) static SEEN: LazyLock<Mutex<HashMap<PathBuf, SystemTime>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn run_file(project: &Path) -> PathBuf {
    project.join(".fridae").join("studio.json")
}

pub(crate) fn review_dir(project: &Path) -> PathBuf {
    project.join(".fridae").join("review")
}

pub(crate) fn read_obj(path: &Path) -> Option<Value> {
    serde_json::from_str::<Value>(&fs::read_to_string(path).ok()?).ok().filter(Value::is_object)
}

pub(crate) fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub(crate) fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    if write_atomic(path, &bytes) { Ok(()) } else { Err("Could not save the game's Studio files.".into()) }
}

/// Change studio.json under the project's lock: `f` gets the run (Null when there is none or it does not read) and
/// says whether to save it, which is done through a temporary file.
pub fn change_run<T>(project: &Path, f: impl FnOnce(&mut Value) -> (bool, T)) -> Result<T, String> {
    let _lock = fridae_lock::acquire(project)?;
    let file = run_file(project);
    let mut run = read_obj(&file).unwrap_or(Value::Null);
    let (save, out) = f(&mut run);
    if save {
        write_json(&file, &run)?;
    }
    Ok(out)
}

pub fn read_run(project: &Path) -> Option<Value> {
    read_obj(&run_file(project))
}

/// The review item files in a project: (id, path).
pub(crate) fn item_files(project: &Path) -> Vec<(String, PathBuf)> {
    let entries = fs::read_dir(review_dir(project)).into_iter().flatten().flatten().map(|e| e.path());
    entries
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_string(), p)))
        .filter(|(id, _)| valid_id(id))
        .collect()
}

/// The items the Review tab shows: newest first, without the ones a newer version replaced. Files that do not read
/// (still being written) are left out.
pub fn items(project: &Path) -> Vec<Value> {
    let mut out: Vec<Value> =
        item_files(project).into_iter().filter_map(|(_, p)| read_obj(&p)).filter(|i| i["id"].is_string() && i["status"] != "superseded").collect();
    out.sort_by(|a, b| b["created"].as_f64().unwrap_or(0.0).total_cmp(&a["created"].as_f64().unwrap_or(0.0)));
    out
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// The name a project goes by in media addresses: the first 12 hex characters of the SHA-1 of its path.
pub fn project_key(project: &str) -> String {
    sha1(project.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..12].to_string()
}

fn media_type(name: &str) -> Option<&'static str> {
    match Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "mp4" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        _ => None,
    }
}

/// The file `/media/review/<key>/<id>/<file>` may serve: one of the item's own pictures or clips, in its folder under
/// a known project's `.fridae/review/`, and nothing else.
fn media_in(known: &[String], key: &str, id: &str, file: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) || !valid_id(file) {
        return None;
    }
    let ctype = media_type(file)?;
    let project = known.iter().find(|p| project_key(p) == key)?;
    let review = review_dir(Path::new(project));
    let item = read_obj(&review.join(format!("{id}.json")))?;
    if !item["media"].as_array()?.iter().any(|m| m.as_str() == Some(file)) {
        return None;
    }
    let path = review.join(id).join(file);
    // a link planted in the folder may not lead out of it
    let (real, root) = (path.canonicalize().ok()?, review.canonicalize().ok()?);
    real.starts_with(&root).then_some((path, ctype))
}

fn known_paths() -> Vec<String> {
    projects::list().iter().filter_map(|p| p["path"].as_str().map(str::to_string)).collect()
}

pub fn media(key: &str, id: &str, file: &str) -> Option<(PathBuf, &'static str)> {
    media_in(&known_paths(), key, id, file)
}

/// Save an item the app changed, noting the new file time so the watcher does not take it for news.
fn write_item(path: &Path, item: &Value, seen: &mut HashMap<PathBuf, SystemTime>) -> Result<(), String> {
    write_json(path, item)?;
    if let Some(m) = mtime(path) {
        seen.insert(path.to_path_buf(), m);
    }
    Ok(())
}

fn clean_note(note: &str) -> Result<String, String> {
    let note = note.trim();
    if note.chars().count() > NOTE_MAX {
        return Err(format!("Keep the note to {NOTE_MAX} characters or fewer."));
    }
    Ok(note.to_string())
}

/// The user approves an item, or asks for a change (`approve` false, with a note). Only an item still waiting can be
/// decided: checked and written under the lock. Returns the message for the Lead.
fn decide(project: &Path, id: &str, approve: bool, note: &str, seen: &mut HashMap<PathBuf, SystemTime>) -> Result<String, String> {
    if !valid_id(id) {
        return Err("That review item is gone.".into());
    }
    let note = clean_note(note)?;
    if !approve && note.is_empty() {
        return Err("Say what to change.".into());
    }
    let path = review_dir(project).join(format!("{id}.json"));
    let _lock = fridae_lock::acquire(project)?;
    let Some(mut item) = read_obj(&path) else { return Err("That review item is gone.".into()) };
    if item["status"] != "waiting" {
        return Err("Already answered.".into());
    }
    item["status"] = json!(if approve { "approved" } else { "changes" });
    item["note"] = json!(note);
    item["decided"] = json!(paths::now_secs());
    write_item(&path, &item, seen)?;
    if approve && item["blocking"] == true {
        supersede_older_changes(project, &item, seen);
    }
    let title = item["title"].as_str().unwrap_or("").trim().to_string();
    Ok(if approve { format!("Review: {title} \u{2014} approved") } else { format!("Review: {title} \u{2014} change: {note}") })
}

/// A blocking item was approved: older blocking items of the same stage still marked "changes" (a redo filed without
/// `replaces`) are superseded by it, so they cannot hold the gate shut. Call under the lock.
fn supersede_older_changes(project: &Path, approved: &Value, seen: &mut HashMap<PathBuf, SystemTime>) {
    let created = approved["created"].as_f64().unwrap_or(0.0);
    for (id, path) in item_files(project) {
        let Some(mut old) = read_obj(&path) else { continue };
        let older = old["created"].as_f64().unwrap_or(0.0) < created;
        if id != approved["id"].as_str().unwrap_or("") && older && old["status"] == "changes" && old["blocking"] == true && old["stage"] == approved["stage"] {
            old["status"] = json!("superseded");
            old["superseded_by"] = approved["id"].clone();
            let _ = write_item(&path, &old, seen);
        }
    }
}

/// Approve the waiting items the user saw (`ids`, the ones on screen) that do not block the run. Returns how many.
fn approve_all(project: &Path, ids: &[String], seen: &mut HashMap<PathBuf, SystemTime>) -> Result<usize, String> {
    let _lock = fridae_lock::acquire(project)?;
    let now = paths::now_secs();
    let mut n = 0;
    for (id, path) in item_files(project) {
        if !ids.contains(&id) {
            continue; // filed after the user looked: it waits for its own look
        }
        let Some(mut item) = read_obj(&path) else { continue };
        if item["status"] != "waiting" || item["blocking"].as_bool().unwrap_or(false) {
            continue;
        }
        item["status"] = json!("approved");
        item["note"] = json!("");
        item["decided"] = json!(now);
        write_item(&path, &item, seen)?;
        n += 1;
    }
    Ok(n)
}

/// Gates and experts at once from a request, checked: gates are stages (kept in stage order), experts 1 to 4.
fn settings_from(body: &Value) -> Result<(Option<Vec<String>>, Option<u64>), String> {
    let gates = match body.get("gates") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            let names: Vec<&str> = list.iter().map(|g| g.as_str().unwrap_or("")).collect();
            if names.iter().any(|g| !STAGES.contains(g)) {
                return Err(format!("Gates must be stages: {}.", STAGES.join(", ")));
            }
            Some(STAGES.iter().filter(|s| names.contains(s)).map(|s| s.to_string()).collect())
        }
        Some(_) => return Err("Gates must be a list of stages.".into()),
    };
    let max = match body.get("max_experts") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|n| (1..=4).contains(n)).ok_or("Experts at once must be 1, 2, 3 or 4.")?),
    };
    Ok((gates, max))
}

/// Every stage done: the run is over and a new one may start.
fn finished(run: &Value) -> bool {
    let stages = run["stages"].as_array().cloned().unwrap_or_default();
    STAGES.iter().all(|s| stages.iter().any(|e| e["id"] == *s && e["status"] == "done"))
}

pub(crate) fn running(run: &Value) -> bool {
    run["on"] == true && !finished(run)
}

fn can_start(project: &Path) -> Result<(), String> {
    if !project.is_dir() {
        return Err("That game's folder is missing.".into());
    }
    match read_run(project) {
        Some(run) if running(&run) => Err(ALREADY.into()),
        _ => Ok(()),
    }
}

/// A new run, as rbxstage's `_new_run` makes it, but on. Decisions on items still in the review folder (ones the
/// archive could not move) count as read, so the new Lead is not handed them.
fn fresh_run(project: &Path, idea: &str, gates: Vec<String>, max_experts: u64, lead: &str) -> Value {
    let read: Vec<String> = item_files(project)
        .into_iter()
        .filter_map(|(_, p)| read_obj(&p))
        .filter(|i| i["status"] == "approved" || i["status"] == "changes")
        .map(|i| format!("{}:{}", i["id"].as_str().unwrap_or(""), i["status"].as_str().unwrap_or("")))
        .collect();
    json!({
        "on": true, "stage": "",
        "stages": STAGES.iter().map(|s| json!({"id": s, "status": "todo", "started": 0, "ended": 0})).collect::<Vec<_>>(),
        "gates": gates, "max_experts": max_experts, "lead": lead, "idea": idea, "started": paths::now_secs(), "paused": false,
        "read_decisions": read,
    })
}

/// Move everything in the review folder to `.fridae/review-archive/<stamp>/` (a free name), each item's file before
/// its media folder, so a failure never leaves a listed item without its pictures. Returns what could not be moved.
/// Call under the project's lock.
fn archive_review(project: &Path, stamp: u64) -> Vec<String> {
    let entries: Vec<PathBuf> = fs::read_dir(review_dir(project)).into_iter().flatten().flatten().map(|e| e.path()).collect();
    archive_entries(project, &stamp.to_string(), entries).1
}

/// Move these review files and folders to `.fridae/review-archive/<stamp>/` (or `<stamp>-2`… when that is taken),
/// item files before media folders, and a folder only when its item file went. Returns the folder and the names that
/// could not be moved; nothing is made when there is nothing to move. Call under the project's lock.
pub(crate) fn archive_entries(project: &Path, stamp: &str, mut entries: Vec<PathBuf>) -> (PathBuf, Vec<String>) {
    let root = project.join(".fridae").join("review-archive");
    let mut dest = root.join(stamp);
    let mut n = 1;
    while dest.exists() {
        n += 1;
        dest = root.join(format!("{stamp}-{n}"));
    }
    if entries.is_empty() {
        return (dest, vec![]);
    }
    if fs::create_dir_all(&dest).is_err() {
        return (dest, entries.iter().map(|p| paths::base_name(&p.display().to_string())).collect());
    }
    // item files first; a folder goes only when its item went (or it has no item file)
    entries.sort_by_key(|p| !p.extension().is_some_and(|e| e == "json"));
    let mut failed: Vec<String> = Vec::new();
    for path in entries {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if path.is_dir() && failed.contains(&format!("{name}.json")) {
            failed.push(name);
            continue;
        }
        if fs::rename(&path, dest.join(&name)).is_err() {
            failed.push(name);
        }
    }
    (dest, failed)
}

/// Under the lock: refuse when a run is going, else archive the last run's review items and write the new run.
/// Returns a warning when some old review files could not be moved.
fn write_fresh(project: &Path, idea: &str, gates: Vec<String>, max_experts: u64, lead: &str) -> Result<Option<String>, String> {
    change_run(project, |run| {
        if running(run) {
            return (false, Err(ALREADY.to_string()));
        }
        let stamp = run["started"].as_f64().filter(|s| *s > 0.0).unwrap_or_else(paths::now_secs) as u64;
        let failed = archive_review(project, stamp);
        crate::studio_undo::ensure_gitignore(project); // checkpoints never commit review media
        *run = fresh_run(project, idea, gates, max_experts, lead);
        let warning = (!failed.is_empty()).then(|| format!("Could not move these old review files into the archive: {}.", failed.join(", ")));
        (true, Ok(warning))
    })?
}

/// The run's Lead, when its chat is open in this session.
pub(crate) fn live_lead(run: &Value) -> Option<Arc<Agent>> {
    run["lead"].as_str().filter(|l| !l.is_empty()).and_then(agents::get)
}

/// Tell the run's Lead what the user decided. With no live Lead the run is marked paused (Resume starts one, and
/// rbx_stage hands it the decision). Returns whether a Lead was told.
fn notify_lead(project: &Path, text: &str) -> bool {
    if let Some(lead) = read_run(project).as_ref().and_then(live_lead) {
        lead.wake_studio(text);
        return true;
    }
    let _ = change_run(project, |run| {
        if !run.is_object() || run["paused"] == true {
            return (false, ());
        }
        run["paused"] = json!(true);
        (true, ())
    });
    false
}

/// A new background chat to lead the project's run, "Lead: <project name>".
fn new_lead_chat(project: &str) -> Option<Arc<Agent>> {
    let name = projects::describe(project)["name"].as_str().unwrap_or("").to_string();
    agents::new_background_chat(&format!("Lead: {name}"), Some(project.to_string()))
}

/// The chat a run starts in: the one the user started it from (`agent`), which must be an idle chat of their own in
/// this project or in none yet (it then joins the project), or else a new Lead chat. The second value says whether
/// the chat was made here.
fn start_lead(project: &str, agent: Option<&str>) -> Result<(Arc<Agent>, bool), String> {
    let Some(id) = agent.filter(|a| !a.is_empty()) else { return new_lead_chat(project).map(|a| (a, true)).ok_or_else(|| NO_SLOT.to_string()) };
    let chat = agents::get(id).ok_or("That chat is gone.")?;
    if chat.role().is_some() {
        return Err("Only a chat of your own can lead a studio run.".into());
    }
    if chat.busy() {
        return Err("That chat is working. Wait for it to finish, then start the run.".into());
    }
    match chat.project() {
        Some(p) if projects::same(&p, project) => {}
        Some(_) => return Err("That chat belongs to another game.".into()),
        None if chat.set_project(project) => {}
        None => return Err("That chat has already started without this game. Start the run from a new chat.".into()),
    }
    Ok((chat, false))
}

const NO_SLOT: &str = "All agent slots are in use. Close a chat, then try again.";

fn kickoff(lead: &Arc<Agent>, text: String) {
    let ask = Ask { studio: true, full: true, overnight: true, kickoff: Some("studio".into()), ..Default::default() };
    let lead = lead.clone();
    thread::spawn(move || lead.ask(&text, &ask));
}

fn start(project: &str, body: &Value) -> Result<Value, String> {
    let dir = Path::new(project);
    let (gates, max) = settings_from(body)?;
    let gates = gates.unwrap_or_else(|| DEFAULT_GATES.iter().map(|g| g.to_string()).collect());
    let idea: String = body["idea"].as_str().unwrap_or("").trim().chars().take(IDEA_MAX).collect();
    can_start(dir)?;
    let (lead, made) = start_lead(project, body["agent"].as_str())?;
    let warning = finish_start(dir, &lead, made, &idea, gates, max.unwrap_or(2))?;
    kickoff(&lead, if idea.is_empty() { "Start a studio run for this game.".into() } else { idea });
    let mut out = json!({"ok": true, "agent": lead.id});
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

/// Write the new run for `lead`; when another request started a run meanwhile, a chat made for this one is closed.
fn finish_start(dir: &Path, lead: &Arc<Agent>, made: bool, idea: &str, gates: Vec<String>, max: u64) -> Result<Option<String>, String> {
    let wrote = write_fresh(dir, idea, gates, max, &lead.id);
    if wrote.is_err() && made {
        agents::remove_agent(&lead.id);
    }
    wrote
}

const NO_RUN: &str = "This game has no studio run to resume.";

/// What Resume found under the lock.
#[derive(Debug, PartialEq)]
enum Claim {
    /// The run is unpaused, and led by the chat given (or still by its live Lead).
    Ours,
    /// Another request gave the run a Lead since it was read: this one.
    Taken(String),
}

/// Under the lock: the run must be on and not finished, and still led by `expected` (the Lead read before); then it
/// is unpaused and, with `new_lead`, handed to that chat.
fn claim_lead(project: &Path, expected: &str, new_lead: Option<&str>) -> Result<Claim, String> {
    change_run(project, |run| {
        if !run.is_object() || !running(run) {
            return (false, Err(NO_RUN.to_string()));
        }
        let now = run["lead"].as_str().unwrap_or("").to_string();
        if now != expected {
            return (false, Ok(Claim::Taken(now)));
        }
        run["paused"] = json!(false);
        // the reason a run stopped by itself goes with the pause
        if run["note"].as_str().is_some_and(|n| n == "failed" || n.starts_with("limit:")) {
            run.as_object_mut().map(|o| o.remove("note"));
        }
        if let Some(id) = new_lead {
            run["lead"] = json!(id);
        }
        (true, Ok(Claim::Ours))
    })?
}

fn resume(project: &str) -> Result<Value, String> {
    let dir = Path::new(project);
    let run = read_run(dir).filter(running).ok_or(NO_RUN)?;
    let expected = run["lead"].as_str().unwrap_or("").to_string();
    let id = match live_lead(&run) {
        Some(lead) => match claim_lead(dir, &expected, None)? {
            Claim::Ours => {
                lead.wake_studio("Resume the studio run.");
                lead.id.clone()
            }
            Claim::Taken(other) => other,
        },
        None => {
            let chat = new_lead_chat(project).ok_or(NO_SLOT)?;
            match claim_lead(dir, &expected, Some(&chat.id)) {
                Ok(Claim::Ours) => {
                    kickoff(&chat, "Adopt this game's studio run and carry on.".into());
                    chat.id.clone()
                }
                Ok(Claim::Taken(other)) => {
                    agents::remove_agent(&chat.id);
                    other
                }
                Err(e) => {
                    agents::remove_agent(&chat.id);
                    return Err(e);
                }
            }
        }
    };
    Ok(json!({"ok": true, "agent": id}))
}

const NO_RUN_TO_END: &str = "This game has no studio run going to end.";

/// Under the lock: a run that is going is switched off with its end time ("on": false, "ended"); its stages, files,
/// review items and checkpoints all stay. Returns the Lead's id.
fn end_run(project: &Path) -> Result<String, String> {
    change_run(project, |run| {
        if !run.is_object() || !running(run) {
            return (false, Err(NO_RUN_TO_END.to_string()));
        }
        run["on"] = json!(false);
        run["ended"] = json!(paths::now_secs());
        run.as_object_mut().map(|o| o.remove("note"));
        (true, Ok(run["lead"].as_str().unwrap_or("").to_string()))
    })?
}

/// End this run: nothing destructive (files and review items stay). The Lead's turn under way and the experts at work
/// are stopped (a Lead blocked in `rbx_team wait` would otherwise go on spending, and so would its experts), nothing
/// carries on, and a new run may start (it archives this one's review items as usual). The run is marked ended first,
/// so a stopped turn's ending already sees it: no carry-on, and held messages go as ordinary ones.
fn end(project: &str) -> Result<Value, String> {
    let lead = end_run(Path::new(project))?;
    // the experts are stopped and closed even when the Lead's chat is closed: they would go on spending without it,
    // and a stopped one left open would only clutter the Lead's chat
    for expert in agents::helpers_of(&lead) {
        agents::remove_agent(&expert.id);
    }
    if let Some(l) = agents::get(&lead).filter(|_| !lead.is_empty()) {
        l.end_run();
        l.stop();
    }
    Ok(json!({"ok": true}))
}

/// Change an existing run's own fields.
fn set_run(project: &Path, f: impl FnOnce(&mut Value)) -> Result<(), String> {
    let had = change_run(project, |run| {
        if !run.is_object() {
            return (false, false);
        }
        f(run);
        (true, true)
    })?;
    if had { Ok(()) } else { Err("This game has no studio run yet.".into()) }
}

/// The project a request names, as the app's list of projects has it: only folders the user added, and that exist.
pub(crate) fn project_of(path: &str) -> Option<String> {
    known_paths().into_iter().find(|p| projects::same(p, path)).filter(|p| Path::new(p).is_dir())
}

fn not_ours() -> Value {
    json!({"ok": false, "error": "That game is not one of your projects."})
}

pub(crate) fn answer(r: Result<Value, String>) -> Value {
    r.unwrap_or_else(|e| json!({"ok": false, "error": e}))
}

/// Whether helper agents (and so a Studio team) work on this AI. Every AI the app runs gets the rbxkit tools with
/// rbx_team, so all of them do.
fn helpers_work(ai: &str) -> bool {
    paths::AIS.contains(&ai)
}

/// Roster ids to the names people read ("ui" is "UI builder"): the kit's roster, then the game's own entries over it.
fn expert_names(kit: &Path, project: &Path) -> serde_json::Map<String, Value> {
    let mut names = serde_json::Map::new();
    for f in [paths::kit_team(kit).join("roster.json"), project.join(".fridae").join("team").join("roster.json")] {
        let list: Value = fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        for e in list.as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (e["id"].as_str(), e["name"].as_str()) {
                names.insert(id.to_string(), json!(name));
            }
        }
    }
    names
}

/// GET /api/review?path=<project>
pub fn list(path: &str) -> Value {
    let Some(project) = project_of(path) else { return not_ours() };
    let dir = Path::new(&project);
    let run = read_run(dir);
    let lead = run.as_ref().and_then(live_lead);
    let ai = lead.as_ref().map(|l| l.ai()).unwrap_or_else(paths::default_ai);
    let board = board::get(&project);
    let needs: Vec<Value> = board["cards"].as_array().into_iter().flatten().filter(|c| c["column"] == "needs").cloned().collect();
    json!({
        "ok": true, "project": project, "key": project_key(&project), "run": run, "items": items(dir), "needs": needs,
        "helpers": helpers_work(&ai), "lead_live": lead.is_some(), "waiting": lead.as_ref().is_some_and(|l| l.studio_waiting()),
        // the Lead's chat is open and actually running the run (after a restart it is open but idle until Resume)
        "lead_active": lead.as_ref().is_some_and(|l| l.busy() || l.overnight() || l.studio_waiting()),
        "experts": expert_names(&paths::PATHS.kit, dir),
    })
}

/// A needs card on the board was marked done (in Review): the Lead of the game's running run hears
/// "Board: <title> — done". A waiting Lead is woken; a working one gets it with its next turn; a paused run, or one
/// whose Lead is not open, is not woken (the card is on the board for when it goes on). Returns whether a Lead was told.
fn needs_done(project: &Path, title: &str) -> Result<Value, String> {
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
    if title.is_empty() {
        return Err("Which card was done?".into());
    }
    let Some(run) = read_run(project).filter(|r| running(r) && r["paused"] != true) else { return Ok(json!({"ok": true, "told": false})) };
    let Some(lead) = live_lead(&run) else { return Ok(json!({"ok": true, "told": false})) };
    let text = board_done_text(&title);
    if lead.studio_waiting() {
        lead.wake_studio(&text);
    } else {
        lead.note(&text);
    }
    Ok(json!({"ok": true, "told": true}))
}

fn board_done_text(title: &str) -> String {
    format!("Board: {title} \u{2014} done")
}

/// POST /api/review {path, action: "approve" | "change" | "approve_all" | "needs_done", id?, note?, ids?, title?}
pub fn review(body: &Value) -> Value {
    let Some(project) = project_of(body["path"].as_str().unwrap_or("")) else { return not_ours() };
    let dir = Path::new(&project);
    let (id, note) = (body["id"].as_str().unwrap_or(""), body["note"].as_str().unwrap_or(""));
    let text = match body["action"].as_str().unwrap_or("") {
        "approve" => decide(dir, id, true, note, &mut SEEN.lock().unwrap()),
        "change" => decide(dir, id, false, note, &mut SEEN.lock().unwrap()),
        "approve_all" => match body["ids"].as_array() {
            Some(list) => {
                let ids: Vec<String> = list.iter().filter_map(Value::as_str).map(str::to_string).collect();
                approve_all(dir, &ids, &mut SEEN.lock().unwrap())
                    .map(|n| if n == 0 { String::new() } else { format!("Review: {n} item{} approved", if n == 1 { "" } else { "s" }) })
            }
            None => Err("Say which items to approve.".into()),
        },
        "needs_done" => return answer(needs_done(dir, body["title"].as_str().unwrap_or(""))),
        _ => Err("Unknown action.".into()),
    };
    match text {
        Ok(text) => {
            if !text.is_empty() {
                notify_lead(dir, &text);
            }
            json!({"ok": true})
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// POST /api/studio-run {path, action: "start" | "pause" | "resume" | "settings" | "undo" | "end", idea?, gates?, max_experts?,
/// stage?, files_only?} ("undo" is studio_undo.rs)
pub fn studio_run(body: &Value) -> Value {
    let path = body["path"].as_str().unwrap_or("");
    if body["action"] == "start" && !Path::new(path).is_dir() {
        return json!({"ok": false, "error": "That game's folder is missing."});
    }
    let Some(project) = project_of(path) else { return not_ours() };
    let dir = Path::new(&project);
    match body["action"].as_str().unwrap_or("") {
        "start" => answer(start(&project, body)),
        "resume" => answer(resume(&project)),
        "pause" => answer(set_run(dir, |run| run["paused"] = json!(true)).map(|_| json!({"ok": true}))),
        "end" => answer(end(&project)),
        "undo" => crate::studio_undo::undo(&project, body["stage"].as_str(), body["files_only"].as_bool().unwrap_or(false)),
        "settings" => answer(settings_from(body).and_then(|(gates, max)| {
            set_run(dir, |run| {
                if let Some(g) = gates {
                    run["gates"] = json!(g);
                }
                if let Some(m) = max {
                    run["max_experts"] = json!(m);
                }
            })
            .map(|_| json!({"ok": true}))
        })),
        _ => json!({"ok": false, "error": "Unknown action."}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio_watch::changes;
    use std::collections::HashSet;

    fn game(name: &str) -> PathBuf {
        let d = crate::paths::test_dir(&format!("studio-run-{name}")).join("My Gäme é");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".fridae").join("review")).unwrap();
        d
    }

    fn put_item(d: &Path, id: &str, extra: Value) {
        let mut item = json!({"id": id, "stage": "build", "expert": "coder", "title": format!("Item {id}"), "summary": "s", "media": [],
            "blocking": false, "status": "waiting", "note": "", "created": 1.0, "decided": 0, "replaces": "", "by_chat": "c1", "extra_key": 7});
        for (k, v) in extra.as_object().unwrap() {
            item[k] = v.clone();
        }
        fs::write(d.join(".fridae").join("review").join(format!("{id}.json")), serde_json::to_string(&item).unwrap()).unwrap();
    }

    fn item(d: &Path, id: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(d.join(".fridae").join("review").join(format!("{id}.json"))).unwrap()).unwrap()
    }

    fn put_run(d: &Path, run: Value) {
        fs::write(run_file(d), serde_json::to_string(&run).unwrap()).unwrap();
    }

    fn run(d: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(run_file(d)).unwrap()).unwrap()
    }

    #[test]
    fn list_newest_first_skips_superseded() {
        let d = game("list");
        put_item(&d, "aa", json!({"created": 10.0}));
        put_item(&d, "bb", json!({"created": 30.0}));
        put_item(&d, "cc", json!({"created": 20.0, "status": "superseded"}));
        put_item(&d, "dd", json!({"created": 5.0, "status": "approved"}));
        fs::write(d.join(".fridae").join("review").join("half.json"), "{\"id\": \"half\", ").unwrap();
        let ids: Vec<String> = items(&d).iter().map(|i| i["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["bb", "aa", "dd"]);
    }

    #[test]
    fn expert_names_take_the_games_own_roster_over_the_kits() {
        let d = game("names");
        let kit = d.join("kit");
        fs::create_dir_all(paths::kit_team(&kit)).unwrap();
        fs::create_dir_all(d.join(".fridae").join("team")).unwrap();
        fs::write(paths::kit_team(&kit).join("roster.json"), r#"[{"id": "ui", "name": "UI builder"}, {"id": "coder", "name": "Coder"}]"#).unwrap();
        fs::write(d.join(".fridae").join("team").join("roster.json"), r#"[{"id": "coder", "name": "Scripter"}, {"id": "x"}]"#).unwrap();
        let names = expert_names(&kit, &d);
        assert_eq!(names["ui"], "UI builder");
        assert_eq!(names["coder"], "Scripter");
        assert!(!names.contains_key("x"));
        assert!(expert_names(&d.join("nowhere"), &d.join("nowhere")).is_empty());
    }

    #[test]
    fn decide_once() {
        let d = game("once");
        put_item(&d, "aa", json!({"title": "Shop menu"}));
        let mut seen = HashMap::new();
        let text = decide(&d, "aa", true, "", &mut seen).unwrap();
        assert_eq!(text, "Review: Shop menu \u{2014} approved");
        let it = item(&d, "aa");
        assert_eq!(it["status"], "approved");
        assert!(it["decided"].as_f64().unwrap() > 1_000_000_000.0);
        assert_eq!(it["extra_key"], 7, "unknown keys are kept");
        // the app's own write is noted, so the watcher does not report it
        let file = d.join(".fridae").join("review").join("aa.json");
        assert_eq!(seen.get(&file), fs::metadata(&file).and_then(|m| m.modified()).ok().as_ref());
        assert_eq!(decide(&d, "aa", true, "", &mut seen).unwrap_err(), "Already answered.");
        assert_eq!(decide(&d, "aa", false, "redo", &mut seen).unwrap_err(), "Already answered.");
        assert_eq!(item(&d, "aa")["status"], "approved");
        assert!(decide(&d, "gone", true, "", &mut seen).is_err());
        assert!(decide(&d, "../aa", true, "", &mut seen).is_err());
        assert!(!crate::fridae_lock::lock_path(&d).exists(), "the lock is let go");
    }

    #[test]
    fn change_needs_note() {
        let d = game("note");
        put_item(&d, "aa", json!({"title": "Map"}));
        let mut seen = HashMap::new();
        assert!(decide(&d, "aa", false, "   ", &mut seen).is_err());
        assert!(decide(&d, "aa", false, &"x".repeat(601), &mut seen).is_err());
        assert_eq!(item(&d, "aa")["status"], "waiting");
        let text = decide(&d, "aa", false, "  Make the lava brighter é ", &mut seen).unwrap();
        assert_eq!(text, "Review: Map \u{2014} change: Make the lava brighter é");
        let it = item(&d, "aa");
        assert_eq!((it["status"].as_str(), it["note"].as_str()), (Some("changes"), Some("Make the lava brighter é")));
    }

    #[test]
    fn approve_all_skips_blocking() {
        let d = game("all");
        put_item(&d, "aa", json!({}));
        put_item(&d, "bb", json!({"blocking": true}));
        put_item(&d, "cc", json!({"status": "changes"}));
        put_item(&d, "dd", json!({}));
        let mut seen = HashMap::new();
        let all: Vec<String> = ["aa", "bb", "cc", "dd"].iter().map(|s| s.to_string()).collect();
        assert_eq!(approve_all(&d, &all, &mut seen).unwrap(), 2);
        assert_eq!(item(&d, "aa")["status"], "approved");
        assert_eq!(item(&d, "dd")["status"], "approved");
        assert_eq!(item(&d, "bb")["status"], "waiting");
        assert_eq!(item(&d, "cc")["status"], "changes");
        assert_eq!(approve_all(&d, &all, &mut seen).unwrap(), 0);
    }

    #[test]
    fn approve_all_approves_only_what_the_user_saw() {
        let d = game("all-seen");
        put_item(&d, "aa", json!({}));
        put_item(&d, "ee", json!({})); // filed after the page drew the list
        let mut seen = HashMap::new();
        assert_eq!(approve_all(&d, &["aa".to_string()], &mut seen).unwrap(), 1);
        assert_eq!(item(&d, "aa")["status"], "approved");
        assert_eq!(item(&d, "ee")["status"], "waiting", "an item the user has not seen waits for its own look");
        assert_eq!(approve_all(&d, &[], &mut seen).unwrap(), 0);
    }

    #[test]
    fn approving_a_newer_blocking_item_supersedes_older_changes_of_its_stage() {
        let d = game("supersede");
        put_item(&d, "old", json!({"stage": "design", "blocking": true, "status": "changes", "created": 1.0}));
        put_item(&d, "other", json!({"stage": "build", "blocking": true, "status": "changes", "created": 1.0}));
        put_item(&d, "loose", json!({"stage": "design", "blocking": false, "status": "changes", "created": 1.0}));
        put_item(&d, "new", json!({"stage": "design", "blocking": true, "created": 5.0}));
        put_item(&d, "later", json!({"stage": "design", "blocking": true, "status": "changes", "created": 9.0}));
        let mut seen = HashMap::new();
        decide(&d, "new", true, "", &mut seen).unwrap();
        let old = item(&d, "old");
        assert_eq!((old["status"].as_str(), old["superseded_by"].as_str()), (Some("superseded"), Some("new")));
        assert_eq!(old["extra_key"], 7, "unknown keys are kept");
        assert_eq!(item(&d, "other")["status"], "changes", "another stage's item is not touched");
        assert_eq!(item(&d, "loose")["status"], "changes", "only blocking items");
        assert_eq!(item(&d, "later")["status"], "changes", "only older ones");
        assert!(!crate::fridae_lock::lock_path(&d).exists());
    }

    #[test]
    fn ending_a_run_keeps_everything_and_allows_a_new_one() {
        let d = game("end");
        let mut r = todo_run("c-no-such-lead");
        r["paused"] = json!(false);
        r["note"] = json!("slots");
        r["future"] = json!({"x": 1});
        r["checkpoints"] = json!([{"stage": "start", "at": 1.0}]);
        put_run(&d, r);
        put_item(&d, "aa", json!({}));
        assert_eq!(can_start(&d).unwrap_err(), ALREADY);
        let p = d.display().to_string();
        assert_eq!(end(&p).unwrap()["ok"], true);
        let after = run(&d);
        assert_eq!(after["on"], false);
        assert!(after["ended"].as_f64().unwrap() > 1_000_000_000.0);
        assert!(after.get("note").is_none());
        assert_eq!((after["future"]["x"].as_i64(), after["checkpoints"].as_array().map(|l| l.len())), (Some(1), Some(1)), "nothing else changes");
        assert_eq!(item(&d, "aa")["status"], "waiting", "review items stay");
        assert_eq!(end(&p).unwrap_err(), NO_RUN_TO_END, "an ended run cannot be ended again");
        assert!(can_start(&d).is_ok(), "a new run may start");
        assert_eq!(write_fresh(&d, "Next", vec![], 2, "c-new").unwrap(), None);
        assert!(d.join(".fridae").join("review-archive").read_dir().unwrap().next().is_some(), "the old run's items are archived as usual");
        assert!(run(&d).get("ended").is_none());
        assert_eq!(end(&d.join("nothing").display().to_string()).unwrap_err(), NO_RUN_TO_END);
    }

    #[test]
    fn a_needs_card_done_tells_the_lead() {
        let d = game("needs");
        assert_eq!(needs_done(&d, "  ").unwrap_err(), "Which card was done?");
        assert_eq!(needs_done(&d, "Add an API key").unwrap()["told"], false, "no run");
        let p = d.display().to_string();
        let lead = agents::new_background_chat("Lead: needs", Some(p.clone())).unwrap();
        let mut r = todo_run(&lead.id);
        r["paused"] = json!(false);
        put_run(&d, r.clone());
        let res = needs_done(&d, "Add   an API key é").unwrap();
        assert_eq!(res["told"], true);
        // the Lead is working (not waiting): it gets the line with its next turn
        assert_eq!(lead.notes_for_test(), vec!["Board: Add an API key é \u{2014} done".to_string()]);
        r["paused"] = json!(true);
        put_run(&d, r);
        assert_eq!(needs_done(&d, "Other").unwrap()["told"], false, "a paused run is not woken");
        assert_eq!(lead.notes_for_test().len(), 1);
        agents::remove_agent(&lead.id);
        assert_eq!(board_done_text("X"), "Board: X \u{2014} done");
    }

    #[test]
    fn media_confined() {
        let d = game("media");
        put_item(&d, "aa", json!({"media": ["shot 1.png", "clip.mp4", "notes.txt"]}));
        let folder = d.join(".fridae").join("review").join("aa");
        fs::create_dir_all(&folder).unwrap();
        for f in ["shot 1.png", "clip.mp4", "other.png", "notes.txt"] {
            fs::write(folder.join(f), b"data").unwrap();
        }
        fs::write(d.join(".fridae").join("x"), b"secret").unwrap();
        let p = d.display().to_string();
        let known = vec![p.clone()];
        let key = project_key(&p);
        let (file, ctype) = media_in(&known, &key, "aa", "shot 1.png").unwrap();
        assert_eq!((file, ctype), (folder.join("shot 1.png"), "image/png"));
        assert_eq!(media_in(&known, &key, "aa", "clip.mp4").unwrap().1, "video/mp4");
        assert!(media_in(&known, &key, "aa", "other.png").is_none(), "not in the item's media");
        assert!(media_in(&known, &key, "aa", "notes.txt").is_none(), "not a picture or clip");
        assert!(media_in(&known, &key, "aa", "../x").is_none());
        assert!(media_in(&known, &key, "aa", "..").is_none());
        assert!(media_in(&known, &key, "../aa", "shot 1.png").is_none());
        assert!(media_in(&known, "000000000000", "aa", "shot 1.png").is_none(), "unknown key");
        assert!(media_in(&known, &key, "zz", "shot 1.png").is_none(), "unknown item");
        assert!(media_in(&[], &key, "aa", "shot 1.png").is_none(), "not a known project");
    }

    #[test]
    fn settings_validated() {
        assert_eq!(settings_from(&json!({"gates": ["launch", "design", "design"], "max_experts": 3})).unwrap(), (Some(vec!["design".into(), "launch".into()]), Some(3)));
        assert_eq!(settings_from(&json!({"gates": []})).unwrap(), (Some(vec![]), None));
        assert_eq!(settings_from(&json!({})).unwrap(), (None, None));
        assert!(settings_from(&json!({"gates": ["design", "ship"]})).is_err());
        assert!(settings_from(&json!({"gates": "design"})).is_err());
        assert!(settings_from(&json!({"max_experts": 0})).is_err());
        assert!(settings_from(&json!({"max_experts": 5})).is_err());
        assert!(settings_from(&json!({"max_experts": 2.5})).is_err());
        assert!(settings_from(&json!({"max_experts": "2"})).is_err());
    }

    #[test]
    fn project_key_stable() {
        assert_eq!(project_key("abc"), "a9993e364706"); // SHA-1 of "abc" starts a9993e364706816a
        let p = "/Users/me/Games/My Gäme é";
        assert_eq!(project_key(p), project_key(p));
        assert_eq!(project_key(p).len(), 12);
        assert!(project_key(p).chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(project_key(p), project_key("/Users/me/Games/Other"));
        // a long input crosses SHA-1's 64-byte blocks
        assert_eq!(project_key(&"a".repeat(1000)), "291e9a6c6699");
    }

    #[test]
    fn start_refuses_running_run() {
        let d = game("start");
        assert!(can_start(&d).is_ok(), "no run yet");
        assert!(can_start(&d.join("missing")).is_err());
        let mut stages: Vec<Value> = STAGES.iter().map(|s| json!({"id": s, "status": "done"})).collect();
        stages[3]["status"] = json!("doing");
        put_run(&d, json!({"on": true, "stages": stages}));
        assert_eq!(can_start(&d).unwrap_err(), "This game already has a studio run. Resume it from Review.");
        stages[3]["status"] = json!("done");
        put_run(&d, json!({"on": true, "stages": stages}));
        assert!(can_start(&d).is_ok(), "a finished run may be followed by a new one");
        put_run(&d, json!({"on": false, "stages": []}));
        assert!(can_start(&d).is_ok());
    }

    fn todo_run(lead: &str) -> Value {
        json!({"on": true, "lead": lead, "paused": true, "stages": STAGES.iter().map(|s| json!({"id": s, "status": "todo"})).collect::<Vec<_>>()})
    }

    fn done_run(started: f64) -> Value {
        json!({"on": true, "started": started, "lead": "c-old", "stages": STAGES.iter().map(|s| json!({"id": s, "status": "done"})).collect::<Vec<_>>()})
    }

    #[test]
    fn resume_claim_is_compare_and_set() {
        let d = game("claim");
        put_run(&d, todo_run("c-old"));
        assert_eq!(claim_lead(&d, "c-old", Some("c-new1")).unwrap(), Claim::Ours);
        assert_eq!((run(&d)["lead"].as_str(), run(&d)["paused"].as_bool()), (Some("c-new1"), Some(false)));
        // a second Resume that read the old Lead finds the run already adopted and changes nothing
        assert_eq!(claim_lead(&d, "c-old", Some("c-new2")).unwrap(), Claim::Taken("c-new1".into()));
        assert_eq!(run(&d)["lead"], "c-new1");
        // waking the live Lead keeps it
        assert_eq!(claim_lead(&d, "c-new1", None).unwrap(), Claim::Ours);
        assert_eq!(run(&d)["lead"], "c-new1");
    }

    #[test]
    fn resume_refuses_finished_or_off_run() {
        let d = game("resume-off");
        let p = d.display().to_string();
        assert_eq!(resume(&p).unwrap_err(), NO_RUN, "no run");
        put_run(&d, done_run(5.0));
        assert_eq!(resume(&p).unwrap_err(), NO_RUN, "finished");
        assert_eq!(claim_lead(&d, "c-old", Some("c-x")).unwrap_err(), NO_RUN);
        let mut off = todo_run("c-old");
        off["on"] = json!(false);
        put_run(&d, off);
        assert_eq!(resume(&p).unwrap_err(), NO_RUN, "off");
        assert!(!agents::agents().iter().any(|a| a.project().as_deref() == Some(p.as_str())), "no chat was made");
    }

    #[test]
    fn start_lead_takes_only_an_idle_chat_of_this_game() {
        let d = game("startlead");
        let other = game("startlead-other");
        let p = d.display().to_string();
        let mine = agents::new_background_chat("Mine", Some(p.clone())).unwrap();
        let theirs = agents::new_background_chat("Theirs", Some(other.display().to_string())).unwrap();
        let helper = agents::spawn_helper(None, "worker", "Helper", Some(p.clone()), None, None).unwrap();
        let (chat, made) = start_lead(&p, Some(&mine.id)).unwrap();
        assert_eq!((chat.id.as_str(), made), (mine.id.as_str(), false));
        let why = |id: &str| start_lead(&p, Some(id)).err();
        assert_eq!(why(&theirs.id).as_deref(), Some("That chat belongs to another game."));
        assert_eq!(why(&helper.id).as_deref(), Some("Only a chat of your own can lead a studio run."));
        assert_eq!(why("c-no-such-chat").as_deref(), Some("That chat is gone."));
        for a in [&mine, &theirs, &helper] {
            agents::remove_agent(&a.id);
        }
    }

    #[test]
    fn double_start_refused_and_leaves_no_chat() {
        let d = game("double");
        let p = d.display().to_string();
        assert_eq!(write_fresh(&d, "Lava obby", vec!["design".into()], 2, "c-first").unwrap(), None);
        assert_eq!(write_fresh(&d, "Other", vec![], 3, "c-second").unwrap_err(), ALREADY);
        let r = run(&d);
        assert_eq!((r["lead"].as_str(), r["idea"].as_str()), (Some("c-first"), Some("Lava obby")));
        // the route refuses before making a chat
        assert_eq!(start(&p, &json!({"idea": "x"})).unwrap_err(), ALREADY);
        assert!(!agents::agents().iter().any(|a| a.project().as_deref() == Some(p.as_str())));
        // a chat made for a start that lost the race is closed again
        let lead = agents::new_background_chat("Lead: race", Some(p.clone())).unwrap();
        assert_eq!(finish_start(&d, &lead, true, "x", vec![], 2).unwrap_err(), ALREADY);
        assert!(agents::get(&lead.id).is_none());
        assert_eq!(run(&d)["lead"], "c-first");
    }

    #[test]
    fn new_run_archives_old_review_items() {
        let d = game("archive");
        let p = d.display().to_string();
        put_run(&d, done_run(1234.5));
        put_item(&d, "aa", json!({"status": "approved", "media": ["shot.png"]}));
        put_item(&d, "bb", json!({}));
        let media = d.join(".fridae").join("review").join("aa");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("shot.png"), b"png").unwrap();
        let (mut seen, mut looked) = (HashMap::new(), HashSet::new());
        changes(&[p.clone()], &mut seen, &mut looked);
        assert_eq!(write_fresh(&d, "Next game", vec![], 2, "c-new").unwrap(), None);
        assert_eq!(fs::read_dir(d.join(".fridae").join("review")).unwrap().count(), 0);
        let arch = d.join(".fridae").join("review-archive").join("1234");
        assert!(arch.join("aa.json").is_file() && arch.join("bb.json").is_file());
        assert_eq!(fs::read(arch.join("aa").join("shot.png")).unwrap(), b"png");
        let r = run(&d);
        assert_eq!((r["lead"].as_str(), r["idea"].as_str(), r["on"].as_bool()), (Some("c-new"), Some("Next game"), Some(true)));
        assert!(r["stages"].as_array().unwrap().iter().all(|s| s["status"] == "todo"));
        assert_eq!(r["read_decisions"], json!([]));
        assert!(items(&d).is_empty());
        // the move is not news for the Review tab: only the new run is reported
        std::thread::sleep(std::time::Duration::from_millis(20));
        let ev = changes(&[p], &mut seen, &mut looked);
        assert!(ev.iter().all(|(k, _)| *k == "studio_run"), "{ev:?}");
        // a second archive with the same stamp gets its own folder
        put_item(&d, "cc", json!({}));
        assert!(archive_review(&d, 1234).is_empty());
        assert!(d.join(".fridae").join("review-archive").join("1234-2").join("cc.json").is_file());
    }

    #[test]
    fn change_run_keeps_unknown_keys() {
        let d = game("change");
        put_run(&d, json!({"on": true, "stage": "build", "future": {"x": 1}}));
        change_run(&d, |r| {
            r["paused"] = json!(true);
            (true, ())
        })
        .unwrap();
        let r = run(&d);
        assert_eq!((r["paused"].as_bool(), r["future"]["x"].as_i64(), r["stage"].as_str()), (Some(true), Some(1), Some("build")));
        assert!(!crate::fridae_lock::lock_path(&d).exists());
        let leftovers = fs::read_dir(d.join(".fridae")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn no_live_lead_pauses_run() {
        let d = game("lead");
        put_run(&d, json!({"on": true, "lead": "c-no-such-chat", "paused": false}));
        assert!(!notify_lead(&d, "Review: x \u{2014} approved"));
        assert_eq!(run(&d)["paused"], true);
    }

    #[test]
    fn watcher_reports_changes_not_own_writes() {
        let d = game("watch");
        let p = d.display().to_string();
        put_item(&d, "aa", json!({}));
        let (mut seen, mut looked) = (HashMap::new(), HashSet::new());
        assert!(changes(&[p.clone()], &mut seen, &mut looked).is_empty(), "what is there at the start is not reported");
        std::thread::sleep(std::time::Duration::from_millis(20));
        put_item(&d, "bb", json!({"title": "New"}));
        put_run(&d, json!({"on": true, "stage": "design"}));
        fs::write(d.join(".fridae").join("review").join("cc.json"), "{\"id\": ").unwrap(); // half written
        let ev = changes(&[p.clone()], &mut seen, &mut looked);
        assert_eq!(ev.len(), 2);
        let review = ev.iter().find(|(k, _)| *k == "review").unwrap();
        assert_eq!((review.1["project"].as_str(), review.1["item"]["title"].as_str()), (Some(p.as_str()), Some("New")));
        let run_ev = ev.iter().find(|(k, _)| *k == "studio_run").unwrap();
        assert_eq!(run_ev.1["run"]["stage"], "design");
        // the app's own decision is not reported; the half-written file is, once it is whole
        decide(&d, "aa", true, "", &mut seen).unwrap();
        put_item(&d, "cc", json!({}));
        let ev = changes(&[p.clone()], &mut seen, &mut looked);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].1["item"]["id"], "cc");
        assert!(changes(&[p], &mut seen, &mut looked).is_empty());
    }
}
