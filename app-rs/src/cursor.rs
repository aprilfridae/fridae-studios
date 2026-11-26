//! Cursor as a chat's AI, through the Cursor Agent command-line tool `cursor-agent` run without its own interface.
//!
//!   cursor-agent -p --output-format stream-json --trust --approve-mcps --workspace <chat folder>
//!                --add-dir <kit> [--add-dir <game>] [--model <id>] [--resume <chat>] --sandbox disabled|enabled
//!
//! The message goes in on stdin; one JSON event per line comes out, close to Claude Code's: `system`/`init` (with
//! the chat's `session_id`), `user`, `thinking`, `assistant` (one per message between tool steps), `tool_call`
//! `started`/`completed` (`{"mcpToolCall": {args, result}}`, `shellToolCall`, `editToolCall`, ...), then `result`
//! with the turn's token counts. Checked against cursor-agent 2026.11.22. A failure before the turn starts (signed
//! out, an unknown model) prints one line on stderr and no events. Not offered on Windows yet (see DEVELOPING.md).
//!
//! A chat can also keep one `cursor-agent acp` process open across its turns (on request, Ask me and the model Auto
//! only: `agents::CursorLaunch`, `acp::Agent::Cursor`; DEVELOPING.md has what was measured). The settings files, the
//! allow list and the deny rules below are the same on both paths, written by the same code (`prepare`).
//!
//! How this differs from Claude Code, and why:
//! - There is no flag for standing instructions, so they are placed at the top of a conversation's first message,
//!   as for the other AIs that are not Claude, with Cursor's own text (`text/cursor.txt`).
//! - Nothing is written where Cursor keeps its own settings, and nothing in a game's folder. Every Fridae chat on
//!   Cursor runs in a folder of the app's own (`work/cursor/chats/<chat>`, with an empty `.git` folder so Cursor takes
//!   it as the project's top and looks no higher), whose `.cursor/mcp.json` holds our two tool servers; the game is
//!   added with `--add-dir`. `CURSOR_CONFIG_DIR` and `CURSOR_DATA_DIR` point at the app's data folder (Application
//!   Support on a Mac, not Documents), so Cursor's permissions file, its saved chats and its approvals for these chats
//!   are the app's, and the user's own `~/.cursor/cli-config.json` is neither read nor changed. The sign-in is not in
//!   those folders (it is in the system keychain), so it still works.
//! - Permissions (`cli-config.json` there, rebuilt before every turn) run in allow-list mode, measured with real turns:
//!   our tool servers and the programs the user allowed (`Shell(<program>)`) run; any other shell command, any other
//!   tool server's tool, and a write outside the workspace folders (the chat's folder, the kit, the game) need
//!   approval, so they are refused (a refused command is reported like Claude Code's, and the page offers to allow the
//!   program). Cursor lets any write into temp folders through, by design. Deny rules (Cursor's own settings, the
//!   app's config, the home folder's hidden files and Library, the user's own Cursor servers, Studio servers under
//!   FRIDAE_NO_STUDIO) hold in every mode.
//! - Full auto does not use `--force`: it keeps the allow list and turns Cursor's sandbox on. Allowed programs and our
//!   tools run as before; any other command runs too, but sandboxed (writes only in the workspace folders and temp
//!   folders, no network); other tool servers and writes outside the workspace are still refused. That is stricter
//!   than Claude Code's bypass mode, never looser.

use crate::paths::Start;
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use serde_json::{Map, Value, json};

use crate::{antigravity::Out, paths};

/// The models Cursor offers, and when the list was last asked for.
static MODELS: Mutex<(Vec<(String, String)>, Option<Instant>)> = Mutex::new((Vec::new(), None));
static LOADING: AtomicBool = AtomicBool::new(false);

/// The model a Cursor chat starts on: Cursor's own choice per request.
pub const DEFAULT_MODEL: &str = "auto";

