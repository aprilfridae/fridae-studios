//! Signing in to the AIs that run on the user's own sign-in: Claude Code, Cursor and Gemini (Antigravity).
//!
//! These three are the user's own tools: their sign-in is the one the user has outside Fridae too (Claude Code's in
//! the login keychain, Cursor's and Antigravity's under the home folder). So the app only ever asks a tool who is
//! signed in, and starts the tool's own sign-in when the user presses Sign in. It never signs anyone out: nothing
//! here runs a tool's logout, and there is no button for it. (OpenCode is different: Fridae keeps a copy of its own,
//! with a sign-in of its own, see `opencode.rs`.)
//!
//! Who is signed in (asked at start, after a sign-in, when a turn fails as signed out, and otherwise every few
//! minutes at most, `due`), with the environment a turn gets (`paths::child_env`):
//!
//!   claude auth status --json          {"loggedIn": true, "email": ...}, or {"loggedIn": false} with exit code 1
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
//! Measured on 23 November 2026 with Claude Code 2.1.319, cursor-agent 2026.11.22 and agy 1.5.15: the status
//! commands for real (signed in) and in a scratch home (signed out); the start of each sign-in in a scratch home, where
//! it cannot see the user's own sign-in, with a stand-in for the browser, ended before anything was done on the page.
//! No sign-in was completed, so how each one ends (its exit code, agy's answer) is not measured: whatever the tool
//! says at its end, the app asks for the status again and goes by that. Not offered on Windows (the lines there say
//! what they said before: sign in from a terminal).

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
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

/// How long the last answer stands before the tool is asked again by itself: one that says "signed in" for half an
/// hour, anything else for five minutes (the user may be signing in from a terminal meanwhile).
const AGAIN_SIGNED_IN: Duration = Duration::from_secs(30 * 60);
const AGAIN_OTHERWISE: Duration = Duration::from_secs(5 * 60);
/// A sign-in nobody finishes in the browser is ended after this long.
const LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);

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

/// A sign-in the user started with the button: "" none, "waiting" for the browser, "done", "failed".
#[derive(Default)]
struct Login {
    pid: Option<u32>,
    state: &'static str,
    message: String,
}

#[derive(Default)]
struct Entry {
    known: Known,
    asked: Option<Instant>,
    asking: bool,
    /// asked for again while an answer was on its way (a sign-in ended meanwhile): that answer is not taken, and the
    /// tool is asked once more
    again: bool,
    login: Login,
}

static STATE: Mutex<Option<HashMap<&'static str, Entry>>> = Mutex::new(None);

fn with<R>(ai: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    let key = AIS.iter().find(|a| **a == ai)?;
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    Some(f(g.get_or_insert_with(HashMap::new).entry(key).or_default()))
}

/// Whether the app offers to sign this AI in: one of the three, and not on Windows (not measured there: the lines
/// keep saying what they said before).
pub fn offered(ai: &str) -> bool {
    AIS.contains(&ai) && !cfg!(windows)
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

/// `claude auth status --json`.
pub fn parse_claude(stdout: &str) -> Known {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else { return Known::Unknown };
    match v["loggedIn"].as_bool() {
        Some(true) => Known::In(v["email"].as_str().and_then(account)),
        Some(false) => Known::Out,
        None => Known::Unknown,
    }
}

/// `cursor-agent status --format json`.
pub fn parse_cursor(stdout: &str) -> Known {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else { return Known::Unknown };
    match v["isAuthenticated"].as_bool() {
        Some(true) => Known::In(v["userInfo"]["email"].as_str().and_then(account)),
        Some(false) => Known::Out,
        None => Known::Unknown,
    }
}

/// `agy models`: a list means signed in (agy does not say as whom); its own "Please sign in" means signed out;
/// anything else (offline, say) is no answer.
pub fn parse_agy(stdout: &str, stderr: &str, ok: bool) -> Known {
    if ok && stdout.lines().any(|l| l.split_once('\t').is_some_and(|(id, name)| !id.trim().is_empty() && !name.trim().is_empty())) {
        Known::In(None)
    } else if !ok && stderr.to_lowercase().contains("please sign in") {
        Known::Out
    } else {
        Known::Unknown
    }
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
    let Some(ai) = AIS.iter().copied().find(|a| *a == ai) else { return };
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
            let known = ask(ai);
            if with(ai, |e| take_answer(e, known)) != Some(true) {
                break;
            }
        }
    });
}

/// An answer has come: kept, unless the tool was asked for again meanwhile (then it is asked once more: true).
fn take_answer(e: &mut Entry, known: Known) -> bool {
    if std::mem::take(&mut e.again) {
        e.asked = Some(Instant::now());
        return true;
    }
    e.known = known;
    e.asking = false;
    false
}

