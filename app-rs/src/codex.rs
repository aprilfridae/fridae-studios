//! ChatGPT as the chat's AI, through OpenAI's command-line tool `codex` run without its own interface.
//!
//!   codex exec [resume <thread>] --json --skip-git-repo-check --trust --sandbox workspace-write -
//!
//! The message goes in on stdin; one JSON event per line comes out: `thread.started`, then `item.started` /
//! `item.completed` for each command, tool call and reply, then `turn.completed` (or `turn.failed`).
//!
//! How this differs from Claude Code, and why:
//! - There is no flag for standing instructions, so they are placed at the top of a conversation's first message.
//! - Tool servers are read from `<workspace>/.codex/config.toml` (only for a trusted folder, hence `--trust`),
//!   so each chat gets a workspace folder. Our servers are marked as approved there, because nobody is present to
//!   answer an approval question.
//! - Its own commands run in its sandbox: they can write inside the workspace and nowhere else.

use crate::paths::Start;
use std::{collections::HashSet, fs, path::Path, process::Command};

use serde_json::{Map, Value};

use crate::{
    antigravity::{Out, pictures_in},
    paths,
};

const BEGIN: &str = "# fridae-studios:begin (written by Fridae Studios; edit outside these two lines)";
const END: &str = "# fridae-studios:end";

static MODELS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();

