//! Fridae's own toolbox for the AIs it runs: an MCP server on the app's local server (`POST/DELETE /mcp`) that lets a
//! chat's AI operate the app itself (chats, projects, settings, the theme). A port of T3 Code's app-control MCP
//! server (commit 9ceb8bfb, MIT, Copyright (c) 2026 T3 Tools Inc.): see THIRD_PARTY.md.

// Pieces the tools and the per-AI wiring use are built here first; until those tasks join, some are unused.
#![allow(dead_code)]

pub mod access;
pub mod bridge;
pub mod card;
pub mod events;
pub mod exposure;
pub mod http;
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

/// Every tool a call can name: the tools, then the deferred ones (and, in tests, the test tools).
pub fn registry() -> Vec<ToolDef> {
    let mut all = tools::defs();
    all.extend(exposure::defs());
    #[cfg(test)]
    all.extend(test_defs());
    all
}

/// The tools tools/list may show (the test tools only where a test asks, on its own thread).
pub(crate) fn listable() -> Vec<ToolDef> {
    #[cfg(test)]
    if !SHOW_TEST_TOOLS.with(|s| s.get()) {
        return registry().into_iter().filter(|d| !d.name.starts_with("test_")).collect();
    }
    registry()
}

#[cfg(test)]
thread_local! {
    /// Tests: tools/list on this thread shows the test tools too.
    pub(crate) static SHOW_TEST_TOOLS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tools whose `clientRequestId` replays the first answer instead of acting twice (scoped to the key's namespace).
pub const IDEMPOTENT: [&str; 4] = ["chat_launch", "chat_send", "chat_interrupt", "chat_update"];
const REPLAY_FOR: std::time::Duration = std::time::Duration::from_secs(3600);
const REPLAY_MAX: usize = 1024;

/// (namespace, tool, clientRequestId) → when, and the answer (None while the first call still runs).
type ReplayKey = (String, String, String);
static REPLAYS: Mutex<Option<std::collections::HashMap<ReplayKey, (std::time::Instant, Option<Value>)>>> = Mutex::new(None);
static REPLAY_DONE: std::sync::Condvar = std::sync::Condvar::new();

fn is_idempotent(name: &str) -> bool {
    IDEMPOTENT.contains(&name) || (cfg!(test) && name.starts_with("test_count"))
}

/// Tests: a first call of `key` marked as running since `at`.
#[cfg(test)]
fn mark_running_for_test(key: &ReplayKey, at: std::time::Instant) {
    REPLAYS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key.clone(), (at, None));
}

/// The answer to replay, or None when this call is the first (it is then marked as running, and a second call with
/// the same key waits for it).
fn replay_begin(key: &ReplayKey) -> Option<Value> {
    let mut guard = REPLAYS.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let map = guard.get_or_insert_with(Default::default);
        // answers are kept an hour; a first call still "running" after an hour is gone (a lost thread), so a retry
        // is not held behind it for ever
        map.retain(|_, (at, _)| at.elapsed() < REPLAY_FOR);
        match map.get(key) {
            Some((_, Some(v))) => return Some(v.clone()),
            Some((_, None)) => guard = REPLAY_DONE.wait_timeout(guard, std::time::Duration::from_secs(1)).unwrap_or_else(|e| e.into_inner()).0,
            None => {
                if map.len() >= REPLAY_MAX {
                    let oldest = map.iter().filter(|(_, (_, v))| v.is_some()).min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone());
                    if let Some(k) = oldest {
                        map.remove(&k);
                    }
                }
                map.insert(key.clone(), (std::time::Instant::now(), None));
                return None;
            }
        }
    }
}

/// The first call is over: keep its answer for replays (None: it crashed, so a retry runs again).
fn replay_end(key: &ReplayKey, answer: Option<Value>) {
    let mut guard = REPLAYS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    match answer {
        Some(v) => {
            map.insert(key.clone(), (std::time::Instant::now(), Some(v)));
        }
        None => {
            map.remove(key);
        }
    }
    REPLAY_DONE.notify_all();
}

