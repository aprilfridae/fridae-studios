//! Signing in to the AIs that run on the user's own sign-in: Claude Code, Cursor and Gemini (Antigravity).
//!
//! These three are the user's own tools: their sign-in is the one the user has outside Fridae too (in the login
//! keychain on a Mac). So the app only ever asks a tool who is signed in, and starts the tool's own sign-in when the
//! user presses Sign in. It never signs anyone out: nothing in the app runs a tool's own command for that, and there
//! is no button for it (a test reads every source file to keep it so). (OpenCode is different: Fridae keeps a copy of
//! its own, with a sign-in of its own, see `opencode.rs`.)
//!
//! Who is signed in (asked at start, after a sign-in, when a turn fails as signed out, and otherwise every few
//! minutes at most, `due`), with the environment a turn of that AI gets and a time limit (`run_limited`):
//!
//!   claude auth status --json          {"loggedIn": true, "authMethod": ..., "email": ...}, or {"loggedIn": false}
//!                                      with exit code 1. A key or a gateway also reads as `loggedIn` (measured by
//!                                      the reviewer: `authMethod` api_key, oauth_token, api_key_helper, third_party)
//!   cursor-agent status --format json  {"isAuthenticated": true, "userInfo": {"email": ...}}, or false
//!   agy models                         the model list; signed out: exit code 1 and "Error: Please sign in ..."
//!                                      (the one `antigravity::ask_models` runs for the model menu anyway; agy has
//!                                      no command that says who is signed in)
//!
//! Signing in (each tool opens the browser itself and waits for it; nothing has to be typed back):
//!
//!   claude auth login       prints "Opening browser to sign in…" and ends when the browser has come back
//!   cursor-agent login      prints "Waiting for browser authentication..." and ends the same way
//!   agy acp                 `initialize`, then `authenticate` with the method "oauth-personal" ("Log in with
//!                           Google"): agy opens Google's page and answers the request when it is done
//!
//! A sign-in is never started over one that is there without the user saying so twice: the button first asks the tool
//! again, and when the answer is not "signed out" it only says what pressing again will do (`login_start`, `sure`).
//! Whether a tool keeps the sign-in that was there when a new one is started and then cancelled is not measured (it
//! would take a sign-in to lose); it can be measured safely on a second account's folder once one is signed in.
//!
//! Measured on 23 November 2026 with Claude Code 2.1.319, cursor-agent 2026.11.22 and agy 1.5.15: the status
//! commands for real (signed in) and in a scratch home (signed out); the start of each sign-in in a scratch home, where
//! it cannot see the user's own sign-in, with a stand-in for the browser, ended before anything was done on the page.
//! No sign-in was completed, so how each one ends (its exit code, agy's answer) is not measured: whatever the tool
//! says at its end, the app asks for the status again and goes by that. Not offered on Windows (the lines there say
//! what they said before: sign in from a terminal).

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    antigravity,
    paths::{self, Start},
    procs,
};

/// The AIs whose sign-in is the user's own.
pub const AIS: [&str; 3] = ["claude", "cursor", "gemini"];

/// A second account's sign-in (`accounts`): the same tool, asked and signed in the same way, in the app's own folder
/// for it. The one sign-in here that is not the user's own outside Fridae.
pub const SECOND: [&str; 1] = ["claude2"];

/// Every sign-in the app knows: the three AIs' own, then the second accounts'.
fn ids() -> impl Iterator<Item = &'static str> {
    AIS.into_iter().chain(SECOND)
}

/// The AI a sign-in belongs to ("claude2" is Claude's second account).
pub fn tool(id: &str) -> &str {
    match id {
        "claude2" => "claude",
        other => other,
    }
}

/// A sign-in's name in a line the user reads.
fn name(id: &str) -> String {
    match id {
        "claude2" => "Claude's second account".to_string(),
        other => crate::expert_ai::ai_name(other).to_string(),
    }
}

/// How long the last answer stands before the tool is asked again by itself: one that says "signed in" for half an
/// hour, anything else for five minutes (the user may be signing in from a terminal meanwhile).
const AGAIN_SIGNED_IN: Duration = Duration::from_secs(30 * 60);
const AGAIN_OTHERWISE: Duration = Duration::from_secs(5 * 60);
/// A sign-in nobody finishes in the browser is ended after this long.
const LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);
/// A tool that does not answer "who is signed in" within this is ended (offline, a question from the keychain).
pub const CHECK_WAIT: Duration = Duration::from_secs(30);
/// A press of Sign in while one waits: within this it is the same click twice; later the waiting one is ended and a
/// new one started, so a page that was closed opens again.
const SAME_CLICK: Duration = Duration::from_secs(3);

/// What a tool says about its sign-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Known {
    /// not asked yet, or the tool gave no answer the app can read (not installed, offline, a new output)
    #[default]
    Unknown,
    /// signed in, as the account the tool names (None when it names none)
    In(Option<String>),
    Out,
}

/// A tool's answer: who is signed in, and whether that is a key or a gateway rather than an account (Claude Code's
/// `authMethod`): the app's sign-in, which is an account's, is then not what a failed turn needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub known: Known,
    pub by_key: bool,
}

/// A sign-in the user started with the button: "" none, "waiting" for the browser, "done", "failed".
#[derive(Default)]
struct Login {
    pid: Option<u32>,
    state: &'static str,
    message: String,
    since: Option<Instant>,
}

#[derive(Default)]
struct Entry {
    known: Known,
    by_key: bool,
    asked: Option<Instant>,
    asking: bool,
    /// asked for again while an answer was on its way (a sign-in ended meanwhile): that answer is not taken, and the
    /// tool is asked once more
    again: bool,
    login: Login,
}

static STATE: Mutex<Option<HashMap<&'static str, Entry>>> = Mutex::new(None);

fn with<R>(ai: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    let key = ids().find(|a| *a == ai)?;
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    Some(f(g.get_or_insert_with(HashMap::new).entry(key).or_default()))
}

/// Whether the app offers this sign-in: one of the three AIs' own, or a second account that exists, and not on
/// Windows (not measured there: the lines keep saying what they said before).
pub fn offered(id: &str) -> bool {
    if cfg!(windows) {
        return false;
    }
    AIS.contains(&id) || (SECOND.contains(&id) && crate::accounts::folder(tool(id)).is_some())
}

