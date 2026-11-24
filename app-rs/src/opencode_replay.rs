//! Real recorded event streams of OpenCode 2.3.2 turns (`app-rs/tests/fixtures/opencode/*.sse`: the app's own calls
//! against `opencode serve`, with the scripted mock model, scrubbed) replayed through `turn_on`: the app's real calls,
//! rules check, event parsing, permission answers and turn ending. Only the server is stood in for: a small HTTP
//! server that answers each call the way OpenCode did and streams the recording as `/api/event`.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use tiny_http::{Header, Response, Server};

use super::*;

/// One call the app made: (method, path, body).
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// How the stand-in differs from what OpenCode did in the recording.
#[derive(Default, Clone)]
struct Twist {
    /// the event stream is served in these pieces, one per `/api/event` (the last again after that)
    pieces: Vec<String>,
    /// the session reads as busy for this many `/api/session/active` asks
    busy: usize,
    /// what `GET /api/session/<id>/permission` lists
    pending: Value,
    /// the session's rules read back changed from the second read on
    tamper: bool,
    /// the session's rules never read back as written
    wrong_rules: bool,
    /// what `GET /api/session` lists (the sessions in a folder)
    listed: Value,
}

/// A stand-in OpenCode serving `sse` as its event stream for session `session`. Returns its port and the calls seen.
fn stand_in(sse: String, session: String) -> (u16, Seen) {
    stand_in_with(sse, session, Twist::default())
}

fn stand_in_with(sse: String, session: String, twist: Twist) -> (u16, Seen) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut rules = Value::Null;
        let (mut reads, mut streams, mut busy) = (0, 0, twist.busy);
        for mut rq in server.incoming_requests() {
            let mut body = String::new();
            let _ = rq.as_reader().read_to_string(&mut body);
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let (method, url) = (rq.method().to_string(), rq.url().to_string());
            let path = url.split('?').next().unwrap_or("").to_string();
            log.lock().unwrap().push((method.clone(), path.clone(), body.clone()));
            let json_ok = |v: Value| Response::from_string(v.to_string()).with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
            let at = format!("/api/session/{session}");
            let r = match (method.as_str(), path.as_str()) {
                ("POST", "/api/session") => {
                    rules = body["permissions"].clone();
                    json_ok(json!({"data": {"id": session}}))
                }
                ("PATCH", p) if p == at => {
                    rules = body["permissions"].clone();
                    json_ok(json!({"data": {"id": session}}))
                }
                ("GET", p) if p == at => {
                    reads += 1;
                    let mut shown = rules.clone();
                    if (twist.tamper && reads > 1) || twist.wrong_rules {
                        shown = json!([{"action": "*", "resource": "*", "effect": "allow"}]);
                    }
                    json_ok(json!({"data": {"id": session, "permissions": shown}}))
                }
                ("GET", "/api/session") => json_ok(json!({"data": twist.listed.as_array().cloned().unwrap_or_default()})),
                ("DELETE", p) if p.starts_with("/api/session/") => json_ok(Value::Null).with_status_code(204),
                ("GET", p) if p == format!("{at}/permission") => json_ok(json!({"data": twist.pending.as_array().cloned().unwrap_or_default()})),
                ("GET", "/api/session/active") => {
                    let a = if busy > 0 { json!({session.clone(): {}}) } else { json!({}) };
                    busy = busy.saturating_sub(1);
                    json_ok(json!({"data": a}))
                }
                ("GET", "/api/event") => {
                    let piece = if twist.pieces.is_empty() { sse.clone() } else { twist.pieces[streams.min(twist.pieces.len() - 1)].clone() };
                    streams += 1;
                    Response::from_string(piece).with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap())
                }
                ("PUT", p) if p.ends_with("/environment") => json_ok(json!({})).with_status_code(204),
                ("PUT" | "DELETE", p) if p.starts_with("/api/experimental/") => json_ok(json!({"data": true})),
                ("POST", p) if p.starts_with(&at) => json_ok(json!({"data": true})),
                _ => json_ok(json!({"error": "not found"})).with_status_code(404),
            };
            let _ = rq.respond(r);
        }
    });
    (port, seen)
}

fn recording(case: &str) -> (String, String) {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("opencode").join(format!("{case}.sse"))).unwrap();
    let session = regex::Regex::new(r"ses_fixture\d+").unwrap().find(&text).unwrap().as_str().to_string();
    (text, session)
}