/// Ask Codex which models it offers (`codex debug models` prints its catalog as JSON). Done once, in the background.
pub fn load_models() {
    std::thread::spawn(|| {
        let Some(exe) = paths::find_codex() else { return };
        let mut cmd = Command::new(exe);
        cmd.args(["debug", "models"]);
        paths::quiet(&mut cmd);
        paths::child_env(&mut cmd, "chatgpt");
        let Ok(out) = cmd.run_output() else { return };
        let Ok(d) = serde_json::from_slice::<Value>(&out.stdout) else { return };
        let mut list: Vec<(i64, String, String)> = d["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["visibility"] == "list")
            .filter_map(|m| Some((m["priority"].as_i64().unwrap_or(99), m["slug"].as_str()?.to_string(), m["display_name"].as_str()?.to_string())))
            .collect();
        list.sort_by_key(|m| m.0);
        let _ = MODELS.set(list.into_iter().map(|(_, id, name)| (id, name)).collect());
    });
}

/// The models Codex offers, as (id, name), best first. Empty until the list has arrived.
pub fn models() -> &'static [(String, String)] {
    MODELS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// The model a ChatGPT chat uses unless the user picks another: the newest Astra (Codex lists newest first).
pub fn default_model() -> Option<String> {
    let list = models();
    list.iter().find(|(id, _)| id.contains("astra")).or(list.first()).map(|(id, _)| id.clone())
}

/// The command for one turn in `workspace`, with our tool servers written where Codex looks for them.
pub fn command(exe: &Path, workspace: &Path, servers: &Value, thread: Option<&str>, model: Option<&str>, effort: Option<&str>, full: bool) -> Command {
    write_servers(workspace, servers);
    let mut cmd = Command::new(exe);
    cmd.arg("exec");
    if let Some(id) = thread {
        cmd.args(["resume", id]);
    }
    cmd.args(["--json", "--skip-git-repo-check", "--trust"]);
    // full auto: no approvals and no sandbox, so it can run anything; otherwise it may only write in the workspace
    if full {
        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
    } else {
        cmd.args(["--sandbox", "workspace-write"]);
    }
    if let Some(model) = model {
        cmd.args(["--model", model]);
    }
    if let Some(e) = effort {
        cmd.arg("-c").arg(format!("model_reasoning_effort={}", quoted(e)));
    }
    cmd.arg("-"); // the message is read from stdin
    cmd.current_dir(workspace);
    cmd
}

/// A string as TOML writes it. JSON's quoting is valid TOML for a basic string.
fn quoted(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// Put our servers into the workspace's Codex settings, between two marker lines, leaving the rest alone.
fn write_servers(workspace: &Path, servers: &Value) {
    let mut block = vec![BEGIN.to_string()];
    for (name, server) in servers.as_object().unwrap_or(&Map::new()) {
        let args: Vec<String> = server["args"].as_array().into_iter().flatten().filter_map(Value::as_str).map(quoted).collect();
        block.push(format!("[mcp_servers.{name}]"));
        block.push(format!("command = {}", quoted(server["command"].as_str().unwrap_or(""))));
        block.push(format!("args = [{}]", args.join(", ")));
        block.push("startup_timeout_sec = 30".to_string());
        block.push("tool_timeout_sec = 900".to_string()); // a texture request may wait minutes for the user
        block.push("default_tools_approval_mode = \"approve\"".to_string());
        if let Some(env) = server["env"].as_object().filter(|e| !e.is_empty()) {
            block.push(format!("[mcp_servers.{name}.env]"));
            for (key, value) in env {
                block.push(format!("{key} = {}", quoted(value.as_str().unwrap_or(""))));
            }
        }
    }
    block.push(END.to_string());
    let block = block.join("\n");
    let dir = workspace.join(".codex");
    let path = dir.join("config.toml");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let new = match (old.find(BEGIN), old.find(END)) {
        (Some(a), Some(b)) if b > a => format!("{}{}{}", &old[..a], block, &old[b + END.len()..]),
        _ if old.trim().is_empty() => block + "\n",
        _ => format!("{}\n\n{}\n", old.trim_end(), block),
    };
    // never through a symbolic link, nor over one: the app makes none, so it is not the app's to write
    let link = |p: &Path| p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink());
    if link(&path) || link(&dir) {
        return;
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(path, new);
}

/// Reads one turn's events.
#[derive(Default)]
pub struct Turn {
    shown: HashSet<String>,
    announced: bool,
}

impl Turn {
    pub fn feed(&mut self, e: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        if !self.announced {
            self.announced = true;
            out.push(Out::Model("ChatGPT".to_string()));
        }
        let item = &e["item"];
        match e["type"].as_str().unwrap_or("") {
            "thread.started" => {
                if let Some(id) = e["thread_id"].as_str().filter(|s| !s.is_empty()) {
                    out.push(Out::Conversation(id.to_string()));
                }
            }
            kind @ ("item.started" | "item.completed") => {
                let id = item["id"].as_str().unwrap_or("").to_string();
                let tool = match item["type"].as_str().unwrap_or("") {
                    "agent_message" => {
                        if kind == "item.completed" {
                            if let Some(text) = item["text"].as_str().filter(|t| !t.trim().is_empty()) {
                                out.push(Out::Text(text.to_string()));
                            }
                        }
                        None
                    }
                    "command_execution" => Some("run_command".to_string()),
                    "file_change" => Some("Edit".to_string()),
                    "web_search" => Some("WebSearch".to_string()),
                    "mcp_tool_call" => {
                        // its tool results arrive as text too: show the pictures our tools report making
                        let tool = item["tool"].as_str().or(item["name"].as_str()).unwrap_or("a tool");
                        if kind == "item.completed" && (tool.starts_with("rbx_") || tool.starts_with("screen_capture")) {
                            let pictures = pictures_in(&item["result"].to_string());
                            if !pictures.is_empty() {
                                out.push(Out::Pictures(pictures));
                            }
                        }
                        Some(tool.to_string())
                    }
                    _ => None,
                };
                // a step is announced once, whether we first hear of it starting or finishing
                if let Some(name) = tool.filter(|_| self.shown.insert(id)) {
                    out.push(Out::Tool(name));
                }
            }
            "turn.completed" => {
                out.push(Out::Usage(crate::usage::Turn::from_codex(&e["usage"])));
                out.push(Out::Finished);
            }
            "turn.failed" | "error" => {
                let why = e["error"]["message"].as_str().or(e["message"].as_str()).filter(|s| !s.is_empty()).unwrap_or("ChatGPT stopped with an error.");
                out.push(Out::Error(why.to_string()));
                out.push(Out::Finished);
            }
            _ => {}
        }
        out
    }
}

/// A plain-language message for what Codex wrote when it failed to start a turn, if we recognise it.
pub fn explain(stderr: &str) -> Option<&'static str> {
    let low = stderr.to_lowercase();
    if low.contains("not logged in") || low.contains("codex login") || low.contains("401") || low.contains("unauthorized") || low.contains("authentication") {
        Some("ChatGPT's command-line tool is not signed in. Open a terminal, run `codex login`, sign in with your ChatGPT account, then send your message again.")
    } else {
        None
    }
}