/// Ask Cursor which models this account may use (`cursor-agent models` prints "id - Name" lines), in the background
/// because it goes to the network. An empty answer (signed out, offline, not installed yet) leaves the list empty, and
/// `refresh_models` asks again later.
pub fn load_models() {
    if LOADING.swap(true, Ordering::SeqCst) {
        return;
    }
    MODELS.lock().unwrap().1 = Some(Instant::now());
    std::thread::spawn(|| {
        let list = paths::find_cursor()
            .and_then(|exe| {
                let mut cmd = Command::new(exe);
                cmd.arg("models");
                environment(&mut cmd);
                paths::quiet(&mut cmd);
                cmd.run_output().ok()
            })
            .map(|out| parse_models(&String::from_utf8_lossy(&out.stdout)))
            .unwrap_or_default();
        if !list.is_empty() {
            MODELS.lock().unwrap().0 = list;
        }
        LOADING.store(false, Ordering::SeqCst);
    });
}

/// While the list is empty, ask again at most once a minute (the page's status calls this).
pub fn refresh_models() {
    let due = {
        let m = MODELS.lock().unwrap();
        m.0.is_empty() && m.1.is_none_or(|t| t.elapsed().as_secs() >= 60)
    };
    if due {
        load_models();
    }
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
pub fn models() -> Vec<(String, String)> {
    MODELS.lock().unwrap().0.clone()
}

/// The model a Cursor chat uses unless the user picks another: Auto, unless the list came and lacks it.
pub fn default_model() -> Option<String> {
    default_of(&models())
}

/// The model a turn asks for: the one chosen when Cursor lists it, or when the list has not arrived (Cursor itself
/// then says if it does not know it); otherwise the default.
pub fn pick_model(chosen: Option<&str>) -> Option<String> {
    pick_from(&models(), chosen)
}

fn pick_from(list: &[(String, String)], chosen: Option<&str>) -> Option<String> {
    // a model id that could not be one (an agent's, say) is never put on a command line
    match chosen.filter(|m| !m.is_empty() && crate::expert_ai::model_id_ok(m)) {
        Some(m) if list.is_empty() || list.iter().any(|(id, _)| id == m) => Some(m.to_string()),
        _ => default_of(list),
    }
}

fn default_of(list: &[(String, String)]) -> Option<String> {
    if list.is_empty() || list.iter().any(|(id, _)| id == DEFAULT_MODEL) { Some(DEFAULT_MODEL.to_string()) } else { list.first().map(|(id, _)| id.clone()) }
}

/// What Cursor can do in Full auto or in Studio that Claude Code cannot there, as one plain line the page shows under
/// the message box. Nothing today: Claude Code runs both with every check off, while Cursor keeps its allow list and
/// sandbox (measured, see DEVELOPING.md), so it can do less, never more. A gap found later goes here.
pub fn gaps() -> Value {
    json!({"full": null, "studio": null})
}

/// What the Experts table in Setup says beside Cursor: how an expert on Cursor differs from one on Claude. Experts
/// run with Ask me's rules (the gap measured there).
pub const EXPERT_GAP: &str = "On Cursor, an expert's file tools can also write into temp folders, which Claude's cannot, and a program you allowed may send its output into any file. Everything else follows the same rules.";

/// Where the app keeps Cursor's settings and saved chats (`config`) and its approvals per workspace (`data`): the
/// per-user app data folder, not Documents (which iCloud or OneDrive may sync). Cursor's account details (email and
/// name, no tokens) are cached there too.
fn store() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("cursor-store");
    #[allow(unreachable_code)]
    {
        let home = paths::home_dir();
        let base = if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local").join("share"))
        };
        base.join(paths::DATA_DIR).join("cursor")
    }
}

pub fn config_dir() -> PathBuf {
    store().join("config")
}

pub fn data_dir() -> PathBuf {
    store().join("data")
}

/// The folder a chat's Cursor works in.
pub fn workspace(agent_id: &str) -> PathBuf {
    paths::work("cursor").join("chats").join(agent_id)
}

/// Whether Cursor still has a saved chat: one a process per message made (`config/chats/<workspace hash>/<chat
/// id>/`), or one a kept process made (`config/acp-sessions/<id>/store.db`; the two are separate stores, and neither
/// kind of process can open the other's: measured). Asked to resume one it does not have, a process per message
/// quietly starts an empty chat under that id, so the app checks first.
pub fn chat_saved(session: &str) -> bool {
    chat_saved_in(&config_dir(), session)
}