/// The turn's outputs (as plain strings), how it ended, and the calls the app made.
fn replay(case: &str, resume: bool) -> (Vec<String>, Ended, Vec<(String, String, Value)>) {
    replay_with(case, resume, |_| Twist::default())
}

fn replay_with(case: &str, resume: bool, twist: impl FnOnce(&str) -> Twist) -> (Vec<String>, Ended, Vec<(String, String, Value)>) {
    let (sse, session) = recording(case);
    let twist = twist(&sse);
    let (port, seen) = stand_in_with(sse, session.clone(), twist);
    let conn = Conn { port, auth: "Basic dGVzdA==".into() };
    let dir = crate::paths::test_dir("opencode-replay");
    let spec = Spec {
        agent_id: format!("replay-{case}"),
        title: "Fridae · replay".into(),
        location: dir.clone(),
        project: None,
        servers: json!({}),
        session: resume.then(|| session.clone()),
        model: Some("mock/scripted".into()),
        full: false,
        instructions: "be brief".into(),
        text: "hello".into(),
    };
    let me = Active::default();
    let mut outs = Vec::new();
    let ended = turn_on(&conn, &spec, &me, &mut |o: Out| {
        outs.push(match o {
            Out::Conversation(s) => format!("conversation {s}"),
            Out::Model(m) => format!("model {m}"),
            Out::Tool(t) | Out::ToolArgs(t, _) => format!("tool {t}"),
            Out::Text(t) => format!("text {t}"),
            Out::Pictures(p) => format!("pictures {}", p.len()),
            Out::Encoded(p) => format!("encoded {}", p.len()),
            Out::Blocked(b) => format!("blocked {}", b.iter().filter_map(|c| c["name"].as_str()).collect::<Vec<_>>().join(",")),
            Out::Refused(b) => format!("refused {}", b.iter().filter_map(|c| c["command"].as_str()).collect::<Vec<_>>().join(",")),
            Out::Error(e) => format!("error {e}"),
            Out::Failed(e) => format!("failed {e}"),
            Out::Usage(u) => format!("usage {}", u.output),
            Out::Finished => "finished".into(),
        })
    });
    let calls = seen.lock().unwrap().clone();
    (outs, ended, calls)
}

fn has(outs: &[String], start: &str) -> bool {
    outs.iter().any(|o| o.starts_with(start))
}

#[test]
fn a_plain_reply() {
    let (outs, ended, calls) = replay("plain", false);
    assert!(ended.err.is_empty() && !ended.killed);
    assert!(has(&outs, "conversation ses_fixture") && outs.contains(&"text OK: hello there".to_string()), "{outs:?}");
    assert!(has(&outs, "usage") && outs.last() == Some(&"finished".to_string()), "{outs:?}");
    // the app's calls, in order: a new session with the rules, the rules read back, the environment, the
    // instructions, the event stream, then the message
    let order: Vec<String> = calls.iter().map(|(m, p, _)| format!("{m} {}", p.split('/').take(3).collect::<Vec<_>>().join("/"))).collect();
    let pos = |s: &str| order.iter().position(|o| o.starts_with(s)).unwrap_or_else(|| panic!("{s} not in {order:?}"));
    assert!(pos("POST /api/session") < pos("GET /api/session"));
    assert!(pos("GET /api/event") < calls.iter().position(|(_, p, _)| p.ends_with("/prompt")).unwrap());
    assert!(calls.iter().any(|(m, p, b)| m == "PUT" && p.ends_with("/environment") && b["variables"].as_object().is_some_and(|v| !v.keys().any(|k| k.starts_with("OPENCODE_")))));
}

#[test]
fn a_resumed_session_gets_this_turns_rules_written_over() {
    let (outs, ended, calls) = replay("resumed", true);
    assert!(ended.err.is_empty());
    assert!(outs.contains(&"text OK: and again".to_string()), "{outs:?}");
    assert!(calls.iter().any(|(m, p, b)| m == "PATCH" && p.starts_with("/api/session/ses_fixture") && b["permissions"].is_array()));
    assert!(!calls.iter().any(|(m, p, _)| m == "POST" && p == "/api/session"), "no new session");
}

#[test]
fn a_tool_call() {
    let (outs, ended, _) = replay("tool", false);
    assert!(ended.err.is_empty());
    assert!(outs.iter().any(|o| o == "tool run_command"), "OpenCode's shell named as the app shows it: {outs:?}");
    assert!(outs.iter().any(|o| o.starts_with("text RESULT:")), "{outs:?}");
}

