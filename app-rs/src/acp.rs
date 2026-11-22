//! The Agent Client Protocol (ACP), spoken to an AI kept open for a chat: JSON-RPC 2.0, one message per line over the
//! process's stdin and stdout, protocol version 1. Gemini runs this way through `agy acp` (checked against agy 1.5.14;
//! DEVELOPING.md has what was measured, and the recordings are `tests/fixtures/gemini/acp-*.wire`). The process and
//! its pipes are `live::Live`; keeping it, and falling back to a process per message, is `agents.rs`, as for Claude.
//!
//! What goes over the wire, in order:
//! - `initialize`, once per process. The app says it offers no file system and no terminal of its own, so the agent
//!   edits files and runs commands itself (and asks first, below).
//! - `session/new`, or `session/resume` for a chat that has a conversation (agy keeps both kinds in one store, so a
//!   conversation a process per message made carries on here, and the other way round). Then `session/set_mode
//!   default`, the model and the effort. The mode must be `default`: in any other the agent approves some steps by
//!   itself. A session whose mode is seen to be anything else is ended.
//! - `session/prompt`. The answer to that request is the end of the turn (`stopReason`, `usage`). Meanwhile the agent
//!   sends `session/update` notifications, which `Turn` turns into the same `Out` values the per-message parser gives
//!   (`antigravity::Turn`), so the chat, the cards, usage and the turn's ending are the same code either way.
//! - `session/request_permission`, a request from the agent, before every tool call (measured: agy's own allow and
//!   deny rules are not applied over ACP). The app is the gate: `decide` answers each one by the app's own rules,
//!   and anything it cannot read or does not know is refused. A refusal ends the turn (measured).
//! - `session/cancel`, a notification, for Stop. The prompt is then answered `cancelled` (measured: 6 ms) and the
//!   process stays for the next message.
//!
//! Ids are kept as they come (agy numbers its requests with strings).

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::{Arc, Weak},
};

use serde_json::{Value, json};

use crate::{
    antigravity::{self, Out},
    live::Live,
    opencode, paths, usage,
};

/// The protocol version the app speaks; an agent that answers another is not used this way.
pub const PROTOCOL: u64 = 1;

// ------------------------------------------------------------------ the gate

/// What a chat's turn may do, for the app's answers to the agent's permission requests. Built afresh for every
/// message (`agents::GeminiLaunch`), so a turn is never judged by an earlier message's mode or folders; the programs
/// the user allowed are read from the app's memory at each request.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Full auto or Studio: everything is let through except writes to the app's own files.
    pub full: bool,
    /// The chat, for commands the user said may run once (`opencode::approve_once`).
    pub agent_id: String,
    /// The app's tool servers by name: their tools are always let through.
    pub servers: Vec<String>,
    /// Folders the file tools may write in (the chat's workspace: the game, or the chat's own folder).
    pub writes: Vec<PathBuf>,
    /// Folders they may also read (the kit, the user's attachments, the chat's pictures).
    pub reads: Vec<PathBuf>,
    /// The app's own files and folders, never written by an agent's file tools (`paths::app_protected`).
    pub protected: Vec<PathBuf>,
    /// The agent's own data folder (`~/.gemini/antigravity-cli`): it keeps a scratch folder per conversation there,
    /// and reads its tool and skill descriptions from it.
    pub agent_dir: PathBuf,
}

/// The app's answer to one permission request, and what the chat is shown when it is a refusal.
pub struct Verdict {
    pub allow: bool,
    pub outs: Vec<Out>,
}

/// The agent's own tools that change files, and those that only read them, by the names agy gives them. A tool of
/// neither list is told by its `kind` (ACP's own: `edit`, `delete`, `move`; `read`, `search`).
const WRITERS: &[&str] = &["write_file", "write_to_file", "replace_file_content", "multi_replace_file_content", "edit_file", "delete_file"];
const READERS: &[&str] = &["view_file", "read_file", "list_dir", "grep_search", "find_by_name", "view_file_outline", "code_search"];
/// Tools that look something up on the web, as Claude Code's WebSearch and WebFetch do in Ask me.
const FETCHERS: &[&str] = &["search_web", "read_url_content"];

/// A path as the checks compare it: absolute, no `..`, symbolic links followed as far as the path exists (a Mac's
/// /tmp is /private/tmp). None for anything else: such a path cannot be judged, so the request is refused.
fn resolve(path: &str) -> Option<PathBuf> {
    let p = match path.strip_prefix("~/") {
        Some(rest) => paths::home_dir().join(rest),
        None => PathBuf::from(path),
    };
    if !p.is_absolute() || path.contains('\0') || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(paths::real_as_far_as_exists(&p))
}

fn inside(p: &Path, dir: &Path) -> bool {
    let dir = paths::real_as_far_as_exists(dir);
    !dir.as_os_str().is_empty() && p.starts_with(&dir)
}

/// Whether a path is one of the app's own files, or in one of its folders, in any case of letters: on a disk that
/// ignores case `CONFIG.JSON` is `config.json`, and where it does not, refusing the other spelling costs nothing.
fn protected(p: &Path, list: &[PathBuf]) -> bool {
    let low = |x: &Path| x.to_string_lossy().to_lowercase();
    let p = low(p);
    list.iter().any(|q| {
        let q = low(&paths::real_as_far_as_exists(q));
        !q.is_empty() && (p == q || p.starts_with(&(q.trim_end_matches('/').to_string() + "/")))
    })
}

