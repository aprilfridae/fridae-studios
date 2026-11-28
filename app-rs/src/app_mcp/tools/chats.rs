//! The chat tools: chat_list, chat_search, chat_read, chat_launch, chat_send, chat_wait, chat_interrupt, chat_update,
//! chat_organize, chat_configure and chat_queue (ports of T3's thread tools onto Fridae's chats).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::{READ_IDEM, chat_chip, cursor_of, cut, max_len, opt_bool, opt_choice, opt_int, opt_text, page, reads, starts_threads, text, words, writes_threads};
use super::projects::{not_found as project_not_found, path_of};
use crate::agents::{self, Agent, Ask};
use crate::app_mcp::{Access, Call, Failure, Hints, Labels, Outcome, Summary, ToolDef, access, chat_link, fail, project_id, run_id};
use crate::{paths, projects};

/// The longest message a tool sends (T3's).
const MAX_MESSAGE: usize = 120_000;
/// chat_read's answer never grows past this many bytes: the oldest items go first.
const READ_BUDGET: usize = 30_000;
/// chat_wait's default and longest wait.
const WAIT_DEFAULT_MS: u64 = 30_000;
const WAIT_MAX_MS: u64 = 45_000;
/// How often chat_wait looks.
const WAIT_POLL: Duration = Duration::from_millis(250);

pub fn defs() -> Vec<ToolDef> {
    let base = ToolDef {
        name: "chat_list",
        title: "List chats",
        description: "List chats newest first, in one project or with `allProjects`. Page with `cursor`.",
        schema: schema_list,
        hints: READ_IDEM,
        access: reads,
        labels: labels_list,
        always_load: false,
        handler: list,
    };
    let write = Hints::WRITE;
    vec![
        base,
        ToolDef {
            name: "chat_search",
            title: "Search chats",
            description: "Search active chats by title, project, then message text. Skips subagent chats. `projectId`: one project, else yours, else all outside a chat. Not exhaustive.",
            schema: schema_search,
            hints: Hints::READ,
            labels: labels_search,
            handler: search,
            ..base
        },
        ToolDef {
            name: "chat_read",
            title: "Read a chat",
            description: "Read any chat by id. View `messages` (default) or `activity` (all items). Newest first; older: `beforePosition=earlierPosition`; from the start: `afterPosition=-1`, then `nextPosition`. Cut-off text: `itemId`, `textOffset=nextTextOffset`. In your reply, name it as its `link` verbatim.",
            schema: schema_read,
            // T3 marks it not read-only (reading a child's result marks it delivered there)
            hints: Hints { read_only: false, destructive: false, idempotent: true, open_world: true },
            labels: labels_read,
            handler: read,
            ..base
        },
        ToolDef {
            name: "chat_launch",
            title: "Start a chat",
            description: "Create a top-level chat in its workspace before starting. Task in `message`. Omit `projectId`, model and modes to inherit yours (`scratch` for no project); modes cannot exceed yours. On error or lost reply, reuse `clientRequestId`. Follow it with `chat_read` or `chat_wait`. Mention it with its `link`.",
            schema: schema_launch,
            hints: write,
            access: starts_threads,
            labels: labels_launch,
            handler: launch,
            ..base
        },
        ToolDef {
            name: "chat_send",
            title: "Send to a chat",
            description: "Send a message to any chat. The target cannot run with broader modes than yours. `mode:'auto'` starts an idle chat or queues behind a running turn. `restart`: stop the running turn and start with this message. In your reply, name it as its `link` verbatim.",
            schema: schema_send,
            hints: write,
            access: writes_threads,
            labels: labels_send,
            handler: send,
            ..base
        },
        ToolDef {
            name: "chat_wait",
            title: "Wait for a chat",
            description: "Wait for a chat's run to finish. No `runId`: latest. Idle: at once. On timeout, with `next` end your turn; without, call again.",
            schema: schema_wait,
            labels: labels_wait,
            handler: wait,
            ..base
        },
        ToolDef {
            name: "chat_interrupt",
            title: "Stop a chat",
            description: "Interrupt a top-level chat's running turn. No `runId`: newest running turn. No effect when idle.",
            schema: schema_interrupt,
            hints: write,
            access: writes_threads,
            labels: labels_interrupt,
            handler: interrupt,
            ..base
        },
        ToolDef {
            name: "chat_update",
            title: "Rename a chat",
            description: "Rename a chat (`action:'rename'` with `title`). Omit `chatId` for this chat. Its project cannot change here.",
            schema: schema_update,
            hints: write,
            access: writes_threads,
            labels: labels_update,
            handler: update,
            ..base
        },
        ToolDef {
            name: "chat_organize",
            title: "Organize a chat",
            description: "Close (archive) or reopen (unarchive) a chat. Omit `chatId` for this chat. Closing this chat applies at turn end. A closed chat stays in your saved chats.",
            schema: schema_organize,
            hints: write,
            access: writes_threads,
            labels: labels_organize,
            handler: organize,
            ..base
        },
        ToolDef {
            name: "chat_configure",
            title: "Configure a chat",
            description: "Read a chat's AI, model, and modes, or set its AI, model, effort, and account with `modelSelection`. Omit `chatId` for this chat. Permission modes stay. Values from `fridae_capabilities`.",
            schema: schema_configure,
            hints: write,
            access: access_configure,
            labels: labels_configure,
            handler: configure,
            ..base
        },
        ToolDef {
            name: "chat_queue",
            title: "Manage waiting messages",
            description: "Manage a chat's queued messages. Omit `chatId` for this chat. list: send order, paged by `cursor` and `limit`. read: up to 16,000 chars of one. cancel. Writes fail once the message is no longer queued.",
            schema: schema_queue,
            hints: write,
            access: access_queue,
            labels: labels_queue,
            handler: queue,
            ..base
        },
    ]
}

// ------------------------------------------------------------------------------------------------------ schemas

fn chat_id_prop() -> Value {
    json!({"type": "string", "description": "A chat's id (chatId), not its link."})
}

fn client_request_id_prop() -> Value {
    json!({"type": "string", "maxLength": 256, "description": "Reuse it to retry the same call. A new call gets a new key."})
}

fn schema_list() -> Value {
    json!({"type": "object", "properties": {
        "projectId": {"type": "string", "description": "A project's id from project_list."},
        "allProjects": {"type": "boolean"},
        "statuses": {"type": "array", "maxItems": 3, "items": {"type": "string", "enum": ["working", "idle", "closed"]}},
        "titleContains": {"type": "string", "maxLength": 200},
        "includeSubagents": {"type": "boolean"},
        "cursor": {"type": "string"},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
    }})
}

fn schema_search() -> Value {
    json!({"type": "object", "properties": {
        "query": {"type": "string", "minLength": 2, "maxLength": 256},
        "limit": {"type": "integer", "minimum": 1, "maximum": 50},
        "projectId": {"type": "string"}
    }, "required": ["query"]})
}

fn schema_read() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "view": {"type": "string", "enum": ["messages", "activity"]},
        "afterPosition": {"type": "integer", "minimum": -1},
        "beforePosition": {"type": "integer", "minimum": 0},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Max 100."},
        "maxCharsPerItem": {"type": "integer", "minimum": 1, "maximum": 20000, "description": "Max 20000."},
        "itemId": {"type": "string"},
        "textOffset": {"type": "integer", "minimum": 0}
    }, "required": ["chatId"]})
}

fn model_selection_prop() -> Value {
    json!({"type": "object", "properties": {
        "ai": {"type": "string", "enum": paths::AIS},
        "model": {"type": "string", "maxLength": 120},
        "effort": {"type": "string", "enum": agents::EFFORTS},
        "account": {"type": "string", "description": "\"\" for the user's own account, else one from fridae_capabilities."}
    }, "additionalProperties": false})
}

