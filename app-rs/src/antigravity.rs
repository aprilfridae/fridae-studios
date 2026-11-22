//! Gemini as a chat's AI, through Google's Antigravity command-line tool `agy` run without its own interface.
//!
//! A chat's turns go to one `agy acp` process kept open for it (`acp.rs`, `agents::GeminiLaunch`). This file is the
//! other way, a process per message, which is the fallback (and all there is on Windows), and what both ways share:
//! the model list, the standing instructions, the `Out` values a turn is read into.
//!
//!   agy --input-format stream-json --output-format stream-json --mode accept-edits --settings {...}
//!
//! One JSON line with the message goes in on stdin; one JSON event per line comes out: `init`, then
//! `step_update` for each step and each piece of text, then `result`. Checked against agy 1.5.10.
//!
//! How this differs from Claude Code, and why:
//! - There is no flag for standing instructions, so they are placed at the top of a conversation's first message.
//! - Tool servers are read from `<workspace>/.agents/mcp_config.json`, so each chat gets a workspace folder.
//! - It is limited the same way Claude Code is: the settings passed for the session allow our two tool servers,
//!   and `accept-edits` lets it write files. Anything else that needs approval (a shell command, the browser) is
//!   refused, because nobody is there to approve it. A refusal ends its turn with a note on stderr, which
//!   `explain` turns into a message for the user.
//! - Tools on a server are all called through one built-in tool, `call_mcp_tool`, which names the real one in
//!   its parameters.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    process::Command,
    sync::OnceLock,
};

use serde_json::{Map, Value, json};

use crate::paths;

static MODELS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Ask Antigravity which models it offers (`agy models` prints one "id<tab>name" per line). Done once, in the
/// background, because it goes to the network.
pub fn load_models() {
    std::thread::spawn(|| {
        let Some(exe) = paths::find_antigravity() else { return };
        let mut cmd = Command::new(exe);
        cmd.arg("models");
        paths::quiet(&mut cmd);
        paths::child_env(&mut cmd, "gemini");
        if let Some(h) = home_override() {
            cmd.env("HOME", h);
        }
        let Ok(out) = cmd.output() else { return };
        let list = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(id, name)| (id.trim().to_string(), name.trim().to_string()))
            .filter(|(id, name)| !id.is_empty() && !id.contains(' ') && !name.is_empty())
            .collect();
        let _ = MODELS.set(list);
    });
}

/// FRIDAE_AGY_HOME and FRIDAE_AGY_DIR (testing only, debug builds only): a copy of the app with a scratch HOME gives
/// agy alone the real one, where it finds its sign-in, and its kept process a data folder of its own (`agy acp
/// --gemini-dir`), so nothing is written among the user's own agy conversations. The app's tool server gets the
/// app's HOME back in its entry.
#[cfg(debug_assertions)]
pub fn home_override() -> Option<std::path::PathBuf> {
    std::env::var_os("FRIDAE_AGY_HOME").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

#[cfg(debug_assertions)]
pub fn dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("FRIDAE_AGY_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

#[cfg(not(debug_assertions))]
pub fn home_override() -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(debug_assertions))]
pub fn dir_override() -> Option<std::path::PathBuf> {
    None
}

/// The models Antigravity offers, as (id, name). Empty until the list has arrived.
pub fn models() -> &'static [(String, String)] {
    MODELS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// The model a Gemini chat uses unless the user picks another: Gemini 4 Carbon where Antigravity offers it
/// (its middle level, since effort is chosen separately), else the first model it lists.
pub fn default_model() -> Option<String> {
    let list = models();
    let carbon: Vec<&(String, String)> = list.iter().filter(|(id, name)| id.contains("carbon") || name.to_lowercase().contains("carbon")).collect();
    carbon
        .iter()
        .find(|(id, _)| id.ends_with("-medium"))
        .or(carbon.first())
        .or(list.iter().find(|(id, _)| id.starts_with("gemini")).as_ref())
        .or(list.first().as_ref())
        .map(|(id, _)| id.clone())
}

/// A model's name for showing, from its id.
pub fn model_name(id: &str) -> String {
    models().iter().find(|(known, _)| known == id).map(|(_, name)| name.clone()).unwrap_or_else(|| id.to_string())
}

/// The arguments of the command for one turn in `workspace` (run there), with our tool servers written where
/// Antigravity looks for them.
pub fn turn_args(workspace: &Path, servers: &Value, conversation: Option<&str>, model: Option<&str>, effort: Option<&str>, full: bool) -> Vec<String> {
    write_servers(workspace, servers);
    let allowed: Vec<String> = servers.as_object().map(|s| s.keys().map(|name| format!("mcp({name}/*)")).collect()).unwrap_or_default();
    let mut args: Vec<String> = ["--input-format", "stream-json", "--output-format", "stream-json", "--mode", "accept-edits", "--settings"].iter().map(|a| a.to_string()).collect();
    // the app's settings are never written by an agent (Antigravity's `write_file(<absolute path>)` rule)
    args.push(json!({"permissions": {"allow": allowed, "deny": denied()}}).to_string());
    if full {
        args.push("--dangerously-skip-permissions".into()); // full auto: nothing is refused
    }
    for (flag, value) in [("--conversation", conversation), ("--model", model), ("--effort", effort)] {
        if let Some(v) = value {
            args.extend([flag.to_string(), v.to_string()]);
        }
    }
    args
}

