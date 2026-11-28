//! Fridae's own toolbox for the AIs it runs: an MCP server on the app's local server (`POST/DELETE /mcp`) that lets a
//! chat's AI operate the app itself (chats, projects, settings, the theme). A port of T3 Code's app-control MCP
//! server (commit 9ceb8bfb, MIT, Copyright (c) 2026 T3 Tools Inc.): see THIRD_PARTY.md.

// Pieces the tools and the per-AI wiring use are built here first; until those tasks join, some are unused.
#![allow(dead_code)]

pub mod bridge;
pub mod exposure;
pub mod instructions;
pub mod keys;
pub mod tools;
#[cfg(test)]
mod e2e_tests;

use std::sync::{Mutex, atomic::AtomicBool};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// The toolbox's server name, as every AI is handed it (tools then read `mcp__fridae__chat_launch` and the like).
pub const SERVER_NAME: &str = "fridae";
/// The MCP protocol version served (T3's stateful one).
pub const PROTOCOL: &str = "2025-06-18";

#[cfg(test)]
static ENDPOINT_FOR_TEST: Mutex<Option<String>> = Mutex::new(None);

/// Where an AI reaches the toolbox: `http://127.0.0.1:<port>/mcp` on the app's own local server; None before it is up.
pub fn endpoint() -> Option<String> {
    #[cfg(test)]
    return ENDPOINT_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(not(test))]
    crate::server::local().map(|(port, _)| format!("http://127.0.0.1:{port}/mcp"))
}

/// Tests: what `endpoint()` answers (None: as if the local server were not up).
#[cfg(test)]
pub fn set_endpoint_for_test(url: Option<String>) {
    *ENDPOINT_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()) = url;
}

/// A project's id for agents: "p" and the first 12 hex of the SHA-256 of its folder, spelled as `projects::same`
/// compares folders (`/` as `\`, no trailing `\`, lower case), so every spelling of one folder has one id.
pub fn project_id(path: &str) -> String {
    let norm = path.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    format!("p{}", &crate::paths::hex(&Sha256::digest(norm.as_bytes()))[..12])
}

/// A link title with `[`, `]` and `\` escaped, so the markdown link stays one link.
fn link_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        if matches!(c, '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A chat as agents mention it (quoted verbatim, drawn as a chip on the page).
pub fn chat_link(chat_id: &str, title: &str) -> String {
    format!("[{}](fridae://chat/{chat_id})", link_title(title))
}

/// A project as agents mention it.
pub fn project_link(project_id: &str, name: &str) -> String {
    format!("[{}](fridae://project/{project_id})", link_title(name))
}

/// A chat's run id from when its turn started (`Agent::turn_started`); None before its first turn.
pub fn run_id(turn_started: f64) -> Option<String> {
    (turn_started > 0.0).then(|| format!("r{}", (turn_started * 1000.0) as u64))
}

/// T3's tool hints (Effect's defaults: read_only false, destructive = !read_only, idempotent false, open_world true).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

impl Hints {
    /// Effect's defaults for a tool that only reads.
    pub const READ: Hints = Hints { read_only: true, destructive: false, idempotent: false, open_world: true };
    /// Effect's defaults for a tool that changes something.
    pub const WRITE: Hints = Hints { read_only: false, destructive: true, idempotent: false, open_world: true };
}

/// What a call may do, and so which caller check runs before its handler (T3's declaration kinds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    /// Reads: the calling chat must exist; no live run needed.
    Reads,
    /// Reads as the caller (tool_search).
    ReadsAsCaller,
    /// Changes chats: a live caller; each target chat checked by the handler with `access::target_within`.
    WritesThreads,
    /// Starts chats: a live caller; modes resolved with `access::resolve_runtime`.
    StartsThreads,
    /// Changes projects, settings or the look: a live caller, then `access::environment_gate` in the handler.
    WritesEnvironment,
}

