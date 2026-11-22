//! The Agent Client Protocol (ACP), spoken to an AI kept open for a chat: JSON-RPC 2.0, one message per line over the
//! process's stdin and stdout, protocol version 1. Gemini can run this way through `agy acp` (on request only for
//! now, `agents::KEPT_ON_REQUEST`; checked against agy 1.5.14;
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
    collections::{HashMap, HashSet},
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

/// Which agent a kept ACP process is. The protocol is the same; the places that ask this are what differs: how a
/// session is opened again, the model's id, who the gate is, what a tool call looks like, usage, how it is ended.
///
/// Cursor (`cursor-agent acp`, checked against 2026.11.22; DEVELOPING.md has what was measured, the recordings are
/// `tests/fixtures/cursor/acp-*.wire`):
/// - a chat's session is opened again with `session/load` (there is no `session/resume`), which plays the history
///   back before it answers; an unknown one is `-32602 Invalid params`, `data.message: Session "<id>" not found`;
/// - there is no mode to set (`agent` is the one a session starts in), and the model takes only the ids the session
///   lists (`default[]` is Auto);
/// - **Cursor's own allow list and deny rules are the gate**, as on a process per message: what they let through
///   runs unasked, and `session/request_permission` comes only for what they do not (a program not on the list, a
///   write outside the workspace folders, another server's tool). The app answers every one of those no: a program
///   becomes the blocked card. A refusal does not end the turn (the model is told and carries on);
/// - a server's tool is named by `rawInput.providerIdentifier` and `toolName`, in the update after the `tool_call`;
/// - the prompt's answer has no usage (measured: none anywhere), so each turn is noted as not reported;
/// - Cursor's sandbox is **not** applied over ACP (measured), so only Ask me runs this way (`agents::CursorLaunch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Agent {
    #[default]
    Agy,
    Cursor,
}

// ------------------------------------------------------------------ the gate

/// What a chat's turn may do, for the app's answers to the agent's permission requests. Built afresh for every
/// message (`agents::GeminiLaunch`), so a turn is never judged by an earlier message's mode or folders; the programs
/// the user allowed are read from the app's memory at each request.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Full auto or Studio: everything is let through except writes to what no AI's file tools may write.
    pub full: bool,
    /// The chat, for what the user said may run once (`opencode::approve_once`).
    pub agent_id: String,
    /// The app's tool servers by name: their tools are always let through.
    pub servers: Vec<String>,
    /// Folders the file tools may write in (the chat's workspace: the game, or the chat's own folder). The first is
    /// the workspace.
    pub writes: Vec<PathBuf>,
    /// Folders they may also read (the kit, the user's attachments, the chat's pictures).
    pub reads: Vec<PathBuf>,
    /// What no AI's file tools may write in any mode (`paths::never_written`, the list OpenCode's rules are made
    /// from too): the app's own files and folders, and the workspace's list of tool servers.
    pub protected: Vec<PathBuf>,
    /// Folders whose hidden files and folders are never written either (the home folder: shell start-up files,
    /// other tools' settings).
    pub hidden_in: Vec<PathBuf>,
    /// The agent's own data folder (`~/.gemini/antigravity-cli`): it keeps a scratch folder per conversation there.
    pub agent_dir: PathBuf,
}

/// The app's answer to one permission request, what the chat is shown when it is a refusal, and a word for the agent
/// with the user's next message (agy takes no reason with a refusal and ends the turn).
pub struct Verdict {
    pub allow: bool,
    pub outs: Vec<Out>,
    pub note: Option<String>,
}

/// The agent's own tools that change files, and that only read them, by the names agy gives them: only names seen
/// in a recording (`tests/fixtures/gemini/acp-*.wire`). A tool of neither list is told by its `kind` (ACP's own:
/// `edit`, `delete`, `move`; `read`, `search`; `fetch`), and one the app cannot place is refused in Ask me.
const WRITERS: &[&str] = &["write_file"];
const READERS: &[&str] = &["read_file"];

/// Whether the app approves a command by itself in Ask me when every program in it is one the user allowed
/// (`paths::command_allowed`). That check is written for `/bin/sh`, so it holds only where agy runs commands with
/// that shell and no start-up file of the user's: see DEVELOPING.md for what was measured.
pub const COMMANDS_BY_APP: bool = false;

/// Whether agy asks before it reads a file outside the workspace (measured, DEVELOPING.md). Where it does not, the
/// app cannot keep its reads to the chat's folders, so a web lookup is not let through without the user either:
/// what was read anywhere could leave with it.
pub const READS_ASKED: bool = true;

/// A path as the checks compare it: absolute, symbolic links followed as far as the path exists (a Mac's /tmp is
/// /private/tmp). Against `base` (Full auto: the workspace) a relative path is taken from there and `..` is worked
/// out; without one, a relative path or one with `..` cannot be judged: None, and the request is refused.
fn resolve(path: &str, base: Option<&Path>) -> Option<PathBuf> {
    let mut p = match path.strip_prefix("~/") {
        Some(rest) => paths::home_dir().join(rest),
        None => PathBuf::from(path),
    };
    if path.contains('\0') || path.is_empty() {
        return None;
    }
    if !p.is_absolute() {
        p = base?.join(p);
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        base?;
        let mut flat = PathBuf::new();
        for c in p.components() {
            match c {
                Component::ParentDir => {
                    flat.pop();
                }
                other => flat.push(other),
            }
        }
        p = flat;
    }
    Some(paths::real_as_far_as_exists(&p))
}

fn inside(p: &Path, dir: &Path) -> bool {
    let dir = paths::real_as_far_as_exists(dir);
    !dir.as_os_str().is_empty() && p.starts_with(&dir)
}