/// Whether the tool last said that somebody is signed in (for a second account: whether it can take a chat).
pub fn signed_in(id: &str) -> bool {
    with(id, |e| matches!(e.known, Known::In(_))).unwrap_or(false)
}

/// A sign-in is gone (a second account was removed): one that waits is ended, and what was known is forgotten.
pub fn forget(id: &str) {
    end_login(id);
    with(id, |e| *e = Entry::default());
}

// ------------------------------------------------------------- who is signed in

/// Words from a tool as the page shows them: without characters that do not show (control characters, and the ones
/// that turn text around or hide in it).
fn plain(text: &str) -> String {
    let hidden = |c: char| c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}');
    text.chars().filter(|c| !hidden(*c)).collect::<String>().trim().to_string()
}

/// An account as the page shows it: what the tool printed, plain, 80 characters at most.
fn account(name: &str) -> Option<String> {
    let shown: String = plain(name).chars().take(80).collect();
    (!shown.is_empty()).then_some(shown)
}

/// Claude Code's ways of being "logged in" that are a key or a gateway, not an account (its `authMethod`).
const CLAUDE_KEY_METHODS: [&str; 4] = ["api_key", "oauth_token", "api_key_helper", "third_party"];

/// `claude auth status --json`.
pub fn parse_claude(stdout: &str) -> Answer {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else { return Answer::default() };
    let by_key = v["authMethod"].as_str().is_some_and(|m| CLAUDE_KEY_METHODS.contains(&m));
    let known = match v["loggedIn"].as_bool() {
        Some(true) => Known::In(v["email"].as_str().and_then(account)),
        Some(false) => Known::Out,
        None => Known::Unknown,
    };
    Answer { by_key: by_key && matches!(known, Known::In(_)), known }
}

/// `cursor-agent status --format json`.
pub fn parse_cursor(stdout: &str) -> Answer {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else { return Answer::default() };
    let known = match v["isAuthenticated"].as_bool() {
        Some(true) => Known::In(v["userInfo"]["email"].as_str().and_then(account)),
        Some(false) => Known::Out,
        None => Known::Unknown,
    };
    Answer { known, by_key: false }
}

/// `agy models`: a list means signed in (agy does not say as whom); its own "Please sign in" means signed out;
/// anything else (offline, say) is no answer.
pub fn parse_agy(stdout: &str, stderr: &str, ok: bool) -> Answer {
    let known = if ok && stdout.lines().any(|l| l.split_once('\t').is_some_and(|(id, name)| !id.trim().is_empty() && !name.trim().is_empty())) {
        Known::In(None)
    } else if !ok && stderr.to_lowercase().contains("please sign in") {
        Known::Out
    } else {
        Known::Unknown
    };
    Answer { known, by_key: false }
}

/// Whether the tool should be asked again now: never while it is being asked or a sign-in waits for the browser; the
/// first time at once; after that only when the last answer is old enough (`AGAIN_SIGNED_IN`, `AGAIN_OTHERWISE`).
fn due(known: &Known, asked_ago: Option<Duration>, asking: bool, logging_in: bool) -> bool {
    if asking || logging_in {
        return false;
    }
    match asked_ago {
        None => true,
        Some(ago) => ago >= if matches!(known, Known::In(_)) { AGAIN_SIGNED_IN } else { AGAIN_OTHERWISE },
    }
}

/// Ask a tool who is signed in, in the background. `now`: whatever the age of the last answer (at start, after a
/// sign-in, after a turn that failed as signed out); otherwise only when `due`.
pub fn check(ai: &str, now: bool) {
    let Some(ai) = ids().find(|a| *a == ai) else { return };
    let go = with(ai, |e| {
        if now && e.asking {
            e.again = true;
            return false;
        }
        let go = now || due(&e.known, e.asked.map(|t| t.elapsed()), e.asking, e.login.state == "waiting");
        if go {
            e.asking = true;
            e.asked = Some(Instant::now());
        }
        go
    });
    if go != Some(true) {
        return;
    }
    std::thread::spawn(move || {
        loop {
            let answer = ask(ai);
            if with(ai, |e| take_answer(e, answer)) != Some(true) {
                break;
            }
        }
    });
}

/// An answer has come: kept, unless the tool was asked for again meanwhile (then it is asked once more: true).
fn take_answer(e: &mut Entry, answer: Answer) -> bool {
    if std::mem::take(&mut e.again) {
        e.asked = Some(Instant::now());
        return true;
    }
    e.known = answer.known;
    e.by_key = answer.by_key;
    e.asking = false;
    false
}

/// A command run to its end, but no longer than `limit`: what it printed (output, error output) and whether it ended
/// well. In a group of its own and in the ledger while it runs, so one that hangs is ended, here or by the next start
/// of the app. None when it could not be started or was ended for taking too long.
pub fn run_limited(cmd: Command, limit: Duration) -> Option<(String, String, bool)> {
    run_fed(cmd, limit, None, "signin")
}

/// The same, with text for the command to read (`input`), noted in the ledger under `what`.
pub fn run_fed(mut cmd: Command, limit: Duration, input: Option<String>, what: &str) -> Option<(String, String, bool)> {
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::quiet(&mut cmd);
    paths::own_group(&mut cmd);
    let mut child = cmd.start().ok()?;
    let pid = child.id();
    procs::record(what, pid);
    // written from a thread of its own (a command that does not read must not hold this one), then closed
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut pipe, text.as_bytes());
        });
    }
    fn read(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut all);
            }
            String::from_utf8_lossy(&all).to_string()
        })
    }
    let (out, err) = (read(child.stdout.take()), read(child.stderr.take()));
    let until = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                paths::end_trees(&[pid]);
                let _ = child.wait();
                break None;
            }
        }
    };
    procs::forget(pid);
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    status.map(|s| (out, err, s.success()))
}

/// For tests: what each tool answers and the program a sign-in starts, in place of the real tools (tests never run
/// them, whatever is installed on the computer running the tests). Held with `TEST_LOCK`.
#[cfg(test)]
#[derive(Default)]
pub struct StandIn {
    pub status: HashMap<&'static str, Answer>,
    /// a shell script to run as the tool's sign-in; None: no sign-in can be started
    pub login: Option<String>,
}
#[cfg(test)]
pub static STAND_IN: Mutex<Option<StandIn>> = Mutex::new(None);
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