/// "Read" → "read", for "Failed to read a chat".
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

/// One tool call from a chat's AI: the tool found, its arguments checked, its caller checked for what it does, the
/// handler run (a panic is an internal error), and the calling chat's tool row drawn (`apptool` events: running, then
/// done, failed or declined). `Err` is a JSON-RPC error: -32601 unknown tool, -32602 bad arguments.
pub fn call_tool(scope: &keys::Scope, name: &str, args: Value, cancelled: &AtomicBool, exposure: exposure::Mode) -> Result<Value, (i64, String)> {
    let def = registry().into_iter().find(|d| d.name == name).ok_or_else(|| (-32601, format!("Unknown tool: {name}")))?;
    // tool_call is the named tool's call: its answer, row and errors are that tool's (Task 3's ruling: a handler's
    // answer would be wrapped a second time)
    if def.name == exposure::TOOL_CALL {
        return exposure::call(scope, args, cancelled, exposure);
    }
    let args = match args {
        Value::Null => Map::new(),
        Value::Object(m) => m,
        _ => return Err((-32602, "arguments must be a JSON object.".to_string())),
    };
    let args = check_args(&def, &args).map_err(|m| (-32602, m))?;
    let replay = match args.get("clientRequestId").and_then(Value::as_str) {
        Some(id) if is_idempotent(name) => Some((scope.namespace.clone(), name.to_string(), id.to_string())),
        _ => None,
    };
    if let Some(v) = replay.as_ref().and_then(replay_begin) {
        return Ok(v);
    }
    // the replay entry is released however this call ends (a guard), and nothing in the call (labels, access, the
    // tool rows, the handler) can unwind out of it: a panic anywhere is an internal error
    let mut guard = ReplayGuard(replay);
    let (result, keep) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_tool(&def, scope, &args, cancelled, exposure))).unwrap_or_else(|_| (crash_result(), false));
    if let Some(key) = guard.0.take() {
        replay_end(&key, keep.then(|| result.clone()));
    }
    Ok(result)
}

/// Releases a first call's replay entry if the call ends without keeping an answer.
struct ReplayGuard(Option<ReplayKey>);

impl Drop for ReplayGuard {
    fn drop(&mut self) {
        if let Some(key) = self.0.take() {
            replay_end(&key, None);
        }
    }
}

/// The call itself, with its tool row; (the MCP result, whether a replay may give it again: not after a crash, nor
/// after the caller check refused, since a retry from a live turn must run).
fn run_tool(def: &ToolDef, scope: &keys::Scope, args: &Map<String, Value>, cancelled: &AtomicBool, exposure: exposure::Mode) -> (Value, bool) {
    let call_id = crate::remote_common::random_hex(8);
    let (labels, summary) = (def.labels)(args);
    // tool_call's row is the called tool's own (its inner call_tool draws it)
    let row_to = agents_get(&scope.chat).filter(|_| def.name != "tool_call");
    let row = |status: &str, label: String, links: Vec<Value>| {
        if let Some(agent) = &row_to {
            agent.put(
                "apptool",
                json!({"call": call_id, "tool": def.name, "status": status, "label": label,
                       "summary": {"verb": summary.verb, "one": summary.one, "many": summary.many}, "links": links}),
            );
        }
    };
    row("running", format!("{} {}", labels.running, labels.detail), vec![]);
    let call = Call::new(scope, &call_id, cancelled, exposure);
    let allowed = match (def.access)(args) {
        Access::Reads | Access::ReadsAsCaller => access::read_caller(scope).map(|_| ()),
        Access::WritesThreads | Access::StartsThreads | Access::WritesEnvironment => access::writing_caller(scope).map(|_| ()),
    };
    let refused = allowed.is_err();
    let outcome = match allowed {
        Err(f) => Some(Err(f)),
        Ok(()) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (def.handler)(&call, args))).ok(),
    };
    let links = call.taken_links();
    match outcome {
        Some(Ok(v)) => {
            row("done", format!("{} {}", labels.done, labels.detail), links);
            (ok_result(&v), true)
        }
        Some(Err(f)) => {
            let (status, word) = if f.code == "declined" { ("declined", "Declined") } else { ("failed", "Failed") };
            row(status, format!("{word} to {} {}", lower_first(labels.action), labels.detail), links);
            (fail_result(&f), !refused)
        }
        None => {
            row("failed", format!("Failed to {} {}", lower_first(labels.action), labels.detail), links);
            (crash_result(), false)
        }
    }
}

