//! The toolbox's tools: chats, projects, settings, the theme, capabilities (ports of T3 Code's orchestrator, project,
//! environment and appearance tools; descriptions are T3's words with `t3_` dropped, thread→chat, T3 Code→Fridae).

pub mod caps;
pub mod chats;
pub mod projects;
pub mod settings;
pub mod theme;

use serde_json::{Map, Value, json};

use super::{Access, Failure, Hints, Labels, Summary, ToolDef, fail};

/// Every tool, in the order tools/list shows them.
pub fn defs() -> Vec<ToolDef> {
    let mut all = caps::defs();
    all.extend(chats::defs());
    all.extend(projects::defs());
    all.extend(settings::defs());
    all.extend(theme::defs());
    all
}

// ------------------------------------------------------------------------------------------- the lists up front

/// How many projects and open chats the lists up front show, and their most bytes.
const NOW_MOST: usize = 30;
const NOW_BYTES: usize = 2_000;

/// The app as it is now, for the top of a turn's message (`agents::send`): the projects and the open chats with their
/// ids, so the AI can act without listing first. Read as `project_list` and `chat_list` read them for this chat (the
/// same access), newest chats first. Names, ids, links and AIs only: no key, no path.
pub fn now_text(scope: &super::keys::Scope) -> String {
    let flag = std::sync::atomic::AtomicBool::new(false);
    let call = super::Call::new(scope, &flag, super::exposure::Mode::Full);
    let arg = |v: Value| v.as_object().cloned().unwrap_or_default();
    let projects = projects::list(&call, &arg(json!({"limit": 100}))).ok().and_then(|v| v["projects"].as_array().cloned()).unwrap_or_default();
    let chats = chats::list(&call, &arg(json!({"allProjects": true, "statuses": ["working", "idle"], "limit": 100}))).ok().and_then(|v| v["chats"].as_array().cloned()).unwrap_or_default();
    now_block(&projects, &chats, &scope.chat)
}

/// The block itself: up to `NOW_MOST` of each, within `NOW_BYTES`, and how many more there are.
pub fn now_block(projects: &[Value], chats: &[Value], current: &str) -> String {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").replace(['\n', '\r'], " ");
    let project_lines: Vec<String> = projects.iter().map(|p| format!("- {} · {} · {}", s(p, "name"), s(p, "projectId"), s(p, "link"))).collect();
    let chat_lines: Vec<String> = chats
        .iter()
        .map(|c| {
            let id = s(c, "chatId");
            let title = if id == current { "(this chat)".to_string() } else { s(c, "title") };
            let project = c["projectId"].as_str().unwrap_or("no project");
            format!("- {title} · {id} · {project} · {}", s(c, "ai"))
        })
        .collect();
    // up to half the room for the projects, the chats taking what they leave
    let take = |lines: &[String], room: usize| -> (Vec<String>, usize) {
        let mut out = Vec::new();
        let mut used = 0;
        for l in lines.iter().take(NOW_MOST) {
            if used + l.len() + 1 > room {
                break;
            }
            used += l.len() + 1;
            out.push(l.clone());
        }
        (out, used)
    };
    let (ps, used) = take(&project_lines, NOW_BYTES / 2);
    let (cs, _) = take(&chat_lines, NOW_BYTES - used);
    let mut out = vec!["<fridae_now>".to_string()];
    if ps.is_empty() {
        out.push("Projects: none".into());
    } else {
        out.push("Projects (name · id · link):".into());
        out.extend(ps.iter().cloned());
        if ps.len() < project_lines.len() {
            out.push(format!("- {} more: project_list", project_lines.len() - ps.len()));
        }
    }
    if cs.is_empty() {
        out.push("Open chats: none".into());
    } else {
        out.push("Open chats, newest first (title · id · project id · AI):".into());
        out.extend(cs.iter().cloned());
        if cs.len() < chat_lines.len() {
            out.push(format!("- {} more: chat_list", chat_lines.len() - cs.len()));
        }
    }
    out.push("</fridae_now>".into());
    out.join("\n")
}

// ------------------------------------------------------------------------------------------------ shared pieces

pub(crate) fn reads(_: &Map<String, Value>) -> Access {
    Access::Reads
}

pub(crate) fn writes_threads(_: &Map<String, Value>) -> Access {
    Access::WritesThreads
}

pub(crate) fn starts_threads(_: &Map<String, Value>) -> Access {
    Access::StartsThreads
}

pub(crate) fn writes_environment(_: &Map<String, Value>) -> Access {
    Access::WritesEnvironment
}

/// Read-only and idempotent (T3's `readOnlyHint` + `idempotentHint`).
pub(crate) const READ_IDEM: Hints = Hints { idempotent: true, ..Hints::READ };

