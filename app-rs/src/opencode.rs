//! OpenCode as a chat's AI, through a server of the app's own: `opencode serve`, driven over HTTP with its event stream.
//!
//!   opencode serve --hostname 127.0.0.1 --port 0      (one per app run, started on first use)
//!
//! A turn: `POST /api/session` (or the saved one, with its rules rewritten), our tool servers added for its folder
//! (`PUT /api/experimental/mcp/<name>`), the app's standing instructions put on the session
//! (`PUT /api/experimental/session/<id>/instructions/entries/fridae`), then `POST /api/session/<id>/prompt`, while
//! `GET /api/event` streams what happens: `session.tool.called`, `session.text.ended`, `session.step.ended` (that
//! step's tokens and cost), `permission.asked`, and one of `session.execution.succeeded|failed|interrupted` at the
//! end. Stop is `POST /api/session/<id>/interrupt`. Checked against opencode v2.3.2 and its own API description
//! (`/openapi.json`). The server's lifecycle and the order of these calls follow t3code's OpenCode 2.x provider
//! (MIT, see THIRD_PARTY.md). Not offered on Windows yet (DEVELOPING.md).
//!
//! How this differs from Claude Code, and why:
//! - OpenCode's desktop app and its command line share one database and a background service. Fridae's chats must
//!   neither show up there nor touch it, so the app's server (and the few one-off commands below) run with folders of
//!   the app's own (`XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and `OPENCODE_CONFIG_DIR` under Application
//!   Support on a Mac), read no project settings (`OPENCODE_DISABLE_PROJECT_CONFIG`) and nothing of Claude Code's.
//!   The user's `~/.config/opencode` and `~/.local/share/opencode` are never written, and the user's own background
//!   service is never used; only the user's default model is read from `opencode.json`.
//! - The server listens on 127.0.0.1 only, on a free port, behind a random password that lives only in its
//!   environment and in this process. It runs in a process group of its own, is noted in the process ledger
//!   (`procs`), stops after IDLE without a turn and when the app quits, and is ended at the next start if a crash
//!   left it behind.
//! - The sign-in lives in that database, so Fridae's copy has a sign-in of its own: free models work without one, and
//!   Setup starts OpenCode's device sign-in for it (`login_start`). Copying the user's sign-in was ruled out: its
//!   refresh token is replaced on every refresh, so two copies would sign each other out.
//! - Everything a turn may do comes from the session's permission rules (the last match wins), rewritten before every
//!   turn: Ask me starts from "deny everything" and allows reading, the chat's own tools, the programs the user
//!   allowed, web pages, and writing in the chat's folders. Any other command *asks*: the app refuses it on the spot
//!   and shows its blocked-command card, and a program the user allows from that card is answered "once" from then on.
//!   Full auto and Studio start from "allow everything", as Claude Code's bypass mode does. In every mode the same
//!   denies follow: Code Mode, OpenCode's own sub-agents, skills, questions, other chats' tools, and writes to
//!   OpenCode's settings, the app's settings and the home folder's hidden files and Library.
//! - One server serves every chat, and tool servers are added per folder, so each chat's tool servers carry a name
//!   of their own (`chat_prefix`): a chat never reaches another chat's tools, which act as that other chat.

use std::{
    collections::HashMap,
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use base64::Engine;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{antigravity::Out, paths, procs};

/// The models Fridae's OpenCode offers, as (id, name), and when the list was last asked for.
static MODELS: Mutex<(Vec<(String, String)>, Option<Instant>)> = Mutex::new((Vec::new(), None));
static LOADING: AtomicBool = AtomicBool::new(false);
/// The integrations Fridae's OpenCode is signed in to, by name (`opencode auth list`).
static SIGNED_IN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The app's OpenCode server, while it runs.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);
/// Turns running now, by chat.
static ACTIVE: Mutex<Option<HashMap<String, Arc<Active>>>> = Mutex::new(None);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// How long the server is kept without a turn before it is stopped (the next turn starts it again in a second or two).
const IDLE: Duration = Duration::from_secs(10 * 60);
/// How long Stop waits for OpenCode to end a turn by itself before the server is ended.
const STOP_WAIT: Duration = Duration::from_secs(12);
/// The event stream sends a heartbeat every 10 to 15 seconds: this long without anything means it is stuck.
const SILENCE: Duration = Duration::from_secs(45);
/// The instructions entry the app keeps on each session.
const INSTRUCTIONS_KEY: &str = "fridae";

// ------------------------------------------------------------------ folders

/// Where the app keeps OpenCode's data (its database, with the sign-in and the saved chats), state, cache and
/// settings: the per-user app data folder, not Documents (which iCloud or OneDrive may sync).
fn store() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("opencode-store");
    #[allow(unreachable_code)]
    {
        let home = paths::home_dir();
        let base = if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local").join("share"))
        };
        base.join("Fridae Studios").join("opencode")
    }
}

/// The XDG folders as the user's own programs see them: what the app was started with, else the usual defaults. Our
/// tool servers get these back, since the OpenCode server itself runs with the app's.
fn users_xdg() -> Vec<(String, String)> {
    let home = paths::home_dir();
    [("XDG_DATA_HOME", home.join(".local").join("share")), ("XDG_STATE_HOME", home.join(".local").join("state")), ("XDG_CACHE_HOME", home.join(".cache"))]
        .into_iter()
        .map(|(k, dflt)| (k.to_string(), env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| dflt.display().to_string())))
        .collect()
}

/// Environment variables of OpenCode's own that would point it back at the user's setup, a server, or a password.
const DROPPED: [&str; 10] = [
    "OPENCODE_CONFIG", "OPENCODE_CONFIG_CONTENT", "OPENCODE_CLI_CONFIG_CONTENT", "OPENCODE_DB", "OPENCODE_SERVER_PASSWORD", "OPENCODE_PASSWORD",
    "OPENCODE_SESSION_ID", "OPENCODE_CONFIG_PROJECT_DISABLE", "OPENCODE_API_KEY", "XDG_CONFIG_DIRS",
];

/// The settings of the app's OpenCode folder: never update the user's OpenCode, never share a chat, no snapshots or
/// language servers in the game's folder, and policies that leave only the chats' own tool servers and no skills.
fn settings() -> Value {
    json!({
        "update": "disable", "autoupdate": false, "share": "disabled", "snapshots": false, "lsp": false, "formatter": false,
        "automations": false, "warming": false,
        "policies": [rule("mcp.use", "*", "deny"), rule("mcp.use", "fr*", "allow"), rule("skill.use", "*", "deny")],
    })
}

/// Every OpenCode process the app starts runs in the app's folders (see the top of this file).
fn environment(cmd: &mut Command) {
    let s = store();
    for k in DROPPED {
        cmd.env_remove(k);
    }
    cmd.env("XDG_DATA_HOME", s.join("data"))
        .env("XDG_STATE_HOME", s.join("state"))
        .env("XDG_CACHE_HOME", s.join("cache"))
        .env("OPENCODE_CONFIG_DIR", s.join("config"))
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
        .env("OPENCODE_DISABLE_CLAUDE_CODE", "1")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1");
    let cfg = s.join("config");
    let _ = fs::create_dir_all(&cfg);
    let _ = fs::write(cfg.join("opencode.json"), serde_json::to_string_pretty(&settings()).unwrap_or_default());
    // testing only, never in a release build: extra settings (a local stand-in model) for a scratch copy of the app
    #[cfg(debug_assertions)]
    if let Some(extra) = env::var_os("FRIDAE_OPENCODE_TEST_CONFIG").filter(|v| !v.is_empty()) {
        cmd.env("OPENCODE_CONFIG", extra);
    }
}

/// A one-off command for Fridae's OpenCode (the model list, the sign-in): its own folders, never the shared service.
fn opencode(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg("--standalone");
    environment(&mut cmd);
    paths::quiet(&mut cmd);
    cmd
}

// ------------------------------------------------------------------- models

/// Ask Fridae's OpenCode which models it may use and what it is signed in to, in the background (it goes to the
/// network). An empty answer leaves the list empty, and `refresh_models` asks again later.
pub fn load_models() {
    if LOADING.swap(true, Ordering::SeqCst) {
        return;
    }
    MODELS.lock().unwrap().1 = Some(Instant::now());
    std::thread::spawn(|| {
        if let Some(exe) = paths::find_opencode() {
            let run = |args: &[&str]| {
                let mut cmd = opencode(&exe);
                cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
                cmd.output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
            };
            let list = parse_models(&run(&["models"]));
            if !list.is_empty() {
                MODELS.lock().unwrap().0 = list;
            }
            *SIGNED_IN.lock().unwrap() = parse_auth_list(&run(&["auth", "list"]));
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

/// A provider as people read it.
fn provider_name(id: &str) -> String {
    match id {
        "opencode-go" => "OpenCode Go".to_string(),
        "opencode" => "OpenCode Zen".to_string(),
        other => other.to_string(),
    }
}

/// The `provider/model` lines of `opencode models`, named "Provider · model" so models of the same name from two
/// providers stay apart, OpenCode Go (the subscription) first, then OpenCode Zen, then the rest, each in OpenCode's order.
pub fn parse_models(text: &str) -> Vec<(String, String)> {
    let mut list: Vec<(usize, String, String)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(char::is_whitespace))
        .filter_map(|l| l.split_once('/').filter(|(p, m)| !p.is_empty() && !m.is_empty()).map(|(p, m)| (l, p, m)))
        .map(|(id, p, m)| {
            let rank = match p {
                "opencode-go" => 0,
                "opencode" => 1,
                _ => 2,
            };
            (rank, id.to_string(), format!("{} · {m}", provider_name(p)))
        })
        .collect();
    list.sort_by_key(|(rank, _, _)| *rank); // stable: OpenCode's own order within each provider
    list.into_iter().map(|(_, id, name)| (id, name)).collect()
}

/// The names in `opencode auth list` ("OpenCode Console  Personal  stored"): the first column.
pub fn parse_auth_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| strip_ansi(l).trim().to_string())
        .filter(|l| l.ends_with("stored") || l.contains("  "))
        .filter_map(|l| l.split("  ").map(str::trim).find(|s| !s.is_empty()).map(str::to_string))
        .filter(|n| !n.is_empty() && n != "stored")
        .collect()
}

/// The models Fridae's OpenCode offers, as (id, name). Empty until the list has arrived.
pub fn models() -> Vec<(String, String)> {
    MODELS.lock().unwrap().0.clone()
}

/// What Fridae's OpenCode is signed in to.
pub fn signed_in() -> Vec<String> {
    SIGNED_IN.lock().unwrap().clone()
}

/// The default model the user set for OpenCode itself (`model` in `~/.config/opencode/opencode.json` or `.jsonc`),
/// read only.
fn users_default() -> Option<String> {
    let dir = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| paths::home_dir().join(".config")).join("opencode");
    ["opencode.jsonc", "opencode.json"].iter().find_map(|f| fs::read_to_string(dir.join(f)).ok()).and_then(|t| model_setting(&t))
}