fn chat_saved_in(config: &Path, session: &str) -> bool {
    !session.is_empty()
        && !session.contains(['/', '\\'])
        && (fs::read_dir(config.join("chats")).into_iter().flatten().flatten().any(|d| d.path().join(session).is_dir())
            // a kept process's session that was never given a message has only its `meta.json`, and cannot be loaded
            || config.join("acp-sessions").join(session).join("store.db").is_file())
}

/// Whether a chat's Cursor conversation is one a kept process made (`cursor-agent acp`, which keeps each in
/// `<settings folder>/acp-sessions/<id>/`): only such a process can carry it on. A chat keeps the path its
/// conversation began on.
pub fn kept_chat_in(config: &Path, session: &str) -> bool {
    !session.is_empty() && !session.contains(['/', '\\', '.']) && config.join("acp-sessions").join(session).is_dir()
}

/// The model's id as a kept process's sessions list it, for a model as the per-message path names it, when the two
/// are known to be the same model. Only Auto is: `auto` there is `default[]` here (measured). The others are not
/// offered alike: a process per message lists 284 ids (`gpt-5.3-codex-low-fast`), a session 49, each one fixed
/// variant (`gpt-5.3-codex[reasoning=medium,fast=false]`) and no other spelling is taken (measured: `Invalid model
/// value`); the plain names do not match either (`composer-2.5` there is `composer-2.5[fast=true]` here, which is
/// `composer-2.5-fast` there). So a chat on another model than Auto runs a process per message.
pub fn kept_model(per_message: Option<&str>) -> Option<&'static str> {
    match per_message.filter(|m| !m.is_empty()) {
        None => Some(KEPT_AUTO),
        Some(m) if m == DEFAULT_MODEL => Some(KEPT_AUTO),
        Some(_) => None,
    }
}

const KEPT_AUTO: &str = "default[]";

/// A kept process's model, as the chat names it (as `Turn::feed` names a process per message's).
pub fn kept_model_name(id: &str) -> String {
    if id == KEPT_AUTO { "Cursor (Auto)".to_string() } else { id.to_string() }
}

/// What the chat is told when Cursor is not signed in, with the Sign in button beside it (`signin::signed_out_line`).
pub const SIGNED_OUT: &str = "Cursor is not signed in. Sign in, then send your message again.";