/// The tool's answer. Tests never run the tools, whatever is installed on the computer running them.
fn ask(ai: &str) -> Known {
    if cfg!(test) {
        return Known::Unknown;
    }
    if ai == "gemini" {
        // the model list is the question, and fills the model menu when it comes (on Windows too, where the answer
        // about the sign-in is not shown)
        return antigravity::ask_models().map(|(out, err, ok)| parse_agy(&out, &err, ok)).unwrap_or_default();
    }
    if !offered(ai) {
        return Known::Unknown;
    }
    let Some(exe) = paths::find_ai(ai) else { return Known::Unknown };
    let mut cmd = Command::new(exe);
    cmd.args(if ai == "claude" { &["auth", "status", "--json"][..] } else { &["status", "--format", "json"][..] });
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    paths::quiet(&mut cmd);
    paths::child_env(&mut cmd, ai);
    // (Claude Code ends with code 1 when nobody is signed in: its answer is read either way)
    let Ok(out) = cmd.run_output() else { return Known::Unknown };
    let text = String::from_utf8_lossy(&out.stdout);
    if ai == "claude" { parse_claude(&text) } else { parse_cursor(&text) }
}

/// At start: every tool is asked once.
pub fn start() {
    for ai in AIS {
        check(ai, true);
    }
}

/// From the page's status: each tool is asked again when its last answer is old enough (`due`).
pub fn refresh() {
    for ai in AIS {
        check(ai, false);
    }
}

// -------------------------------------------------------- a turn that failed

/// What the chat is told when a turn failed because nobody is signed in: the app's own line, for the Sign in button
/// beside it. For Claude Code that replaces the tool's own words ("Not logged in · Please run /login"); for Cursor
/// and Gemini the line is already the app's (`cursor::SIGNED_OUT`, `antigravity::SIGNED_OUT`). None when the words
/// are not about signing in, or where the app offers no sign-in (the words then stay as they are).
pub fn signed_out_line(ai: &str, said: &str) -> Option<&'static str> {
    if !offered(ai) {
        return None;
    }
    let low = said.to_lowercase();
    match ai {
        "claude" => (low.contains("not logged in") || low.contains("please run /login")).then_some("Claude is not signed in. Sign in, then send your message again."),
        "cursor" => (said == crate::cursor::SIGNED_OUT).then_some(crate::cursor::SIGNED_OUT),
        _ => (said == antigravity::SIGNED_OUT).then_some(antigravity::SIGNED_OUT),
    }
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
    /// nothing for the sign-in (a notification, something else)
    Other,
}

fn acp_line(line: &str) -> Acp {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return Acp::Other };
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