fn schema_launch() -> Value {
    json!({"type": "object", "properties": {
        "projectId": {"type": "string", "description": "A project's id from project_list."},
        "scratch": {"type": "boolean", "description": "No project: the chat works in its own folder. Not with `projectId`."},
        "title": {"type": "string", "maxLength": 60},
        "modelSelection": model_selection_prop(),
        "runtimeMode": {"type": "string", "enum": ["approval-required", "full-access", "inherit"]},
        "interactionMode": {"type": "string", "enum": ["default", "inherit"]},
        "message": {"type": "string", "maxLength": MAX_MESSAGE, "description": "First prompt. Omit it for idle."},
        "clientRequestId": client_request_id_prop()
    }, "required": ["title"]})
}

fn schema_send() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "message": {"type": "string", "maxLength": MAX_MESSAGE},
        "mode": {"type": "string", "enum": ["auto", "restart"]},
        "clientRequestId": client_request_id_prop()
    }, "required": ["chatId", "message"]})
}

fn schema_wait() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "runId": {"type": "string"},
        "timeoutMs": {"type": "integer", "minimum": 0, "maximum": WAIT_MAX_MS}
    }, "required": ["chatId"]})
}

fn schema_interrupt() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "runId": {"type": "string"},
        "reason": {"type": "string", "maxLength": 500},
        "stop": {"type": "boolean"},
        "clientRequestId": client_request_id_prop()
    }, "required": ["chatId"]})
}

fn schema_update() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "action": {"type": "string", "enum": ["rename"]},
        "title": {"type": "string", "maxLength": 60},
        "clientRequestId": client_request_id_prop()
    }, "required": ["action", "title"]})
}

fn schema_organize() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "action": {"type": "string", "enum": ["archive", "unarchive"]}
    }, "required": ["action"]})
}

fn schema_configure() -> Value {
    json!({"type": "object", "properties": {"chatId": chat_id_prop(), "modelSelection": model_selection_prop()}})
}

fn schema_queue() -> Value {
    json!({"type": "object", "properties": {
        "chatId": chat_id_prop(),
        "action": {"type": "string", "enum": ["list", "read", "cancel"]},
        "queuedId": {"type": "string"},
        "cursor": {"type": "string"},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
    }, "required": ["action"]})
}

// ------------------------------------------------------------------------------------------- labels and access

fn labels_list(_: &Map<String, Value>) -> (Labels, Summary) {
    words("List", "Listing", "Listed", "chats", "Listed", "chat list", "chat lists")
}

fn labels_search(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Search", "Searching", "Searched", "chats", "Searched", "search", "searches")
}

fn labels_read(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Read", "Reading", "Read", "a chat", "Read", "chat", "chats")
}

fn labels_launch(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Start", "Starting", "Started", "a chat", "Started", "chat", "chats")
}

fn labels_send(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Send", "Sending", "Sent", "a message to a chat", "Sent", "message", "messages")
}

fn labels_wait(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Wait", "Waiting", "Waited", "for a chat", "Waited for", "chat", "chats")
}

fn labels_interrupt(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Stop", "Stopping", "Stopped", "a chat", "Stopped", "chat", "chats")
}

fn labels_update(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Rename", "Renaming", "Renamed", "a chat", "Renamed", "chat", "chats")
}

fn labels_organize(args: &Map<String, Value>) -> (Labels, Summary) {
    match text(args, "action") {
        Some("unarchive") => words("Reopen", "Reopening", "Reopened", "a chat", "Reopened", "chat", "chats"),
        _ => words("Close", "Closing", "Closed", "a chat", "Closed", "chat", "chats"),
    }
}

fn sets(args: &Map<String, Value>) -> bool {
    args.get("modelSelection").is_some_and(|v| !v.is_null())
}

fn access_configure(args: &Map<String, Value>) -> Access {
    if sets(args) { Access::WritesThreads } else { Access::Reads }
}

fn labels_configure(args: &Map<String, Value>) -> (Labels, Summary) {
    if sets(args) {
        words("Change", "Changing", "Changed", "a chat's AI or model", "Changed", "chat", "chats")
    } else {
        words("Read", "Reading", "Read", "a chat's settings", "Read", "chat", "chats")
    }
}

fn access_queue(args: &Map<String, Value>) -> Access {
    if text(args, "action") == Some("cancel") { Access::WritesThreads } else { Access::Reads }
}

fn labels_queue(args: &Map<String, Value>) -> (Labels, Summary) {
    match text(args, "action") {
        Some("read") => words("Read", "Reading", "Read", "a waiting message", "Read", "waiting message", "waiting messages"),
        Some("cancel") => words("Cancel", "Cancelling", "Cancelled", "a waiting message", "Cancelled", "waiting message", "waiting messages"),
        _ => words("List", "Listing", "Listed", "waiting messages", "Listed", "waiting message list", "waiting message lists"),
    }
}

// -------------------------------------------------------------------------------------------------- chat items

fn status_of(a: &Agent) -> &'static str {
    if a.is_working() { "working" } else { "idle" }
}

/// An open chat as chat_list shows it.
fn open_item(a: &Agent) -> Value {
    let title = a.name();
    let limits = access::limits_of(a);
    json!({"chatId": a.id, "projectId": a.project().map(|p| project_id(&p)), "title": title, "link": chat_link(&a.id, &title),
           "status": status_of(a), "latestRunId": run_id(a.turn_started()), "ai": a.ai(), "model": a.model(),
           "runtimeMode": limits.runtime.as_str(), "interactionMode": limits.interaction.as_str(), "parentChatId": a.parent_id(),
           "subagent": a.is_subagent(), "itemCount": a.log_snapshot().len(), "updatedAt": a.last_activity()})
}

/// A saved chat (one of `agents::history()`, or its file) as chat_list shows it.
fn saved_item(c: &Value, items: Option<usize>) -> Value {
    let id = c["id"].as_str().unwrap_or("");
    let title = c["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Chat");
    let subagent = !c["parent"].is_null() || !c["role"].is_null() || !c["team"].is_null();
    json!({"chatId": id, "projectId": c["project"].as_str().map(project_id), "title": title, "link": chat_link(id, title),
           "status": "closed", "latestRunId": null, "ai": c["ai"].as_str().unwrap_or("claude"), "model": c["model"].as_str(),
           "runtimeMode": "approval-required", "interactionMode": "default", "parentChatId": c["parent"].as_str(),
           "subagent": subagent, "itemCount": items.unwrap_or_else(|| c["messages"].as_u64().unwrap_or(0) as usize), "updatedAt": c["updated"].as_f64().unwrap_or(0.0)})
}

