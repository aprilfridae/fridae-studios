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

/// The command for one turn in `workspace`, with our tool servers written where Codex looks for them. Fridae's
/// toolbox (`app_mcp`) is given on the command line only (`toolbox_overrides`), never in the workspace's file.
#[allow(clippy::too_many_arguments)]
pub fn command(exe: &Path, workspace: &Path, servers: &Value, thread: Option<&str>, model: Option<&str>, effort: Option<&str>, full: bool, toolbox: Option<&crate::app_mcp::Wire>) -> Command {
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
    if let Some(w) = toolbox {
        for o in toolbox_overrides(&w.url) {
            cmd.arg("-c").arg(o);
        }
        // the key: in the environment only, read by Codex through `bearer_token_env_var`
        cmd.env("FRIDAE_MCP_KEY", &w.raw);
    }
    cmd.arg("-"); // the message is read from stdin
    cmd.current_dir(workspace);
    cmd
}

/// Fridae's toolbox as Codex's config overrides (T3's per-thread overrides): `fridae` with every tool but the blocking
/// `chat_wait`, and `fridae-direct` with only that one (Code Mode would turn a blocking wait into a polled nested
/// call). Each is approved as the app's other servers are (`write_servers`: nobody is there to answer an approval
/// question), may take 65 minutes a call (an Allow/Don't card waits up to 10), and reads its key from
/// `FRIDAE_MCP_KEY`. The instructions go in as developer instructions.
fn toolbox_overrides(url: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (name, tools) in [("fridae", "disabled_tools"), ("fridae-direct", "enabled_tools")] {
        out.push(format!("mcp_servers.{name}.url={}", quoted(url)));
        out.push(format!("mcp_servers.{name}.bearer_token_env_var=\"FRIDAE_MCP_KEY\""));
        out.push(format!("mcp_servers.{name}.tool_timeout_sec=3900"));
        out.push(format!("mcp_servers.{name}.{tools}=[\"chat_wait\"]"));
        out.push(format!("mcp_servers.{name}.default_tools_approval_mode=\"approve\""));
    }
    // (probe P2, codex-cli 0.169.1: accepted under --strict-config, which refuses unknown fields)
    out.push("mcp_servers.fridae-direct.omit_tools_from=[\"code_mode\",\"deferred\"]".to_string());
    out.push(format!("developer_instructions={}", quoted(crate::app_mcp::instructions::system_text().trim())));
    out
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
                        // Fridae's toolbox draws its own rows: named with its server so the chat knows it (`agents::is_toolbox_call`)
                        if let Some(server) = item["server"].as_str().filter(|s| *s == "fridae" || *s == "fridae-direct") {
                            if self.shown.insert(item["id"].as_str().unwrap_or("").to_string()) {
                                out.push(Out::Tool(format!("{server}.{tool}")));
                            }
                            return out;
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_overrides_on_the_command_line_not_in_the_project() {
        let ws = std::env::temp_dir().join(format!("fridae-codex-ws-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ws);
        fs::create_dir_all(&ws).unwrap();
        let w = crate::app_mcp::Wire { url: "http://127.0.0.1:5/mcp".into(), raw: "k-codex-raw".into(), deferred: false };
        let servers = json!({"rbxkit": {"command": "python3", "args": ["mcp.py"]}});
        let cmd = command(Path::new("/bin/codex"), &ws, &servers, None, None, None, false, Some(&w));
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        for want in [
            "mcp_servers.fridae.url=\"http://127.0.0.1:5/mcp\"",
            "mcp_servers.fridae.bearer_token_env_var=\"FRIDAE_MCP_KEY\"",
            "mcp_servers.fridae.tool_timeout_sec=3900",
            "mcp_servers.fridae.disabled_tools=[\"chat_wait\"]",
            "mcp_servers.fridae-direct.url=\"http://127.0.0.1:5/mcp\"",
            "mcp_servers.fridae-direct.bearer_token_env_var=\"FRIDAE_MCP_KEY\"",
            "mcp_servers.fridae-direct.tool_timeout_sec=3900",
            "mcp_servers.fridae-direct.enabled_tools=[\"chat_wait\"]",
            "mcp_servers.fridae-direct.omit_tools_from=[\"code_mode\",\"deferred\"]",
            "mcp_servers.fridae.default_tools_approval_mode=\"approve\"",
        ] {
            let at = args.iter().position(|a| a == want).unwrap_or_else(|| panic!("{want} in {args:?}"));
            assert_eq!(args[at - 1], "-c");
        }
        assert!(args.iter().any(|a| a.starts_with("developer_instructions=") && a.contains("Ordinary coding tasks need no Fridae tools.")));
        assert!(!args.iter().any(|a| a.contains("k-codex-raw")), "the key is never an argument");
        let env: Vec<(String, Option<String>)> = cmd.get_envs().map(|(k, v)| (k.to_string_lossy().to_string(), v.map(|v| v.to_string_lossy().to_string()))).collect();
        assert!(env.contains(&("FRIDAE_MCP_KEY".to_string(), Some("k-codex-raw".to_string()))));
        let toml = fs::read_to_string(ws.join(".codex").join("config.toml")).unwrap();
        assert!(toml.contains("[mcp_servers.rbxkit]") && !toml.contains("mcp_servers.fridae") && !toml.contains("k-codex-raw"), "{toml}");
        // without the toolbox: none of it
        let cmd = command(Path::new("/bin/codex"), &ws, &servers, None, None, None, false, None);
        assert!(!cmd.get_args().any(|a| a.to_string_lossy().contains("fridae")));
        assert!(!cmd.get_envs().any(|(k, _)| k == "FRIDAE_MCP_KEY"));
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_toolbox_call_is_named_by_its_server() {
        let mut t = Turn::default();
        let out = t.feed(&json!({"type": "item.started", "item": {"id": "i1", "type": "mcp_tool_call", "server": "fridae", "tool": "chat_launch"}}));
        assert!(out.iter().any(|o| matches!(o, Out::Tool(n) if n == "fridae.chat_launch")), "{}", out.len());
        let out = t.feed(&json!({"type": "item.started", "item": {"id": "i2", "type": "mcp_tool_call", "server": "rbxkit", "tool": "rbx_team"}}));
        assert!(out.iter().any(|o| matches!(o, Out::Tool(n) if n == "rbx_team")));
    }
}