/// A tool row's words and how done rows fold, from T3's label tuple.
pub(crate) const fn words(action: &'static str, running: &'static str, done: &'static str, detail: &'static str, verb: &'static str, one: &'static str, many: &'static str) -> (Labels, Summary) {
    (Labels { action, running, done, detail }, Summary { verb, one, many })
}

/// At most `max` characters, then "…".
pub(crate) fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// A string argument, trimmed; None when missing or empty.
pub(crate) fn text<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A string argument that must be a string when given.
pub(crate) fn opt_text(args: &Map<String, Value>, key: &str) -> Result<Option<String>, Failure> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(fail("invalid_request", format!("`{key}` must be a string."))),
    }
}

/// A true/false argument that must be one when given.
pub(crate) fn opt_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, Failure> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(fail("invalid_request", format!("`{key}` must be true or false."))),
    }
}

/// A whole-number argument within `lo..=hi` when given.
pub(crate) fn opt_int(args: &Map<String, Value>, key: &str, lo: i64, hi: i64) -> Result<Option<i64>, Failure> {
    match args.get(key) {
        None => Ok(None),
        Some(v) => match v.as_i64() {
            Some(n) if (lo..=hi).contains(&n) => Ok(Some(n)),
            _ => Err(fail("invalid_request", format!("`{key}` must be a whole number from {lo} to {hi}."))),
        },
    }
}

/// A string argument that must be one of `allowed` when given.
pub(crate) fn opt_choice(args: &Map<String, Value>, key: &str, allowed: &[&str]) -> Result<Option<String>, Failure> {
    match opt_text(args, key)? {
        None => Ok(None),
        Some(s) if allowed.contains(&s.trim()) => Ok(Some(s.trim().to_string())),
        Some(s) => Err(fail("invalid_request", format!("Unknown {key} \"{}\". Use {}.", cut(&s, 64), allowed.join(", ")))),
    }
}

/// A string that is at most `max` characters long (T3's maxLength).
pub(crate) fn max_len(key: &str, s: &str, max: usize) -> Result<(), Failure> {
    if s.chars().count() > max {
        return Err(fail("invalid_request", format!("`{key}` is longer than {max} characters.")));
    }
    Ok(())
}

/// A page cursor: the offset into the list, as a string.
pub(crate) fn cursor_of(args: &Map<String, Value>) -> Result<usize, Failure> {
    match text(args, "cursor") {
        None => Ok(0),
        Some(c) => c.parse::<usize>().map_err(|_| fail("invalid_request", "That cursor is not one this tool gave. Start again without `cursor`.")),
    }
}

/// One page of a list and the cursor of the next (null at the end).
pub(crate) fn page<T: Clone>(all: &[T], from: usize, limit: usize) -> (Vec<T>, Value) {
    let page: Vec<T> = all.iter().skip(from).take(limit).cloned().collect();
    let next = from + page.len();
    (page, if next < all.len() { json!(next.to_string()) } else { Value::Null })
}

/// An AI's name as people read it.
pub(crate) fn ai_name(ai: &str) -> &'static str {
    match ai {
        "gemini" => "Gemini",
        "chatgpt" => "ChatGPT",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        _ => "Claude",
    }
}

/// A chat chip for the tool row.
pub(crate) fn chat_chip(id: &str, title: &str) -> Value {
    json!({"kind": "chat", "id": id, "title": title})
}

/// A project chip for the tool row.
pub(crate) fn project_chip(id: &str, title: &str) -> Value {
    json!({"kind": "project", "id": id, "title": title})
}