/// FRIDAE_CURSOR_HOME (testing only, debug builds only) gives a copy of the app with a scratch HOME the real one for
/// Cursor alone, because Cursor finds its sign-in through the home folder's keychain.
#[cfg(debug_assertions)]
fn home_override() -> Option<PathBuf> {
    env::var_os("FRIDAE_CURSOR_HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(not(debug_assertions))]
fn home_override() -> Option<PathBuf> {
    None
}

/// The home folder Cursor runs with.
fn cursor_home() -> PathBuf {
    home_override().unwrap_or_else(paths::home_dir)
}

/// Cursor's settings and data folders are the app's (see the top of this file). For a turn, and for asking Cursor who
/// is signed in and starting its sign-in (`signin`), so those find what a turn finds.
pub fn environment(cmd: &mut Command) {
    paths::child_env(cmd, "cursor");
    cmd.env("CURSOR_CONFIG_DIR", config_dir()).env("CURSOR_DATA_DIR", data_dir());
    if let Some(h) = home_override() {
        cmd.env("HOME", h);
    }
}

/// The names of the tool servers the user set up for Cursor itself (`~/.cursor/mcp.json`), read only. Read the way
/// Cursor reads it, comments and trailing commas allowed. A file that still cannot be read gives no names; their tools
/// are refused anyway, since only ours are on the allow list.
fn user_servers(home: &Path) -> Vec<String> {
    fs::read_to_string(home.join(".cursor").join("mcp.json")).map(|s| server_names(&s)).unwrap_or_default()
}

fn server_names(text: &str) -> Vec<String> {
    serde_json::from_str::<Value>(&loose_json(text))
        .ok()
        .and_then(|v| v["mcpServers"].as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// JSON with comments and trailing commas made plain JSON (outside strings).
pub(crate) fn loose_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' {
            // a comma followed only by space before a closing bracket is dropped
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// A path as a pattern: glob characters in it (a folder named "Ann [Work]") become `?`, which matches any one
/// character, so the rule still covers the path (and at most a few look-alikes).
fn as_pattern(path: &Path) -> String {
    path.display().to_string().chars().map(|c| if matches!(c, '[' | ']' | '{' | '}' | '*' | '?') { '?' } else { c }).collect()
}

/// What Cursor may do without asking. `servers` are ours; `allowed` the programs the user allowed; `theirs` the
/// servers of the user's own Cursor setup (denied, as Claude Code is given only ours); `protect` folders its file tools
/// must not write in.
pub fn permissions(servers: &Value, allowed: &[String], theirs: &[String], protect: &[String]) -> Value {
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
    // (Cursor matches server names without regard to case: measured)
    if !ours.iter().any(|n| n.to_lowercase().contains("studio")) {
        deny.push("Mcp(*studio*:*)".to_string());
    }
    // never its own settings, permissions or tool lists, in any mode
    deny.push("Write(**/.cursor/**)".to_string());
    for p in protect {
        deny.push(format!("Write({p})"));
        deny.push(format!("Write({p}/**)"));
    }
    json!({"allow": allow, "deny": deny})
}

/// What Cursor's file tools are kept out of: the app's Cursor settings, the app's own settings file (it holds the
/// programs agents may run), and the home folder's hidden folders and files (shell start-up files, other tools'
/// settings) and Library. Outside the workspace folders a write is refused anyway in every mode; these rules also
/// cover the kit and the work folder, which are workspace folders.
/// As patterns: paths with their glob characters made `?`, and `<home>/.*` written as a pattern on purpose (every
/// hidden name; before, it went through `as_pattern` and became `.?`, two-letter names only).
fn protected(home: &Path) -> Vec<String> {
    let mut list: Vec<String> = [config_dir(), data_dir()].iter().map(|p| as_pattern(p)).collect();
    list.extend(paths::app_protected().iter().map(|(p, _)| as_pattern(p)));
    if !home.as_os_str().is_empty() {
        let h = as_pattern(home);
        list.extend([format!("{h}/.*"), format!("{h}/Library")]);
    }
    list
}

/// What Cursor itself keeps in its settings file that the app leaves alone (the model last used, the account and
/// privacy caches). Everything else is rebuilt each turn, so nothing a turn wrote there lasts.
const KEPT: [&str; 11] = [
    "model", "selectedModel", "modelParameters", "modelSelectionHistory", "hasChangedDefaultModel", "exploreSubagentModel",
    "privacyCache", "authInfo", "serverConfigCache", "autoReviewAvailabilityCache", "network",
];

/// Write the app's Cursor settings file: the permissions, with only `KEPT` of what was there.
fn write_config(dir: &Path, permissions: &Value) {
    let path = dir.join("cli-config.json");
    let old = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    let mut cfg = json!({"version": 1});
    for k in KEPT {
        if let Some(v) = old.get(k) {
            cfg[k] = v.clone();
        }
    }
    cfg["permissions"] = permissions.clone();
    cfg["approvalMode"] = json!("allowlist");
    cfg["autoAcceptWebSearch"] = json!(true); // as Claude Code's WebSearch is allowed
    // off unless a turn asks for it (Full auto passes --sandbox enabled): the programs the user allowed run as they do
    // for Claude Code
    cfg["sandbox"] = json!({"mode": "disabled"});
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
    if home_override().is_some() {
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

/// Cursor's name for a project's folder under its data folder (`projects/<this>`): the project's real path with
/// every run of other characters than letters and digits made one `-` (as cursor-agent 2026.11.22 does).
fn project_slug(path: &Path) -> String {
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut out = String::new();
    for c in real.display().to_string().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// `--approve-mcps` approves every tool server Cursor finds, so the user's own servers (from `~/.cursor/mcp.json`) are
/// put on the chat project's disabled list, which Cursor reads before it starts any: they are not even started (their
/// tools are refused by the allow list anyway). Measured with cursor-agent 2026.11.22: a server named there is not
/// started, the others are. Servers from Cursor plugins are not listed (their names are not known here): they may
/// start, and their tools are refused.
fn write_disabled(workspace: &Path, users: &[String], ours: &Value) {
    let names: Vec<&String> = users.iter().filter(|n| ours.get(n.as_str()).is_none()).collect();
    let dir = data_dir().join("projects").join(project_slug(workspace));
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("mcp-disabled.json"), serde_json::to_string_pretty(&names).unwrap_or_default());
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
    // Full auto: the allow list stays, and commands it does not cover run in Cursor's sandbox instead of being refused.
    // Not `--force`, which would let anything run unconfined and other tool servers' tools through.
    a.extend(["--sandbox".to_string(), if full { "enabled" } else { "disabled" }.to_string()]);
    a
}

/// What Cursor may do without asking, as the app's settings file says it now (`permissions`): our servers, the
/// programs the user allowed, the user's own servers and the app's files denied.
pub fn rules_now(servers: &Value) -> Value {
    let home = cursor_home();
    let mut rules = permissions(servers, &paths::allowed_commands(), &user_servers(&home), &protected(&home));
    deny_reads(&mut rules, &paths::app_secrets());
    rules
}

/// The places that hold a sign-in or a key (`paths::app_secrets`) are never read either, in any mode: `Read(<path>)`
/// and `Read(<path>/**)` in Cursor's deny list (a deny rule wins over every allow, and holds with --force).
fn deny_reads(rules: &mut Value, secrets: &[PathBuf]) {
    let Some(deny) = rules["deny"].as_array_mut() else { return };
    for p in secrets {
        let p = as_pattern(p);
        deny.extend([json!(format!("Read({p})")), json!(format!("Read({p}/**)"))]);
    }
}

/// Everything a Cursor process reads at its start is written: the chat's workspace, the app's Cursor settings (the
/// allow list and deny rules, rebuilt), and the list of the user's own servers it must not start.
pub fn prepare(workspace: &Path, servers: &Value) {
    write_workspace(workspace, servers);
    let home = cursor_home();
    write_config(&config_dir(), &rules_now(servers));
    let _ = fs::create_dir_all(data_dir());
    write_disabled(workspace, &user_servers(&home), servers);
}

/// The command for one turn, with the workspace and the app's Cursor settings written first. `first`: arguments
/// before all others (none for cursor-agent; the tests' stand-in is a script given to `sh`).
#[allow(clippy::too_many_arguments)]
pub fn command_with(exe: &Path, first: &[std::ffi::OsString], workspace: &Path, servers: &Value, session: Option<&str>, model: Option<&str>, full: bool, project: Option<&str>, kit: &Path) -> Command {
    prepare(workspace, servers);
    let mut cmd = Command::new(exe);
    cmd.args(first).args(args(workspace, kit, project, session, model, full));
    environment(&mut cmd);
    cmd.current_dir(workspace);
    cmd
}

/// The arguments of a process kept open for a chat: `cursor-agent --trust --approve-mcps --add-dir <kit> [--add-dir
/// <game>] --sandbox disabled acp`, the per-message ones without the message's own (`-p`, the output format, the
/// model, `--resume`, `--workspace`: the session is given its folder over the protocol). Always `--sandbox
/// disabled`: the sandbox is not applied over ACP whatever this says (measured), which is why only Ask me, where a
/// process per message runs without it too, is kept open.
pub fn kept_args(kit: &Path, project: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = ["--trust", "--approve-mcps", "--add-dir"].map(str::to_string).to_vec();
    a.push(kit.display().to_string());
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        a.extend(["--add-dir".to_string(), p.to_string()]);
    }
    a.extend(["--sandbox", "disabled", "acp"].map(str::to_string));
    a
}

/// The command of a process kept open for a chat, in the chat's workspace, with the same environment and the same
/// settings files as a process per message (`prepare`: Cursor's allow list and deny rules decide here as there).
pub fn kept_command(exe: &Path, first: &[std::ffi::OsString], workspace: &Path, servers: &Value, project: Option<&str>, kit: &Path) -> Command {
    prepare(workspace, servers);
    let mut cmd = Command::new(exe);
    cmd.args(first).args(kept_args(kit, project));
    environment(&mut cmd);
    cmd.current_dir(workspace);
    cmd
}

/// Our tool servers as a kept process is handed them (`session/new`), with the app's settings in each one's
/// environment as the workspace's file has them: over ACP Cursor does not read that file (measured), and gives a
/// server only HOME, PATH and what its entry lists, as on a process per message.
pub fn kept_servers(servers: &Value) -> Value {
    servers_file(servers, &passed_env())["mcpServers"].clone()
}

/// The folders Cursor's settings and saved chats are in, for what a kept process's launch depends on.
pub fn folders() -> (PathBuf, PathBuf, PathBuf) {
    (config_dir(), data_dir(), cursor_home())
}

/// How Cursor runs here, in its own words (instead of the text the other AIs get): its folders, the allowed programs
/// and the blocked card, Full auto's sandbox, the web.
pub fn instructions(project: Option<&str>, allowed: &[String]) -> String {
    let game = match project.filter(|p| !p.is_empty()) {
        Some(p) => format!(" The game's folder is added to your workspace: read and write the game's files (GAME.md included) by their full paths under {p}."),
        None => String::new(),
    };
    let programs = if allowed.is_empty() { "none yet".to_string() } else { allowed.join(", ") };
    include_str!("text/cursor.txt").replace("{game}", &game).replace("{programs}", &programs)
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
                let id = e["call_id"].as_str().filter(|s| !s.is_empty()).or(body["toolCallId"].as_str()).or(e["tool_call"]["toolCallId"].as_str()).unwrap_or("").to_string();
                let args = &body["args"];
                let name = match kind {
                    // it reads each tool's description before calling it: not worth a line in the chat
                    "getMcpToolsToolCall" => None,
                    "mcpToolCall" => {
                        let server = args["serverIdentifier"].as_str().or(args["providerIdentifier"].as_str()).unwrap_or("");
                        let tool = args["toolName"].as_str().unwrap_or("a tool");
                        Some(if server.is_empty() { tool.to_string() } else { format!("mcp__{server}__{tool}") })
                    }
                    "shellToolCall" | "writeShellStdinToolCall" => Some("run_command".to_string()),
                    "editToolCall" => Some("Edit".to_string()),
                    "writeToolCall" => Some("Write".to_string()),
                    "readToolCall" => Some("Read".to_string()),
                    "deleteToolCall" => Some("Delete".to_string()),
                    "globToolCall" => Some("Glob".to_string()),
                    "grepToolCall" | "semSearchToolCall" => Some("Grep".to_string()),
                    "lsToolCall" => Some("list_dir".to_string()),
                    "webSearchToolCall" => Some("WebSearch".to_string()),
                    "webFetchToolCall" => Some("WebFetch".to_string()),
                    // its own bookkeeping (to-do lists, plans, lint checks): not steps the user needs to see
                    "updateTodosToolCall" | "readTodosToolCall" | "readLintsToolCall" | "createPlanToolCall" | "createGoalToolCall"
                    | "updateGoalToolCall" | "awaitToolCall" | "partialToolCall" => None,
                    "taskToolCall" => Some("Task".to_string()),
                    other => Some(other.trim_end_matches("ToolCall").to_string()),
                };
                // a step is announced once, whether we first hear of it starting or finishing; a server's tool comes
                // with its arguments, so a Lead's rbx_team wait or rbx_stage status is not taken for progress
                // (a step with no id at all is announced when it starts, so it is not announced twice)
                let first = if id.is_empty() { e["subtype"] != "completed" } else { self.shown.insert(id) };
                if let Some(name) = name.filter(|_| first) {
                    if kind == "mcpToolCall" {
                        out.push(Out::ToolArgs(name, args["args"].clone()));
                    } else {
                        out.push(Out::Tool(name));
                    }
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
            // an error in the middle of a turn, if Cursor sends one: shown, but the turn fails only if Cursor's result
            // says so (or no result comes), since it may recover and finish
            "error" => {
                let why = e["message"].as_str().or(e["error"]["message"].as_str()).or(e["error"].as_str()).filter(|s| !s.trim().is_empty());
                out.push(Out::Error(why.map(str::to_string).unwrap_or_else(|| "Cursor reported an error.".to_string())));
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
        return Some(SIGNED_OUT.to_string());
    }
    if low.contains("cannot use this model") {
        return Some("Cursor does not offer the chosen model to this account. Pick another model in the message box (Auto always works), then send your message again.".to_string());
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
        assert!(matches!(&start[..], [Out::ToolArgs(n, a)] if n == "mcp__rbxkit__rbx_vault" && a["action"] == "search"), "a server's tool comes with what it was asked");
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
        assert!(matches!(&out[0], Out::ToolArgs(n, _) if n == "mcp__rbxkit__rbx_texture"));
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
        assert!(!full.contains(&"--force".to_string()), "full auto keeps the allow list");
        assert_eq!(full[full.len() - 2..], ["--sandbox", "enabled"], "full auto runs other commands in Cursor's sandbox");
        assert_eq!(a[a.len() - 2..], ["--sandbox", "disabled"], "otherwise allowed programs run as they do for Claude Code");
    }

    #[test]
    fn other_steps_are_named_and_its_bookkeeping_is_hidden() {
        let step = |kind: &str, id: &str| format!(r#"{{"type":"tool_call","subtype":"started","call_id":"{id}","tool_call":{{"{kind}":{{"args":{{}}}}}}}}"#);
        let mut t = Turn::default();
        for (kind, name) in [("globToolCall", "Glob"), ("grepToolCall", "Grep"), ("lsToolCall", "list_dir"), ("deleteToolCall", "Delete"), ("webSearchToolCall", "WebSearch")] {
            assert!(matches!(&feed(&mut t, &step(kind, kind))[..], [Out::Tool(n)] if n == name), "{kind}");
        }
        for kind in ["updateTodosToolCall", "readTodosToolCall", "readLintsToolCall", "createPlanToolCall"] {
            assert!(feed(&mut t, &step(kind, kind)).is_empty(), "{kind}");
        }
        // no call_id: the id inside the step is used, so a step is still announced once
        let inner = r#"{"type":"tool_call","subtype":"started","tool_call":{"readToolCall":{"args":{}},"toolCallId":"r1"}}"#;
        assert_eq!(feed(&mut t, inner).len(), 1);
        assert!(feed(&mut t, &inner.replace("started", "completed")).is_empty());
        let err = feed(&mut t, r#"{"type":"error","message":"Connection lost"}"#);
        assert!(matches!(&err[..], [Out::Error(m)] if m == "Connection lost"), "shown, not a failed turn by itself");
        // a step with no id anywhere: announced when it starts, not again when it completes
        let bare = r#"{"type":"tool_call","subtype":"started","tool_call":{"globToolCall":{"args":{}}}}"#;
        assert_eq!(feed(&mut t, bare).len(), 1);
        assert!(feed(&mut t, &bare.replace("started", "completed")).is_empty());
    }

    #[test]
    fn the_users_own_servers_are_disabled_for_the_chat() {
        assert_eq!(project_slug(Path::new("/no/such/Fridae Studios/work/cursor/chats/c1")), "no-such-Fridae-Studios-work-cursor-chats-c1");
        let ws = paths::test_dir("cursor-disabled");
        write_disabled(&ws, &["github".into(), "rbxkit".into()], &json!({"rbxkit": {}}));
        let file = data_dir().join("projects").join(project_slug(&ws)).join("mcp-disabled.json");
        let names: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(names, json!(["github"]), "ours stay on");
    }

    #[test]
    fn a_model_the_list_lacks_falls_back_to_the_default() {
        let list = vec![("auto".to_string(), "Auto".to_string()), ("gpt-5.3".to_string(), "GPT".to_string())];
        assert_eq!(pick_from(&list, Some("gpt-5.3")).as_deref(), Some("gpt-5.3"));
        assert_eq!(pick_from(&list, Some("gone-model")).as_deref(), Some("auto"));
        assert_eq!(pick_from(&list, None).as_deref(), Some("auto"));
        assert_eq!(pick_from(&[], Some("gpt-5.3")).as_deref(), Some("gpt-5.3"), "before the list arrives, Cursor itself checks");
        assert_ne!(pick_from(&[], Some("--yolo")).as_deref(), Some("--yolo"), "never an option on the command line");
    }

    #[test]
    fn a_saved_chat_is_found_by_its_id() {
        let cfg = crate::paths::test_dir("cursor-saved");
        let _ = fs::remove_dir_all(&cfg);
        fs::create_dir_all(cfg.join("chats").join("hash1").join("abc-123")).unwrap();
        assert!(chat_saved_in(&cfg, "abc-123"));
        assert!(!chat_saved_in(&cfg, "gone"));
        assert!(!chat_saved_in(&cfg, ""));
        assert!(!chat_saved_in(&cfg, "../hash1"));
        let _ = fs::remove_dir_all(&cfg);
    }

    #[test]
    fn the_users_cursor_servers_are_read_as_cursor_reads_them() {
        let text = "{\n  // mine\n  \"mcpServers\": {\n    \"notion\": {\"url\": \"https://x.example/a//b\"}, /* off */\n    \"git\": {\"command\": \"g\",},\n  },\n}";
        let mut names = server_names(text);
        names.sort();
        assert_eq!(names, ["git", "notion"]);
        assert!(server_names("not json").is_empty());
    }

    #[test]
    fn cursor_may_not_read_where_a_sign_in_or_a_key_is_kept() {
        let mut p = permissions(&json!({}), &[], &[], &[]);
        deny_reads(&mut p, &[PathBuf::from("/s/Fridae Studios/github"), PathBuf::from("/kit/config.json")]);
        let deny: Vec<&str> = p["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        for r in ["Read(/s/Fridae Studios/github)", "Read(/s/Fridae Studios/github/**)", "Read(/kit/config.json)", "Read(/kit/config.json/**)"] {
            assert!(deny.contains(&r), "{r}: {deny:?}");
        }
        // the rules Cursor gets each turn carry every secret, the GitHub sign-in among them
        let now = rules_now(&json!({}));
        let deny: Vec<&str> = now["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        let github = as_pattern(&crate::github::folder());
        assert!(deny.contains(&format!("Read({github}/**)").as_str()), "{deny:?}");
        assert!(paths::app_secrets().iter().all(|s| deny.contains(&format!("Read({})", as_pattern(s)).as_str())));
    }

    #[test]
    fn paths_with_glob_characters_still_match_themselves() {
        assert_eq!(as_pattern(Path::new("/Users/Ann [Work]/Lib*")), "/Users/Ann ?Work?/Lib?");
        let list = protected(Path::new("/h/{a}"));
        assert!(list.contains(&"/h/?a?/.*".to_string()), "every hidden name, the folder's own glob characters made ?");
        assert!(list.contains(&"/h/?a?/Library".to_string()));
        assert!(list.iter().any(|p| p.ends_with("processes.json")) && list.iter().any(|p| p.ends_with("agent-tools")), "the app's own files");
        let p = permissions(&json!({}), &[], &[], &list);
        assert!(p["deny"].as_array().unwrap().contains(&json!("Write(/h/?a?/.*)")));
    }

    #[test]
    fn permissions_allow_ours_and_deny_the_rest() {
        let servers = json!({"rbxkit": {"type": "stdio", "command": "py"}, "Roblox_Studio": {"type": "stdio", "command": "studio"}});
        let p = permissions(&servers, &["git".into()], &["rbxkit".into(), "notion".into(), "notion".into()], &["/w/cursor/config".to_string()]);
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
        assert!(deny.contains(&"Mcp(Roblox_Studio:*)") && deny.contains(&"Mcp(*studio*:*)"), "Cursor matches server names in any case");
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
        fs::write(cfg.join("cli-config.json"), r#"{"version": 1, "model": {"modelId": "default"}, "sandbox": {"networkAccess": "x"}, "permissions": {"allow": ["Shell(*)"]}, "maxMode": true, "attribution": {}}"#).unwrap();
        write_config(&cfg, &json!({"allow": ["Mcp(rbxkit:*)"], "deny": []}));
        let back: Value = serde_json::from_str(&fs::read_to_string(cfg.join("cli-config.json")).unwrap()).unwrap();
        assert_eq!(back["permissions"]["allow"], json!(["Mcp(rbxkit:*)"]), "the app's permissions replace any others");
        assert_eq!(back["model"]["modelId"], "default", "what Cursor keeps there stays");
        assert_eq!(back["sandbox"], json!({"mode": "disabled"}), "nothing a turn wrote into the sandbox settings lasts");
        assert!(back.get("maxMode").is_none() && back.get("attribution").is_none(), "only what Cursor itself keeps is kept");
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
        assert_eq!(explain(out, false).as_deref(), Some("Cursor is not signed in. Sign in, then send your message again."));
        assert!(explain(out, true).is_none(), "a turn that answered is not explained by stderr");
        assert!(explain("Cannot use this model: nonsense-model. Available models: auto, gpt-5.2", false).unwrap().contains("Auto always works"));
        assert!(explain("something else", false).is_none());
    }
}