/// Antigravity's deny rules for the app's own files (`paths::app_protected`): `write_file(<absolute path>)`, which
/// covers a folder's contents as its grants do.
fn denied() -> Vec<String> {
    crate::paths::app_protected().into_iter().map(|(p, _)| format!("write_file({})", p.display())).collect()
}

/// Add our servers to the workspace's list, leaving any the user put there.
fn write_servers(workspace: &Path, servers: &Value) {
    let dir = workspace.join(".agents");
    let path = dir.join("mcp_config.json");
    let mut cfg = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if !cfg["mcpServers"].is_object() {
        cfg["mcpServers"] = json!({});
    }
    for (name, server) in servers.as_object().unwrap_or(&Map::new()) {
        let mut server = server.clone();
        if let Some(o) = server.as_object_mut() {
            o.remove("type"); // Antigravity tells a local server by its `command`
            o.insert("timeoutSeconds".into(), json!(900)); // a texture request may wait minutes for the user
        }
        cfg["mcpServers"][name] = server;
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(path, serde_json::to_string_pretty(&cfg).unwrap_or_default());
}

/// Takes our servers out of the workspace's list again, leaving any the user put there. A kept process is handed
/// them over its protocol (`acp::servers_param`), and agy also starts every server of this list when a prompt comes
/// (measured), so one left here by an earlier process per message would run twice.
pub fn unwrite_servers(workspace: &Path, servers: &Value) {
    let path = workspace.join(".agents").join("mcp_config.json");
    let Some(mut cfg) = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { return };
    let Some(list) = cfg["mcpServers"].as_object_mut() else { return };
    let before = list.len();
    list.retain(|name, _| servers.get(name).is_none());
    if list.len() != before {
        let _ = fs::write(path, serde_json::to_string_pretty(&cfg).unwrap_or_default());
    }
}

/// The line that carries a message in on stdin.
pub fn message_line(text: &str) -> String {
    json!({"event": "user", "message": {"content": text}}).to_string() + "\n"
}

/// A conversation's first message, with the standing instructions on top. Used for every AI that has no
/// separate place for them.
pub fn with_instructions(instructions: &str, text: &str) -> String {
    format!(
        "[Standing instructions for this whole conversation. Follow them from now on; do not repeat them back or mention them.]\n\
         {}\n[End of the standing instructions.]\n\n{text}",
        instructions.trim()
    )
}

/// What one line of an AI's output means for the chat.
pub enum Out {
    Conversation(String),
    Model(String),
    Tool(String),
    /// A tool server's tool with its arguments (Cursor), so a status check is not taken for progress.
    ToolArgs(String, Value),
    Text(String),
    /// Image files a tool reported making, to show as thumbnails.
    Pictures(Vec<String>),
    /// Pictures a tool returned, as base64 (Cursor hands them back like Claude Code does).
    Encoded(Vec<String>),
    /// Commands that were refused because the user has not allowed their program: {name, command}.
    Blocked(Vec<Value>),
    /// Commands the app did not approve by itself for how they are written, not for their program: {command, reason,
    /// once} (once: the card may offer to run that exact command one time).
    Refused(Vec<Value>),
    Error(String),
    /// The AI answered, but its result says the turn failed: counted as a failed turn, like Claude Code's.
    Failed(String),
    /// What the turn used.
    Usage(crate::usage::Turn),
    Finished,
}

/// Image files named in a tool's text result that exist on disk: the ones meant for looking at first, two at most.
pub fn pictures_in(output: &str) -> Vec<String> {
    let text = output.replace("\\\\", "\\");
    let mut found: Vec<String> = Vec::new();
    for piece in text.split(['"', '\n', '\r', '\t']) {
        let path = piece.trim();
        let low = path.to_lowercase();
        let image = [".png", ".jpg", ".jpeg", ".webp"].iter().any(|e| low.ends_with(e));
        if image && path.len() < 400 && Path::new(path).is_absolute() && Path::new(path).is_file() && !found.iter().any(|f| f == path) {
            found.push(path.to_string());
        }
    }
    let for_looking = |p: &String| ["look", "sheet", "check", "shot", "preview"].iter().any(|w| p.to_lowercase().contains(w));
    found.sort_by_key(|p| !for_looking(p));
    found.truncate(2);
    found
}

/// Reads one turn's events. Text arrives in pieces; a reply is shown when its step is done.
#[derive(Default)]
pub struct Turn {
    texts: HashMap<i64, String>,
    shown: HashSet<i64>,
    said_something: bool,
}

impl Turn {
    pub fn feed(&mut self, e: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        let step = &e["step_update"];
        if let Some(id) = e["conversation_id"].as_str().or(step["conversation_id"].as_str()).or(e["result"]["conversation_id"].as_str()).filter(|s| !s.is_empty()) {
            out.push(Out::Conversation(id.to_string()));
        }
        match e["event"].as_str().unwrap_or("") {
            "init" => {
                let model = e["init"]["model"].as_str().filter(|m| !m.is_empty());
                out.push(Out::Model(model.map(model_name).unwrap_or_else(|| "Gemini".to_string())));
            }
            "step_update" => {
                let index = step["step_index"].as_i64().unwrap_or(-1);
                let done = step["state"] == "DONE";
                match step["step_type"].as_str().unwrap_or("") {
                    "tool" => {
                        let params = &step["tool_info"]["parameters"];
                        let name = step["tool_name"].as_str().or(step["tool_info"]["name"].as_str()).unwrap_or("");
                        // a server's tool is named in the parameters, which may only arrive once the step is done
                        let shown_as = if name == "call_mcp_tool" { params["ToolName"].as_str().filter(|_| true).or(done.then_some("a tool")) } else { Some(name) };
                        // it reads each tool's description file before calling it: not worth a line in the chat
                        let housekeeping = params["Path"].as_str().is_some_and(|p| p.replace('\\', "/").contains("/antigravity-cli/mcp/"));
                        if let Some(label) = shown_as.filter(|l| !l.is_empty() && !housekeeping) {
                            if self.shown.insert(index) {
                                out.push(Out::Tool(label.to_string()));
                            }
                        }
                        // its tool results arrive as text, so the pictures our tools made are found by the paths they report
                        let ours = params["ToolName"].as_str().is_some_and(|t| t.starts_with("rbx_") || t.starts_with("screen_capture"));
                        if done && ours {
                            let pictures = pictures_in(step["tool_info"]["output"].as_str().unwrap_or(""));
                            if !pictures.is_empty() {
                                out.push(Out::Pictures(pictures));
                            }
                        }
                    }
                    "agent_response" => {
                        if let Some(piece) = step["text_delta"].as_str() {
                            self.texts.entry(index).or_default().push_str(piece);
                        }
                        if done && self.shown.insert(index) {
                            if let Some(text) = self.texts.remove(&index).filter(|t| !t.trim().is_empty()) {
                                self.said_something = true;
                                out.push(Out::Text(text.trim_end().to_string()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                let r = &e["result"];
                out.push(Out::Usage(crate::usage::Turn::from_antigravity(&r["usage"])));
                // anything written but never marked done still belongs to the reply
                let mut left: Vec<(i64, String)> = self.texts.drain().filter(|(_, t)| !t.trim().is_empty()).collect();
                left.sort_by_key(|(i, _)| *i);
                for (_, text) in left {
                    self.said_something = true;
                    out.push(Out::Text(text.trim_end().to_string()));
                }
                if !self.said_something {
                    if let Some(text) = r["response"].as_str().filter(|t| !t.trim().is_empty()) {
                        out.push(Out::Text(text.trim_end().to_string()));
                    }
                }
                let status = r["status"].as_str().unwrap_or("");
                if !matches!(status, "SUCCESS" | "") {
                    let why = r["error"]["message"].as_str().or(r["error"].as_str()).filter(|s| !s.is_empty());
                    out.push(Out::Error(why.map(str::to_string).unwrap_or_else(|| format!("Gemini stopped: {}.", status.to_lowercase()))));
                }
                out.push(Out::Finished);
            }
            _ => {}
        }
        out
    }
}

/// What the chat is told when nobody is signed in to Antigravity.
pub const SIGNED_OUT: &str = "Gemini is not signed in. Open a terminal, run `agy`, sign in with your Google account, close it, then send your message again.";

/// A plain-language message for what Antigravity wrote on stderr, if we recognise it.
pub fn explain(stderr: &str, got_result: bool) -> Option<String> {
    let low = stderr.to_lowercase();
    if low.contains("auto-denied") {
        // e.g. `a tool required the "command" permission that headless mode cannot prompt for, so it was auto-denied`
        let what = stderr.split("required the \"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
        let what = match what {
            "command" => "run a command on this computer",
            "browser" => "use a web browser",
            "" => "do something",
            other => return Some(format!("Gemini tried to use \"{other}\", which this app does not allow, and stopped there. Ask again, and tell it to use the building tools.")),
        };
        return Some(format!("Gemini tried to {what}, which this app does not allow, and stopped there. Ask again, and tell it to use the building tools."));
    }
    if !got_result && (low.contains("authentication required") || low.contains("not authenticated") || low.contains("please sign in") || low.contains("login required")) {
        return Some(SIGNED_OUT.to_string());
    }
    None
}
