//! `Fridae --mcp-bridge`: the toolbox over stdio, for AIs that take only stdio tool servers (an exact port of T3 Code's
//! AcpMcpStdioBridge.ts). Newline-delimited JSON-RPC on stdin is POSTed verbatim to the toolbox endpoint
//! (`FRIDAE_MCP_ENDPOINT`, with `FRIDAE_MCP_AUTHORIZATION` and, when set, `FRIDAE_MCP_TOOL_EXPOSURE`), and every message
//! of each answer is written to stdout as one line. The key arrives in the environment only and is never printed.

use std::io::{BufRead, Read, Write};
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

/// Request or answer headers, as (name, value).
pub type Headers = Vec<(String, String)>;

/// What the endpoint answered.
pub struct HttpAnswer {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

/// Where the bridge sends, and with what (from its environment).
pub struct BridgeConfig {
    pub endpoint: String,
    pub authorization: String,
    pub exposure: Option<String>,
}

impl std::fmt::Debug for BridgeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeConfig").field("endpoint", &self.endpoint).field("authorization", &"[redacted]").field("exposure", &self.exposure).finish()
    }
}

const MISSING_ENV: &str = "fridae --mcp-bridge needs FRIDAE_MCP_ENDPOINT and FRIDAE_MCP_AUTHORIZATION.";

/// The bridge's settings from its environment.
pub fn config_from(env: impl Fn(&str) -> Option<String>) -> Result<BridgeConfig, String> {
    // an empty value, or a placeholder left as it was (an AI that does not fill `${env:X}` in its tool list), is missing
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty() && !v.contains("${"));
    match (set("FRIDAE_MCP_ENDPOINT"), set("FRIDAE_MCP_AUTHORIZATION")) {
        (Some(endpoint), Some(authorization)) => {
            let exposure = env("FRIDAE_MCP_TOOL_EXPOSURE").map(|e| e.trim().to_string()).filter(|e| e.eq_ignore_ascii_case("deferred-proxy"));
            Ok(BridgeConfig { endpoint, authorization, exposure })
        }
        _ => Err(MISSING_ENV.to_string()),
    }
}

/// The bridge with its environment, streams and the real transport: missing settings → a line on `err` and 2.
pub fn main_with(env: impl Fn(&str) -> Option<String>, input: impl Read, output: Arc<Mutex<dyn Write + Send>>, err: &mut dyn Write) -> i32 {
    match config_from(env) {
        Ok(cfg) => run(std::io::BufReader::new(input), output, &cfg, &http_send),
        Err(m) => {
            let _ = writeln!(err, "{m}");
            2
        }
    }
}

/// Runs the bridge (`Fridae --mcp-bridge`); the process exit code.
pub fn main() -> i32 {
    let out: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(std::io::stdout()));
    main_with(|k| std::env::var(k).ok(), std::io::stdin(), out, &mut std::io::stderr())
}

/// One request over HTTP: no time limit (an Allow/Don't card waits up to 10 minutes) and no proxy (the endpoint is on
/// 127.0.0.1). A non-2xx status is an answer, not an error.
pub fn http_send(method: &str, cfg: &BridgeConfig, headers: &Headers, body: &str) -> Result<HttpAnswer, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).proxy(None).build().into();
    let answer = if method == "DELETE" {
        let mut req = agent.delete(&cfg.endpoint);
        for (n, v) in headers {
            req = req.header(n.as_str(), v.as_str());
        }
        req.call()
    } else {
        let mut req = agent.post(&cfg.endpoint);
        for (n, v) in headers {
            req = req.header(n.as_str(), v.as_str());
        }
        req.send(body.as_bytes())
    };
    let mut resp = answer.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let headers = resp.headers().iter().map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
    let body = resp.body_mut().with_config().limit(u64::MAX).read_to_vec().map_err(|e| e.to_string())?;
    Ok(HttpAnswer { status, headers, body })
}

fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// The protocol version an initialize answer names.
fn protocol_version_of(entry: &Value) -> Option<String> {
    entry.get("result").and_then(|r| r.get("protocolVersion")).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string)
}

/// The JSON-RPC messages of an answer: its body as JSON, or the `data:` of each server-sent event.
fn payloads(answer: &HttpAnswer) -> Result<Vec<Value>, String> {
    let text = String::from_utf8_lossy(&answer.body);
    let sse = header(&answer.headers, "content-type").is_some_and(|t| t.to_ascii_lowercase().starts_with("text/event-stream"));
    if !sse {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        return serde_json::from_str(&text).map(|v| vec![v]).map_err(|e| format!("Invalid JSON in the answer: {e}"));
    }
    let mut out = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let flush = |data: &mut Vec<String>, out: &mut Vec<Value>| -> Result<(), String> {
        if !data.is_empty() {
            let joined = data.join("\n");
            data.clear();
            if !joined.trim().is_empty() {
                out.push(serde_json::from_str(&joined).map_err(|e| format!("Invalid JSON in an event: {e}"))?);
            }
        }
        Ok(())
    };
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            flush(&mut data, &mut out)?;
        } else if let Some(d) = line.strip_prefix("data:") {
            data.push(d.strip_prefix(' ').unwrap_or(d).to_string());
        }
    }
    flush(&mut data, &mut out)?;
    Ok(out)
}

/// A gate a request waits at until the handshake step before it is over.
struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    fn new() -> Arc<Gate> {
        Arc::new(Gate { open: Mutex::new(false), opened: Condvar::new() })
    }
    fn wait(&self) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        while !*open {
            open = self.opened.wait(open).unwrap_or_else(|e| e.into_inner());
        }
    }
    fn release(&self) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.opened.notify_all();
    }
}

/// Opens its gate however the step ends (a panic too), so nothing waits for ever.
struct Releases(Arc<Gate>);

impl Drop for Releases {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// What the bridge carries from one answer to the next requests.
struct Shared<'a> {
    cfg: &'a BridgeConfig,
    send: &'a (dyn Fn(&str, &BridgeConfig, &Headers, &str) -> Result<HttpAnswer, String> + Sync),
    output: Arc<Mutex<dyn Write + Send>>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
}

impl Shared<'_> {
    fn write(&self, message: &Value) {
        let mut out = self.output.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    }

    fn error(&self, id: Value, code: i64, message: &str) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    }

    fn headers(&self) -> Headers {
        let mut h: Headers = vec![
            ("content-type".into(), "application/json".into()),
            ("accept".into(), "application/json, text/event-stream".into()),
            ("authorization".into(), self.cfg.authorization.clone()),
        ];
        if let Some(e) = &self.cfg.exposure {
            h.push(("fridae-tool-exposure".into(), e.clone()));
        }
        if let Some(s) = self.session.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            h.push(("mcp-session-id".into(), s));
        }
        if let Some(v) = self.protocol.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            h.push(("mcp-protocol-version".into(), v));
        }
        h
    }

    /// One line POSTed verbatim; its answer's messages written out, or its failure answered when it has an id.
    fn forward(&self, line: &str, id: Option<Value>) {
        let answer = match (self.send)("POST", self.cfg, &self.headers(), line) {
            Ok(a) => a,
            Err(e) => {
                if let Some(id) = id {
                    self.error(id, -32603, &format!("Fridae toolbox bridge request failed: {e}"));
                }
                return;
            }
        };
        if let Some(s) = header(&answer.headers, "mcp-session-id") {
            *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.to_string());
        }
        if !(200..300).contains(&answer.status) {
            if let Some(id) = id {
                self.error(id, -32603, &format!("Fridae toolbox endpoint responded with HTTP {}.", answer.status));
            }
            return;
        }
        match payloads(&answer) {
            Ok(all) => {
                for payload in all {
                    let entries = match payload {
                        Value::Array(a) => a,
                        one => vec![one],
                    };
                    for entry in entries {
                        if let Some(v) = protocol_version_of(&entry) {
                            *self.protocol.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
                        }
                        self.write(&entry);
                    }
                }
            }
            Err(e) => {
                if let Some(id) = id {
                    self.error(id, -32603, &format!("Fridae toolbox bridge request failed: {e}"));
                }
            }
        }
    }
}