/// The tool's answer, asked now (it takes a moment: never on a thread the page waits for longer than a press).
fn ask(ai: &str) -> Answer {
    #[cfg(test)]
    return STAND_IN.lock().unwrap().as_ref().and_then(|s| s.status.get(ai).cloned()).unwrap_or_default();
    #[allow(unreachable_code)]
    {
        if ai == "gemini" {
            // the model list is the question, and fills the model menu when it comes (on Windows too, where the answer
            // about the sign-in is not shown)
            return antigravity::ask_models().map(|(out, err, ok)| parse_agy(&out, &err, ok)).unwrap_or_default();
        }
        if !offered(ai) {
            return Answer::default();
        }
        let Some(exe) = paths::find_ai(tool(ai)) else { return Answer::default() };
        let mut cmd = Command::new(exe);
        cmd.args(if tool(ai) == "claude" { &["auth", "status", "--json"][..] } else { &["status", "--format", "json"][..] });
        if !tool_env(&mut cmd, ai) {
            return Answer::default();
        }
        // (Claude Code ends with code 1 when nobody is signed in: its answer is read either way)
        let Some((out, _, _)) = run_limited(cmd, CHECK_WAIT) else { return Answer::default() };
        if tool(ai) == "claude" { parse_claude(&out) } else { parse_cursor(&out) }
    }
}

/// The environment a turn of this AI gets, for its status and its sign-in: the same keys kept or passed
/// (`paths::child_env`), for Cursor the app's own config and data folders a Cursor turn runs with, and for a second
/// account its own folder. False when that folder is not there: the command must then not be run at all, since
/// without it the tool would ask about, or sign in, the user's own account.
fn tool_env(cmd: &mut Command, id: &str) -> bool {
    match tool(id) {
        "cursor" => crate::cursor::environment(cmd),
        ai => paths::child_env(cmd, ai),
    }
    if id == "gemini" && let Some(h) = antigravity::home_override() {
        cmd.env("HOME", h);
    }
    !SECOND.contains(&id) || crate::accounts::env(cmd, tool(id), 2)
}

/// At start: every tool is asked once.
pub fn start() {
    for ai in ids() {
        check(ai, true);
    }
}

/// From the page's status: each tool is asked again when its last answer is old enough (`due`).
pub fn refresh() {
    for ai in ids() {
        check(ai, false);
    }
}

// -------------------------------------------------------- a turn that failed

/// What the chat is told when a turn failed: the words, and whether the Sign in button goes beside them.
#[derive(Debug, PartialEq)]
pub struct Failure {
    pub text: String,
    pub sign_in: bool,
}

/// Claude Code's line for "nobody is signed in", in the app's words: for the user's own account, and for a chat that
/// runs on the second one.
const CLAUDE_SIGNED_OUT: &str = "Claude is not signed in. Sign in, then send your message again.";
const CLAUDE2_SIGNED_OUT: &str = "Claude's second account is not signed in. Sign in, then send your message again.";

/// A failed turn's words as the chat shows them. Only Claude Code's plain "Not logged in" becomes the app's own line.
/// Its other lines that end in "Please run /login" (a key that is not valid, an account without access, a gateway's
/// refusal) keep the tool's words, which say why; the button goes beside them only when the last status was an
/// account's, not a key's or a gateway's (`by_key`): the app's sign-in is an account's, and would change how the
/// user's own Claude Code signs in. Cursor's and Gemini's signed-out lines are already the app's
/// (`cursor::SIGNED_OUT`, `antigravity::SIGNED_OUT`). Where no sign-in is offered, nothing changes.
fn failure_of(ai: &str, said: &str, by_key: bool, offered: bool) -> Failure {
    let as_said = |sign_in: bool| Failure { text: said.to_string(), sign_in };
    if !offered {
        return as_said(false);
    }
    let low = said.trim().to_lowercase();
    match tool(ai) {
        "claude" if low.starts_with("not logged in") => Failure { text: if ai == "claude2" { CLAUDE2_SIGNED_OUT } else { CLAUDE_SIGNED_OUT }.to_string(), sign_in: true },
        "claude" if low.contains("please run /login") => as_said(!by_key),
        "cursor" => as_said(said == crate::cursor::SIGNED_OUT),
        "gemini" => as_said(said == antigravity::SIGNED_OUT),
        _ => as_said(false),
    }
}

/// See `failure_of`, with what the tool last said about its sign-in.
pub fn failure(ai: &str, said: &str) -> Failure {
    let by_key = with(ai, |e| e.by_key).unwrap_or(false);
    failure_of(ai, said, by_key, offered(ai))
}

/// A turn of this AI failed as signed out: the tool is asked again now, so the page says so too.
pub fn turn_failed(ai: &str) {
    check(ai, true);
}

// ------------------------------------------------------------------ signing in

/// The requests agy's sign-in takes over ACP, one JSON line each.
fn acp_initialize() -> String {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}}}).to_string()
}

/// agy's own name for signing in with a Google account, among the methods it lists (`authMethods`).
const AGY_METHOD: &str = "oauth-personal";

/// What a line from `agy acp` means for the sign-in under way.
#[derive(Debug, PartialEq)]
enum Acp {
    /// its answer to `initialize` offers the Google sign-in: ask for it (the line to send)
    Authenticate(String),
    /// the sign-in is over: done, or why not
    Ended(Result<(), String>),
    /// nothing for the sign-in (a notification, a request of agy's own, something else)
    Other,
}

fn acp_line(line: &str) -> Acp {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return Acp::Other };
    // a request or a notification from agy is not an answer, whatever id it carries
    if !v["method"].is_null() {
        return Acp::Other;
    }
    let failed = |v: &Value| v["error"]["data"]["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or("agy gave no reason").chars().take(200).collect::<String>();
    match v["id"].as_i64() {
        Some(1) if v["error"].is_object() => Acp::Ended(Err(failed(&v))),
        Some(1) => {
            let offers = v["result"]["authMethods"].as_array().is_some_and(|m| m.iter().any(|x| x["id"] == AGY_METHOD));
            if offers {
                Acp::Authenticate(json!({"jsonrpc": "2.0", "id": 2, "method": "authenticate", "params": {"methodId": AGY_METHOD}}).to_string())
            } else {
                Acp::Ended(Err("This version of agy does not offer its Google sign-in to other programs. Run agy once in a terminal to sign in.".to_string()))
            }
        }
        Some(2) if v["error"].is_object() => Acp::Ended(Err(failed(&v))),
        Some(2) => Acp::Ended(Ok(())),
        _ => Acp::Other,
    }
}

