//! Agents starting other agents. A tool writes a request file; this picks it up,
//! starts a worker or a reviewer in its own chat, and writes the outcome back.
//!
//!   work/team-requests/<id>.json   {"id", "from", "role", "name", "task"}
//!   work/team-results/<id>.json    {"id", "agent", "name", "role", "status", "reason", "text"}
//!
//! Studio mode adds a named expert: the request also has "expert", "expert_name", "expert_text" and "writes"
//! (the tool read them from the roster; this never does), and the result "expert". Only one expert that changes the
//! game ("writes") runs per project at a time.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::Mutex,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    paths, studio_run,
};

fn write_result(id: &str, value: Value) {
    let dir = paths::work("team-results");
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join(format!("{id}.tmp"));
    if fs::write(&tmp, value.to_string()).is_ok() {
        let _ = fs::rename(&tmp, dir.join(format!("{id}.json")));
    }
}

/// The writer places in use: project folder -> the title of the helper holding it. In memory only: it starts empty,
/// as it should, because no helper survives a restart.
static WRITERS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn with_writers<T>(f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
    f(WRITERS.lock().unwrap().get_or_insert_with(HashMap::new))
}

/// Take the project's one writer place for `agent_name`, or say who has it. Experts that only read or review never need it.
fn writer_gate(held: &mut HashMap<String, String>, project: &str, agent_name: &str, writes: bool) -> Result<(), String> {
    if !writes {
        return Ok(());
    }
    if let Some(who) = held.get(project) {
        return Err(format!("Another expert is changing the game right now ({who}). Wait for it, then start this one."));
    }
    held.insert(project.to_string(), agent_name.to_string());
    Ok(())
}

/// A helper that held the writer place lets go of it (only once, however many ways its chat ends).
pub fn release(agent: &Agent) {
    if let Some(project) = agent.take_writer() {
        with_writers(|w| w.remove(&project));
    }
}

/// "Coder: Shop menu", cut to the 34 characters every chat title gets.
fn expert_title(expert_name: &str, name: &str) -> String {
    format!("{expert_name}: {name}").chars().take(34).collect()
}

/// Studio's run file carries a note the page shows ("slots": waiting for a free agent slot). Only touches a run that exists.
fn set_slots_note(project: &str, on: bool) {
    let dir = Path::new(project);
    if !studio_run::run_file(dir).is_file() {
        return;
    }
    let _ = studio_run::change_run(dir, |run| {
        if !run.is_object() {
            return (false, ());
        }
        if on {
            if run["note"] == "slots" {
                return (false, ());
            }
            run["note"] = json!("slots");
        } else if run["note"] == "slots" {
            run.as_object_mut().map(|o| o.remove("note"));
        } else {
            return (false, ());
        }
        (true, ())
    });
}

fn start(req: &Value) {
    let id = paths::base_name(req["id"].as_str().unwrap_or(""));
    let role = if req["role"] == "critic" { "critic" } else { "worker" };
    let name = req["name"].as_str().unwrap_or("").trim().to_string();
    let task = req["task"].as_str().unwrap_or("").trim().to_string();
    let expert = req["expert"].as_str().map(str::trim).filter(|e| !e.is_empty()).map(str::to_string);
    let with_expert = |mut v: Value| {
        if let Some(e) = &expert {
            v["expert"] = json!(e);
        }
        v
    };
    let refuse = |reason: &str| write_result(&id, with_expert(json!({"id": id, "name": name, "role": role, "status": "refused", "reason": reason})));
    if id.is_empty() || task.is_empty() {
        return refuse("The request had no task.");
    }
    let from = agents::get(req["from"].as_str().unwrap_or(""));
    if from.as_ref().is_some_and(|a| a.team().is_some()) {
        return refuse("An agent that was started by another agent may not start more.");
    }
    let project = from.as_ref().and_then(|a| a.project());
    let ai = from.as_ref().map(|a| a.ai());
    let parent = from.as_ref().map(|a| a.id.as_str());
    let agent = if let Some(expert_id) = &expert {
        let expert_name = req["expert_name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(expert_id);
        let title = expert_title(expert_name, &name);
        let key = project.clone().unwrap_or_default();
        let writes = req["writes"].as_bool().unwrap_or(false);
        if let Err(why) = with_writers(|w| writer_gate(w, &key, &title, writes)) {
            return refuse(&why); // a refused request never takes the place
        }
        let info = agents::Expert { id: expert_id.clone(), name: expert_name.to_string(), text: req["expert_text"].as_str().unwrap_or("").to_string() };
        let Some(agent) = agents::spawn_expert(&id, role, &title, project.clone(), ai, parent, info) else {
            if writes {
                with_writers(|w| w.remove(&key));
            }
            if let Some(p) = &project {
                set_slots_note(p, true);
            }
            return refuse("All agent slots are in use. Wait for one to finish, or close a chat.");
        };
        if writes {
            agent.set_writer(&key);
        }
        if let Some(p) = &project {
            set_slots_note(p, false);
        }
        agent
    } else {
        let title = if role == "critic" && !name.to_lowercase().starts_with("review") { format!("Review: {name}") } else { name.clone() };
        let Some(agent) = agents::spawn_helper(Some(&id), role, &title, project, ai, parent) else {
            return refuse("All agent slots are in use. Wait for one to finish, or close a chat.");
        };
        agent
    };
    write_result(&id, with_expert(json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "running"})));
    thread::spawn(move || agent.send(&task, &agents::Ask::default()));
}