#[test]
fn a_refused_command_is_answered_and_shown() {
    let _held = crate::paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = crate::paths::set_allowed(&["whoami".to_string()], false);
    let (outs, ended, calls) = replay("refused", false);
    assert!(ended.err.is_empty());
    assert!(outs.contains(&"blocked whoami".to_string()), "{outs:?}");
    let reply = calls.iter().find(|(_, p, _)| p.contains("/permission/")).expect("the request was answered");
    assert_eq!(reply.2["decision"], "reject");
    assert!(reply.2["message"].as_str().unwrap().contains("whoami"));
}

#[test]
fn a_failed_turn_and_a_usage_limit() {
    let (outs, ended, _) = replay("failed", false);
    assert!(ended.err.is_empty(), "the turn ran; its end event says it failed");
    assert!(outs.iter().any(|o| o.starts_with("failed ") && o.contains("Model unavailable")), "{outs:?}");
    let (outs, _, _) = replay("limit", false);
    assert!(outs.iter().any(|o| o.starts_with("failed Free usage limit reached") && o.contains("resets in about 12 hours")), "{outs:?}");
}

/// The permission request of the "refused" recording.
fn asked_request(sse: &str) -> Value {
    let line = sse.lines().find(|l| l.contains("\"permission.asked\"")).unwrap();
    serde_json::from_str::<Value>(line.trim_start_matches("data: ")).unwrap()["data"].clone()
}

#[test]
fn an_allowed_command_runs_only_while_the_rules_read_back_as_written() {
    let _held = crate::paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = crate::paths::set_allowed(&["whoami".to_string()], true);
    // rules intact: answered "once", and the rules were read again before it and at the end
    let (_, ended, calls) = replay("refused", false);
    assert!(ended.err.is_empty() && !ended.killed);
    let reply = calls.iter().position(|(_, p, _)| p.contains("/permission/")).expect("answered");
    assert_eq!(calls[reply].2["decision"], "once");
    let reads: Vec<usize> = calls.iter().enumerate().filter(|(_, (m, p, _))| m == "GET" && p.starts_with("/api/session/ses_") && !p.contains("/permission")).map(|(i, _)| i).collect();
    assert!(reads.len() >= 3 && reads[1] < reply && *reads.last().unwrap() > reply, "{reads:?} {reply}");
    // rules changed behind the app's back: nothing is let run, OpenCode is ended, the chat says why
    let (outs, ended, calls) = replay_with("refused", false, |_| Twist { tamper: true, ..Default::default() });
    let _ = crate::paths::set_allowed(&["whoami".to_string()], false);
    assert!(ended.killed, "the server is ended");
    assert!(!calls.iter().any(|(_, p, b)| p.contains("/permission/") && b["decision"] == "once"), "never answered once");
    assert!(outs.iter().any(|o| o.starts_with("error This chat's rules in OpenCode")), "{outs:?}");
}

#[test]
fn a_program_that_reads_other_processes_environment_is_never_let_run() {
    let _held = crate::paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let allowed = vec!["ps".to_string(), "git".to_string()];
    let d = decide("shell", &["ps eww -ax".to_string()], &allowed);
    assert!(!d.once && d.blocked.is_empty(), "refused, and no card offers to allow it");
    assert!(d.message.contains("ps"));
    let d = decide("shell", &["git status | ps -E".to_string()], &allowed);
    assert!(!d.once && d.blocked.is_empty());
    // the whole command the call was given is checked too, not only what the request lists
    let mut turn = Turn::new("x", &json!({}), None);
    let s = "ses_1";
    turn.feed(s, &json!({"type": "session.tool.input.started", "data": {"sessionID": s, "id": "c1", "name": "shell"}}));
    turn.feed(s, &json!({"type": "session.tool.called", "data": {"sessionID": s, "id": "c1", "input": {"command": "git status; ./git"}}}));
    let step = turn.feed(s, &json!({"type": "permission.asked", "data": {"sessionID": s, "id": "per_1", "action": "shell", "resources": ["git status"], "source": {"type": "tool", "id": "c1"}}}));
    let ask = step.ask.unwrap();
    assert!(ask.resources.contains(&"git status; ./git".to_string()));
    assert!(!decide(&ask.action, &ask.resources, &allowed).once);
}

