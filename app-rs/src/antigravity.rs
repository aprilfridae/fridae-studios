//! Gemini as a chat's AI, through Google's Antigravity command-line tool `agy` run without its own interface.
//!
//! This file is a process per message, which is how a Gemini chat runs by default, and what it shares with the other
//! way, one `agy acp` process kept open for the chat (`acp.rs`, `agents::GeminiLaunch`; built, on request only, see
//! DEVELOPING.md): the model list, the standing instructions, the `Out` values a turn is read into.
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
//!   and `accept-edits` lets it write files. A shell command is refused by a deny rule of the app's (`command(*)`),
//!   which tells the model no and lets the turn carry on. Anything else that needs approval (the browser) is
//!   refused because nobody is there to approve it, and that refusal ends its turn with a note on stderr, which
//!   `explain` turns into a message for the user. Full auto refuses nothing but writes to the app's own files.
//! - Reading files: `read_file(*)` is allowed, as Claude Code's Read is, but never the places that hold a sign-in or a
//!   key (`paths::app_secrets`: a `read_file(<path>)` deny rule each). A deny rule beats an allow (measured, agy with
//!   `gemini-3.8-flash-low`, 2026-11-25: under these two rules a file outside the workspace was read and one in a denied
//!   folder was refused with "permission check failed", and the turn carried on). Without the allow, Ask me ended the
//!   turn at the first read outside the workspace ("auto-denied").
//! - The swarm board's hook (`swarm::live`): `<workspace>/.agents/hooks.json` holds a `PreInvocation` hook of the app's
//!   (`write_hooks`), which shows a Lead or member what is new for it before each model call. agy runs whatever that
//!   file names, so it is the app's to write, as the list of tool servers is (`own_list`).
//! - Its own `generate_image` tool needs no approval. The picture lands in agy's folder for the conversation and
//!   is shown in the chat from there (`Generated`); `rbx_image keep` copies it into the game's project.
//! - Tools on a server are all called through one built-in tool, `call_mcp_tool`, which names the real one in
//!   its parameters.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use serde_json::{Map, Value, json};

use crate::paths;

static MODELS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Ask Antigravity which models it offers (`agy models` prints one "id<tab>name" per line) and keep the list. It
/// goes to the network, so only from a background thread (`signin::check`, at start and when the sign-in is asked
/// about: with nobody signed in it prints no list but "Please sign in", which is how the app knows). Hands back what
/// it printed (output, error output, ended well); None when agy is not installed, could not be run, or did not answer
/// in time. The list is
/// kept once it has come: an empty answer (signed out, offline) leaves it to a later one.
pub fn ask_models() -> Option<(String, String, bool)> {
    ask_models_of(None)
}

/// The same question asked of an account: the user's own (None), whose answer fills the model menu, or an added one
/// by its name (in its own home, `accounts::env_of`; None when it has no folder, and then nothing is run).
pub fn ask_models_of(account: Option<&str>) -> Option<(String, String, bool)> {
    let exe = paths::find_antigravity()?;
    let mut cmd = Command::new(exe);
    cmd.arg("models");
    paths::child_env(&mut cmd, "gemini");
    if account.is_some() {
        if !crate::accounts::env_of(&mut cmd, "gemini", account) {
            return None;
        }
    } else if let Some(h) = home_override() {
        cmd.env("HOME", h);
    }
    // the home agy runs in, for its log: an added account's own (`accounts::home`), else the user's
    let home = cmd.get_envs().find(|(k, _)| *k == if cfg!(windows) { "USERPROFILE" } else { "HOME" }).and_then(|(_, v)| v.map(PathBuf::from)).unwrap_or_else(paths::home_dir);
    let began = std::time::SystemTime::now();
    // with a time limit, in a group of its own and in the ledger: one that hangs (offline) is ended
    let (stdout, mut stderr, ok) = crate::signin::run_limited(cmd, crate::signin::CHECK_WAIT)?;
    // Google's "Verify your account to continue" reaches only agy's log, never what it prints (measured, 2026-11-25)
    if log_asks_to_verify(&home, began) {
        stderr.push_str("\nVerify your account to continue.\n");
    }
    let list = model_lines(&stdout);
    if !list.is_empty() && account.is_none() {
        let _ = MODELS.set(list);
    }
    Some((stdout, stderr, ok))
}