/// Whether text reads as a path: absolute, from the home folder, or with a Windows drive.
fn path_like(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/') || s.starts_with('~') || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// An argument that holds what is written, not where: never read as a path, whatever it starts with.
fn content_key(key: &str) -> bool {
    let k = key.to_lowercase();
    ["content", "text", "chunk", "code", "summary", "description", "instruction", "query", "pattern", "commandline"].iter().any(|w| k.contains(w))
}

/// An argument that names a file or folder, by its name (`Path`, `TargetFile`, `DirectoryPath`, `SearchPath`...).
fn path_key(key: &str) -> bool {
    let k = key.to_lowercase();
    ["path", "paths", "file", "files", "dir", "directory", "folder", "uri"].iter().any(|w| k.ends_with(w))
}

/// Every file or folder a tool call names: its `locations`, the files of the changes it shows (`content` of type
/// `diff`), each argument named as a path, and any other argument that reads as one. Err when one of them cannot be
/// judged (not absolute, `..` in it, a path argument that is not text).
fn named_paths(tool: &Value) -> Result<Vec<PathBuf>, ()> {
    let mut texts: Vec<String> = Vec::new();
    for l in tool["locations"].as_array().into_iter().flatten() {
        texts.push(l["path"].as_str().ok_or(())?.to_string());
    }
    for c in tool["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "diff") {
        texts.push(c["path"].as_str().ok_or(())?.to_string());
    }
    fn walk(v: &Value, key: &str, depth: usize, out: &mut Vec<String>) -> Result<(), ()> {
        match v {
            Value::String(s) if path_key(key) => {
                if !s.is_empty() {
                    out.push(s.clone());
                }
            }
            Value::String(s) if !content_key(key) && path_like(s) => out.push(s.clone()),
            Value::Array(items) if depth < 4 => {
                for i in items {
                    walk(i, key, depth + 1, out)?;
                }
            }
            Value::Object(map) if depth < 4 => {
                for (k, v) in map {
                    walk(v, k, depth + 1, out)?;
                }
            }
            // a path argument that is a number or true, or arguments nested too deep to read
            Value::Number(_) | Value::Bool(_) if path_key(key) => return Err(()),
            Value::Array(_) | Value::Object(_) => return Err(()),
            _ => {}
        }
        Ok(())
    }
    walk(&tool["rawInput"], "", 0, &mut texts)?;
    let mut out: Vec<PathBuf> = Vec::new();
    for t in texts {
        // a web address under a key such as `Uri` is not a file
        if t.starts_with("http://") || t.starts_with("https://") {
            continue;
        }
        let p = resolve(&t).ok_or(())?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// A tool call's name for the chat: its title as the agent gives it, cut short, with nothing that does not show.
fn shown(tool: &Value) -> String {
    let t = tool["title"].as_str().filter(|t| !t.trim().is_empty()).or(tool["name"].as_str()).unwrap_or("something");
    opencode::visible(&t.chars().take(120).collect::<String>())
}

/// One of the app's tool servers' tools: agy names them `mcp_<server>_<tool>`. Returns (server, tool).
fn server_tool<'a>(name: &'a str, servers: &'a [String]) -> Option<(&'a str, &'a str)> {
    let rest = name.strip_prefix("mcp_")?;
    // the longest server name first, so `Roblox_Studio` is never read as a server `Roblox`
    let mut by_len: Vec<&String> = servers.iter().collect();
    by_len.sort_by_key(|s| std::cmp::Reverse(s.len()));
    by_len.into_iter().find_map(|s| rest.strip_prefix(s.as_str()).and_then(|t| t.strip_prefix('_')).filter(|t| !t.is_empty()).map(|t| (s.as_str(), t)))
}

const STOPPED_THERE: &str = "Gemini stopped there.";

/// The app's answer to a permission request's `toolCall`, for a turn under `rules`, in the conversation `session`.
/// `allowed`: the programs the user allowed, from the app's memory now.
///
/// Full auto and Studio: everything is let through except a file tool's write to one of the app's own files (in any
/// case of letters), or a write that does not say where. Ask me: the app's tool servers' tools; a command whose
/// programs the user allowed, written plainly (`paths::command_allowed`), or one the user said may run once; a write
/// inside the chat's workspace and not to the app's files; a read inside the chat's folders; a web lookup. Everything
/// else is refused, and so is any request that cannot be read: no tool call, no id, a path that cannot be judged.
pub fn decide(tool: &Value, rules: &Rules, session: &str, allowed: &[String]) -> Verdict {
    let allow = || Verdict { allow: true, outs: Vec::new() };
    let refuse = |why: String| Verdict { allow: false, outs: vec![Out::Error(why)] };
    if !tool.is_object() || tool["toolCallId"].as_str().is_none_or(str::is_empty) {
        return refuse(format!("Gemini asked to do something Fridae could not read, so it was refused. {STOPPED_THERE}"));
    }
    let what = shown(tool);
    let name = tool["name"].as_str().unwrap_or("");
    let kind = tool["kind"].as_str().unwrap_or("");
    let raw = &tool["rawInput"];
    if !(raw.is_object() || raw.is_null()) {
        return refuse(format!("Gemini asked to use \"{what}\" in a way Fridae could not read, so it was refused. {STOPPED_THERE}"));
    }
    if server_tool(name, &rules.servers).is_some() {
        return allow();
    }
    let command = raw["CommandLine"].as_str();
    let is_command = command.is_some() || kind == "execute" || name == "run_command";
    let writes = matches!(kind, "edit" | "delete" | "move") || WRITERS.contains(&name);
    let reads = !writes && (matches!(kind, "read" | "search") || READERS.contains(&name));
    let named = named_paths(tool);
    let own = |p: &Path| protected(p, &rules.protected);
    let own_file = "one of Fridae's own files, which no AI's file tools may change";
    if rules.full {
        if is_command {
            return allow();
        }
        return match named {
            Ok(_) if reads => allow(),
            Err(()) if reads => allow(),
            Ok(paths) if writes && paths.is_empty() => refuse(format!("Gemini asked to change a file without saying which (\"{what}\"), so it was refused. {STOPPED_THERE}")),
            Ok(paths) => match paths.iter().find(|p| own(p)) {
                Some(p) => refuse(format!("Gemini tried to write {}, {own_file}, so it was refused. {STOPPED_THERE}", p.display())),
                None => allow(),
            },
            Err(()) => refuse(format!("Gemini asked to use \"{what}\" on a path Fridae could not check, so it was refused. {STOPPED_THERE}")),
        };
    }
    if is_command {
        let Some(command) = command.filter(|c| !c.trim().is_empty()) else {
            return refuse(format!("Gemini asked to run a command Fridae could not read (\"{what}\"), so it was refused. {STOPPED_THERE}"));
        };
        let request = [command.to_string()];
        let d = opencode::decide("shell", &request, allowed);
        if d.once || opencode::take_once(&rules.agent_id, "shell", &request, allowed) {
            return allow();
        }
        let mut outs = Vec::new();
        if !d.blocked.is_empty() {
            outs.push(Out::Blocked(d.blocked));
        }
        if !d.refused.is_empty() {
            opencode::note_refused(&rules.agent_id, &d.refused);
            outs.push(Out::Refused(d.refused));
        }
        if outs.is_empty() {
            outs.push(Out::Error(format!("Gemini tried to run a command that is not allowed here (\"{what}\"). {STOPPED_THERE}")));
        }
        return Verdict { allow: false, outs };
    }
    let Ok(paths) = named else {
        return refuse(format!("Gemini asked to use \"{what}\" on a path Fridae could not check, so it was refused. {STOPPED_THERE}"));
    };
    // its own scratch folder for this conversation, and what it reads of its own tools and skills
    let scratch = rules.agent_dir.join("brain").join(session);
    let own_scratch = |p: &Path| !session.is_empty() && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && inside(p, &scratch);
    if writes {
        if paths.is_empty() {
            return refuse(format!("Gemini asked to change a file without saying which (\"{what}\"), so it was refused. {STOPPED_THERE}"));
        }
        if let Some(p) = paths.iter().find(|p| own(p)) {
            return refuse(format!("Gemini tried to write {}, {own_file}, so it was refused. {STOPPED_THERE}", p.display()));
        }
        return match paths.iter().find(|p| !(rules.writes.iter().any(|d| inside(p, d)) || own_scratch(p))) {
            Some(p) => refuse(format!("Gemini tried to write {}, which is outside this chat's folder, so it was refused. {STOPPED_THERE} In Ask me it writes only in the chat's own folder or its game.", p.display())),
            None => allow(),
        };
    }
    if reads {
        let own_reads = [rules.agent_dir.join("mcp"), rules.agent_dir.join("builtin")];
        let may = |p: &Path| rules.writes.iter().chain(&rules.reads).chain(&own_reads).any(|d| inside(p, d)) || own_scratch(p);
        if paths.is_empty() {
            return refuse(format!("Gemini asked to read something without saying where (\"{what}\"), so it was refused. {STOPPED_THERE}"));
        }
        return match paths.iter().find(|p| !may(p)) {
            Some(p) => refuse(format!("Gemini tried to read {}, which is outside this chat's folders, so it was refused. {STOPPED_THERE}", p.display())),
            None => allow(),
        };
    }
    if (kind == "fetch" || FETCHERS.contains(&name)) && paths.is_empty() {
        return allow();
    }
    // the output of a command the app let run
    if name == "command_status" && paths.is_empty() {
        return allow();
    }
    refuse(format!("Gemini asked to use \"{what}\", which Fridae does not let it do in Ask me, so it was refused. {STOPPED_THERE} Ask again, and tell it to use the building tools."))
}

/// The answer to send for a permission request: the option of the right kind among those the agent offers (agy:
/// `allow`/`deny`; other agents name them otherwise, but all give the `kind`). Never an "always" option: each call is
/// asked about. When the agent offers no way to say no, the request is answered as cancelled; when it offers no way
/// to say yes once, the answer is no.
fn answer(options: &Value, allow: bool) -> Value {
    let pick = |kind: &str| options.as_array().into_iter().flatten().find(|o| o["kind"] == kind).and_then(|o| o["optionId"].as_str()).map(str::to_string);
    let chosen = if allow { pick("allow_once") } else { None }.or_else(|| pick("reject_once"));
    match chosen {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

// ------------------------------------------------------------------ a turn's updates

/// Whether a line is work the agent began by itself when it comes with no turn of the app's on the process: the
/// agent speaking, a tool call starting, or a permission request (`live::Live` ends the process at such a line).
/// Measured: a turn ended with a command still running in the background, and when the command ended agy asked the
/// model again and wrote an answer nobody had asked for (`tests/fixtures/gemini/acp-background.wire`). What trails a
/// turn (a tool's last `tool_call_update`, usage, the title) is not such work.
pub fn stray(line: &[u8]) -> bool {
    let Ok(m) = serde_json::from_slice::<Value>(line) else { return false };
    let update = m["params"]["update"]["sessionUpdate"].as_str().unwrap_or("");
    m["method"] == "session/request_permission" || (m["method"] == "session/update" && matches!(update, "tool_call" | "agent_message_chunk" | "agent_thought_chunk"))
}

/// The mode a session is in, as a list of its options says (`configOptions`), when it says.
fn mode_in(options: &Value) -> Option<&str> {
    options.as_array()?.iter().find(|o| o["id"] == "mode")?.get("currentValue")?.as_str()
}

/// Reads one turn's `session/update`s into what the chat shows. Text arrives in pieces and is shown whole: before the
/// next tool call, and at the turn's end.
pub struct Turn {
    servers: Vec<String>,
    text: String,
    seen: HashSet<String>,
    open: HashSet<String>,
    /// the agent said something or called a tool
    pub began: bool,
    said: bool,
    /// a request of this turn was refused by the app (the agent then ends the turn without a word)
    pub refused: bool,
    /// the session was seen in a mode other than `default`
    pub mode_lost: bool,
}

impl Turn {
    pub fn new(servers: &[String]) -> Turn {
        Turn { servers: servers.to_vec(), text: String::new(), seen: HashSet::new(), open: HashSet::new(), began: false, said: false, refused: false, mode_lost: false }
    }

    pub fn tool_open(&self) -> bool {
        !self.open.is_empty()
    }

    fn flush(&mut self, out: &mut Vec<Out>) {
        let text = std::mem::take(&mut self.text);
        if !text.trim().is_empty() {
            self.said = true;
            out.push(Out::Text(text.trim_end().to_string()));
        }
    }

    /// One `session/update` (its `update` part).
    pub fn feed(&mut self, u: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        match u["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" => {
                if let Some(piece) = u["content"]["text"].as_str().filter(|_| u["content"]["type"] == "text") {
                    self.began = true;
                    self.text.push_str(piece);
                }
            }
            "tool_call" | "tool_call_update" => {
                let Some(id) = u["toolCallId"].as_str().filter(|s| !s.is_empty()) else { return out };
                self.began = true;
                let name = u["name"].as_str().filter(|n| !n.is_empty()).or(u["title"].as_str()).unwrap_or("");
                let servers = self.servers.clone();
                let ours = server_tool(name, &servers);
                if self.seen.insert(id.to_string()) {
                    self.flush(&mut out);
                    // it reads each tool's description file before calling it: not worth a line in the chat
                    let housekeeping = u["rawInput"]["Path"].as_str().or(u["locations"][0]["path"].as_str()).is_some_and(|p| p.replace('\\', "/").contains("/antigravity-cli/mcp/"));
                    match ours {
                        // named as Claude Code, Cursor and OpenCode name a server's tool, with what it was asked
                        Some((server, tool)) => out.push(Out::ToolArgs(format!("mcp__{server}__{tool}"), u["rawInput"].clone())),
                        None if !name.is_empty() && !housekeeping => out.push(Out::Tool(name.to_string())),
                        None => {}
                    }
                }
                match u["status"].as_str().unwrap_or("") {
                    "completed" | "failed" => {
                        self.open.remove(id);
                        // a tool's result arrives as text, when it arrives: the pictures the app's tools made are
                        // found by the paths they report
                        let shows = ours.is_some_and(|(_, t)| t.starts_with("rbx_") || t.starts_with("screen_capture"));
                        if shows && u["status"] == "completed" {
                            let mut text = u["rawOutput"]["output"].as_str().unwrap_or("").to_string();
                            for c in u["content"].as_array().into_iter().flatten() {
                                text.push('\n');
                                text.push_str(c["content"]["text"].as_str().unwrap_or(""));
                            }
                            let pictures = antigravity::pictures_in(&text);
                            if !pictures.is_empty() {
                                out.push(Out::Pictures(pictures));
                            }
                        }
                    }
                    _ => {
                        self.open.insert(id.to_string());
                    }
                }
            }
            "config_option_update" => {
                if mode_in(&u["configOptions"]).is_some_and(|m| m != "default") {
                    self.mode_lost = true;
                }
            }
            "current_mode_update" => {
                if u["currentModeId"].as_str().is_some_and(|m| m != "default") {
                    self.mode_lost = true;
                }
            }
            _ => {}
        }
        out
    }

    /// The answer to the turn's `session/prompt`: its end.
    pub fn finish(&mut self, answer: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        self.flush(&mut out);
        if let Some(e) = answer.get("error") {
            let said = e["message"].as_str().unwrap_or("").trim();
            out.push(Out::Failed(if signed_out(e) { antigravity::SIGNED_OUT.to_string() } else if said.is_empty() { "Gemini could not run this message.".to_string() } else { format!("Gemini could not run this message: {said}") }));
            out.push(Out::Finished);
            return out;
        }
        let r = &answer["result"];
        if r["usage"].is_object() {
            out.push(Out::Usage(usage::Turn::from_agy_acp(&r["usage"])));
        }
        match r["stopReason"].as_str().unwrap_or("") {
            // agy ends a turn at once, without a word, when a request is refused (measured): the card or the line
            // that went with the refusal says why. Otherwise a turn with nothing in it is said.
            "end_turn" => {
                if !self.said && !self.refused && self.seen.is_empty() {
                    out.push(Out::Error("Gemini ended this turn without saying anything.".into()));
                }
            }
            "cancelled" => {}
            // the one stop that is not a normal end (read in t3code's Antigravity adapter, not measured here): the
            // plan's usage is spent. Said in words `agents::usage_limit` knows, so a studio run pauses on it.
            "refusal" => out.push(Out::Failed("Gemini refused this message: its usage limit was reached, or it would not answer. Try again once the limit resets.".into())),
            other => out.push(Out::Error(format!("Gemini stopped: {}.", if other.is_empty() { "no reason given" } else { other }))),
        }
        out.push(Out::Finished);
        out
    }
}

/// agy's answer when nobody is signed in: `{code: -32000, message: "Authentication required"}` (measured).
fn signed_out(error: &Value) -> bool {
    let m = error["message"].as_str().unwrap_or("").to_lowercase();
    m.contains("authentication required") || m.contains("not authenticated") || m.contains("please sign in") || m.contains("login required")
}

// ------------------------------------------------------------------ the client

/// What a kept ACP process is given for a message.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// The chat's workspace: where the agent works, and its tool servers are started.
    pub cwd: PathBuf,
    /// The app's tool servers, as the app keeps them: {name: {command, args, env: {..}}}.
    pub servers: Value,
    /// The model as the per-message path names it (agy's own id, effort and all), and the effort setting.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub rules: Rules,
}

/// The tool servers as `session/new` takes them: {name, command, args, env: [{name, value}]}. A texture request may
/// wait minutes for the user, so each is given the per-message path's time limit (`timeoutSeconds`: taken by agy's
/// `session/new`, measured; whether it is kept to was not).
pub fn servers_param(servers: &Value) -> Value {
    let list: Vec<Value> = servers
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, s)| {
            let env: Vec<Value> = s["env"].as_object().into_iter().flatten().map(|(k, v)| json!({"name": k, "value": v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())})).collect();
            json!({"name": name, "command": s["command"], "args": s["args"].as_array().cloned().unwrap_or_default(), "env": env, "timeoutSeconds": 900})
        })
        .collect();
    Value::Array(list)
}