fn model_setting(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(&crate::cursor::loose_json(text)).ok()?;
    v["model"].as_str().map(str::trim).filter(|m| m.contains('/')).map(|m| m.split('#').next().unwrap_or(m).to_string())
}

/// The model an OpenCode chat uses unless the user picks another: the user's own OpenCode default when Fridae's copy
/// offers it, else the first OpenCode Go model, else the first model listed. Nothing before the list arrives (OpenCode
/// then picks).
pub fn default_model() -> Option<String> {
    default_of(&models(), users_default().as_deref())
}

fn default_of(list: &[(String, String)], users: Option<&str>) -> Option<String> {
    if let Some(u) = users.filter(|u| list.iter().any(|(id, _)| id == u)) {
        return Some(u.to_string());
    }
    list.iter().find(|(id, _)| id.starts_with("opencode-go/")).or(list.first()).map(|(id, _)| id.clone()).or_else(|| users.map(str::to_string))
}

/// The model a turn asks for: the one chosen when OpenCode lists it, or when the list has not arrived (OpenCode itself
/// then says if it cannot use it); otherwise the default.
pub fn pick_model(chosen: Option<&str>) -> Option<String> {
    pick_from(&models(), chosen, users_default().as_deref())
}

fn pick_from(list: &[(String, String)], chosen: Option<&str>, users: Option<&str>) -> Option<String> {
    // a model id that could not be one (an agent's, say) is never put on a command line
    match chosen.filter(|m| !m.is_empty() && crate::expert_ai::model_id_ok(m)) {
        Some(m) if list.is_empty() || list.iter().any(|(id, _)| id == m) => Some(m.to_string()),
        _ => default_of(list, users),
    }
}

/// A model's name as the page shows it ("OpenCode Go · glm-5.3").
pub fn model_name(id: &str) -> String {
    models().into_iter().find(|(m, _)| m == id).map(|(_, n)| n).unwrap_or_else(|| match id.split_once('/') {
        Some((p, m)) => format!("{} · {m}", provider_name(p)),
        None => id.to_string(),
    })
}

/// What OpenCode can do in Full auto or in Studio that Claude Code cannot there, as one plain line the page shows under
/// the message box. Nothing known: Claude Code runs both with every check off, while OpenCode keeps its deny rules
/// (measured, see DEVELOPING.md). A gap found later goes here.
pub fn gaps() -> Value {
    json!({"full": null, "studio": null})
}

// -------------------------------------------------------------- permissions

/// A path as a pattern: OpenCode's patterns know only `*` and `?`, so those characters in a path become `?` (one
/// character), and the rule still covers the path.
fn as_pattern(path: &Path) -> String {
    path.display().to_string().chars().map(|c| if matches!(c, '*' | '?') { '?' } else { c }).collect()
}

