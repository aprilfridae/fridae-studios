//! The New project page's Ideas: a snapshot of Roblox's public charts, and an agent that
//! turns it into game ideas backed by those numbers.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    paths,
};

static REFRESHING: AtomicBool = AtomicBool::new(false);

fn dir() -> PathBuf {
    paths::work("research")
}

fn read(name: &str) -> Value {
    fs::read_to_string(dir().join(name)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

/// Fetch today's charts with the research tool. Takes a few seconds; one at a time.
pub fn refresh() {
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = fs::create_dir_all(dir());
    let out = dir().join("market.json");
    let result = paths::cli(&["research", "snapshot", "--out", &out.display().to_string()]).output();
    let problem = match result {
        Ok(o) if o.status.success() => String::new(),
        Ok(o) => String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect(),
        Err(e) => format!("Could not run the research tool: {e}"),
    };
    let _ = fs::write(dir().join("market-error.txt"), problem);
    REFRESHING.store(false, Ordering::SeqCst);
}

/// The chat that is researching right now, if there is one.
fn running() -> Option<String> {
    agents::agents().iter().find(|a| a.role().as_deref() == Some("research") && a.busy()).map(|a| a.id.clone())
}

/// Everything the Ideas list shows.
pub fn state() -> Value {
    let error = fs::read_to_string(dir().join("market-error.txt")).unwrap_or_default();
    json!({
        "market": read("market.json"), "refreshing": REFRESHING.load(Ordering::SeqCst), "error": error,
        "ideas": read("ideas.json"), "researching": running(),
    })
}

/// Start the research agent in a chat of its own.
pub fn find_ideas(focus: &str, ai: Option<&str>, model: Option<&str>) -> Value {
    if let Some(id) = running() {
        return json!({"ok": true, "agent": id});
    }
    let focus = focus.trim();
    let name = if focus.is_empty() { "Research: game ideas".to_string() } else { format!("Research: {focus}") };
    let Some(agent) = agents::spawn_helper(None, "research", &name, None, ai.filter(|a| paths::AIS.contains(a)).map(str::to_string), None) else {
        return json!({"ok": false, "error": "All eight chats are in use. Close one, then try again."});
    };
    let task = if focus.is_empty() {
        "Research the Roblox market as it is right now and propose five game ideas.".to_string()
    } else {
        format!("Research the Roblox market as it is right now and propose five game ideas. My steer: {focus}")
    };
    let model = model.map(str::to_string);
    let id = agent.id.clone();
    thread::spawn(move || agent.send(&task, &agents::Ask { model, ..Default::default() }));
    json!({"ok": true, "agent": id})
}

/// Keep the research agent's answer, so the tab still shows it after a restart.
pub fn finished(agent: &Agent) {
    if agent.role().as_deref() != Some("research") {
        return;
    }
    let text = agent.last_reply();
    if text.contains("\"ideas\"") {
        let _ = fs::create_dir_all(dir());
        let saved = json!({"when": paths::now_secs(), "name": agent.name(), "agent": agent.id, "text": text});
        let _ = fs::write(dir().join("ideas.json"), saved.to_string());
    }
}