/// What the client needs of the chat's turn it runs for.
pub trait Host {
    /// Something for the chat.
    fn out(&mut self, out: Out);
    /// The process answered a request: it speaks the protocol (the start watchdog stops waiting).
    fn heard(&mut self);
    /// The agent has started on the turn's message: from now Stop asks it to cancel instead of ending the process.
    fn taken(&mut self);
    /// A line came: whether it started one of the app's tools that wait long on purpose, and whether a tool call is
    /// open (the stall guard, and what Stop does with the process).
    fn activity(&mut self, long_tool: bool, tool_open: bool);
    /// The agent said something or called a tool: the message is never sent a second time.
    fn began(&mut self);
    /// End the process now, with all it started.
    fn end(&mut self);
}

/// How a turn on a kept ACP process ended.
#[derive(Default, Debug, PartialEq)]
pub struct End {
    /// the process's output ended, or the process was ended, before the prompt was answered
    pub ended: bool,
    /// this process cannot serve the message as a process per message would (the model or effort is not offered)
    pub unfit: bool,
    /// the turn ended with a tool call still open: a command it left running in the background. The agent starts
    /// work by itself when such a command ends (measured), so the process is not kept.
    pub background: bool,
}

/// A process's session, kept on it between turns.
struct Session {
    id: String,
}