/// A folder's patterns, as written and as the system resolves it (/tmp is /private/tmp on a Mac): OpenCode matches
/// the resolved path.
fn spellings(dir: &Path) -> Vec<String> {
    let mut out = vec![as_pattern(dir)];
    if let Ok(real) = dir.canonicalize() {
        let r = as_pattern(&real);
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

fn rule(action: &str, resource: &str, effect: &str) -> Value {
    json!({"action": action, "resource": resource, "effect": effect})
}

/// The start of every tool server name of one chat: "fr" and eight characters of the chat's id, hashed (OpenCode
/// puts a server's name in front of each of its tools, and tool names must stay short).
pub fn chat_prefix(agent_id: &str) -> String {
    let h = Sha256::digest(agent_id.as_bytes());
    format!("fr{}-", h.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

/// A chat's name for one of its tool servers.
fn server_name(agent_id: &str, server: &str) -> String {
    let clean: String = server.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    chat_prefix(agent_id) + &clean
}

/// The folders a turn works with: its working folder (the game's, or the chat's own when it has none), the kit, and
/// the app's work folder (where attachments and made files live).
pub struct Folders<'a> {
    pub location: &'a Path,
    pub project: Option<&'a Path>,
    pub kit: &'a Path,
    pub work: &'a Path,
}

/// OpenCode's permission rules for a turn of chat `agent_id` (the last rule that matches wins). `allowed` the programs
/// the user allowed; `protect` patterns of folders and files no file tool may write in any mode (see `protected`).
pub fn permissions(full: bool, agent_id: &str, allowed: &[String], folders: &Folders, protect: &[String]) -> Vec<Value> {
    let mine = format!("{}*", chat_prefix(agent_id));
    // anything not allowed here is refused, in every mode: OpenCode also has tools Claude Code is never given (a
    // browser it drives, image generation, moving its sessions between folders), and a denied tool is not offered
    let mut r = vec![rule("*", "*", "deny")];
    for a in ["read", "glob", "grep", "list"] {
        r.push(rule(a, "*", "allow")); // reading is not limited in the chat's folders, as for Claude Code
    }
    if full {
        // Full auto and Studio: any command, any file, any folder, the web, as Claude Code's bypass mode, except the
        // denies below
        for a in ["shell", "edit", "external_directory", "webfetch", "websearch"] {
            r.push(rule(a, "*", "allow"));
        }
    } else {
        // a command asks, so the app can refuse it with its card (or answer "once" for a program allowed meanwhile);
        // the allowed programs run without asking ("git *" also matches a bare "git")
        r.push(rule("shell", "*", "ask"));
        for p in allowed {
            r.push(rule("shell", &format!("{p} *"), "allow"));
        }
        // a path inside the working folder comes as a relative one; anything else (or one going up) is refused, then
        // the chat's other folders are allowed again
        r.push(rule("edit", "*", "allow"));
        r.push(rule("edit", "/*", "deny"));
        r.push(rule("edit", "../*", "deny"));
        let mut dirs: Vec<&Path> = vec![folders.location, folders.kit, folders.work];
        dirs.extend(folders.project);
        for d in dirs {
            for p in spellings(d) {
                r.push(rule("edit", &format!("{p}/*"), "allow"));
                // outside the working folder OpenCode asks about the folder first: only these folders pass
                r.push(rule("external_directory", &p, "allow"));
                r.push(rule("external_directory", &format!("{p}/*"), "allow"));
            }
        }
        r.push(rule("webfetch", "*", "allow"));
        r.push(rule("websearch", "*", "allow"));
        r.push(rule(&mine, "*", "allow"));
    }
    // every mode: other chats' tools are never this chat's (they act as that chat)
    r.push(rule("fr????????-*", "*", "deny"));
    r.push(rule(&mine, "*", "allow"));
    // tools run one by one where the chat sees them (Code Mode would run them inside a script), no sub-agents of
    // OpenCode's own (they bring rules of their own), no skills, no questions (nobody can answer)
    for a in ["execute", "subagent", "task", "skill", "question"] {
        r.push(rule(a, "*", "deny"));
    }
    // never OpenCode's own settings, wherever they are
    for p in [".opencode/*", "*/.opencode/*", "*opencode.json", "*opencode.jsonc"] {
        r.push(rule("edit", p, "deny"));
    }
    for p in protect {
        r.push(rule("edit", p, "deny"));
        r.push(rule("edit", &format!("{p}/*"), "deny"));
    }
    r
}

/// What no file tool may write in any mode, as patterns: the app's OpenCode folders, the app's own settings file (it
/// holds the programs agents may run), and the home folder's hidden files and folders (shell start-up files, other
/// tools' settings) and Library.
fn protected(home: &Path) -> Vec<String> {
    let mut list: Vec<String> = [store(), paths::PATHS.home.join("config.json")].iter().flat_map(|p| spellings(p)).collect();
    if !home.as_os_str().is_empty() {
        for h in spellings(home) {
            list.push(format!("{h}/.*")); // a pattern on purpose: every hidden name
            list.push(format!("{h}/Library"));
        }
    }
    list
}

/// Our tool servers as OpenCode takes them (`PUT /api/experimental/mcp/<name>`), under the chat's own names, with the
/// user's XDG folders back in their environment. Code Mode is off for each, so every tool call is its own step.
pub fn servers_config(agent_id: &str, servers: &Value, extra: &[(String, String)]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (name, s) in servers.as_object().unwrap_or(&Map::new()) {
        let mut command: Vec<Value> = vec![s["command"].clone()];
        command.extend(s["args"].as_array().cloned().unwrap_or_default());
        let mut environment = s["env"].as_object().cloned().unwrap_or_default();
        for (k, v) in extra {
            environment.entry(k.clone()).or_insert_with(|| json!(v));
        }
        out.push((
            server_name(agent_id, name),
            json!({"type": "local", "command": command, "environment": environment, "codemode": false,
                   "timeout": {"execution": 900_000}}), // a texture request may wait minutes for the user
        ));
    }
    out
}

/// How OpenCode runs here, in its own words: its folders, the allowed programs and the blocked card, what Full auto
/// changes, the web.
pub fn instructions(project: Option<&str>, allowed: &[String], full: bool) -> String {
    let game = match project.filter(|p| !p.is_empty()) {
        Some(p) => format!("Your working directory is the game's folder, {p}."),
        None => "Your working directory is a scratch folder the app keeps for this chat; this chat has no game.".to_string(),
    };
    let programs = if allowed.is_empty() { "none yet".to_string() } else { allowed.join(", ") };
    let mode = if full {
        "The user has turned on full auto: any program may run, and your file tools may write outside those folders too, except OpenCode's and the app's own settings and the hidden files and Library in the home folder."
    } else {
        "Any other program is refused, and so is writing anywhere else."
    };
    include_str!("text/opencode.txt").replace("{game}", &game).replace("{programs}", &programs).replace("{mode}", mode)
}

// ------------------------------------------------------------------- server

struct Server {
    child: Child,
    port: u16,
    auth: String,
    last_used: Instant,
}

/// How to reach the server: its port and the Authorization header.
#[derive(Clone)]
struct Conn {
    port: u16,
    auth: String,
}

/// A password nobody can guess: 24 random bytes as hex (from the system's random source; a hash of the time and the
/// process as a fallback).
fn password() -> String {
    let mut bytes = [0u8; 24];
    let ok = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok();
    if !ok {
        let seed = format!("{:?}{}{:p}", std::time::SystemTime::now(), std::process::id(), &bytes);
        bytes[..24].copy_from_slice(&Sha256::digest(seed.as_bytes())[..24]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The port in the line the server prints once it listens ("server listening on http://127.0.0.1:58866").
fn listening_port(line: &str) -> Option<u16> {
    let rest = line.split("listening on http://127.0.0.1:").nth(1)?;
    rest.trim().trim_end_matches('/').parse().ok()
}

/// The running server, started if need be. Any caller waits while it starts (a second or two).
fn ensure() -> Result<Conn, String> {
    let mut guard = SERVER.lock().unwrap();
    if let Some(s) = guard.as_mut() {
        if matches!(s.child.try_wait(), Ok(None)) {
            s.last_used = Instant::now();
            return Ok(Conn { port: s.port, auth: s.auth.clone() });
        }
        procs::forget(s.child.id());
        *guard = None;
    }
    let exe = paths::find_opencode().ok_or("OpenCode is not installed.")?;
    let pw = password();
    let mut cmd = Command::new(&exe);
    cmd.args(["serve", "--hostname", "127.0.0.1", "--port", "0"]);
    environment(&mut cmd);
    cmd.env("OPENCODE_PASSWORD", &pw).current_dir(store()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::quiet(&mut cmd);
    paths::own_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("Could not start OpenCode: {e}"))?;
    procs::record("opencode", child.id());
    let (tx, rx) = mpsc::channel::<u16>();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let Some(out) = stdout else { return };
        let mut told = false;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if !told && let Some(p) = listening_port(&line) {
                told = true;
                let _ = tx.send(p);
            }
        }
    });
    // the last of what it says on stderr, for a message when it does not start (never the password: it is not printed)
    let tail: Arc<Mutex<String>> = Arc::default();
    if let Some(err) = child.stderr.take() {
        let tail = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let mut t = tail.lock().unwrap();
                t.push_str(&line);
                t.push('\n');
                if t.len() > 2000 {
                    let cut = t.len() - 2000;
                    let cut = (cut..t.len()).find(|&i| t.is_char_boundary(i)).unwrap_or(t.len());
                    t.drain(..cut);
                }
            }
        });
    }
    let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("opencode:{pw}")));
    let port = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(p) => p,
        Err(_) => {
            let pid = child.id();
            paths::end_trees(&[pid]);
            let _ = child.wait();
            procs::forget(pid);
            let said = tail.lock().unwrap().trim().to_string();
            return Err(if said.is_empty() { "OpenCode's server did not start.".to_string() } else { format!("OpenCode's server did not start: {said}") });
        }
    };
    let conn = Conn { port, auth: auth.clone() };
    // this API is OpenCode 2's
    match call(&conn, "GET", "/api/info", None) {
        Ok((200, _)) => {}
        Ok((404, _)) => {
            let pid = child.id();
            paths::end_trees(&[pid]);
            let _ = child.wait();
            procs::forget(pid);
            return Err("Fridae needs OpenCode 2 or newer. Run `opencode upgrade` in a terminal, then send your message again.".to_string());
        }
        _ => {}
    }
    *guard = Some(Server { child, port, auth, last_used: Instant::now() });
    drop(guard);
    if !WATCHING.swap(true, Ordering::SeqCst) {
        std::thread::spawn(watch_idle);
    }
    Ok(conn)
}

/// The server's address while it runs, without starting it.
fn current() -> Option<Conn> {
    SERVER.lock().unwrap().as_ref().map(|s| Conn { port: s.port, auth: s.auth.clone() })
}

fn active() -> std::sync::MutexGuard<'static, Option<HashMap<String, Arc<Active>>>> {
    let mut g = ACTIVE.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

/// Stops the server once it has gone IDLE without a turn.
fn watch_idle() {
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let idle = active().as_ref().is_none_or(|a| a.is_empty()) && SERVER.lock().unwrap().as_ref().is_some_and(|s| s.last_used.elapsed() >= IDLE);
        if idle {
            stop_server();
        }
    }
}

/// End the server and everything it started: its process group (SIGTERM, then SIGKILL after a grace), then the
/// terminal helper it keeps in a group of its own (`opencode-pty`, which a SIGTERM to the server ends by itself).
fn stop_server() {
    let Some(mut s) = SERVER.lock().unwrap().take() else { return };
    let pid = s.child.id();
    paths::end_trees(&[pid]);
    let _ = s.child.wait();
    procs::forget(pid);
    sweep_helpers();
}

/// OpenCode's terminal helper runs from the app's own OpenCode folder: any left (the server was killed) is ended.
fn sweep_helpers() {
    let prefix = store().join("cache").join("opencode").join("bin").join("opencode-pty");
    let groups: Vec<u32> = procs::orphans_under(&prefix.display().to_string()).into_iter().map(|(pid, pgid)| if pgid == pid { pgid } else { pid }).collect();
    if !groups.is_empty() {
        paths::end_trees(&groups);
    }
}

/// At app start: what a crashed run of the app left (see `procs`), and any terminal helper of ours.
pub fn reap() {
    sweep_helpers();
}

/// When the app quits: no sign-in waits, and the server and all it started end.
pub fn shut_down() {
    login_cancel();
    stop_server();
}

// --------------------------------------------------------------------- http

/// A request to the server, answered as (status, JSON body; Null when empty). "database is locked" is tried again
/// three times, a quarter of a second apart (as t3code does).
fn call(conn: &Conn, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let mut tries = 0;
    loop {
        let (status, text) = request(conn, method, path, body, Duration::from_secs(60))?;
        if status >= 500 && text.to_lowercase().contains("database is locked") && tries < 3 {
            tries += 1;
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let v = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        return Ok((status, v));
    }
}

fn connect(conn: &Conn, read_timeout: Duration) -> Result<TcpStream, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], conn.port));
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("OpenCode's server did not answer: {e}"))?;
    let _ = s.set_read_timeout(Some(read_timeout));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_nodelay(true);
    Ok(s)
}

fn head(conn: &Conn, method: &str, path: &str, accept: &str, body: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nAccept: {accept}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        conn.port,
        conn.auth,
        body.len()
    )
}

/// The status line and headers (lower-cased names) of a response.
fn read_head<R: BufRead>(r: &mut R) -> Result<(u16, Vec<(String, String)>), String> {
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("OpenCode's server sent no answer.")?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok((status, headers))
}

fn chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"))
}

/// A body sent in chunks, read as one stream.
struct Chunked<R: BufRead> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut size = String::new();
            // the CRLF after the previous chunk, then the size line
            while size.trim().is_empty() {
                size.clear();
                if self.inner.read_line(&mut size)? == 0 {
                    self.done = true;
                    return Ok(0);
                }
            }
            let hex = size.trim().split(';').next().unwrap_or("");
            self.left = usize::from_str_radix(hex, 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk"))?;
            if self.left == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let want = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            self.done = true;
        }
        self.left -= n;
        Ok(n)
    }
}

fn request(conn: &Conn, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<(u16, String), String> {
    let mut s = connect(conn, timeout)?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    s.write_all(head(conn, method, path, "application/json", &body).as_bytes()).map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let (status, headers) = read_head(&mut r)?;
    let mut bytes = Vec::new();
    if chunked(&headers) {
        Chunked { inner: r, left: 0, done: false }.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    } else if let Some(n) = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<u64>().ok()) {
        r.take(n).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    } else {
        r.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    }
    Ok((status, String::from_utf8_lossy(&bytes).to_string()))
}