#[test]
fn a_request_asked_while_the_stream_was_down_is_answered_after_it_is_back() {
    let _held = crate::paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = crate::paths::set_allowed(&["whoami".to_string()], false);
    let (outs, ended, calls) = replay_with("refused", false, |sse| {
        // the stream drops just before the request; the session is still busy; the request is pending; the reopened
        // stream carries the rest
        let cut = sse.find("\"permission.asked\"").map(|i| sse[..i].rfind("data: ").unwrap()).unwrap();
        let rest: String = sse[cut..].lines().filter(|l| !l.contains("\"permission.asked\"")).collect::<Vec<_>>().join("\n") + "\n";
        Twist { pieces: vec![sse[..cut].to_string(), rest], busy: 1, pending: json!([asked_request(sse)]), ..Default::default() }
    });
    assert!(ended.err.is_empty(), "{}", ended.err);
    assert!(outs.contains(&"blocked whoami".to_string()), "{outs:?}");
    assert!(calls.iter().any(|(m, p, _)| m == "GET" && p.ends_with("/permission")), "pending requests listed");
    assert!(calls.iter().any(|(_, p, b)| p.contains("/permission/per_") && b["decision"] == "reject"), "and answered");
}

#[test]
fn tool_servers_are_removed_when_a_turn_ends_early() {
    // the message is refused (a 404 from the stand-in for an unknown prompt path is enough): the servers added are
    // removed all the same
    let (sse, session) = recording("plain");
    let (port, seen) = stand_in(sse, session);
    let conn = Conn { port, auth: "Basic dGVzdA==".into() };
    let spec = Spec {
        agent_id: "replay-early".into(),
        title: "x".into(),
        location: crate::paths::test_dir("opencode-early"),
        project: None,
        servers: json!({"rbxkit": {"command": "python3", "args": ["x.py"]}}),
        session: None,
        model: None,
        full: false,
        instructions: "x".into(),
        text: "x".into(),
    };
    let me = Active::default();
    me.cancelled.store(true, std::sync::atomic::Ordering::SeqCst); // Stop before the message: an early end
    let ended = turn_on(&conn, &spec, &me, &mut |_| {});
    assert!(ended.err.is_empty());
    let calls = seen.lock().unwrap().clone();
    let put = calls.iter().filter(|(m, p, _)| m == "PUT" && p.starts_with("/api/experimental/mcp/")).count();
    let del = calls.iter().filter(|(m, p, _)| m == "DELETE" && p.starts_with("/api/experimental/mcp/")).count();
    assert!(put > 0 && put == del, "{put} added, {del} removed");
}

type Calls = Vec<(String, String, Value)>;

/// A writer (`write_only_on`) against the stand-in serving `case`: what it gave back, and the calls it made.
fn written(case: &str, twist: impl FnOnce(&str) -> Twist, cancelled: bool) -> (Result<String, String>, crate::usage::Turn, Calls) {
    let (sse, session) = recording(case);
    let twist = twist(&sse);
    let (port, seen) = stand_in_with(sse, session, twist);
    let conn = Conn { port, auth: "Basic dGVzdA==".into() };
    let cancel = AtomicBool::new(cancelled);
    let out = write_only_on(&conn, "mock/scripted", "Summarise: the user wants a door.", &cancel, &crate::paths::test_dir("opencode-writers"));
    let calls = seen.lock().unwrap().clone();
    match out {
        Ok(w) => (Ok(w.text), w.usage, calls),
        Err(e) => (Err(e), Default::default(), calls),
    }
}

fn called(calls: &[(String, String, Value)], method: &str, end: &str) -> bool {
    calls.iter().any(|(m, p, _)| m == method && p.ends_with(end))
}

