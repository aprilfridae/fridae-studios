//! End-to-end checks of the toolbox: the real HTTP server, a caller chat with a key, and (once joined) the stdio
//! bridge and each AI's launch pieces. Every test is `#[ignore]` because it needs Task 2's tools or Task 3's wiring;
//! Task 6 removes the marks. Live per-AI checks cannot run under `cargo test` (test builds start no AI).

use super::{drop_chat, keys, test_chat};
use serde_json::{Value, json};
use std::sync::Arc;

const ACCEPT: &str = "application/json, text/event-stream";

/// A tiny_http server on 127.0.0.1:0 that serves `/mcp` through `http::serve`; returns it with its URL.
fn local_server() -> (Arc<tiny_http::Server>, String) {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = server.server_addr().to_ip().unwrap().port();
    let s = server.clone();
    std::thread::spawn(move || {
        while let Ok(req) = s.recv() {
            std::thread::spawn(move || super::http::serve(req));
        }
    });
    (server, format!("http://127.0.0.1:{port}/mcp"))
}

fn client() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into()
}

/// POST one JSON-RPC message; the answer's status, session header and parsed body.
fn rpc(agent: &ureq::Agent, url: &str, key: &str, session: Option<&str>, body: Value) -> (u16, Option<String>, Value) {
    let bearer = format!("Bearer {key}");
    let mut req = agent.post(url).header("Authorization", bearer.as_str()).header("Accept", ACCEPT).header("Content-Type", "application/json");
    if let Some(s) = session {
        req = req.header("Mcp-Session-Id", s);
    }
    let mut r = req.send(body.to_string()).unwrap();
    let status = r.status().as_u16();
    let sid = r.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_string);
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, sid, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn init_message() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"1"}}})
}

/// The text of a tools/call result's first content item, parsed as JSON.
fn tool_json(answer: &Value) -> Value {
    let text = answer["result"]["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
#[ignore = "needs Task 2"]
fn toolbox_over_http_end_to_end() {
    let caller = test_chat("claude");
    let key = keys::global().prepare(&caller.id, &caller.ai(), false).raw;
    let (server, url) = local_server();
    let agent = client();
    let seen = crate::agents::BUS.after(0, 0.0).len();

    let (status, sid, init) = rpc(&agent, &url, &key, None, init_message());
    assert_eq!(status, 200);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let sid = sid.expect("an initialize answer carries a session id");

    let (status, _, list) = rpc(&agent, &url, &key, Some(&sid), json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert_eq!(status, 200);
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names.len(), 21, "{names:?}");
    for n in ["fridae_capabilities", "chat_list", "chat_launch", "project_update", "theme_set"] {
        assert!(names.contains(&n), "missing {n}");
    }

    let launch = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chat_launch","arguments":{"title":"From the toolbox","clientRequestId":"e2e-1"}}});
    let (status, _, made) = rpc(&agent, &url, &key, Some(&sid), launch);
    assert_eq!(status, 200);
    assert_ne!(made["result"]["isError"], true, "{made}");
    let made = tool_json(&made);
    let chat_id = made["chatId"].as_str().expect("chat_launch answers with chatId").to_string();

    let events = crate::agents::BUS.after(seen, 0.0);
    assert!(events.iter().any(|e| e["kind"] == "agent_new" && e["agent"] == chat_id.as_str()), "no agent_new");
    assert!(events.iter().any(|e| e["kind"] == "renamed" && e["name"] == "From the toolbox"), "no renamed");

    let (_, _, listed) = rpc(&agent, &url, &key, Some(&sid), json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"chat_list","arguments":{}}}));
    assert!(tool_json(&listed).to_string().contains(&chat_id), "chat_list lacks the new chat");

    let bearer = format!("Bearer {key}");
    let r = agent.delete(&url).header("Authorization", bearer.as_str()).header("Mcp-Session-Id", sid.as_str()).call().unwrap();
    assert_eq!(r.status().as_u16(), 204);

    server.unblock();
    if let Some(launched) = crate::agents::agents().into_iter().find(|a| a.id == chat_id) {
        drop_chat(&launched);
    }
    drop_chat(&caller);
}

#[test]
#[ignore = "needs Task 3"]
fn bridge_end_to_end() {
    // Task 3's `bridge::run(input, output, &BridgeConfig, post)` takes an HTTP poster whose types do not exist before
    // Task 3 lands, so the body is written in `bridge_end_to_end_body` below and compiled in at the join (Task 6).
    panic!("Task 6: remove the cfg(any()) on bridge_end_to_end_body and call it from here");
}

#[cfg(any())] // Task 6: delete this line once Task 3's bridge is joined
fn bridge_end_to_end_body() {
    let caller = test_chat("claude");
    let key = keys::global().prepare(&caller.id, &caller.ai(), false).raw;
    let (server, url) = local_server();
    let agent = client();
    let cfg = super::bridge::BridgeConfig { endpoint: url, authorization: format!("Bearer {key}"), exposure: None };
    // Task 3's poster: forwards each line to the server over `agent`, returns the status, headers and body.
    let post = |c: &super::bridge::BridgeConfig, h: &super::bridge::Headers, body: &str| -> Result<super::bridge::HttpAnswer, String> {
        let _ = (c, h, body, &agent);
        Err("adapt to Task 3's Headers and HttpAnswer".into())
    };
    let input = [
        init_message().to_string(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chat_list","arguments":{}}}).to_string(),
    ]
    .join("\n");
    let out: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let code = super::bridge::run(std::io::Cursor::new(input), out.clone(), &cfg, &post);
    assert_eq!(code, 0);
    let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
    let lines: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 21);
    assert_ne!(lines[2]["result"]["isError"], true);
    server.unblock();
    drop_chat(&caller);
}

#[test]
#[ignore = "needs Task 3"]
fn every_ai_gets_a_toolbox_entry() {
    // For each of claude, chatgpt, opencode, gemini, cursor the launch pieces Task 3 builds (the Claude `--mcp-config`
    // file, the Codex `-c` args, OpenCode's `servers_config`, ACP's `servers_param`) must name `fridae` and none may
    // contain the raw key. Their builders are Task 3's and are not in this branch: enabled at the join (Task 6).
    panic!("Task 6: remove the cfg(any()) on every_ai_gets_a_toolbox_entry_body and call it from here");
}

#[cfg(any())] // Task 6: delete this line once Task 3's wiring is joined
fn every_ai_gets_a_toolbox_entry_body() {
    for ai in ["claude", "chatgpt", "opencode", "gemini", "cursor"] {
        let chat = test_chat(ai);
        let wire = super::wire_for(&chat.id, ai, false).expect("a wire");
        let pieces: Vec<String> = match ai {
            "claude" => vec![/* text of the written --mcp-config file, via Task 3's builder */],
            "chatgpt" => vec![/* Codex `-c` args */],
            "opencode" => vec![/* serde_json of servers_config(..) */],
            _ => vec![/* serde_json of acp::servers_param(..) */],
        };
        assert!(!pieces.is_empty(), "{ai}: fill in Task 3's builder");
        for p in &pieces {
            assert!(p.contains(super::SERVER_NAME), "{ai}: no fridae entry");
            assert!(!p.contains(&wire.raw), "{ai}: raw key leaked");
        }
        drop_chat(&chat);
    }
}