/// The server's event stream: one JSON event at a time.
struct Events {
    r: BufReader<Box<dyn Read + Send>>,
}

impl Events {
    fn open(conn: &Conn) -> Result<Events, String> {
        let mut s = connect(conn, SILENCE)?;
        s.write_all(head(conn, "GET", "/api/event", "text/event-stream", "").as_bytes()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(s);
        let (status, headers) = read_head(&mut r)?;
        if status != 200 {
            return Err(format!("OpenCode's event stream answered {status}."));
        }
        let body: Box<dyn Read + Send> = if chunked(&headers) { Box::new(Chunked { inner: r, left: 0, done: false }) } else { Box::new(r) };
        Ok(Events { r: BufReader::new(body) })
    }

    /// The next event; None when the stream has ended. A stream silent for SILENCE (no heartbeat) is an error.
    fn next(&mut self) -> Result<Option<Value>, String> {
        let mut data = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match self.r.read_line(&mut line) {
                Ok(0) => return Ok(None),
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err("silent".to_string()),
                Err(e) => return Err(e.to_string()),
            }
            let l = line.trim_end_matches(['\r', '\n']);
            if l.is_empty() {
                if !data.is_empty() {
                    if let Ok(v) = serde_json::from_str::<Value>(&data) {
                        return Ok(Some(v));
                    }
                    data.clear();
                }
                continue;
            }
            if let Some(d) = l.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(d.strip_prefix(' ').unwrap_or(d));
            }
        }
    }
}

