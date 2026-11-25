//! Connecting GitHub (Setup, Connections) and putting a project on it (Project details).
//!
//! Signing in is GitHub's device sign-in for an OAuth App the maintainer registered: Fridae asks GitHub for a
//! one-time code (`POST /login/device/code`), the user enters it on GitHub's page, and Fridae asks every `interval`
//! seconds whether it was approved (`POST /login/oauth/access_token`), five seconds slower each time GitHub says
//! `slow_down`, until it gets the token, or GitHub says `expired_token` or `access_denied`. An answer that is not
//! GitHub's (a 5xx, a page that is not JSON) is asked again until the code runs out. No client secret exists.
//! Scopes: `repo` (make and push private repositories) and `read:user` (the username and picture shown).
//!
//! The token stays with the app:
//! - it is kept in the app's own Application Support folder (`folder()`), mode 0600, among the places no agent's
//!   file tools may write (`paths::app_protected`) or read (`paths::app_secrets`);
//! - it is never put in an agent's environment, a tool server's config, argv, a log or the page;
//! - a push runs git with the app itself as its `GIT_ASKPASS` helper and a one-time code in its environment
//!   (`FRIDAE_GIT_ASKPASS`): the helper trades that code, once, with the app over 127.0.0.1 for the token, and only
//!   while that push runs (`Trade`). Git runs with `-c credential.helper=` and `GIT_TERMINAL_PROMPT=0`, so the
//!   user's own credential helpers are neither asked nor written, and nothing waits at a prompt. The user's
//!   `~/.gitconfig`, keychain and `gh` sign-in are never changed.
//!
//! Creating a repository or pushing happens only from the two buttons in Project details (`put`, `push`): agents
//! have no tool for either. A push goes only to the repository Put on GitHub made for that project (`targets.json`
//! beside the token), owned by the account that is connected, and not when the project's own git settings change
//! where or how it is sent (`config_problem`). Both run in the background; the page follows `last_push`.

use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};

use crate::paths;

/// The public Client ID of Fridae Studios' OAuth App on GitHub (device flow enabled). `FRIDAE_GITHUB_CLIENT_ID`
/// replaces it when set (empty: sign-in is not set up, as in a copy built without one).
pub const GITHUB_CLIENT_ID: &str = "Ov23lioOCcFrnPqpNYTr";
const SCOPES: &str = "repo read:user";
pub const NOT_SET_UP: &str = "GitHub sign-in isn't set up in this copy of Fridae yet.";
/// Set by the app in git's environment for a push: "<port>:<one-time code>" (`Trade`). Its presence starts the app
/// in askpass mode.
const ASKPASS_ENV: &str = "FRIDAE_GIT_ASKPASS";
/// The user name GitHub takes with a token as the password.
const USERNAME: &str = "x-access-token";
/// A push or a repository's first push that takes longer than this is ended.
const PUSH_WAIT: Duration = Duration::from_secs(300);

/// One of GitHub's seconds (tests run the same waits a hundred times faster).
#[cfg(not(test))]
const UNIT: Duration = Duration::from_secs(1);
#[cfg(test)]
const UNIT: Duration = Duration::from_millis(10);

const EXPIRED: &str = "The code ran out of time before it was entered on GitHub. Press Connect to get a new one.";
const DENIED: &str = "Connecting was cancelled on GitHub's page. Press Connect to try again.";
pub(crate) const OFFLINE: &str = "Could not reach GitHub. Check the internet connection and try again.";
pub(crate) const NOT_CONNECTED: &str = "Connect GitHub first: Setup, Connections.";
const REFUSED: &str = "GitHub no longer accepts Fridae's sign-in. Disconnect GitHub in Setup, Connections, and connect again.";
const BUSY: &str = "Fridae is already sending this project to GitHub.";
const NOT_PUT: &str = "This project is not on GitHub from Fridae yet. Press Put on GitHub first.";

/// For tests: GitHub's addresses (a stub on 127.0.0.1), the Client ID, and which folders count as projects.
#[cfg(test)]
#[derive(Default)]
pub struct StandIn {
    pub web: String,
    pub api: String,
    /// raw.githubusercontent.com
    pub raw: String,
    /// the vault's repository, as "owner/name"
    pub vault_repo: Option<String>,
    pub client_id: Option<String>,
    pub projects: Vec<String>,
}
#[cfg(test)]
pub static STAND_IN: Mutex<Option<StandIn>> = Mutex::new(None);
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

fn client_id() -> String {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().and_then(|s| s.client_id.clone()).unwrap_or_else(|| GITHUB_CLIENT_ID.to_string());
    #[allow(unreachable_code)]
    std::env::var("FRIDAE_GITHUB_CLIENT_ID").unwrap_or_else(|_| GITHUB_CLIENT_ID.to_string()).trim().to_string()
}

pub fn configured() -> bool {
    !client_id().is_empty()
}

/// github.com, where the sign-in pages and the repositories are.
pub(crate) fn web_base() -> String {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().map(|s| s.web.clone()).unwrap_or_default();
    #[allow(unreachable_code)]
    "https://github.com".to_string()
}

pub(crate) fn api_base() -> String {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().map(|s| s.api.clone()).unwrap_or_default();
    #[allow(unreachable_code)]
    "https://api.github.com".to_string()
}

/// raw.githubusercontent.com, where a repository's files are read without a token.
pub(crate) fn raw_base() -> String {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().map(|s| s.raw.clone()).unwrap_or_default();
    #[allow(unreachable_code)]
    "https://raw.githubusercontent.com".to_string()
}

/// Whether a folder is one of the user's projects (the only folders `put` and `push` touch).
fn known_project(path: &str) -> bool {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().is_some_and(|s| s.projects.iter().any(|p| p == path)) && Path::new(path).is_dir();
    #[allow(unreachable_code)]
    {
        crate::projects::known(path) && Path::new(path).is_dir()
    }
}

// ----------------------------------------------------------------- the token

/// Where the sign-in is kept: beside the second accounts' folders (`accounts::base`), in the app's own data folder.
pub fn folder() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("github");
    #[allow(unreachable_code)]
    crate::opencode::own_store().parent().map(Path::to_path_buf).unwrap_or_default().join("github")
}

fn token_file() -> PathBuf {
    folder().join("token")
}

fn user_file() -> PathBuf {
    folder().join("user.json")
}

/// The repository Put on GitHub made for each project: `{"<project path>": {"repo": "owner/name", "remote": "origin"}}`.
fn targets_file() -> PathBuf {
    folder().join("targets.json")
}