/// Shared test helpers: chats as callers, calls through `call_tool` (so access and tool rows run), results read back.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, atomic::AtomicBool};

    use serde_json::{Value, json};

    use crate::agents::{Agent, Ask, BUS};
    use crate::app_mcp::{call_tool, exposure, keys};

    /// The tool tests take turns: each needs up to four of the app's eight chat slots at once, and side by side they
    /// would each hold some and wait for the rest.
    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn scope(a: &Agent) -> keys::Scope {
        keys::Scope { chat: a.id.clone(), ai: a.ai(), subagent: a.is_subagent(), namespace: format!("ns-{}", a.id) }
    }

    /// The MCP result of a call by `a` (a JSON-RPC error panics).
    pub(crate) fn call(a: &Agent, name: &str, args: Value) -> Value {
        call_tool(&scope(a), name, args, &AtomicBool::new(false), exposure::Mode::Full).unwrap_or_else(|e| panic!("{name}: JSON-RPC error {e:?}"))
    }

    /// The JSON-RPC error of a call refused before it runs.
    pub(crate) fn rpc_error(a: &Agent, name: &str, args: Value) -> (i64, String) {
        call_tool(&scope(a), name, args, &AtomicBool::new(false), exposure::Mode::Full).expect_err("a JSON-RPC error")
    }

    /// A good result's structured content.
    pub(crate) fn ok(r: &Value) -> Value {
        assert_eq!(r["isError"], false, "{r}");
        r["structuredContent"].clone()
    }

    /// A failed result's (code, message).
    pub(crate) fn err(r: &Value) -> (String, String) {
        assert_eq!(r["isError"], true, "{r}");
        let t: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap_or(json!({}));
        (t["code"].as_str().unwrap_or("").to_string(), t["message"].as_str().unwrap_or("").to_string())
    }

    /// A caller chat: running a turn (a live caller), in Full auto or not.
    pub(crate) fn caller(full: bool) -> Arc<Agent> {
        let a = crate::app_mcp::test_chat("claude");
        a.set_last_ask_for_test(Ask { full, ..Default::default() });
        a.set_running_for_test(true);
        a
    }

    pub(crate) fn bus_len() -> usize {
        BUS.after(0, 0.0).len()
    }

    pub(crate) fn bus_since(n: usize) -> Vec<Value> {
        BUS.after(n, 0.0)
    }

    /// Waits (up to 10 s) for a condition.
    pub(crate) fn until(what: &str, mut f: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !f() {
            assert!(std::time::Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// A folder for a test's project: `H:/rust/tmp/app-tools-<name>` where that exists (the build machine), else the
    /// test run's own folder (other computers).
    pub(crate) fn project_dir(name: &str) -> String {
        let base = std::path::Path::new("H:/rust/tmp");
        let dir = if base.is_dir() { base.join(format!("app-tools-{name}")) } else { crate::paths::test_dir(&format!("app-tools-{name}")) };
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir.display().to_string()
    }

    /// Answers the next card in `a`'s chat from another thread, as the page would; joins to the card shown.
    pub(crate) fn answer_card(a: &Arc<Agent>, allow: bool) -> std::thread::JoinHandle<Value> {
        let (a, shown) = (a.clone(), cards(a).len());
        std::thread::spawn(move || {
            // a card shown after this was called (earlier ones in the chat are over)
            let mut card = Value::Null;
            until("a card", || {
                card = cards(&a).get(shown).cloned().unwrap_or(Value::Null);
                !card.is_null()
            });
            assert!(crate::app_mcp::card::answer(&a.id, card["card"].as_str().unwrap(), allow));
            card
        })
    }

    /// The chat's `appcard` events.
    pub(crate) fn cards(a: &Agent) -> Vec<Value> {
        a.log_snapshot().into_iter().filter(|e| e["kind"] == "appcard").collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_are_the_21_in_order_with_pinned_descriptions() {
        let names: Vec<&str> = defs().iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            [
                "fridae_capabilities", "chat_list", "chat_search", "chat_read", "chat_launch", "chat_send", "chat_wait", "chat_interrupt", "chat_update", "chat_organize", "chat_configure", "chat_queue", "project_list",
                "project_read", "project_create", "project_update", "project_delete", "settings_read", "settings_update", "theme_read", "theme_set"
            ]
        );
        let desc = |n: &str| defs().into_iter().find(|d| d.name == n).unwrap().description;
        assert_eq!(desc("chat_list"), "List chats newest first, in one project or with `allProjects`. Page with `cursor`.");
        assert_eq!(desc("chat_wait"), "Wait for a chat's run to finish. No `runId`: latest. Idle: at once. On timeout, with `next` end your turn; without, call again.");
        assert_eq!(desc("project_list"), "List projects. Pages can shift.");
        // a deliberate departure from T3's "Follow it with chat_read or chat_wait": waiting is for when it is needed
        assert_eq!(desc("chat_launch"), "Create a top-level chat in its workspace before starting. Task in `message`. Omit `projectId`, model and modes to inherit yours (`scratch` for no project); modes cannot exceed yours. On error or lost reply, reuse `clientRequestId`. Its result confirms it: answer the user with its `link` at once; wait for or read it only when the user asks what it said or your task needs its reply.");
        assert!(defs().iter().all(|d| !d.description.contains("Follow it with")));
        // the result is the confirmation: nothing to read back after an action
        for n in ["chat_launch", "chat_send", "project_create", "project_update", "project_delete", "settings_update", "theme_set"] {
            assert!(desc(n).contains("Its result confirms it"), "{n}: {}", desc(n));
        }
        assert_eq!(desc("project_update"), "Update one project's settings. Omitted fields are unchanged. Its result confirms it.");
        assert_eq!(desc("project_delete"), "Delete one project. Nonempty needs `force: true`. Folder on disk stays. Its result confirms it.");
        let caps = defs().into_iter().next().unwrap();
        assert!(caps.always_load && caps.hints.read_only && caps.hints.idempotent);
        for d in defs() {
            let schema = (d.schema)();
            assert_eq!(schema["type"], "object", "{}", d.name);
            assert!(schema["properties"].is_object(), "{}", d.name);
            assert!(!d.description.contains("T3") && !d.description.contains("thread"), "{}", d.name);
        }
    }
}
