//! Each game's board: what is being worked on, what needs the user, what is done.
//!
//! Kept in `<project>/.fridae/board.json`, the same file the agents' `rbx_board` tool (rbxboard.py) writes, so it
//! travels with the game. Besides the cards agents and the user add, the app keeps one card per chat up to date by
//! itself: it moves to "doing" when the chat starts working, and to "done" or "needs" when it stops. The window
//! shows each working card's current step live.

use std::{fs, path::PathBuf, sync::Mutex};

use serde_json::{Value, json};

use crate::paths;

pub const COLUMNS: [&str; 4] = ["todo", "doing", "needs", "done"];
/// Reads and writes of the board file from this app go one at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn file(project: &str) -> PathBuf {
    PathBuf::from(project).join(".fridae").join("board.json")
}

fn load(project: &str) -> Value {
    fs::read_to_string(file(project))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|v| v["cards"].is_array())
        .unwrap_or_else(|| json!({"cards": []}))
}

fn save(project: &str, data: &Value) {
    let path = file(project);
    let _ = fs::create_dir_all(path.parent().unwrap());
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_string_pretty(data).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// Change the board under the lock, saving it afterwards.
fn change<T>(project: &str, f: impl FnOnce(&mut Vec<Value>) -> T) -> T {
    let _guard = LOCK.lock().unwrap();
    let mut data = load(project);
    let out = f(data["cards"].as_array_mut().unwrap());
    save(project, &data);
    out
}

/// The whole board of a project, for the window.
pub fn get(project: &str) -> Value {
    if !std::path::Path::new(project).is_dir() {
        return json!({"ok": false, "cards": []});
    }
    let _guard = LOCK.lock().unwrap();
    let mut b = load(project);
    b["ok"] = json!(true);
    b
}

/// A card's new position goes to the end of the list, so each column reads in the order things happened.
fn put_last(cards: &mut Vec<Value>, i: usize) -> usize {
    let c = cards.remove(i);
    cards.push(c);
    cards.len() - 1
}

/// What the user does on the board: add, move, edit or remove a card.
pub fn act(project: &str, body: &Value) -> Value {
    if !std::path::Path::new(project).is_dir() {
        return json!({"ok": false});
    }
    let text = |k: &str, max: usize| body[k].as_str().map(|s| s.trim().chars().take(max).collect::<String>());
    let column = body["column"].as_str().filter(|c| COLUMNS.contains(c)).map(str::to_string);
    let id = body["id"].as_str().unwrap_or("").to_string();
    let now = paths::now_secs();
    let ok = change(project, |cards| match body["action"].as_str().unwrap_or("") {
        "add" => {
            let Some(title) = text("title", 120).filter(|t| !t.is_empty()) else { return false };
            cards.push(json!({"id": paths::random_hex(1)[..6].to_string(), "title": title, "note": text("note", 600).unwrap_or_default(),
                "column": column.unwrap_or_else(|| "todo".into()), "by": "you", "created": now, "updated": now}));
            true
        }
        "move" | "edit" => {
            let Some(i) = cards.iter().position(|c| c["id"] == id.as_str()) else { return false };
            let i = if column.is_some() { put_last(cards, i) } else { i };
            let c = &mut cards[i];
            if let Some(col) = column {
                c["column"] = json!(col);
            }
            if let Some(t) = text("title", 120).filter(|t| !t.is_empty()) {
                c["title"] = json!(t);
            }
            if let Some(n) = text("note", 600) {
                c["note"] = json!(n);
            }
            c["updated"] = json!(now);
            true
        }
        "remove" => {
            let before = cards.len();
            cards.retain(|c| c["id"] != id.as_str());
            cards.len() < before
        }
        _ => false,
    });
    json!({"ok": ok, "board": get(project)})
}

/// The app's own card for a chat: created or updated, and moved to `column`.
pub fn track(project: &str, chat: &str, chat_name: &str, column: &str, title: Option<&str>, note: Option<&str>) {
    if !std::path::Path::new(project).is_dir() {
        return;
    }
    let now = paths::now_secs();
    change(project, |cards| {
        let found = cards.iter().position(|c| c["auto"] == chat);
        let i = match found {
            Some(i) => put_last(cards, i),
            None => {
                cards.push(json!({"id": paths::random_hex(1)[..6].to_string(), "auto": chat, "chat": chat, "by": chat_name, "created": now, "title": chat_name, "note": ""}));
                cards.len() - 1
            }
        };
        let c = &mut cards[i];
        c["column"] = json!(column);
        c["by"] = json!(chat_name);
        c["updated"] = json!(now);
        if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
            c["title"] = json!(t.chars().take(120).collect::<String>());
        }
        if let Some(n) = note {
            c["note"] = json!(n.trim().chars().take(600).collect::<String>());
        }
    });
}

/// The first sentence or line of a message, short enough for a card.
pub fn gist(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('[')).unwrap_or("");
    let cut = line.find(". ").map(|i| &line[..i]).unwrap_or(line);
    let mut s: String = cut.trim_end_matches('.').chars().take(max).collect();
    if cut.chars().count() > max {
        s.push('\u{2026}');
    }
    s
}