/// Whether a log agy wrote in `home` since `began` (`<home>/.gemini/antigravity-cli/log/cli-*.log`, one per run) has
/// Google's request to verify the account (`signin::asks_to_verify`). Read only; a missing folder is no.
pub(crate) fn log_asks_to_verify(home: &Path, began: std::time::SystemTime) -> bool {
    let dir = home.join(".gemini").join("antigravity-cli").join("log");
    let Ok(entries) = fs::read_dir(&dir) else { return false };
    // a moment's slack: the file's time is when agy last wrote it, which is after the run began
    let since = began - std::time::Duration::from_secs(2);
    entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("cli-")).filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since)).any(|e| fs::read_to_string(e.path()).is_ok_and(|t| crate::signin::asks_to_verify(&t)))
}

/// The "id<tab>name" lines of `agy models`.
fn model_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(id, name)| (id.trim().to_string(), name.trim().to_string()))
        .filter(|(id, name)| !id.is_empty() && !id.contains(' ') && !name.is_empty())
        .collect()
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

/// FRIDAE_AGY_SHELL (testing only, debug builds only): the SHELL a kept agy is told of instead of `/bin/sh`, to
/// measure whether it goes by it.
#[cfg(debug_assertions)]
pub fn shell_override() -> Option<std::ffi::OsString> {
    std::env::var_os("FRIDAE_AGY_SHELL").filter(|v| !v.is_empty())
}