/// A saved chat's file, if there is one.
fn saved_file(id: &str) -> Option<Value> {
    let path = paths::work("chats").join(format!("{}.json", paths::base_name(id.trim())));
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn chip(a: &Agent) -> Value {
    chat_chip(&a.id, &a.name())
}

fn link_of(a: &Agent) -> String {
    chat_link(&a.id, &a.name())
}

/// Which project a list or search keeps to: the one named, else (unless all) the caller's, where "no project" is a
/// choice of its own. None: every chat.
fn project_scope(caller: &access::Caller, args: &Map<String, Value>, all: bool) -> Result<Option<Option<String>>, Failure> {
    if let Some(id) = text(args, "projectId") {
        return path_of(id).map(|p| Some(Some(p))).ok_or_else(project_not_found);
    }
    Ok(if all { None } else { Some(caller.agent.project()) })
}

fn in_scope(scope: &Option<Option<String>>, project: Option<&str>) -> bool {
    match scope {
        None => true,
        Some(None) => project.is_none(),
        Some(Some(p)) => project.is_some_and(|q| projects::same(p, q)),
    }
}

// ------------------------------------------------------------------------------------------------------- reads

fn list(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::read_caller(call.scope)?;
    let all = opt_bool(args, "allProjects")?.unwrap_or(false);
    let scope = project_scope(&caller, args, all)?;
    let statuses: Vec<String> = match args.get("statuses") {
        None => vec!["working".into(), "idle".into(), "closed".into()],
        Some(Value::Array(v)) if v.len() <= 3 && v.iter().all(|s| matches!(s.as_str(), Some("working" | "idle" | "closed"))) => v.iter().filter_map(|s| s.as_str().map(str::to_string)).collect(),
        Some(_) => return Err(fail("invalid_request", "`statuses` takes up to 3 of working, idle, closed.")),
    };
    let title_has = opt_text(args, "titleContains")?.map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
    let subagents = opt_bool(args, "includeSubagents")?.unwrap_or(false);
    let limit = opt_int(args, "limit", 1, 100)?.unwrap_or(10) as usize;
    let mut chats: Vec<Value> = agents::agents().iter().filter(|a| in_scope(&scope, a.project().as_deref())).map(|a| open_item(a)).collect();
    chats.extend(agents::history().iter().filter(|c| in_scope(&scope, c["project"].as_str())).map(|c| saved_item(c, None)));
    chats.retain(|c| {
        statuses.iter().any(|s| c["status"] == s.as_str())
            && (subagents || c["subagent"] != true)
            && title_has.as_ref().is_none_or(|t| c["title"].as_str().unwrap_or("").to_lowercase().contains(t))
    });
    chats.sort_by(|a, b| b["updatedAt"].as_f64().unwrap_or(0.0).total_cmp(&a["updatedAt"].as_f64().unwrap_or(0.0)));
    let (shown, next) = page(&chats, cursor_of(args)?, limit);
    let project = match &scope {
        Some(Some(p)) => json!(project_id(p)),
        _ => Value::Null,
    };
    Ok(json!({"projectId": project, "currentChatId": caller.agent.id, "chats": shown, "nextCursor": next, "total": chats.len()}))
}

/// Up to 200 characters of `text` around the first place `q` (lower case) is found in it.
fn snippet(text: &str, q: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(q)?;
    // lower-casing can change byte lengths: then the window starts at the beginning
    let start_char = if lower.len() == text.len() && text.is_char_boundary(at) { text[..at].chars().count().saturating_sub(60) } else { 0 };
    Some(text.chars().skip(start_char).take(200).collect())
}

/// The first message in a log whose text holds `q`: ("user" | "assistant", snippet).
fn text_hit(events: &[Value], q: &str) -> Option<(&'static str, String)> {
    events.iter().find_map(|e| {
        let source = match e["kind"].as_str() {
            Some("user") => "user",
            Some("text") => "assistant",
            _ => return None,
        };
        snippet(e["text"].as_str()?, q).map(|s| (source, s))
    })
}

fn search(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::read_caller(call.scope)?;
    let query = opt_text(args, "query")?.unwrap_or_default();
    let query = query.trim();
    if query.chars().count() < 2 || query.chars().count() > 256 {
        return Err(fail("invalid_request", "`query` must be 2 to 256 characters."));
    }
    let q = query.to_lowercase();
    let limit = opt_int(args, "limit", 1, 50)?.unwrap_or(10) as usize;
    // one project: the one named, else the caller's, else every chat
    let scope = match text(args, "projectId") {
        Some(_) => project_scope(&caller, args, true)?,
        None => caller.agent.project().map(|p| Some(Some(p))).unwrap_or(None),
    };
    let open: Vec<Arc<Agent>> = agents::agents().into_iter().filter(|a| !a.is_subagent() && in_scope(&scope, a.project().as_deref())).collect();
    let saved: Vec<Value> = agents::history()
        .into_iter()
        .filter(|c| c["parent"].is_null() && c["role"].is_null() && c["team"].is_null() && in_scope(&scope, c["project"].as_str()))
        .collect();
    let mut matches: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut push = |id: &str, title: &str, project: Option<&str>, source: &str, snip: Option<String>, matches: &mut Vec<Value>| {
        if seen.iter().any(|s| s == id) {
            return;
        }
        seen.push(id.to_string());
        let mut m = json!({"chatId": id, "projectId": project.map(project_id), "title": title, "link": chat_link(id, title), "source": source});
        if let Some(s) = snip {
            m["snippet"] = json!(s);
        }
        matches.push(m);
    };
    let project_name = |p: Option<&str>| p.map(|p| projects::describe(p)["name"].as_str().unwrap_or("").to_lowercase()).unwrap_or_default();
    // titles and project names first
    for a in &open {
        let (title, project) = (a.name(), a.project());
        if title.to_lowercase().contains(&q) {
            push(&a.id, &title, project.as_deref(), "title", None, &mut matches);
        }
    }
    for c in &saved {
        let (id, title) = (c["id"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or("Chat"));
        if title.to_lowercase().contains(&q) {
            push(id, title, c["project"].as_str(), "title", None, &mut matches);
        }
    }
    for a in &open {
        let project = a.project();
        if project_name(project.as_deref()).contains(&q) {
            push(&a.id, &a.name(), project.as_deref(), "project", None, &mut matches);
        }
    }
    // then the message text of open chats, then of saved ones
    for a in &open {
        if let Some((source, snip)) = text_hit(&a.log_snapshot(), &q) {
            push(&a.id, &a.name(), a.project().as_deref(), source, Some(snip), &mut matches);
        }
    }
    for c in &saved {
        if matches.len() >= limit {
            break;
        }
        let id = c["id"].as_str().unwrap_or("");
        let events = saved_file(id).and_then(|d| d["events"].as_array().cloned()).unwrap_or_default();
        if let Some((source, snip)) = text_hit(&events, &q) {
            push(id, c["name"].as_str().unwrap_or("Chat"), c["project"].as_str(), source, Some(snip), &mut matches);
        }
    }
    matches.truncate(limit);
    Ok(json!({"currentChatId": caller.agent.id, "matches": matches}))
}

/// One log event as a chat_read item, its text from `offset`, at most `max` characters.
fn read_item(position: usize, e: &Value, offset: usize, max: usize) -> Value {
    let full: Option<String> = match e["text"].as_str() {
        Some(t) => Some(t.to_string()),
        None => {
            let mut rest = e.as_object().cloned().unwrap_or_default();
            rest.remove("kind");
            // a card's id is the user's to answer with: no agent reads it
            if matches!(e["kind"].as_str(), Some("appcard" | "appcard_done")) {
                rest.remove("card");
            }
            (!rest.is_empty()).then(|| Value::Object(rest).to_string())
        }
    };
    let mut item = json!({"position": position, "itemId": format!("i{position}"), "type": e["kind"].as_str().unwrap_or(""), "text": null, "textTruncated": false});
    if let Some(full) = full {
        let total = full.chars().count();
        let shown: String = full.chars().skip(offset).take(max).collect();
        let end = offset + shown.chars().count();
        item["text"] = json!(shown);
        if end < total {
            item["textTruncated"] = json!(true);
            item["nextTextOffset"] = json!(end);
        }
    }
    item
}

fn read(call: &Call, args: &Map<String, Value>) -> Outcome {
    access::read_caller(call.scope)?;
    let id = text(args, "chatId").ok_or_else(|| fail("target_required", "Pass chatId."))?;
    let (mut chat, events, closed) = match agents::get(id) {
        Some(a) => {
            let mut item = open_item(&a);
            item["activeRunId"] = if a.is_working() { json!(run_id(a.turn_started())) } else { Value::Null };
            call.links(vec![chip(&a)]);
            (item, a.log_snapshot(), false)
        }
        None => {
            let d = saved_file(id).filter(|d| d["id"].as_str() == Some(paths::base_name(id).as_str())).ok_or_else(|| access::chat_not_found(id))?;
            let events: Vec<Value> = d["events"].as_array().cloned().unwrap_or_default().into_iter().filter(|e| e["kind"].is_string()).collect();
            let messages = events.iter().filter(|e| e["kind"] == "user").count();
            let summary = json!({"id": d["id"], "name": d["name"], "project": d["project"], "ai": d["ai"], "model": d["model"], "parent": d["parent"], "role": d["role"], "team": d["team"], "updated": d["updated"], "messages": messages});
            let mut item = saved_item(&summary, Some(events.len()));
            item["activeRunId"] = Value::Null;
            call.links(vec![chat_chip(item["chatId"].as_str().unwrap_or(""), item["title"].as_str().unwrap_or(""))]);
            (item, events, true)
        }
    };
    chat["closed"] = json!(closed);
    let view = opt_choice(args, "view", &["messages", "activity"])?.unwrap_or_else(|| "messages".into());
    let limit = opt_int(args, "limit", 1, 100)?.unwrap_or(20) as usize;
    let max = opt_int(args, "maxCharsPerItem", 1, 20_000)?.unwrap_or(4_000) as usize;
    let after = opt_int(args, "afterPosition", -1, i64::MAX)?;
    let before = opt_int(args, "beforePosition", 0, i64::MAX)?;
    let offset = opt_int(args, "textOffset", 0, i64::MAX)?.unwrap_or(0) as usize;
    let viewed: Vec<(usize, &Value)> = events.iter().enumerate().filter(|(_, e)| view == "activity" || matches!(e["kind"].as_str(), Some("user" | "text" | "error"))).collect();
    // which items, and whether later ones are left
    let (mut picked, mut has_more, newest_first): (Vec<(usize, &Value)>, bool, bool) = if let Some(item_id) = text(args, "itemId") {
        let pos = item_id.strip_prefix('i').and_then(|p| p.parse::<usize>().ok());
        let hit = viewed.iter().find(|(p, _)| Some(*p) == pos).copied().ok_or_else(|| fail("invalid_request", "That item is not in this chat's view. Read the chat again for item ids."))?;
        (vec![hit], false, false)
    } else if let Some(a) = after {
        let later: Vec<(usize, &Value)> = viewed.iter().filter(|(p, _)| *p as i64 > a).copied().collect();
        let more = later.len() > limit;
        (later.into_iter().take(limit).collect(), more, false)
    } else {
        let upto = before.map(|b| b as usize).unwrap_or(usize::MAX);
        let earlier: Vec<(usize, &Value)> = viewed.iter().filter(|(p, _)| *p < upto).copied().collect();
        let skip = earlier.len().saturating_sub(limit);
        (earlier.into_iter().skip(skip).collect(), false, true)
    };
    let item_offset = if args.contains_key("itemId") { offset } else { 0 };
    let shape = |picked: &[(usize, &Value)], has_more: bool| -> Value {
        let items: Vec<Value> = picked.iter().map(|(p, e)| read_item(*p, e, item_offset, max)).collect();
        let earlier = picked.first().filter(|(first, _)| viewed.iter().any(|(p, _)| p < first)).map(|(p, _)| json!(p)).unwrap_or(Value::Null);
        json!({"chat": chat, "items": items, "nextPosition": picked.last().map(|(p, _)| json!(p)).unwrap_or(Value::Null), "hasMore": has_more, "earlierPosition": earlier})
    };
    let mut out = shape(&picked, has_more);
    // the answer stays within its budget: the oldest items go (the newest, reading forward)
    while out.to_string().len() > READ_BUDGET && !picked.is_empty() {
        if newest_first {
            picked.remove(0);
        } else {
            picked.pop();
        }
        has_more = true;
        out = shape(&picked, has_more);
    }
    Ok(out)
}

/// chat_wait's wait: the default when none is given, never past the longest (tests: at most 300 ms).
pub(crate) fn wait_ms(asked: Option<i64>) -> u64 {
    asked.map(|ms| ms.clamp(0, WAIT_MAX_MS as i64) as u64).unwrap_or(WAIT_DEFAULT_MS)
}

fn wait(call: &Call, args: &Map<String, Value>) -> Outcome {
    access::read_caller(call.scope)?;
    let id = text(args, "chatId").ok_or_else(|| fail("target_required", "Pass chatId."))?;
    let timeout = wait_ms(args.get("timeoutMs").and_then(Value::as_i64));
    let timeout = Duration::from_millis(if cfg!(test) { timeout.min(300) } else { timeout });
    let Some(target) = agents::get(id) else {
        // a closed chat runs nothing
        return match saved_file(id) {
            Some(_) => Ok(json!({"chatId": paths::base_name(id), "runId": null, "status": "idle", "timedOut": false})),
            None => Err(access::chat_not_found(id)),
        };
    };
    call.links(vec![chip(&target)]);
    let answer = |timed_out: bool| {
        let mut out = json!({"chatId": target.id, "runId": run_id(target.turn_started()), "status": status_of(&target), "timedOut": timed_out});
        if timed_out && target.studio_lead_loop() {
            out["next"] = json!({"action": "end_turn", "why": "It keeps working on its own. Its replies stay in that chat: read them later with chat_read."});
        }
        out
    };
    if let Some(run) = text(args, "runId") {
        if run_id(target.turn_started()).as_deref() != Some(run) {
            return Ok(answer(false));
        }
    }
    let start = Instant::now();
    loop {
        if !target.is_working() {
            return Ok(answer(false));
        }
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() || call.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(answer(true));
        }
        std::thread::sleep(left.min(WAIT_POLL));
    }
}

// ------------------------------------------------------------------------------------------------------ writes

/// A chat's model selection: AI, model, effort and the account it runs on ("" the user's own).
fn selection_of(a: &Agent) -> Value {
    let ask = a.last_ask();
    json!({"ai": a.ai(), "model": a.model(), "effort": ask.effort, "account": account_of(a)})
}

/// The account a chat runs on, by name ("" for the user's own), from its latest account pick (the chat shows it by
/// its place, `account_pick`).
fn account_of(a: &Agent) -> String {
    let place = a.log_snapshot().iter().rev().find(|e| e["kind"] == "account_pick").and_then(|e| e["account"].as_u64()).unwrap_or(1);
    if place <= 1 {
        return String::new();
    }
    let state = crate::accounts::state();
    state[a.ai().as_str()]["accounts"].as_array().into_iter().flatten().find(|x| x["n"].as_u64() == Some(place)).and_then(|x| x["name"].as_str()).unwrap_or("").to_string()
}

/// A `modelSelection` argument, checked: (ai, model, effort, account), each None when not given.
#[allow(clippy::type_complexity)]
fn selection_arg(args: &Map<String, Value>) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>), Failure> {
    let Some(v) = args.get("modelSelection") else {
        return Ok((None, None, None, None));
    };
    let Some(sel) = v.as_object() else {
        return Err(fail("invalid_request", "`modelSelection` must be an object."));
    };
    if let Some(k) = sel.keys().find(|k| !matches!(k.as_str(), "ai" | "model" | "effort" | "account")) {
        return Err(fail("invalid_request", format!("Unknown modelSelection field \"{}\". Use ai, model, effort and account.", cut(k, 64))));
    }
    let ai = opt_text(sel, "ai")?.map(|a| a.trim().to_string());
    if let Some(a) = &ai {
        if !paths::AIS.contains(&a.as_str()) {
            return Err(fail("invalid_request", format!("That AI is not one Fridae offers here. Use one of: {}.", paths::AIS.join(", "))));
        }
    }
    let model = opt_text(sel, "model")?.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    if let Some(m) = &model {
        if !crate::expert_ai::model_id_ok(m) {
            return Err(fail("invalid_request", "That model id is not one an AI takes. fridae_capabilities lists them."));
        }
    }
    let effort = opt_choice(sel, "effort", &agents::EFFORTS)?;
    let account = opt_text(sel, "account")?.map(|a| a.trim().to_string());
    Ok((ai, model, effort, account))
}