/// Called when an agent's turn ends: hand its reply back if another agent is waiting for it.
pub fn finished(agent: &Agent) {
    release(agent);
    if let Some((id, role)) = agent.team() {
        let text = agent.last_reply();
        let text = if text.is_empty() { "The agent stopped without a reply.".to_string() } else { text };
        let mut result = json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "done", "text": text});
        if let Some(e) = agent.expert() {
            result["expert"] = json!(e);
        }
        write_result(&id, result);
    }
}

/// Requests left over from a previous run can never be answered: clear them once at start, then watch.
pub fn watch() {
    let dir = paths::work("team-requests");
    let _ = fs::remove_dir_all(&dir);
    loop {
        thread::sleep(Duration::from_millis(700));
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let req = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let _ = fs::remove_file(&path);
            if let Some(req) = req {
                start(&req);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_writer_is_refused() {
        let mut held = HashMap::new();
        assert!(writer_gate(&mut held, "/g/one", "Coder: Shop", true).is_ok());
        let why = writer_gate(&mut held, "/g/one", "World builder: Map", true).unwrap_err();
        assert_eq!(why, "Another expert is changing the game right now (Coder: Shop). Wait for it, then start this one.");
        // readers are never held up, another game is separate, and a refusal took nothing
        assert!(writer_gate(&mut held, "/g/one", "Security reviewer: Remotes", false).is_ok());
        assert!(writer_gate(&mut held, "/g/two", "World builder: Map", true).is_ok());
        assert_eq!(held.get("/g/one").map(String::as_str), Some("Coder: Shop"));
    }

    #[test]
    fn writer_lock_released_on_finish() {
        // `release` frees the place the helper holds, once; a second call must not free a later holder's
        let a = agents::new_agent(None).unwrap();
        let key = "/g/release-test".to_string();
        with_writers(|w| writer_gate(w, &key, "Coder: A", true)).unwrap();
        a.set_writer(&key);
        finished(&a);
        assert!(with_writers(|w| writer_gate(w, &key, "Coder: B", true)).is_ok());
        release(&a);
        assert_eq!(with_writers(|w| w.get(&key).cloned()).as_deref(), Some("Coder: B"));
        with_writers(|w| w.remove(&key));
        agents::remove_agent(&a.id);
    }

    #[test]
    fn expert_title_is_name_then_task_and_cut() {
        assert_eq!(expert_title("Coder", "Shop menu"), "Coder: Shop menu");
        let long = expert_title("Security reviewer", "Check every remote event in the whole game");
        assert_eq!(long.chars().count(), 34);
        assert!(long.starts_with("Security reviewer: Check every"));
        assert_eq!(expert_title("Ui", "é".repeat(40).as_str()).chars().count(), 34);
    }

    fn game(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fridae-team-{name}-{}", std::process::id())).join("My Gäme");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".fridae")).unwrap();
        d
    }

    fn read(d: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(d.join(".fridae").join("studio.json")).unwrap()).unwrap()
    }

    #[test]
    fn slots_refusal_sets_note() {
        let d = game("note");
        let p = d.to_str().unwrap();
        // no run file: nothing is created
        set_slots_note(p, true);
        assert!(!d.join(".fridae").join("studio.json").exists());
        fs::write(d.join(".fridae").join("studio.json"), r#"{"on": true, "stage": "build"}"#).unwrap();
        set_slots_note(p, true);
        let run = read(&d);
        assert_eq!(run["note"], "slots");
        assert_eq!(run["stage"], "build");
        assert!(!crate::fridae_lock::lock_path(&d).exists(), "the lock is let go");
        set_slots_note(p, false);
        let run = read(&d);
        assert!(run.get("note").is_none());
        assert_eq!(run["on"], true);
        // another note is not ours to clear
        fs::write(d.join(".fridae").join("studio.json"), r#"{"note": "other"}"#).unwrap();
        set_slots_note(p, false);
        assert_eq!(read(&d)["note"], "other");
    }
}