/// Start the tool's own sign-in. The tool opens the browser; the app waits for the tool to end, then asks who is
/// signed in. Answers where it stands (`login_state`), or why it could not start.
pub fn login_start(ai: &str) -> Value {
    let Some(ai) = AIS.iter().copied().find(|a| *a == ai).filter(|a| offered(a)) else { return json!({"ok": false, "error": "Fridae cannot start that sign-in here."}) };
    let Some(exe) = paths::find_ai(ai) else { return json!({"ok": false, "error": format!("{} is not installed.", crate::expert_ai::ai_name(ai))}) };
    let mut child = {
        // checked and started under one lock, so two quick clicks start one sign-in
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let e = g.get_or_insert_with(HashMap::new).entry(ai).or_default();
        if e.login.state == "waiting" {
            drop(g);
            return login_state(ai);
        }
        let mut cmd = Command::new(&exe);
        cmd.args(match ai {
            "claude" => &["auth", "login"][..],
            "cursor" => &["login"][..],
            _ => &["acp"][..],
        });
        // agy takes its requests on its input; the others are given none (nothing is typed into them)
        cmd.stdin(if ai == "gemini" { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
        paths::quiet(&mut cmd);
        paths::own_group(&mut cmd);
        paths::child_env(&mut cmd, ai);
        if ai == "gemini" && let Some(h) = antigravity::home_override() {
            cmd.env("HOME", h);
        }
        let child = match cmd.start() {
            Ok(c) => c,
            Err(err) => return json!({"ok": false, "error": format!("Could not start {}: {err}", crate::expert_ai::ai_name(ai))}),
        };
        procs::record("signin", child.id());
        e.login = Login { pid: Some(child.id()), state: "waiting", message: String::new() };
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
        let mine = with(ai, |e| {
            let mine = e.login.pid == Some(pid);
            if mine {
                e.login.pid = None;
                // (cancelled: its state was cleared already, and stays so)
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

/// End a sign-in that is still waiting for the browser (Cancel, or the app quitting).
pub fn login_cancel(ai: &str) {
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

/// When the app quits: no sign-in is left waiting.
pub fn shut_down() {
    for ai in AIS {
        login_cancel(ai);
    }
}

fn state_of(e: &Entry, offered: bool) -> Value {
    let (state, who) = match &e.known {
        Known::In(who) => ("in", who.clone()),
        Known::Out => ("out", None),
        Known::Unknown => ("unknown", None),
    };
    json!({"offered": offered, "state": state, "who": who, "login": {"state": e.login.state, "message": e.login.message}})
}

/// Where one AI's sign-in stands, for the page.
pub fn login_state(ai: &str) -> Value {
    let mut v = with(ai, |e| state_of(e, offered(ai))).unwrap_or_else(|| json!({"offered": false}));
    v["ok"] = json!(true);
    v
}

/// Where each AI's sign-in stands, for the page's status: `offered`, `state` ("in", "out", "unknown"), `who` (the
/// account, when the tool names it) and `login` (the sign-in started here: its `state` and, failed, its `message`).
pub fn state() -> Value {
    let mut out = serde_json::Map::new();
    for ai in AIS {
        out.insert(ai.to_string(), with(ai, |e| state_of(e, offered(ai))).unwrap_or(Value::Null));
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
        assert_eq!(parse_claude(CLAUDE_IN), Known::In(Some("someone@example.com".into())));
        assert_eq!(parse_claude(CLAUDE_OUT), Known::Out, "its exit code is 1 then; the answer is read all the same");
        assert_eq!(parse_cursor(CURSOR_IN), Known::In(Some("someone@example.com".into())));
        assert_eq!(parse_cursor(CURSOR_OUT), Known::Out);
        assert_eq!(parse_agy(AGY_IN, "Fetching available models...\n", true), Known::In(None), "agy does not say as whom");
        assert_eq!(parse_agy("", AGY_OUT_ERR, false), Known::Out);
        // no answer the app can read is "not known", never "signed out": the Sign in button is not shown on a guess
        for nothing in ["", "not json", "{}", r#"{"loggedIn": "yes"}"#, "Error: connect ETIMEDOUT"] {
            assert_eq!(parse_claude(nothing), Known::Unknown, "{nothing}");
            assert_eq!(parse_cursor(nothing), Known::Unknown, "{nothing}");
        }
        assert_eq!(parse_agy("", "Fetching available models...\nError: could not reach the server\n", false), Known::Unknown);
        assert_eq!(parse_agy("", "", true), Known::Unknown, "ended well with no list");
        assert_eq!(parse_agy(AGY_IN, AGY_OUT_ERR, false), Known::Out);
        // signed in with a key: no account to name
        assert_eq!(parse_claude(r#"{"loggedIn": true, "authMethod": "api_key"}"#), Known::In(None));
        // an account is shown as text: nothing that would not show as itself, and not without end
        let odd = parse_cursor(&json!({"isAuthenticated": true, "userInfo": {"email": format!("a\u{202e}b{}@x.io", "c".repeat(200))}}).to_string());
        assert!(matches!(&odd, Known::In(Some(w)) if w.chars().count() == 80 && !w.contains('\u{202e}')), "{odd:?}");
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
        assert!(take_answer(&mut e, Known::Out), "the old answer is dropped and the tool asked once more");
        assert_eq!((e.known.clone(), e.asking, e.again), (Known::Unknown, true, false));
        assert!(!take_answer(&mut e, Known::In(None)));
        assert_eq!((e.known.clone(), e.asking), (Known::In(None), false));
    }

    #[test]
    fn a_turn_that_failed_as_signed_out_says_so_in_the_apps_words() {
        if cfg!(windows) {
            return; // no sign-in is offered there: the lines stay as they were
        }
        // Claude Code's own words, as a signed-out turn in the app ended on 23 November 2026
        assert_eq!(signed_out_line("claude", "Not logged in \u{b7} Please run /login"), Some("Claude is not signed in. Sign in, then send your message again."));
        assert_eq!(signed_out_line("claude", "Invalid API key \u{b7} Please run /login"), Some("Claude is not signed in. Sign in, then send your message again."));
        assert_eq!(signed_out_line("claude", "You've hit your usage limit"), None);
        // Cursor's and Gemini's are the app's own lines already: the same line, now with the button
        assert_eq!(signed_out_line("cursor", crate::cursor::SIGNED_OUT), Some("Cursor is not signed in. Sign in, then send your message again."));
        assert_eq!(signed_out_line("gemini", antigravity::SIGNED_OUT), Some("Gemini is not signed in. Sign in, then send your message again."));
        assert_eq!(signed_out_line("cursor", "Cursor could not run this message."), None);
        // OpenCode has its own sign-in and its own lines; ChatGPT is not offered
        assert_eq!(signed_out_line("opencode", "Not logged in"), None);
        assert_eq!(signed_out_line("chatgpt", "Not logged in"), None);
        // each line is one the page can put a button beside: short, and it never sends the user to a terminal
        for line in [crate::cursor::SIGNED_OUT, antigravity::SIGNED_OUT] {
            assert!(!line.contains("terminal") && !line.contains('`') && line.matches(". ").count() == 1, "{line}");
        }
    }

    #[test]
    fn agys_sign_in_is_asked_for_over_acp_and_its_answer_ends_it() {
        // agy 1.5.15's answer to `initialize` in a scratch home (measured; the capabilities left out)
        let init = r#"{"jsonrpc":"2.0","result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"auth":{"logout":{}}},"agentInfo":{"name":"Antigravity ACP Server","version":"1.5.15"},"authMethods":[{"id":"oauth-personal","name":"Log in with Google","description":"Log in with your Google account"},{"id":"oauth-business","name":"Log in with Gemini Enterprise","description":"Log in with your Gemini Enterprise account"},{"id":"gemini-api-key","name":"Gemini API key","description":"Use an API key with Gemini Developer API"},{"id":"agent-platform","name":"Gemini Enterprise Agent Platform","description":"Use Gemini Enterprise Agent Platform (formerly Vertex AI) with Application Default Credentials"}]},"id":1}"#;
        let Acp::Authenticate(request) = acp_line(init) else { panic!("the Google sign-in is offered") };
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!((request["method"].as_str(), request["params"]["methodId"].as_str(), request["id"].as_i64()), (Some("authenticate"), Some("oauth-personal"), Some(2)));
        // the app only ever asks to sign in: never the "logout" agy also lists
        assert!(!acp_initialize().contains("logout") && !request.to_string().contains("logout"));
        // its answer to that request ends the sign-in: well, or with its reason (stand-ins: no sign-in was completed)
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","result":{},"id":2}"#), Acp::Ended(Ok(())));
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Authentication required"},"id":2}"#), Acp::Ended(Err("Authentication required".into())));
        assert_eq!(acp_line(r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error","data":{"message":"the sign-in was cancelled"}},"id":2}"#), Acp::Ended(Err("the sign-in was cancelled".into())));
        // an agy that lists no Google sign-in: said, with the way that still works
        let Acp::Ended(Err(why)) = acp_line(r#"{"jsonrpc":"2.0","result":{"protocolVersion":1,"authMethods":[{"id":"gemini-api-key"}]},"id":1}"#) else { panic!() };
        assert!(why.contains("Run agy once in a terminal"), "{why}");
        // anything else is not about the sign-in
        for other in ["", "not json", r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#, r#"{"jsonrpc":"2.0","result":{},"id":7}"#] {
            assert_eq!(acp_line(other), Acp::Other, "{other}");
        }
    }

    #[test]
    fn what_a_sign_in_printed_is_a_reason_only_when_it_is_plain_words() {
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
        assert_eq!(state_of(&e, true), json!({"offered": true, "state": "in", "who": "someone@example.com", "login": {"state": "", "message": ""}}));
        let e = Entry { known: Known::Out, login: Login { pid: Some(1), state: "waiting", message: String::new() }, ..Default::default() };
        assert_eq!(state_of(&e, true), json!({"offered": true, "state": "out", "who": null, "login": {"state": "waiting", "message": ""}}));
        assert_eq!(state_of(&Entry::default(), false)["state"], "unknown");
        // every one of the three is in the status, and nothing else
        let all = state();
        assert_eq!(all.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), ["claude", "cursor", "gemini"]);
        assert!(offered("claude") == !cfg!(windows) && !offered("opencode") && !offered("chatgpt"));
        // a sign-in for anything else is never started
        assert_eq!(login_start("opencode")["ok"], false);
        assert_eq!(login_start("../x")["ok"], false);
    }

    #[test]
    fn nothing_here_can_sign_anyone_out() {
        // the whole of this file, but for this test: no tool is ever started with "logout", in any spelling
        let source = include_str!("signin.rs");
        let code = source.split("fn nothing_here_can_sign_anyone_out").next().unwrap();
        let args: Vec<&str> = code.lines().filter(|l| l.contains("cmd.args(") || l.contains("=> &[")).collect();
        assert!(!args.is_empty());
        for line in args {
            assert!(!line.to_lowercase().contains("logout") && !line.to_lowercase().contains("signout"), "{line}");
        }
        assert!(!code.contains("\"method\": \"logout\"") && !code.contains("auth/logout"));
    }
}