/// Waits (up to `most`) for a chat's turn to have started after `before`; its run id then.
fn started_run(a: &Agent, before: f64, most: Duration) -> Option<String> {
    let end = Instant::now() + most;
    while a.turn_started() <= before && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    (a.turn_started() > before).then(|| run_id(a.turn_started())).flatten()
}

fn launch(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::writing_caller(call.scope)?;
    let title = text(args, "title").ok_or_else(|| fail("invalid_request", "Pass a title."))?.to_string();
    let runtime = access::resolve_runtime(caller.limits.runtime, opt_text(args, "runtimeMode")?.as_deref())?;
    access::resolve_interaction(caller.limits.interaction, opt_text(args, "interactionMode")?.as_deref())?;
    let message = opt_text(args, "message")?.filter(|m| !m.trim().is_empty());
    if let Some(m) = &message {
        max_len("message", m, MAX_MESSAGE)?;
    }
    let scratch = opt_bool(args, "scratch")?.unwrap_or(false);
    let project = match (text(args, "projectId"), scratch) {
        (Some(_), true) => return Err(fail("invalid_request", "Pass projectId or scratch, not both.")),
        (Some(id), false) => Some(path_of(id).ok_or_else(project_not_found)?),
        (None, true) => None,
        (None, false) => caller.agent.project(),
    };
    let (ai, model, effort, account) = selection_arg(args)?;
    let ai = ai.unwrap_or_else(|| caller.agent.ai());
    // the caller's model and effort go with its AI
    let same_ai = ai == caller.agent.ai();
    let model = model.or_else(|| if same_ai { caller.agent.model() } else { None });
    let effort = effort.or_else(|| if same_ai { caller.agent.last_ask().effort } else { None });
    let full = runtime == access::Runtime::FullAccess;
    let chat = agents::new_agent(Some(&ai)).ok_or_else(|| fail("capability_denied", "Fridae is at its chat limit. Close a chat first."))?;
    chat.rename(&title);
    if let Some(p) = &project {
        chat.set_project(p);
    }
    chat.set_pick(model.as_deref(), effort.as_deref());
    if let Some(acc) = &account {
        if let Err(e) = chat.pick_account(acc) {
            agents::remove_agent(&chat.id);
            return Err(fail("invalid_request", e));
        }
    }
    chat.set_mode_for_launch(full);
    call.links(vec![chip(&chat)]);
    let run = match &message {
        Some(m) => {
            let (a, m, ask) = (chat.clone(), m.clone(), Ask { full, model: model.clone(), effort: effort.clone(), ..Default::default() });
            std::thread::spawn(move || a.ask(&m, &ask));
            started_run(&chat, 0.0, Duration::from_secs(1))
        }
        None => None,
    };
    Ok(json!({"chatId": chat.id, "link": link_of(&chat), "projectId": project.as_deref().map(project_id), "modelSelection": selection_of(&chat),
              "runId": run, "status": if message.is_some() { "running" } else { "idle" }}))
}

