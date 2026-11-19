//! The link to the Fridae Studios plugin inside Roblox Studio.
//!
//! The plugin (studio/plugin/FridaeStudios.lua) calls this app on a fixed local
//! port about once a second. It reports what is selected and any errors in the
//! output, and collects the few fixed requests the app may make of it (undoing
//! an agent's turn). The secret it sends was written into the plugin file when
//! the app installed it.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tiny_http::{Method, Server};

use crate::{
    agents,
    paths::{self, PATHS},
    server::{body_json, forbidden, head, is_local, send_json},
};

pub const PORT: u16 = 34931;
const FILE_NAME: &str = "FridaeStudios.rbxmx";
/// The plugin's own version. Raise it only when studio/plugin/FridaeStudios.lua changes: that is what asks
/// people to update the plugin and restart Studio, so it must not move with every app release.
const PLUGIN_VERSION: &str = "2.7.3";

#[derive(Default)]
struct Live {
    listening: bool,
    seen: HashMap<String, f64>,
    place: Value,
    version: String,
    selection: Value,
    logs: Vec<Value>,
    seq: u64,
    commands: Vec<Value>,
    next_id: u64,
    results: HashMap<u64, Value>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static ANSWERED: Condvar = Condvar::new();

fn with<T>(f: impl FnOnce(&mut Live) -> T) -> T {
    f(LIVE.lock().unwrap().get_or_insert_with(Live::default))
}

fn token() -> String {
    paths::load_config().get("plugin_token").and_then(Value::as_str).unwrap_or("").to_string()
}

fn plugin_file() -> PathBuf {
    if cfg!(target_os = "macos") {
        return paths::home_dir().join("Documents").join("Roblox").join("Plugins").join(FILE_NAME);
    }
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Roblox").join("Plugins").join(FILE_NAME)
}

fn installed_version() -> Option<String> {
    let text = fs::read_to_string(plugin_file()).ok()?;
    let rest = text.split("local VERSION = \"").nth(1)?;
    Some(rest.split('"').next()?.to_string())
}

/// Studio's edit session has spoken to us in the last few seconds.
pub fn connected() -> bool {
    with(|l| l.seen.get("Edit").is_some_and(|t| paths::now_secs() - t < 6.0))
}

/// What the window shows about the plugin.
pub fn status() -> Value {
    let installed = installed_version();
    let (listening, selection, place, running, errors) = with(|l| {
        let recent = l.logs.iter().filter(|e| e["level"] == "error" && paths::now_secs() - e["t"].as_f64().unwrap_or(0.0) < 300.0).count();
        (l.listening, l.selection.clone(), l.place.clone(), l.version.clone(), recent)
    });
    let live = connected();
    json!({
        "installed": installed.is_some(), "installed_version": installed, "current": installed.as_deref() == Some(PLUGIN_VERSION), "wanted_version": PLUGIN_VERSION,
        "listening": listening, "connected": live, "running_version": if live { running } else { String::new() },
        "place": if live { place } else { Value::Null }, "selection": if live { selection } else { Value::Null }, "recent_errors": errors,
        "studio_running": !live && installed.is_some() && studio_running(),
    })
}

fn studio_running() -> bool {
    // Studio writes this launcher when its connector is on; whether Studio itself is open matters only for the hint we show
    if !cfg!(windows) {
        return std::process::Command::new("pgrep").args(["-x", "RobloxStudio"]).output().map(|o| o.status.success()).unwrap_or(false);
    }
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/FI", "IMAGENAME eq RobloxStudioBeta.exe", "/NH"]);
    paths::quiet(&mut cmd);
    cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).contains("RobloxStudioBeta")).unwrap_or(false)
}

/// Put the plugin into Studio's local plugins folder. Studio loads it the next time it starts.
pub fn install() -> Result<String, String> {
    let source = fs::read_to_string(PATHS.kit.join("studio").join("plugin").join("FridaeStudios.lua")).map_err(|e| format!("The plugin file is missing from the program: {e}"))?;
    let mut secret = token();
    if secret.len() < 32 {
        secret = paths::random_hex(3);
        let mut cfg = paths::load_config();
        cfg.insert("plugin_token".into(), json!(secret));
        paths::save_config(&cfg);
    }
    let source = source.replace("{{TOKEN}}", &secret).replace("{{PORT}}", &PORT.to_string()).replace("{{VERSION}}", PLUGIN_VERSION);
    if source.contains("]]>") {
        return Err("The plugin source cannot be packaged.".into());
    }
    let model = format!(
        "<roblox version=\"4\">\n<Item class=\"Script\" referent=\"RBX0\">\n<Properties>\n<string name=\"Name\">FridaeStudios</string>\n<ProtectedString name=\"Source\"><![CDATA[{source}]]></ProtectedString>\n</Properties>\n</Item>\n</roblox>\n"
    );
    let path = plugin_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create Studio's plugins folder: {e}"))?;
    }
    fs::write(&path, model).map_err(|e| format!("Could not write the plugin: {e}"))?;
    Ok(path.display().to_string())
}

pub fn remove() -> bool {
    fs::remove_file(plugin_file()).is_ok()
}