/// Write a file only its owner can read (0600), in a folder only its owner can open (0700), replaced whole.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut open = fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        open.mode(0o600);
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    {
        let mut f = open.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

pub(crate) fn save_token(token: &str) -> std::io::Result<()> {
    write_private(&token_file(), token)
}

pub(crate) fn read_token() -> Option<String> {
    fs::read_to_string(token_file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

pub fn connected() -> bool {
    read_token().is_some()
}

// ------------------------------------------------------- the one-time askpass

/// A push's one chance to hand git the token: a listener on 127.0.0.1 that gives the token once, to whoever sends
/// its one-time code first, and closes when the push ends (dropped) or after `PUSH_WAIT`.
struct Trade {
    /// "<port>:<code>", for git's environment
    env: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn open_trade(token: String) -> Option<Trade> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    listener.set_nonblocking(true).ok()?;
    let port = listener.local_addr().ok()?.port();
    let code = paths::random_hex(2);
    let stop = Arc::new(AtomicBool::new(false));
    let (want, halt) = (code.clone(), stop.clone());
    let thread = std::thread::spawn(move || {
        let until = Instant::now() + PUSH_WAIT;
        while !halt.load(Ordering::SeqCst) && Instant::now() < until {
            let Ok((stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let mut line = String::new();
            let _ = BufReader::new((&stream).take(200)).read_line(&mut line);
            if same(line.trim(), &want) {
                let _ = (&stream).write_all(format!("{token}\n").as_bytes());
                return; // once
            }
        }
    });
    Some(Trade { env: format!("{port}:{code}"), stop, thread: Some(thread) })
}

impl Drop for Trade {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The askpass helper's answer to one of git's prompts: the user name GitHub takes with a token, or the token, traded
/// once with the running app for the one-time code in `spec` ("<port>:<code>"). Nothing else is ever answered.
pub fn askpass_answer(prompt: &str, spec: &str) -> Option<String> {
    let low = prompt.trim_start().to_lowercase();
    if low.starts_with("username") {
        return Some(USERNAME.to_string());
    }
    if !low.starts_with("password") {
        return None;
    }
    let (port, code) = spec.split_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p > 0)?;
    if code.len() != 32 || !code.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(3)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    s.write_all(format!("{code}\n").as_bytes()).ok()?;
    let mut answer = String::new();
    let _ = (&mut s).take(4096).read_to_string(&mut answer);
    let answer = answer.trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

/// The app started by git as its askpass helper (`FRIDAE_GIT_ASKPASS` set): answer the prompt on stdout and say how
/// to exit. None: an ordinary start.
pub fn askpass_main() -> Option<i32> {
    let spec = std::env::var_os(ASKPASS_ENV)?.to_string_lossy().to_string();
    let prompt = std::env::args().nth(1).unwrap_or_default();
    match askpass_answer(&prompt, &spec) {
        Some(answer) => {
            println!("{answer}");
            Some(0)
        }
        None => Some(1),
    }
}

/// The program git runs as its askpass helper: this app. (Tests run the app built beside the test program: cargo
/// builds it for `tests/askpass.rs`.)
fn askpass_program() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    #[cfg(test)]
    return me.ancestors().skip(1).map(|d| d.join(if cfg!(windows) { "fridae.exe" } else { "fridae" })).find(|p| p.is_file());
    #[allow(unreachable_code)]
    Some(me)
}

// ---------------------------------------------------------------- the sign-in

struct Pending {
    user_code: String,
    verification_uri: String,
    /// when the code runs out, in seconds since 1970 (for the page)
    expires_at: u64,
}

/// A Put on GitHub or a push from the page, as the page follows it.
struct Job {
    /// "running", "ok" or "error"
    state: &'static str,
    at: u64,
    text: String,
}

#[derive(Default)]
struct State {
    /// counts sign-ins started, cancelled and forgotten: a waiting sign-in's thread, or a profile being fetched,
    /// counts only while it is still the latest
    flow: u64,
    pending: Option<Pending>,
    error: String,
    /// Disconnect was pressed (the page says how to remove the app's access on GitHub too)
    forgot: bool,
    /// the profile is being asked for (a sign-in kept from before, with no saved profile)
    asking_user: bool,
    /// each project's last Put on GitHub or push from here
    jobs: HashMap<String, Job>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub(crate) fn http() -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .proxy(None)
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .build()
        .into()
}

pub(crate) fn answer(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, Value), String> {
    let mut resp = resp.map_err(|_| OFFLINE.to_string())?;
    let code = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    Ok((code, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, Value), String> {
    answer(http().post(url).header("Accept", "application/json").header("User-Agent", "fridae-studios").send_form(form.iter().copied()))
}

/// A call to GitHub's API with the token.
pub(crate) fn api(method: &str, path: &str, token: &str, body: Option<Value>) -> Result<(u16, Value), String> {
    let url = format!("{}{path}", api_base());
    let auth = format!("Bearer {token}");
    let agent = http();
    let resp = if method == "POST" || method == "PATCH" {
        let req = if method == "POST" { agent.post(&url) } else { agent.patch(&url) };
        req
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "fridae-studios")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &auth)
            .send_json(body.unwrap_or(json!({})))
    } else {
        agent.get(&url).header("Accept", "application/vnd.github+json").header("User-Agent", "fridae-studios").header("X-GitHub-Api-Version", "2022-11-28").header("Authorization", &auth).call()
    };
    answer(resp)
}

/// GitHub's own words, plain and short, when it gives some.
pub(crate) fn said(v: &Value) -> String {
    let text = v["error_description"].as_str().or(v["message"].as_str()).or(v["error"].as_str()).unwrap_or("");
    text.chars().filter(|c| !c.is_control()).take(200).collect::<String>().trim().to_string()
}

/// What one answer to "was it approved?" means.
#[derive(Debug, PartialEq)]
enum Step {
    /// ask again after this many seconds
    Wait(u64),
    Done(String),
    Fail(String),
}

/// `code`: the answer's HTTP status. A server error or an answer that is not GitHub's JSON is asked again.
fn step(code: u16, v: &Value, interval: u64) -> Step {
    if code >= 500 || !v.is_object() {
        return Step::Wait(interval);
    }
    if let Some(token) = v["access_token"].as_str().map(str::trim).filter(|t| !t.is_empty()) {
        return Step::Done(token.to_string());
    }
    match v["error"].as_str() {
        Some("authorization_pending") => Step::Wait(interval),
        // GitHub adds five seconds to the interval each time it says this, and may say what the interval now is
        Some("slow_down") => Step::Wait(v["interval"].as_u64().unwrap_or(0).max(interval + 5)),
        Some("expired_token") => Step::Fail(EXPIRED.to_string()),
        Some("access_denied") => Step::Fail(DENIED.to_string()),
        Some(_) => Step::Fail(format!("GitHub did not connect: {}. Press Connect to try again.", said(v).trim_end_matches('.'))),
        None => Step::Fail("GitHub gave an answer Fridae could not read. Press Connect to try again.".to_string()),
    }
}

/// Ask GitHub for the signed-in user's name and picture, and keep them beside the token (they are not secret), only
/// while sign-in `flow` is still the latest and that token is still the one kept (never after Disconnect).
fn fetch_user(token: &str, flow: u64) -> bool {
    let Ok((200, v)) = api("GET", "/user", token, None) else { return false };
    let Some(login) = v["login"].as_str().filter(|l| !l.is_empty() && l.len() <= 39 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) else { return false };
    let avatar = v["avatar_url"].as_str().filter(|a| a.starts_with("https://")).unwrap_or("");
    let text = json!({"login": login, "avatar_url": avatar}).to_string();
    with(|s| s.flow == flow && read_token().as_deref() == Some(token) && write_private(&user_file(), &text).is_ok())
}

pub(crate) fn user() -> Value {
    fs::read_to_string(user_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// Connect: ask GitHub for a code, show it, open GitHub's page, and wait in the background for the approval. A code
/// already waiting is kept (pressed twice: no second page).
pub fn connect() -> Value {
    if !configured() {
        with(|s| s.error = NOT_SET_UP.to_string());
        return reply(false);
    }
    if with(|s| s.pending.as_ref().is_some_and(|p| p.expires_at > now())) {
        return reply(true);
    }
    // this sign-in's number, taken before GitHub is asked: a Cancel meanwhile makes it stale
    let flow = with(|s| {
        s.flow += 1;
        s.pending = None;
        s.error.clear();
        s.forgot = false;
        s.flow
    });
    let fail = |e: String| {
        with(|s| {
            if s.flow == flow {
                s.error = e;
            }
        });
        reply(false)
    };
    let id = client_id();
    let (code, v) = match post_form(&format!("{}/login/device/code", web_base()), &[("client_id", &id), ("scope", SCOPES)]) {
        Ok(got) => got,
        Err(e) => return fail(e),
    };
    let (Some(device_code), Some(user_code)) = (v["device_code"].as_str().filter(|_| code == 200), v["user_code"].as_str()) else {
        let why = said(&v);
        return fail(if why.is_empty() { "GitHub did not give a code. Try again in a moment.".to_string() } else { format!("GitHub did not give a code: {why}") });
    };
    let page = v["verification_uri"].as_str().filter(|u| u.starts_with(&format!("{}/", web_base()))).map(str::to_string).unwrap_or_else(|| format!("{}/login/device", web_base()));
    let expires_in = v["expires_in"].as_u64().unwrap_or(900).min(3600);
    let interval = v["interval"].as_u64().unwrap_or(5).clamp(1, 60);
    let current = with(|s| {
        let current = s.flow == flow;
        if current {
            s.pending = Some(Pending { user_code: user_code.chars().take(20).collect(), verification_uri: page.clone(), expires_at: now() + expires_in });
        }
        current
    });
    if !current {
        return reply(false); // cancelled while GitHub was asked
    }
    let device_code = device_code.to_string();
    std::thread::spawn(move || wait_for_approval(flow, &id, &device_code, interval, Instant::now() + UNIT * expires_in as u32));
    open_page(&page);
    reply(true)
}

fn open_page(url: &str) {
    if cfg!(test) || !url.starts_with(&format!("{}/", web_base())) {
        return;
    }
    let _ = paths::open_that(url);
}

/// Ask GitHub every `interval` seconds whether the code was approved, until it is, fails, runs out, or this sign-in
/// is no longer the latest (cancelled, or started again).
fn wait_for_approval(flow: u64, id: &str, device_code: &str, mut interval: u64, until: Instant) {
    let current = || with(|s| s.flow == flow);
    let end = |error: &str| {
        with(|s| {
            if s.flow == flow {
                s.pending = None;
                s.error = error.to_string();
            }
        })
    };
    loop {
        std::thread::sleep(UNIT * interval as u32);
        if !current() {
            return;
        }
        if Instant::now() >= until {
            return end(EXPIRED);
        }
        let form = [("client_id", id), ("device_code", device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")];
        // offline for a moment: asked again at the next interval
        let Ok((code, v)) = post_form(&format!("{}/login/oauth/access_token", web_base()), &form) else { continue };
        match step(code, &v, interval) {
            Step::Wait(next) => interval = next.min(120),
            Step::Fail(why) => return end(&why),
            Step::Done(token) => {
                // saved only while this is still the sign-in the user wants (not cancelled a moment ago)
                let saved = with(|s| s.flow == flow && save_token(&token).is_ok());
                if !saved {
                    return end(if current() { "Fridae could not save the GitHub sign-in on this computer. Try again." } else { "" });
                }
                fetch_user(&token, flow);
                return end("");
            }
        }
    }
}

pub fn cancel() -> Value {
    with(|s| {
        s.flow += 1;
        s.pending = None;
        s.error.clear();
    });
    reply(true)
}

/// Delete Fridae's copy of the sign-in (the app's access on GitHub is revoked on GitHub's settings page, which the
/// page then points to). Under the one lock, so no sign-in or profile that was on its way writes anything after.
pub fn disconnect() -> Value {
    with(|s| {
        s.flow += 1;
        s.pending = None;
        s.error.clear();
        s.forgot = true;
        for f in [token_file(), token_file().with_extension("tmp"), user_file(), user_file().with_extension("tmp")] {
            let _ = fs::remove_file(f);
        }
    });
    reply(!token_file().exists())
}

/// Where the sign-in stands, for the page. Never the token.
pub fn status(project: Option<&str>) -> Value {
    let connected = connected();
    let who = user();
    if connected && who.is_null() && !with(|s| std::mem::replace(&mut s.asking_user, true)) {
        // kept from before, with no saved profile: asked for once, in the background
        let flow = with(|s| s.flow);
        std::thread::spawn(move || {
            if let Some(t) = read_token() {
                fetch_user(&t, flow);
            }
            with(|s| s.asking_user = false);
        });
    }
    let mut v = with(|s| {
        json!({
            "configured": configured(),
            "connected": connected,
            "login": if connected { who["login"].clone() } else { Value::Null },
            "avatar_url": if connected { who["avatar_url"].clone() } else { Value::Null },
            "pending": s.pending.as_ref().map(|p| json!({"user_code": p.user_code, "verification_uri": p.verification_uri, "expires_at": p.expires_at})),
            "error": s.error,
            "forgot": s.forgot && !connected,
            "revoke_url": format!("{}/settings/connections/applications/{}", web_base(), client_id()),
        })
    });
    if let Some(p) = project.filter(|p| known_project(p)) {
        v["project"] = project_info(p);
    }
    v
}

fn reply(ok: bool) -> Value {
    let mut v = status(None);
    v["ok"] = json!(ok);
    v
}

// ------------------------------------------------------------- git and GitHub

/// Run git in a project for a GitHub job: never with the project's hooks or fsmonitor (`projects::GIT_SAFE`), never
/// asking the user's credential helpers or a terminal; with `askpass` (a `Trade`'s "<port>:<code>"), the app answers
/// git's prompts. Ended after `PUSH_WAIT`.
fn git(dir: &Path, args: &[&str], askpass: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.args(crate::projects::GIT_SAFE).args(["-c", "credential.helper=", "-c", "core.askPass="]).args(args).current_dir(dir).stdin(Stdio::null());
    cmd.env("GIT_TERMINAL_PROMPT", "0").env_remove("SSH_ASKPASS").env_remove(ASKPASS_ENV).env_remove("GIT_ASKPASS");
    if let Some(spec) = askpass {
        let Some(helper) = askpass_program() else { return (false, "Fridae could not find its own program to hand git the sign-in.".to_string()) };
        cmd.env("GIT_ASKPASS", helper).env(ASKPASS_ENV, spec);
    }
    #[cfg(test)]
    tests::isolate(&mut cmd);
    match crate::signin::run_fed(cmd, PUSH_WAIT, None, "github", &|| false) {
        Some((out, err, ok)) => {
            let text = [out.trim(), err.trim()].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("\n");
            (ok, text)
        }
        None => (false, "Git did not finish in time.".to_string()),
    }
}

/// "owner/name" as GitHub names a repository, when `repo` is one.
fn repo_ok(repo: &str) -> bool {
    repo.split_once('/').is_some_and(|(owner, name)| !owner.is_empty() && owner.len() <= 39 && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && name_ok(name))
}

fn targets() -> Map<String, Value> {
    fs::read_to_string(targets_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// The repository Put on GitHub made for a project, and the remote it set: ("owner/name", "origin").
fn target_of(path: &str) -> Option<(String, String)> {
    let t = targets().get(path)?.clone();
    let (repo, remote) = (t["repo"].as_str()?.to_string(), t["remote"].as_str()?.to_string());
    (repo_ok(&repo) && matches!(remote.as_str(), "origin" | "github")).then_some((repo, remote))
}

fn pin(path: &str, repo: &str, remote: &str) -> bool {
    let mut all = targets();
    all.insert(path.to_string(), json!({"repo": repo, "remote": remote}));
    write_private(&targets_file(), &Value::Object(all).to_string()).is_ok()
}

fn project_info(path: &str) -> Value {
    let target = target_of(path);
    let last = with(|s| s.jobs.get(path).map(|j| json!({"state": j.state, "ok": j.state == "ok", "at": j.at, "text": j.text})));
    json!({
        "repo": target.as_ref().map(|t| t.0.clone()),
        "remote": target.as_ref().map(|t| t.1.clone()),
        "web_url": target.as_ref().map(|t| format!("{}/{}", web_base(), t.0)),
        "last_push": last,
    })
}

/// A repository name GitHub takes: letters, digits, `-`, `_` and `.`, at most 100.
pub fn name_ok(name: &str) -> bool {
    (1..=100).contains(&name.len()) && name != "." && name != ".." && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A branch git can be told to push without reading it as an option.
fn branch_ok(branch: &str) -> bool {
    !branch.is_empty() && !branch.starts_with('-') && !branch.contains([':', ' ', '\n', '\0'])
}

/// What a project needs before it can go to GitHub: a known project, a repository with a commit, a branch.
fn ready(path: &str) -> Result<(PathBuf, String), String> {
    if !known_project(path) {
        return Err("That folder is not one of your projects.".to_string());
    }
    let dir = PathBuf::from(path);
    if !crate::projects::git_installed() {
        return Err("Git is not installed. Get it from https://git-scm.com and reopen the app.".to_string());
    }
    if !dir.join(".git").exists() {
        return Err("Start a git repository for this project first.".to_string());
    }
    if !git(&dir, &["rev-parse", "--verify", "--quiet", "HEAD"], None).0 {
        return Err("Commit your files first, then send them to GitHub.".to_string());
    }
    let (ok, branch) = git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"], None);
    let branch = branch.trim().to_string();
    if !ok || branch.is_empty() {
        return Err("The project is not on a branch right now. Switch to a branch, then try again.".to_string());
    }
    if !branch_ok(&branch) {
        return Err(format!("Fridae won't push a branch named \"{branch}\". Switch to a branch whose name starts with a letter or a number."));
    }
    Ok((dir, branch))
}

/// Why the project's own git settings stop a push to `remote`, if they do: anything there that changes where or how
/// git sends (`url.*.insteadOf`/`pushInsteadOf`, any `http.*`, the remote's `pushurl`, `push`, `mirror`, `vcs`,
/// `receivepack`, `proxy`, or a second address). The user's own global settings are theirs and are not judged.
fn config_problem(dir: &Path, remote: &str) -> Option<String> {
    let (ok, out) = git(dir, &["config", "--includes", "--list", "--show-scope"], None);
    if !ok {
        return Some("Fridae could not read this project's git settings.".to_string());
    }
    let mut urls = 0;
    for line in out.lines() {
        let Some((scope, entry)) = line.split_once('\t') else { continue };
        if !matches!(scope, "local" | "worktree") {
            continue;
        }
        let key = entry.split_once('=').map(|x| x.0).unwrap_or(entry);
        let low = key.to_lowercase();
        let of_remote = key.strip_prefix(&format!("remote.{remote}.")).map(str::to_lowercase);
        if of_remote.as_deref() == Some("url") {
            urls += 1;
        }
        let bad = (low.starts_with("url.") && (low.ends_with(".insteadof") || low.ends_with(".pushinsteadof")))
            || low.starts_with("http.")
            || of_remote.as_deref().is_some_and(|k| ["pushurl", "push", "mirror", "vcs", "receivepack", "proxy"].contains(&k));
        if bad {
            return Some(format!("This project's own git settings change where or how it is sent ({key}), so Fridae won't push it to GitHub. Remove that setting from the project's .git/config, then try again."));
        }
    }
    (urls > 1).then(|| format!("The project's \"{remote}\" remote has more than one address, so Fridae won't push it to GitHub."))
}

/// Whether a push of this project goes where Put on GitHub said, to the account that is connected: the remote's
/// push address (as git resolves it) is that repository's, and nothing in the project's settings changes that.
fn check_target(dir: &Path, repo: &str, remote: &str) -> Result<(), String> {
    let login = user()["login"].as_str().map(str::to_string).ok_or("Fridae doesn't know yet which GitHub account is connected. Try again in a moment.")?;
    let owner = repo.split('/').next().unwrap_or("");
    if !owner.eq_ignore_ascii_case(&login) {
        return Err(format!("This project's GitHub copy ({repo}) belongs to {owner}, not to the connected account ({login}), so Fridae won't push it."));
    }
    if let Some(why) = config_problem(dir, remote) {
        return Err(why);
    }
    let (ok, url) = git(dir, &["remote", "get-url", "--push", "--all", remote], None);
    let want = format!("{}/{repo}", web_base());
    let lines: Vec<&str> = url.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if !ok || lines.len() != 1 || !(lines[0] == want || lines[0] == format!("{want}.git")) {
        return Err(format!("The project's \"{remote}\" remote no longer points at {repo} on GitHub, so Fridae won't push there. Press Put on GitHub to make a new copy, or set the address back."));
    }
    Ok(())
}

/// Push the current branch to a GitHub remote with the app's sign-in. The line to show, and whether it worked.
fn push_branch(dir: &Path, remote: &str, branch: &str, token: String) -> (bool, String) {
    let Some(trade) = open_trade(token) else { return (false, "Fridae could not get ready to hand git the sign-in.".to_string()) };
    let spec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let (ok, out) = git(dir, &["push", "-u", "--", remote, &spec], Some(&trade.env));
    drop(trade);
    let low = out.to_lowercase();
    if ok {
        return (true, if low.contains("everything up-to-date") { "GitHub already had everything.".to_string() } else { "Sent your latest commits to GitHub.".to_string() });
    }
    let line = if low.contains("authentication failed") || low.contains("could not read username") || low.contains("could not read password") || low.contains(" 403") || low.contains("permission to") {
        REFUSED.to_string()
    } else if low.contains("rejected") || low.contains("fetch first") || low.contains("non-fast-forward") {
        "GitHub has changes this computer doesn't have yet. Press Pull first, then push again.".to_string()
    } else if low.contains("could not resolve host") || low.contains("unable to access") || low.contains("timed out") {
        OFFLINE.to_string()
    } else {
        let last = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(200).collect::<String>();
        format!("The push did not work. Git said: {last}")
    };
    (false, line)
}

/// Start a job for a project in the background (one at a time per project); its end is kept for the page.
fn start_job(path: &str, first: &str, work: impl FnOnce() -> (bool, String) + Send + 'static) -> Value {
    let started = with(|s| {
        if s.jobs.get(path).is_some_and(|j| j.state == "running") {
            return false;
        }
        s.jobs.insert(path.to_string(), Job { state: "running", at: now(), text: first.to_string() });
        true
    });
    if !started {
        return json!({"ok": false, "busy": true, "text": BUSY, "github": project_info(path)});
    }
    let key = path.to_string();
    std::thread::spawn(move || {
        let (ok, text) = work();
        with(|s| s.jobs.insert(key, Job { state: if ok { "ok" } else { "error" }, at: now(), text }));
    });
    json!({"ok": true, "running": true, "text": first, "github": project_info(path)})
}

fn running(path: &str) -> bool {
    with(|s| s.jobs.get(path).is_some_and(|j| j.state == "running"))
}

/// Put on GitHub: make a repository under the user's account (private unless the user unticked it), make it the
/// project's remote (`origin`, or `github` when `origin` is already something else, which is left as it is), pin it
/// as this project's GitHub copy, and push the current branch there. Checked at once; the rest runs in the background.
pub fn put(path: &str, name: &str, private: bool) -> Value {
    let fail = |text: &str| json!({"ok": false, "text": text});
    if running(path) {
        return json!({"ok": false, "busy": true, "text": BUSY});
    }
    let (dir, branch) = match ready(path) {
        Ok(r) => r,
        Err(why) => return fail(&why),
    };
    let name = name.trim().to_string();
    if !name_ok(&name) {
        return fail("Use only letters, numbers, dashes, dots and underscores in the name (no spaces), 100 at most.");
    }
    let Some(token) = read_token() else { return fail(NOT_CONNECTED) };
    if user()["login"].is_null() {
        status(None); // asks GitHub for the profile again
        return fail("Fridae doesn't know yet which GitHub account is connected. Try again in a moment.");
    }
    if let Some(why) = config_problem(&dir, "origin").or_else(|| config_problem(&dir, "github")) {
        return fail(&why);
    }
    let key = path.to_string();
    start_job(path, "Putting the project on GitHub...", move || put_now(&key, &dir, &branch, &name, private, token))
}

fn put_now(path: &str, dir: &Path, branch: &str, name: &str, private: bool, token: String) -> (bool, String) {
    let body = json!({"name": name, "private": private, "description": "Made with Fridae Studios", "auto_init": false});
    let (code, v) = match api("POST", "/user/repos", &token, Some(body)) {
        Ok(got) => got,
        Err(e) => return (false, e),
    };
    match code {
        201 => {}
        401 => return (false, REFUSED.to_string()),
        422 if v["errors"].as_array().into_iter().flatten().any(|e| e["message"].as_str().is_some_and(|m| m.contains("already exists"))) => {
            return (false, format!("You already have a repository called {name} on GitHub. Pick another name."));
        }
        _ => {
            let why = said(&v);
            return (false, if why.is_empty() { format!("GitHub did not make the repository (it answered {code}).") } else { format!("GitHub did not make the repository: {why}") });
        }
    }
    let full = v["full_name"].as_str().unwrap_or("").to_string();
    let clone = format!("{}/{full}.git", web_base());
    if !repo_ok(&full) || v["clone_url"].as_str() != Some(clone.as_str()) {
        return (false, "GitHub's answer did not name the new repository in a way Fridae could check.".to_string());
    }
    let (has_origin, origin) = git(dir, &["remote", "get-url", "origin"], None);
    let remote = if !has_origin {
        let (ok, out) = git(dir, &["remote", "add", "origin", &clone], None);
        if !ok {
            return (false, format!("The repository {full} was made on GitHub, but git could not save its address: {out}"));
        }
        "origin"
    } else if origin.trim().trim_end_matches(".git") == clone.trim_end_matches(".git") {
        "origin"
    } else {
        let had = git(dir, &["remote", "get-url", "github"], None).0;
        let (ok, out) = git(dir, &["remote", if had { "set-url" } else { "add" }, "github", &clone], None);
        if !ok {
            return (false, format!("The repository {full} was made on GitHub, but git could not save its address: {out}"));
        }
        "github"
    };
    if !pin(path, &full, remote) {
        return (false, format!("The repository {full} was made on GitHub, but Fridae could not note it on this computer."));
    }
    let kind = if v["private"].as_bool().unwrap_or(private) { "private" } else { "public" };
    let (ok, line) = match check_target(dir, &full, remote) {
        Ok(()) => push_branch(dir, remote, branch, token),
        Err(why) => (false, why),
    };
    let mut text = if ok { format!("Made the {kind} repository {full} on GitHub and sent your project there.") } else { format!("Made the {kind} repository {full} on GitHub, but sending your project there did not work. {line}") };
    if remote == "github" {
        text.push_str(" Your other online copy (\"origin\") was left as it is; GitHub is saved as \"github\".");
    }
    (ok, text)
}

/// Push now: send the current branch to the project's GitHub copy, after checking it is still the one Put on GitHub
/// made, on the connected account. Checked at once; the push runs in the background.
pub fn push(path: &str) -> Value {
    let fail = |text: &str| json!({"ok": false, "text": text});
    if running(path) {
        return json!({"ok": false, "busy": true, "text": BUSY});
    }
    let (dir, branch) = match ready(path) {
        Ok(r) => r,
        Err(why) => return fail(&why),
    };
    let Some(token) = read_token() else { return fail(NOT_CONNECTED) };
    let Some((repo, remote)) = target_of(path) else { return fail(NOT_PUT) };
    if let Err(why) = check_target(&dir, &repo, &remote) {
        return fail(&why);
    }
    // checked again in the job, right before git runs: the folder's git setup may have changed since the press
    start_job(path, &format!("Sending to {repo} on GitHub..."), move || match check_target(&dir, &repo, &remote) {
        Ok(()) => push_branch(&dir, &remote, &branch, token),
        Err(why) => (false, why),
    })
}

/// Open a GitHub page in the browser: the sign-in page, the app's settings page there, or a project's repository.
fn open(body: &Value) -> Value {
    let url = match body["what"].as_str().unwrap_or("") {
        "device" => with(|s| s.pending.as_ref().map(|p| p.verification_uri.clone())),
        "settings" => Some(format!("{}/settings/connections/applications/{}", web_base(), client_id())),
        "repo" => body["project"].as_str().filter(|p| known_project(p)).and_then(target_of).map(|t| format!("{}/{}", web_base(), t.0)),
        _ => None,
    };
    let ok = url.as_deref().is_some_and(|u| u.starts_with(&format!("{}/", web_base())));
    if let Some(u) = url.filter(|_| ok) {
        open_page(&u);
    }
    json!({"ok": ok})
}

/// POST /api/github/<action>.
pub fn post(path: &str, body: &Value) -> Value {
    let project = body["project"].as_str().unwrap_or("");
    match path.trim_start_matches("/api/github/") {
        "connect" => connect(),
        "cancel" => cancel(),
        "disconnect" => disconnect(),
        "put" => put(project, body["name"].as_str().unwrap_or(""), body["private"].as_bool().unwrap_or(true)),
        "push" => push(project),
        "open" => open(body),
        _ => json!({"ok": false, "error": "Unknown action."}),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{MutexGuard, atomic::AtomicU64},
    };

    /// Git in tests never sees the user's own settings or credential helpers: a scratch home and global config.
    pub fn isolate(cmd: &mut Command) {
        let home = paths::test_dir("github-home");
        let _ = fs::create_dir_all(&home);
        let global = home.join("gitconfig");
        if !global.exists() {
            let _ = fs::write(&global, "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n");
        }
        cmd.env("HOME", &home).env("XDG_CONFIG_HOME", &home).env("GIT_CONFIG_GLOBAL", &global).env("GIT_CONFIG_NOSYSTEM", "1");
    }

    fn sh_git(dir: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(dir);
        isolate(&mut cmd);
        use crate::paths::Start;
        let o = cmd.run_output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    const TOKEN: &str = "gho_testtoken0123456789";

    /// Each request the stand-in had: (when, path, body).
    type Log = Arc<Mutex<Vec<(Instant, String, String)>>>;

    /// A stand-in for GitHub on 127.0.0.1: the device sign-in, the API, and git over HTTP (git http-backend) behind
    /// the token as a password.
    struct Stub {
        url: String,
        /// the answers to "was it approved?", in order (then: still pending); {"__status": 502} answers that status
        /// with a page that is not JSON
        answers: Arc<Mutex<VecDeque<Value>>>,
        log: Log,
        repos: PathBuf,
        /// how long the device code takes to come, in ms
        delay: Arc<AtomicU64>,
    }

    fn respond(req: tiny_http::Request, code: u16, body: &str, headers: &[(&str, &str)]) {
        let mut r = tiny_http::Response::from_string(body).with_status_code(code);
        for (k, v) in headers {
            r = r.with_header(tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
        }
        let _ = req.respond(r);
    }

    fn stub(existing: &[&str]) -> Stub {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let url = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let repos = paths::test_dir(&format!("github-repos-{}", paths::random_hex(1)));
        fs::create_dir_all(repos.join("tester")).unwrap();
        let answers: Arc<Mutex<VecDeque<Value>>> = Default::default();
        let log: Log = Default::default();
        let delay: Arc<AtomicU64> = Default::default();
        let (a, l, root, base, d) = (answers.clone(), log.clone(), repos.clone(), url.clone(), delay.clone());
        let existing: Vec<String> = existing.iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let full = req.url().to_string();
                let path = full.split('?').next().unwrap_or("").to_string();
                let auth = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
                let ctype = req.headers().iter().find(|h| h.field.equiv("Content-Type")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
                let mut raw = Vec::new();
                let _ = req.as_reader().read_to_end(&mut raw);
                l.lock().unwrap().push((Instant::now(), path.clone(), String::from_utf8_lossy(&raw).to_string()));
                let bearer = auth == format!("Bearer {TOKEN}");
                match path.as_str() {
                    "/login/device/code" => {
                        let (wait, base) = (d.load(Ordering::SeqCst), base.clone());
                        // answered from a thread of its own, so a slow code holds nothing else up
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(wait));
                            respond(req, 200, &json!({"device_code": "dev-secret-1", "user_code": "ABCD-1234", "verification_uri": format!("{base}/login/device"), "expires_in": 900, "interval": 1}).to_string(), &[]);
                        });
                    }
                    "/login/oauth/access_token" => {
                        let v = a.lock().unwrap().pop_front().unwrap_or(json!({"error": "authorization_pending"}));
                        match v["__status"].as_u64() {
                            Some(code) => respond(req, code as u16, "<html>Unicorn!</html>", &[]),
                            None => respond(req, 200, &v.to_string(), &[]),
                        }
                    }
                    "/user" if bearer => respond(req, 200, &json!({"login": "tester", "avatar_url": "https://avatars.example.com/u/1"}).to_string(), &[]),
                    "/user/repos" if bearer => {
                        let v: Value = serde_json::from_slice(&raw).unwrap_or_default();
                        let name = v["name"].as_str().unwrap_or("").to_string();
                        if existing.contains(&name) {
                            respond(req, 422, &json!({"message": "Repository creation failed.", "errors": [{"resource": "Repository", "code": "custom", "field": "name", "message": "name already exists on this account"}]}).to_string(), &[]);
                            continue;
                        }
                        sh_git(&root.join("tester"), &["init", "-q", "--bare", &format!("{name}.git")]);
                        respond(req, 201, &json!({"full_name": format!("tester/{name}"), "private": v["private"], "html_url": format!("{base}/tester/{name}"), "clone_url": format!("{base}/tester/{name}.git")}).to_string(), &[]);
                    }
                    "/user" | "/user/repos" => respond(req, 401, r#"{"message":"Bad credentials"}"#, &[]),
                    p if p.starts_with("/tester/") => {
                        use base64::Engine;
                        let want = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{USERNAME}:{TOKEN}")));
                        if auth != want {
                            respond(req, 401, "", &[("WWW-Authenticate", "Basic realm=\"GitHub\"")]);
                            continue;
                        }
                        let query = full.split_once('?').map(|x| x.1).unwrap_or("").to_string();
                        let mut cmd = Command::new("git");
                        cmd.arg("http-backend").env("GIT_PROJECT_ROOT", &root).env("GIT_HTTP_EXPORT_ALL", "1").env("PATH_INFO", p).env("QUERY_STRING", query);
                        cmd.env("REQUEST_METHOD", req.method().as_str()).env("CONTENT_TYPE", &ctype).env("CONTENT_LENGTH", raw.len().to_string()).env("REMOTE_USER", "tester").env("REMOTE_ADDR", "127.0.0.1");
                        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
                        isolate(&mut cmd);
                        use crate::paths::Start;
                        let mut child = cmd.start().unwrap();
                        let mut input = child.stdin.take().unwrap();
                        let body = raw.clone();
                        let writer = std::thread::spawn(move || {
                            let _ = input.write_all(&body);
                        });
                        let out = child.wait_with_output().unwrap().stdout;
                        let _ = writer.join();
                        let split = out.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4)).or_else(|| out.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2))).unwrap_or((out.len(), 0));
                        let head = String::from_utf8_lossy(&out[..split.0]).to_string();
                        let mut r = tiny_http::Response::from_data(out[split.0 + split.1..].to_vec());
                        for line in head.lines() {
                            if let Some((k, v)) = line.split_once(':') {
                                if k.eq_ignore_ascii_case("Status") {
                                    r = r.with_status_code(v.trim().split(' ').next().unwrap_or("200").parse::<u16>().unwrap_or(200));
                                } else {
                                    r = r.with_header(tiny_http::Header::from_bytes(k.trim().as_bytes(), v.trim().as_bytes()).unwrap());
                                }
                            }
                        }
                        let _ = req.respond(r);
                    }
                    _ => respond(req, 404, r#"{"message":"Not Found"}"#, &[]),
                }
            }
        });
        Stub { url, answers, log, repos, delay }
    }

    /// Held for a test: one GitHub stand-in, a clean state and no sign-in kept.
    fn setup(existing: &[&str], projects: Vec<String>) -> (MutexGuard<'static, ()>, Stub) {
        let held = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = stub(existing);
        *STAND_IN.lock().unwrap() = Some(StandIn { web: s.url.clone(), api: s.url.clone(), raw: String::new(), vault_repo: None, client_id: None, projects });
        disconnect();
        let _ = fs::remove_file(targets_file());
        // (the count carries on: a waiting thread left by an earlier test must never find its number current again)
        with(|st| {
            let flow = st.flow + 1;
            *st = State { flow, ..State::default() };
        });
        (held, s)
    }

    fn wait_until(what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let v = status(None);
            if f(&v) {
                return v;
            }
            assert!(Instant::now() < until, "{what}: {v}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A job's end, as the page sees it: `last_push`.
    fn finished(path: &str) -> Value {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let last = status(Some(path))["project"]["last_push"].clone();
            if last["state"] != "running" {
                return last;
            }
            assert!(Instant::now() < until, "the job did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn access_requests(s: &Stub) -> Vec<(Instant, String)> {
        s.log.lock().unwrap().iter().filter(|(_, p, _)| p == "/login/oauth/access_token").map(|(t, _, b)| (*t, b.clone())).collect()
    }

    fn requests_to(s: &Stub, path: &str) -> usize {
        s.log.lock().unwrap().iter().filter(|(_, p, _)| p == path).count()
    }

    /// Signed in as the stand-in's user, as a finished sign-in leaves it.
    fn signed_in() {
        save_token(TOKEN).unwrap();
        write_private(&user_file(), r#"{"login":"tester","avatar_url":""}"#).unwrap();
    }

    #[test]
    fn each_answer_to_was_it_approved_means_one_thing() {
        assert_eq!(step(200, &json!({"error": "authorization_pending"}), 5), Step::Wait(5));
        assert_eq!(step(200, &json!({"error": "slow_down"}), 5), Step::Wait(10), "five seconds slower");
        assert_eq!(step(200, &json!({"error": "slow_down", "interval": 15}), 5), Step::Wait(15), "or what GitHub says, when more");
        assert_eq!(step(200, &json!({"error": "slow_down", "interval": 2}), 5), Step::Wait(10));
        assert_eq!(step(200, &json!({"error": "expired_token"}), 5), Step::Fail(EXPIRED.into()));
        assert_eq!(step(200, &json!({"error": "access_denied"}), 5), Step::Fail(DENIED.into()));
        assert_eq!(step(200, &json!({"access_token": "t", "token_type": "bearer"}), 5), Step::Done("t".into()));
        assert!(matches!(step(200, &json!({"error": "unsupported_grant_type", "error_description": "Bad grant"}), 5), Step::Fail(w) if w.contains("Bad grant")));
        // GitHub having a bad moment is not an answer: asked again
        assert_eq!(step(502, &json!({"error": "access_denied"}), 5), Step::Wait(5));
        assert_eq!(step(200, &Value::Null, 5), Step::Wait(5));
        assert!(matches!(step(200, &json!({}), 5), Step::Fail(_)));
    }

    #[test]
    fn connect_shows_the_code_then_keeps_the_token_only_the_app_can_read() {
        let (_held, s) = setup(&[], vec![]);
        s.answers.lock().unwrap().extend([json!({"error": "authorization_pending"}), json!({"access_token": TOKEN, "token_type": "bearer", "scope": "repo,read:user"})]);
        let v = connect();
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["pending"]["user_code"], "ABCD-1234");
        assert_eq!(v["pending"]["verification_uri"], format!("{}/login/device", s.url));
        assert!(v["pending"]["expires_at"].as_u64().unwrap() > now() + 800);
        assert!(!v.to_string().contains("dev-secret-1"), "the device code is never shown");
        // what was asked for: the app's Client ID and the two scopes, and the device-code grant
        let first = s.log.lock().unwrap()[0].clone();
        assert_eq!(first.1, "/login/device/code");
        assert!(first.2.contains(&format!("client_id={GITHUB_CLIENT_ID}")) && first.2.contains("scope=repo+read%3Auser"), "{}", first.2);
        let v = wait_until("connected", |v| v["connected"] == true && v["login"] == "tester");
        assert_eq!((v["avatar_url"].as_str(), v["pending"].is_null(), v["error"].as_str()), (Some("https://avatars.example.com/u/1"), true, Some("")));
        let asks = access_requests(&s);
        assert_eq!(asks.len(), 2);
        assert!(asks[0].1.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code") && asks[0].1.contains("device_code=dev-secret-1"), "{}", asks[0].1);
        // the token: in the app's own folder, 0600, and that folder is one no agent's file tools may write or read
        assert_eq!(fs::read_to_string(token_file()).unwrap(), TOKEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(token_file()).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(folder()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert!(paths::app_protected().iter().any(|(p, dir)| *dir && token_file().starts_with(p)), "its folder is protected");
        assert!(paths::app_secrets().iter().any(|p| token_file().starts_with(p)), "and secret");
        assert!(!v.to_string().contains(TOKEN));
    }

    #[test]
    fn slow_down_makes_the_next_question_five_seconds_later() {
        let (_held, s) = setup(&[], vec![]);
        s.answers.lock().unwrap().extend([json!({"error": "authorization_pending"}), json!({"error": "slow_down", "interval": 6}), json!({"error": "authorization_pending"}), json!({"access_token": TOKEN})]);
        connect();
        wait_until("connected", |v| v["connected"] == true);
        let asks = access_requests(&s);
        assert_eq!(asks.len(), 4);
        let gap = |i: usize| asks[i + 1].0 - asks[i].0;
        // the interval was 1 (10 ms in tests); after slow_down it is 6 (60 ms) for every question after
        assert!(gap(1) >= UNIT * 6 && gap(2) >= UNIT * 6, "{:?} {:?}", gap(1), gap(2));
        assert!(gap(0) < UNIT * 6, "{:?}", gap(0));
    }

    #[test]
    fn a_server_error_or_a_page_that_is_not_json_is_asked_again() {
        let (_held, s) = setup(&[], vec![]);
        s.answers.lock().unwrap().extend([json!({"__status": 502}), json!({"__status": 200}), json!({"__status": 503}), json!({"access_token": TOKEN})]);
        connect();
        let v = wait_until("connected", |v| v["connected"] == true || v["pending"].is_null());
        assert_eq!((v["connected"].as_bool(), v["error"].as_str()), (Some(true), Some("")), "{v}");
        assert_eq!(access_requests(&s).len(), 4);
    }

    #[test]
    fn a_code_that_ran_out_or_was_refused_is_said_plainly() {
        for (answer, line) in [("expired_token", EXPIRED), ("access_denied", DENIED)] {
            let (_held, s) = setup(&[], vec![]);
            s.answers.lock().unwrap().push_back(json!({"error": answer}));
            connect();
            let v = wait_until(answer, |v| v["pending"].is_null());
            assert_eq!((v["error"].as_str(), v["connected"].as_bool()), (Some(line), Some(false)));
            assert!(!token_file().exists());
        }
    }

    #[test]
    fn cancel_stops_waiting_and_nothing_is_kept() {
        let (_held, s) = setup(&[], vec![]);
        connect();
        let v = cancel();
        assert!(v["pending"].is_null());
        s.answers.lock().unwrap().push_back(json!({"access_token": TOKEN}));
        std::thread::sleep(UNIT * 5);
        assert!(!connected(), "a sign-in approved after Cancel is not kept");
    }

    #[test]
    fn cancel_while_github_is_asked_for_a_code_wins() {
        let (_held, s) = setup(&[], vec![]);
        s.delay.store(300, Ordering::SeqCst);
        let pressed = std::thread::spawn(connect);
        while requests_to(&s, "/login/device/code") == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        cancel();
        let v = pressed.join().unwrap();
        assert_eq!((v["ok"].as_bool(), v["pending"].is_null()), (Some(false), true), "{v}");
        std::thread::sleep(UNIT * 5);
        assert!(status(None)["pending"].is_null() && access_requests(&s).is_empty(), "nothing waits for an approval");
    }

    #[test]
    fn connect_pressed_again_while_a_code_waits_keeps_that_code() {
        let (_held, s) = setup(&[], vec![]);
        let first = connect();
        let again = connect();
        assert_eq!((again["ok"].as_bool(), &again["pending"]["user_code"]), (Some(true), &first["pending"]["user_code"]));
        assert_eq!(requests_to(&s, "/login/device/code"), 1, "GitHub is asked once, and no second page opens");
        cancel();
    }

    #[test]
    fn without_a_client_id_connect_says_it_is_not_set_up() {
        let (_held, s) = setup(&[], vec![]);
        STAND_IN.lock().unwrap().as_mut().unwrap().client_id = Some(String::new());
        assert_eq!(status(None)["configured"], false);
        let v = connect();
        assert_eq!((v["ok"].as_bool(), v["error"].as_str()), (Some(false), Some(NOT_SET_UP)));
        assert!(s.log.lock().unwrap().is_empty(), "GitHub is not asked");
        STAND_IN.lock().unwrap().as_mut().unwrap().client_id = None;
        assert_eq!(status(None)["configured"], true);
    }

    #[test]
    fn disconnect_removes_the_sign_in_and_nothing_on_its_way_writes_it_back() {
        let (_held, _s) = setup(&[], vec![]);
        signed_in();
        fs::write(token_file().with_extension("tmp"), TOKEN).unwrap();
        let flow = with(|s| s.flow);
        assert_eq!(status(None)["connected"], true);
        assert_eq!(status(None)["forgot"], false);
        let v = disconnect();
        assert_eq!((v["ok"].as_bool(), v["connected"].as_bool(), v["forgot"].as_bool()), (Some(true), Some(false), Some(true)));
        assert!(!token_file().exists() && !user_file().exists() && !token_file().with_extension("tmp").exists());
        // a profile that was on its way when Disconnect was pressed is not kept
        assert!(!fetch_user(TOKEN, flow));
        assert!(!user_file().exists());
    }

    #[test]
    fn the_askpass_helper_trades_its_code_once_and_never_reads_a_file() {
        let (_held, _s) = setup(&[], vec![]);
        signed_in();
        let trade = open_trade(TOKEN.to_string()).unwrap();
        assert_eq!(askpass_answer("Username for 'https://github.com': ", "").as_deref(), Some(USERNAME));
        let port = trade.env.split_once(':').unwrap().0.to_string();
        assert_eq!(askpass_answer("Password: ", &format!("{port}:{}", "0".repeat(32))), None, "a wrong code gets nothing");
        assert_eq!(askpass_answer("Password for 'https://x-access-token@github.com': ", &trade.env).as_deref(), Some(TOKEN));
        assert_eq!(askpass_answer("Password: ", &trade.env), None, "once only");
        assert_eq!(askpass_answer("Are you sure (yes/no)? ", &trade.env), None);
        drop(trade);
        // the token file's path, or anything that is not "<port>:<code>", is no way to the token
        let path = token_file().display().to_string();
        assert_eq!(askpass_answer("Password: ", &path), None);
        assert_eq!(askpass_answer("Password: ", &format!("{port}:{path}")), None);
    }

    /// A project with one commit, as a known project.
    fn project(s: &Stub) -> String {
        let dir = s.repos.parent().unwrap().join(format!("game-{}", paths::random_hex(1)));
        fs::create_dir_all(&dir).unwrap();
        sh_git(&dir, &["init", "-q", "-b", "main"]);
        fs::write(dir.join("GAME.md"), "# A game\n").unwrap();
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-q", "-m", "first"]);
        let path = dir.display().to_string();
        STAND_IN.lock().unwrap().as_mut().unwrap().projects.push(path.clone());
        path
    }

    fn commit_more(dir: &Path) {
        fs::write(dir.join(format!("more-{}.txt", paths::random_hex(1))), "more").unwrap();
        sh_git(dir, &["add", "-A"]);
        sh_git(dir, &["commit", "-q", "-m", "more"]);
    }

    #[test]
    fn put_on_github_makes_a_private_repo_sets_the_remote_and_pushes_with_the_askpass_helper() {
        let (_held, s) = setup(&[], vec![]);
        assert!(askpass_program().is_some(), "the app built beside the tests (cargo builds it for tests/askpass.rs)");
        let path = project(&s);
        assert_eq!(put(&path, "my-game", true)["text"], NOT_CONNECTED);
        signed_in();
        let v = post("/api/github/put", &json!({"project": path, "name": "my-game"}));
        assert_eq!((v["ok"].as_bool(), v["running"].as_bool()), (Some(true), Some(true)), "answered at once, done in the background: {v}");
        assert_eq!(post("/api/github/push", &json!({"project": path}))["busy"], true, "one job at a time");
        let last = finished(&path);
        assert_eq!(last["ok"], true, "{last}");
        assert!(last["text"].as_str().unwrap().starts_with("Made the private repository tester/my-game on GitHub"), "{last}");
        let info = status(Some(&path))["project"].clone();
        assert_eq!((info["repo"].as_str(), info["remote"].as_str(), info["web_url"].as_str()), (Some("tester/my-game"), Some("origin"), Some(format!("{}/tester/my-game", s.url).as_str())));
        let made = s.log.lock().unwrap().iter().find(|(_, p, _)| p == "/user/repos").map(|x| x.2.clone()).unwrap();
        let made: Value = serde_json::from_str(&made).unwrap();
        assert_eq!((made["name"].as_str(), made["private"].as_bool()), (Some("my-game"), Some(true)), "private unless unticked");
        // the commit is on the stand-in GitHub, which took it only with the token as the password
        let dir = Path::new(&path);
        let bare = s.repos.join("tester").join("my-game.git");
        assert_eq!(sh_git(&bare, &["rev-parse", "main"]), sh_git(dir, &["rev-parse", "HEAD"]));
        assert_eq!(sh_git(dir, &["remote", "get-url", "origin"]), format!("{}/tester/my-game.git", s.url));
        assert_eq!(sh_git(dir, &["rev-parse", "--abbrev-ref", "main@{upstream}"]), "origin/main", "pushed with -u");
        // nothing about the sign-in was written into the project's git settings
        let config = fs::read_to_string(dir.join(".git").join("config")).unwrap();
        assert!(!config.contains(TOKEN) && !config.contains("credential") && !config.contains("askpass"), "{config}");
        // Push now: a new commit goes up the same way, and the line is kept for the page
        commit_more(dir);
        let v = post("/api/github/push", &json!({"project": path}));
        assert_eq!((v["ok"].as_bool(), v["text"].as_str()), (Some(true), Some("Sending to tester/my-game on GitHub...")), "{v}");
        let last = finished(&path);
        assert_eq!((last["ok"].as_bool(), last["text"].as_str()), (Some(true), Some("Sent your latest commits to GitHub.")), "{last}");
        assert_eq!(sh_git(&bare, &["rev-parse", "main"]), sh_git(dir, &["rev-parse", "HEAD"]));
        push(&path);
        assert_eq!(finished(&path)["text"], "GitHub already had everything.");
    }

    #[test]
    fn a_project_with_another_origin_gets_a_github_remote_beside_it() {
        let (_held, s) = setup(&[], vec![]);
        let path = project(&s);
        let dir = Path::new(&path);
        sh_git(dir, &["remote", "add", "origin", "https://gitlab.example.com/me/game.git"]);
        signed_in();
        put(&path, "game2", false);
        let last = finished(&path);
        assert_eq!(last["ok"], true, "{last}");
        assert!(last["text"].as_str().unwrap().contains("public repository") && last["text"].as_str().unwrap().contains("left as it is"), "{last}");
        assert_eq!(status(Some(&path))["project"]["remote"], "github");
        assert_eq!(sh_git(dir, &["remote", "get-url", "origin"]), "https://gitlab.example.com/me/game.git");
        commit_more(dir);
        assert_eq!(push(&path)["ok"], true);
        assert_eq!(finished(&path)["ok"], true);
    }

    #[test]
    fn push_goes_only_to_the_pinned_repository_on_the_connected_account() {
        let (_held, s) = setup(&[], vec![]);
        let path = project(&s);
        let dir = Path::new(&path);
        signed_in();
        assert_eq!(push(&path)["text"], NOT_PUT, "a GitHub address the user pasted is not pushed with the app's sign-in");
        put(&path, "pinned", true);
        assert_eq!(finished(&path)["ok"], true);
        // the remote moved to another repository, even another of the user's: refused
        sh_git(dir, &["remote", "set-url", "origin", &format!("{}/tester/elsewhere.git", s.url)]);
        assert!(push(&path)["text"].as_str().unwrap().contains("no longer points at tester/pinned"));
        sh_git(dir, &["remote", "set-url", "origin", &format!("{}/tester/pinned.git", s.url)]);
        // a second push address, or settings that send it elsewhere or otherwise
        for (key, value) in [("remote.origin.pushurl", "https://evil.example.com/x.git"), ("url.https://evil.example.com/.pushInsteadOf", "http://127.0.0.1"), ("http.proxy", "http://127.0.0.1:9"), ("remote.origin.mirror", "true"), ("http.sslVerify", "false")] {
            sh_git(dir, &["config", key, value]);
            let v = push(&path);
            assert_eq!(v["ok"], false, "{key}: {v}");
            assert!(v["text"].as_str().unwrap().to_lowercase().contains(&key.to_lowercase()), "{key}: {v}");
            sh_git(dir, &["config", "--unset", key]);
        }
        sh_git(dir, &["config", "--add", "remote.origin.url", "https://evil.example.com/y.git"]);
        assert!(push(&path)["text"].as_str().unwrap().contains("more than one address"));
        sh_git(dir, &["config", "--unset", "remote.origin.url", "evil"]);
        // signed in as someone else now: the copy is not theirs
        write_private(&user_file(), r#"{"login":"someone-else"}"#).unwrap();
        assert!(push(&path)["text"].as_str().unwrap().contains("belongs to tester"));
        write_private(&user_file(), r#"{"login":"tester"}"#).unwrap();
        commit_more(dir);
        assert_eq!(push(&path)["ok"], true);
        assert_eq!(finished(&path)["ok"], true);
    }

    #[test]
    fn a_branch_that_reads_like_an_option_is_not_pushed() {
        assert!(branch_ok("main") && branch_ok("feature/x") && !branch_ok("-x") && !branch_ok("--mirror") && !branch_ok("a:b"));
        let (_held, s) = setup(&[], vec![]);
        let path = project(&s);
        let dir = Path::new(&path);
        signed_in();
        sh_git(dir, &["update-ref", "refs/heads/--mirror", "HEAD"]);
        sh_git(dir, &["symbolic-ref", "HEAD", "refs/heads/--mirror"]);
        let v = put(&path, "opt", true);
        assert!(v["text"].as_str().unwrap().contains("won't push a branch named \"--mirror\""), "{v}");
        assert_eq!(requests_to(&s, "/user/repos"), 0);
    }

    #[test]
    fn a_name_that_is_taken_or_not_allowed_is_said_plainly() {
        let (_held, s) = setup(&["taken"], vec![]);
        let path = project(&s);
        signed_in();
        put(&path, "taken", true);
        let last = finished(&path);
        assert_eq!((last["ok"].as_bool(), last["text"].as_str()), (Some(false), Some("You already have a repository called taken on GitHub. Pick another name.")));
        assert!(sh_git(Path::new(&path), &["remote"]).is_empty(), "no remote is set for a repository that was not made");
        assert!(status(Some(&path))["project"]["repo"].is_null());
        assert!(put(&path, "my game", true)["text"].as_str().unwrap().contains("no spaces"));
        assert!(!name_ok("") && !name_ok("..") && !name_ok("a/b") && name_ok("My_Game.v2-x"));
    }

    #[test]
    fn only_the_users_projects_and_only_with_a_commit() {
        let (_held, s) = setup(&[], vec![]);
        signed_in();
        let elsewhere = s.repos.display().to_string();
        assert_eq!(put(&elsewhere, "x", true)["text"], "That folder is not one of your projects.");
        assert_eq!(push(&elsewhere)["text"], "That folder is not one of your projects.");
        let empty = s.repos.parent().unwrap().join(format!("empty-{}", paths::random_hex(1)));
        fs::create_dir_all(&empty).unwrap();
        sh_git(&empty, &["init", "-q"]);
        let empty = empty.display().to_string();
        STAND_IN.lock().unwrap().as_mut().unwrap().projects.push(empty.clone());
        assert_eq!(put(&empty, "x", true)["text"], "Commit your files first, then send them to GitHub.");
        assert_eq!(requests_to(&s, "/user/repos"), 0, "no repository is made for it");
    }

    #[test]
    fn the_token_never_reaches_an_agent_or_the_page() {
        let (_held, s) = setup(&[], vec![]);
        signed_in();
        let path = project(&s);
        // the tool servers every agent gets (their commands, arguments and environment)
        for no_studio in [false, true] {
            assert!(!crate::agents::test_servers(no_studio).to_string().contains(TOKEN));
        }
        // every route's answer, and what the page reads while a job runs and after
        let mut answers = vec![status(Some(&path)), post("/api/github/put", &json!({"project": path, "name": "seen"}))];
        answers.push(status(Some(&path)));
        answers.push(finished(&path));
        commit_more(Path::new(&path));
        answers.push(post("/api/github/push", &json!({"project": path})));
        answers.push(finished(&path));
        for action in ["open", "cancel", "nothing", "connect", "disconnect"] {
            answers.push(post(&format!("/api/github/{action}"), &json!({"what": "settings"})));
        }
        for a in &answers {
            assert!(!a.to_string().contains(TOKEN), "{a}");
        }
        assert_eq!(answers[3]["ok"], true, "{}", answers[3]);
    }
}