fn send(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::writing_caller(call.scope)?;
    let target = access::resolve_chat(&caller, text(args, "chatId"), false)?;
    if target.id == caller.agent.id {
        return Err(fail("invalid_request", "A chat cannot send to itself."));
    }
    access::target_within(&caller, &target)?;
    let message = opt_text(args, "message")?.filter(|m| !m.trim().is_empty()).ok_or_else(|| fail("invalid_request", "Pass a message."))?;
    max_len("message", &message, MAX_MESSAGE)?;
    let restart = opt_choice(args, "mode", &["auto", "restart"])?.as_deref() == Some("restart");
    call.links(vec![chip(&target)]);
    // the target's own mode, never the caller's
    let own = target.last_ask();
    let ask = Ask { full: own.full, studio: own.studio, model: own.model.clone(), effort: own.effort.clone(), ..Default::default() };
    let mut restarted = false;
    if restart && target.is_working() {
        target.stop();
        crate::studio_undo::pause_if_lead(&target);
        let end = Instant::now() + Duration::from_secs(5);
        while target.is_working() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        restarted = true;
    }
    let before = target.turn_started();
    let delivery = if target.is_working() {
        // held behind the running turn (returns at once)
        target.ask(&message, &ask);
        "queued"
    } else {
        let (a, m) = (target.clone(), message.clone());
        std::thread::spawn(move || a.ask(&m, &ask));
        if restarted { "restarted" } else { "started" }
    };
    let run = if delivery == "queued" { run_id(target.turn_started()) } else { started_run(&target, before, Duration::from_secs(1)) };
    Ok(json!({"chatId": target.id, "link": link_of(&target), "runId": run, "status": status_of(&target), "delivery": delivery}))
}

fn interrupt(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::writing_caller(call.scope)?;
    let target = access::resolve_chat(&caller, text(args, "chatId"), false)?;
    access::target_within(&caller, &target)?;
    opt_text(args, "reason")?;
    opt_bool(args, "stop")?;
    call.links(vec![chip(&target)]);
    let current = run_id(target.turn_started());
    if !target.is_working() {
        return Ok(json!({"chatId": target.id, "runId": null, "status": "no_active_run"}));
    }
    if let Some(run) = text(args, "runId") {
        if current.as_deref() != Some(run) {
            return Err(fail("run_not_found", "That run is not the chat's running turn. Leave runId out for the newest."));
        }
    }
    // as the window's Stop: a studio run's Lead is paused too
    target.stop();
    crate::studio_undo::pause_if_lead(&target);
    Ok(json!({"chatId": target.id, "runId": current, "status": "interrupt_requested"}))
}

fn update(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::writing_caller(call.scope)?;
    opt_choice(args, "action", &["rename"])?.ok_or_else(|| fail("invalid_request", "Pass action: 'rename'."))?;
    let target = access::resolve_chat(&caller, text(args, "chatId"), true)?;
    access::target_within(&caller, &target)?;
    let title = text(args, "title").ok_or_else(|| fail("invalid_request", "Pass a title."))?;
    if !target.rename(title) {
        return Err(fail("invalid_request", "Pass a title."));
    }
    call.links(vec![chip(&target)]);
    Ok(json!({"chatId": target.id, "link": link_of(&target), "title": target.name()}))
}

fn organize(call: &Call, args: &Map<String, Value>) -> Outcome {
    let caller = access::writing_caller(call.scope)?;
    let action = opt_choice(args, "action", &["archive", "unarchive"])?.ok_or_else(|| fail("invalid_request", "Pass action: 'archive' or 'unarchive'."))?;
    if action == "unarchive" {
        let id = text(args, "chatId").map(paths::base_name).unwrap_or_else(|| caller.agent.id.clone());
        let chat = match agents::get(&id) {
            Some(open) => open,
            None => {
                if saved_file(&id).is_none() {
                    return Err(access::chat_not_found(&id));
                }
                agents::open_chat(&id).ok_or_else(|| fail("capability_denied", "Fridae is at its chat limit. Close a chat first."))?
            }
        };
        call.links(vec![chip(&chat)]);
        return Ok(json!({"chatId": chat.id, "link": link_of(&chat), "closed": false}));
    }
    let target = access::resolve_chat(&caller, text(args, "chatId"), true)?;
    access::target_within(&caller, &target)?;
    call.links(vec![chip(&target)]);
    if target.id == caller.agent.id {
        // this chat: closed once its turn is over (the turn that called this is still running)
        let me = target.clone();
        std::thread::spawn(move || {
            while me.is_working() && !me.is_closed() {
                std::thread::sleep(Duration::from_millis(250));
            }
            if !me.is_closed() {
                agents::remove_agent(&me.id);
            }
        });
        return Ok(json!({"chatId": target.id, "link": link_of(&target), "closesWhenTurnEnds": true}));
    }
    agents::remove_agent(&target.id);
    Ok(json!({"chatId": target.id, "link": link_of(&target), "closed": true}))
}