/// A note for the agent about Studio right now: the selection, and errors since `after`.
/// Returns the note and the newest log number it covered.
pub fn context_note(after: u64) -> (String, u64) {
    if !connected() {
        return (String::new(), after);
    }
    with(|l| {
        let mut lines: Vec<String> = Vec::new();
        let count = l.selection["count"].as_u64().unwrap_or(0);
        if count > 0 {
            lines.push(format!("Selected in Studio right now ({count}):"));
            for item in l.selection["items"].as_array().into_iter().flatten() {
                let detail = item["detail"].as_str().map(|d| format!(", {d}")).unwrap_or_default();
                lines.push(format!("- {} ({}{detail})", item["path"].as_str().unwrap_or(""), item["class"].as_str().unwrap_or("")));
            }
            if count > 12 {
                lines.push(format!("- and {} more", count - 12));
            }
        } else {
            lines.push("Nothing is selected in Studio right now.".to_string());
        }
        let fresh: Vec<&Value> = l.logs.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > after).collect();
        if !fresh.is_empty() {
            lines.push(format!("New in Studio's output since your last turn ({} lines, newest last):", fresh.len()));
            for e in fresh.iter().rev().take(15).rev() {
                lines.push(format!("- [{} {}] {}", e["dm"].as_str().unwrap_or(""), e["level"].as_str().unwrap_or(""), e["text"].as_str().unwrap_or("")));
            }
        }
        (format!("\n\n[Studio right now, from the Fridae plugin. {}]", lines.join("\n")), l.seq)
    })
}

/// Tell the plugin a turn is beginning, so it remembers every script's text as it is now.
pub fn begin(since: f64) {
    if connected() {
        with(|l| {
            l.next_id += 1;
            let id = l.next_id;
            l.commands.push(json!({"id": id, "kind": "begin", "since": since}));
        });
    }
}

/// Tell the plugin an overnight run is beginning: it keeps this snapshot of every script until the next night.
pub fn pin(since: f64) {
    if connected() {
        with(|l| {
            l.next_id += 1;
            let id = l.next_id;
            l.commands.push(json!({"id": id, "kind": "begin", "since": since, "pin": true}));
        });
    }
}

/// Ask the plugin to undo the agent's steps made since a moment in time.
pub fn revert(since: f64) -> Value {
    revert_within(since, 40)
}

/// The same, waiting up to `secs` for Studio (undoing a whole night can take minutes).
pub fn revert_within(since: f64, secs: u64) -> Value {
    if !connected() {
        return json!({"ok": false, "text": "Studio is not connected. Open Studio with the Fridae plugin installed."});
    }
    let id = with(|l| {
        l.next_id += 1;
        let id = l.next_id;
        l.commands.push(json!({"id": id, "kind": "revert", "since": since}));
        id
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut guard = LIVE.lock().unwrap();
    loop {
        if let Some(r) = guard.as_mut().and_then(|l| l.results.remove(&id)) {
            let undone = r["result"]["undone"].as_u64().unwrap_or(0);
            let scripts = r["result"]["scripts"].as_u64().unwrap_or(0);
            let put_back = match scripts {
                0 => String::new(),
                1 => " 1 script was put back as it was.".to_string(),
                n => format!(" {n} scripts were put back as they were."),
            };
            let text = match (undone, r["result"]["blocked"].as_str()) {
                (0, Some(top)) => format!("Nothing was undone: the newest step in Studio's history is \"{top}\", which is not the agent's. Undo that yourself with Ctrl+Z, then try again."),
                (n, Some(top)) => format!("Undid {n} step(s), then stopped at \"{top}\", which is not the agent's. Scripts were left alone. Undo that step yourself with Ctrl+Z, then try again."),
                (0, None) if scripts == 0 => "There was nothing of the agent's to undo from this turn.".to_string(),
                (0, None) => format!("No steps to undo.{put_back}"),
                (n, None) => format!("Undid {n} step(s) in Studio.{put_back} Ctrl+Y brings the steps back, but not the script text."),
            };
            return json!({"ok": r["ok"] == true, "undone": undone, "scripts": scripts, "text": text});
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return json!({"ok": false, "text": "Studio did not answer in time."});
        }
        guard = ANSWERED.wait_timeout(guard, left).unwrap().0;
    }
}

fn sync(body: &Value) -> Value {
    let dm = body["dm"].as_str().unwrap_or("Edit").to_string();
    let commands = with(|l| {
        l.seen.insert(dm.clone(), paths::now_secs());
        if dm == "Edit" {
            l.place = body["place"].clone();
            l.version = body["version"].as_str().unwrap_or("").to_string();
            if body["selection"].is_object() {
                l.selection = body["selection"].clone();
            }
        }
        for e in body["logs"].as_array().into_iter().flatten() {
            l.seq += 1;
            let seq = l.seq;
            l.logs.push(json!({"seq": seq, "dm": dm, "level": e["level"], "text": e["text"], "t": e["t"]}));
        }
        let extra = l.logs.len().saturating_sub(400);
        l.logs.drain(..extra);
        for r in body["results"].as_array().into_iter().flatten() {
            if let Some(id) = r["id"].as_u64() {
                l.results.insert(id, r.clone());
            }
        }
        if dm == "Edit" { std::mem::take(&mut l.commands) } else { Vec::new() }
    });
    ANSWERED.notify_all();
    let chats: Vec<Value> = agents::agents().iter().map(|a| json!({"name": a.name(), "busy": a.busy()})).collect();
    json!({"commands": commands, "agents": chats})
}

/// Listen for the plugin. Does nothing if another copy of the app already has the port.
pub fn start() {
    let Ok(server) = Server::http(("127.0.0.1", PORT)) else { return };
    with(|l| l.listening = true);
    let server = Arc::new(server);
    thread::spawn(move || {
        while let Ok(mut req) = server.recv() {
            thread::spawn(move || {
                let secret = token();
                let allowed = is_local(&req) && secret.len() >= 32 && head(&req, "X-Fridae") == secret && *req.method() == Method::Post && req.url() == "/plugin/sync";
                let resp = if allowed { send_json(sync(&body_json(&mut req, 2_000_000))) } else { forbidden() };
                let _ = req.respond(resp);
            });
        }
    });
}
