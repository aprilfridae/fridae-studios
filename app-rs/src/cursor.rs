//! Cursor as a chat's AI, through the Cursor Agent command-line tool `cursor-agent` run without its own interface.
//!
//!   cursor-agent -p --output-format stream-json --trust --approve-mcps --workspace <chat folder>
//!                --add-dir <kit> [--add-dir <game>] [--model <id>] [--resume <chat>] [--force]
//!
//! The message goes in on stdin; one JSON event per line comes out, close to Claude Code's: `system`/`init` (with
//! the chat's `session_id`), `user`, `thinking`, `assistant` (one per message between tool steps), `tool_call`
//! `started`/`completed` (`{"mcpToolCall": {args, result}}`, `shellToolCall`, `editToolCall`, ...), then `result`
//! with the turn's token counts. Checked against cursor-agent 2026.11.22. A failure before the turn starts (signed
//! out, an unknown model) prints one line on stderr and no events.
//!
//! How this differs from Claude Code, and why:
//! - There is no flag for standing instructions, so they are placed at the top of a conversation's first message,
//!   as for the other AIs that are not Claude.
//! - Nothing is written where Cursor keeps its own settings, and nothing in a game's folder. Every Fridae chat on
//!   Cursor runs in a folder of the app's own (`work/cursor/chats/<chat>`, with an empty `.git` folder so Cursor takes
//!   it as the project's top and looks no higher), whose `.cursor/mcp.json` holds our two tool servers; the game is
//!   added with `--add-dir`. `CURSOR_CONFIG_DIR` and `CURSOR_DATA_DIR` point at `work/cursor/config` and
//!   `work/cursor/data`, so Cursor's permissions file, its saved chats and its approvals for these chats are the
//!   app's, and the user's own `~/.cursor/cli-config.json` (and the permissions in it) is neither read nor changed.
//!   The sign-in is not in those folders (it is in the system keychain), so it still works.
//! - Permissions (`work/cursor/config/cli-config.json`, rewritten before every turn): our tool servers and the
//!   programs the user allowed (`Shell(<program>)`) are allowed; any other shell command is refused, because nobody is
//!   there to approve it (the refusal is reported like Claude Code's, and the page offers to allow the program).
//!   Tool servers the user set up for Cursor itself in `~/.cursor/mcp.json` are denied by name, and with
//!   FRIDAE_NO_STUDIO anything named like Studio is denied too. Cursor lets its file tools write anywhere unless a
//!   rule denies it (Claude Code only writes in its folders); the deny rules cover Cursor's own settings and the
//!   hidden folders and Library of the home folder. See DEVELOPING.md for what that leaves open.
//! - Full auto passes `--force`: shell commands run unasked, as in Claude Code's bypass mode; the deny rules for tool
//!   servers still hold.

use std::{
    env,
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use serde_json::{Map, Value, json};

use crate::{antigravity::Out, paths};

static MODELS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// The model a Cursor chat starts on: Cursor's own choice per request.
pub const DEFAULT_MODEL: &str = "auto";

/// Ask Cursor which models this account may use (`cursor-agent models` prints "id - Name" lines). Done once, in the
/// background, because it goes to the network.
pub fn load_models() {
    std::thread::spawn(|| {
        let Some(exe) = paths::find_cursor() else { return };
        let mut cmd = Command::new(exe);
        cmd.arg("models");
        environment(&mut cmd);
        paths::quiet(&mut cmd);
        let Ok(out) = cmd.output() else { return };
        let _ = MODELS.set(parse_models(&String::from_utf8_lossy(&out.stdout)));
    });
}

/// The "id - Name" lines of `cursor-agent models`, without the "(current, default)" notes.
pub fn parse_models(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once(" - "))
        .map(|(id, name)| {
            let name = name.trim();
            let name = match name.rfind(" (") {
                Some(at) if name.ends_with(')') && ["current", "default"].iter().any(|w| name[at..].contains(w)) => &name[..at],
                _ => name,
            };
            (id.trim().to_string(), name.trim().to_string())
        })
        .filter(|(id, name)| !id.is_empty() && !id.contains(char::is_whitespace) && !name.is_empty())
        .collect()
}

/// The models Cursor offers, as (id, name). Empty until the list has arrived.
pub fn models() -> &'static [(String, String)] {
    MODELS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// The model a Cursor chat uses unless the user picks another: Auto, unless the list came and lacks it.
pub fn default_model() -> Option<String> {
    default_of(models())
}

fn default_of(list: &[(String, String)]) -> Option<String> {
    if list.is_empty() || list.iter().any(|(id, _)| id == DEFAULT_MODEL) { Some(DEFAULT_MODEL.to_string()) } else { list.first().map(|(id, _)| id.clone()) }
}

