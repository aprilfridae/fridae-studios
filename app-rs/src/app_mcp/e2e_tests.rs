//! End-to-end checks of the toolbox: the real HTTP server, a caller chat with a key, the stdio bridge and each AI's
//! launch pieces. Live per-AI checks cannot run under `cargo test` (test builds start no AI).

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
fn toolbox_over_http_end_to_end() {
    let _turn = super::tools::testing::serial();
    // a caller running a turn in Full auto (a write needs a live caller; Full auto raises no card)
    let caller = super::tools::testing::caller(true);
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
fn bridge_end_to_end() {
    // the stdio bridge (`Fridae --mcp-bridge`, as Gemini's kept process starts it) over the real HTTP server
    let _turn = super::tools::testing::serial();
    let caller = test_chat("claude");
    let key = keys::global().prepare(&caller.id, &caller.ai(), false).raw;
    let (server, url) = local_server();
    let cfg = super::bridge::BridgeConfig { endpoint: url, authorization: format!("Bearer {key}"), exposure: None };
    let input = [
        init_message().to_string(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chat_list","arguments":{}}}).to_string(),
    ]
    .join("
");
    let out: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let code = super::bridge::run(std::io::Cursor::new(input.into_bytes()), out.clone(), &cfg, &super::bridge::http_send);
    assert_eq!(code, 0);
    let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
    // the bridge sends each line as it comes, so answers may arrive in any order: by id
    let mut lines: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    lines.sort_by_key(|l| l["id"].as_i64());
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[1]["result"]["tools"].as_array().map(Vec::len), Some(21), "{text}");
    assert_ne!(lines[2]["result"]["isError"], true, "{}", lines[2]);
    assert!(tool_json(&lines[2]).to_string().contains(&caller.id), "chat_list names the caller");
    assert!(!text.contains(&key), "the key never comes back");
    server.unblock();
    drop_chat(&caller);
}

#[test]
fn every_ai_gets_a_toolbox_entry() {
    // what each AI's launch is handed (the Claude `--mcp-config` file, the Codex `-c` args, OpenCode's
    // servers_config, ACP's servers_param over HTTP and over the bridge) names `fridae` and never holds the raw key
    let _turn = super::tools::testing::serial();
    let work = std::path::PathBuf::from(std::env::var("TMP").unwrap_or_else(|_| std::env::temp_dir().display().to_string())).join(format!("fridae-e2e-wire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    for ai in ["claude", "chatgpt", "opencode", "gemini", "cursor"] {
        let chat = test_chat(ai);
        let raw = keys::global().prepare(&chat.id, ai, false).raw;
        let wire = super::Wire { url: "http://127.0.0.1:4567/mcp".into(), raw: raw.clone(), deferred: ai == "gemini" };
        let none = json!({});
        let pieces: Vec<String> = match ai {
            "claude" => {
                let mut servers = json!({});
                servers[super::SERVER_NAME] = crate::agents::claude_toolbox_entry(&wire);
                let file = crate::agents::write_mcp_config(&chat.id, &servers);
                let text = std::fs::read_to_string(&file).unwrap();
                let _ = std::fs::remove_file(&file);
                assert!(text.contains("${FRIDAE_MCP_KEY}"), "{ai}: the placeholder");
                vec![text]
            }
            "chatgpt" => {
                let cmd = crate::codex::command(std::path::Path::new("codex"), &work, &none, None, None, None, false, Some(&wire));
                let env: Vec<(String, String)> = cmd.get_envs().filter_map(|(k, v)| Some((k.to_string_lossy().to_string(), v?.to_string_lossy().to_string()))).collect();
                assert!(env.contains(&("FRIDAE_MCP_KEY".to_string(), raw.clone())), "{ai}: the key travels in the environment");
                let args = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>().join(" ");
                let file = std::fs::read_to_string(work.join(".codex").join("config.toml")).unwrap_or_default();
                assert!(!file.contains("mcp_servers.fridae") && !file.contains(&raw), "{ai}: nothing of the toolbox in the workspace's file");
                vec![args]
            }
            "opencode" => vec![serde_json::to_string(&crate::opencode::servers_config(&chat.id, &none, &[], Some(&wire))).unwrap()],
            _ => vec![
                serde_json::to_string(&crate::acp::servers_param(&none, Some(&wire), true)).unwrap(),
                serde_json::to_string(&crate::acp::servers_param(&none, Some(&wire), false)).unwrap(),
            ],
        };
        for p in &pieces {
            assert!(p.contains(super::SERVER_NAME), "{ai}: no fridae entry in {p}");
            if matches!(ai, "claude" | "chatgpt") {
                // a file in the app's folder and a command line: the key is only in the process's environment
                assert!(!p.contains(&raw), "{ai}: raw key leaked");
            } else {
                // bodies handed over in memory (Fridae's own OpenCode server, the ACP pipe): the key only as the
                // Authorization value
                assert!(p.contains(&format!("Bearer {raw}")), "{ai}: the key is handed over");
                assert!(!p.replace(&format!("Bearer {raw}"), "").contains(&raw), "{ai}: the key only as Authorization");
            }
        }
        drop_chat(&chat);
    }
    let _ = std::fs::remove_dir_all(&work);
}