fn agents_get(id: &str) -> Option<std::sync::Arc<crate::agents::Agent>> {
    crate::agents::get(id)
}

/// Tests: a new open chat on `ai`. The open chats of all tests share `agents::MAX_AGENTS` slots, so this waits (up
/// to a minute) for one to be free rather than fail.
#[cfg(test)]
pub(crate) fn test_chat(ai: &str) -> std::sync::Arc<crate::agents::Agent> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(a) = crate::agents::new_agent(Some(ai)) {
            return a;
        }
        assert!(std::time::Instant::now() < until, "no free chat slot for a minute");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Tests: close a chat a test made (its run marked over first) and revoke its key.
#[cfg(test)]
pub(crate) fn drop_chat(a: &crate::agents::Agent) {
    a.set_running_for_test(false);
    keys::global().revoke_chat(&a.id);
    crate::agents::remove_agent(&a.id);
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
    fn writes(_: &Map<String, Value>) -> Access {
        Access::WritesThreads
    }
    fn starts(_: &Map<String, Value>) -> Access {
        Access::StartsThreads
    }
    fn schema_fail() -> Value {
        json!({"type": "object", "properties": {"code": {"type": "string"}}})
    }
    fn schema_count() -> Value {
        json!({"type": "object", "properties": {"clientRequestId": {"type": "string"}, "tag": {"type": "string"}}})
    }
    fn schema_none() -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn failing(_: &Call, args: &Map<String, Value>) -> Outcome {
        let code = CODES.iter().find(|c| args.get("code").and_then(Value::as_str) == Some(**c)).copied().unwrap_or("invalid_request");
        Err(fail(code, "Test failure."))
    }
    fn bad_labels(_: &Map<String, Value>) -> (Labels, Summary) {
        panic!("test labels panic on purpose")
    }
    fn panicking(_: &Call, _: &Map<String, Value>) -> Outcome {
        panic!("test tool panics on purpose")
    }
    fn counting(_: &Call, _: &Map<String, Value>) -> Outcome {
        Ok(json!({"n": COUNTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1}))
    }
    fn writing(_: &Call, _: &Map<String, Value>) -> Outcome {
        WROTE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"wrote": true}))
    }
    fn waiting(call: &Call, _: &Map<String, Value>) -> Outcome {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !call.cancelled.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(json!({"cancelled": call.cancelled.load(std::sync::atomic::Ordering::SeqCst)}))
    }
    let base = ToolDef {
        name: "test_read",
        title: "Read a chat (test)",
        description: "Test tool.",
        schema: schema_read,
        hints: Hints::READ,
        access: reads,
        labels: labels_read,
        always_load: false,
        handler: read,
    };
    vec![
        base,
        ToolDef { name: "test_fail", schema: schema_fail, handler: failing, ..base },
        ToolDef { name: "test_panic", schema: schema_none, handler: panicking, ..base },
        ToolDef { name: "test_count", schema: schema_count, hints: Hints::WRITE, access: starts, handler: counting, ..base },
        ToolDef { name: "test_write", schema: schema_none, hints: Hints::WRITE, access: writes, handler: writing, ..base },
        ToolDef { name: "test_wait", schema: schema_none, always_load: true, handler: waiting, ..base },
        ToolDef { name: "test_count_badlabels", schema: schema_count, labels: bad_labels, handler: counting, ..base },
    ]
}