/// A tool row's words, T3's [action, running, done, detail] ("Rename", "Renaming", "Renamed", "a project").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Labels {
    pub action: &'static str,
    pub running: &'static str,
    pub done: &'static str,
    pub detail: &'static str,
}

/// How several done rows of one kind fold into one ("Renamed 2 projects").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub verb: &'static str,
    pub one: &'static str,
    pub many: &'static str,
}

/// One toolbox tool.
#[derive(Clone, Copy)]
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// The JSON Schema object of its arguments (`properties` are the accepted arguments).
    pub schema: fn() -> Value,
    pub hints: Hints,
    /// Per call (chat_configure reads or sets).
    pub access: fn(&Map<String, Value>) -> Access,
    /// Per call (tools with an `action`).
    pub labels: fn(&Map<String, Value>) -> (Labels, Summary),
    /// `_meta {"anthropic/alwaysLoad": true}` in tools/list (Claude defers the rest).
    pub always_load: bool,
    pub handler: fn(&Call, &Map<String, Value>) -> Outcome,
}

/// One call of a tool, as its handler sees it.
pub struct Call<'a> {
    pub scope: &'a keys::Scope,
    pub call_id: &'a str,
    /// Set when the client cancels the call (notifications/cancelled) or its session ends.
    pub cancelled: &'a AtomicBool,
    pub exposure: exposure::Mode,
    links: Mutex<Vec<Value>>,
}