fn configure(call: &Call, args: &Map<String, Value>) -> Outcome {
    let setting = sets(args);
    let caller = if setting { access::writing_caller(call.scope)? } else { access::read_caller(call.scope)? };
    let target = access::resolve_chat(&caller, text(args, "chatId"), true)?;
    call.links(vec![chip(&target)]);
    if setting {
        access::target_within(&caller, &target)?;
        let (ai, model, effort, account) = selection_arg(args)?;
        let was = target.ai();
        let ai = ai.unwrap_or_else(|| was.clone());
        if ai != was {
            target.switch_ai(&ai).map_err(|e| fail("thread_not_sendable", e))?;
        }
        // a model or effort not given is kept while the AI stays (a new AI starts on its own defaults)
        let kept = target.last_ask();
        let same = ai == was;
        let model = model.or_else(|| if same { kept.model.clone() } else { None });
        let effort = effort.or_else(|| if same { kept.effort.clone() } else { None });
        target.set_pick(model.as_deref(), effort.as_deref());
        if let Some(acc) = &account {
            target.pick_account(acc).map_err(|e| fail("invalid_request", e))?;
        }
    }
    let limits = access::limits_of(&target);
    Ok(json!({"chatId": target.id, "modelSelection": selection_of(&target), "runtimeMode": limits.runtime.as_str(), "interactionMode": limits.interaction.as_str()}))
}