#[cfg(not(debug_assertions))]
pub fn shell_override() -> Option<std::ffi::OsString> {
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

/// Whether a model's id ends in its own thinking level (`gemini-4-neon-high`, `gemini-3.8-flash-low`). Such a model is
/// never given `--effort`: agy refuses the turn when the two differ.
pub fn names_its_level(id: &str) -> bool {
    id.rsplit_once('-').is_some_and(|(_, last)| matches!(last, "low" | "medium" | "high" | "xhigh" | "max"))
}

/// A model's name for showing, from its id.
pub fn model_name(id: &str) -> String {
    models().iter().find(|(known, _)| known == id).map(|(_, name)| name.clone()).unwrap_or_else(|| id.to_string())
}

/// The arguments of the command for one turn in `workspace` (run there), with our tool servers written where
/// Antigravity looks for them.
pub fn turn_args(workspace: &Path, servers: &Value, conversation: Option<&str>, model: Option<&str>, effort: Option<&str>, full: bool) -> Vec<String> {
    write_servers(workspace, servers);
    write_hooks(workspace, servers["rbxkit"]["env"]["RBXKIT_AGENT_ID"].as_str().unwrap_or(""));
    let mut allowed: Vec<String> = servers.as_object().map(|s| s.keys().map(|name| format!("mcp({name}/*)")).collect()).unwrap_or_default();
    // reading files anywhere, as Claude Code may (a turn in Ask me otherwise ends at the first file outside the
    // workspace); the places with a sign-in or a key are denied below, and a deny beats an allow (module comment)
    allowed.push(READ_ANY.into());
    let mut args: Vec<String> = ["--input-format", "stream-json", "--output-format", "stream-json", "--mode", "accept-edits", "--settings"].iter().map(|a| a.to_string()).collect();
    // the app's settings are never written by an agent (Antigravity's `write_file(<absolute path>)` rule)
    let mut deny = denied();
    // nor is the workspace's own list of tool servers: agy starts whatever that list names, and the app keeps the
    // entries it did not make (`write_servers`). The kept path protects it too (`GeminiLaunch::new`). Measured in
    // Ask me (`commands-refused-softly-and-own-server-list.ndjson`): the write was refused, the turn went on, and
    // another file in the workspace was written.
    deny.extend(own_list(workspace));
    // never a sign-in or a key
    deny.extend(paths::app_secrets().iter().map(|p| format!("read_file({})", p.display())));
    if !full {
        // A command needs approval nobody is there to give, and a step refused for that ends the turn at once
        // ("auto-denied", `explain`). Refused by a rule of ours instead, the model is told no and the turn carries
        // on (measured, agy 1.5.15: `tests/fixtures/gemini/command-refused-softly-then-image.ndjson`; a command with
        // a path, one with a pipe and one with `&&` likewise, in the recording named above). The programs the user
        // allowed are not handed to agy: it runs commands through zsh, which reads the user's `.zshenv`.
        deny.push(COMMANDS_REFUSED.into());
    }
    args.push(json!({"permissions": {"allow": allowed, "deny": deny}}).to_string());
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

/// Antigravity's rule that lets it read any file (the app's secrets are denied: `turn_args`).
const READ_ANY: &str = "read_file(*)";

/// Antigravity's rule that refuses every shell command.
const COMMANDS_REFUSED: &str = "command(*)";

/// Antigravity's deny rules for the app's own files (`paths::app_protected`): `write_file(<absolute path>)`, which
/// covers a folder's contents as its grants do.
fn denied() -> Vec<String> {
    crate::paths::app_protected().into_iter().map(|(p, _)| format!("write_file({})", p.display())).collect()
}

/// The deny rules for the workspace's list of tool servers (`.agents/mcp_config.json`): by the path as the app has
/// it, and by the path with links followed where that is another (agy names a file by its real path: a Mac's /tmp is
/// /private/tmp).
fn own_list(workspace: &Path) -> Vec<String> {
    let mut wss = vec![workspace.to_path_buf()];
    if let Some(real) = fs::canonicalize(workspace).ok().filter(|r| r != workspace) {
        wss.push(real);
    }
    // and its hooks (`write_hooks`): agy runs those too
    wss.iter().flat_map(|ws| OWN_FILES.iter().map(move |f| ws.join(".agents").join(f))).map(|p| format!("write_file({})", p.display())).collect()
}

/// The files in a workspace's `.agents` folder that name programs agy runs, and that the app keeps.
pub const OWN_FILES: [&str; 2] = ["mcp_config.json", "hooks.json"];

/// The name of the app's own entry in a workspace's `.agents/hooks.json`.
const HOOK_NAME: &str = "fridae-board";

/// Put the swarm board's hook for this chat (`swarm::live::gemini_hook`) in the workspace's `.agents/hooks.json`,
/// leaving any hooks the user put there. Nothing for a chat without an id, or where that file is a link
/// (`list_is_link`).
pub(crate) fn write_hooks(workspace: &Path, agent_id: &str) {
    let Some(hook) = crate::swarm::live::gemini_hook(agent_id) else { return };
    let dir = workspace.join(".agents");
    let path = dir.join("hooks.json");
    if is_link(&dir) || is_link(&path) {
        return;
    }
    let mut cfg = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if cfg[HOOK_NAME] == hook {
        return;
    }
    cfg[HOOK_NAME] = hook;
    let _ = fs::create_dir_all(&dir);
    replace_whole(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default().as_bytes());
}

/// Add our servers to the workspace's list, leaving any the user put there.
pub(crate) fn write_servers(workspace: &Path, servers: &Value) {
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
    if list_is_link(workspace) {
        return; // not the app's to write: `list_is_link`
    }
    let _ = fs::create_dir_all(&dir);
    replace_whole(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default().as_bytes());
}

/// Takes our servers out of the workspace's list again, leaving any the user put there. A kept process is handed
/// them over its protocol (`acp::servers_param`), and agy also starts every server of this list when a prompt comes
/// (measured), so one left here by an earlier process per message would run twice.
///
/// The file is replaced whole (a temporary file renamed over it), never written in place, and one that does not
/// read as a list is left alone. Returns the file as it was when it was changed, for `rewrite_list` to put back
/// once the turn is over.
pub fn unwrite_servers(workspace: &Path, servers: &Value) -> Option<Vec<u8>> {
    let path = workspace.join(".agents").join("mcp_config.json");
    if list_is_link(workspace) {
        return None;
    }
    let was = fs::read(&path).ok()?;
    let mut cfg = serde_json::from_slice::<Value>(&was).ok()?;
    let list = cfg["mcpServers"].as_object_mut()?;
    let before = list.len();
    list.retain(|name, _| servers.get(name).is_none());
    if list.len() == before {
        return None;
    }
    replace_whole(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default().as_bytes()).then_some(was)
}

/// Puts the workspace's list back as `unwrite_servers` found it.
pub fn rewrite_list(workspace: &Path, was: &[u8]) {
    replace_whole(&workspace.join(".agents").join("mcp_config.json"), was);
}

/// The workspace's list of tool servers as it is now (None: there is none), for `put_back`.
pub fn list_now(workspace: &Path) -> Option<Vec<u8>> {
    fs::read(workspace.join(".agents").join("mcp_config.json")).ok()
}

/// A turn on a process of its own is over: the workspace's list of tool servers is put back as it was when the turn
/// began (`list_now`) if it is another now, and removed if there was none. True when it had been changed.
///
/// The deny rule for the file (`own_list`) is agy's, and agy compares case (measured for the app's settings,
/// `full-auto-deny-rules.ndjson`): on a disk that ignores case `.agents/MCP_CONFIG.JSON` is this same file and gets
/// past the rule, in the one folder where Ask me accepts writes. agy starts what the list names at the next
/// message, so whatever a turn left there is undone before one comes. A link is left alone (`list_is_link`).
pub fn put_back(workspace: &Path, was: &Option<Vec<u8>>) -> bool {
    if list_is_link(workspace) || list_now(workspace) == *was {
        return false;
    }
    let path = workspace.join(".agents").join("mcp_config.json");
    match was {
        Some(bytes) => replace_whole(&path, bytes),
        None => fs::remove_file(&path).is_ok(),
    }
}

/// What a chat is told when its turn had changed the list.
pub const LIST_PUT_BACK: &str = "Gemini changed this folder's list of tool servers (.agents/mcp_config.json) during its turn. Fridae put the list back as it was: what is on that list is started as a program, so only you change it.";

/// Whether the workspace's list of tool servers is a symbolic link. The app makes no such link, so one is the
/// user's or came with the game's folder, and it may name any file (a shell start-up file, the app's settings): the
/// app neither writes through it nor replaces it. The list is then not written, and the chat is told
/// (`LIST_IS_A_LINK`).
pub fn list_is_link(workspace: &Path) -> bool {
    is_link(&workspace.join(".agents").join("mcp_config.json")) || is_link(&workspace.join(".agents"))
}

fn is_link(path: &Path) -> bool {
    path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink())
}

/// What a chat is told when the list could not be written for that reason.
pub const LIST_IS_A_LINK: &str = "This folder's list of tool servers for Gemini (.agents/mcp_config.json) is a link to another file. Fridae does not write through a link it did not make, so it could not put its building tools on that list, and Gemini may not have them in this chat. Replace the link with a plain file (or remove it) and send your message again.";

/// Replaces a file whole: written beside it, then renamed over it, so nobody reads half of it. Never through a
/// symbolic link, and never over one: a link here is not the app's (see `list_is_link`), so it is left as it is and
/// nothing is written. False when nothing was written.
fn replace_whole(path: &Path, bytes: &[u8]) -> bool {
    if is_link(path) || path.parent().is_some_and(is_link) {
        return false;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", paths::random_hex(1)));
    let done = fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_ok();
    if !done {
        let _ = fs::remove_file(&tmp);
    }
    done
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

/// Antigravity's own data folder as a process per message has it: `<home>/.gemini/antigravity-cli`. (Tests have a
/// folder of their own, so none reads the user's.)
pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("agy-data").join("antigravity-cli");
    #[cfg(not(test))]
    home_override().unwrap_or_else(paths::home_dir).join(".gemini").join("antigravity-cli")
}

/// Antigravity's folder for one conversation inside its data folder (`brain/<id>`): its scratch files, and where
/// its `generate_image` tool leaves each picture (`<name>_<time>.jpg`, measured). None for an id that is no plain
/// name.
pub fn conversation_dir(data: &Path, conversation: &str) -> Option<PathBuf> {
    let plain = !conversation.is_empty() && conversation.len() <= 80 && conversation.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then(|| data.join("brain").join(conversation))
}

const PICTURE_ENDINGS: [&str; 4] = [".png", ".jpg", ".jpeg", ".webp"];
/// The largest file shown as a generated picture (agy's are under a megabyte).
const PICTURE_CAP: u64 = 20 * 1024 * 1024;

/// The pictures Antigravity's own image tool makes during a turn. No event names the file (the tool runs in a helper
/// of agy's, and only the model's words say where the picture went), so the conversation's folder is looked at: an
/// image file that was not there when the turn began is one this turn made.
#[derive(Default)]
pub struct Generated {
    dir: Option<PathBuf>,
    known: HashSet<PathBuf>,
}

impl Generated {
    /// Start watching a conversation's folder: what is in it now is not this turn's. The first call counts.
    pub fn watch(&mut self, dir: PathBuf) {
        if self.dir.is_none() {
            self.known = Self::images(&dir).into_iter().collect();
            self.dir = Some(dir);
        }
    }

    fn images(dir: &Path) -> Vec<PathBuf> {
        let is_picture = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| PICTURE_ENDINGS.iter().any(|e| n.to_lowercase().ends_with(e)));
        // a plain file of a size a picture has: never a link (it may name any file) and nothing over the cap, which
        // would be read whole to be shown
        let plain = |p: &PathBuf| p.symlink_metadata().is_ok_and(|m| m.file_type().is_file() && m.len() <= PICTURE_CAP);
        let mut found: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| is_picture(p) && plain(p)).collect();
        found.sort();
        found
    }

    /// The pictures made since the last look, four at most. While the turn runs (`ended` false) a file written in
    /// the last moments waits for the next look, so none is read half written.
    pub fn fresh(&mut self, ended: bool) -> Vec<String> {
        let Some(dir) = self.dir.clone() else { return Vec::new() };
        let settled = |p: &PathBuf| ended || p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_millis() >= 1500);
        let new: Vec<PathBuf> = Self::images(&dir).into_iter().filter(|p| !self.known.contains(p) && settled(p)).take(4).collect();
        self.known.extend(new.iter().cloned());
        new.iter().map(|p| p.display().to_string()).collect()
    }
}