#[test]
fn a_writer_is_a_session_that_may_do_nothing_gets_the_text_once_and_is_removed() {
    let (out, used, calls) = written("plain", |_| Twist::default(), false);
    assert_eq!(out.as_deref(), Ok("OK: hello there"), "what the model wrote");
    assert!(used.total() > 0, "and what writing it used, for the usage record");
    // the session: every permission denied and nothing else, in the writers' own folder, under the writer's title
    let made = calls.iter().find(|(m, p, _)| m == "POST" && p == "/api/session").expect("a session of its own");
    assert_eq!(made.2["permissions"], json!([{"action": "*", "resource": "*", "effect": "deny"}]));
    assert_eq!(made.2["title"], WRITER_TITLE);
    assert!(made.2["location"]["directory"].as_str().is_some_and(|d| d.ends_with("opencode-writers")));
    assert_eq!(made.2["model"], json!({"providerID": "mock", "id": "scripted"}));
    // its rules are read back before anything is sent; the text goes once; then the session is removed
    let pos = |m: &str, end: &str| calls.iter().position(|(cm, p, _)| cm == m && p.ends_with(end)).unwrap_or_else(|| panic!("{m} …{end} not in {calls:?}"));
    let session = calls.iter().find(|(m, p, _)| m == "GET" && p.starts_with("/api/session/ses_fixture")).map(|(_, p, _)| p.clone()).expect("read back");
    assert!(pos("POST", "/api/session") < pos("GET", &session) && pos("GET", &session) < pos("POST", "/prompt") && pos("POST", "/prompt") < pos("DELETE", &session));
    assert_eq!(calls.iter().filter(|(_, p, _)| p.ends_with("/prompt")).count(), 1);
    assert_eq!(calls.iter().find(|(_, p, _)| p.ends_with("/prompt")).unwrap().2["text"], "Summarise: the user wants a door.");
    // nothing a chat's turn gets: no tool servers, no environment for commands, no standing instructions
    assert!(!calls.iter().any(|(m, _, _)| m == "PUT" || m == "PATCH"), "{calls:?}");
}

#[test]
fn a_writer_that_fails_or_does_anything_but_write_gives_nothing_and_is_removed_all_the_same() {
    let removed = |calls: &[(String, String, Value)]| calls.iter().any(|(m, p, _)| m == "DELETE" && p.starts_with("/api/session/ses_fixture"));
    // the model cannot be used (here: the free limit, as OpenCode really says it): the reason, for the next writer's turn
    let (out, _, calls) = written("limit", |_| Twist::default(), false);
    assert!(out.as_ref().is_err_and(|e| e.starts_with("Free usage limit reached")), "{out:?}");
    assert!(removed(&calls));
    // a model that calls a tool: ended there, never answered, nothing of what it wrote is used
    let (out, _, calls) = written("tool", |_| Twist::default(), false);
    assert_eq!(out, Err("The model tried to use a tool instead of writing.".to_string()));
    assert!(called(&calls, "POST", "/interrupt") && removed(&calls));
    // one that asks to run something: the same (nothing is ever answered "once")
    let (out, _, calls) = written("refused", |_| Twist::default(), false);
    assert_eq!(out, Err("The model tried to use a tool instead of writing.".to_string()));
    assert!(!calls.iter().any(|(_, p, _)| p.contains("/permission/")) && removed(&calls));
    // rules that do not read back as written: nothing is sent
    let (out, _, calls) = written("plain", |_| Twist { wrong_rules: true, ..Default::default() }, false);
    assert!(out.is_err_and(|e| e.contains("did not take the writer's rules")));
    assert!(!called(&calls, "POST", "/prompt") && removed(&calls));
    // given up on before it began (its time was up while the server started): nothing is sent
    let (out, _, calls) = written("plain", |_| Twist::default(), true);
    assert!(out.is_err());
    assert!(!called(&calls, "POST", "/prompt") && removed(&calls));
    // the stream ends before the answer: interrupted, removed
    let (out, _, calls) = written("plain", |sse| Twist { pieces: vec![sse[..sse.find("\"session.text.ended\"").map(|i| sse[..i].rfind("data: ").unwrap()).unwrap()].to_string()], ..Default::default() }, false);
    assert_eq!(out, Err("OpenCode's event stream ended before it had answered.".to_string()));
    assert!(called(&calls, "POST", "/interrupt") && removed(&calls));
}

#[test]
fn writers_left_behind_are_removed_by_the_next_and_nothing_else_is() {
    let left = json!([{"id": "ses_left_behind", "title": WRITER_TITLE}, {"id": "ses_not_a_writer", "title": "A chat"}]);
    let (out, _, calls) = written("plain", |_| Twist { listed: left, ..Default::default() }, false);
    assert!(out.is_ok());
    let listing = calls.iter().find(|(m, p, _)| m == "GET" && p == "/api/session").expect("the writers' folder is looked at");
    let _ = listing;
    assert!(called(&calls, "DELETE", "/api/session/ses_left_behind"));
    assert!(!called(&calls, "DELETE", "/api/session/ses_not_a_writer"));
}