/// Where the app keeps what Cursor writes for its chats: `config` (permissions and saved chats), `data` (approvals
/// per workspace) and `chats/<id>` (each chat's workspace).
fn root() -> PathBuf {
    paths::work("cursor")
}

pub fn config_dir() -> PathBuf {
    root().join("config")
}

pub fn data_dir() -> PathBuf {
    root().join("data")
}

/// The folder a chat's Cursor works in.
pub fn workspace(agent_id: &str) -> PathBuf {
    root().join("chats").join(agent_id)
}

/// The home folder Cursor runs with. FRIDAE_CURSOR_HOME (testing only) gives a copy of the app with a scratch HOME the
/// real one for Cursor alone, because Cursor finds its sign-in through the home folder's keychain.
fn cursor_home() -> PathBuf {
    env::var_os("FRIDAE_CURSOR_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(paths::home_dir)
}

fn home_override() -> bool {
    env::var_os("FRIDAE_CURSOR_HOME").is_some_and(|v| !v.is_empty())
}

/// Cursor's settings and data folders are the app's (see the top of this file).
fn environment(cmd: &mut Command) {
    cmd.env("CURSOR_CONFIG_DIR", config_dir()).env("CURSOR_DATA_DIR", data_dir());
    if home_override() {
        cmd.env("HOME", cursor_home());
    }
}

/// The names of the tool servers the user set up for Cursor itself (`~/.cursor/mcp.json`), read only.
fn user_servers(home: &Path) -> Vec<String> {
    fs::read_to_string(home.join(".cursor").join("mcp.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["mcpServers"].as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// What Cursor may do without asking. `servers` are ours; `allowed` the programs the user allowed; `theirs` the
/// servers of the user's own Cursor setup (denied, as Claude Code is given only ours); `protect` folders its file tools
/// must not write in.
pub fn permissions(servers: &Value, allowed: &[String], theirs: &[String], protect: &[PathBuf]) -> Value {
    let ours: Vec<&String> = servers.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    let mut allow: Vec<String> = ours.iter().map(|name| format!("Mcp({name}:*)")).collect();
    allow.extend(allowed.iter().map(|p| format!("Shell({p})")));
    allow.push("WebFetch(*)".to_string()); // Claude Code is given WebFetch (and WebSearch) here too
    let mut deny: Vec<String> = Vec::new();
    // a server of ours replaces theirs of the same name (the workspace's file comes first), so only the others go
    let mut seen = HashSet::new();
    for name in theirs.iter().filter(|n| !ours.contains(n)) {
        if seen.insert(name.clone()) {
            deny.push(format!("Mcp({name}:*)"));
        }
    }
    // no Studio for this copy (FRIDAE_NO_STUDIO): whatever Studio server Cursor finds elsewhere is refused too
    if !ours.iter().any(|n| n.to_lowercase().contains("studio")) {
        deny.extend(["Mcp(*studio*:*)", "Mcp(*Studio*:*)", "Mcp(*STUDIO*:*)"].map(str::to_string));
    }
    // its file tools may write anywhere unless denied: never its own settings, permissions or tool lists
    deny.push("Write(**/.cursor/**)".to_string());
    for dir in protect {
        deny.push(format!("Write({})", dir.display()));
        deny.push(format!("Write({}/**)", dir.display()));
    }
    json!({"allow": allow, "deny": deny})
}

/// The folders Cursor's file tools are kept out of: the app's Cursor settings, and the home folder's hidden folders
/// and files (shell start-up files, other tools' settings) and Library.
fn protected(home: &Path) -> Vec<PathBuf> {
    let mut list = vec![config_dir(), data_dir()];
    if !home.as_os_str().is_empty() {
        list.extend([home.join(".*"), home.join("Library")]);
    }
    list
}

/// Write the permissions into the app's Cursor settings file, keeping what Cursor itself stores there.
fn write_config(dir: &Path, permissions: &Value) {
    let path = dir.join("cli-config.json");
    let mut cfg = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if !cfg["version"].is_number() {
        cfg["version"] = json!(1);
    }
    cfg["permissions"] = permissions.clone();
    cfg["approvalMode"] = json!("allowlist");
    cfg["autoAcceptWebSearch"] = json!(true); // as Claude Code's WebSearch is allowed
    if !cfg["sandbox"].is_object() {
        cfg["sandbox"] = json!({});
    }
    cfg["sandbox"]["mode"] = json!("disabled"); // the programs the user allowed run as they do for Claude Code
    let _ = fs::create_dir_all(dir);
    // two chats may start at once: write a file of our own, then put it in place in one step
    let tmp = dir.join(format!("cli-config.json.{}-{}.tmp", std::process::id(), paths::stamp()));
    if fs::write(&tmp, serde_json::to_string_pretty(&cfg).unwrap_or_default()).is_ok() && fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// The app's own settings a tool server reads from its environment. Cursor starts tool servers with only HOME, PATH
/// and what the server's entry lists (Claude Code passes its whole environment), so these are listed for it. No keys
/// or tokens: this file is written to disk.
const PASSED: [&str; 9] = ["RBXKIT_WORK", "RBXKIT_BLENDER", "RBXKIT_STUDIO_MCP", "RBXKIT_STUDIO_ID", "FRIDAE_AGY", "LOCALAPPDATA", "APPDATA", "USERPROFILE", "TMPDIR"];

/// The settings from `PASSED` this copy of the app has, and HOME when Cursor itself runs with another
/// (FRIDAE_CURSOR_HOME), so the servers see the app's home folder, not Cursor's.
fn passed_env() -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = PASSED.iter().filter_map(|k| env::var(k).ok().filter(|v| !v.is_empty()).map(|v| (k.to_string(), v))).collect();
    if home_override() {
        list.push(("HOME".to_string(), paths::home_dir().display().to_string()));
    }
    list
}

/// Our tool servers as the workspace's `.cursor/mcp.json` gives them to Cursor, with `extra` added to each one's
/// environment (what the server's own entry sets wins).
pub fn servers_file(servers: &Value, extra: &[(String, String)]) -> Value {
    let mut out = Map::new();
    for (name, server) in servers.as_object().unwrap_or(&Map::new()) {
        let mut server = server.clone();
        if let Some(o) = server.as_object_mut() {
            o.remove("type"); // Cursor tells a local server by its `command`
            if !extra.is_empty() {
                if !o.get("env").is_some_and(Value::is_object) {
                    o.insert("env".into(), json!({}));
                }
                for (k, v) in extra {
                    if o["env"].get(k).is_none() || k == "HOME" {
                        o["env"][k] = json!(v);
                    }
                }
            }
        }
        out.insert(name.clone(), server);
    }
    json!({"mcpServers": out})
}

/// Make the chat's workspace: our servers in `.cursor/mcp.json` (the whole file is ours), and an empty `.git` so
/// Cursor takes this folder as the project's top even when the work folder sits inside a git repository.
fn write_workspace(workspace: &Path, servers: &Value) {
    let dir = workspace.join(".cursor");
    let _ = fs::create_dir_all(&dir);
    let _ = fs::create_dir_all(workspace.join(".git"));
    let _ = fs::write(dir.join("mcp.json"), serde_json::to_string_pretty(&servers_file(servers, &passed_env())).unwrap_or_default());
}

/// The arguments of one turn (the message goes in on stdin).
pub fn args(workspace: &Path, kit: &Path, project: Option<&str>, session: Option<&str>, model: Option<&str>, full: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-p", "--output-format", "stream-json", "--trust", "--approve-mcps", "--workspace"].map(str::to_string).to_vec();
    a.push(workspace.display().to_string());
    a.extend(["--add-dir".to_string(), kit.display().to_string()]);
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        a.extend(["--add-dir".to_string(), p.to_string()]);
    }
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        a.extend(["--model".to_string(), m.to_string()]);
    }
    if let Some(s) = session.filter(|s| !s.is_empty()) {
        a.extend(["--resume".to_string(), s.to_string()]);
    }
    if full {
        a.push("--force".to_string()); // full auto: shell commands run unasked; the deny rules still hold
    }
    a
}

/// The command for one turn, with the workspace and the app's Cursor settings written first.
pub fn command(exe: &Path, workspace: &Path, servers: &Value, session: Option<&str>, model: Option<&str>, full: bool, project: Option<&str>, kit: &Path) -> Command {
    write_workspace(workspace, servers);
    let home = cursor_home();
    write_config(&config_dir(), &permissions(servers, &paths::allowed_commands(), &user_servers(&home), &protected(&home)));
    let _ = fs::create_dir_all(data_dir());
    let mut cmd = Command::new(exe);
    cmd.args(args(workspace, kit, project, session, model, full));
    environment(&mut cmd);
    cmd.current_dir(workspace);
    cmd
}

/// What Cursor is told about its folders: it does not work in the game's folder (as the project text says), but in
/// the app's folder for the chat, with the game's folder added.
pub fn folder_note(project: Option<&str>) -> String {
    let mut s = String::from(
        "\nFOR YOU, ON CURSOR: your working directory is a scratch folder the app keeps for this chat, not a game folder. \
         Do not put work there. Commands the user has not allowed are refused, as are writes to Cursor's own settings.",
    );
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        s += &format!(" The game's folder is added to your workspace: read and write the game's files (GAME.md included) by their full paths under {p}.");
    }
    s + "\n"
}

/// The tool step one `tool_call` event is about: its kind (`mcpToolCall`, `shellToolCall`...) and its body.
fn step(e: &Value) -> Option<(&str, &Value)> {
    e["tool_call"].as_object()?.iter().find(|(k, _)| k.ends_with("ToolCall")).map(|(k, v)| (k.as_str(), v))
}

/// Reads one turn's events.
#[derive(Default)]
pub struct Turn {
    said_something: bool,
    shown: HashSet<String>,
    blocked: Vec<Value>,
}

impl Turn {
    pub fn feed(&mut self, e: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        match e["type"].as_str().unwrap_or("") {
            "system" if e["subtype"] == "init" => {
                if let Some(id) = e["session_id"].as_str().filter(|s| !s.is_empty()) {
                    out.push(Out::Conversation(id.to_string()));
                }
                let model = e["model"].as_str().map(str::trim).filter(|m| !m.is_empty()).unwrap_or("Auto");
                out.push(Out::Model(if model.eq_ignore_ascii_case("auto") { "Cursor (Auto)".to_string() } else { model.to_string() }));
            }
            "assistant" => {
                for b in e["message"]["content"].as_array().into_iter().flatten() {
                    if let Some(t) = b["text"].as_str().filter(|t| b["type"] == "text" && !t.trim().is_empty()) {
                        self.said_something = true;
                        out.push(Out::Text(t.trim_end().to_string()));
                    }
                }
            }
            "tool_call" => {
                let Some((kind, body)) = step(e) else { return out };
                let id = e["call_id"].as_str().unwrap_or("").to_string();
                let args = &body["args"];
                let name = match kind {
                    // it reads each tool's description before calling it: not worth a line in the chat
                    "getMcpToolsToolCall" => None,
                    "mcpToolCall" => {
                        let server = args["serverIdentifier"].as_str().or(args["providerIdentifier"].as_str()).unwrap_or("");
                        let tool = args["toolName"].as_str().unwrap_or("a tool");
                        Some(if server.is_empty() { tool.to_string() } else { format!("mcp__{server}__{tool}") })
                    }
                    "shellToolCall" => Some("run_command".to_string()),
                    "editToolCall" => Some("Edit".to_string()),
                    "writeToolCall" => Some("Write".to_string()),
                    "readToolCall" => Some("Read".to_string()),
                    other => Some(other.trim_end_matches("ToolCall").to_string()),
                };
                // a step is announced once, whether we first hear of it starting or finishing
                if let Some(name) = name.filter(|_| self.shown.insert(id)) {
                    out.push(Out::Tool(name));
                }
                if e["subtype"] != "completed" {
                    return out;
                }
                let result = &body["result"];
                match kind {
                    "mcpToolCall" => {
                        // only pictures from our own tools and Studio screenshots are shown
                        let tool = args["toolName"].as_str().unwrap_or("");
                        if tool.starts_with("rbx_") || tool.starts_with("screen_capture") {
                            let pictures: Vec<String> = result["success"]["content"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|c| c["image"]["data"].as_str())
                                .filter(|d| !d.is_empty())
                                .take(4)
                                .map(str::to_string)
                                .collect();
                            if !pictures.is_empty() {
                                out.push(Out::Encoded(pictures));
                            }
                        }
                    }
                    "shellToolCall" if result["rejected"].is_object() => {
                        // a command the user has not allowed: the page offers to allow each program
                        let command = result["rejected"]["command"].as_str().or(args["command"].as_str()).unwrap_or("");
                        if let Some(name) = paths::program_name(command).filter(|n| !self.blocked.iter().any(|b| b["name"] == *n)) {
                            self.blocked.push(json!({"name": name, "command": command.chars().take(200).collect::<String>()}));
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                out.push(Out::Usage(crate::usage::Turn::from_cursor(&e["usage"])));
                if !self.said_something {
                    if let Some(text) = e["result"].as_str().filter(|t| !t.trim().is_empty() && e["is_error"] != true) {
                        out.push(Out::Text(text.trim_end().to_string()));
                    }
                }
                if !self.blocked.is_empty() {
                    out.push(Out::Blocked(std::mem::take(&mut self.blocked)));
                }
                if e["is_error"] == true || e["subtype"].as_str().is_some_and(|s| s != "success") {
                    let why = e["result"].as_str().or(e["error"]["message"].as_str()).or(e["error"].as_str()).filter(|s| !s.trim().is_empty());
                    out.push(Out::Failed(why.map(str::to_string).unwrap_or_else(|| "Cursor stopped with an error.".to_string())));
                }
                out.push(Out::Finished);
            }
            _ => {}
        }
        out
    }
}

/// A plain-language message for what Cursor wrote on stderr when a turn failed to start, if we recognise it.
pub fn explain(stderr: &str, got_result: bool) -> Option<String> {
    if got_result {
        return None;
    }
    let low = stderr.to_lowercase();
    if low.contains("authentication required") || low.contains("not logged in") || low.contains("agent login") || low.contains("unauthorized") {
        return Some("Cursor is not signed in. Open a terminal, run `cursor-agent login`, sign in with your Cursor account, then send your message again.".to_string());
    }
    if low.contains("cannot use this model") {
        return Some("Cursor does not offer the chosen model to this account. Pick another model under the message box (Auto always works), then send your message again.".to_string());
    }
    if low.contains("usage limit") || low.contains("rate limit") || low.contains("quota") {
        let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return Some(format!("Cursor refused the turn: {line}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines from cursor-agent 2026.11.22 (long texts and picture data shortened).
    const INIT: &str = r#"{"type":"system","subtype":"init","apiKeySource":"login","cwd":"/tmp/ws","session_id":"ff455855-e83e-4480-9d3c-b80cfc773520","model":"Auto","permissionMode":"default"}"#;
    const THINKING: &str = r#"{"type":"thinking","subtype":"delta","text":" user just said \"hi\" - a simple greeting.","session_id":"00000000-0000-0000-0000-000000000000","timestamp_ms":1795343926523}"#;
    const SAY: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll call `rbx_vault` and run that shell echo in parallel, then reply in one short sentence."}]},"session_id":"ff455855-e83e-4480-9d3c-b80cfc773520","model_call_id":"5b0de173-4928-4e3c-8731-d934db95979d-0-xwld","timestamp_ms":1795343700410}"#;
    const DESCRIBE: &str = r#"{"type":"tool_call","subtype":"started","call_id":"call-a6c2-0\nfc_4796_0","tool_call":{"getMcpToolsToolCall":{"args":{"server":"rbxkit","toolName":"rbx_vault","toolCallId":"call-a6c2-0\nfc_4796_0"}},"hookAdditionalContexts":[],"toolCallId":"call-a6c2-0\nfc_4796_0","startedAtMs":"1795343700390"},"model_call_id":"5b0d-0-xwld","session_id":"ff455855-e83e-4480-9d3c-b80cfc773520","timestamp_ms":1795343700411}"#;
    const MCP_START: &str = r#"{"type":"tool_call","subtype":"started","call_id":"call-4ff8-2\nfc_80cf_0","tool_call":{"mcpToolCall":{"args":{"name":"rbxkit-rbx_vault","args":{"action":"search","query":"datastore retry","k":1},"toolCallId":"call-4ff8-2\nfc_80cf_0","providerIdentifier":"rbxkit","toolName":"rbx_vault","smartModeApprovalOnly":false,"skipApproval":false,"serverIdentifier":"rbxkit","symbolicPathArguments":[],"argumentsRepaired":false,"refuseSavedSecrets":false},"description":"Search vault for datastore retry guidance"},"hookAdditionalContexts":[],"toolCallId":"call-4ff8-2\nfc_80cf_0","startedAtMs":"1795343702340"},"session_id":"ff455855-e83e-4480-9d3c-b80cfc773520"}"#;
    const MCP_DONE: &str = r#"{"type":"tool_call","subtype":"completed","call_id":"call-4ff8-2\nfc_80cf_0","tool_call":{"mcpToolCall":{"args":{"name":"rbxkit-rbx_vault","args":{"action":"search","query":"datastore retry","k":1},"providerIdentifier":"rbxkit","toolName":"rbx_vault","serverIdentifier":"rbxkit"},"result":{"success":{"content":[{"text":{"text":"Results for \"datastore retry\" (vault, guides, team, yours):\n1. [vault] Error Handling and Retries  (note, 16 KB)"}}],"isError":false}}}},"session_id":"ff455855-e83e-4480-9d3c-b80cfc773520"}"#;
    const PICTURES: &str = r#"{"type":"tool_call","subtype":"completed","call_id":"call-7fa2-15\nfc_27a3_0","tool_call":{"mcpToolCall":{"args":{"name":"rbxkit-rbx_texture","args":{"no_preview":true,"folder":"exp2","code":"x, y = rbx.tex_coords(64)"},"providerIdentifier":"rbxkit","toolName":"rbx_texture","serverIdentifier":"rbxkit"},"result":{"success":{"content":[{"text":{"text":"{\n \"look_at\": \"/tmp/work/vfx/exp2/_check.png\"\n}"}},{"image":{"data":"iVBORw0KGgoAAAANSUhEUgAAAYAAAAGWCAIAAAD+","mimeType":"image/png"}},{"image":{"data":"iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAYAAACq","mimeType":"image/png"}}],"isError":false,"systemReminders":[]}},"description":"Draw 64x64 soft glow"},"hookAdditionalContexts":[],"toolCallId":"call-7fa2-15\nfc_27a3_0"},"session_id":"ff455855"}"#;
    const SHELL_REFUSED: &str = r#"{"type":"tool_call","subtype":"completed","call_id":"call-4ff8-3\nfc_80cf_1","tool_call":{"shellToolCall":{"args":{"command":"echo hello-from-shell","workingDirectory":"","timeout":30000,"simpleCommands":["echo"]},"result":{"rejected":{"command":"echo hello-from-shell","workingDirectory":"/tmp/ws","reason":"","isReadonly":false}}}},"session_id":"ff455855"}"#;
    const EDIT_DENIED: &str = r#"{"type":"tool_call","subtype":"completed","call_id":"call-e","tool_call":{"editToolCall":{"args":{"path":"/tmp/outside/denied/e.txt","streamContent":"e"},"result":{"writePermissionDenied":{"path":"/tmp/outside/denied/e.txt","error":"Write permission denied: /tmp/outside/denied/e.txt: Blocked by permissions configuration","isReadonly":false}}}},"session_id":"ff455855"}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","duration_ms":11395,"duration_api_ms":11395,"is_error":false,"result":"I'll call `rbx_vault` and run that shell echo in parallel, then reply in one short sentence.Called `rbx_vault`.","session_id":"ff455855-e83e-4480-9d3c-b80cfc773520","request_id":"5b0de173-4928-4e3c-8731-d934db95979d","usage":{"inputTokens":13200,"outputTokens":428,"cacheReadTokens":51840,"cacheWriteTokens":0,"reasoningTokens":199}}"#;

    fn feed(t: &mut Turn, line: &str) -> Vec<Out> {
        t.feed(&serde_json::from_str(line).unwrap())
    }

    #[test]
    fn a_turn_reads_like_the_chat_shows_it() {
        let mut t = Turn::default();
        let init = feed(&mut t, INIT);
        assert!(matches!(&init[0], Out::Conversation(id) if id == "ff455855-e83e-4480-9d3c-b80cfc773520"));
        assert!(matches!(&init[1], Out::Model(m) if m == "Cursor (Auto)"));
        assert!(feed(&mut t, THINKING).is_empty(), "thinking is not shown");
        assert!(matches!(&feed(&mut t, SAY)[0], Out::Text(s) if s.starts_with("I'll call")));
        assert!(feed(&mut t, DESCRIBE).is_empty(), "reading a tool's description is housekeeping");
        let start = feed(&mut t, MCP_START);
        assert!(matches!(&start[..], [Out::Tool(n)] if n == "mcp__rbxkit__rbx_vault"));
        assert!(feed(&mut t, MCP_DONE).is_empty(), "announced once; a text result has no pictures");
        let end = feed(&mut t, RESULT);
        let Out::Usage(u) = &end[0] else { panic!("usage first") };
        assert_eq!((u.input, u.output, u.cache_read, u.cache_write), (13200, 428, 51840, 0));
        assert!(!end.iter().any(|o| matches!(o, Out::Text(_))), "the result repeats what was already shown");
        assert!(!end.iter().any(|o| matches!(o, Out::Failed(_))));
        assert!(matches!(end.last(), Some(Out::Finished)));
    }

    #[test]
    fn pictures_from_our_tools_are_shown() {
        let mut t = Turn::default();
        let out = feed(&mut t, PICTURES);
        assert!(matches!(&out[0], Out::Tool(n) if n == "mcp__rbxkit__rbx_texture"));
        let Some(Out::Encoded(p)) = out.get(1) else { panic!("pictures expected") };
        assert_eq!(p.len(), 2);
        assert!(p[0].starts_with("iVBORw0KGgo"));
        // another server's pictures are not ours to show
        let other = PICTURES.replace("rbx_texture", "draw_thing");
        assert!(!feed(&mut Turn::default(), &other).iter().any(|o| matches!(o, Out::Encoded(_))));
    }

    #[test]
    fn a_refused_command_is_offered_to_allow() {
        let mut t = Turn::default();
        let out = feed(&mut t, SHELL_REFUSED);
        assert!(matches!(&out[..], [Out::Tool(n)] if n == "run_command"));
        assert!(feed(&mut t, &SHELL_REFUSED.replace("call-4ff8-3", "call-again")).len() == 1, "the same program is offered once");
        assert!(feed(&mut t, EDIT_DENIED).iter().all(|o| matches!(o, Out::Tool(n) if n == "Edit")), "a refused write is no program");
        let end = feed(&mut t, RESULT);
        let Some(Out::Blocked(list)) = end.iter().find(|o| matches!(o, Out::Blocked(_))) else { panic!("blocked expected") };
        assert_eq!(list.len(), 1);
        assert_eq!((list[0]["name"].as_str(), list[0]["command"].as_str()), (Some("echo"), Some("echo hello-from-shell")));
    }

    #[test]
    fn a_failed_result_counts_as_failed_and_an_unsaid_result_is_shown() {
        let failed = r#"{"type":"result","subtype":"error","is_error":true,"result":"You have hit your usage limit.","session_id":"s","usage":{}}"#;
        let out = feed(&mut Turn::default(), failed);
        assert!(out.iter().any(|o| matches!(o, Out::Failed(m) if m.contains("usage limit"))));
        assert!(!out.iter().any(|o| matches!(o, Out::Text(_))));
        let out = feed(&mut Turn::default(), RESULT);
        assert!(out.iter().any(|o| matches!(o, Out::Text(m) if m.ends_with("Called `rbx_vault`."))), "nothing was said, so the result is");
    }

    #[test]
    fn the_model_list_reads_from_its_lines() {
        let text = "Available models\n\nauto - Auto (current, default)\ngpt-5.3-codex-low - Codex 5.3 Low\nclaude-opus-5-5-high - Claude Opus 5.5 High\nnot a model line\n - nameless\n";
        let list = parse_models(text);
        assert_eq!(list[0], ("auto".to_string(), "Auto".to_string()));
        assert_eq!(list[1], ("gpt-5.3-codex-low".to_string(), "Codex 5.3 Low".to_string()));
        assert_eq!(list.len(), 3);
        assert_eq!(default_of(&list).as_deref(), Some("auto"));
        assert_eq!(default_of(&[]).as_deref(), Some("auto"), "Auto before the list arrives");
        assert_eq!(default_of(&list[1..]).as_deref(), Some("gpt-5.3-codex-low"));
    }

    #[test]
    fn the_command_line_carries_the_folders_model_and_session() {
        let a = args(Path::new("/w/cursor/chats/c1"), Path::new("/kit"), Some("/games/Odd One In é"), Some("abc"), Some("auto"), false);
        let s = a.join(" ");
        assert!(s.starts_with("-p --output-format stream-json --trust --approve-mcps --workspace /w/cursor/chats/c1"));
        assert!(s.contains("--add-dir /kit --add-dir /games/Odd One In é --model auto --resume abc"));
        assert!(!a.contains(&"--force".to_string()));
        let full = args(Path::new("/w"), Path::new("/kit"), None, None, None, true);
        assert_eq!(full.iter().filter(|x| *x == "--add-dir").count(), 1, "no game, only the kit");
        assert!(!full.contains(&"--resume".to_string()) && !full.contains(&"--model".to_string()));
        assert_eq!(full.last().map(String::as_str), Some("--force"));
    }

    #[test]
    fn permissions_allow_ours_and_deny_the_rest() {
        let servers = json!({"rbxkit": {"type": "stdio", "command": "py"}, "Roblox_Studio": {"type": "stdio", "command": "studio"}});
        let p = permissions(&servers, &["git".into()], &["rbxkit".into(), "notion".into(), "notion".into()], &[PathBuf::from("/w/cursor/config")]);
        let allow: Vec<&str> = p["allow"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        let deny: Vec<&str> = p["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(allow.contains(&"Mcp(rbxkit:*)") && allow.contains(&"Mcp(Roblox_Studio:*)") && allow.contains(&"Shell(git)"));
        assert_eq!(deny.iter().filter(|d| **d == "Mcp(notion:*)").count(), 1, "the user's own servers are denied once");
        assert!(!deny.contains(&"Mcp(rbxkit:*)"), "ours replaces theirs of the same name, so it is never denied");
        assert!(!deny.iter().any(|d| d.contains("studio")), "Studio is on: nothing named like it is denied");
        assert!(deny.contains(&"Write(**/.cursor/**)") && deny.contains(&"Write(/w/cursor/config/**)"));
        assert!(!allow.iter().any(|a| a.starts_with("Shell(*") || a.starts_with("Write")), "no blanket allow");
        assert!(allow.contains(&"WebFetch(*)"), "web pages, as Claude Code may fetch them");
    }

    #[test]
    fn no_studio_keeps_every_studio_server_away() {
        // FRIDAE_NO_STUDIO: the agent's servers have no Studio, and Cursor must not reach one set up elsewhere
        let servers = crate::agents::test_servers(true);
        assert!(servers.get("Roblox_Studio").is_none());
        let file = servers_file(&servers, &[]);
        assert_eq!(file["mcpServers"]["rbxkit"]["env"]["FRIDAE_NO_STUDIO"], "1");
        assert!(file["mcpServers"].get("Roblox_Studio").is_none());
        assert!(file["mcpServers"]["rbxkit"].get("type").is_none());
        let p = permissions(&servers, &[], &["Roblox_Studio".into()], &[]);
        let deny: Vec<&str> = p["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(deny.contains(&"Mcp(Roblox_Studio:*)") && deny.contains(&"Mcp(*studio*:*)") && deny.contains(&"Mcp(*Studio*:*)"));
        assert!(!p["allow"].as_array().unwrap().iter().any(|a| a.as_str().unwrap().to_lowercase().contains("studio")));
    }

    #[test]
    fn files_are_written_in_the_apps_folders_only() {
        let dir = std::env::temp_dir().join("fridae-tests").join(format!("cursor-{}", std::process::id())).join("Odd é");
        let ws = dir.join("chats").join("c1");
        write_workspace(&ws, &json!({"rbxkit": {"type": "stdio", "command": "py", "args": ["mcp.py"], "env": {"RBXKIT_AGENT_ID": "c1"}}}));
        let file: Value = serde_json::from_str(&fs::read_to_string(ws.join(".cursor").join("mcp.json")).unwrap()).unwrap();
        assert_eq!(file["mcpServers"]["rbxkit"]["args"][0], "mcp.py");
        assert!(ws.join(".git").is_dir(), "the workspace is its own project top");
        let cfg = dir.join("config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("cli-config.json"), r#"{"version": 1, "model": {"modelId": "default"}, "sandbox": {"networkAccess": "x"}, "permissions": {"allow": ["Shell(*)"]}}"#).unwrap();
        write_config(&cfg, &json!({"allow": ["Mcp(rbxkit:*)"], "deny": []}));
        let back: Value = serde_json::from_str(&fs::read_to_string(cfg.join("cli-config.json")).unwrap()).unwrap();
        assert_eq!(back["permissions"]["allow"], json!(["Mcp(rbxkit:*)"]), "the app's permissions replace any others");
        assert_eq!(back["model"]["modelId"], "default", "what Cursor keeps there stays");
        assert_eq!((back["sandbox"]["mode"].as_str(), back["sandbox"]["networkAccess"].as_str()), (Some("disabled"), Some("x")));
        assert_eq!(back["approvalMode"], "allowlist");
        assert_eq!(back["autoAcceptWebSearch"], true);
        assert_eq!(fs::read_dir(&cfg).unwrap().count(), 1, "no temporary file left behind");
        // the app's settings reach the servers (Cursor passes them only HOME and PATH); the entry's own values win
        let extra = [("RBXKIT_WORK".to_string(), "/scratch/work".to_string()), ("HOME".to_string(), "/scratch/home".to_string()), ("RBXKIT_AGENT_ID".to_string(), "x".to_string())];
        let pinned = servers_file(&json!({"rbxkit": {"command": "py", "env": {"RBXKIT_AGENT_ID": "c1"}}}), &extra);
        let env = &pinned["mcpServers"]["rbxkit"]["env"];
        assert_eq!((env["HOME"].as_str(), env["RBXKIT_WORK"].as_str(), env["RBXKIT_AGENT_ID"].as_str()), (Some("/scratch/home"), Some("/scratch/work"), Some("c1")));
        assert!(!PASSED.iter().any(|k| k.contains("KEY") || k.contains("TOKEN")), "no secrets go in the file");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn stderr_is_explained() {
        let out = "Error: Authentication required. Please run 'agent login' first, or set CURSOR_API_KEY environment variable.";
        assert!(explain(out, false).unwrap().contains("cursor-agent login"));
        assert!(explain(out, true).is_none(), "a turn that answered is not explained by stderr");
        assert!(explain("Cannot use this model: nonsense-model. Available models: auto, gpt-5.2", false).unwrap().contains("Auto always works"));
        assert!(explain("something else", false).is_none());
    }
}