enum CallError {
    /// the output ended
    Closed,
    /// the agent answered with an error
    Rpc(Value),
}

struct Client<'a> {
    live: &'a Arc<Live>,
    host: &'a mut dyn Host,
}

impl Client<'_> {
    fn send(&self, m: &Value) -> bool {
        self.live.write(m).is_ok()
    }

    /// A request, and its answer. What comes meanwhile is not a turn's: a request from the agent is refused, and a
    /// session seen in another mode than `default` is given up (as the output ending).
    fn call(&mut self, rx: &std::sync::mpsc::Receiver<crate::live::Line>, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.live.next_request();
        if !self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            return Err(CallError::Closed);
        }
        loop {
            let Ok(line) = rx.recv() else { return Err(CallError::Closed) };
            let Some(m) = line.and_then(|l| serde_json::from_slice::<Value>(&l).ok()) else { continue };
            self.host.activity(false, false);
            if m["method"].is_string() {
                if !m["id"].is_null() {
                    self.refuse_request(&m);
                } else if mode_in(&m["params"]["update"]["configOptions"]).is_some_and(|mode| mode != "default") && method != "session/set_mode" {
                    return Err(CallError::Closed);
                }
                continue;
            }
            if m["id"] == json!(id) {
                self.host.heard();
                return match m.get("error") {
                    Some(e) => Err(CallError::Rpc(e.clone())),
                    None => Ok(m["result"].clone()),
                };
            }
        }
    }

    /// A request from the agent that is not answered by the gate: outside a turn, for another session, or of a kind
    /// the app does not offer. Never left unanswered (the agent would wait), never a yes.
    fn refuse_request(&self, m: &Value) {
        let reply = if m["method"] == "session/request_permission" {
            json!({"jsonrpc": "2.0", "id": m["id"], "result": answer(&m["params"]["options"], false)})
        } else {
            json!({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": "Fridae Studios does not offer this."}})
        };
        self.send(&reply);
    }
}

const LOST_CONVERSATION: &str = "Gemini could not find this chat's earlier conversation, so it started a new one and does not remember the chat so far. Tell it again what matters.";

/// One turn on a kept ACP process: on its first, the handshake and the session (`resume`: the chat's conversation, if
/// it has one); then the message, and the agent's updates and requests until the prompt is answered.
pub fn turn(live: &Arc<Live>, spec: &Spec, resume: Option<&str>, text: &str, host: &mut dyn Host) -> End {
    let rx = live.lines();
    let mut c = Client { live, host };
    let gone = |c: &mut Client, unfit: bool| {
        c.host.end();
        End { ended: true, unfit, background: false }
    };
    let known = live.state().and_then(|s| s.downcast::<Session>().ok());
    let session = match known {
        Some(s) => {
            c.host.heard(); // it has answered before
            s
        }
        None => {
            let hello = json!({"protocolVersion": PROTOCOL, "clientInfo": {"name": "fridae-studios", "version": paths::VERSION},
                               "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}});
            match c.call(&rx, "initialize", hello) {
                Ok(r) if r["protocolVersion"] == json!(PROTOCOL) => {}
                _ => return gone(&mut c, false),
            }
            let place = |extra: Value| {
                let mut p = json!({"cwd": spec.cwd, "mcpServers": servers_param(&spec.servers)});
                for (k, v) in extra.as_object().into_iter().flatten() {
                    p[k] = v.clone();
                }
                p
            };
            let mut opened = match resume {
                Some(id) => match c.call(&rx, "session/resume", place(json!({"sessionId": id}))) {
                    // a conversation agy no longer has: a new one, and the chat is told (as a process per message
                    // does, which starts one without a word)
                    Err(CallError::Rpc(e)) if !signed_out(&e) => {
                        c.host.out(Out::Error(LOST_CONVERSATION.into()));
                        c.call(&rx, "session/new", place(json!({})))
                    }
                    other => other,
                },
                None => c.call(&rx, "session/new", place(json!({}))),
            };
            // a resumed session's answer names no id of its own when the agent leaves it out: it is the one asked for
            if let (Ok(r), Some(id)) = (&mut opened, resume) {
                if r["sessionId"].is_null() && r.is_object() {
                    r["sessionId"] = json!(id);
                }
            }
            let id = match opened {
                Ok(r) => match r["sessionId"].as_str().filter(|s| !s.is_empty()) {
                    Some(id) => id.to_string(),
                    None => return gone(&mut c, false),
                },
                // nobody is signed in: said as the per-message path says it, and no second process is tried
                Err(CallError::Rpc(e)) if signed_out(&e) => {
                    c.host.out(Out::Failed(antigravity::SIGNED_OUT.into()));
                    c.host.out(Out::Finished);
                    return End::default();
                }
                Err(_) => return gone(&mut c, false),
            };
            // the app is the gate only in `default`: any other mode approves some steps by itself
            let mut settled = match c.call(&rx, "session/set_mode", json!({"sessionId": id, "modeId": "default"})) {
                Ok(_) => Value::Null,
                Err(_) => return gone(&mut c, false),
            };
            if let Some(model) = &spec.model {
                match c.call(&rx, "session/set_config_option", json!({"sessionId": id, "configId": "model", "value": model})) {
                    Ok(r) => settled = r,
                    Err(CallError::Rpc(_)) => return gone(&mut c, true),
                    Err(CallError::Closed) => return gone(&mut c, false),
                }
            }
            if let Some(effort) = &spec.effort {
                match c.call(&rx, "session/set_config_option", json!({"sessionId": id, "configId": "thought_level", "value": effort})) {
                    Ok(r) => settled = r,
                    Err(CallError::Rpc(_)) => return gone(&mut c, true),
                    Err(CallError::Closed) => return gone(&mut c, false),
                }
            }
            if mode_in(&settled["configOptions"]).is_some_and(|m| m != "default") {
                return gone(&mut c, false);
            }
            let s = Arc::new(Session { id });
            live.set_state(s.clone());
            // Stop: `session/cancel`, a notification; the prompt is then answered `cancelled`
            let (weak, sid): (Weak<Live>, String) = (Arc::downgrade(live), s.id.clone());
            live.set_stopper(Box::new(move || weak.upgrade().is_some_and(|l| l.write(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}})).is_ok())));
            s
        }
    };
    c.host.out(Out::Conversation(session.id.clone()));
    if let Some(m) = &spec.model {
        c.host.out(Out::Model(antigravity::model_name(m)));
    }
    let prompt = live.next_request();
    if !c.send(&json!({"jsonrpc": "2.0", "id": prompt, "method": "session/prompt", "params": {"sessionId": session.id, "prompt": [{"type": "text", "text": text}]}})) {
        return gone(&mut c, false);
    }
    let mut t = Turn::new(&spec.rules.servers);
    loop {
        let Ok(line) = rx.recv() else {
            // it died, or was ended: what it had written so far is still shown
            let mut left = Vec::new();
            t.flush(&mut left);
            for o in left {
                c.host.out(o);
            }
            return End { ended: true, ..End::default() };
        };
        let Some(m) = line.and_then(|l| serde_json::from_slice::<Value>(&l).ok()) else { continue };
        let mut long_tool = false;
        if m["method"] == "session/request_permission" && !m["id"].is_null() {
            let p = &m["params"];
            if p["sessionId"] != session.id.as_str() {
                c.refuse_request(&m);
                continue;
            }
            c.host.taken();
            c.host.began();
            let v = decide(&p["toolCall"], &spec.rules, &session.id, &paths::allowed_commands());
            if !v.allow {
                t.refused = true;
            }
            for o in v.outs {
                c.host.out(o);
            }
            if !c.send(&json!({"jsonrpc": "2.0", "id": m["id"], "result": answer(&p["options"], v.allow)})) {
                return gone(&mut c, false);
            }
        } else if m["method"].is_string() && !m["id"].is_null() {
            c.refuse_request(&m);
        } else if m["method"] == "session/update" {
            let u = &m["params"]["update"];
            if m["params"]["sessionId"] != session.id.as_str() {
                continue;
            }
            // the agent's own work on the message, not what a prompt's arrival alone sets off
            let work = matches!(u["sessionUpdate"].as_str().unwrap_or(""), "agent_message_chunk" | "agent_thought_chunk" | "tool_call" | "tool_call_update") || u["used"].as_u64().is_some_and(|n| n > 0);
            if work {
                c.host.taken();
            }
            let outs = t.feed(u);
            if t.began {
                c.host.began();
            }
            long_tool = outs.iter().any(|o| matches!(o, Out::Tool(n) | Out::ToolArgs(n, _) if paths::waits_long(n)));
            for o in outs {
                c.host.out(o);
            }
            if t.mode_lost {
                c.host.out(Out::Error("Gemini left the mode in which it asks Fridae before each step, so Fridae ended this turn. Send your message again.".into()));
                return gone(&mut c, false);
            }
        } else if m["id"] == json!(prompt) && (m.get("result").is_some() || m.get("error").is_some()) {
            c.host.heard();
            for o in t.finish(&m) {
                c.host.out(o);
            }
            c.host.activity(false, false);
            return End { background: t.tool_open(), ..End::default() };
        }
        c.host.activity(long_tool, t.tool_open());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(full: bool) -> (Rules, PathBuf) {
        let root = paths::test_dir("acp-gate");
        for d in ["ws/sub", "kit", "work/chats", "gemini/antigravity-cli/brain/s-1", "gemini/antigravity-cli/mcp/rbxkit", "attachments"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("kit/config.json"), "{}").unwrap();
        let r = Rules {
            full,
            agent_id: "gate-chat".into(),
            servers: vec!["rbxkit".into(), "Roblox_Studio".into()],
            writes: vec![root.join("ws")],
            reads: vec![root.join("kit"), root.join("attachments")],
            protected: vec![root.join("kit/config.json"), root.join("work/chats"), root.join("work/usage.jsonl")],
            agent_dir: root.join("gemini/antigravity-cli"),
        };
        (r, root)
    }

    fn call(name: &str, kind: &str, raw: Value, locations: &[&Path]) -> Value {
        json!({"toolCallId": "call_1", "name": name, "kind": kind, "title": name, "status": "pending", "rawInput": raw,
               "locations": locations.iter().map(|p| json!({"path": p})).collect::<Vec<_>>()})
    }

    fn write(path: &Path) -> Value {
        call("write_file", "edit", json!({"Content": "/* not a path */ x", "Path": path}), &[path])
    }

    fn command(line: &str) -> Value {
        call("run_command", "execute", json!({"CommandLine": line, "WaitMsBeforeAsync": 500}), &[])
    }

    fn said(v: &Verdict) -> String {
        v.outs.iter().map(|o| match o {
            Out::Error(t) => t.clone(),
            Out::Blocked(c) => format!("blocked {}", Value::Array(c.clone())),
            Out::Refused(c) => format!("refused {}", Value::Array(c.clone())),
            _ => String::new(),
        }).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn ask_me_lets_through_the_apps_tools_and_writes_in_the_workspace_only() {
        let (r, root) = rules(false);
        let ok = |t: &Value| decide(t, &r, "s-1", &[]).allow;
        // the app's own tool servers, by agy's names for their tools (as recorded: `mcp_probe_probe_echo`)
        assert!(ok(&call("mcp_rbxkit_rbx_blender", "other", json!({"action": "build"}), &[])));
        assert!(ok(&call("mcp_Roblox_Studio_run_code", "other", json!({}), &[])));
        // another server's tool (one of the user's own in agy's settings) is not the app's to allow
        let v = decide(&call("mcp_other_do", "other", json!({}), &[]), &r, "s-1", &[]);
        assert!(!v.allow && said(&v).contains("does not let it do in Ask me"), "{}", said(&v));
        // a write inside the workspace, a new file in a folder of it; content that starts like a path is no path
        assert!(ok(&write(&root.join("ws/note.txt"))));
        assert!(ok(&write(&root.join("ws/sub/deep/new.lua"))));
        // outside it: the kit, the home folder, another chat's folder
        for p in ["kit/readme.md", "other/x.txt", "ws-other/x.txt"] {
            let v = decide(&write(&root.join(p)), &r, "s-1", &[]);
            assert!(!v.allow && said(&v).contains("outside this chat's folder"), "{p}: {}", said(&v));
        }
        // its own scratch folder for this conversation, and no other conversation's
        assert!(ok(&write(&root.join("gemini/antigravity-cli/brain/s-1/task.md"))));
        assert!(!ok(&write(&root.join("gemini/antigravity-cli/brain/s-2/task.md"))));
        assert!(!ok(&write(&root.join("gemini/antigravity-cli/settings.json"))));
        // reads: the workspace, the kit, attachments, its own tool descriptions; nothing else
        let read = |p: &str| call("view_file", "read", json!({"AbsolutePath": root.join(p)}), &[&root.join(p)]);
        for p in ["ws/a.lua", "kit/guide.md", "attachments/a.png", "gemini/antigravity-cli/mcp/rbxkit/x.json"] {
            assert!(ok(&read(p)), "{p}");
        }
        assert!(!ok(&read("elsewhere/secret.txt")));
        assert!(!ok(&call("view_file", "read", json!({}), &[])), "a read that does not say where");
        // a web lookup, and the output of a command already let run
        assert!(ok(&call("search_web", "fetch", json!({"query": "roblox /tmp"}), &[])));
        assert!(ok(&call("command_status", "other", json!({"CommandId": "c1"}), &[])));
        // anything else the app does not know
        assert!(!ok(&call("browser_subagent", "other", json!({"Task": "open a page"}), &[])));
        assert!(!ok(&call("generate_image", "other", json!({"Prompt": "a cat"}), &[])));
    }

    #[test]
    fn a_path_that_cannot_be_judged_and_a_request_that_cannot_be_read_are_refused_in_every_mode() {
        for full in [false, true] {
            let (r, root) = rules(full);
            let no = |t: &Value| !decide(t, &r, "s-1", &["git".to_string()]).allow;
            assert!(no(&Value::Null), "no tool call at all");
            assert!(no(&json!({"name": "write_file", "kind": "edit"})), "no id");
            assert!(no(&json!({"toolCallId": "c", "name": "write_file", "kind": "edit", "rawInput": "text"})), "arguments that are no object");
            // a write that names no file, a relative path, `..`, a path that is a number
            assert!(no(&call("write_file", "edit", json!({"Content": "x"}), &[])));
            assert!(no(&call("write_file", "edit", json!({"Path": "note.txt"}), &[])));
            assert!(no(&write(&root.join("ws/../kit/config.json"))));
            assert!(no(&call("write_file", "edit", json!({"Path": 7}), &[])));
            // a location without a path
            assert!(no(&json!({"toolCallId": "c", "name": "write_file", "kind": "edit", "rawInput": {}, "locations": [{"line": 3}]})));
            // a tool the app does not know that names one of the app's files under an argument of any name
            assert!(no(&call("move_thing", "other", json!({"Destination": root.join("kit/config.json")}), &[])));
        }
    }

    #[test]
    fn the_apps_own_files_are_refused_in_any_case_of_letters_in_full_auto_too() {
        for full in [false, true] {
            let (mut r, root) = rules(full);
            // even were the kit a folder the chat may write in
            r.writes.push(root.join("kit"));
            r.writes.push(root.join("work"));
            for p in ["kit/config.json", "kit/CONFIG.JSON", "kit/Config.Json", "work/chats/c1.json", "work/CHATS/c1.json", "work/usage.jsonl", "work/USAGE.JSONL"] {
                let v = decide(&write(&root.join(p)), &r, "s-1", &[]);
                assert!(!v.allow && said(&v).contains("Fridae's own files"), "{full} {p}: {}", said(&v));
            }
            // by any tool that changes files, and when only a `diff` names the file
            assert!(!decide(&call("replace_file_content", "edit", json!({"TargetFile": root.join("kit/CONFIG.json")}), &[]), &r, "s-1", &[]).allow);
            let mut by_diff = call("some_editor", "edit", json!({}), &[&root.join("ws/a.txt")]);
            by_diff["content"] = json!([{"type": "diff", "path": root.join("work/chats/c2.json"), "newText": "x"}]);
            assert!(!decide(&by_diff, &r, "s-1", &[]).allow);
            // a file beside them is not theirs
            assert!(decide(&write(&root.join("kit/notes.md")), &r, "s-1", &[]).allow);
        }
    }

    #[test]
    fn full_auto_lets_everything_else_through() {
        let (r, root) = rules(true);
        let ok = |t: &Value| decide(t, &r, "s-1", &[]).allow;
        assert!(ok(&write(&root.join("elsewhere/x.txt"))), "outside the workspace");
        assert!(ok(&command("rm -rf build && ./tool $(date)")), "any command, as before");
        assert!(ok(&call("browser_subagent", "other", json!({"Task": "open a page"}), &[])));
        assert!(ok(&call("view_file", "read", json!({"AbsolutePath": root.join("kit/config.json")}), &[])), "reading is not writing");
        assert!(ok(&call("mcp_rbxkit_rbx_project", "other", json!({}), &[])));
    }

    #[test]
    fn ask_me_runs_only_commands_the_strict_check_passes_and_shows_a_card_for_the_rest() {
        let (r, _) = rules(false);
        let git = vec!["git".to_string()];
        assert!(decide(&command("git status"), &r, "s-1", &git).allow);
        assert!(decide(&command("git log --oneline 2>&1 | git stripspace"), &r, "s-1", &git).allow);
        // a program the user has not allowed: the card that offers to allow it
        let v = decide(&command("touch denied.txt"), &r, "s-1", &git);
        assert!(!v.allow && matches!(&v.outs[..], [Out::Blocked(c)] if c[0]["name"] == "touch" && c[0]["command"] == "touch denied.txt"), "{}", said(&v));
        // an allowed program in a form the strict check refuses: the card that says why and offers it once
        for bad in ["./git status", "git status > out.txt", "git log $(whoami)", "FOO=1 git status", "git status; rm -rf x", "git -c core.pager=less log"] {
            let v = decide(&command(bad), &r, "s-1", &git);
            assert!(!v.allow, "{bad}");
            assert!(v.outs.iter().any(|o| matches!(o, Out::Refused(_) | Out::Blocked(_))), "{bad}: {}", said(&v));
        }
        // a program that reads other programs' settings never runs, and is not offered
        let v = decide(&command("ps eww"), &r, "s-1", &["ps".to_string()]);
        assert!(!v.allow && matches!(&v.outs[..], [Out::Refused(c)] if c[0]["once"] == false), "{}", said(&v));
        // a command that cannot be read
        assert!(!decide(&call("run_command", "execute", json!({}), &[]), &r, "s-1", &git).allow);
        assert!(!decide(&call("run_command", "execute", json!({"CommandLine": "  "}), &[]), &r, "s-1", &git).allow);
        // "Run it once": the exact command a card of this chat showed, one time
        let v = decide(&command("./git status"), &r, "s-1", &git);
        assert!(!v.allow);
        assert!(opencode::approve_once("gate-chat", "./git status"));
        assert!(decide(&command("./git status"), &r, "s-1", &git).allow);
        assert!(!decide(&command("./git status"), &r, "s-1", &git).allow, "used up");
    }

    #[test]
    fn the_answer_is_the_option_of_the_right_kind_and_never_an_always() {
        // agy's options, as recorded
        let agy = json!([{"optionId": "allow", "name": "Allow", "kind": "allow_once"}, {"optionId": "deny", "name": "Deny", "kind": "reject_once"}]);
        assert_eq!(answer(&agy, true), json!({"outcome": {"outcome": "selected", "optionId": "allow"}}));
        assert_eq!(answer(&agy, false), json!({"outcome": {"outcome": "selected", "optionId": "deny"}}));
        // another agent's names (Cursor's, as recorded): told by kind
        let cursor = json!([{"optionId": "allow-once", "kind": "allow_once"}, {"optionId": "allow-always", "kind": "allow_always"}, {"optionId": "reject-once", "kind": "reject_once"}]);
        assert_eq!(answer(&cursor, true)["outcome"]["optionId"], "allow-once");
        assert_eq!(answer(&cursor, false)["outcome"]["optionId"], "reject-once");
        // only an "always" on offer: no; nothing to say no with: cancelled
        let always = json!([{"optionId": "a", "kind": "allow_always"}, {"optionId": "r", "kind": "reject_once"}]);
        assert_eq!(answer(&always, true)["outcome"]["optionId"], "r");
        assert_eq!(answer(&json!([{"optionId": "a", "kind": "allow_always"}]), true), json!({"outcome": {"outcome": "cancelled"}}));
        assert_eq!(answer(&Value::Null, false), json!({"outcome": {"outcome": "cancelled"}}));
    }

    #[test]
    fn tool_servers_go_in_as_a_list_with_their_environment_as_pairs() {
        let p = servers_param(&json!({"rbxkit": {"type": "stdio", "command": "python3", "args": ["mcp.py"], "env": {"RBXKIT_AGENT_ID": "c1", "FRIDAE_GEMINI_USE_API": "0"}}}));
        assert_eq!(p[0]["name"], "rbxkit");
        assert_eq!(p[0]["command"], "python3");
        assert_eq!(p[0]["args"], json!(["mcp.py"]));
        assert!(p[0]["env"].as_array().unwrap().contains(&json!({"name": "RBXKIT_AGENT_ID", "value": "c1"})));
        assert!(p[0].get("type").is_none());
        assert_eq!(servers_param(&json!({})), json!([]));
    }

    #[test]
    fn what_trails_a_turn_is_not_taken_for_work_begun_alone_but_a_tool_call_or_a_request_is() {
        assert!(stray(br#"{"jsonrpc":"2.0","method":"session/request_permission","params":{},"id":"4"}"#));
        assert!(stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"c"}}}"#));
        assert!(stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}}}}"#));
        assert!(!stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","status":"failed"}}}"#));
        assert!(!stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"usage_update","used":3}}}"#));
        assert!(!stray(br#"{"jsonrpc":"2.0","result":{"stopReason":"end_turn"},"id":7}"#));
        assert!(!stray(b"not json"));
    }

    #[test]
    fn a_refusal_is_a_usage_limit_in_words_the_run_loop_knows_and_other_stops_are_said() {
        let outs = |answer: Value| Turn::new(&[]).finish(&answer);
        let text = |o: &[Out]| o.iter().filter_map(|o| match o { Out::Failed(t) | Out::Error(t) => Some(t.clone()), _ => None }).collect::<Vec<_>>().join("|");
        let o = outs(json!({"result": {"stopReason": "refusal"}}));
        assert!(matches!(&o[..], [Out::Failed(t), Out::Finished] if t.contains("usage limit")), "{}", text(&o));
        let o = outs(json!({"result": {"stopReason": "max_tokens", "usage": {"inputTokens": 5, "outputTokens": 1, "thoughtTokens": 2, "cachedReadTokens": 3}}}));
        assert!(matches!(&o[..], [Out::Usage(u), Out::Error(t), Out::Finished] if t == "Gemini stopped: max_tokens." && (u.input, u.output, u.cache_read) == (5, 3, 3)));
        let o = outs(json!({"error": {"code": -32000, "message": "Authentication required"}}));
        assert!(matches!(&o[..], [Out::Failed(t), Out::Finished] if t == antigravity::SIGNED_OUT));
        let o = outs(json!({"result": {"stopReason": "end_turn"}}));
        assert!(text(&o).contains("without saying anything"));
        assert!(matches!(&outs(json!({"result": {"stopReason": "cancelled"}}))[..], [Out::Finished]));
    }
}