/// Reads one turn's events. Text arrives in pieces; a reply is shown when its step is done.
#[derive(Default)]
pub struct Turn {
    texts: HashMap<i64, String>,
    shown: HashSet<i64>,
    said_something: bool,
    made: Generated,
    /// agy's data folder when it is not the app's own home's (a turn on the second account: its home's)
    pub data: Option<PathBuf>,
}

impl Turn {
    pub fn feed(&mut self, e: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        let step = &e["step_update"];
        if let Some(id) = e["conversation_id"].as_str().or(step["conversation_id"].as_str()).or(e["result"]["conversation_id"].as_str()).filter(|s| !s.is_empty()) {
            out.push(Out::Conversation(id.to_string()));
            if let Some(dir) = conversation_dir(&self.data.clone().unwrap_or_else(data_dir), id) {
                self.made.watch(dir);
            }
        }
        // the pictures its own image tool made: looked for whenever a step is over, and when the turn is
        let ended = e["event"] == "result";
        if ended || matches!(step["state"].as_str(), Some("DONE" | "ERROR")) {
            let pictures = self.made.fresh(ended);
            if !pictures.is_empty() {
                out.push(Out::Pictures(pictures));
            }
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
                    // its image tool runs in a helper of its own (recorded: `tool_name: invoke_subagent`, the
                    // helper's `type_name: image-generator`), shown as the tool it is
                    "subagent" => {
                        let image = step["subagent_info"]["subagents"].as_array().is_some_and(|l| l.iter().any(|a| a["type_name"] == "image-generator"));
                        if image && self.shown.insert(index) {
                            out.push(Out::Tool("generate_image".into()));
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
                    match why.filter(|w| quota_reached(w)) {
                        // the plan's quota: a failed turn (not just a line in the chat), so the account is marked used
                        // up and the chat may carry on with Gemini's other account (`agents::roll_over`)
                        Some(quota) => out.push(Out::Failed(quota.to_string())),
                        None => out.push(Out::Error(why.map(str::to_string).unwrap_or_else(|| format!("Gemini stopped: {}.", status.to_lowercase())))),
                    }
                }
                out.push(Out::Finished);
            }
            _ => {}
        }
        out
    }
}