impl<'a> Call<'a> {
    pub fn new(scope: &'a keys::Scope, call_id: &'a str, cancelled: &'a AtomicBool, exposure: exposure::Mode) -> Call<'a> {
        Call { scope, call_id, cancelled, exposure, links: Mutex::new(Vec::new()) }
    }

    /// Chips for this call's tool row: `{"kind": "chat"|"project", "id", "title"}` each (added to any given before).
    pub fn links(&self, links: Vec<Value>) {
        self.links.lock().unwrap_or_else(|e| e.into_inner()).extend(links);
    }

    fn taken_links(&self) -> Vec<Value> {
        std::mem::take(&mut *self.links.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// A declared failure: one of `CODES` and a message for the agent.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

pub type Outcome = Result<Value, Failure>;

pub fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure { code, message: message.into() }
}

/// T3's failure codes (contracts/orchestratorMcp.ts) and Fridae's card's three.
pub const CODES: [&str; 19] = [
    "capability_denied",
    "parent_not_active",
    "provider_unavailable",
    "model_unavailable",
    "runtime_mode_escalation_denied",
    "interaction_mode_escalation_denied",
    "task_not_found",
    "task_not_cancellable",
    "thread_not_found",
    "run_not_found",
    "thread_not_sendable",
    "thread_not_interruptible",
    "invalid_request",
    "orchestration_error",
    "thread_credential_required",
    "target_required",
    "declined",
    "cancelled",
    "timed_out",
];

/// A tool's answer: both forms, since clients differ in which they read.
pub fn ok_result(v: &Value) -> Value {
    json!({"isError": false, "structuredContent": v, "content": [{"type": "text", "text": v.to_string()}]})
}

pub fn fail_result(f: &Failure) -> Value {
    let text = json!({"_tag": "FridaeToolFailure", "code": f.code, "message": f.message}).to_string();
    json!({"isError": true, "content": [{"type": "text", "text": text}]})
}

/// A handler that panicked (the cause is never shown to the agent).
pub fn crash_result() -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": "Tool execution failed due to an internal server error."}]})
}

const MAX_NAMED_ARGUMENTS: usize = 5;
const MAX_ARGUMENT_NAME_CHARS: usize = 64;

/// T3's cutText: at most `max` characters, then "…".
fn cut_text(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The arguments a tool accepts (the keys of its schema's `properties`), with null fields dropped (optional fields
/// accept null). An argument it does not list is refused by name, as T3's advertisedArgumentsOnly does.
pub fn check_args(def: &ToolDef, args: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let schema = (def.schema)();
    let accepted: Vec<String> = schema["properties"].as_object().map(|p| p.keys().cloned().collect()).unwrap_or_default();
    let unknown: Vec<&String> = args.keys().filter(|k| !accepted.contains(k)).collect();
    if !unknown.is_empty() {
        let named: Vec<String> = unknown.iter().take(MAX_NAMED_ARGUMENTS).map(|k| Value::String(cut_text(k, MAX_ARGUMENT_NAME_CHARS)).to_string()).collect();
        let more = if unknown.len() > named.len() { format!(" and {} more", unknown.len() - named.len()) } else { String::new() };
        let accepted_text = if accepted.is_empty() { "none".to_string() } else { accepted.join(", ") };
        return Err(format!("Unknown argument{} {}{more}. Accepted arguments: {accepted_text}.", if unknown.len() == 1 { "" } else { "s" }, named.join(", ")));
    }
    Ok(args.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// How an AI's launch reaches the toolbox: the endpoint and the chat's key (never logged: Debug is redacted).
#[derive(Clone)]
pub struct Wire {
    pub url: String,
    pub raw: String,
    /// The AI takes the deferred tools (`fridae-tool-exposure: deferred-proxy`): Antigravity (Gemini) only.
    pub deferred: bool,
}

impl Wire {
    /// The `Authorization` header value.
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.raw)
    }
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// The chat's toolbox wiring for a launch of its AI: its key (`keys::Registry::prepare`) and the endpoint. None while
/// the local server is not up.
pub fn wire_for(chat: &str, ai: &str, subagent: bool) -> Option<Wire> {
    let url = endpoint()?;
    let cred = keys::global().prepare(chat, ai, subagent);
    Some(Wire { url, raw: cred.raw, deferred: ai == "gemini" })
}

/// Tools that exist only in tests (called through `call_tool`, listed only where a test asks).
#[cfg(test)]
fn test_defs() -> Vec<ToolDef> {
    fn labels_read(_: &Map<String, Value>) -> (Labels, Summary) {
        (Labels { action: "Read", running: "Reading", done: "Read", detail: "a chat" }, Summary { verb: "Read", one: "chat", many: "chats" })
    }
    fn reads(_: &Map<String, Value>) -> Access {
        Access::Reads
    }
    fn schema_read() -> Value {
        json!({"type": "object", "properties": {"chatId": {"type": "string"}}})
    }
    fn read(call: &Call, args: &Map<String, Value>) -> Outcome {
        let id = args.get("chatId").and_then(Value::as_str).unwrap_or(&call.scope.chat).to_string();
        call.links(vec![json!({"kind": "chat", "id": id, "title": "Test"})]);
        Ok(json!({"chatId": id}))
    }
    vec![ToolDef {
        name: "test_read",
        title: "Read a chat (test)",
        description: "Test tool.",
        schema: schema_read,
        hints: Hints::READ,
        access: reads,
        labels: labels_read,
        always_load: false,
        handler: read,
    }]
}

/// A random UUIDv4 (no `uuid` crate): 16 secure random bytes, version nibble 4, variant bits 10.
pub fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    crate::remote_common::random(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::paths::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn abc_schema() -> Value {
        json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {}, "c": {}}})
    }

    fn def_with(schema: fn() -> Value) -> ToolDef {
        ToolDef { schema, ..test_defs()[0] }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn ok_and_fail_shapes() {
        assert_eq!(ok_result(&json!({"a": 1})), json!({"isError": false, "structuredContent": {"a": 1}, "content": [{"type": "text", "text": "{\"a\":1}"}]}));
        let f = fail_result(&fail("thread_not_found", "x"));
        assert_eq!(f["isError"], json!(true));
        assert!(f.get("structuredContent").is_none());
        let text: Value = serde_json::from_str(f["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, json!({"_tag": "FridaeToolFailure", "code": "thread_not_found", "message": "x"}));
        assert_eq!(crash_result()["content"][0]["text"], "Tool execution failed due to an internal server error.");
        assert_eq!(CODES.len(), 19);
        assert!(CODES.contains(&"timed_out") && CODES.contains(&"target_required"));
    }

    #[test]
    fn unknown_arguments_are_named() {
        let def = def_with(abc_schema);
        assert_eq!(check_args(&def, &obj(json!({"a": 1, "zz": 2}))), Err("Unknown argument \"zz\". Accepted arguments: a, b, c.".to_string()));
        assert_eq!(check_args(&def, &obj(json!({"x": 1, "y": 2}))), Err("Unknown arguments \"x\", \"y\". Accepted arguments: a, b, c.".to_string()));
        let six = obj(json!({"u1": 0, "u2": 0, "u3": 0, "u4": 0, "u5": 0, "u6": 0}));
        assert_eq!(check_args(&def, &six), Err("Unknown arguments \"u1\", \"u2\", \"u3\", \"u4\", \"u5\" and 1 more. Accepted arguments: a, b, c.".to_string()));
        let long = "k".repeat(70);
        let err = check_args(&def, &obj(json!({ long.clone(): 1 }))).unwrap_err();
        assert!(err.starts_with(&format!("Unknown argument \"{}…\".", "k".repeat(64))), "{err}");
        let none = def_with(|| json!({"type": "object", "properties": {}}));
        assert_eq!(check_args(&none, &obj(json!({"q": 1}))), Err("Unknown argument \"q\". Accepted arguments: none.".to_string()));
        assert_eq!(check_args(&def, &obj(json!({"a": 1, "c": "x"}))), Ok(obj(json!({"a": 1, "c": "x"}))));
    }

    #[test]
    fn null_optional_fields_are_dropped() {
        let def = def_with(abc_schema);
        assert_eq!(check_args(&def, &obj(json!({"a": null}))), Ok(Map::new()));
        assert!(check_args(&def, &obj(json!({"zz": null}))).is_err(), "an unknown argument is unknown even when null");
    }

    #[test]
    fn project_id_is_stable_across_spellings() {
        let id = project_id("C:/Games/Shop/");
        assert_eq!(id, project_id("c:\\games\\shop"));
        assert!(id.starts_with('p'));
        assert_eq!(id.len(), 13);
        assert_ne!(id, project_id("c:\\games\\shop2"));
    }

    #[test]
    fn links_escape_brackets() {
        assert_eq!(chat_link("c1-2", "a [b]"), "[a \\[b\\]](fridae://chat/c1-2)");
        assert_eq!(project_link("p0123456789ab", "x\\y"), "[x\\\\y](fridae://project/p0123456789ab)");
    }

    #[test]
    fn run_id_from_turn_start() {
        assert_eq!(run_id(1732.5), Some("r1732500".to_string()));
        assert_eq!(run_id(0.0), None);
    }

    #[test]
    fn wire_debug_is_redacted() {
        let w = Wire { url: "http://127.0.0.1:1/mcp".into(), raw: "sekrit-key".into(), deferred: false };
        assert_eq!(w.authorization(), "Bearer sekrit-key");
        let shown = format!("{w:?}");
        assert!(!shown.contains("sekrit") && shown.contains("[redacted]"), "{shown}");
        set_endpoint_for_test(None);
        assert!(wire_for("c-wire", "claude", false).is_none(), "no server, no toolbox");
        set_endpoint_for_test(Some("http://127.0.0.1:9/mcp".into()));
        let w = wire_for("c-wire", "gemini", false).unwrap();
        assert_eq!((w.url.as_str(), w.deferred), ("http://127.0.0.1:9/mcp", true), "Gemini takes the deferred tools");
        assert_eq!(keys::global().resolve(&w.raw).unwrap().chat, "c-wire");
        assert!(!wire_for("c-wire2", "claude", false).unwrap().deferred);
        keys::global().revoke_chat("c-wire");
        keys::global().revoke_chat("c-wire2");
        set_endpoint_for_test(None);
    }

    #[test]
    fn uuids_are_version_4() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }
}