/// A line a sign-in printed, as a reason to show when it did not finish: only a line that says something went
/// wrong, plain, and never a link (its query is a secret of that sign-in). What the tools print while all is well
/// ("Opening browser to sign in…", "Waiting for browser authentication...", the prompt for a pasted code) is no reason.
fn reason(line: &str) -> Option<String> {
    let plain = plain(line);
    let low = plain.to_lowercase();
    let wrong = ["error", "fail", "denied", "expired", "invalid", "could not", "cannot", "unable"].iter().any(|w| low.contains(w));
    (wrong && !low.contains("http://") && !low.contains("https://")).then(|| plain.chars().take(200).collect())
}

/// The tool's own sign-in, as a command. Tests get the stand-in program, or none.
fn login_command(ai: &str) -> Result<Command, String> {
    #[cfg(test)]
    return match STAND_IN.lock().unwrap().as_ref().and_then(|s| s.login.clone()) {
        Some(script) => {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg(script).env("FRIDAE_SIGNIN_AI", ai);
            Ok(cmd)
        }
        None => Err("Tests start no sign-in.".to_string()),
    };
    #[allow(unreachable_code)]
    {
        let exe = paths::find_ai(tool(ai)).ok_or_else(|| format!("{} is not installed.", crate::expert_ai::ai_name(tool(ai))))?;
        let mut cmd = Command::new(exe);
        cmd.args(match tool(ai) {
            "claude" => &["auth", "login"][..],
            "cursor" => &["login"][..],
            _ => &["acp"][..],
        });
        if !tool_env(&mut cmd, ai) {
            return Err("The second account's folder is not there. Remove it in Setup and add it again.".to_string());
        }
        Ok(cmd)
    }
}

/// What the button says instead of starting, when the tool does not say plainly that nobody is signed in: a sign-in
/// that is there is the user's own, also outside Fridae, and finishing a new one replaces it.
fn confirm_line(ai: &str, known: &Known) -> String {
    let name = name(ai);
    // a second account's sign-in is the app's own: it is replaced here only
    let also = if SECOND.contains(&ai) { "" } else { ", also outside Fridae" };
    match known {
        Known::In(Some(who)) => format!("{name} says it is signed in as {who}. Signing in again replaces that sign-in{also}. Press again to go on."),
        Known::In(None) => format!("{name} says it is signed in. Signing in again replaces that sign-in{also}. Press again to go on."),
        _ => format!("Fridae could not tell whether {name} is signed in. Signing in replaces a sign-in that is there{also}. Press again to go on."),
    }
}

/// Start the tool's own sign-in. The tool opens the browser; the app waits for the tool to end, then asks who is
/// signed in. Answers where it stands (`login_state`), or why it did not start.
///
/// It first asks the tool who is signed in, and starts at once only when the answer is "nobody". Otherwise it
/// answers `confirm` with a line that says what a new sign-in does, and starts only when pressed again (`sure`): the
/// button beside a failed message is there while the tool may still say "signed in". A press while a sign-in waits
/// ends that one and starts afresh, so a page that was closed opens again (two clicks in quick succession are one).
pub fn login_start(ai: &str, sure: bool) -> Value {
    let Some(ai) = ids().find(|a| *a == ai).filter(|a| offered(a)) else { return json!({"ok": false, "error": "Fridae cannot start that sign-in here."}) };
    match with(ai, |e| (e.login.state == "waiting").then(|| e.login.since.map(|t| t.elapsed()).unwrap_or_default())).flatten() {
        Some(age) if age < SAME_CLICK => return login_state(ai),
        Some(_) => end_login(ai),
        None => {}
    }
    if !sure {
        let answer = ask(ai);
        let known = answer.known.clone();
        with(ai, |e| {
            e.known = answer.known;
            e.by_key = answer.by_key;
            e.asked = Some(Instant::now());
        });
        if known != Known::Out {
            let mut v = login_state(ai);
            v["ok"] = json!(false);
            v["confirm"] = json!(true);
            v["error"] = json!(confirm_line(ai, &known));
            return v;
        }
    }
    let mut child = {
        // checked and started under one lock, so two presses that both got this far start one sign-in
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let e = g.get_or_insert_with(HashMap::new).entry(ai).or_default();
        if e.login.state == "waiting" {
            drop(g);
            return login_state(ai);
        }
        let mut cmd = match login_command(ai) {
            Ok(c) => c,
            Err(why) => return json!({"ok": false, "error": why}),
        };
        // agy takes its requests on its input; the others are given none (nothing is typed into them)
        cmd.stdin(if ai == "gemini" { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
        paths::quiet(&mut cmd);
        paths::own_group(&mut cmd);
        let child = match cmd.start() {
            Ok(c) => c,
            Err(err) => return json!({"ok": false, "error": format!("Could not start {}: {err}", crate::expert_ai::ai_name(tool(ai)))}),
        };
        procs::record("signin", child.id());
        e.login = Login { pid: Some(child.id()), state: "waiting", message: String::new(), since: Some(Instant::now()) };
        child
    };
    let pid = child.id();
    // what it says on stderr is read (a full pipe would stop it) and dropped
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut BufReader::new(err), &mut std::io::sink());
        });
    }
    // nobody finished it in the browser: ended, so no tool waits for ever
    std::thread::spawn(move || {
        std::thread::sleep(LOGIN_WAIT);
        if with(ai, |e| e.login.pid == Some(pid) && e.login.state == "waiting") == Some(true) {
            login_cancel(ai);
        }
    });
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut last = String::new();
        // agy: whether its sign-in ended well, from its answer (None: it ended without one)
        let mut answered: Option<Result<(), String>> = None;
        if ai == "gemini" && let Some(input) = stdin.as_mut() {
            let _ = writeln!(input, "{}", acp_initialize());
            let _ = input.flush();
        }
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if ai == "gemini" {
                    match acp_line(&line) {
                        Acp::Authenticate(request) => {
                            if let Some(input) = stdin.as_mut() {
                                let _ = writeln!(input, "{request}");
                                let _ = input.flush();
                            }
                        }
                        Acp::Ended(how) => {
                            answered = Some(how);
                            break;
                        }
                        Acp::Other => {}
                    }
                } else if let Some(r) = reason(&line) {
                    last = r;
                }
            }
        }
        drop(stdin);
        if ai == "gemini" {
            paths::end_trees(&[pid]); // the ACP server has done its one job
        }
        let exited = child.wait().is_ok_and(|s| s.success());
        procs::forget(pid);
        let (ok, why) = match (ai, answered) {
            ("gemini", Some(Ok(()))) => (true, String::new()),
            ("gemini", Some(Err(why))) => (false, why),
            ("gemini", None) => (false, String::new()),
            _ => (exited, last),
        };
        // only the sign-in this thread reads: one that was cancelled, or replaced by a newer one, is not touched
        let mine = with(ai, |e| {
            let mine = e.login.pid == Some(pid);
            if mine {
                e.login.pid = None;
                if e.login.state == "waiting" {
                    e.login.state = if ok { "done" } else { "failed" };
                    e.login.message = if ok { String::new() } else { why };
                    // whatever the tool said at its end, the answer that counts is who is signed in now
                    e.known = Known::Unknown;
                }
            }
            mine
        });
        if mine == Some(true) {
            check(ai, true);
        }
    });
    login_state(ai)
}

