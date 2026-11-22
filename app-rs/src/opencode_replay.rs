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

/// A stand-in OpenCode serving `sse` as its event stream for session `session`. Returns its port and the calls seen.
fn stand_in(sse: String, session: String) -> (u16, Seen) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut rules = Value::Null;
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
                ("GET", p) if p == at => json_ok(json!({"data": {"id": session, "permissions": rules}})),
                ("GET", "/api/session/active") => json_ok(json!({"data": {}})),
                ("GET", "/api/event") => Response::from_string(sse.clone()).with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap()),
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
    let (sse, session) = recording(case);
    let (port, seen) = stand_in(sse, session.clone());
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