/// The bridge over `input` and `output`, sending with `send` ("POST" or "DELETE", the settings, the headers, the
/// body). Session set-up (initialize, notifications/initialized) runs in order behind a barrier that later requests
/// wait at; a client's answer to a server request never waits (it could be what the server is waiting for). Other
/// requests run at once, each on a thread of its own. At the end of stdin: every request finished, the session
/// ended, 0.
pub fn run(input: impl BufRead, output: Arc<Mutex<dyn Write + Send>>, cfg: &BridgeConfig, send: &(dyn Fn(&str, &BridgeConfig, &Headers, &str) -> Result<HttpAnswer, String> + Sync)) -> i32 {
    let shared = Shared { cfg, send, output, session: Mutex::new(None), protocol: Mutex::new(None) };
    let shared = &shared;
    std::thread::scope(|scope| {
        let mut barrier: Option<Arc<Gate>> = None;
        let mut input = input;
        let mut raw = Vec::new();
        loop {
            raw.clear();
            // split on \n only: U+2028 and U+2029, which JSON leaves raw inside strings, stay inside the message
            match input.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&raw);
            let line = text.strip_suffix('\n').unwrap_or(&text);
            let line = line.strip_suffix('\r').unwrap_or(line).to_string();
            if line.trim().is_empty() {
                continue;
            }
            let parsed: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    shared.error(Value::Null, -32700, "Parse error");
                    continue;
                }
            };
            // (an array passes as JavaScript's `typeof "object"` lets it; other non-objects are ignored)
            if !(parsed.is_object() || parsed.is_array()) {
                continue;
            }
            let id = parsed.get("id").cloned();
            let method = parsed.get("method").and_then(Value::as_str).map(str::to_string);
            let is_response = id.is_some() && (parsed.get("result").is_some() || parsed.get("error").is_some());
            let handshake = matches!(method.as_deref(), Some("initialize") | Some("notifications/initialized"));
            if is_response {
                scope.spawn(move || shared.forward(&line, id));
            } else if handshake {
                let prior = barrier.take();
                let gate = Gate::new();
                barrier = Some(gate.clone());
                scope.spawn(move || {
                    let _open = Releases(gate);
                    if let Some(p) = prior {
                        p.wait();
                    }
                    shared.forward(&line, id);
                });
            } else {
                let wait = barrier.clone();
                scope.spawn(move || {
                    if let Some(g) = wait {
                        g.wait();
                    }
                    shared.forward(&line, id);
                });
            }
        }
    });
    // every request is over (the scope waited for them): end the session, if one was made
    if shared.session.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        let mut h = shared.headers();
        h.retain(|(n, _)| n != "content-type");
        let _ = send("DELETE", cfg, &h, "");
    }
    0
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::mpsc;
    use std::time::Duration;

    const KEY: &str = "Bearer sekrit-bridge-key";

    fn cfg(exposure: Option<&str>) -> BridgeConfig {
        BridgeConfig { endpoint: "http://127.0.0.1:9/mcp".into(), authorization: KEY.into(), exposure: exposure.map(str::to_string) }
    }

    /// One request the fake endpoint saw.
    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        headers: Headers,
        body: String,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }
    }

    fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> Result<HttpAnswer, String> {
        Ok(HttpAnswer { status, headers: headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(), body: body.as_bytes().to_vec() })
    }

    /// Runs the bridge over `input` with `reply` as the endpoint; (what it wrote, one JSON message per line; what the
    /// endpoint saw, in order; the exit code).
    fn bridge(input: &str, c: &BridgeConfig, reply: impl Fn(&Seen) -> Result<HttpAnswer, String> + Sync) -> (Vec<Value>, Vec<Seen>, i32) {
        let out: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let seen: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
        let send = |method: &str, _: &BridgeConfig, headers: &Headers, body: &str| {
            let s = Seen { method: method.to_string(), headers: headers.clone(), body: body.to_string() };
            seen.lock().unwrap().push(s.clone());
            reply(&s)
        };
        let code = run(std::io::Cursor::new(input.as_bytes().to_vec()), out.clone(), c, &send);
        let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        let lines = text.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|_| panic!("not one JSON message per line: {l}"))).collect();
        (lines, seen.into_inner().unwrap(), code)
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"agy","version":"1"}}}"#;
    const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

    fn server(s: &Seen) -> Result<HttpAnswer, String> {
        if s.method == "DELETE" {
            return answer(204, &[], "");
        }
        let req: Value = serde_json::from_str(&s.body).unwrap();
        match req["method"].as_str() {
            Some("initialize") => answer(200, &[("Mcp-Session-Id", "sess-1"), ("content-type", "application/json")], r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fridae","version":"1"}}}"#),
            Some("notifications/initialized") => answer(202, &[], ""),
            _ => answer(200, &[("content-type", "application/json")], &json!({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": []}}).to_string()),
        }
    }

    #[test]
    fn forwards_and_replays_session_and_version() {
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let (out, seen, code) = bridge(&format!("{INIT}\n{INITIALIZED}\n\n{list}\n"), &cfg(None), server);
        assert_eq!(code, 0);
        let posts: Vec<&Seen> = seen.iter().filter(|s| s.method == "POST").collect();
        assert_eq!(posts.len(), 3, "the empty line is skipped");
        assert_eq!(posts[0].body, INIT, "posted verbatim");
        assert_eq!((posts[0].header("mcp-session-id"), posts[0].header("mcp-protocol-version")), (None, None));
        for p in &posts {
            assert_eq!(p.header("content-type"), Some("application/json"));
            assert_eq!(p.header("accept"), Some("application/json, text/event-stream"));
            assert_eq!(p.header("authorization"), Some(KEY));
            assert_eq!(p.header("fridae-tool-exposure"), None);
        }
        let last = posts.iter().find(|p| p.body == list).unwrap();
        assert_eq!((last.header("mcp-session-id"), last.header("mcp-protocol-version")), (Some("sess-1"), Some("2025-06-18")));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "fridae");
        assert_eq!(out[1], json!({"jsonrpc": "2.0", "id": 2, "result": {"tools": []}}));
        // stdin over: the session is ended
        let end = seen.last().unwrap();
        assert_eq!((end.method.as_str(), end.header("mcp-session-id"), end.header("authorization")), ("DELETE", Some("sess-1"), Some(KEY)));
    }

    #[test]
    fn notifications_silent_failures_synthesised() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":\"six\",\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\"}\n";
        let (out, _, code) = bridge(input, &cfg(None), |s| {
            let req: Value = serde_json::from_str(&s.body).unwrap();
            if req["id"] == "six" || req["method"] == "notifications/cancelled" { Err("connection refused".into()) } else { answer(500, &[], "boom") }
        });
        assert_eq!(code, 0);
        let mut out = out;
        out.sort_by_key(|v| v["id"].to_string()); // "\"six\"" before "5"
        assert_eq!(
            out,
            vec![
                json!({"jsonrpc": "2.0", "id": "six", "error": {"code": -32603, "message": "Fridae toolbox bridge request failed: connection refused"}}),
                json!({"jsonrpc": "2.0", "id": 5, "error": {"code": -32603, "message": "Fridae toolbox endpoint responded with HTTP 500."}}),
            ],
            "notifications fail silently"
        );
    }

    #[test]
    fn parse_errors() {
        let (out, seen, _) = bridge("{not json\n5\nnull\n\"x\"\n[{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}]\n", &cfg(None), |_| answer(200, &[], r#"[{"jsonrpc":"2.0","id":9,"result":{}}]"#));
        assert_eq!(out[0], json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
        assert_eq!(out[1], json!({"jsonrpc": "2.0", "id": 9, "result": {}}), "an answer array is split into lines");
        assert_eq!(out.len(), 2);
        assert_eq!(seen.iter().filter(|s| s.method == "POST").count(), 1, "non-object JSON is ignored; an array is passed on as T3 does");
    }

    #[test]
    fn keeps_u2028_inside_a_message() {
        let line = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"chat_send\",\"arguments\":{\"text\":\"a\u{2028}b\u{2029}c\"}}}";
        let (out, seen, _) = bridge(&format!("{line}\r\n"), &cfg(None), |s| answer(200, &[], &json!({"jsonrpc": "2.0", "id": 3, "result": {"echo": serde_json::from_str::<Value>(&s.body).unwrap()["params"]["arguments"]["text"]}}).to_string()));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, line);
        assert_eq!(out[0]["result"]["echo"], "a\u{2028}b\u{2029}c");
    }

    #[test]
    fn sse_messages_forwarded() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: [{\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{\"a\":1}},\ndata: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{}}]\n\n";
        let (out, _, _) = bridge("{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n", &cfg(None), |_| answer(200, &[("Content-Type", "text/event-stream")], sse));
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[0]["method"], "notifications/progress");
        assert_eq!(out[1], json!({"jsonrpc": "2.0", "id": 4, "result": {"a": 1}}));
        assert_eq!(out[2]["id"], 5);
    }

    #[test]
    fn client_responses_skip_the_barrier() {
        // initialize is held until the client's answer to a server request has been posted: were that answer behind
        // the barrier, initialize would wait out its 5 s and the order below would be the other way round
        let (tx, rx) = mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let order = Mutex::new(Vec::new());
        let reply = |s: &Seen| {
            let req: Value = serde_json::from_str(&s.body).unwrap_or(Value::Null);
            if req["method"] == "initialize" {
                let got = rx.lock().unwrap().recv_timeout(Duration::from_secs(5)).is_ok();
                order.lock().unwrap().push(if got { "init after answer" } else { "init timed out" });
                return server(s);
            }
            if req.get("result").is_some() {
                order.lock().unwrap().push("answer");
                let _ = tx.send(());
                return answer(202, &[], "");
            }
            server(s)
        };
        let input = format!("{INIT}\n{{\"jsonrpc\":\"2.0\",\"id\":\"srv-1\",\"result\":{{}}}}\n");
        let (out, _, _) = bridge(&input, &cfg(None), reply);
        assert_eq!(*order.lock().unwrap(), vec!["answer", "init after answer"]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn requests_wait_for_the_handshake() {
        let order = Mutex::new(Vec::new());
        let reply = |s: &Seen| {
            let req: Value = serde_json::from_str(&s.body).unwrap_or(Value::Null);
            if req["method"] == "initialize" {
                std::thread::sleep(Duration::from_millis(200));
            }
            order.lock().unwrap().push(req["method"].as_str().unwrap_or("").to_string());
            server(s)
        };
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        bridge(&format!("{INIT}\n{list}\n{INITIALIZED}\n"), &cfg(None), reply);
        let order = order.lock().unwrap();
        assert_eq!(order[0], "initialize");
        assert!(order.contains(&"tools/list".to_string()) && order.contains(&"notifications/initialized".to_string()));
    }

    #[test]
    fn keeps_the_servers_jsonrpc_error() {
        let err = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"Unknown argument \"zz\"."}}"#;
        let (out, _, _) = bridge("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\"}\n", &cfg(None), |_| answer(200, &[], err));
        assert_eq!(out, vec![serde_json::from_str::<Value>(err).unwrap()]);
    }

    #[test]
    fn forwards_the_exposure_header() {
        let (_, seen, _) = bridge(&format!("{INIT}\n"), &cfg(Some("deferred-proxy")), server);
        assert!(!seen.is_empty() && seen.iter().all(|s| s.header("fridae-tool-exposure") == Some("deferred-proxy")), "{seen:?}");
        let (_, seen, _) = bridge(&format!("{INIT}\n"), &cfg(None), server);
        assert!(!seen.is_empty() && seen.iter().all(|s| s.header("fridae-tool-exposure").is_none()));
    }

    #[test]
    fn missing_env_exits_2() {
        // an empty value, or a placeholder the AI did not fill (Cursor's `${env:X}`, a plain `${X}`), is missing too
        let unfilled = [("FRIDAE_MCP_ENDPOINT", "${env:FRIDAE_MCP_ENDPOINT}"), ("FRIDAE_MCP_AUTHORIZATION", "${FRIDAE_MCP_AUTHORIZATION}")];
        let empty = [("FRIDAE_MCP_ENDPOINT", "http://127.0.0.1:9/mcp"), ("FRIDAE_MCP_AUTHORIZATION", " ")];
        for env in [vec![], vec![("FRIDAE_MCP_ENDPOINT", "http://127.0.0.1:9/mcp")], vec![("FRIDAE_MCP_AUTHORIZATION", KEY)], unfilled.to_vec(), empty.to_vec()] {
            let lookup = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
            let config = config_from(lookup);
            assert!(config.is_err());
            assert_eq!(config.unwrap_err(), "fridae --mcp-bridge needs FRIDAE_MCP_ENDPOINT and FRIDAE_MCP_AUTHORIZATION.");
        }
        let both = |k: &str| match k {
            "FRIDAE_MCP_ENDPOINT" => Some("http://127.0.0.1:9/mcp".to_string()),
            "FRIDAE_MCP_AUTHORIZATION" => Some(KEY.to_string()),
            "FRIDAE_MCP_TOOL_EXPOSURE" => Some("deferred-proxy".to_string()),
            _ => None,
        };
        let c = config_from(both).unwrap();
        assert_eq!((c.endpoint.as_str(), c.authorization.as_str(), c.exposure.as_deref()), ("http://127.0.0.1:9/mcp", KEY, Some("deferred-proxy")));
        let mut err = Vec::new();
        assert_eq!(main_with(|_| None, std::io::empty(), Arc::new(Mutex::new(Vec::new())), &mut err), 2);
        assert_eq!(String::from_utf8(err).unwrap(), "fridae --mcp-bridge needs FRIDAE_MCP_ENDPOINT and FRIDAE_MCP_AUTHORIZATION.\n");
    }

    #[test]
    fn never_prints_the_authorization() {
        let c = cfg(None);
        let shown = format!("{c:?}");
        assert!(!shown.contains("sekrit") && shown.contains("[redacted]"), "{shown}");
        let (out, _, _) = bridge("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n{bad\n", &c, |s| {
            if s.body.contains("\"id\":1") { answer(401, &[], "{\"error\":\"invalid_mcp_credential\"}") } else { Err("refused".into()) }
        });
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("sekrit"), "{text}");
        // and through the real transport, against an endpoint that is not there
        let mut err = Vec::new();
        let out: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let env = |k: &str| match k {
            "FRIDAE_MCP_ENDPOINT" => Some("http://127.0.0.1:1/mcp".to_string()),
            "FRIDAE_MCP_AUTHORIZATION" => Some(KEY.to_string()),
            _ => None,
        };
        assert_eq!(main_with(env, std::io::Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".to_vec()), out.clone(), &mut err), 0);
        let all = String::from_utf8_lossy(&out.lock().unwrap()).to_string() + &String::from_utf8_lossy(&err);
        assert!(all.contains("Fridae toolbox bridge request failed: ") && !all.contains("sekrit"), "{all}");
    }

    #[test]
    fn real_http_round_trip() {
        // a stand-in endpoint on 127.0.0.1 that answers like the toolbox and notes what it was sent
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = srv.server_addr().to_ip().unwrap().port();
        let (tx, rx) = mpsc::channel::<(String, Option<String>, Option<String>, String)>();
        let handle = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok(Some(mut req)) = srv.recv_timeout(Duration::from_secs(10)) else { return };
                let h = |n: &str| req.headers().iter().find(|x| x.field.as_str().as_str().eq_ignore_ascii_case(n)).map(|x| x.value.as_str().to_string());
                let (auth, sid) = (h("authorization"), h("mcp-session-id"));
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let _ = tx.send((req.method().as_str().to_string(), auth, sid, body.clone()));
                let resp = if req.method().as_str() == "DELETE" {
                    tiny_http::Response::from_data(Vec::new()).with_status_code(204)
                } else if body.contains("initialize") {
                    tiny_http::Response::from_data(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}"#.as_bytes().to_vec())
                        .with_header(tiny_http::Header::from_bytes("Mcp-Session-Id", "real-1").unwrap())
                        .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap())
                } else {
                    tiny_http::Response::from_data(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#.as_bytes().to_vec())
                };
                let _ = req.respond(resp);
            }
        });
        let c = BridgeConfig { endpoint: format!("http://127.0.0.1:{port}/mcp"), authorization: KEY.into(), exposure: None };
        let out: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let input = format!("{INIT}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n");
        assert_eq!(run(std::io::Cursor::new(input.into_bytes()), out.clone(), &c, &http_send), 0);
        handle.join().unwrap();
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got.len(), 3);
        assert_eq!((got[0].0.as_str(), got[0].1.as_deref(), got[0].2.as_deref()), ("POST", Some(KEY), None));
        assert_eq!((got[1].0.as_str(), got[1].2.as_deref()), ("POST", Some("real-1")));
        assert_eq!((got[2].0.as_str(), got[2].2.as_deref()), ("DELETE", Some("real-1")));
        let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
    }

    #[test]
    fn bridge_against_the_real_toolbox() {
        // the bridge, the real transport and the real endpoint (`http::handle_parts`), with a chat's real key
        let chat = crate::app_mcp::test_chat("gemini");
        let raw = crate::app_mcp::keys::global().prepare(&chat.id, "gemini", false).raw;
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = srv.server_addr().to_ip().unwrap().port();
        let handle = std::thread::spawn(move || {
            while let Ok(Some(mut req)) = srv.recv_timeout(Duration::from_secs(5)) {
                let mut body = Vec::new();
                let _ = req.as_reader().read_to_end(&mut body);
                let headers: Vec<(String, String)> = req.headers().iter().map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string())).collect();
                let reply = crate::app_mcp::http::handle_parts(req.method().as_str(), &headers, &body);
                let deleted = req.method().as_str() == "DELETE";
                let mut resp = tiny_http::Response::from_data(reply.body).with_status_code(reply.status);
                for (n, v) in reply.headers {
                    resp = resp.with_header(tiny_http::Header::from_bytes(n.as_bytes(), v.as_bytes()).unwrap());
                }
                let _ = req.respond(resp);
                if deleted {
                    return true;
                }
            }
            false
        });
        let c = BridgeConfig { endpoint: format!("http://127.0.0.1:{port}/mcp"), authorization: format!("Bearer {raw}"), exposure: Some("deferred-proxy".into()) };
        let out: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let input = format!("{INIT}
{INITIALIZED}
{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}}
");
        assert_eq!(run(std::io::Cursor::new(input.into_bytes()), out.clone(), &c, &http_send), 0);
        assert!(handle.join().unwrap(), "the session was ended at the end of stdin");
        let text = String::from_utf8(out.lock().unwrap().clone()).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], "fridae");
        let names: Vec<&str> = lines[1]["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"tool_search") && names.contains(&"tool_call"), "deferred tools listed: {names:?}");
        assert!(!text.contains(&raw));
        crate::app_mcp::drop_chat(&chat);
    }
}