fn queue(call: &Call, args: &Map<String, Value>) -> Outcome {
    let action = opt_choice(args, "action", &["list", "read", "cancel"])?.ok_or_else(|| fail("invalid_request", "Pass action: list, read or cancel."))?;
    let caller = if action == "cancel" { access::writing_caller(call.scope)? } else { access::read_caller(call.scope)? };
    let target = access::resolve_chat(&caller, text(args, "chatId"), true)?;
    call.links(vec![chip(&target)]);
    let items = target.queue_items();
    let gone = || fail("run_not_found", "That message is no longer queued.");
    let qid = || text(args, "queuedId").ok_or_else(|| fail("invalid_request", "Pass queuedId."));
    match action.as_str() {
        "list" => {
            let limit = opt_int(args, "limit", 1, 100)?.unwrap_or(20) as usize;
            let all: Vec<Value> = items.iter().map(|(id, t)| json!({"queuedId": id, "text": t.chars().take(200).collect::<String>(), "truncated": t.chars().count() > 200})).collect();
            let (shown, next) = page(&all, cursor_of(args)?, limit);
            Ok(json!({"items": shown, "nextCursor": next}))
        }
        "read" => {
            let id = qid()?;
            let (_, t) = items.iter().find(|(q, _)| q == id).ok_or_else(gone)?;
            Ok(json!({"queuedId": id, "text": t.chars().take(16_000).collect::<String>(), "truncated": t.chars().count() > 16_000}))
        }
        _ => {
            access::target_within(&caller, &target)?;
            let id = qid()?;
            if !target.unqueue(id) {
                return Err(gone());
            }
            Ok(json!({"chatId": target.id, "queuedId": id, "cancelled": true}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use crate::agents::{self, Ask, BUS};
    use crate::app_mcp::{chat_link, drop_chat, test_chat};
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn launch_ok(a: &Arc<agents::Agent>, args: Value) -> Value {
        // the chats of all tests share the app's slots: a full app is retried, with a new request id each time
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let r = call(a, "chat_launch", args.clone());
            if r["isError"] == true && err(&r).0 == "capability_denied" && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            return ok(&r);
        }
    }

    /// A live caller (Ask me) in a project: the project is set before its turn "runs".
    fn caller_in(dir: &str) -> Arc<agents::Agent> {
        let a = test_chat("claude");
        assert!(a.set_project(dir));
        a.set_running_for_test(true);
        a
    }

    fn drop_id(id: &str) {
        if let Some(c) = agents::get(id) {
            drop_chat(&c);
        }
    }

    #[test]
    fn chat_list_newest_first_and_pages() {
        let _serial = serial();
        let dir = project_dir("chatlist");
        let a = caller(false);
        let b = test_chat("claude");
        assert!(b.set_project(&dir));
        b.rename("First");
        std::thread::sleep(std::time::Duration::from_millis(30));
        let c = test_chat("claude");
        assert!(c.set_project(&dir));
        c.rename("Second");
        let pid = crate::app_mcp::project_id(&dir);
        let first = ok(&call(&a, "chat_list", json!({"projectId": pid, "limit": 1})));
        assert_eq!(first["total"], 2);
        assert_eq!(first["projectId"], pid.as_str());
        assert_eq!(first["chats"][0]["chatId"], c.id.as_str(), "newest first");
        assert_eq!(first["chats"][0]["link"], chat_link(&c.id, "Second"));
        assert_eq!(first["chats"][0]["status"], "idle");
        assert_eq!(first["currentChatId"], a.id.as_str());
        let next = ok(&call(&a, "chat_list", json!({"projectId": pid, "limit": 1, "cursor": first["nextCursor"]})));
        assert_eq!(next["chats"][0]["chatId"], b.id.as_str());
        assert!(next["nextCursor"].is_null());
        b.set_running_for_test(true);
        let working = ok(&call(&a, "chat_list", json!({"projectId": pid, "statuses": ["working"]})));
        assert_eq!(working["total"], 1);
        assert_eq!(working["chats"][0]["status"], "working");
        // the caller's own project by default (it has none: the chats with no project)
        let mine = ok(&call(&a, "chat_list", json!({"limit": 100})));
        assert!(mine["chats"].as_array().unwrap().iter().all(|x| x["projectId"].is_null()));
        drop_chat(&b);
        drop_chat(&c);
        drop_chat(&a);
        crate::projects::forget(&dir);
    }

    #[test]
    fn chat_list_marks_closed_and_hides_subagents_by_default() {
        let _serial = serial();
        let dir = project_dir("chatclosed");
        let a = caller_in(&dir);
        let gone = test_chat("claude");
        assert!(gone.set_project(&dir));
        gone.put("user", json!({"text": "hello there"})); // a chat with a message is kept when closed
        let gone_id = gone.id.clone();
        drop_chat(&gone);
        let helper = agents::spawn_expert("team-list", "worker", "Helper: job", Some(dir.clone()), Some("claude".into()), Some(&a.id), agents::Expert { id: "x".into(), name: "X".into(), text: String::new() }).unwrap();
        let got = ok(&call(&a, "chat_list", json!({})));
        let ids: Vec<&str> = got["chats"].as_array().unwrap().iter().map(|c| c["chatId"].as_str().unwrap()).collect();
        assert!(ids.contains(&gone_id.as_str()) && !ids.contains(&helper.id.as_str()), "{ids:?}");
        let closed = got["chats"].as_array().unwrap().iter().find(|c| c["chatId"] == gone_id.as_str()).unwrap();
        assert_eq!(closed["status"], "closed");
        let with = ok(&call(&a, "chat_list", json!({"includeSubagents": true})));
        let sub = with["chats"].as_array().unwrap().iter().find(|c| c["chatId"] == helper.id.as_str()).expect("listed when asked");
        assert_eq!((sub["subagent"].as_bool(), sub["parentChatId"].as_str()), (Some(true), Some(a.id.as_str())));
        drop_chat(&helper);
        drop_chat(&a);
        let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{gone_id}.json")));
        crate::projects::forget(&dir);
    }

    #[test]
    fn chat_search_titles_before_text_skips_subagents() {
        let _serial = serial();
        let a = caller(false);
        let word = format!("zebra{}", a.id.replace('-', ""));
        let by_text = test_chat("claude");
        by_text.put("user", json!({"text": format!("please find the {word} in here")}));
        let by_title = test_chat("claude");
        by_title.rename(&format!("About {word}"));
        let helper = agents::spawn_expert("team-search", "worker", &format!("H {word}"), None, Some("claude".into()), Some(&a.id), agents::Expert { id: "y".into(), name: "Y".into(), text: String::new() }).unwrap();
        let got = ok(&call(&a, "chat_search", json!({"query": word})));
        let m = got["matches"].as_array().unwrap();
        let ids: Vec<&str> = m.iter().map(|x| x["chatId"].as_str().unwrap()).collect();
        assert_eq!(ids, [by_title.id.as_str(), by_text.id.as_str()], "{got}");
        assert_eq!(m[0]["source"], "title");
        assert_eq!(m[1]["source"], "user");
        assert!(m[1]["snippet"].as_str().unwrap().contains(&word) && m[1]["snippet"].as_str().unwrap().chars().count() <= 200);
        assert_eq!(err(&call(&a, "chat_search", json!({"query": "z"}))).0, "invalid_request");
        drop_chat(&helper);
        for c in [&by_text, &by_title] {
            let id = c.id.clone();
            drop_chat(c);
            let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{id}.json")));
        }
        drop_chat(&a);
    }

    #[test]
    fn chat_read_never_shows_a_card_id() {
        // a card is answered only by the user, but its id stays out of what any agent can read
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.put("appcard", json!({"card": "cardsecret01", "title": "Rename project", "text": "Rename it.", "allow": "Rename it"}));
        t.put("appcard_done", json!({"card": "cardsecret01", "title": "Rename project", "outcome": "declined"}));
        let got = ok(&call(&a, "chat_read", json!({"chatId": t.id, "view": "activity"})));
        assert_eq!(got["items"].as_array().unwrap().len(), 2, "{got}");
        assert!(!got.to_string().contains("cardsecret01"), "{got}");
        assert!(got.to_string().contains("declined"), "{got}");
        crate::app_mcp::drop_chat(&t);
        crate::app_mcp::drop_chat(&a);
    }

    #[test]
    fn chat_read_pages_and_caps_at_30000_bytes() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.put("user", json!({"text": "start"}));
        for i in 0..200 {
            t.put("text", json!({"text": format!("{i:03} {}", "x".repeat(1000))}));
        }
        let got = ok(&call(&a, "chat_read", json!({"chatId": t.id, "limit": 100})));
        assert!(got.to_string().len() <= 30_000, "{}", got.to_string().len());
        assert_eq!(got["hasMore"], true);
        let items = got["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.last().unwrap()["text"].as_str().unwrap().starts_with("199 "), "newest last in the page");
        assert!(got["earlierPosition"].is_number());
        let older = ok(&call(&a, "chat_read", json!({"chatId": t.id, "beforePosition": got["earlierPosition"], "limit": 5})));
        let older_items = older["items"].as_array().unwrap();
        assert_eq!(older_items.len(), 5);
        assert!(older_items.last().unwrap()["position"].as_i64() < got["earlierPosition"].as_i64());
        let from_start = ok(&call(&a, "chat_read", json!({"chatId": t.id, "afterPosition": -1, "limit": 2})));
        assert_eq!(from_start["items"][0]["text"], "start");
        assert_eq!(from_start["items"][0]["type"], "user");
        let cut = ok(&call(&a, "chat_read", json!({"chatId": t.id, "afterPosition": -1, "limit": 2, "maxCharsPerItem": 10})));
        assert_eq!(cut["items"][1]["textTruncated"], true);
        assert_eq!(cut["items"][1]["nextTextOffset"], 10);
        let item = ok(&call(&a, "chat_read", json!({"chatId": t.id, "itemId": cut["items"][1]["itemId"], "textOffset": 10, "maxCharsPerItem": 5})));
        assert_eq!(item["items"][0]["text"], "xxxxx");
        assert_eq!(item["chat"]["link"], chat_link(&t.id, &t.name()));
        assert_eq!(err(&call(&a, "chat_read", json!({"chatId": "c-nope"}))).0, "thread_not_found");
        // a saved (closed) chat reads from its file
        let id = t.id.clone();
        drop_chat(&t);
        let saved = ok(&call(&a, "chat_read", json!({"chatId": id, "limit": 1})));
        assert_eq!(saved["chat"]["closed"], true);
        assert!(saved["items"][0]["text"].as_str().unwrap().starts_with("199 "));
        let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{id}.json")));
        drop_chat(&a);
    }

    #[test]
    fn chat_launch_makes_a_sidebar_chat() {
        let _serial = serial();
        let dir = project_dir("launch");
        crate::projects::add(&dir, Some("plain")).unwrap();
        let pid = crate::app_mcp::project_id(&dir);
        let a = caller(false);
        let n = BUS.after(0, 0.0).len();
        let got = launch_ok(&a, json!({"title": "Build the shop", "projectId": pid, "message": "Make a shop."}));
        let id = got["chatId"].as_str().unwrap().to_string();
        assert_eq!(got["link"], chat_link(&id, "Build the shop"));
        assert_eq!(got["projectId"], pid.as_str());
        let ev: Vec<Value> = BUS.after(n, 0.0).into_iter().filter(|e| e["agent"] == id.as_str()).collect();
        assert!(ev.iter().any(|e| e["kind"] == "agent_new"));
        assert!(ev.iter().any(|e| e["kind"] == "renamed" && e["name"] == "Build the shop"));
        assert!(ev.iter().any(|e| e["kind"] == "project" && e["path"] == dir.as_str()));
        let c = agents::get(&id).unwrap();
        until("the first message", || c.last_user_text_for_test().as_deref() == Some("Make a shop."));
        let row = a.log_snapshot().into_iter().rev().find(|e| e["kind"] == "apptool").unwrap();
        assert_eq!(row["links"][0], json!({"kind": "chat", "id": id, "title": "Build the shop"}));
        drop_id(&id);
        let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{id}.json")));
        drop_chat(&a);
        crate::projects::forget(&dir);
    }

    #[test]
    fn chat_launch_inherits_and_never_widens() {
        let _serial = serial();
        let low = caller(false);
        let (code, _) = err(&call(&low, "chat_launch", json!({"title": "Wide", "runtimeMode": "full-access"})));
        assert_eq!(code, "runtime_mode_escalation_denied");
        let full = caller(true);
        let got = launch_ok(&full, json!({"title": "Inherits", "scratch": true}));
        let id = got["chatId"].as_str().unwrap().to_string();
        assert!(agents::get(&id).unwrap().last_ask().full, "an idle launched chat keeps the caller's mode");
        assert_eq!(got["status"], "idle");
        assert!(got["projectId"].is_null());
        drop_id(&id);
        let got = launch_ok(&full, json!({"title": "Lower", "scratch": true, "runtimeMode": "approval-required"}));
        let id = got["chatId"].as_str().unwrap().to_string();
        assert!(!agents::get(&id).unwrap().last_ask().full);
        drop_id(&id);
        drop_chat(&low);
        drop_chat(&full);
    }

    #[test]
    fn chat_launch_replays_client_request_id() {
        let _serial = serial();
        let a = caller(false);
        let before = agents::agents().len();
        let req = format!("req-{}", a.id);
        let first = launch_ok(&a, json!({"title": "Once", "scratch": true, "clientRequestId": req}));
        let again = ok(&call(&a, "chat_launch", json!({"title": "Once", "scratch": true, "clientRequestId": req})));
        assert_eq!(first["chatId"], again["chatId"]);
        assert!(agents::agents().len() <= before + 1, "one chat made for two calls");
        drop_id(first["chatId"].as_str().unwrap());
        drop_chat(&a);
    }

    #[test]
    fn chat_send_refuses_a_broader_target() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.set_last_ask_for_test(Ask { full: true, ..Default::default() });
        let (code, msg) = err(&call(&a, "chat_send", json!({"chatId": t.id, "message": "hi"})));
        assert_eq!(code, "runtime_mode_escalation_denied");
        assert!(msg.contains("above this caller's approval-required/default limit") && msg.contains("This call does not create an approval request."), "{msg}");
        let (code, msg) = err(&call(&a, "chat_send", json!({"chatId": a.id, "message": "me"})));
        assert_eq!((code.as_str(), msg.as_str()), ("invalid_request", "A chat cannot send to itself."));
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_send_queues_when_busy() {
        let _serial = serial();
        let a = caller(true);
        let t = test_chat("claude");
        t.set_running_for_test(true);
        let got = ok(&call(&a, "chat_send", json!({"chatId": t.id, "message": "after this"})));
        assert_eq!(got["delivery"], "queued");
        assert_eq!(got["link"], chat_link(&t.id, &t.name()));
        assert_eq!(t.queued(), 1);
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_send_uses_the_targets_mode() {
        let _serial = serial();
        let a = caller(true);
        let t = test_chat("claude");
        let got = ok(&call(&a, "chat_send", json!({"chatId": t.id, "message": "do it your way"})));
        assert_eq!(got["delivery"], "started");
        until("the message", || t.last_user_text_for_test().as_deref() == Some("do it your way"));
        assert!(!t.last_ask().full, "the target's own mode, never the caller's Full auto");
        let id = t.id.clone();
        drop_chat(&t);
        let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{id}.json")));
        drop_chat(&a);
    }

    #[test]
    fn chat_wait_times_out_at_the_cap() {
        let _serial = serial();
        assert_eq!(super::wait_ms(Some(999_999)), 45_000);
        assert_eq!(super::wait_ms(None), 30_000);
        let a = caller(false);
        let t = test_chat("claude");
        t.set_running_for_test(true);
        let start = std::time::Instant::now();
        let got = ok(&call(&a, "chat_wait", json!({"chatId": t.id, "timeoutMs": 999_999})));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!((got["status"].as_str(), got["timedOut"].as_bool()), (Some("working"), Some(true)));
        assert!(got.get("next").is_none());
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_wait_idle_at_once() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        let got = ok(&call(&a, "chat_wait", json!({"chatId": t.id})));
        assert_eq!((got["status"].as_str(), got["timedOut"].as_bool()), (Some("idle"), Some(false)));
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_wait_next_for_a_studio_lead() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.set_last_ask_for_test(Ask { studio: true, ..Default::default() });
        t.set_running_for_test(true);
        let got = ok(&call(&a, "chat_wait", json!({"chatId": t.id, "timeoutMs": 100})));
        assert_eq!(got["timedOut"], true);
        assert_eq!(got["next"]["action"], "end_turn");
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_interrupt_stops_and_reports_no_active_run_when_idle() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        let got = ok(&call(&a, "chat_interrupt", json!({"chatId": t.id})));
        assert_eq!(got["status"], "no_active_run");
        t.set_running_for_test(true);
        let got = ok(&call(&a, "chat_interrupt", json!({"chatId": t.id})));
        assert_eq!(got["status"], "interrupt_requested");
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_update_renames_this_chat_by_default() {
        let _serial = serial();
        let a = caller(false);
        let n = BUS.after(0, 0.0).len();
        let got = ok(&call(&a, "chat_update", json!({"action": "rename", "title": "  My   new name "})));
        assert_eq!(got, json!({"chatId": a.id, "link": chat_link(&a.id, "My new name"), "title": "My new name"}));
        assert!(BUS.after(n, 0.0).iter().any(|e| e["kind"] == "renamed" && e["agent"] == a.id.as_str() && e["name"] == "My new name"));
        assert_eq!(err(&call(&a, "chat_update", json!({"action": "rename", "title": " "}))).0, "invalid_request");
        drop_chat(&a);
    }

    #[test]
    fn chat_organize_archive_this_chat_waits_for_turn_end() {
        let _serial = serial();
        let a = caller(false);
        let got = ok(&call(&a, "chat_organize", json!({"action": "archive"})));
        assert_eq!(got["closesWhenTurnEnds"], true);
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(agents::get(&a.id).is_some(), "still open while its turn runs");
        a.set_running_for_test(false);
        until("the chat to close", || agents::get(&a.id).is_none());
        drop_chat(&a);
    }

    #[test]
    fn chat_organize_unarchive_reopens() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.put("user", json!({"text": "keep me"}));
        let id = t.id.clone();
        let got = ok(&call(&a, "chat_organize", json!({"chatId": id, "action": "archive"})));
        assert_eq!(got["closed"], true);
        assert!(agents::get(&id).is_none());
        let got = ok(&call(&a, "chat_organize", json!({"chatId": id, "action": "unarchive"})));
        assert_eq!(got, json!({"chatId": id, "link": chat_link(&id, &agents::get(&id).unwrap().name()), "closed": false}));
        drop_id(&id);
        let _ = std::fs::remove_file(crate::paths::work("chats").join(format!("{id}.json")));
        drop_chat(&a);
    }

    #[test]
    fn chat_configure_reads_and_sets() {
        let _serial = serial();
        let a = caller(false);
        let got = ok(&call(&a, "chat_configure", json!({})));
        assert_eq!(got["chatId"], a.id.as_str());
        assert_eq!(got["modelSelection"]["ai"], "claude");
        assert_eq!((got["runtimeMode"].as_str(), got["interactionMode"].as_str()), (Some("approval-required"), Some("default")));
        let t = test_chat("claude");
        let set = ok(&call(&a, "chat_configure", json!({"chatId": t.id, "modelSelection": {"ai": "gemini"}})));
        assert_eq!(set["modelSelection"]["ai"], "gemini");
        assert_eq!(t.ai(), "gemini", "a chat not started yet just takes the AI");
        let set = ok(&call(&a, "chat_configure", json!({"chatId": t.id, "modelSelection": {"ai": "gemini", "model": "gemini-x", "effort": "high"}})));
        assert_eq!(set["modelSelection"]["model"], "gemini-x");
        assert_eq!(t.last_ask().effort.as_deref(), Some("high"));
        assert_eq!(err(&call(&a, "chat_configure", json!({"chatId": t.id, "modelSelection": {"ai": "nope"}}))).0, "invalid_request");
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn chat_configure_has_no_mode_fields() {
        let _serial = serial();
        let a = caller(true);
        for arg in ["runtimeMode", "full", "studio"] {
            assert_eq!(rpc_error(&a, "chat_configure", json!({ arg: "full-access" })).0, -32602, "{arg}");
        }
        drop_chat(&a);
    }

    #[test]
    fn chat_queue_list_read_cancel() {
        let _serial = serial();
        let a = caller(false);
        let t = test_chat("claude");
        t.set_running_for_test(true);
        t.queue_first(&"y".repeat(300), Ask::default());
        t.queue_first("first one", Ask::default());
        let got = ok(&call(&a, "chat_queue", json!({"chatId": t.id, "action": "list"})));
        let items = got["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!((items[0]["text"].as_str(), items[0]["truncated"].as_bool()), (Some("first one"), Some(false)));
        assert_eq!((items[1]["text"].as_str().unwrap().chars().count(), items[1]["truncated"].as_bool()), (200, Some(true)));
        let qid = items[1]["queuedId"].as_str().unwrap().to_string();
        let read = ok(&call(&a, "chat_queue", json!({"chatId": t.id, "action": "read", "queuedId": qid})));
        assert_eq!(read["text"].as_str().unwrap().len(), 300);
        let cancel = ok(&call(&a, "chat_queue", json!({"chatId": t.id, "action": "cancel", "queuedId": qid})));
        assert_eq!(cancel, json!({"chatId": t.id, "queuedId": qid, "cancelled": true}));
        assert_eq!(t.queued(), 1);
        let (code, msg) = err(&call(&a, "chat_queue", json!({"chatId": t.id, "action": "cancel", "queuedId": qid})));
        assert_eq!((code.as_str(), msg.as_str()), ("run_not_found", "That message is no longer queued."));
        drop_chat(&t);
        drop_chat(&a);
    }

    #[test]
    fn subagent_follows_the_same_rules() {
        let _serial = serial();
        let parent = test_chat("claude");
        let helper = agents::spawn_expert("team-sub", "worker", "Sub: job", None, Some("claude".into()), Some(&parent.id), agents::Expert { id: "z".into(), name: "Z".into(), text: String::new() }).unwrap();
        helper.set_running_for_test(true);
        assert!(scope(&helper).subagent);
        let (code, _) = err(&call(&helper, "chat_launch", json!({"title": "Wider", "runtimeMode": "full-access"})));
        assert_eq!(code, "runtime_mode_escalation_denied");
        drop_chat(&helper);
        drop_chat(&parent);
    }
}
