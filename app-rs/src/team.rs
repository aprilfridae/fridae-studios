//! Agents starting other agents. A tool writes a request file; this picks it up,
//! starts a worker or a reviewer in its own chat, and writes the outcome back.
//!
//!   work/team-requests/<id>.json   {"id", "from", "role", "name", "task"}
//!   work/team-results/<id>.json    {"id", "agent", "name", "role", "status", "reason", "text"}

use std::{fs, thread, time::Duration};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    paths,
};

fn write_result(id: &str, value: Value) {
    let dir = paths::work("team-results");
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join(format!("{id}.tmp"));
    if fs::write(&tmp, value.to_string()).is_ok() {
        let _ = fs::rename(&tmp, dir.join(format!("{id}.json")));
    }
}

fn start(req: &Value) {
    let id = paths::base_name(req["id"].as_str().unwrap_or(""));
    let role = if req["role"] == "critic" { "critic" } else { "worker" };
    let name = req["name"].as_str().unwrap_or("").trim().to_string();
    let task = req["task"].as_str().unwrap_or("").trim().to_string();
    let refuse = |reason: &str| write_result(&id, json!({"id": id, "name": name, "role": role, "status": "refused", "reason": reason}));
    if id.is_empty() || task.is_empty() {
        return refuse("The request had no task.");
    }
    let from = agents::get(req["from"].as_str().unwrap_or(""));
    if from.as_ref().is_some_and(|a| a.team().is_some()) {
        return refuse("An agent that was started by another agent may not start more.");
    }
    let project = from.as_ref().and_then(|a| a.project());
    let title = if role == "critic" && !name.to_lowercase().starts_with("review") { format!("Review: {name}") } else { name.clone() };
    let Some(agent) = agents::spawn_helper(Some(&id), role, &title, project, from.as_ref().map(|a| a.ai()), from.as_ref().map(|a| a.id.as_str())) else {
        return refuse("All agent slots are in use. Wait for one to finish, or close a chat.");
    };
    write_result(&id, json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "running"}));
    thread::spawn(move || agent.send(&task, &agents::Ask::default()));
}

/// Called when an agent's turn ends: hand its reply back if another agent is waiting for it.
pub fn finished(agent: &Agent) {
    if let Some((id, role)) = agent.team() {
        let text = agent.last_reply();
        let text = if text.is_empty() { "The agent stopped without a reply.".to_string() } else { text };
        write_result(&id, json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "done", "text": text}));
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