fn quote(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// `?location[directory]=<folder>`, as OpenCode's API takes a folder.
fn at(dir: &Path) -> String {
    format!("?location%5Bdirectory%5D={}", quote(&dir.display().to_string()))
}

/// A short reason from an error answer.
fn said(v: &Value) -> String {
    v["message"].as_str().or(v["error"]["message"].as_str()).or(v.as_str()).unwrap_or("").chars().take(300).collect()
}

// -------------------------------------------------------------------- turns

/// One turn of a chat on OpenCode.
pub struct Spec {
    pub agent_id: String,
    pub title: String,
    pub location: PathBuf,
    pub project: Option<PathBuf>,
    pub servers: Value,
    pub session: Option<String>,
    pub model: Option<String>,
    pub full: bool,
    pub instructions: String,
    pub text: String,
}

/// How a turn ended, beyond what its events said: a reason it could not run, and whether Stop had to end the server.
#[derive(Default)]
pub struct Ended {
    pub err: String,
    pub killed: bool,
}

/// A running turn, for Stop.
#[derive(Default)]
struct Active {
    session: Mutex<Option<String>>,
    cancelled: AtomicBool,
    done: AtomicBool,
    killed: AtomicBool,
}

/// Whether the server still has a saved chat (it is started if need be). A chat it no longer has starts afresh.
pub fn chat_saved(session: &str) -> bool {
    let Ok(conn) = ensure() else { return true }; // no server: the turn says why
    !matches!(call(&conn, "GET", &format!("/api/session/{}", quote(session)), None), Ok((404, _)))
}

/// A model id ("opencode-go/glm-5.3") as OpenCode's model reference.
fn model_ref(id: &str) -> Option<Value> {
    let (p, m) = id.split_once('/').filter(|(p, m)| !p.is_empty() && !m.is_empty())?;
    Some(json!({"providerID": p, "id": m}))
}

/// Run one turn, handing what happens to `put` as it comes. Returns once the turn has ended.
pub fn run(spec: &Spec, mut put: impl FnMut(Out)) -> Ended {
    let me = Arc::new(Active::default());
    active().as_mut().unwrap().insert(spec.agent_id.clone(), me.clone());
    let ended = run_with(spec, &me, &mut put);
    me.done.store(true, Ordering::SeqCst);
    {
        let mut g = active();
        let a = g.as_mut().unwrap();
        if a.get(&spec.agent_id).is_some_and(|x| Arc::ptr_eq(x, &me)) {
            a.remove(&spec.agent_id);
        }
    }
    if let Some(s) = SERVER.lock().unwrap().as_mut() {
        s.last_used = Instant::now();
    }
    Ended { killed: me.killed.load(Ordering::SeqCst), ..ended }
}

fn fail(err: impl Into<String>) -> Ended {
    Ended { err: err.into(), killed: false }
}

fn run_with(spec: &Spec, me: &Active, put: &mut impl FnMut(Out)) -> Ended {
    let conn = match ensure() {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let folders = Folders { location: &spec.location, project: spec.project.as_deref(), kit: &paths::PATHS.kit, work: &paths::PATHS.work };
    let rules = permissions(spec.full, &spec.agent_id, &paths::allowed_commands(), &folders, &protected(&paths::home_dir()));
    let model = spec.model.as_deref().and_then(model_ref);
    // the saved conversation, with this turn's rules, or a new one
    let mut session = None;
    if let Some(id) = spec.session.as_deref().filter(|s| !s.is_empty()) {
        let path = format!("/api/session/{}", quote(id));
        match call(&conn, "PATCH", &path, Some(&json!({"permissions": rules}))) {
            Ok((s, _)) if s < 300 => session = Some(id.to_string()),
            Ok((404, _)) => {}
            Ok((s, v)) => return fail(format!("OpenCode could not reopen this chat ({s}): {}", said(&v))),
            Err(e) => return fail(e),
        }
        if let (Some(_), Some(m)) = (&session, &model) {
            let _ = call(&conn, "POST", &format!("{path}/model"), Some(&json!({"model": m})));
        }
    }
    let session = match session {
        Some(s) => s,
        None => {
            let mut body = json!({"title": spec.title.chars().take(80).collect::<String>(), "location": {"directory": spec.location}, "permissions": rules});
            if let Some(m) = &model {
                body["model"] = m.clone();
            }
            match call(&conn, "POST", "/api/session", Some(&body)) {
                Ok((200, v)) if v["data"]["id"].is_string() => v["data"]["id"].as_str().unwrap().to_string(),
                Ok((s, v)) => return fail(format!("OpenCode could not start a chat ({s}): {}", said(&v))),
                Err(e) => return fail(e),
            }
        }
    };
    *me.session.lock().unwrap() = Some(session.clone());
    put(Out::Conversation(session.clone()));
    if let Some(m) = &spec.model {
        put(Out::Model(model_name(m)));
    }
    // the chat's tool servers for its folder (added again every turn: an idle folder drops them, and an unchanged one
    // keeps its connection)
    let mut bad = Vec::new();
    for (name, cfg) in servers_config(&spec.agent_id, &spec.servers, &users_xdg()) {
        match call(&conn, "PUT", &format!("/api/experimental/mcp/{}{}", quote(&name), at(&spec.location)), Some(&json!({"config": cfg}))) {
            Ok((s, _)) if s < 300 => {}
            _ => bad.push(name.trim_start_matches(&chat_prefix(&spec.agent_id)).to_string()),
        }
    }
    if !bad.is_empty() {
        put(Out::Error(format!("Could not connect to: {}. Open Setup to see what is missing.", bad.join(", "))));
    }
    // the app's standing instructions, given afresh every turn
    match call(&conn, "PUT", &format!("/api/experimental/session/{}/instructions/entries/{INSTRUCTIONS_KEY}", quote(&session)), Some(&json!({"value": spec.instructions}))) {
        Ok((s, _)) if s < 300 => {}
        Ok((s, v)) => return fail(format!("OpenCode did not take the app's instructions ({s}): {}", said(&v))),
        Err(e) => return fail(e),
    }
    if me.cancelled.load(Ordering::SeqCst) {
        return Ended::default();
    }
    // the event stream first, so nothing of the turn is missed
    let mut events = match Events::open(&conn) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    match call(&conn, "POST", &format!("/api/session/{}/prompt", quote(&session)), Some(&json!({"text": spec.text}))) {
        Ok((200, _)) => {}
        Ok((409, _)) => return fail("OpenCode is still busy with this chat's last message. Wait a moment, or press Stop, then send again."),
        Ok((s, v)) => return fail(format!("OpenCode did not take the message ({s}): {}", said(&v))),
        Err(e) => return fail(e),
    }
    let mut turn = Turn::new(&spec.agent_id, &spec.servers, spec.model.clone());
    let mut asked: std::collections::HashSet<String> = Default::default();
    loop {
        let e = match events.next() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(why) => {
                for o in turn.finish() {
                    put(o);
                }
                let text = if why == "silent" { "OpenCode stopped answering (nothing came for 45 seconds)." } else { "The connection to OpenCode was lost." };
                return fail(text);
            }
        };
        let step = turn.feed(&session, &e);
        for o in step.outs {
            put(o);
        }
        if let Some(ask) = step.ask.filter(|a| asked.insert(a.id.clone())) {
            let d = decide(&ask.action, &ask.resources, &paths::allowed_commands());
            if !d.blocked.is_empty() {
                put(Out::Blocked(d.blocked.clone()));
            }
            let mut body = json!({"decision": if d.once { "once" } else { "reject" }});
            if !d.once {
                body["message"] = json!(d.message);
            }
            let _ = call(&conn, "POST", &format!("/api/session/{}/permission/{}/reply", quote(&session), quote(&ask.id)), Some(&body));
        }
        if step.ended {
            for o in turn.finish() {
                put(o);
            }
            return Ended::default();
        }
    }
    for o in turn.finish() {
        put(o);
    }
    fail(if me.killed.load(Ordering::SeqCst) { "" } else { "The connection to OpenCode was lost." })
}

/// Stop a chat's turn: OpenCode is asked to end it; if it has not within STOP_WAIT, the server is ended (with every
/// OpenCode turn running), and the turn says so. Blocks until then: call it off the page's thread.
pub fn stop(agent_id: &str) {
    let Some(me) = active().as_ref().and_then(|a| a.get(agent_id).cloned()) else { return };
    me.cancelled.store(true, Ordering::SeqCst);
    let session = me.session.lock().unwrap().clone();
    let until = Instant::now() + STOP_WAIT;
    // asked from a thread of its own, so a server that does not answer still gets only STOP_WAIT
    if let (Some(s), Some(conn)) = (session, current()) {
        std::thread::spawn(move || {
            let _ = request(&conn, "POST", &format!("/api/session/{}/interrupt", quote(&s)), None, Duration::from_secs(5));
        });
    }
    while !me.done.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !me.done.load(Ordering::SeqCst) {
        me.killed.store(true, Ordering::SeqCst);
        stop_server();
    }
}

/// What the app answers a permission request: "once" for a command whose programs are all allowed, otherwise a
/// refusal with a word for the model, and the commands the page offers to allow.
pub struct Decision {
    pub once: bool,
    pub message: String,
    pub blocked: Vec<Value>,
}

pub fn decide(action: &str, resources: &[String], allowed: &[String]) -> Decision {
    if action == "shell" && !resources.is_empty() {
        let mut blocked: Vec<Value> = Vec::new();
        for command in resources {
            match paths::program_name(command) {
                Some(n) if allowed.contains(&n) => {}
                Some(n) => {
                    if !blocked.iter().any(|b| b["name"] == n) {
                        blocked.push(json!({"name": n, "command": command.chars().take(200).collect::<String>()}));
                    }
                }
                None => blocked.push(json!({"name": "", "command": command.chars().take(200).collect::<String>()})),
            }
        }
        if blocked.is_empty() {
            return Decision { once: true, message: String::new(), blocked };
        }
        let names: Vec<&str> = blocked.iter().filter_map(|b| b["name"].as_str()).filter(|n| !n.is_empty()).collect();
        let message = format!(
            "The user has not allowed {} here, so it was not run. Do not try another way to run it; say in one line what you wanted to run and why, and carry on with the rest.",
            if names.is_empty() { "this command".to_string() } else { names.join(", ") }
        );
        blocked.retain(|b| b["name"] != "");
        return Decision { once: false, message, blocked };
    }
    let message = match action {
        "external_directory" => "That folder is outside this chat's folders, so it is not allowed here.",
        _ => "That is not allowed here.",
    };
    Decision { once: false, message: message.to_string(), blocked: Vec::new() }
}

/// A permission request of the turn's session.
pub struct Ask {
    pub id: String,
    pub action: String,
    pub resources: Vec<String>,
}

/// What one event means for the turn.
#[derive(Default)]
pub struct Step {
    pub outs: Vec<Out>,
    pub ask: Option<Ask>,
    pub ended: bool,
}

/// Reads one turn's events.
pub struct Turn {
    model: Option<String>,
    prefixes: Vec<(String, String)>,
    tools: HashMap<String, String>,
    usage: crate::usage::Turn,
    cost: f64,
    answered: bool,
}

impl Turn {
    /// `servers` are the chat's tool servers, whose tools come as `<chat prefix><server>_<tool>`.
    pub fn new(agent_id: &str, servers: &Value, model: Option<String>) -> Turn {
        let mut prefixes: Vec<(String, String)> = servers.as_object().map(|o| o.keys().map(|n| (server_name(agent_id, n) + "_", n.clone())).collect()).unwrap_or_default();
        prefixes.sort_by_key(|(p, _)| std::cmp::Reverse(p.len())); // the longest name first
        Turn { model, prefixes, tools: HashMap::new(), usage: Default::default(), cost: 0.0, answered: false }
    }

    /// A tool's name as the chat knows it: a server's tool as `mcp__<server>__<tool>`, as Claude Code names it (so
    /// Studio steps are recognised), the built-in ones by Claude Code's names; None for OpenCode's own bookkeeping.
    fn tool_name(&self, tool: &str) -> Option<String> {
        if let Some((p, server)) = self.prefixes.iter().find(|(p, _)| tool.starts_with(p.as_str())) {
            return Some(format!("mcp__{server}__{}", &tool[p.len()..]));
        }
        Some(
            match tool {
                "shell" | "bash" => "run_command",
                "edit" | "patch" | "apply_patch" | "multiedit" => "Edit",
                "write" => "Write",
                "read" => "Read",
                "glob" => "Glob",
                "grep" => "Grep",
                "list" => "list_dir",
                "webfetch" => "WebFetch",
                "websearch" => "WebSearch",
                "task" | "subagent" => "Task",
                "todowrite" | "todoread" | "question" | "invalid" => return None,
                other => other,
            }
            .to_string(),
        )
    }

    /// One event of the server's stream; events of other sessions are ignored.
    pub fn feed(&mut self, session: &str, e: &Value) -> Step {
        let mut step = Step::default();
        let d = &e["data"];
        if d["sessionID"].as_str() != Some(session) {
            return step;
        }
        match e["type"].as_str().unwrap_or("") {
            "session.text.ended" => {
                if let Some(t) = d["text"].as_str().map(str::trim).filter(|t| !t.is_empty()) {
                    step.outs.push(Out::Text(t.to_string()));
                }
            }
            "session.tool.called" => {
                let Some(name) = self.tool_name(d["name"].as_str().unwrap_or_else(|| self.tools.get(d["id"].as_str().unwrap_or("")).map(String::as_str).unwrap_or(""))) else { return step };
                if let Some(id) = d["id"].as_str() {
                    self.tools.insert(id.to_string(), name.clone());
                }
                step.outs.push(if name.starts_with("mcp__") { Out::ToolArgs(name, d["input"].clone()) } else { Out::Tool(name) });
            }
            "session.tool.input.started" => {
                // the name comes here; `session.tool.called` has only the call's id and input
                if let (Some(id), Some(n)) = (d["id"].as_str(), d["name"].as_str()) {
                    self.tools.insert(id.to_string(), n.to_string());
                }
            }
            "session.tool.success" => {
                let name = self.tools.get(d["id"].as_str().unwrap_or("")).cloned().unwrap_or_default();
                let short = name.rsplit("__").next().unwrap_or("");
                // only pictures from our own tools and Studio screenshots are shown
                if name.starts_with("mcp__") && (short.starts_with("rbx_") || short.starts_with("screen_capture")) {
                    let pictures: Vec<String> = d["content"].as_array().into_iter().flatten().filter_map(picture).take(4).collect();
                    if !pictures.is_empty() {
                        step.outs.push(Out::Encoded(pictures));
                    }
                }
            }
            // a step stopped part way (Stop) still used what it used
            "session.step.ended" | "session.step.failed" => {
                let t = &d["tokens"];
                let n = |v: &Value| v.as_u64().unwrap_or(0);
                self.usage.input += n(&t["input"]);
                self.usage.output += n(&t["output"]) + n(&t["reasoning"]); // OpenCode counts thinking apart
                self.usage.cache_read += n(&t["cache"]["read"]);
                self.usage.cache_write += n(&t["cache"]["write"]);
                self.cost += d["cost"].as_f64().unwrap_or(0.0);
            }
            "permission.asked" => {
                step.ask = Some(Ask {
                    id: d["id"].as_str().unwrap_or("").to_string(),
                    action: d["action"].as_str().unwrap_or("").to_string(),
                    resources: d["resources"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
                });
                step.ask = step.ask.filter(|a| !a.id.is_empty());
            }
            "session.execution.succeeded" => {
                self.answered = true;
                step.ended = true;
            }
            "session.execution.failed" => {
                self.answered = true;
                step.outs.push(Out::Failed(failure_text(&d["error"], self.model.as_deref())));
                step.ended = true;
            }
            "session.execution.interrupted" => step.ended = true,
            _ => {}
        }
        step
    }

    /// Once the turn has ended: what it used, and whether OpenCode answered (a stopped turn did not).
    pub fn finish(&mut self) -> Vec<Out> {
        let mut usage = std::mem::take(&mut self.usage);
        if self.cost > 0.0 {
            usage.cost = Some(self.cost);
        }
        self.cost = 0.0;
        let mut out = vec![Out::Usage(usage)];
        if std::mem::take(&mut self.answered) {
            out.push(Out::Finished);
        }
        out
    }
}

/// A picture in a tool's result, as base64: `{type: "file", uri: "data:image/png;base64,…"}` (OpenCode's MCP
/// results), or `{type: "image", data}`.
fn picture(c: &Value) -> Option<String> {
    if let Some(uri) = c["uri"].as_str().or(c["url"].as_str()) {
        let (head, data) = uri.split_once(',')?;
        return (head.starts_with("data:image/") && head.ends_with(";base64") && !data.is_empty()).then(|| data.to_string());
    }
    (c["type"] == "image").then(|| c["data"].as_str().filter(|d| !d.is_empty()).map(str::to_string)).flatten()
}

/// A plain message for a failed turn's error.
fn failure_text(err: &Value, model: Option<&str>) -> String {
    let message = err["message"].as_str().or(err.as_str()).unwrap_or("OpenCode stopped with an error.").trim().to_string();
    match err["type"].as_str().unwrap_or("") {
        "provider.no-route" => format!(
            "{message}. Fridae's OpenCode cannot use {} now: it is not signed in to that provider, or the model is gone. Sign in under Setup, Connections, OpenCode for Fridae, or pick another model under the message box.",
            model.unwrap_or("this model")
        ),
        "provider.free-limit" => format!("{message} Pick another model under the message box, or sign Fridae's OpenCode in to your plan under Setup, Connections."),
        _ => message,
    }
}

/// A plain-language message for why a turn could not run, if we recognise it.
pub fn explain(err: &str, got_result: bool) -> Option<String> {
    if got_result || err.trim().is_empty() {
        return None;
    }
    let low = err.to_lowercase();
    if low.contains("unauthorized") || low.contains("not authenticated") || low.contains("no credentials") {
        return Some("OpenCode is not signed in for Fridae. Open Setup, Connections, and press Sign in to OpenCode for Fridae, then send your message again.".to_string());
    }
    if low.contains("database") && (low.contains("locked") || low.contains("migration")) {
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return Some(format!("OpenCode could not open Fridae's copy of its database: {line}"));
    }
    Some(err.trim().to_string())
}

// ------------------------------------------------------------------ sign in

/// The device sign-in Setup starts: OpenCode prints a page and a code, then waits until the user approves it there.
#[derive(Default)]
struct Login {
    pid: Option<u32>,
    state: &'static str,
    url: String,
    code: String,
    message: String,
}

static LOGIN: Mutex<Login> = Mutex::new(Login { pid: None, state: "", url: String::new(), code: String::new(), message: String::new() });

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The sign-in page and code in what `opencode auth login` prints.
pub fn login_line(line: &str) -> (Option<String>, Option<String>) {
    let line = strip_ansi(line);
    let url = line.split_whitespace().find(|w| w.starts_with("https://opencode.ai/")).map(str::to_string);
    let code = line.split_once("Enter code:").map(|(_, c)| c.trim().to_string()).filter(|c| !c.is_empty() && !c.contains(char::is_whitespace));
    (url, code)
}

/// Start signing Fridae's OpenCode in to the user's OpenCode account (the OpenCode Console: OpenCode Go and Zen).
pub fn login_start() -> Value {
    let Some(exe) = paths::find_opencode() else { return json!({"ok": false, "error": "OpenCode is not installed."}) };
    if LOGIN.lock().unwrap().state == "waiting" {
        return login_state();
    }
    let mut cmd = opencode(&exe);
    cmd.args(["auth", "login", "opencode", "--method", "device"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::own_group(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("Could not start OpenCode: {e}")}),
    };
    *LOGIN.lock().unwrap() = Login { pid: Some(child.id()), state: "waiting", ..Default::default() };
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut last = String::new();
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let (url, code) = login_line(&line);
                let mut l = LOGIN.lock().unwrap();
                if let Some(u) = url {
                    l.url = u;
                }
                if let Some(c) = code {
                    l.code = c;
                }
                let plain = strip_ansi(&line).trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != ')').trim().to_string();
                if !plain.is_empty() {
                    last = plain;
                }
            }
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        {
            let mut l = LOGIN.lock().unwrap();
            l.pid = None;
            if l.state == "waiting" {
                l.state = if ok { "done" } else { "failed" };
                l.message = if ok { String::new() } else { last };
            }
        }
        // what it may now use; a server with no turn running starts again with the new sign-in
        if ok && active().as_ref().is_none_or(|a| a.is_empty()) {
            stop_server();
        }
        MODELS.lock().unwrap().0.clear();
        load_models();
    });
    // the page and code come within a second or two
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        let l = LOGIN.lock().unwrap();
        if !l.url.is_empty() || l.state != "waiting" {
            break;
        }
    }
    login_state()
}