/// End the sign-in that waits, if one does, and its process.
fn end_login(ai: &str) {
    let pid = with(ai, |e| {
        if e.login.state == "waiting" {
            e.login.state = "";
        }
        e.login.pid.take()
    })
    .flatten();
    if let Some(pid) = pid {
        paths::end_trees(&[pid]);
        procs::forget(pid);
    }
}

/// End a sign-in that is still waiting for the browser (Cancel, or nobody finished it in time), then ask the tool
/// who is signed in: it may have been finished in the browser a moment before.
pub fn login_cancel(ai: &str) {
    end_login(ai);
    check(ai, true);
}

/// When the app quits: no sign-in is left waiting.
pub fn shut_down() {
    for ai in ids() {
        end_login(ai);
    }
}

fn state_of(e: &Entry, offered: bool, kept_key: bool) -> Value {
    let (state, who) = match &e.known {
        Known::In(who) => ("in", who.clone()),
        Known::Out => ("out", None),
        Known::Unknown => ("unknown", None),
    };
    json!({"offered": offered, "state": state, "who": who, "kept_key": kept_key, "login": {"state": e.login.state, "message": e.login.message}})
}

/// Whether a key for this AI is set on this computer and kept from it (the Setup switch "Use API keys from my
/// environment" is off): then "not signed in" may only want that switch.
fn kept_key(ai: &str) -> bool {
    offered(ai) && !paths::env_keys_on() && !paths::env_keys_present(tool(ai)).is_empty()
}

/// Where one AI's sign-in stands, for the page.
pub fn login_state(ai: &str) -> Value {
    let mut v = with(ai, |e| state_of(e, offered(ai), kept_key(ai))).unwrap_or_else(|| json!({"offered": false}));
    v["ok"] = json!(true);
    v
}