#[cfg(test)]
static COUNTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static WROTE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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

    fn scope_for(a: &crate::agents::Agent, namespace: &str) -> keys::Scope {
        keys::Scope { chat: a.id.clone(), ai: a.ai(), subagent: false, namespace: namespace.into() }
    }

    fn apptools(a: &crate::agents::Agent) -> Vec<Value> {
        a.log_snapshot().into_iter().filter(|e| e["kind"] == "apptool").collect()
    }

    fn run(scope: &keys::Scope, name: &str, args: Value) -> Result<Value, (i64, String)> {
        call_tool(scope, name, args, &AtomicBool::new(false), exposure::Mode::Full)
    }

    #[test]
    fn call_tool_puts_running_then_done_with_labels() {
        let a = test_chat("claude");
        let scope = scope_for(&a, "ns-1");
        let got = run(&scope, "test_read", json!({"chatId": null})).unwrap();
        assert_eq!(got, ok_result(&json!({"chatId": a.id})));
        let ev = apptools(&a);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!((ev[0]["status"].as_str(), ev[0]["label"].as_str()), (Some("running"), Some("Reading a chat")));
        assert_eq!((ev[1]["status"].as_str(), ev[1]["label"].as_str()), (Some("done"), Some("Read a chat")));
        assert_eq!(ev[0]["call"], ev[1]["call"]);
        assert!(ev[0]["call"].as_str().is_some_and(|c| !c.is_empty()));
        assert_eq!(ev[1]["tool"], "test_read");
        assert_eq!(ev[1]["summary"], json!({"verb": "Read", "one": "chat", "many": "chats"}));
        assert_eq!(ev[1]["links"], json!([{"kind": "chat", "id": a.id, "title": "Test"}]));
        drop_chat(&a);
    }

    #[test]
    fn failed_and_declined_forms() {
        let a = test_chat("claude");
        let scope = scope_for(&a, "ns-2");
        let got = run(&scope, "test_fail", json!({"code": "invalid_request"})).unwrap();
        assert_eq!(got, fail_result(&fail("invalid_request", "Test failure.")));
        let got = run(&scope, "test_fail", json!({"code": "declined"})).unwrap();
        assert_eq!(got["isError"], true);
        let ev = apptools(&a);
        let ends: Vec<(&str, &str)> = ev.iter().filter(|e| e["status"] != "running").map(|e| (e["status"].as_str().unwrap(), e["label"].as_str().unwrap())).collect();
        assert_eq!(ends, vec![("failed", "Failed to read a chat"), ("declined", "Declined to read a chat")]);
        drop_chat(&a);
    }

    #[test]
    fn a_panic_is_an_internal_error() {
        let a = test_chat("claude");
        let got = run(&scope_for(&a, "ns-3"), "test_panic", json!({})).unwrap();
        assert_eq!(got, crash_result());
        assert_eq!(apptools(&a).last().unwrap()["status"], "failed");
        drop_chat(&a);
    }

    #[test]
    fn unknown_tool_is_method_not_found() {
        let a = test_chat("claude");
        assert_eq!(run(&scope_for(&a, "ns-4"), "nope", json!({})), Err((-32601, "Unknown tool: nope".to_string())));
        drop_chat(&a);
    }

    #[test]
    fn bad_args_are_invalid_params() {
        let a = test_chat("claude");
        let scope = scope_for(&a, "ns-5");
        assert_eq!(run(&scope, "test_read", json!({"zz": 1})), Err((-32602, "Unknown argument \"zz\". Accepted arguments: chatId.".to_string())));
        assert_eq!(run(&scope, "test_read", json!([1])), Err((-32602, "arguments must be a JSON object.".to_string())));
        assert_eq!(apptools(&a).len(), 0, "refused arguments draw no row");
        assert!(run(&scope, "test_read", Value::Null).is_ok(), "no arguments at all is an empty object");
        drop_chat(&a);
    }

    #[test]
    fn client_request_id_replays_within_the_namespace() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let before = COUNTED.load(std::sync::atomic::Ordering::SeqCst);
        let first = run(&scope_for(&a, "ns-6"), "test_count", json!({"clientRequestId": "same", "tag": a.id})).unwrap();
        let again = run(&scope_for(&a, "ns-6"), "test_count", json!({"clientRequestId": "same", "tag": a.id})).unwrap();
        assert_eq!(first, again);
        assert_eq!(COUNTED.load(std::sync::atomic::Ordering::SeqCst) - before, 1, "the handler ran once");
        let other = run(&scope_for(&a, "ns-7"), "test_count", json!({"clientRequestId": "same", "tag": a.id})).unwrap();
        assert_ne!(other, first, "another key's namespace runs it again");
        assert!(IDEMPOTENT.contains(&"chat_launch") && IDEMPOTENT.contains(&"chat_send") && IDEMPOTENT.contains(&"chat_interrupt") && IDEMPOTENT.contains(&"chat_update"));
        drop_chat(&a);
    }

    /// `call_tool` on another thread, given at most 5 s (a replay stuck "running" would block for ever).
    fn run_within_5s(scope: &keys::Scope, name: &'static str, args: Value) -> Result<Value, (i64, String)> {
        let (tx, rx) = std::sync::mpsc::channel();
        let scope = scope.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run(&scope, name, args));
        });
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the call never returned")
    }

    #[test]
    fn a_panic_outside_the_handler_releases_the_replay() {
        let a = test_chat("claude");
        let scope = scope_for(&a, "ns-9");
        let args = json!({"clientRequestId": "boom", "tag": a.id});
        assert_eq!(run_within_5s(&scope, "test_count_badlabels", args.clone()), Ok(crash_result()), "a panic in the labels is an internal error");
        assert_eq!(run_within_5s(&scope, "test_count_badlabels", args), Ok(crash_result()), "the retry runs again instead of waiting for ever");
        drop_chat(&a);
    }

    #[test]
    fn stale_running_replays_are_pruned() {
        let key: ReplayKey = ("ns-10".into(), "test_count".into(), "stale".into());
        mark_running_for_test(&key, std::time::Instant::now().checked_sub(REPLAY_FOR + std::time::Duration::from_secs(1)).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let k = key.clone();
        std::thread::spawn(move || {
            let _ = tx.send(replay_begin(&k));
        });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).expect("blocked on a stale entry"), None, "runs as the first call");
        replay_end(&key, None);
    }

    #[test]
    fn access_refusals_are_not_replayed() {
        let a = test_chat("claude");
        let scope = scope_for(&a, "ns-11");
        let args = json!({"clientRequestId": "later", "tag": a.id});
        let refused = run(&scope, "test_count", args.clone()).unwrap();
        assert!(refused["content"][0]["text"].as_str().unwrap().contains("parent_not_active"));
        a.set_running_for_test(true);
        let ran = run(&scope, "test_count", args).unwrap();
        assert_eq!(ran["isError"], false, "a retry from a live turn runs: {ran}");
        drop_chat(&a);
    }

    #[test]
    fn writes_check_the_live_caller_before_the_handler() {
        let a = test_chat("claude");
        let before = WROTE.load(std::sync::atomic::Ordering::SeqCst);
        let got = run(&scope_for(&a, "ns-8"), "test_write", json!({})).unwrap();
        assert_eq!(got, fail_result(&fail("parent_not_active", "The calling AI no longer owns an active chat run.")));
        assert_eq!(WROTE.load(std::sync::atomic::Ordering::SeqCst), before, "handler not run");
        a.set_running_for_test(true);
        assert_eq!(run(&scope_for(&a, "ns-8"), "test_write", json!({})).unwrap()["isError"], false);
        drop_chat(&a);
    }

    #[test]
    fn uuids_are_version_4() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }
}