/// Open the sign-in page in the browser (only OpenCode's own page).
pub fn login_open() -> bool {
    let url = LOGIN.lock().unwrap().url.clone();
    url.starts_with("https://opencode.ai/") && open::that(&url).is_ok()
}

/// End a sign-in that is still waiting (Cancel, or the app quitting).
pub fn login_cancel() {
    let pid = {
        let mut l = LOGIN.lock().unwrap();
        if l.state == "waiting" {
            l.state = "";
        }
        l.pid.take()
    };
    if let Some(pid) = pid {
        paths::end_trees(&[pid]);
    }
}

/// Where the sign-in stands, for the page.
pub fn login_state() -> Value {
    let l = LOGIN.lock().unwrap();
    json!({"ok": true, "state": l.state, "url": l.url, "code": l.code, "message": l.message})
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real events from opencode v2.3.2's `GET /api/event` (the long `location` and `durable` fields left out, long
    // texts shortened). The tool server there was named "rbxkit"; a chat's own name goes in front in `named`.
    const SHELL_STARTED: &str = r#"{"id":"evt_203285ba1001njw86ONqWOXCZX","created":1795349306273,"type":"session.tool.input.started","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_203285149001muFwqKOC51YtKy","id":"8da3a68e-8cae-4848-94be-bcf90a2ed89f","name":"shell"}}"#;
    const SHELL_CALLED: &str = r#"{"id":"evt_203285bcb001NjIn30phsqH1QI","created":1795349306315,"type":"session.tool.called","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_203285149001muFwqKOC51YtKy","id":"8da3a68e-8cae-4848-94be-bcf90a2ed89f","input":{"command":"ls"},"executed":false}}"#;
    const VAULT_STARTED: &str = r#"{"id":"evt_20329d351001BF2CGTHlYbXf3w","created":1795349402449,"type":"session.tool.input.started","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_20329c2c8001iLnMYPWkCYC4QC","id":"52cd6401-d467-4249-8b26-ea27fc882cdf","name":"rbxkit_rbx_vault"}}"#;
    const VAULT_CALLED: &str = r#"{"id":"evt_20329d38e001u5S8F2BqggUAfj","created":1795349402510,"type":"session.tool.called","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_20329c2c8001iLnMYPWkCYC4QC","id":"52cd6401-d467-4249-8b26-ea27fc882cdf","input":{"action":"list"},"executed":false}}"#;
    const VAULT_OK: &str = r#"{"id":"evt_20329d3b80011MIM9bF46Pa0PR","created":1795349402552,"type":"session.tool.success","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_20329c2c8001iLnMYPWkCYC4QC","id":"52cd6401-d467-4249-8b26-ea27fc882cdf","content":[{"type":"text","text":"Folders: 01 Studio and Workflow, 02 Luau Scripting, 03 Archi"}],"metadata":{"truncated":false},"executed":false}}"#;
    const TEXT: &str = r#"{"id":"evt_20329dffd002lV3XSKmjtSxJgG","created":1795349405693,"type":"session.text.ended","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_20329d3c3001QXZcYCLw7qM1vN","ordinal":0,"text":"- Code word: HERON\n- `rbxkit_rbx_vault` (action=list): folders include 01 Studio & Workflow through 62 Engine Reference Cards."}}"#;
    const STEP: &str = r#"{"id":"evt_203285c58001h9DbfOqagG1otr","created":1795349306456,"type":"session.step.ended","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","assistantMessageID":"msg_203285149001muFwqKOC51YtKy","finish":"tool-calls","rawFinish":"tool_calls","tokens":{"input":951,"output":16,"reasoning":43,"cache":{"read":85,"write":0}},"cost":0}}"#;
    const ASKED: &str = r#"{"id":"evt_20328626a002pPIBeAHmMjM9X3","created":1795349308010,"type":"permission.asked","data":{"id":"per_20328626a001LIUY0ho5Op1QZE","sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","action":"shell","resources":["whoami"],"save":["whoami *"],"source":{"type":"tool","messageID":"msg_203285c5e001DDy6KWh2W5Ttil","id":"78c3350a-0f5a-46b7-8b84-1b9f4a6da656"}}}"#;
    const SUCCEEDED: &str = r#"{"id":"evt_20329e0090018dJ2v3Q8yq2U1j","created":1795349405705,"type":"session.execution.succeeded","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ"}}"#;
    const FREE_LIMIT: &str = r#"{"id":"evt_2032a7f5f001RO99lbTIsu9Jgs","created":1795349446495,"type":"session.execution.failed","data":{"sessionID":"ses_dfcd7c812ffejiqeRr1NEGO8XJ","error":{"type":"provider.free-limit","message":"Free usage limit reached. Connect a provider or subscribe to OpenCode Go to keep going, or switch models.","status":429,"retryAfterMs":42554000}}}"#;
    const INTERRUPTED: &str = r#"{"id":"evt_2032d2889001YJ3hHybcKhRFK1","created":1795349620873,"type":"session.execution.interrupted","data":{"sessionID":"ses_dfcd2e7ceffeE17rYscsQ01pf6","reason":"user"}}"#;
    const STEP_STOPPED: &str = r#"{"id":"evt_2032d2886001QedovQ5YEaETAr","created":1795349620870,"type":"session.step.failed","data":{"sessionID":"ses_dfcd2e7ceffeE17rYscsQ01pf6","assistantMessageID":"msg_2032d186f001zTZyK7g1T2HQTm","error":{"type":"aborted","message":"Step interrupted"},"rawFinish":"tool_calls","tokens":{"input":120,"output":9,"reasoning":0,"cache":{"read":0,"write":0}},"cost":0}}"#;
    const SID: &str = "ses_dfcd7c812ffejiqeRr1NEGO8XJ";

    fn ours() -> Value {
        json!({"rbxkit": {"type": "stdio", "command": "py", "args": ["mcp.py"], "env": {"RBXKIT_AGENT_ID": "c1"}}, "Roblox_Studio": {"type": "stdio", "command": "studio", "args": ["--stdio"]}})
    }

    /// An event as it comes for chat "c1", whose tool servers carry its own prefix.
    fn named(line: &str) -> Value {
        serde_json::from_str(&line.replace("\"rbxkit_rbx_vault\"", &format!("\"{}rbxkit_rbx_vault\"", chat_prefix("c1")))).unwrap()
    }

    fn feed(t: &mut Turn, line: &str) -> Step {
        t.feed(SID, &named(line))
    }

    #[test]
    fn a_turn_reads_like_the_chat_shows_it() {
        let mut t = Turn::new("c1", &ours(), Some("opencode/big-pickle".into()));
        assert!(feed(&mut t, SHELL_STARTED).outs.is_empty(), "the name comes first, the call after");
        assert!(matches!(&feed(&mut t, SHELL_CALLED).outs[..], [Out::Tool(n)] if n == "run_command"));
        feed(&mut t, VAULT_STARTED);
        let vault = feed(&mut t, VAULT_CALLED).outs;
        assert!(matches!(&vault[..], [Out::ToolArgs(n, a)] if n == "mcp__rbxkit__rbx_vault" && a["action"] == "list"), "a server's tool comes with what it was asked");
        assert!(feed(&mut t, VAULT_OK).outs.is_empty(), "no pictures in a text result");
        assert!(matches!(&feed(&mut t, TEXT).outs[..], [Out::Text(s)] if s.starts_with("- Code word: HERON")));
        feed(&mut t, STEP);
        feed(&mut t, STEP);
        let end = feed(&mut t, SUCCEEDED);
        assert!(end.ended && end.outs.is_empty());
        let fin = t.finish();
        let Out::Usage(u) = &fin[0] else { panic!("usage first") };
        assert_eq!((u.input, u.output, u.cache_read, u.cache_write, u.cost), (2 * 951, 2 * (16 + 43), 2 * 85, 0, None), "steps add up, thinking counts as output, no cost when free");
        assert!(matches!(fin.last(), Some(Out::Finished)));
    }

    #[test]
    fn another_sessions_events_are_not_this_turns() {
        let mut t = Turn::new("c1", &ours(), None);
        assert!(!t.feed("ses_other", &named(SUCCEEDED)).ended);
        assert!(t.feed("ses_other", &named(TEXT)).outs.is_empty());
        assert!(t.feed("ses_other", &named(ASKED)).ask.is_none());
    }

    #[test]
    fn another_chats_tools_are_not_named_as_ours() {
        let mut t = Turn::new("c2", &ours(), None);
        t.feed(SID, &named(VAULT_STARTED));
        let out = t.feed(SID, &named(VAULT_CALLED)).outs;
        assert!(matches!(&out[..], [Out::Tool(n)] if n.starts_with("fr") && !n.starts_with("mcp__")));
    }

    #[test]
    fn pictures_from_our_tools_are_shown() {
        let mut t = Turn::new("c1", &ours(), None);
        feed(&mut t, VAULT_STARTED);
        feed(&mut t, VAULT_CALLED);
        let with = VAULT_OK.replace(r#"{"type":"text","text":"Folders: 01 Studio and Workflow, 02 Luau Scripting, 03 Archi"}"#, r#"{"type":"text","text":"made"},{"type":"file","uri":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAYAAAAGWCAIAAA","mime":"image/png"},{"type":"image","data":"iVBORw0KGgoB","mimeType":"image/png"}"#);
        let out = feed(&mut t, &with).outs;
        let Some(Out::Encoded(p)) = out.first() else { panic!("pictures expected") };
        assert_eq!(p.len(), 2);
        assert!(p[0].starts_with("iVBORw0KGgo") && !p[0].contains("base64"));
        // a shell command's result is never shown as pictures
        let mut t = Turn::new("c1", &ours(), None);
        feed(&mut t, SHELL_STARTED);
        feed(&mut t, SHELL_CALLED);
        let shell = with.replace("52cd6401-d467-4249-8b26-ea27fc882cdf", "8da3a68e-8cae-4848-94be-bcf90a2ed89f");
        assert!(feed(&mut t, &shell).outs.is_empty());
    }

    #[test]
    fn a_command_that_asks_is_refused_with_a_card_unless_allowed_meanwhile() {
        let mut t = Turn::new("c1", &ours(), None);
        let ask = feed(&mut t, ASKED).ask.expect("asked");
        assert_eq!((ask.id.as_str(), ask.action.as_str(), ask.resources.as_slice()), ("per_20328626a001LIUY0ho5Op1QZE", "shell", &["whoami".to_string()][..]));
        let d = decide(&ask.action, &ask.resources, &["git".into()]);
        assert!(!d.once && d.message.contains("whoami") && d.message.contains("Do not try another way"));
        assert_eq!(d.blocked, vec![json!({"name": "whoami", "command": "whoami"})]);
        // allowed from the card while the turn runs: answered once
        let d = decide(&ask.action, &ask.resources, &["git".into(), "whoami".into()]);
        assert!(d.once && d.blocked.is_empty());
        // a compound command: every program must be allowed; each refused one is offered once
        let d = decide("shell", &["git status".into(), "rm -rf x".into(), "rm y".into()], &["git".into()]);
        assert!(!d.once);
        assert_eq!(d.blocked.len(), 1);
        assert_eq!(d.blocked[0]["name"], "rm");
        // anything else that asks is refused, with no card
        let d = decide("external_directory", &["/Users/x/elsewhere/*".into()], &["git".into()]);
        assert!(!d.once && d.blocked.is_empty() && d.message.contains("outside"));
        assert!(!decide("edit", &["a.txt".into()], &[]).once);
    }

    #[test]
    fn errors_count_as_failed_turns_with_a_way_on_and_a_stop_is_no_answer() {
        let mut t = Turn::new("c1", &ours(), Some("opencode/big-pickle".into()));
        let s = feed(&mut t, FREE_LIMIT);
        assert!(s.ended);
        assert!(s.outs.iter().any(|o| matches!(o, Out::Failed(m) if m.contains("Free usage limit") && m.contains("another model"))));
        assert!(matches!(t.finish().last(), Some(Out::Finished)), "OpenCode answered, with an error");
        let mut t = Turn::new("c1", &ours(), None);
        t.feed("ses_dfcd2e7ceffeE17rYscsQ01pf6", &named(STEP_STOPPED));
        let stopped = t.feed("ses_dfcd2e7ceffeE17rYscsQ01pf6", &named(INTERRUPTED));
        assert!(stopped.ended && stopped.outs.is_empty());
        let fin = t.finish();
        assert!(!fin.iter().any(|o| matches!(o, Out::Finished)), "a stopped turn is not an answer");
        assert!(matches!(&fin[0], Out::Usage(u) if u.input == 120 && u.output == 9), "what the stopped step used still counts");
        let no_route = json!({"type": "provider.no-route", "message": "Model unavailable: opencode-go/glm-5.3"});
        assert!(failure_text(&no_route, Some("opencode-go/glm-5.3")).contains("Sign in under Setup"));
    }

    #[test]
    fn costs_are_kept_when_opencode_reports_one() {
        let mut t = Turn::new("c1", &ours(), None);
        feed(&mut t, &STEP.replace(r#""cost":0"#, r#""cost":0.0125"#));
        feed(&mut t, &STEP.replace(r#""cost":0"#, r#""cost":0.0025"#));
        let Out::Usage(u) = &t.finish()[0] else { panic!() };
        assert!((u.cost.unwrap() - 0.015).abs() < 1e-9);
    }

    #[test]
    fn housekeeping_tools_are_hidden_and_built_in_ones_named_like_claudes() {
        let t = Turn::new("c1", &json!({"rbxkit": {}, "rbxkit_extra": {}}), None);
        let p = chat_prefix("c1");
        assert_eq!(t.tool_name(&format!("{p}rbxkit_extra_go")).as_deref(), Some("mcp__rbxkit_extra__go"), "the longest server name wins");
        assert_eq!(t.tool_name(&format!("{p}rbxkit_rbx_team")).as_deref(), Some("mcp__rbxkit__rbx_team"));
        for (tool, name) in [("edit", "Edit"), ("read", "Read"), ("glob", "Glob"), ("grep", "Grep"), ("list", "list_dir"), ("webfetch", "WebFetch"), ("websearch", "WebSearch")] {
            assert_eq!(t.tool_name(tool).as_deref(), Some(name));
        }
        assert!(t.tool_name("todowrite").is_none());
    }

    #[test]
    fn each_chat_has_short_tool_server_names_of_its_own() {
        let (a, b) = (chat_prefix("c1"), chat_prefix("c2"));
        assert_ne!(a, b);
        assert_eq!(a.len(), 11);
        assert!(a.starts_with("fr") && a.ends_with('-') && a[2..10].chars().all(|c| c.is_ascii_hexdigit()));
        // the longest tool name stays under the 64 characters model providers take
        assert!(server_name("c1", "Roblox_Studio").len() + 1 + "insert_from_creator_store".len() < 64);
        let cfg = servers_config("c1", &crate::agents::test_servers(true), &[("XDG_DATA_HOME".into(), "/u/.local/share".into())]);
        assert_eq!(cfg.len(), 1, "no Studio server under FRIDAE_NO_STUDIO");
        let (name, rbx) = &cfg[0];
        assert_eq!(name, &format!("{a}rbxkit"));
        assert_eq!(rbx["environment"]["FRIDAE_NO_STUDIO"], "1");
        assert_eq!(rbx["environment"]["XDG_DATA_HOME"], "/u/.local/share", "the tool server sees the user's folders, not OpenCode's");
        assert_eq!((rbx["type"].as_str(), rbx["codemode"].as_bool()), (Some("local"), Some(false)));
        assert_eq!(rbx["timeout"]["execution"], 900_000);
    }

    #[test]
    fn the_model_list_reads_from_its_lines_subscription_first() {
        let text = "abliteration-ai/abliterated-model\nopencode/big-pickle\nopencode/beam-free\nopencode-go/glm-5.3\nopencode-go/beam-free\nnot a model line\n";
        let list = parse_models(text);
        let ids: Vec<&str> = list.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["opencode-go/glm-5.3", "opencode-go/beam-free", "opencode/big-pickle", "opencode/beam-free", "abliteration-ai/abliterated-model"]);
        assert_eq!(list[0].1, "OpenCode Go · glm-5.3");
        assert_eq!(list[3].1, "OpenCode Zen · beam-free");
        assert_eq!(list[4].1, "abliteration-ai · abliterated-model");
        // the default: the user's own OpenCode default when offered, else the first OpenCode Go model, else the first
        assert_eq!(default_of(&list, Some("opencode/big-pickle")).as_deref(), Some("opencode/big-pickle"));
        assert_eq!(default_of(&list, Some("anthropic/gone")).as_deref(), Some("opencode-go/glm-5.3"));
        assert_eq!(default_of(&list[2..], None).as_deref(), Some("opencode/big-pickle"));
        assert_eq!(default_of(&[], None), None, "before the list arrives OpenCode picks");
        assert_eq!(pick_from(&list, Some("gone/model"), None).as_deref(), Some("opencode-go/glm-5.3"));
        assert_eq!(pick_from(&[], Some("opencode-go/glm-5.3"), None).as_deref(), Some("opencode-go/glm-5.3"));
        assert_eq!(model_setting("{\n // mine\n \"model\": \"opencode-go/kimi-k3#high\",\n}").as_deref(), Some("opencode-go/kimi-k3"));
        assert_eq!(model_setting("{\"theme\": \"x\"}"), None);
        assert_eq!(model_ref("opencode-go/glm-5.3"), Some(json!({"providerID": "opencode-go", "id": "glm-5.3"})));
        assert_eq!(model_ref("nonsense"), None);
    }

    #[test]
    fn the_sign_in_page_and_code_are_found_in_its_output() {
        // as `opencode auth login opencode --method device` prints them (a made-up code)
        let (url, code) = login_line("\u{25cf}  Enter code: ABCD-1234");
        assert_eq!((url, code.as_deref()), (None, Some("ABCD-1234")));
        let (url, _) = login_line("\u{25cf}  https://opencode.ai/console/device?user_code=ABCD-1234&client_id=opencode-cli");
        assert_eq!(url.as_deref(), Some("https://opencode.ai/console/device?user_code=ABCD-1234&client_id=opencode-cli"));
        assert_eq!(login_line("\u{1b}[?25l\u{25cf}  Waiting for authorization...\u{1b}[?25h"), (None, None));
        assert_eq!(parse_auth_list("abliteration.ai   abliteration.ai             stored\nOpenCode Console  Personal                    stored\n"), ["abliteration.ai", "OpenCode Console"]);
    }

    #[test]
    fn ask_me_allows_ours_and_the_chats_folders_and_denies_the_rest() {
        let f = Folders { location: Path::new("/games/Odd One In é"), project: Some(Path::new("/games/Odd One In é")), kit: Path::new("/kit"), work: Path::new("/work") };
        let r = permissions(false, "c1", &["git".into()], &f, &["/h/.*".into(), "/kit/config.json".into()]);
        let has = |a: &str, res: &str, eff: &str| r.iter().any(|x| x == &rule(a, res, eff));
        let last = |a: &str, res: &str| r.iter().rposition(|x| x["action"] == a && x["resource"] == res);
        let mine = format!("{}*", chat_prefix("c1"));
        assert_eq!(r[0], rule("*", "*", "deny"), "everything else is refused");
        assert!(has("shell", "git *", "allow") && has("shell", "*", "ask") && !has("shell", "*", "allow"));
        assert!(last("shell", "git *") > last("shell", "*"), "an allowed program runs without asking");
        assert!(has(&mine, "*", "allow") && last(&mine, "*") > last("fr????????-*", "*"), "this chat's tools, not another's");
        assert!(has("edit", "/games/Odd One In é/*", "allow") && has("edit", "/kit/*", "allow") && has("edit", "/work/*", "allow"));
        assert!(has("external_directory", "/work/*", "allow") && !has("external_directory", "*", "allow"), "only the chat's folders outside its own");
        assert!(last("edit", "../*") > last("edit", "*"), "a path going up out of the game is refused");
        assert!(last("edit", "/kit/config.json") > last("edit", "/kit/*"), "the app's settings stay protected inside the kit");
        assert!(has("edit", "/h/.*/*", "deny") && has("edit", "*/.opencode/*", "deny"));
        for a in ["execute", "subagent", "skill", "question"] {
            assert!(has(a, "*", "deny"), "{a}");
        }
        assert!(has("webfetch", "*", "allow") && has("websearch", "*", "allow"));
        let r = permissions(false, "c2", &[], &f, &[]);
        assert!(!r.iter().any(|x| x["action"] == "shell" && x["effect"] == "allow"), "no program allowed: every command asks");
    }

    #[test]
    fn full_auto_allows_everything_but_keeps_the_denies() {
        let f = Folders { location: Path::new("/g"), project: Some(Path::new("/g")), kit: Path::new("/kit"), work: Path::new("/work") };
        let r = permissions(true, "c1", &[], &f, &["/store".into()]);
        assert_eq!(r[0], rule("*", "*", "deny"), "a tool not named here (OpenCode's browser, say) is not offered");
        let at = |x: &Value| r.iter().position(|y| y == x).unwrap();
        for a in ["shell", "edit", "external_directory", "webfetch", "websearch", "read"] {
            assert!(at(&rule(a, "*", "allow")) > 0, "{a}");
        }
        assert!(at(&rule("edit", "/store/*", "deny")) > at(&rule("edit", "*", "allow")) && at(&rule("execute", "*", "deny")) > 0, "denies come after the allows, so they win");
        assert!(at(&rule("fr????????-*", "*", "deny")) > 0, "another chat's tools stay refused");
        assert!(!r.iter().any(|x| x["effect"] == "ask"), "nothing asks: nobody is there to answer");
    }

    #[test]
    fn the_home_folders_hidden_files_are_protected_by_pattern() {
        let p = protected(Path::new("/Users/someone"));
        assert!(p.contains(&"/Users/someone/.*".to_string()) && p.contains(&"/Users/someone/Library".to_string()));
        assert!(p.iter().any(|x| x.ends_with("config.json")) && p.iter().any(|x| x.ends_with("opencode-store")));
        assert!(protected(Path::new("")).len() <= 4, "no home: no home patterns");
    }

    #[test]
    fn the_settings_hide_other_tool_servers_and_share_nothing() {
        let s = settings();
        assert_eq!((s["share"].as_str(), s["snapshots"].as_bool(), s["autoupdate"].as_bool()), (Some("disabled"), Some(false), Some(false)));
        let p = s["policies"].as_array().unwrap();
        assert_eq!(p[0], rule("mcp.use", "*", "deny"), "a server not a chat's own is hidden, a Studio one set up elsewhere included");
        assert!(p.contains(&rule("mcp.use", "fr*", "allow")) && p.contains(&rule("skill.use", "*", "deny")));
        let text = instructions(Some("/games/g"), &["git".into()], false);
        assert!(text.contains("/games/g") && text.contains("git") && text.contains("card"));
        assert!(instructions(None, &[], true).contains("full auto"));
    }

    #[test]
    fn http_pieces_read_what_the_server_sends() {
        assert_eq!(listening_port("server listening on http://127.0.0.1:58866"), Some(58866));
        assert_eq!(listening_port("something else"), None);
        let pw = password();
        assert_eq!(pw.len(), 48);
        assert_ne!(pw, password());
        assert_eq!(at(Path::new("/games/Odd One/é")), "?location%5Bdirectory%5D=%2Fgames%2FOdd%20One%2F%C3%A9");
        // a chunked body, as the event stream comes
        let stream = "data: {\"type\":\"server.connected\",\"data\":{}}\n\n: heartbeat\n\n";
        let mut raw = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_string();
        for piece in [&stream[..13], &stream[13..30], &stream[30..]] {
            raw += &format!("{:x}\r\n{piece}\r\n", piece.len());
        }
        raw += "0\r\n\r\n";
        let raw: &'static [u8] = raw.into_bytes().leak();
        let mut r = BufReader::new(raw);
        let (status, headers) = read_head(&mut r).unwrap();
        assert_eq!(status, 200);
        assert!(chunked(&headers));
        let body: Box<dyn Read + Send> = Box::new(Chunked { inner: r, left: 0, done: false });
        let mut ev = Events { r: BufReader::new(body) };
        assert_eq!(ev.next().unwrap().unwrap()["type"], "server.connected");
        assert!(ev.next().unwrap().is_none(), "a heartbeat is no event; the stream then ends");
    }

    #[test]
    fn why_a_turn_could_not_run_is_said_plainly() {
        assert!(explain("Error: Unauthorized", false).unwrap().contains("Sign in to OpenCode for Fridae"));
        assert!(explain("Error: Unauthorized", true).is_none());
        assert_eq!(explain("OpenCode is not installed.", false).as_deref(), Some("OpenCode is not installed."));
        assert!(explain("", false).is_none());
    }
}
