//! `POST/DELETE /mcp`: MCP's streamable HTTP transport, protocol 2025-06-18, stateful sessions, JSON answers only
//! (port of T3 Code's McpHttpServer.ts and Effect's McpServer.layerHttp / mcpRuntime.ts).
//!
//! Admission, in T3's order: any Origin → 403 (before credentials); a chat's key (`Authorization: Bearer`) else 401;
//! method (POST, DELETE) else 405; JSON content type else 415; Accept listing JSON and SSE else 406; body size (413);
//! then the session and protocol header, JSON parsing, batches, and the JSON-RPC method. The page's token is never
//! looked at here, and a key is never logged (only the reason it was refused, to `FRIDAE_LOG` when set).

use std::{
    collections::HashMap,
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use serde_json::{Value, json};

use super::{PROTOCOL, SERVER_NAME, ToolDef, call_tool, exposure, keys, listable};

/// The largest request body taken.
pub const MAX_BODY: usize = 4_000_000;
/// Sessions kept at most (the least recently used goes first).
const MAX_SESSIONS: usize = 512;
const SUPPORTED: [&str; 1] = [PROTOCOL];
/// Effect's codes for a protocol header it does not serve, and one that differs from the session's.
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
const HEADER_MISMATCH: i64 = -32020;

/// An answer, before it goes on the socket.
pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct Session {
    /// SHA-256 of the key that made it: a session answers that key only.
    key_hash: String,
    protocol: String,
    exposure: exposure::Mode,
    last_used: Instant,
    /// Calls under way, by JSON-RPC id (as JSON text): their cancel flags.
    in_flight: HashMap<String, Arc<AtomicBool>>,
}

static SESSIONS: Mutex<Option<HashMap<String, Session>>> = Mutex::new(None);

fn sessions<R>(f: impl FnOnce(&mut HashMap<String, Session>) -> R) -> R {
    f(SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

fn reply(status: u16, headers: Vec<(&str, String)>, body: Vec<u8>) -> Reply {
    let mut all: Vec<(String, String)> = vec![("cache-control".into(), "no-store".into())];
    all.extend(headers.into_iter().map(|(n, v)| (n.to_string(), v)));
    Reply { status, headers: all, body }
}

fn empty(status: u16) -> Reply {
    reply(status, vec![], vec![])
}

fn json_reply(status: u16, v: &Value, mut headers: Vec<(&str, String)>) -> Reply {
    headers.insert(0, ("content-type", "application/json".into()));
    reply(status, headers, v.to_string().into_bytes())
}

fn rpc_error(status: u16, id: &Value, error: Value) -> Reply {
    json_reply(status, &json!({"jsonrpc": "2.0", "id": id, "error": error}), vec![])
}

fn invalid_request(status: u16, id: &Value, message: &str) -> Reply {
    rpc_error(status, id, json!({"code": -32600, "message": message}))
}

fn unsupported_version(id: &Value, requested: &str) -> Reply {
    rpc_error(
        400,
        id,
        json!({"code": UNSUPPORTED_PROTOCOL_VERSION, "message": format!("Unsupported protocol version '{requested}'"), "data": {"supported": SUPPORTED, "requested": requested}}),
    )
}

/// The reason a key was refused, to the developer log when `FRIDAE_LOG` is set (never the key).
fn log_refusal(r: keys::Refusal) {
    if let Ok(log) = std::env::var("FRIDAE_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "mcp: rejected a request with an unusable key ({})", r.reason());
        }
    }
}

fn unauthorized(r: keys::Refusal) -> Reply {
    log_refusal(r);
    let challenge = if r.presented() { "Bearer error=\"invalid_token\"" } else { "Bearer" };
    json_reply(401, &json!({"error": "invalid_mcp_credential", "message": "A valid Fridae toolbox key is required."}), vec![("www-authenticate", challenge.into())])
}

/// A media-type list (Content-Type or Accept) has this type.
fn lists(header: &str, media: &str) -> bool {
    header.split(',').any(|part| part.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case(media))
}

/// A tool as tools/list shows it.
fn listing(def: &ToolDef) -> Value {
    let mut t = json!({
        "name": def.name, "title": def.title, "description": def.description, "inputSchema": (def.schema)(),
        "annotations": {"title": def.title, "readOnlyHint": def.hints.read_only, "destructiveHint": def.hints.destructive,
                        "idempotentHint": def.hints.idempotent, "openWorldHint": def.hints.open_world},
    });
    if def.always_load {
        t["_meta"] = json!({"anthropic/alwaysLoad": true});
    }
    t
}

/// One request to `/mcp` from a tiny_http server (the app's local server, or a test's).
pub fn serve(mut req: tiny_http::Request) {
    let method = req.method().as_str().to_string();
    let headers: Vec<(String, String)> = req.headers().iter().map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string())).collect();
    // what the headers alone refuse is answered before any body is read (a web page can POST here)
    let answer = match early_refusal(&method, &headers) {
        Some(refused) => refused,
        None => {
            let mut body = Vec::new();
            let _ = req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
            handle_parts(&method, &headers, &body)
        }
    };
    let mut resp = tiny_http::Response::from_data(answer.body).with_status_code(answer.status);
    for (n, v) in answer.headers {
        if let Ok(h) = tiny_http::Header::from_bytes(n.as_bytes(), v.as_bytes()) {
            resp = resp.with_header(h);
        }
    }
    let _ = req.respond(resp);
}