/// Whether agy's words say that the plan's quota is spent: "Individual quota reached. Please upgrade your
/// subscription to increase your limits. Resets in 2h6m48s." (as the user's chats of 22 November 2026 have it,
/// from a process per message). The one line known.
pub fn quota_reached(said: &str) -> bool {
    said.to_lowercase().contains("quota reached")
}

/// That line in a turn's error output (where a process per message that fails before it has a result says why),
/// without the "Error: " agy puts before what it prints there. None when the output holds no such line.
pub fn quota_line(error_output: &str) -> Option<String> {
    let line = error_output.lines().rev().map(str::trim).find(|l| quota_reached(l))?;
    Some(line.strip_prefix("Error:").map(str::trim).unwrap_or(line).to_string())
}

/// What the chat is told when nobody is signed in to Antigravity: with the Sign in button beside it
/// (`signin::signed_out_line`), or on Windows, where the app offers no sign-in, with the way through a terminal.
pub const SIGNED_OUT: &str = if cfg!(windows) {
    "Gemini is not signed in. Open a terminal, run `agy`, sign in with your Google account, close it, then send your message again."
} else {
    "Gemini is not signed in. Sign in, then send your message again."
};

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `agy models` does not print Google's "verify your account"; agy's own log of that run does (measured,
    /// 2026-11-25). Only a log written since the run began counts.
    #[test]
    fn googles_verify_request_is_read_from_the_runs_own_log() {
        let t = paths::test_dir("agy-verify-log");
        let _ = fs::remove_dir_all(&t);
        let log = t.join(".gemini").join("antigravity-cli").join("log");
        fs::create_dir_all(&log).unwrap();
        let old = log.join("cli-20261125_224950.log");
        fs::write(&old, "W1125 server_oauth.go:133] Account ineligible: Your current account is not eligible for Antigravity. Verify your account to continue.\n").unwrap();
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        let began = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
        assert!(!log_asks_to_verify(&t, began), "an older run's log is not this run's");
        fs::write(log.join("cli-20261125_225128.log"), "I1125 fine\nE1125 errorreport.go:282] PERMISSION_DENIED (code 403): Verify your account to continue.\n").unwrap();
        assert!(log_asks_to_verify(&t, began));
        fs::write(log.join("cli-20261125_225128.log"), "I1125 fine\nW1125 You are not logged into Antigravity.\n").unwrap();
        assert!(!log_asks_to_verify(&t, began), "agy's own start-up noise is not it");
        assert!(!log_asks_to_verify(&t.join("nowhere"), began));
    }

    #[test]
    fn a_model_that_names_its_level_is_told_apart() {
        // ids as `agy models` lists them (agy 1.5.15)
        for id in ["gemini-4-neon-high", "gemini-4-hydrogen-xhigh", "gemini-4-carbon-medium", "gemini-3.8-flash-low", "claude-opus-5-5-high", "gpt-oss-120b-medium"] {
            assert!(names_its_level(id), "{id}");
        }
        for id in ["gemini-pro-orchestrator", "gemini-pro-orchestrator-neon", "gemini-pro-orchestrator-argon", "high", ""] {
            assert!(!names_its_level(id), "{id}");
        }
    }

    fn settings(args: &[String]) -> Value {
        let at = args.iter().position(|a| a == "--settings").unwrap();
        serde_json::from_str(&args[at + 1]).unwrap()
    }

    fn texts(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn outside_full_auto_every_command_is_refused_by_a_rule_so_the_turn_carries_on() {
        let ws = paths::test_dir("agy-args");
        let servers = json!({"rbxkit": {"command": "py"}, "Roblox_Studio": {"command": "studio"}});
        let ask = turn_args(&ws, &servers, Some("c-1"), Some("gemini-3.8-flash-low"), None, false);
        let s = settings(&ask);
        let deny = texts(&s["permissions"]["deny"]);
        assert!(deny.iter().any(|d| d == "command(*)"), "{deny:?}");
        assert!(deny.iter().any(|d| d.starts_with("write_file(") && d.contains("chats")), "the app's own files stay refused: {deny:?}");
        // nor its own list of tool servers (agy starts what that names), where agy runs its image tool, or the count
        let own = format!("write_file({})", ws.join(".agents").join("mcp_config.json").display());
        assert!(deny.contains(&own), "{deny:?}");
        // nor its hooks (agy runs what they name)
        assert!(deny.contains(&format!("write_file({})", ws.join(".agents").join("hooks.json").display())), "{deny:?}");
        assert!(deny.contains(&format!("write_file({})", paths::work("antigravity").display())), "{deny:?}");
        // the app's two tool servers and reading files, and nothing else: no program is handed to agy, also not one
        // the user allowed
        let mut allow = texts(&s["permissions"]["allow"]);
        allow.sort();
        assert_eq!(allow, ["mcp(Roblox_Studio/*)", "mcp(rbxkit/*)", "read_file(*)"]);
        // but never a place with a sign-in or a key (a deny beats an allow in agy, measured)
        let secrets = paths::app_secrets();
        assert!(!secrets.is_empty());
        for p in secrets {
            assert!(deny.contains(&format!("read_file({})", p.display())), "{p:?} {deny:?}");
        }
        assert!(!ask.iter().any(|a| a == "--dangerously-skip-permissions"));
        // Full auto: commands run, so the rule is not there
        let full = turn_args(&ws, &servers, None, None, None, true);
        let deny = texts(&settings(&full)["permissions"]["deny"]);
        assert!(!deny.iter().any(|d| d.starts_with("command(")), "{deny:?}");
        assert!(deny.iter().any(|d| d.starts_with("write_file(")) && deny.contains(&own), "{deny:?}");
        assert!(full.iter().any(|a| a == "--dangerously-skip-permissions"));
        // a workspace reached through a link is named both ways (agy names a file by its real path)
        #[cfg(unix)]
        {
            let real = paths::test_dir("agy-args-real");
            fs::create_dir_all(&real).unwrap();
            let link = paths::test_dir("agy-args-link");
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let deny = texts(&settings(&turn_args(&link, &servers, None, None, None, false))["permissions"]["deny"]);
            for ws in [link.clone(), fs::canonicalize(&real).unwrap()] {
                assert!(deny.contains(&format!("write_file({})", ws.join(".agents").join("mcp_config.json").display())), "{deny:?}");
            }
        }
    }

    #[test]
    fn the_board_hook_is_kept_in_the_workspace_beside_the_users_own() {
        let ws = paths::test_dir("agy-hooks");
        let _ = fs::remove_dir_all(&ws);
        fs::create_dir_all(ws.join(".agents")).unwrap();
        fs::write(ws.join(".agents").join("hooks.json"), r#"{"mine": {"Stop": []}}"#).unwrap();
        let servers = json!({"rbxkit": {"command": "py", "env": {"RBXKIT_AGENT_ID": "c1abc"}}});
        turn_args(&ws, &servers, None, None, None, false);
        let hooks: Value = serde_json::from_str(&fs::read_to_string(ws.join(".agents").join("hooks.json")).unwrap()).unwrap();
        assert_eq!(hooks["mine"], json!({"Stop": []}), "the user's own hook stays");
        let cmd = hooks[HOOK_NAME]["PreInvocation"][0]["command"].as_str().unwrap();
        assert!(cmd.starts_with('"') && cmd.ends_with("\" --board-hook gemini c1abc"), "{cmd}");
        // a chat without an id gets none, and nothing is written for it
        let other = paths::test_dir("agy-hooks-none");
        let _ = fs::remove_dir_all(&other);
        turn_args(&other, &json!({"rbxkit": {"command": "py"}}), None, None, None, false);
        assert!(!other.join(".agents").join("hooks.json").exists());
    }

    fn picture(path: &Path, seconds_old: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])).save(path).unwrap();
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(seconds_old)).unwrap();
    }

    fn pictures(outs: &[Out]) -> Vec<String> {
        outs.iter().flat_map(|o| if let Out::Pictures(p) = o { p.clone() } else { Vec::new() }).collect()
    }

    #[test]
    fn a_picture_its_own_image_tool_made_is_shown_once_and_an_earlier_turns_never() {
        let id = "00000000-0000-4000-8000-00000000b001";
        let dir = conversation_dir(&data_dir(), id).unwrap();
        let _ = fs::remove_dir_all(&dir);
        picture(&dir.join("earlier_1.jpg"), 600);
        fs::write(dir.join("notes.md"), "x").unwrap();
        let mut t = Turn::default();
        let step = |index: i64, state: &str, kind: &str, more: Value| {
            let mut s = json!({"conversation_id": id, "step_index": index, "state": state, "step_type": kind});
            s.as_object_mut().unwrap().extend(more.as_object().cloned().unwrap_or_default());
            json!({"event": "step_update", "step_update": s})
        };
        assert!(pictures(&t.feed(&json!({"event": "init", "conversation_id": id, "init": {"model": "gemini-3.8-flash-low"}}))).is_empty());
        // the image tool, as recorded: a helper of agy's own
        let helper = json!({"tool_name": "invoke_subagent", "subagent_info": {"subagents": [{"type_name": "image-generator", "role": "Image Generator"}]}});
        let outs = t.feed(&step(3, "ACTIVE", "subagent", helper.clone()));
        assert!(matches!(&outs[..], [Out::Conversation(_), Out::Tool(n)] if n == "generate_image"), "named as the tool it is");
        let outs = t.feed(&step(3, "DONE", "subagent", helper));
        assert!(!outs.iter().any(|o| matches!(o, Out::Tool(_))) && pictures(&outs).is_empty(), "once, and no picture yet");
        // the picture arrives while the model waits; the next finished step shows it, once. A link is never
        // followed (it may name any file) and a file too large to be a picture is not read
        picture(&dir.join("red_apple_2.jpg"), 10);
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("earlier_1.jpg"), dir.join("linked_4.jpg")).unwrap();
        let huge = fs::File::create(dir.join("huge_5.png")).unwrap();
        huge.set_len(PICTURE_CAP + 1).unwrap();
        huge.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(10)).unwrap();
        assert!(pictures(&t.feed(&step(5, "ACTIVE", "tool", json!({"tool_name": "schedule"})))).is_empty(), "a step that is not over");
        assert_eq!(pictures(&t.feed(&step(5, "DONE", "tool", json!({"tool_name": "schedule"})))), [dir.join("red_apple_2.jpg").display().to_string()]);
        assert!(pictures(&t.feed(&step(6, "DONE", "agent_response", json!({})))).is_empty());
        // one written this instant waits (it may not be whole) and is shown when the turn ends, before the ending
        picture(&dir.join("pear_3.png"), 0);
        assert!(pictures(&t.feed(&step(7, "ERROR", "tool", json!({"tool_name": "run_command"})))).is_empty());
        let outs = t.feed(&json!({"event": "result", "result": {"conversation_id": id, "status": "SUCCESS", "response": "done"}}));
        assert_eq!(pictures(&outs), [dir.join("pear_3.png").display().to_string()]);
        assert!(matches!(outs.last(), Some(Out::Finished)));
        // the next turn of the same conversation: nothing of this one's is shown again
        let mut next = Turn::default();
        next.feed(&json!({"event": "init", "conversation_id": id, "init": {}}));
        assert!(pictures(&next.feed(&json!({"event": "result", "result": {"conversation_id": id, "status": "SUCCESS"}}))).is_empty());
    }

    #[test]
    fn a_conversations_folder_is_only_ever_inside_agys_own() {
        let data = Path::new("/data/antigravity-cli");
        assert_eq!(conversation_dir(data, "00bd7342-4e1f-40e5").unwrap(), data.join("brain").join("00bd7342-4e1f-40e5"));
        for bad in ["", "..", "../x", "a/b", "a\\b", "."] {
            assert!(conversation_dir(data, bad).is_none(), "{bad}");
        }
    }
}