fn low(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// `p` is `q` or inside it, in any case of letters.
fn under(p: &str, q: &str) -> bool {
    !q.is_empty() && (p == q || p.starts_with(&(q.trim_end_matches('/').to_string() + "/")))
}

/// Why a path is one no file tool may write, if it is: one of the protected places or inside one, in any case of
/// letters (on a disk that ignores case `CONFIG.JSON` is `config.json`, and where it does not, refusing the other
/// spelling costs nothing); a hidden name directly in a folder of `hidden_in`, or that folder's `Library`; or a
/// with `holding`, a place that *holds* one of these (a delete or a move of `work` takes `work/chats` with it).
fn never_written(p: &Path, rules: &Rules, holding: bool) -> bool {
    let lp = low(p);
    let places: Vec<String> = rules.protected.iter().map(|q| low(&paths::real_as_far_as_exists(q))).collect();
    if places.iter().any(|q| under(&lp, q) || (holding && under(q, &lp))) {
        return true;
    }
    rules.hidden_in.iter().any(|h| {
        let h = paths::real_as_far_as_exists(h);
        let lh = low(&h);
        if lh.is_empty() {
            return false;
        }
        if holding && under(&lh, &lp) {
            return true; // the folder itself, or one that holds it
        }
        match Path::new(&lp).strip_prefix(&lh).ok().and_then(|rest| rest.components().next()) {
            Some(Component::Normal(first)) => first.to_string_lossy().starts_with('.') || first == "library",
            _ => false,
        }
    })
}

/// A folder no file tool writes under in Ask me, anywhere in the path and in any case (`paths::ASK_ME_NEVER_UNDER`:
/// `.git`, where a hook or an alias would run when an allowed `git` runs), if the path has one.
fn ask_me_never(p: &Path) -> Option<&'static str> {
    p.components().find_map(|c| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        paths::ASK_ME_NEVER_UNDER.iter().copied().find(|n| *n == name)
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

/// An argument that names a file or folder, by its name as agy writes them (`Path`, `TargetFile`, `SearchPath`) or
/// with an underscore (`file_path`); `profile` is not one.
fn path_key(key: &str) -> bool {
    let k = key.to_lowercase();
    ["Path", "Paths", "File", "Files", "Dir", "Directory", "Folder", "Uri"].iter().any(|w| {
        let l = w.to_lowercase();
        key.ends_with(w) || k == l || k.ends_with(&format!("_{l}"))
    })
}

/// Every file or folder a tool call names: its `locations`, the files of the changes it shows (`content` of type
/// `diff`), each argument named as a path, and any other argument that reads as one. Err when one of them cannot be
/// judged (see `resolve`; a path argument that is not text).
fn named_paths(tool: &Value, base: Option<&Path>) -> Result<Vec<PathBuf>, ()> {
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
        let p = resolve(&t, base).ok_or(())?;
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

/// One of the app's tool servers' tools: agy names them `mcp_<server>_<tool>` (recorded for both of the app's
/// servers). Returns (server, tool). The app's own server's tools all begin `rbx_`, so another server whose name
/// only starts like it (`rbxkit_x`) is not taken for it.
fn server_tool<'a>(name: &'a str, servers: &'a [String]) -> Option<(&'a str, &'a str)> {
    let rest = name.strip_prefix("mcp_")?;
    // the longest server name first, so `Roblox_Studio` is never read as a server `Roblox`
    let mut by_len: Vec<&String> = servers.iter().collect();
    by_len.sort_by_key(|s| std::cmp::Reverse(s.len()));
    by_len
        .into_iter()
        .find_map(|s| rest.strip_prefix(s.as_str()).and_then(|t| t.strip_prefix('_')).filter(|t| !t.is_empty()).map(|t| (s.as_str(), t)))
        .filter(|(server, tool)| *server != "rbxkit" || tool.starts_with("rbx_"))
}

const STOPPED_THERE: &str = "Gemini stopped there.";

/// The card for something the app does not let through by itself but the user may, one time: {command, shown,
/// reason, once, no_once}, as `opencode::decide` makes them for a command.
fn once_card(agent_id: &str, text: &str, reason: &str) -> Out {
    let no = opencode::not_offered(text);
    let card = vec![json!({"command": text, "shown": opencode::visible(text), "reason": reason, "once": no.is_none(), "no_once": no})];
    opencode::note_refused(agent_id, &card);
    Out::Refused(card)
}

/// What a web lookup asks for, as its card shows it and its one-time approval is kept by.
fn lookup_text(tool: &Value) -> String {
    let raw = &tool["rawInput"];
    let what = ["Url", "url", "query", "Query"].iter().find_map(|k| raw[*k].as_str()).map(str::to_string).unwrap_or_else(|| raw.to_string());
    format!("web lookup: {}", what.chars().take(250).collect::<String>())
}

/// The app's answer to a permission request's `toolCall`, for a turn under `rules`, in the conversation `session`.
/// `allowed`: the programs the user allowed, from the app's memory now.
///
/// Every mode: a request that cannot be read is refused (no tool call, no id, arguments that are no object); so is a
/// change of the session's mode; the app's tool servers' tools pass.
///
/// Full auto and Studio: everything else passes except a file tool's write to what no file tool may write
/// (`never_written`), a write that does not say where, and a tool that names a path the app cannot judge.
///
/// Ask me: a command passes when the user said that exact command may run once, or, where the app may approve
/// commands by itself (`COMMANDS_BY_APP`), when every program in it is one the user allowed, written plainly
/// (`paths::command_allowed`); a write passes inside the chat's workspace, or the agent's own scratch folder for
/// this conversation, and never under `.git` or to what no file tool may write; a read, should the agent ask,
/// inside the chat's folders; a web lookup only where the agent's reads are asked about (`READS_ASKED`), else when
/// the user said that lookup may run once. Everything else is refused.
pub fn decide(tool: &Value, rules: &Rules, session: &str, allowed: &[String]) -> Verdict {
    let allow = || Verdict { allow: true, outs: Vec::new(), note: None };
    let refuse = |why: String| Verdict { allow: false, note: Some(format!("[Fridae refused a step of your last turn and the turn ended there: {why}]")), outs: vec![Out::Error(why)] };
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
    // the app answers requests only in the mode in which every step asks
    if kind == "switch_mode" {
        return refuse(format!("Gemini asked to change how it asks before its steps (\"{what}\"), which Fridae never lets it, so it was refused. {STOPPED_THERE}"));
    }
    if server_tool(name, &rules.servers).is_some() {
        return allow();
    }
    let command = raw["CommandLine"].as_str();
    let is_command = command.is_some() || kind == "execute" || name == "run_command";
    let writes = matches!(kind, "edit" | "delete" | "move") || WRITERS.contains(&name);
    let reads = !writes && (matches!(kind, "read" | "search") || READERS.contains(&name));
    let workspace = rules.writes.first().map(PathBuf::as_path);
    let named = named_paths(tool, if rules.full { workspace } else { None });
    // its own scratch folder for this conversation
    let scratch = rules.agent_dir.join("brain").join(session);
    let own_scratch = |p: &Path| !session.is_empty() && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && inside(p, &scratch);
    let own = |p: &Path| !own_scratch(p) && never_written(p, rules, true);
    let own_file = "which no AI's file tools may change (Fridae's own files, hidden files in your home folder)";
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
        if opencode::take_once(&rules.agent_id, "shell", &request, if COMMANDS_BY_APP { allowed } else { &[] }) {
            return allow();
        }
        let d = opencode::decide("shell", &request, allowed);
        if d.once && COMMANDS_BY_APP {
            return allow();
        }
        let mut outs = Vec::new();
        if d.once {
            // its programs are allowed, but here the app does not approve a command by itself
            outs.push(once_card(&rules.agent_id, command, "Fridae cannot check how Gemini's own shell would run it, so on Gemini every command is asked about, also for programs you have allowed."));
        }
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
        return Verdict { allow: false, outs, note: Some(format!("[Fridae did not run this command of your last turn, and the turn ended there: {}. The user was shown it and may allow it. Do not try it another way.]", command.chars().take(300).collect::<String>())) };
    }
    let Ok(paths) = named else {
        return refuse(format!("Gemini asked to use \"{what}\" on a path Fridae could not check, so it was refused. {STOPPED_THERE}"));
    };
    if writes {
        if paths.is_empty() {
            return refuse(format!("Gemini asked to change a file without saying which (\"{what}\"), so it was refused. {STOPPED_THERE}"));
        }
        if let Some(p) = paths.iter().find(|p| own(p)) {
            return refuse(format!("Gemini tried to write {}, {own_file}, so it was refused. {STOPPED_THERE}", p.display()));
        }
        if let Some((p, n)) = paths.iter().filter(|p| !own_scratch(p)).find_map(|p| ask_me_never(p).map(|n| (p, n))) {
            return refuse(format!("Gemini tried to write {}, inside a {n} folder, which no file tool may change in Ask me (what is written there can run when an allowed program does), so it was refused. {STOPPED_THERE}", p.display()));
        }
        return match paths.iter().find(|p| !(rules.writes.iter().any(|d| inside(p, d)) || own_scratch(p))) {
            Some(p) => refuse(format!("Gemini tried to write {}, which is outside this chat's folder, so it was refused. {STOPPED_THERE} In Ask me it writes only in the chat's own folder or its game.", p.display())),
            None => allow(),
        };
    }
    if reads {
        // never the app's own settings, other chats or the copies' folder, wherever the chat's folders lie
        let may = |p: &Path| (rules.writes.iter().chain(&rules.reads).any(|d| inside(p, d)) && !never_written(p, rules, false)) || own_scratch(p);
        if paths.is_empty() {
            return refuse(format!("Gemini asked to read something without saying where (\"{what}\"), so it was refused. {STOPPED_THERE}"));
        }
        return match paths.iter().find(|p| !may(p)) {
            Some(p) => refuse(format!("Gemini tried to read {}, which is outside this chat's folders, so it was refused. {STOPPED_THERE}", p.display())),
            None => allow(),
        };
    }
    if kind == "fetch" && paths.is_empty() {
        if READS_ASKED {
            return allow();
        }
        let text = lookup_text(tool);
        if opencode::take_exact(&rules.agent_id, &text) {
            return allow();
        }
        let card = once_card(&rules.agent_id, &text, "on this connection Gemini can read files anywhere on this computer without asking, so what it sends to the web is asked about each time.");
        return Verdict { allow: false, outs: vec![card], note: Some("[Fridae did not let your last turn look something up on the web, and the turn ended there. The user was shown it and may allow it.]".into()) };
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
        None => cancelled(),
    }
}

/// The answer to a request that came after Stop, or that the agent gave no way to say no to.
fn cancelled() -> Value {
    json!({"outcome": {"outcome": "cancelled"}})
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
    agent: Agent,
    /// Full auto or Studio (Cursor: a refused program gets no card there)
    full: bool,
    /// Cursor: each tool call's kind and command as its updates gave them, the calls already shown in the chat, the
    /// app's tools by call, and the programs refused in this turn (the card, shown at the turn's end)
    kinds: HashMap<String, String>,
    commands: HashMap<String, String>,
    named: HashSet<String>,
    tools: HashMap<String, String>,
    blocked: Vec<Value>,
    servers: Vec<String>,
    text: String,
    seen: HashSet<String>,
    open: HashSet<String>,
    /// open calls of the app's tools that wait long on purpose (`paths::waits_long`)
    long: HashSet<String>,
    /// where the app's tool server notes the pictures of each result (`Spec::pictures`), and how many of its lines
    /// have been shown
    pictures: Option<PathBuf>,
    pictures_seen: usize,
    /// the agent said something or called a tool
    pub began: bool,
    said: bool,
    /// a request of this turn was refused by the app (the agent then ends the turn without a word)
    pub refused: bool,
    /// the session was seen in a mode other than `default`
    pub mode_lost: bool,
}

impl Turn {
    #[cfg(test)]
    pub fn new(servers: &[String], pictures: Option<&Path>) -> Turn {
        Turn::of(Agent::Agy, false, servers, pictures)
    }

    pub fn of(agent: Agent, full: bool, servers: &[String], pictures: Option<&Path>) -> Turn {
        Turn { agent, full, kinds: HashMap::new(), commands: HashMap::new(), named: HashSet::new(), tools: HashMap::new(), blocked: Vec::new(), servers: servers.to_vec(), text: String::new(), seen: HashSet::new(), open: HashSet::new(), long: HashSet::new(), pictures: pictures.map(Path::to_path_buf), pictures_seen: 0, began: false, said: false, refused: false, mode_lost: false }
    }

    pub fn tool_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// One of the app's tools that wait long on purpose is open (the stall guard then does not count the quiet).
    pub fn long_open(&self) -> bool {
        !self.long.is_empty()
    }

    /// The pictures the app's tool server noted since the last look: one line per tool result, `{"images": [..]}`.
    /// A tool server's result does not come over ACP (measured), so `rbxkit_mcp.py` writes the image files of each
    /// result here (FRIDAE_TOOL_PICTURES). Shown as the per-message path shows them (`pictures_in`: files that
    /// exist, the ones for looking at first, two at most). A file that is missing or does not read shows nothing.
    fn new_pictures(&mut self) -> Vec<String> {
        let Some(text) = self.pictures.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
        let lines: Vec<&str> = text.lines().collect();
        let fresh = lines.get(self.pictures_seen..).unwrap_or(&[]);
        self.pictures_seen = lines.len();
        let named: Vec<String> = fresh
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .flat_map(|v| v["images"].as_array().cloned().unwrap_or_default())
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect();
        antigravity::pictures_in(&named.join("\n"))
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
            // Cursor thinks before each of its messages, also between two with no tool call the client is told of
            // (recorded: a step of its own it does not show): what it wrote before is a message of its own, shown as
            // one, as a process per message shows each
            "agent_thought_chunk" if self.agent == Agent::Cursor => self.flush(&mut out),
            "tool_call" | "tool_call_update" if self.agent == Agent::Cursor => self.cursor_tool(u, &mut out),
            "config_option_update" | "current_mode_update" if self.agent == Agent::Cursor => {}
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
                        self.long.remove(id);
                        // the pictures the app's own tools made, from the tool server's notes (a failed call's are
                        // passed over, so they are never taken for the next call's)
                        if ours.is_some_and(|(_, t)| t.starts_with("rbx_")) {
                            let pictures = self.new_pictures();
                            if !pictures.is_empty() && u["status"] == "completed" {
                                out.push(Out::Pictures(pictures));
                            }
                        }
                    }
                    _ => {
                        self.open.insert(id.to_string());
                        if ours.is_some_and(|(_, t)| paths::waits_long(t)) {
                            self.long.insert(id.to_string());
                        }
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

    /// Cursor's tool call or its update (recorded shapes, `tests/fixtures/cursor/acp-*.wire`). A call is told by its
    /// `kind` when it comes (`execute` with `rawInput.command`, `edit`, `read`, `search`...); a tool server's comes as
    /// `kind: other`, `title: "MCP: tool"` and is named only by the update after it (`rawInput: {providerIdentifier,
    /// toolName, args}`), so each is shown once, as soon as it can be named, by the names a process per message
    /// gives (`cursor::Turn`). Its own bookkeeping (reading a tool's description, to-dos) is not sent at all.
    fn cursor_tool(&mut self, u: &Value, out: &mut Vec<Out>) {
        let Some(id) = u["toolCallId"].as_str().filter(|s| !s.is_empty()) else { return };
        self.began = true;
        if self.seen.insert(id.to_string()) {
            self.flush(out);
        }
        if let Some(k) = u["kind"].as_str().filter(|k| !k.is_empty()) {
            self.kinds.insert(id.to_string(), k.to_string());
        }
        let raw = &u["rawInput"];
        if let Some(c) = raw["command"].as_str().filter(|c| !c.trim().is_empty()) {
            self.commands.insert(id.to_string(), c.to_string());
        }
        if let Some(tool) = raw["toolName"].as_str().filter(|t| !t.is_empty()) {
            let server = raw["providerIdentifier"].as_str().or(raw["serverIdentifier"].as_str()).unwrap_or("");
            self.tools.insert(id.to_string(), if server.is_empty() { tool.to_string() } else { format!("mcp__{server}__{tool}") });
        }
        let ours = self.tools.get(id).and_then(|n| n.strip_prefix("mcp__")).and_then(|n| n.split_once("__")).filter(|(server, _)| self.servers.iter().any(|s| s == server)).map(|(_, tool)| tool.to_string());
        if !self.named.contains(id) {
            let shown = match (self.tools.get(id), self.kinds.get(id).map(String::as_str).unwrap_or("")) {
                (Some(name), _) => Some(Out::ToolArgs(name.clone(), raw["args"].clone())),
                (None, "execute") => Some(Out::Tool("run_command".into())),
                (None, "edit") => Some(Out::Tool("Edit".into())),
                (None, "delete") => Some(Out::Tool("Delete".into())),
                (None, "move") => Some(Out::Tool("Move".into())),
                (None, "read") => Some(Out::Tool("Read".into())),
                (None, "search") => Some(Out::Tool("Grep".into())),
                (None, "fetch") => Some(Out::Tool("WebFetch".into())),
                // `other` before its name has come, or its thinking: nothing yet
                _ => None,
            };
            if let Some(o) = shown {
                self.named.insert(id.to_string());
                out.push(o);
            }
        }
        match u["status"].as_str().unwrap_or("") {
            "completed" | "failed" => {
                self.open.remove(id);
                self.long.remove(id);
                if ours.as_deref().is_some_and(|t| t.starts_with("rbx_")) {
                    let pictures = self.new_pictures();
                    if !pictures.is_empty() && u["status"] == "completed" {
                        out.push(Out::Pictures(pictures));
                    }
                }
            }
            _ => {
                self.open.insert(id.to_string());
                if ours.as_deref().is_some_and(paths::waits_long) {
                    self.long.insert(id.to_string());
                }
            }
        }
    }

    /// Cursor's permission request, which comes only for what its own allow list and rules did not let through
    /// (measured: a program not on the list, with the text `Not in allowlist: <program>`; a file-tool write outside
    /// the workspace folders, with the file in a `diff`). The answer is always no, as a process per message refuses
    /// the same things; a program is kept for the card that offers to allow it (Ask me), shown at the turn's end. The
    /// request has no `rawInput`: the command is the one the call's `tool_call` gave, else its title, which is the
    /// command between backquotes.
    fn cursor_request(&mut self, tool: &Value) -> Verdict {
        let id = tool["toolCallId"].as_str().unwrap_or("");
        let kind = tool["kind"].as_str().or(self.kinds.get(id).map(String::as_str)).unwrap_or("");
        if kind == "execute" && !self.full {
            let titled = tool["title"].as_str().map(|t| t.trim()).map(|t| t.strip_prefix('`').and_then(|t| t.strip_suffix('`')).unwrap_or(t).to_string());
            let command = self.commands.get(id).cloned().or(titled).unwrap_or_default();
            if let Some(name) = paths::program_name(&command).filter(|n| !self.blocked.iter().any(|b| b["name"] == *n)) {
                self.blocked.push(json!({"name": name, "command": command.chars().take(200).collect::<String>()}));
            }
        }
        self.refused = true;
        Verdict { allow: false, outs: Vec::new(), note: None }
    }

    /// Whether a Cursor request is let through (never), for tests outside this file.
    #[cfg(test)]
    pub fn cursor_request_for_test(&mut self, tool: &Value) -> bool {
        self.cursor_request(tool).allow
    }

    /// The end of a Cursor turn: the card for the programs it was refused, and the turn noted as one whose usage
    /// Cursor did not report (the prompt's answer is `{stopReason}` alone).
    fn cursor_finish(&mut self, answer: &Value, out: &mut Vec<Out>) {
        if !self.blocked.is_empty() {
            out.push(Out::Blocked(std::mem::take(&mut self.blocked)));
        }
        if let Some(e) = answer.get("error") {
            let said = said_in(e);
            out.push(Out::Failed(if signed_out(e) { crate::cursor::SIGNED_OUT.to_string() } else if said.is_empty() { "Cursor could not run this message.".to_string() } else { format!("Cursor could not run this message: {said}") }));
            out.push(Out::Finished);
            return;
        }
        out.push(Out::Usage(usage::Turn::not_reported()));
        match answer["result"]["stopReason"].as_str().unwrap_or("") {
            "end_turn" => {
                if !self.said && self.seen.is_empty() {
                    out.push(Out::Error("Cursor ended this turn without saying anything.".into()));
                }
            }
            "cancelled" => {}
            "refusal" => out.push(Out::Failed("Cursor refused this message: its usage limit was reached, or it would not answer. Try again once the limit resets.".into())),
            other => out.push(Out::Error(format!("Cursor stopped: {}.", if other.is_empty() { "no reason given" } else { other }))),
        }
        out.push(Out::Finished);
    }

    /// The answer to the turn's `session/prompt`: its end.
    pub fn finish(&mut self, answer: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        self.flush(&mut out);
        if self.agent == Agent::Cursor {
            self.cursor_finish(answer, &mut out);
            return out;
        }
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
    let m = format!("{} {}", error["message"].as_str().unwrap_or(""), error["data"]["message"].as_str().unwrap_or("")).to_lowercase();
    m.contains("authentication required") || m.contains("not authenticated") || m.contains("please sign in") || m.contains("login required") || m.contains("not logged in") || m.contains("agent login")
}

/// What an error says: Cursor puts the words in `data.message` (`message` is only "Invalid params").
fn said_in(error: &Value) -> String {
    error["data"]["message"].as_str().filter(|m| !m.trim().is_empty()).or(error["message"].as_str()).unwrap_or("").trim().to_string()
}

// ------------------------------------------------------------------ the client

/// What a kept ACP process is given for a message.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Which agent the process is.
    pub agent: Agent,
    /// The chat's workspace: where the agent works, and its tool servers are started.
    pub cwd: PathBuf,
    /// The app's tool servers, as the app keeps them: {name: {command, args, env: {..}}}.
    pub servers: Value,
    /// The model as the per-message path names it (agy's own id, effort and all), and the effort setting. Cursor: the
    /// id its sessions list (`cursor::kept_model`), and no effort.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub rules: Rules,
    /// The file the app's tool server notes each result's pictures in for this chat (`Turn::new_pictures`).
    pub pictures: Option<PathBuf>,
}

/// The tool servers as `session/new` takes them: {name, command, args, env: [{name, value}]}. Each asks for the
/// per-message path's time limit (`timeoutSeconds`), which agy takes and does **not** keep to: measured twice, a
/// call to a server handed over this way is cut off after three minutes ("timed out after 3m0s"), also with
/// `timeout_seconds`, `timeout_ms`, `timeoutMs`, `requestTimeout` and `timeout` set. This is why Gemini's kept
/// process is not the default yet (`agents::KEPT_ON_REQUEST`).
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
    /// Stop was pressed on this turn: nothing more is let through.
    fn stopped(&mut self) -> bool;
    /// A word for the agent, to go with the user's next message.
    fn note(&mut self, text: &str);
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
    agent: Agent,
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
                } else if self.agent == Agent::Agy && mode_in(&m["params"]["update"]["configOptions"]).is_some_and(|mode| mode != "default") && method == "session/set_config_option" {
                    return Err(CallError::Closed);
                }
                continue;
            }
            if m["id"] == json!(id) {
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

/// How long after a failed `session/resume` it is tried once more.
const RESUME_AGAIN: std::time::Duration = std::time::Duration::from_millis(if cfg!(test) { 150 } else { 1000 });

/// agy's answer to a resume of a conversation it does not have (measured): `Failed to read session <id>; retry, or
/// load it with session/load`.
fn lost_session(error: &Value) -> bool {
    error["message"].as_str().is_some_and(|m| m.contains("Failed to read session"))
}

const LOST_CONVERSATION: &str = "Gemini could not find this chat's earlier conversation, so it started a new one and does not remember the chat so far. Tell it again what matters.";

/// Cursor's answer to `session/load` of a session it does not have (measured): `-32602 Invalid params`, with
/// `data.message: Session "<id>" not found`. A session that was made but never given a message is not kept either.
fn cursor_lost(error: &Value) -> bool {
    let said = said_in(error).to_lowercase();
    said.contains("session") && said.contains("not found")
}

const CURSOR_LOST: &str = "Cursor could not find this chat's earlier conversation, so it started a new one and does not remember the chat so far. Tell it again what matters.";

/// One turn on a kept ACP process: on its first, the handshake and the session (`resume`: the chat's conversation, if
/// it has one); then the message, and the agent's updates and requests until the prompt is answered.
pub fn turn(live: &Arc<Live>, spec: &Spec, resume: Option<&str>, text: &str, host: &mut dyn Host) -> End {
    let rx = live.lines();
    let cursor = spec.agent == Agent::Cursor;
    let mut c = Client { live, host, agent: spec.agent };
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
                // Cursor: `session/load`, which plays the chat's history back (passed over: it comes before the
                // answer) and has no `session/resume`. A session Cursor no longer has is said, and a new one made
                Some(id) if cursor => match c.call(&rx, "session/load", place(json!({"sessionId": id}))) {
                    Err(CallError::Rpc(e)) if cursor_lost(&e) => {
                        c.host.out(Out::Error(CURSOR_LOST.into()));
                        c.call(&rx, "session/new", place(json!({})))
                    }
                    Err(CallError::Rpc(e)) if !signed_out(&e) => return gone(&mut c, false),
                    other => other,
                },
                Some(id) => {
                    let mut tried = c.call(&rx, "session/resume", place(json!({"sessionId": id})));
                    // any error but "sign in": once more after a moment (a store another process has just let go of)
                    if matches!(&tried, Err(CallError::Rpc(e)) if !signed_out(e)) {
                        std::thread::sleep(RESUME_AGAIN);
                        tried = c.call(&rx, "session/resume", place(json!({"sessionId": id})));
                    }
                    match tried {
                        // a conversation agy no longer has (measured: `-32603 Failed to read session <id>`): a new
                        // one, and the chat is told, as a process per message does (which starts one without a word)
                        Err(CallError::Rpc(e)) if lost_session(&e) => {
                            c.host.out(Out::Error(LOST_CONVERSATION.into()));
                            c.call(&rx, "session/new", place(json!({})))
                        }
                        // anything else: this process is given up and the message goes per message, the chat's
                        // conversation kept
                        Err(CallError::Rpc(e)) if !signed_out(&e) => return gone(&mut c, false),
                        other => other,
                    }
                }
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
                    Some(id) => {
                        c.host.heard(); // it speaks the protocol and has a session: it takes messages
                        id.to_string()
                    }
                    None => return gone(&mut c, false),
                },
                // nobody is signed in: said as the per-message path says it, and no second process is tried
                Err(CallError::Rpc(e)) if signed_out(&e) => {
                    c.host.heard();
                    c.host.out(Out::Failed(if cursor { crate::cursor::SIGNED_OUT } else { antigravity::SIGNED_OUT }.into()));
                    c.host.out(Out::Finished);
                    return End::default();
                }
                Err(_) => return gone(&mut c, false),
            };
            // agy: the app is the gate only in `default`: any other mode approves some steps by itself. Cursor has no
            // such mode: a session starts in `agent`, where its own allow list decides
            let mut settled = match if cursor { Ok(Value::Null) } else { c.call(&rx, "session/set_mode", json!({"sessionId": id, "modeId": "default"})) } {
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
            if !cursor && mode_in(&settled["configOptions"]).is_some_and(|m| m != "default") {
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
        c.host.out(Out::Model(if cursor { crate::cursor::kept_model_name(m) } else { antigravity::model_name(m) }));
    }
    let prompt = live.next_request();
    if !c.send(&json!({"jsonrpc": "2.0", "id": prompt, "method": "session/prompt", "params": {"sessionId": session.id, "prompt": [{"type": "text", "text": text}]}})) {
        return gone(&mut c, false);
    }
    let mut t = Turn::of(spec.agent, spec.rules.full, &spec.rules.servers, spec.pictures.as_deref());
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
        if m["method"] == "session/request_permission" && !m["id"].is_null() {
            let p = &m["params"];
            if p["sessionId"] != session.id.as_str() {
                c.refuse_request(&m);
                continue;
            }
            c.host.taken();
            c.host.began();
            // after Stop nothing more is let through, whatever it is
            if c.host.stopped() {
                t.refused = true;
                if !c.send(&json!({"jsonrpc": "2.0", "id": m["id"], "result": cancelled()})) {
                    return gone(&mut c, false);
                }
                continue;
            }
            let v = if cursor { t.cursor_request(&p["toolCall"]) } else { decide(&p["toolCall"], &spec.rules, &session.id, &paths::allowed_commands()) };
            if !v.allow {
                t.refused = true;
            }
            for o in v.outs {
                c.host.out(o);
            }
            if let Some(note) = &v.note {
                c.host.note(note);
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
        c.host.activity(t.long_open(), t.tool_open());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(full: bool) -> (Rules, PathBuf) {
        let root = paths::test_dir("acp-gate");
        for d in ["ws/sub", "ws/.git/hooks", "ws/nested/.git", "kit", "work/chats", "home/.ssh", "home/Library", "home/games", "gemini/antigravity-cli/brain/s-1", "attachments"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("kit/config.json"), "{}").unwrap();
        let r = Rules {
            full,
            agent_id: "gate-chat".into(),
            servers: vec!["rbxkit".into(), "Roblox_Studio".into()],
            writes: vec![root.join("ws")],
            reads: vec![root.join("kit"), root.join("attachments")],
            protected: vec![root.join("kit/config.json"), root.join("work/chats"), root.join("work/usage.jsonl"), root.join("ws/.agents/mcp_config.json")],
            hidden_in: vec![root.join("home")],
            agent_dir: root.join("gemini/antigravity-cli"),
        };
        (r, root)
    }

    fn call(name: &str, kind: &str, raw: Value, locations: &[&Path]) -> Value {
        json!({"toolCallId": "call_1", "name": name, "kind": kind, "title": name, "status": "pending", "rawInput": raw,
               "locations": locations.iter().map(|p| json!({"path": p})).collect::<Vec<_>>()})
    }

    /// agy's file write, as recorded.
    fn write(path: &Path) -> Value {
        call("write_file", "edit", json!({"Content": "/* not a path */ x", "Path": path}), &[path])
    }

    /// agy's command, as recorded.
    fn command(line: &str) -> Value {
        call("run_command", "execute", json!({"CommandLine": line, "WaitMsBeforeAsync": 500}), &[])
    }

    /// agy's file read, as recorded.
    fn read(path: &Path) -> Value {
        call("read_file", "read", json!({"Path": path}), &[path])
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
        // the app's own tool servers, by agy's names for their tools (both recorded)
        assert!(ok(&call("mcp_rbxkit_rbx_board", "other", json!({"action": "list"}), &[])));
        assert!(ok(&call("mcp_Roblox_Studio_get_studio_state", "other", json!({}), &[])));
        // another server's tool (one of the user's own in agy's settings), and one whose name only starts like the app's
        for other in ["mcp_other_do", "mcp_rbxkit_x_do", "mcp_rbxkit_extra_rbx_board", "mcp_Roblox_tool"] {
            let v = decide(&call(other, "other", json!({}), &[]), &r, "s-1", &[]);
            assert!(!v.allow && said(&v).contains("does not let it do in Ask me"), "{other}: {}", said(&v));
        }
        // a write inside the workspace, a new file in a folder of it; content that starts like a path is no path
        assert!(ok(&write(&root.join("ws/note.txt"))));
        assert!(ok(&write(&root.join("ws/sub/deep/new.lua"))));
        // outside it: the kit, the home folder, another chat's folder
        for p in ["kit/readme.md", "other/x.txt", "ws-other/x.txt", "home/games/x.txt"] {
            let v = decide(&write(&root.join(p)), &r, "s-1", &[]);
            assert!(!v.allow && said(&v).contains("outside this chat's folder"), "{p}: {}", said(&v));
            assert!(v.note.is_some_and(|n| n.contains("refused")), "the agent is told with the next message");
        }
        // its own scratch folder for this conversation, and no other conversation's
        assert!(ok(&write(&root.join("gemini/antigravity-cli/brain/s-1/task.md"))));
        assert!(!ok(&write(&root.join("gemini/antigravity-cli/brain/s-2/task.md"))));
        assert!(!ok(&write(&root.join("gemini/antigravity-cli/settings.json"))));
        // anything else the app does not know, and a change of the mode in which every step asks
        assert!(!ok(&call("browser_subagent", "other", json!({"Task": "open a page"}), &[])));
        assert!(!ok(&call("generate_image", "other", json!({"Prompt": "a cat"}), &[])));
        assert!(!ok(&call("command_status", "other", json!({"CommandId": "c1"}), &[])), "a name no recording has");
    }

    #[test]
    fn a_read_agy_asks_about_passes_only_in_the_chats_folders_and_never_for_the_apps_own_files() {
        // measured: agy asks before reading a file outside its workspace (`acp-read-outside.wire`), not inside it
        let (mut r, root) = rules(false);
        let ok = |r: &Rules, p: &str| decide(&read(&root.join(p)), r, "s-1", &[]).allow;
        for p in ["ws/a.lua", "kit/guide.md", "attachments/a.png", "gemini/antigravity-cli/brain/s-1/plan.md"] {
            assert!(ok(&r, p), "{p}");
        }
        for p in ["elsewhere/secret.txt", "home/.ssh/id_rsa", "work/chats/c2.json", "gemini/antigravity-cli/brain/s-2/plan.md"] {
            let v = decide(&read(&root.join(p)), &r, "s-1", &[]);
            assert!(!v.allow && said(&v).contains("outside this chat's folders"), "{p}: {}", said(&v));
        }
        // the app's settings in a folder the chat may read (running from source the kit holds them), another chat's
        // saved state were the work folder readable, in any case of letters
        assert!(!ok(&r, "kit/config.json") && !ok(&r, "kit/CONFIG.JSON"));
        r.reads.push(root.join("work"));
        assert!(!ok(&r, "work/chats/c2.json") && !ok(&r, "work/CHATS/c2.json") && !ok(&r, "work/usage.jsonl"));
        assert!(ok(&r, "work/notes.md"));
        assert!(!decide(&call("read_file", "read", json!({}), &[]), &r, "s-1", &[]).allow, "a read that does not say where");
        // a search of the web names no file and is not the app's to stop (measured: agy does not ask); were it to
        // ask as a fetch, it passes where reads are kept to the chat's folders
        assert_eq!(decide(&call("some_fetch", "fetch", json!({"Url": "https://example.com/a"}), &[]), &r, "s-1", &[]).allow, READS_ASKED);
    }

    #[test]
    fn nothing_is_written_under_a_git_folder_in_ask_me() {
        let (r, root) = rules(false);
        // a hook, an alias or a pager written there would run when an allowed `git` runs
        for p in ["ws/.git/config", "ws/.git/hooks/pre-commit", "ws/nested/.git/config", "ws/.GIT/config", "ws/sub/.Git/hooks/post-checkout", "ws/.git"] {
            let v = decide(&write(&root.join(p)), &r, "s-1", &["git".to_string()]);
            assert!(!v.allow && said(&v).contains(".git folder"), "{p}: {}", said(&v));
        }
        // by any tool that changes files: an edit, a delete, a move into it, a diff that names it
        assert!(!decide(&call("some_editor", "edit", json!({"TargetFile": root.join("ws/.git/config")}), &[]), &r, "s-1", &[]).allow);
        assert!(!decide(&call("remove", "delete", json!({}), &[&root.join("ws/.git/hooks")]), &r, "s-1", &[]).allow);
        assert!(!decide(&call("rename", "move", json!({"Path": root.join("ws/a.txt"), "NewPath": root.join("ws/.git/hooks/pre-push")}), &[]), &r, "s-1", &[]).allow);
        let mut by_diff = call("some_editor", "edit", json!({}), &[&root.join("ws/a.txt")]);
        by_diff["content"] = json!([{"type": "diff", "path": root.join("ws/.git/config"), "newText": "[core]\n\tfsmonitor = x"}]);
        assert!(!decide(&by_diff, &r, "s-1", &[]).allow);
        // a file that only has git in its name is not a git folder
        assert!(decide(&write(&root.join("ws/.gitignore")), &r, "s-1", &[]).allow);
        assert!(decide(&write(&root.join("ws/git/notes.md")), &r, "s-1", &[]).allow);
        // Full auto: as OpenCode's Full auto, and any command runs there anyway
        let (full, _) = rules(true);
        assert!(decide(&write(&root.join("ws/.git/config")), &full, "s-1", &[]).allow);
    }

    #[test]
    fn a_path_that_cannot_be_judged_and_a_request_that_cannot_be_read_are_refused() {
        for full in [false, true] {
            let (r, root) = rules(full);
            let no = |t: &Value| !decide(t, &r, "s-1", &["git".to_string()]).allow;
            assert!(no(&Value::Null), "no tool call at all");
            assert!(no(&json!({"name": "write_file", "kind": "edit"})), "no id");
            assert!(no(&json!({"toolCallId": "c", "name": "write_file", "kind": "edit", "rawInput": "text"})), "arguments that are no object");
            assert!(no(&call("write_file", "edit", json!({"Content": "x"}), &[])), "a write that names no file");
            assert!(no(&call("write_file", "edit", json!({"Path": 7}), &[])));
            assert!(no(&json!({"toolCallId": "c", "name": "write_file", "kind": "edit", "rawInput": {}, "locations": [{"line": 3}]})));
            // `..` that ends in one of the app's files, and a tool the app does not know that names one under an
            // argument of any name
            assert!(no(&write(&root.join("ws/../kit/config.json"))));
            assert!(no(&call("move_thing", "other", json!({"Destination": root.join("kit/config.json")}), &[])));
            assert!(no(&call("set_mode", "switch_mode", json!({"mode": "auto"}), &[])), "the mode in which every step asks is never left");
        }
        // Ask me: a relative path and any `..` cannot be judged (the folder agy works in is its own to say)
        let (ask, root) = rules(false);
        assert!(!decide(&call("write_file", "edit", json!({"Path": "note.txt"}), &[]), &ask, "s-1", &[]).allow);
        assert!(!decide(&write(&root.join("ws/sub/../note.txt")), &ask, "s-1", &[]).allow);
        // Full auto: taken from the workspace and worked out, so a turn does not end on it; and an argument that only
        // sounds like a path (`profile`) is not one
        let (full, _) = rules(true);
        assert!(decide(&call("write_file", "edit", json!({"Path": "note.txt"}), &[]), &full, "s-1", &[]).allow);
        assert!(decide(&write(&root.join("ws/sub/../note.txt")), &full, "s-1", &[]).allow);
        assert!(!decide(&call("write_file", "edit", json!({"Path": "../kit/config.json"}), &[]), &full, "s-1", &[]).allow);
        assert!(decide(&call("tune", "other", json!({"profile": "fast", "Profile": "x/y"}), &[]), &full, "s-1", &[]).allow);
    }

    #[test]
    fn what_no_file_tool_may_write_is_refused_in_any_case_of_letters_in_full_auto_too() {
        for full in [false, true] {
            let (mut r, root) = rules(full);
            // even were these folders the chat may write in
            for d in ["kit", "work", "home"] {
                r.writes.push(root.join(d));
            }
            for p in ["kit/config.json", "kit/CONFIG.JSON", "kit/Config.Json", "work/chats/c1.json", "work/CHATS/c1.json", "work/usage.jsonl", "work/USAGE.JSONL", "ws/.agents/mcp_config.json",
                      "home/.zshenv", "home/.ssh/authorized_keys", "home/.config/x/y.toml", "home/Library/LaunchAgents/a.plist", "home/LIBRARY/x"] {
                let v = decide(&write(&root.join(p)), &r, "s-1", &[]);
                assert!(!v.allow && said(&v).contains("no AI's file tools may change"), "{full} {p}: {}", said(&v));
            }
            // by any tool that changes files, and when only a `diff` names the file
            assert!(!decide(&call("some_editor", "edit", json!({"TargetFile": root.join("kit/CONFIG.json")}), &[]), &r, "s-1", &[]).allow);
            let mut by_diff = call("some_editor", "edit", json!({}), &[&root.join("ws/a.txt")]);
            by_diff["content"] = json!([{"type": "diff", "path": root.join("work/chats/c2.json"), "newText": "x"}]);
            assert!(!decide(&by_diff, &r, "s-1", &[]).allow);
            // a place that holds one of them: a delete or a move takes what is inside with it
            for p in ["work", "kit", "ws/.agents", "home"] {
                assert!(!decide(&call("remove", "delete", json!({}), &[&root.join(p)]), &r, "s-1", &[]).allow, "{full} {p}");
            }
            assert!(!decide(&call("rename", "move", json!({"Path": root.join("ws/x"), "NewPath": root.join("ws/.agents")}), &[]), &r, "s-1", &[]).allow);
            // a file beside them is not theirs; a folder in the home folder that is not hidden is not either
            assert!(decide(&write(&root.join("kit/notes.md")), &r, "s-1", &[]).allow);
            assert!(decide(&write(&root.join("home/games/a.lua")), &r, "s-1", &[]).allow);
            // its own scratch folder for the conversation is its own, wherever its data folder lies
            r.agent_dir = root.join("home/.gemini/antigravity-cli");
            assert!(decide(&write(&root.join("home/.gemini/antigravity-cli/brain/s-1/task.md")), &r, "s-1", &[]).allow);
            assert!(!decide(&write(&root.join("home/.gemini/antigravity-cli/settings.json")), &r, "s-1", &[]).allow);
        }
    }

    #[test]
    fn the_two_gates_are_made_from_one_list() {
        // what OpenCode's rules protect (`opencode::protected`) and what this gate is given (`agents::send`) both come
        // from `paths::never_written`: the app's own files, the home folder's hidden names and Library
        let home = Path::new("/Users/someone");
        let list = paths::never_written(home, &[PathBuf::from("/store")]);
        assert!(list.contains(&paths::NoWrite::Place(PathBuf::from("/store"))));
        assert!(list.contains(&paths::NoWrite::HiddenIn(home.to_path_buf())) && list.contains(&paths::NoWrite::Place(home.join("Library"))));
        for (p, _) in paths::app_protected() {
            assert!(list.contains(&paths::NoWrite::Place(p.clone())), "{}", p.display());
        }
        assert!(paths::app_protected().iter().any(|(p, dir)| *dir && p.ends_with("tool-pictures")), "the pictures notes are the app's own");
        assert_eq!(paths::ASK_ME_NEVER_UNDER, [".git"]);
        assert!(!paths::never_written(Path::new(""), &[]).iter().any(|p| matches!(p, paths::NoWrite::HiddenIn(_))), "no home: no home rules");
    }

    #[test]
    fn full_auto_lets_everything_else_through() {
        let (r, root) = rules(true);
        let ok = |t: &Value| decide(t, &r, "s-1", &[]).allow;
        assert!(ok(&write(&root.join("elsewhere/x.txt"))), "outside the workspace");
        assert!(ok(&command("rm -rf build && ./tool $(date)")), "any command, as before");
        assert!(ok(&call("browser_subagent", "other", json!({"Task": "open a page"}), &[])));
        assert!(ok(&read(&root.join("kit/config.json"))), "reading is not writing");
        assert!(ok(&call("mcp_rbxkit_rbx_project", "other", json!({}), &[])));
    }

    #[test]
    fn ask_me_runs_no_command_by_itself_and_shows_a_card_for_each() {
        // measured: agy runs a command through zsh, which reads the user's `.zshenv` first, whatever SHELL says; the
        // app's check is written for `/bin/sh`, so it approves nothing here by itself
        assert!(!COMMANDS_BY_APP);
        let (r, _) = rules(false);
        let git = vec!["git".to_string()];
        // an allowed program, written plainly: asked about, with the offer to run that command once
        let v = decide(&command("git status"), &r, "s-1", &git);
        assert!(!v.allow && matches!(&v.outs[..], [Out::Refused(c)] if c[0]["command"] == "git status" && c[0]["once"] == true && c[0]["reason"].as_str().unwrap().contains("every command is asked about")), "{}", said(&v));
        assert!(v.note.is_some_and(|n| n.contains("git status")));
        // a program the user has not allowed: the card that offers to allow it
        let v = decide(&command("touch denied.txt"), &r, "s-1", &git);
        assert!(!v.allow && matches!(&v.outs[..], [Out::Blocked(c)] if c[0]["name"] == "touch" && c[0]["command"] == "touch denied.txt"), "{}", said(&v));
        // an allowed program in a form the strict check refuses: the card that says why
        for bad in ["./git status", "git status > out.txt", "git log $(whoami)", "FOO=1 git status", "git status; rm -rf x", "git -c core.pager=less log"] {
            let v = decide(&command(bad), &r, "s-1", &git);
            assert!(!v.allow && v.outs.iter().any(|o| matches!(o, Out::Refused(_) | Out::Blocked(_))), "{bad}: {}", said(&v));
        }
        // a program that reads other programs' settings never runs, and is not offered
        let v = decide(&command("ps eww"), &r, "s-1", &["ps".to_string()]);
        assert!(!v.allow && matches!(&v.outs[..], [Out::Refused(c)] if c[0]["once"] == false), "{}", said(&v));
        // a command that cannot be read
        assert!(!decide(&call("run_command", "execute", json!({}), &[]), &r, "s-1", &git).allow);
        assert!(!decide(&call("run_command", "execute", json!({"CommandLine": "  "}), &[]), &r, "s-1", &git).allow);
        // "Run it once": the exact command a card of this chat showed, one time, also for an allowed program
        for cmd in ["./git status", "git log"] {
            assert!(!decide(&command(cmd), &r, "s-1", &git).allow);
            assert!(opencode::approve_once("gate-chat", cmd), "{cmd}");
            assert!(decide(&command(cmd), &r, "s-1", &git).allow, "{cmd}");
            assert!(!decide(&command(cmd), &r, "s-1", &git).allow, "{cmd}: used up");
        }
        assert!(!opencode::approve_once("gate-chat", "rm -rf x"), "never shown");
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
        assert_eq!(answer(&json!([{"optionId": "a", "kind": "allow_always"}]), true), cancelled());
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
    fn what_trails_a_turn_is_not_taken_for_work_begun_alone_but_an_answer_a_tool_call_or_a_request_is() {
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
        let outs = |answer: Value| Turn::new(&[], None).finish(&answer);
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

    fn update(id: &str, name: &str, status: &str) -> Value {
        json!({"sessionUpdate": if status == "start" { "tool_call" } else { "tool_call_update" }, "toolCallId": id, "name": name, "status": if status == "start" { "in_progress" } else { status }, "rawInput": {}})
    }

    #[test]
    fn a_long_wait_stays_marked_until_its_own_call_ends() {
        let servers = vec!["rbxkit".to_string()];
        let mut t = Turn::new(&servers, None);
        t.feed(&update("a", "mcp_rbxkit_rbx_team", "start"));
        assert!(t.long_open() && t.tool_open());
        // the lines that follow the first (agy sends `pending`, then `in_progress` again), and another call's
        t.feed(&update("a", "mcp_rbxkit_rbx_team", "pending"));
        t.feed(&update("a", "mcp_rbxkit_rbx_team", "in_progress"));
        t.feed(&update("b", "mcp_rbxkit_rbx_board", "start"));
        t.feed(&update("b", "mcp_rbxkit_rbx_board", "completed"));
        assert!(t.long_open(), "still waiting on purpose");
        t.feed(&update("a", "mcp_rbxkit_rbx_team", "completed"));
        assert!(!t.long_open() && !t.tool_open());
    }

    #[test]
    fn pictures_come_from_the_tool_servers_notes_when_its_call_completes() {
        let dir = paths::test_dir("acp-pictures");
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("render.png"), dir.join("sheet_check.png"), dir.join("third.jpg"));
        for p in [&a, &b, &c] {
            std::fs::write(p, b"png").unwrap();
        }
        let notes = dir.join("c1.jsonl");
        let servers = vec!["rbxkit".to_string(), "Roblox_Studio".to_string()];
        let pictures = |outs: Vec<Out>| outs.into_iter().find_map(|o| if let Out::Pictures(p) = o { Some(p) } else { None });
        let note = |images: Value| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&notes).unwrap();
            writeln!(f, "{}", json!({"images": images})).unwrap();
        };
        let mut t = Turn::new(&servers, Some(&notes));
        // no file yet, a call with no pictures: nothing, and nothing breaks
        t.feed(&update("a", "mcp_rbxkit_rbx_board", "start"));
        assert!(pictures(t.feed(&update("a", "mcp_rbxkit_rbx_board", "completed"))).is_none());
        // a result with pictures, as `rbxkit_mcp.py` notes it: shown once, the one for looking at first, two at most
        t.feed(&update("b", "mcp_rbxkit_rbx_blender", "start"));
        note(json!([a, b, c, dir.join("missing.png"), "relative.png", "/etc/hosts"]));
        assert!(pictures(t.feed(&update("b", "mcp_rbxkit_rbx_blender", "pending"))).is_none(), "only when the call has finished");
        let shown = pictures(t.feed(&update("b", "mcp_rbxkit_rbx_blender", "completed"))).unwrap();
        assert_eq!(shown, [b.display().to_string(), a.display().to_string()]);
        // the next call: only its own
        t.feed(&update("c", "mcp_rbxkit_rbx_texture", "start"));
        note(json!([c]));
        assert_eq!(pictures(t.feed(&update("c", "mcp_rbxkit_rbx_texture", "completed"))).unwrap(), [c.display().to_string()]);
        // a failed call, another server's tool, the agent's own tool: none
        note(json!([a]));
        t.feed(&update("d", "mcp_rbxkit_rbx_icon", "start"));
        assert!(pictures(t.feed(&update("d", "mcp_rbxkit_rbx_icon", "failed"))).is_none());
        t.feed(&update("e", "mcp_Roblox_Studio_screen_capture", "start"));
        assert!(pictures(t.feed(&update("e", "mcp_Roblox_Studio_screen_capture", "completed"))).is_none());
        // a file that does not read as notes shows nothing and breaks nothing
        std::fs::write(&notes, "not json\n{\"images\": 7}\n[]\n").unwrap();
        let mut t = Turn::new(&servers, Some(&notes));
        t.feed(&update("f", "mcp_rbxkit_rbx_blender", "start"));
        assert!(pictures(t.feed(&update("f", "mcp_rbxkit_rbx_blender", "completed"))).is_none());
    }

    // ------------------------------------------------------------ Cursor (real lines of cursor-agent 2026.11.22)

    /// A tool server's call as Cursor sends it: named only by the second line. The third is its end, with nothing of
    /// the tool's result (here a picture: the model saw it, the client is told `success`).
    const CURSOR_MCP: [&str; 4] = [
        r#"{"sessionUpdate": "tool_call", "toolCallId": "call-63602c4f-2\nfc_6418c086_0", "title": "MCP: tool", "kind": "other", "status": "pending", "rawInput": {}}"#,
        r#"{"sessionUpdate": "tool_call_update", "toolCallId": "call-63602c4f-2\nfc_6418c086_0", "title": "rbxkit: rbx_texture", "rawInput": {"providerIdentifier": "rbxkit", "toolName": "rbx_texture", "args": {"folder": "exp2"}}}"#,
        r#"{"sessionUpdate": "tool_call_update", "toolCallId": "call-63602c4f-2\nfc_6418c086_0", "status": "in_progress"}"#,
        r#"{"sessionUpdate": "tool_call_update", "toolCallId": "call-63602c4f-2\nfc_6418c086_0", "status": "completed", "rawOutput": {"success": true}}"#,
    ];
    const CURSOR_COMMAND: &str = r#"{"sessionUpdate": "tool_call", "toolCallId": "call-b2f33267-1\nfc_93834950_0", "title": "`touch /fridae-probe-am.txt`", "kind": "execute", "status": "pending", "rawInput": {"command": "touch /fridae-probe-am.txt"}}"#;
    /// The request for that command: no `rawInput`, the command in the title, why in `content`.
    const CURSOR_ASKS_COMMAND: &str = r#"{"toolCallId": "call-b2f33267-1\nfc_93834950_0", "title": "`touch /fridae-probe-am.txt`", "kind": "execute", "status": "pending", "content": [{"type": "content", "content": {"type": "text", "text": "Not in allowlist: touch"}}]}"#;
    const CURSOR_EDIT: [&str; 2] = [
        r#"{"sessionUpdate": "tool_call", "toolCallId": "call-fb152736-3\nfc_ec35938f_0", "title": "Edit File", "kind": "edit", "status": "pending", "rawInput": {}}"#,
        r#"{"sessionUpdate": "tool_call_update", "toolCallId": "call-fb152736-3\nfc_ec35938f_0", "title": "Edit `/fridae-probe-am2/x.txt`", "rawInput": {"path": "/fridae-probe-am2/x.txt"}, "locations": [{"path": "/fridae-probe-am2/x.txt"}]}"#,
    ];
    /// The request for a write outside the workspace folders.
    const CURSOR_ASKS_WRITE: &str = r#"{"toolCallId": "call-fb152736-3\nfc_ec35938f_0", "title": "Write /fridae-probe-am2/x.txt", "kind": "edit", "status": "pending", "content": [{"type": "diff", "path": "/fridae-probe-am2/x.txt", "oldText": null, "newText": "x"}]}"#;
    const CURSOR_OPTIONS: &str = r#"[{"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"}, {"optionId": "allow-always", "name": "Allow always", "kind": "allow_always"}, {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"}]"#;

    fn v(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn cursors_tool_calls_are_named_once_they_can_be_and_its_tools_pictures_come_from_the_notes() {
        let dir = paths::test_dir("acp-cursor-pictures");
        std::fs::create_dir_all(&dir).unwrap();
        let (png, notes) = (dir.join("_check.png"), dir.join("c1.jsonl"));
        std::fs::write(&png, b"png").unwrap();
        let servers = vec!["rbxkit".to_string(), "Roblox_Studio".to_string()];
        let mut t = Turn::of(Agent::Cursor, false, &servers, Some(&notes));
        assert!(t.feed(&v(CURSOR_MCP[0])).is_empty(), "\"MCP: tool\" says nothing yet");
        assert!(t.tool_open() && !t.long_open());
        let named = t.feed(&v(CURSOR_MCP[1]));
        assert!(matches!(&named[..], [Out::ToolArgs(n, a)] if n == "mcp__rbxkit__rbx_texture" && a["folder"] == "exp2"), "as a process per message names it, with what it was asked");
        assert!(t.long_open(), "a texture may wait for the user: the stall guard does not count the quiet");
        assert!(t.feed(&v(CURSOR_MCP[2])).is_empty(), "shown once");
        std::fs::write(&notes, format!("{}\n", json!({"images": [png]}))).unwrap();
        let done = t.feed(&v(CURSOR_MCP[3]));
        assert!(matches!(&done[..], [Out::Pictures(p)] if p == &[png.display().to_string()]), "the result is not in the stream: the tool server's notes are");
        assert!(!t.tool_open() && !t.long_open());
        // a command and a file edit, by Claude Code's names for them
        assert!(matches!(&t.feed(&v(CURSOR_COMMAND))[..], [Out::Tool(n)] if n == "run_command"));
        assert!(matches!(&t.feed(&v(CURSOR_EDIT[0]))[..], [Out::Tool(n)] if n == "Edit"));
        assert!(t.feed(&v(CURSOR_EDIT[1])).is_empty());
        // another server's tool is shown under its own name and its pictures are not ours to show
        let other = CURSOR_MCP[1].replace("rbxkit", "notion").replace("rbx_texture", "draw");
        let mut t = Turn::of(Agent::Cursor, false, &servers, Some(&notes));
        t.feed(&v(CURSOR_MCP[0]));
        assert!(matches!(&t.feed(&v(&other))[..], [Out::ToolArgs(n, _)] if n == "mcp__notion__draw"));
        std::fs::write(&notes, format!("{}\n", json!({"images": [png]}))).unwrap();
        assert!(t.feed(&v(CURSOR_MCP[3])).is_empty());
        // a change of mode is nothing to Cursor's turn (agy's is ended for it)
        t.feed(&json!({"sessionUpdate": "current_mode_update", "currentModeId": "ask"}));
        assert!(!t.mode_lost);
    }

    #[test]
    fn cursors_requests_are_all_refused_and_a_program_gets_its_card_at_the_turns_end() {
        let mut t = Turn::of(Agent::Cursor, false, &["rbxkit".to_string()], None);
        t.feed(&v(CURSOR_COMMAND));
        let verdict = t.cursor_request(&v(CURSOR_ASKS_COMMAND));
        assert!(!verdict.allow && verdict.outs.is_empty() && verdict.note.is_none(), "refused; the model is told by Cursor and carries on");
        assert_eq!(answer(&v(CURSOR_OPTIONS), false), json!({"outcome": {"outcome": "selected", "optionId": "reject-once"}}));
        assert_eq!(answer(&v(CURSOR_OPTIONS), true)["outcome"]["optionId"], "allow-once", "never \"always\", were the app ever to say yes");
        // the same program again, and a write outside the workspace: one card entry, none for the write
        t.cursor_request(&v(CURSOR_ASKS_COMMAND));
        t.feed(&v(CURSOR_EDIT[0]));
        t.feed(&v(CURSOR_EDIT[1]));
        assert!(!t.cursor_request(&v(CURSOR_ASKS_WRITE)).allow);
        // a request the app cannot read is refused like any other
        assert!(!t.cursor_request(&json!({})).allow && !t.cursor_request(&json!("x")).allow);
        let end = t.finish(&json!({"jsonrpc": "2.0", "id": 4, "result": {"stopReason": "end_turn"}}));
        let Some(Out::Blocked(card)) = end.iter().find(|o| matches!(o, Out::Blocked(_))) else { panic!("the card") };
        assert_eq!(card, &[json!({"name": "touch", "command": "touch /fridae-probe-am.txt"})]);
        // the prompt's answer is `{stopReason}` alone: the turn is noted as one whose usage was not reported
        assert!(end.iter().any(|o| matches!(o, Out::Usage(u) if u.unreported && u.total() == 0)));
        assert!(matches!(end.last(), Some(Out::Finished)));
        assert!(!end.iter().any(|o| matches!(o, Out::Error(_) | Out::Failed(_))));
        // without the earlier `tool_call`, the command is read from the request's title
        let mut t = Turn::of(Agent::Cursor, false, &[], None);
        t.cursor_request(&v(CURSOR_ASKS_COMMAND));
        assert!(matches!(&t.finish(&json!({"result": {"stopReason": "cancelled"}}))[..], [Out::Blocked(c), Out::Usage(_), Out::Finished] if c[0]["command"] == "touch /fridae-probe-am.txt"));
    }

    #[test]
    fn cursors_turn_endings_and_errors_are_said_in_its_name() {
        let end = |answer: Value| Turn::of(Agent::Cursor, false, &[], None).finish(&answer);
        assert!(matches!(&end(json!({"result": {"stopReason": "end_turn"}}))[..], [Out::Usage(_), Out::Error(e), Out::Finished] if e == "Cursor ended this turn without saying anything."));
        assert!(matches!(&end(json!({"result": {"stopReason": "max_turn_requests"}}))[..], [Out::Usage(_), Out::Error(e), Out::Finished] if e == "Cursor stopped: max_turn_requests."));
        // an error's words are in `data.message` (measured: `message` is only "Invalid params")
        let failed = end(json!({"error": {"code": -32602, "message": "Invalid params", "data": {"message": "Invalid model value: auto"}}}));
        assert!(matches!(&failed[..], [Out::Failed(e), Out::Finished] if e == "Cursor could not run this message: Invalid model value: auto"));
        let out = end(json!({"error": {"code": -32000, "message": "Authentication required. Please run 'agent login' first."}}));
        assert!(matches!(&out[..], [Out::Failed(e), Out::Finished] if e == crate::cursor::SIGNED_OUT));
        // the answer to a load of a session Cursor does not have (recorded)
        let lost = v(r#"{"code": -32602, "message": "Invalid params", "data": {"message": "Session \"11111111-2222-3333-4444-555555555555\" not found"}}"#);
        assert!(cursor_lost(&lost) && !signed_out(&lost));
        assert!(!cursor_lost(&json!({"code": -32602, "message": "Invalid params", "data": {"message": "Invalid model value: auto"}})));
        // a line of the history played back by a load, with no turn on the process, would be work nobody asked for
        assert!(stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"replay-0-3","title":"Find","kind":"search","status":"pending"}}}"#));
        assert!(!stray(br#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"session_info_update","title":"Permissions Test Steps"}}}"#));
    }
}