/// Admission's steps that need only the headers, in the same order as `handle_parts` (Origin 403, key 401, method 405,
/// 415, 406, and a declared body over the limit 413); None when the request goes on to its body.
pub(crate) fn early_refusal(method: &str, headers: &[(String, String)]) -> Option<Reply> {
    let head = |name: &str| headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
    if head("origin").is_some() {
        return Some(empty(403));
    }
    if let Err(r) = keys::bearer(head("authorization").unwrap_or("")).and_then(|raw| keys::global().resolve(raw)) {
        return Some(unauthorized(r));
    }
    match method {
        "POST" => {}
        "DELETE" => return None,
        _ => return Some(reply(405, vec![("allow", "POST, DELETE".into())], vec![])),
    }
    if !lists(head("content-type").unwrap_or(""), "application/json") {
        return Some(empty(415));
    }
    let accept = head("accept").unwrap_or("");
    if !(lists(accept, "application/json") && lists(accept, "text/event-stream")) {
        return Some(empty(406));
    }
    if head("content-length").and_then(|n| n.trim().parse::<usize>().ok()).is_some_and(|n| n > MAX_BODY) {
        return Some(empty(413));
    }
    None
}

/// The whole of admission and dispatch, for one request.
pub(crate) fn handle_parts(method: &str, headers: &[(String, String)], body: &[u8]) -> Reply {
    let head = |name: &str| headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
    // (1) the transport allows no browser Origin: refused before credentials, as the MCP spec requires
    if head("origin").is_some() {
        return empty(403);
    }
    // (2) a chat's key
    let (scope, key_hash) = match keys::bearer(head("authorization").unwrap_or("")).and_then(|raw| keys::global().resolve(raw).map(|s| (s, keys::hash(raw)))) {
        Ok(found) => found,
        Err(r) => return unauthorized(r),
    };
    let sid = head("mcp-session-id").map(str::to_string);
    // (3) method
    match method {
        "POST" => {}
        "DELETE" => return end_session(sid.as_deref(), &key_hash),
        _ => return reply(405, vec![("allow", "POST, DELETE".into())], vec![]),
    }
    // (4) content type and accept
    if !lists(head("content-type").unwrap_or(""), "application/json") {
        return empty(415);
    }
    let accept = head("accept").unwrap_or("");
    if !(lists(accept, "application/json") && lists(accept, "text/event-stream")) {
        return empty(406);
    }
    if body.len() > MAX_BODY {
        return empty(413);
    }
    let version = head("mcp-protocol-version").map(str::trim).map(str::to_string);
    // the session named, if it is this key's: (protocol, exposure)
    let known = sid.as_deref().and_then(|s| sessions(|all| all.get(s).filter(|x| x.key_hash == key_hash).map(|x| (x.protocol.clone(), x.exposure))));
    let session_missing = sid.is_some() && known.is_none();
    // (5) JSON; session and protocol-header errors come before a parse error
    let msg: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            if let Some(v) = version.as_deref().filter(|v| !SUPPORTED.contains(v)) {
                return unsupported_version(&Value::Null, v);
            }
            if session_missing {
                return empty(404);
            }
            return rpc_error(200, &Value::Null, json!({"code": -32700, "message": "Parse error"}));
        }
    };
    // (6) batches are not taken in 2025-06-18
    if let Some(batch) = msg.as_array() {
        return if batch.is_empty() { invalid_request(400, &Value::Null, "Invalid Request") } else { empty(400) };
    }
    let has_id = msg.get("id").is_some();
    let id = match msg.get("id") {
        Some(v @ (Value::String(_) | Value::Number(_))) => v.clone(),
        _ => Value::Null,
    };
    let is_jsonrpc = msg["jsonrpc"] == "2.0";
    let rpc_method = msg["method"].as_str();
    let is_request = is_jsonrpc && rpc_method.is_some() && (!has_id || !id.is_null());
    let has_result = msg.get("result").is_some();
    let has_error = msg.get("error").is_some();
    let is_response = is_jsonrpc && has_id && matches!(msg["id"], Value::String(_) | Value::Number(_) | Value::Null) && has_result != has_error;
    let is_init = rpc_method == Some("initialize");
    if session_missing {
        return empty(404);
    }
    if !is_init {
        if let Some(v) = version.as_deref() {
            if !SUPPORTED.contains(&v) {
                return unsupported_version(&id, v);
            }
            if let Some((negotiated, _)) = known.as_ref().filter(|(p, _)| p != v) {
                return rpc_error(
                    400,
                    &id,
                    json!({"code": HEADER_MISMATCH, "message": format!("MCP-Protocol-Version header '{v}' does not match negotiated protocol version '{negotiated}'")}),
                );
            }
        }
    }
    if !is_request && !is_response {
        return invalid_request(200, &id, "Invalid Request");
    }
    if is_init && sid.is_some() {
        return invalid_request(400, &id, "initialize must not include an MCP-Session-Id header");
    }
    if !is_init && sid.is_none() {
        return invalid_request(400, &id, "MCP-Session-Id header is required; send initialize to start a session");
    }
    if is_init {
        return initialize(&id, &key_hash, head("fridae-tool-exposure").unwrap_or(""));
    }
    let sid = sid.unwrap_or_default();
    let exposure = known.map(|(_, e)| e).unwrap_or(exposure::Mode::Full);
    sessions(|all| {
        if let Some(s) = all.get_mut(&sid) {
            s.last_used = Instant::now();
        }
    });
    let with_session = |status: u16, v: &Value| json_reply(status, v, vec![("mcp-session-id", sid.clone())]);
    // (7) a client's answer, or a notification: acknowledged
    if is_response || !has_id {
        if rpc_method == Some("notifications/cancelled") {
            let named = msg["params"]["requestId"].to_string();
            sessions(|all| {
                if let Some(flag) = all.get(&sid).and_then(|s| s.in_flight.get(&named)) {
                    flag.store(true, Ordering::SeqCst);
                }
            });
        }
        return reply(202, vec![("mcp-session-id", sid.clone())], vec![]);
    }
    // (8) the method
    let result = match rpc_method.unwrap_or("") {
        "ping" => json!({}),
        "tools/list" => json!({"tools": listable().iter().filter(|d| exposure::listed(exposure, d.name)).map(listing).collect::<Vec<_>>()}),
        "tools/call" => {
            let Some(name) = msg["params"]["name"].as_str() else {
                return with_session(200, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "Tool name is required."}}));
            };
            let args = msg["params"].get("arguments").cloned().unwrap_or(Value::Null);
            let flag = Arc::new(AtomicBool::new(false));
            let key = id.to_string();
            sessions(|all| {
                if let Some(s) = all.get_mut(&sid) {
                    s.in_flight.insert(key.clone(), flag.clone());
                }
            });
            let answer = call_tool(&scope, name, args, &flag, exposure);
            sessions(|all| {
                if let Some(s) = all.get_mut(&sid) {
                    s.in_flight.remove(&key);
                }
            });
            match answer {
                Ok(v) => v,
                Err((code, message)) => return with_session(200, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})),
            }
        }
        other => return with_session(200, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {other}")}})),
    };
    with_session(200, &json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// initialize: a new session for this key (UUIDv4), on our protocol whatever version was offered.
fn initialize(id: &Value, key_hash: &str, exposure_header: &str) -> Reply {
    let sid = super::uuid_v4();
    sessions(|all| {
        while all.len() >= MAX_SESSIONS {
            let Some(oldest) = all.iter().min_by_key(|(_, s)| s.last_used).map(|(k, _)| k.clone()) else { break };
            if let Some(s) = all.remove(&oldest) {
                s.in_flight.values().for_each(|f| f.store(true, Ordering::SeqCst));
            }
        }
        all.insert(
            sid.clone(),
            Session { key_hash: key_hash.to_string(), protocol: PROTOCOL.to_string(), exposure: exposure::mode_of(exposure_header), last_used: Instant::now(), in_flight: HashMap::new() },
        );
    });
    let result = json!({"protocolVersion": PROTOCOL, "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": SERVER_NAME, "version": crate::paths::VERSION}});
    json_reply(200, &json!({"jsonrpc": "2.0", "id": id, "result": result}), vec![("mcp-session-id", sid), ("mcp-protocol-version", PROTOCOL.to_string())])
}