/// Where each AI's sign-in stands, for the page's status: `offered`, `state` ("in", "out", "unknown"), `who` (the
/// account, when the tool names it), `kept_key` and `login` (the sign-in started here: its `state` and, failed, its
/// `message`).
pub fn state() -> Value {
    let mut out = serde_json::Map::new();
    for ai in ids() {
        out.insert(ai.to_string(), with(ai, |e| state_of(e, offered(ai), kept_key(ai))).unwrap_or(Value::Null));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // What the tools printed on 23 November 2026 (Claude Code 2.1.319, cursor-agent 2026.11.22-e9042b2, agy 1.5.15),
    // signed in for real and, in a scratch home, signed in to nothing. The account, names and ids are replaced.
    const CLAUDE_IN: &str = r#"{
  "loggedIn": true,
  "authMethod": "claude.ai",
  "apiProvider": "firstParty",
  "analyticsDisabled": false,
  "projectsDirectory": "/Users/someone/.claude/projects",
  "configDirectory": "/Users/someone/.claude",
  "loginExpiresAt": "2026-12-19T10:06:36.575Z",
  "email": "someone@example.com",
  "orgId": "00000000-0000-0000-0000-000000000000",
  "orgName": "someone@example.com's Organization",
  "subscriptionType": "max",
  "storedClaudeAiLogin": true,
  "planTier": "max_20x"
}
"#;
    const CLAUDE_OUT: &str = r#"{
  "loggedIn": false,
  "authMethod": "none",
  "apiProvider": "firstParty",
  "analyticsDisabled": false,
  "projectsDirectory": "/tmp/scratch/home/.claude/projects",
  "configDirectory": "/tmp/scratch/home/.claude",
  "loginExpiresAt": null
}
"#;
    const CURSOR_IN: &str = r#"{
  "status": "authenticated",
  "isAuthenticated": true,
  "hasAccessToken": true,
  "hasRefreshToken": true,
  "userInfo": {
    "email": "someone@example.com",
    "userId": 1,
    "firstName": "Some",
    "lastName": "One",
    "createdAt": "2026-03-02T06:55:57.047Z"
  }
}
"#;
    const CURSOR_OUT: &str = r#"{
  "status": "unauthenticated",
  "isAuthenticated": false,
  "hasAccessToken": false,
  "hasRefreshToken": false,
  "message": "Not logged in"
}
"#;
    const AGY_IN: &str = "gemini-4-neon-high\tGemini 4 Neon (High)\ngemini-4-neon-xhigh\tGemini 4 Neon (XHigh)\ngemini-4-carbon-medium\tGemini 4 Carbon (Medium)\n";
    const AGY_OUT_ERR: &str = "Fetching available models...\nError: Please sign in to view available models. Launch the CLI without arguments to sign in.\n";

    #[test]
    fn each_tools_own_answer_says_who_is_signed_in() {
        assert_eq!(parse_claude(CLAUDE_IN), Answer { known: Known::In(Some("someone@example.com".into())), by_key: false });
        assert_eq!(parse_claude(CLAUDE_OUT).known, Known::Out, "its exit code is 1 then; the answer is read all the same");
        assert_eq!(parse_cursor(CURSOR_IN).known, Known::In(Some("someone@example.com".into())));
        assert_eq!(parse_cursor(CURSOR_OUT).known, Known::Out);
        assert_eq!(parse_agy(AGY_IN, "Fetching available models...\n", true).known, Known::In(None), "agy does not say as whom");
        assert_eq!(parse_agy("", AGY_OUT_ERR, false).known, Known::Out);
        // no answer the app can read is "not known", never "signed out": the Sign in button is not shown on a guess
        for nothing in ["", "not json", "{}", r#"{"loggedIn": "yes"}"#, "Error: connect ETIMEDOUT"] {
            assert_eq!(parse_claude(nothing).known, Known::Unknown, "{nothing}");
            assert_eq!(parse_cursor(nothing).known, Known::Unknown, "{nothing}");
        }
        assert_eq!(parse_agy("", "Fetching available models...\nError: could not reach the server\n", false).known, Known::Unknown);
        assert_eq!(parse_agy("", "", true).known, Known::Unknown, "ended well with no list");
        assert_eq!(parse_agy(AGY_IN, AGY_OUT_ERR, false).known, Known::Out);
        // a key or a gateway reads as signed in, with no account, and is told apart (the reviewer's measurements of
        // Claude Code 2.1.319: `authMethod` with ANTHROPIC_API_KEY, with a base URL and token, with a key helper,
        // with Bedrock)
        for method in ["api_key", "oauth_token", "api_key_helper", "third_party"] {
            assert_eq!(parse_claude(&json!({"loggedIn": true, "authMethod": method}).to_string()), Answer { known: Known::In(None), by_key: true }, "{method}");
        }
        assert!(!parse_claude(r#"{"loggedIn": false, "authMethod": "none"}"#).by_key);
        // an account is shown as text: nothing that would not show as itself, and not without end
        let odd = parse_cursor(&json!({"isAuthenticated": true, "userInfo": {"email": format!("a\u{202e}b{}@x.io", "c".repeat(200))}}).to_string());
        assert!(matches!(&odd.known, Known::In(Some(w)) if w.chars().count() == 80 && !w.contains('\u{202e}')), "{odd:?}");
    }

    #[test]
    fn a_tool_is_asked_again_only_when_its_answer_is_old() {
        let mins = |m: u64| Some(Duration::from_secs(m * 60));
        assert!(due(&Known::Unknown, None, false, false), "never asked: now");
        // never on a timer faster than a few minutes
        assert!(!due(&Known::Out, mins(4), false, false) && due(&Known::Out, mins(5), false, false));
        assert!(!due(&Known::Unknown, mins(4), false, false) && due(&Known::Unknown, mins(5), false, false));
        assert!(!due(&Known::In(None), mins(29), false, false) && due(&Known::In(None), mins(30), false, false));
        // not twice at once, and not while a sign-in waits for the browser (its end asks)
        assert!(!due(&Known::Out, mins(60), true, false) && !due(&Known::Out, mins(60), false, true));
        assert!(!due(&Known::Unknown, None, true, false));
    }

    #[test]
    fn an_answer_that_was_on_its_way_when_a_sign_in_ended_is_not_taken() {
        // asked at start; the user signs in meanwhile; the sign-in's end asks again while the first answer is out
        let mut e = Entry { asking: true, again: true, known: Known::Unknown, ..Default::default() };
        assert!(take_answer(&mut e, Answer { known: Known::Out, by_key: false }), "the old answer is dropped and the tool asked once more");
        assert_eq!((e.known.clone(), e.asking, e.again), (Known::Unknown, true, false));
        assert!(!take_answer(&mut e, Answer { known: Known::In(None), by_key: true }));
        assert_eq!((e.known.clone(), e.asking, e.by_key), (Known::In(None), false, true));
    }

    #[test]
    fn a_turn_that_failed_says_not_signed_in_only_when_that_is_what_the_tool_said() {
        let app = |text: &str| Failure { text: text.to_string(), sign_in: true };
        let as_said = |text: &str, sign_in: bool| Failure { text: text.to_string(), sign_in };
        // Claude Code's own words, as a signed-out turn in the app ended on 23 November 2026: the app's line, with the button
        assert_eq!(failure_of("claude", "Not logged in \u{b7} Please run /login", false, true), app("Claude is not signed in. Sign in, then send your message again."));
        assert_eq!(failure_of("claude", "Not logged in", false, true), app("Claude is not signed in. Sign in, then send your message again."));
        // its other lines that end the same way mean other things: the tool's words stay, since they say why
        let key = "Invalid API key \u{b7} Please run /login";
        assert_eq!(failure_of("claude", key, false, true), as_said(key, true), "an account's sign-in: the button may help");
        assert_eq!(failure_of("claude", key, true, true), as_said(key, false), "a key or a gateway: the app's sign-in is not the way, so no button");
        let access = "Your account does not have access to Claude Code. Please run /login";
        assert_eq!(failure_of("claude", access, true, true), as_said(access, false));
        assert_eq!(failure_of("claude", "You've hit your usage limit", false, true), as_said("You've hit your usage limit", false));
        // Cursor's and Gemini's are the app's own lines already: the same line, now with the button
        assert_eq!(failure_of("cursor", crate::cursor::SIGNED_OUT, false, true), app(crate::cursor::SIGNED_OUT));
        assert_eq!(failure_of("gemini", antigravity::SIGNED_OUT, false, true), app(antigravity::SIGNED_OUT));
        assert_eq!(failure_of("cursor", "Cursor could not run this message.", false, true), as_said("Cursor could not run this message.", false));
        // a chat on Claude's second account: the same words mean that account, and the button signs that one in
        assert_eq!(failure_of("claude2", "Not logged in \u{b7} Please run /login", false, true), app("Claude's second account is not signed in. Sign in, then send your message again."));
        assert_eq!(confirm_line("claude2", &Known::In(None)), "Claude's second account says it is signed in. Signing in again replaces that sign-in. Press again to go on.");
        assert_eq!(confirm_line("claude", &Known::In(None)), "Claude says it is signed in. Signing in again replaces that sign-in, also outside Fridae. Press again to go on.");
        // OpenCode has its own sign-in and its own lines; ChatGPT is not offered; nor is anything on Windows
        assert_eq!(failure_of("opencode", "Not logged in", false, false), as_said("Not logged in", false));
        assert_eq!(failure_of("claude", "Not logged in \u{b7} Please run /login", false, false), as_said("Not logged in \u{b7} Please run /login", false));
        if !cfg!(windows) {
            // each of the app's lines is one the page can put a button beside: short, and it never sends the user to a terminal
            for line in [CLAUDE_SIGNED_OUT, crate::cursor::SIGNED_OUT, antigravity::SIGNED_OUT] {
                assert!(!line.contains("terminal") && !line.contains('`') && line.matches(". ").count() == 1, "{line}");
            }
        }
    }

    #[test]
    fn agys_sign_in_is_asked_for_over_acp_and_its_answer_ends_it() {
        // agy 1.5.15's answer to `initialize` in a scratch home (measured; the capabilities left out)
        let init = r#"{"jsonrpc":"2.0","result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true},"agentInfo":{"name":"Antigravity ACP Server","version":"1.5.15"},"authMethods":[{"id":"oauth-personal","name":"Log in with Google","description":"Log in with your Google account"},{"id":"oauth-business","name":"Log in with Gemini Enterprise","description":"Log in with your Gemini Enterprise account"},{"id":"gemini-api-key","name":"Gemini API key","description":"Use an API key with Gemini Developer API"},{"id":"agent-platform","name":"Gemini Enterprise Agent Platform","description":"Use Gemini Enterprise Agent Platform (formerly Vertex AI) with Application Default Credentials"}]},"id":1}"#;
        let Acp::Authenticate(request) = acp_line(init) else { panic!("the Google sign-in is offered") };
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!((request["method"].as_str(), request["params"]["methodId"].as_str(), request["id"].as_i64()), (Some("authenticate"), Some("oauth-personal"), Some(2)));
        // the two requests are the only ones the app ever sends agy for a sign-in
        assert_eq!(serde_json::from_str::<Value>(&acp_initialize()).unwrap()["method"], "initialize");
        // its answer to that request ends the sign-in: well, or with its reason (stand-ins: no sign-in was completed)
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","result":{},"id":2}"#), Acp::Ended(Ok(())));
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Authentication required"},"id":2}"#), Acp::Ended(Err("Authentication required".into())));
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error","data":{"message":"the sign-in was cancelled"}},"id":2}"#), Acp::Ended(Err("the sign-in was cancelled".into())));
        // an agy that lists no Google sign-in: said, with the way that still works
        let Acp::Ended(Err(why)) = acp_line(r#"{"jsonrpc":"2.0","result":{"protocolVersion":1,"authMethods":[{"id":"gemini-api-key"}]},"id":1}"#) else { panic!() };
        assert!(why.contains("Run agy once in a terminal"), "{why}");
        // a request of agy's own with the same id is not an answer, and anything else is not about the sign-in
        for other in [
            r#"{"jsonrpc":"2.0","id":1,"method":"fs/read_text_file","params":{"path":"/x"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"session/request_permission","params":{}}"#,
            "",
            "not json",
            r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#,
            r#"{"jsonrpc":"2.0","result":{},"id":7}"#,
        ] {
            assert_eq!(acp_line(other), Acp::Other, "{other}");
        }
    }

    #[test]
    fn what_a_sign_in_printed_is_a_reason_only_when_it_says_what_went_wrong() {
        // stdout of `claude auth login` and `cursor-agent login`, started in a scratch home on 23 November 2026 and
        // ended before anyone signed in (the links' queries left out)
        let claude = ["Opening browser to sign in\u{2026}", "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=x", "Paste code here if prompted > "];
        let cursor = ["Starting login process...", "Authenticating with Cursor...", "Waiting for browser authentication...", "If your browser didn't open, use this link:", "https://cursor.com/loginDeepControl?challenge=x&uuid=y", ""];
        for line in claude.iter().chain(&cursor) {
            assert_eq!(reason(line), None, "{line}: what a sign-in says while all is well is no reason for its failing");
        }
        // stand-ins (no sign-in was seen to fail): a line that says what went wrong is kept, a link never
        assert_eq!(reason("  Error: the sign-in was denied\u{202e}  ").as_deref(), Some("Error: the sign-in was denied"));
        assert_eq!(reason("Login failed. Try again at https://cursor.com/login?x=1"), None);
        assert_eq!(reason("   "), None);
    }

    #[test]
    fn the_page_is_told_where_each_sign_in_stands() {
        let e = Entry { known: Known::In(Some("someone@example.com".into())), ..Default::default() };
        assert_eq!(state_of(&e, true, false), json!({"offered": true, "state": "in", "who": "someone@example.com", "kept_key": false, "login": {"state": "", "message": ""}}));
        let e = Entry { known: Known::Out, login: Login { pid: Some(1), state: "waiting", message: String::new(), since: None }, ..Default::default() };
        assert_eq!(state_of(&e, true, true), json!({"offered": true, "state": "out", "who": null, "kept_key": true, "login": {"state": "waiting", "message": ""}}));
        assert_eq!(state_of(&Entry::default(), false, false)["state"], "unknown");
        // every one of the three is in the status, and nothing else
        let all = state();
        assert_eq!(all.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), ["claude", "claude2", "cursor", "gemini"]);
        assert!(offered("claude") == !cfg!(windows) && !offered("opencode") && !offered("chatgpt"));
        // a second account's sign-in is offered only once that account exists, and is Claude's own tool
        assert!(!offered("claude2") && all["claude2"]["offered"] == false && tool("claude2") == "claude" && tool("cursor") == "cursor");
        assert_eq!(login_start("claude2", true)["ok"], false, "no second account: nothing to sign in");
        // a sign-in for anything else is never started
        assert_eq!(login_start("opencode", true)["ok"], false);
        assert_eq!(login_start("../x", true)["ok"], false);
    }

    /// A stand-in tool for the tests below, and the state as a fresh app has it.
    #[cfg(unix)]
    fn stand_in(status: Known, login: Option<&str>) {
        *STAND_IN.lock().unwrap() = Some(StandIn { status: ids().map(|a| (a, Answer { known: status.clone(), by_key: false })).collect(), login: login.map(str::to_string) });
        *STATE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[cfg(unix)]
    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < until, "never: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        !paths::leader_exited(pid, Duration::ZERO)
    }

    #[cfg(unix)]
    #[test]
    fn a_sign_in_starts_once_is_ended_by_cancel_and_leaves_nothing_behind() {
        let _one = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // the tool says nobody is signed in: one press starts its sign-in (a stand-in that waits, as the tools do)
        stand_in(Known::Out, Some("echo 'Waiting for browser authentication...'; sleep 60"));
        let first = login_start("cursor", false);
        assert_eq!((first["ok"].as_bool(), first["login"]["state"].as_str()), (Some(true), Some("waiting")), "{first}");
        let pid = with("cursor", |e| e.login.pid).flatten().expect("a process");
        assert!(alive(pid) && procs::recorded(pid), "running, and in the ledger for a crash to find");
        // a second click straight away is the same click: no second process
        assert_eq!(login_start("cursor", false)["login"]["state"], "waiting");
        assert_eq!(with("cursor", |e| e.login.pid).flatten(), Some(pid));
        // Cancel ends it and clears the ledger, and the tool is asked again who is signed in
        with("cursor", |e| e.known = Known::Unknown);
        login_cancel("cursor");
        assert!(!alive(pid) && !procs::recorded(pid));
        assert_eq!(login_state("cursor")["login"]["state"], "");
        wait_for("the status asked again after Cancel", || with("cursor", |e| e.known == Known::Out) == Some(true));
        // a press long after one began waiting (its page was closed) ends that one and starts afresh
        login_start("claude", false);
        let old = with("claude", |e| e.login.pid).flatten().unwrap();
        with("claude", |e| e.login.since = Some(Instant::now() - Duration::from_secs(60)));
        login_start("claude", false);
        let new = with("claude", |e| e.login.pid).flatten().unwrap();
        assert!(old != new && !alive(old) && alive(new), "the old one is ended, a new one waits");
        // the old one's late end does not touch the newer one
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!((with("claude", |e| e.login.pid).flatten(), login_state("claude")["login"]["state"].as_str()), (Some(new), Some("waiting")));
        // quitting leaves none
        shut_down();
        assert!(!alive(new) && !procs::recorded(new));
        *STAND_IN.lock().unwrap() = None;
    }

    #[cfg(unix)]
    #[test]
    fn a_sign_in_is_never_started_over_one_that_is_there_without_a_second_press() {
        let _one = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // the page's last answer was "signed out" (five minutes old, say), the user has signed in from a terminal since
        stand_in(Known::In(Some("someone@example.com".into())), Some("sleep 60"));
        with("claude", |e| e.known = Known::Out);
        let asked = login_start("claude", false);
        assert_eq!((asked["ok"].as_bool(), asked["confirm"].as_bool(), asked["login"]["state"].as_str()), (Some(false), Some(true), Some("")), "{asked}");
        assert_eq!(asked["error"], "Claude says it is signed in as someone@example.com. Signing in again replaces that sign-in, also outside Fridae. Press again to go on.");
        assert_eq!(with("claude", |e| e.login.pid).flatten(), None, "nothing was started");
        assert_eq!(asked["state"], "in", "and the page has the fresh answer");
        // the tool gives no answer the app can read: the same care
        stand_in(Known::Unknown, Some("sleep 60"));
        let asked = login_start("gemini", false);
        assert_eq!((asked["confirm"].as_bool(), with("gemini", |e| e.login.pid).flatten()), (Some(true), None));
        assert!(asked["error"].as_str().unwrap().starts_with("Fridae could not tell whether Gemini is signed in."), "{asked}");
        // pressed again: the user said so, and it starts
        let sure = login_start("gemini", true);
        assert_eq!((sure["ok"].as_bool(), sure["login"]["state"].as_str()), (Some(true), Some("waiting")));
        shut_down();
        // a sign-in that ends: well (the tool is asked, and says who), or not (said, with its own reason when it gave one)
        stand_in(Known::In(Some("someone@example.com".into())), Some("echo 'Opening browser to sign in'; exit 0"));
        login_start("claude", true);
        wait_for("the sign-in's end", || login_state("claude")["login"]["state"] == "done");
        wait_for("the status after it", || login_state("claude")["state"] == "in");
        assert_eq!(login_state("claude")["who"], "someone@example.com");
        stand_in(Known::Out, Some("echo 'Error: the sign-in was denied'; exit 1"));
        login_start("cursor", true);
        wait_for("the failed sign-in's end", || login_state("cursor")["login"]["state"] == "failed");
        assert_eq!(login_state("cursor")["login"]["message"], "Error: the sign-in was denied");
        // with no stand-in a test can start no sign-in at all
        stand_in(Known::Out, None);
        assert_eq!(login_start("claude", true), json!({"ok": false, "error": "Tests start no sign-in."}));
        *STAND_IN.lock().unwrap() = None;
    }

    #[cfg(unix)]
    #[test]
    fn a_status_command_that_hangs_is_ended() {
        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "echo started; sleep 60"]);
        let began = Instant::now();
        assert_eq!(run_limited(slow, Duration::from_millis(300)), None, "no answer: not known");
        assert!(began.elapsed() < Duration::from_secs(8), "ended, not waited for");
        let mut quick = Command::new("/bin/sh");
        quick.args(["-c", "echo out; echo err >&2; exit 1"]);
        assert_eq!(run_limited(quick, Duration::from_secs(10)), Some(("out\n".to_string(), "err\n".to_string(), false)));
    }

    #[test]
    fn nothing_in_the_app_can_sign_anyone_out() {
        // every source file of the app, the page and the Python tools: the word for a tool's own "sign out" command
        // appears in no line of code, only in comments and in tests (this one, and the ones that say what agy lists)
        let word = ["log", "out"].concat();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("src")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
        files.push(root.join("..").join("ui").join("index.html"));
        files.extend(std::fs::read_dir(root.join("..")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "py")));
        assert!(files.len() > 30, "the app's sources were found: {}", files.len());
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            // test files, and the tests at the end of a file, are not the app
            if name.ends_with("_tests.rs") || name.ends_with("_replay.rs") {
                continue;
            }
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
            for (n, line) in code.lines().enumerate() {
                let t = line.trim_start();
                let comment = t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') || t.starts_with('#');
                assert!(comment || !line.to_lowercase().contains(&word), "{name}:{}: {line}", n + 1);
            }
        }
    }
}