/// DELETE: the session ends and its calls under way are cancelled; later requests naming it get 404.
fn end_session(sid: Option<&str>, key_hash: &str) -> Reply {
    let Some(sid) = sid else {
        return invalid_request(400, &Value::Null, "MCP-Session-Id header is required; send initialize to start a session");
    };
    let ended = sessions(|all| match all.get(sid) {
        Some(s) if s.key_hash == key_hash => all.remove(sid),
        _ => None,
    });
    match ended {
        Some(s) => {
            s.in_flight.values().for_each(|f| f.store(true, Ordering::SeqCst));
            empty(204)
        }
        None => empty(404),
    }
}

#[cfg(test)]
pub(crate) fn in_flight_for_test(sid: &str, id_json: &str) -> bool {
    sessions(|all| all.get(sid).is_some_and(|s| s.in_flight.contains_key(id_json)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_mcp::{SHOW_TEST_TOOLS, drop_chat, test_chat};
    use serde_json::json;

    #[test]
    fn deferred_list_has_the_common_tools_and_stays_small() {
        // Antigravity freezes and cuts long tool lists: the deferred list holds the tools a chat most often needs
        // (with their full schemas, so no search first), well under ~10 KB
        let listed: Vec<Value> = crate::app_mcp::listable().iter().filter(|d| crate::app_mcp::exposure::listed(crate::app_mcp::exposure::Mode::DeferredProxy, d.name)).map(listing).collect();
        let names: Vec<&str> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for n in ["fridae_capabilities", "tool_search", "tool_call", "chat_launch", "chat_send", "chat_list", "project_list"] {
            assert!(names.contains(&n), "{n} not in {names:?}");
        }
        assert_eq!(names.len(), 7, "{names:?}");
        assert!(listed.iter().all(|t| t["inputSchema"]["type"] == "object"));
        let size = serde_json::to_string(&json!({"tools": listed})).unwrap().len();
        assert!(size < 8_000, "{size} bytes");
    }

    const ACCEPT: &str = "application/json, text/event-stream";

    fn key_of(a: &crate::agents::Agent) -> String {
        keys::global().prepare(&a.id, &a.ai(), false).raw
    }

    fn heads(key: Option<&str>, session: Option<&str>, extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut h = vec![("Content-Type".to_string(), "application/json".to_string()), ("Accept".to_string(), ACCEPT.to_string())];
        if let Some(k) = key {
            h.push(("Authorization".into(), format!("Bearer {k}")));
        }
        if let Some(s) = session {
            h.push(("Mcp-Session-Id".into(), s.into()));
        }
        for (n, v) in extra {
            h.retain(|(hn, _)| !hn.eq_ignore_ascii_case(n));
            h.push((n.to_string(), v.to_string()));
        }
        h
    }

    fn post(key: Option<&str>, session: Option<&str>, body: &str) -> Reply {
        handle_parts("POST", &heads(key, session, &[]), body.as_bytes())
    }

    fn header<'a>(r: &'a Reply, name: &str) -> Option<&'a str> {
        r.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn json_of(r: &Reply) -> Value {
        serde_json::from_slice(&r.body).unwrap_or(Value::Null)
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

    fn session(key: &str) -> String {
        let r = post(Some(key), None, INIT);
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        header(&r, "mcp-session-id").unwrap().to_string()
    }

    #[test]
    fn origin_is_refused_before_credentials() {
        let r = handle_parts("POST", &heads(None, None, &[("Origin", "http://evil.example")]), INIT.as_bytes());
        assert_eq!((r.status, r.body.len()), (403, 0));
        assert_eq!(header(&r, "cache-control"), Some("no-store"));
    }

    #[test]
    fn missing_key_401_with_t3s_shape() {
        let r = post(None, None, INIT);
        assert_eq!(r.status, 401);
        assert_eq!(json_of(&r), json!({"error": "invalid_mcp_credential", "message": "A valid Fridae toolbox key is required."}));
        assert_eq!(header(&r, "www-authenticate"), Some("Bearer"));
        assert_eq!(header(&r, "cache-control"), Some("no-store"));
        let r = post(Some("not-a-key"), None, INIT);
        assert_eq!(r.status, 401);
        assert_eq!(header(&r, "www-authenticate"), Some("Bearer error=\"invalid_token\""));
        let r = handle_parts("POST", &heads(None, None, &[("Authorization", "Basic abc")]), INIT.as_bytes());
        assert_eq!((r.status, header(&r, "www-authenticate")), (401, Some("Bearer")));
    }

    #[test]
    fn the_page_token_never_works() {
        let r = handle_parts("POST", &heads(None, None, &[("X-Token", "page-token")]), INIT.as_bytes());
        assert_eq!(r.status, 401);
        // `?t=` is not even looked at: the request's path is not part of admission
        assert_eq!(post(None, None, INIT).status, 401);
    }

    #[test]
    fn methods() {
        let a = test_chat("claude");
        let key = key_of(&a);
        for m in ["GET", "PUT", "PATCH", "OPTIONS"] {
            let r = handle_parts(m, &heads(Some(&key), None, &[]), b"");
            assert_eq!((r.status, header(&r, "allow")), (405, Some("POST, DELETE")), "{m}");
        }
        drop_chat(&a);
    }

    #[test]
    fn content_type_415() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let r = handle_parts("POST", &heads(Some(&key), None, &[("Content-Type", "text/plain")]), INIT.as_bytes());
        assert_eq!(r.status, 415);
        let r = handle_parts("POST", &heads(Some(&key), None, &[("Content-Type", "application/json; charset=utf-8")]), INIT.as_bytes());
        assert_eq!(r.status, 200);
        drop_chat(&a);
    }

    #[test]
    fn accept_406() {
        let a = test_chat("claude");
        let key = key_of(&a);
        assert_eq!(handle_parts("POST", &heads(Some(&key), None, &[("Accept", "application/json")]), INIT.as_bytes()).status, 406);
        let mut no_accept = heads(Some(&key), None, &[]);
        no_accept.retain(|(n, _)| n != "Accept");
        assert_eq!(handle_parts("POST", &no_accept, INIT.as_bytes()).status, 406);
        drop_chat(&a);
    }

    #[test]
    fn initialize_mints_a_session() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let r = post(Some(&key), None, INIT);
        assert_eq!(r.status, 200);
        let sid = header(&r, "mcp-session-id").unwrap();
        assert_eq!((sid.len(), &sid[14..15]), (36, "4"));
        assert_eq!(header(&r, "mcp-protocol-version"), Some("2025-06-18"));
        assert_eq!(header(&r, "content-type"), Some("application/json"));
        assert_eq!(
            json_of(&r),
            json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": "fridae", "version": crate::paths::VERSION}}})
        );
        let r = post(Some(&key), None, &INIT.replace("2025-06-18", "2099-01-01"));
        assert_eq!(json_of(&r)["result"]["protocolVersion"], "2025-06-18", "an unknown offered version is answered with ours");
        drop_chat(&a);
    }

    #[test]
    fn initialize_with_a_session_id_400() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), INIT);
        assert_eq!(r.status, 400);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32600, "message": "initialize must not include an MCP-Session-Id header"}}));
        drop_chat(&a);
    }

    #[test]
    fn no_session_400() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let r = post(Some(&key), None, r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#);
        assert_eq!(r.status, 400);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32600, "message": "MCP-Session-Id header is required; send initialize to start a session"}}));
        drop_chat(&a);
    }

    #[test]
    fn unknown_session_404_empty() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let r = post(Some(&key), Some("00000000-0000-4000-8000-000000000000"), r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        assert_eq!((r.status, r.body.len()), (404, 0));
        drop_chat(&a);
    }

    #[test]
    fn another_keys_session_404() {
        let (a, b) = (test_chat("claude"), test_chat("claude"));
        let sid = session(&key_of(&a));
        let r = post(Some(&key_of(&b)), Some(&sid), r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        assert_eq!((r.status, r.body.len()), (404, 0));
        drop_chat(&a);
        drop_chat(&b);
    }

    #[test]
    fn claudes_server_discover_gets_effects_version_rejection() {
        // Claude Code 2.1.326 first tries the sessionless 2026-07-28 protocol. A server that speaks only 2025-06-18
        // answers it as Effect's admission does (mcpRuntime.ts selectHttpProtocol: a version claim with no sessionless
        // protocol configured is 400 + -32022 with the supported list), and Claude then initializes on 2025-06-18
        // (seen in agents::tests::probe_claude_against_the_real_toolbox). T3 itself answers 200: it also configures
        // v2026_07_28, which Fridae does not speak.
        let a = test_chat("claude");
        let key = key_of(&a);
        let body = br#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"claude-code","version":"2.1.326"},"io.modelcontextprotocol/clientCapabilities":{"roots":{"listChanged":true}}}}}"#;
        let r = handle_parts("POST", &heads(Some(&key), None, &[("MCP-Protocol-Version", "2026-07-28")]), body);
        assert_eq!(r.status, 400);
        assert_eq!(
            json_of(&r),
            json!({"jsonrpc": "2.0", "id": "server-discover-probe-1", "error": {"code": -32022, "message": "Unsupported protocol version '2026-07-28'", "data": {"supported": ["2025-06-18"], "requested": "2026-07-28"}}})
        );
        assert_eq!(header(&r, "mcp-session-id"), None, "no session is opened by it");
        drop_chat(&a);
    }

    #[test]
    fn refusals_come_before_the_body_is_read() {
        // a web page can POST to 127.0.0.1: what the headers alone refuse is answered without reading any body
        let a = test_chat("claude");
        let key = key_of(&a);
        let status = |h: Vec<(String, String)>| early_refusal("POST", &h).map(|r| r.status);
        assert_eq!(status(heads(Some(&key), None, &[("Origin", "http://evil.example")])), Some(403));
        assert_eq!(status(heads(None, None, &[])), Some(401));
        assert_eq!(early_refusal("PUT", &heads(Some(&key), None, &[])).map(|r| r.status), Some(405));
        assert_eq!(status(heads(Some(&key), None, &[("Content-Type", "text/plain")])), Some(415));
        assert_eq!(status(heads(Some(&key), None, &[("Accept", "application/json")])), Some(406));
        assert_eq!(status(heads(Some(&key), None, &[("Content-Length", "99999999")])), Some(413));
        assert_eq!(status(heads(Some(&key), None, &[("Content-Length", "120")])), None, "a good request goes on to its body");
        assert!(early_refusal("DELETE", &heads(Some(&key), Some("s"), &[])).is_none());
        drop_chat(&a);
    }

    #[test]
    fn bad_protocol_header_400() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = handle_parts("POST", &heads(Some(&key), Some(&sid), &[("MCP-Protocol-Version", "1999-01-01")]), br#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#);
        assert_eq!(r.status, 400);
        assert_eq!(
            json_of(&r),
            json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32022, "message": "Unsupported protocol version '1999-01-01'", "data": {"supported": ["2025-06-18"], "requested": "1999-01-01"}}})
        );
        let ok = handle_parts("POST", &heads(Some(&key), Some(&sid), &[("MCP-Protocol-Version", "2025-06-18")]), br#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#);
        assert_eq!(ok.status, 200);
        drop_chat(&a);
    }

    #[test]
    fn parse_error_is_200_with_32700() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), "{not json");
        assert_eq!(r.status, 200);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
        // a session error comes before the parse error
        assert_eq!(post(Some(&key), Some("00000000-0000-4000-8000-000000000000"), "{not json").status, 404);
        drop_chat(&a);
    }

    #[test]
    fn batches_400() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), "[]");
        assert_eq!(r.status, 400);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Invalid Request"}}));
        let r = post(Some(&key), Some(&sid), r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        assert_eq!((r.status, r.body.len()), (400, 0));
        drop_chat(&a);
    }

    #[test]
    fn notification_only_202() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!((r.status, r.body.len()), (202, 0));
        // a client's answer to a server request is acknowledged the same way
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":9,"result":{}}"#);
        assert_eq!(r.status, 202);
        drop_chat(&a);
    }

    #[test]
    fn ping_answers_empty_object() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":"p1","method":"ping"}"#);
        assert_eq!(r.status, 200);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": "p1", "result": {}}));
        assert_eq!(header(&r, "mcp-session-id"), Some(sid.as_str()));
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#);
        assert_eq!(json_of(&r)["error"]["code"], -32601);
        drop_chat(&a);
    }

    #[test]
    fn delete_ends_the_session() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = handle_parts("DELETE", &heads(Some(&key), Some(&sid), &[]), b"");
        assert_eq!((r.status, r.body.len()), (204, 0));
        assert_eq!(post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).status, 404);
        assert_eq!(handle_parts("DELETE", &heads(Some(&key), Some(&sid), &[]), b"").status, 404);
        drop_chat(&a);
    }

    #[test]
    fn tools_list_lists_names_schemas_annotations() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        SHOW_TEST_TOOLS.with(|s| s.set(true));
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        SHOW_TEST_TOOLS.with(|s| s.set(false));
        let tools = json_of(&r)["result"]["tools"].as_array().cloned().unwrap();
        let read = tools.iter().find(|t| t["name"] == "test_read").unwrap();
        assert_eq!(
            read,
            &json!({"name": "test_read", "title": "Read a chat (test)", "description": "Test tool.",
                    "inputSchema": {"type": "object", "properties": {"chatId": {"type": "string"}}},
                    "annotations": {"title": "Read a chat (test)", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true}})
        );
        let wait = tools.iter().find(|t| t["name"] == "test_wait").unwrap();
        assert_eq!(wait["_meta"], json!({"anthropic/alwaysLoad": true}));
        assert!(tools.iter().all(|t| t["name"] != "tool_search" && t["name"] != "tool_call"));
        // without the test switch the test tools are never listed
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        assert!(json_of(&r)["result"]["tools"].as_array().unwrap().iter().all(|t| !t["name"].as_str().unwrap().starts_with("test_")));
        drop_chat(&a);
    }

    #[test]
    fn tools_call_round_trip() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let sid = session(&key);
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"test_read","arguments":{}}}"#);
        assert_eq!(json_of(&r), json!({"jsonrpc": "2.0", "id": 5, "result": crate::app_mcp::ok_result(&json!({"chatId": a.id}))}));
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope"}}"#);
        assert_eq!(json_of(&r)["error"], json!({"code": -32601, "message": "Unknown tool: nope"}));
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"test_read","arguments":{"x":1}}}"#);
        assert_eq!(json_of(&r)["error"]["code"], -32602);

        // notifications/cancelled reaches the call it names
        let (k2, s2) = (key.clone(), sid.clone());
        let waiter = std::thread::spawn(move || post(Some(&k2), Some(&s2), r#"{"jsonrpc":"2.0","id":"w1","method":"tools/call","params":{"name":"test_wait"}}"#));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !in_flight_for_test(&sid, "\"w1\"") {
            assert!(std::time::Instant::now() < until, "the call never started");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let r = post(Some(&key), Some(&sid), r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"w1"}}"#);
        assert_eq!(r.status, 202);
        let done = waiter.join().unwrap();
        assert_eq!(json_of(&done)["result"]["structuredContent"], json!({"cancelled": true}));
        drop_chat(&a);
    }

    #[test]
    fn body_too_large_413() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let big = vec![b' '; MAX_BODY + 1];
        assert_eq!(handle_parts("POST", &heads(Some(&key), None, &[]), &big).status, 413);
        drop_chat(&a);
    }

    #[test]
    fn serves_over_a_real_port() {
        let a = test_chat("claude");
        let key = key_of(&a);
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let s = server.clone();
        std::thread::spawn(move || {
            while let Ok(req) = s.recv() {
                std::thread::spawn(move || serve(req));
            }
        });
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into();
        let url = format!("http://127.0.0.1:{port}/mcp");
        let bearer = format!("Bearer {key}");
        let mut r = agent
            .post(&url)
            .header("Authorization", bearer.as_str())
            .header("Accept", ACCEPT)
            .header("Content-Type", "application/json")
            .send(INIT)
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let sid = r.headers().get("mcp-session-id").unwrap().to_str().unwrap().to_string();
        assert!(r.body_mut().read_to_string().unwrap().contains("\"protocolVersion\":\"2025-06-18\""));
        let mut r = agent
            .post(&url)
            .header("Authorization", bearer.as_str())
            .header("Accept", ACCEPT)
            .header("Content-Type", "application/json")
            .header("Mcp-Session-Id", sid.as_str())
            .send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let listed: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap();
        assert!(listed["result"]["tools"].is_array());
        let r = agent.delete(&url).header("Authorization", bearer.as_str()).header("Mcp-Session-Id", sid.as_str()).call().unwrap();
        assert_eq!(r.status().as_u16(), 204);
        server.unblock();
        drop_chat(&a);
    }
}
