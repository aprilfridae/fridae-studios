//! Which tools a client is shown, and the deferred tools `tool_search` and `tool_call` (port of T3 Code's
//! McpToolExposure.ts and McpHttpServer.ts's registerToolCall). Antigravity (Gemini) freezes its tool list per turn
//! and cuts long lists, so it lists only a few tools (`fridae-tool-exposure: deferred-proxy`), finds the others with
//! `tool_search` and runs them with `tool_call`. Every other AI lists everything but those two.

use std::sync::atomic::AtomicBool;

use serde_json::{Map, Value, json};

use super::{Access, Call, Hints, Labels, Outcome, Summary, ToolDef, fail, keys};

pub const TOOL_SEARCH: &str = "tool_search";
pub const TOOL_CALL: &str = "tool_call";
pub const MAX_RESULTS: usize = 10;
// Antigravity caps inline MCP tool results around 10 KB; stay comfortably under it so JSON schemas never truncate.
pub const MAX_PROXY_SCHEMA_BYTES: usize = 8_000;

/// What a deferred-proxy client lists before it searches.
const PROXY_DIRECT_TOOLS: [&str; 3] = ["fridae_capabilities", TOOL_SEARCH, TOOL_CALL];

/// How a client takes the tools: all of them, or (Antigravity) only a few plus tool_search/tool_call.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Full,
    DeferredProxy,
}

/// The mode a `fridae-tool-exposure` header asks for.
pub fn mode_of(header: &str) -> Mode {
    if header.trim().eq_ignore_ascii_case("deferred-proxy") { Mode::DeferredProxy } else { Mode::Full }
}

/// Whether tools/list shows the tool in this mode. (Calls are never gated by it: a deferred tool is still called.)
pub fn listed(mode: Mode, name: &str) -> bool {
    match mode {
        Mode::DeferredProxy => PROXY_DIRECT_TOOLS.contains(&name),
        Mode::Full => name != TOOL_SEARCH && name != TOOL_CALL,
    }
}

/// A tool as search sees it.
pub struct Searchable {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// T3's score: per term, the exact name 100, else a name containing it 3, else a description containing it 1.
fn score(tool: &Searchable, terms: &[String]) -> u32 {
    let name = tool.name.to_lowercase();
    let description = tool.description.to_lowercase();
    terms.iter().map(|t| if *t == name { 100 } else if name.contains(t.as_str()) { 3 } else if description.contains(t.as_str()) { 1 } else { 0 }).sum()
}

fn bytes(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(usize::MAX)
}

/// The best `MAX_RESULTS` matches for a query. Deferred-proxy answers carry each tool's schema while the whole answer
/// stays within `MAX_PROXY_SCHEMA_BYTES` (then name and description only; a first tool too big even so is cut to 200
/// characters), since its client calls them through `tool_call`; others get names and descriptions.
pub fn rank(tools: &[Searchable], query: &str, mode: Mode) -> Vec<Value> {
    let terms: Vec<String> = query.to_lowercase().split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')).filter(|t| !t.is_empty()).map(str::to_string).collect();
    let mut found: Vec<(&Searchable, u32)> = tools.iter().filter(|t| t.name != TOOL_SEARCH && t.name != TOOL_CALL).map(|t| (t, score(t, &terms))).filter(|(_, s)| *s > 0).collect();
    found.sort_by(|a, b| b.1.cmp(&a.1)); // stable, as JS's sort is
    found.truncate(MAX_RESULTS);
    if mode != Mode::DeferredProxy {
        return found.iter().map(|(t, _)| json!({"name": t.name, "description": t.description})).collect();
    }
    let mut results: Vec<Value> = Vec::new();
    let fits = |results: &[Value], next: &Value| {
        let mut all = results.to_vec();
        all.push(next.clone());
        bytes(&json!({"tools": all})) <= MAX_PROXY_SCHEMA_BYTES
    };
    for (t, _) in found {
        let with_schema = json!({"name": t.name, "description": t.description, "inputSchema": t.schema});
        if fits(&results, &with_schema) {
            results.push(with_schema);
            continue;
        }
        let compact = json!({"name": t.name, "description": t.description});
        if fits(&results, &compact) {
            results.push(compact);
            continue;
        }
        if results.is_empty() {
            let cut: String = t.description.chars().take(200).collect();
            results.push(json!({"name": t.name, "description": format!("{cut} (Truncated: exceeds response budget.)")}));
        }
        break;
    }
    results
}

/// Search over the tools a call can name (those tools/list may show).
pub fn search(query: &str, mode: Mode) -> Vec<Value> {
    let tools: Vec<Searchable> = super::listable().iter().map(|d| Searchable { name: d.name.to_string(), description: d.description.to_string(), schema: (d.schema)() }).collect();
    rank(&tools, query, mode)
}

fn reads_as_caller(_: &Map<String, Value>) -> Access {
    Access::ReadsAsCaller
}

fn search_schema() -> Value {
    json!({"type": "object", "properties": {"query": {"type": "string", "maxLength": 500}}, "required": ["query"]})
}

fn search_labels(_: &Map<String, Value>) -> (Labels, Summary) {
    (Labels { action: "Find", running: "Finding", done: "Found", detail: "Fridae tools" }, Summary { verb: "Found", one: "tool search", many: "tool searches" })
}

fn search_handler(call: &Call, args: &Map<String, Value>) -> Outcome {
    let query = args.get("query").and_then(Value::as_str).ok_or_else(|| fail("invalid_request", "query is required."))?;
    if query.chars().count() > 500 {
        return Err(fail("invalid_request", "query must be at most 500 characters."));
    }
    Ok(json!({"tools": search(query, call.exposure)}))
}

fn call_schema() -> Value {
    json!({"type": "object", "properties": {
        "name": {"type": "string", "description": "Exact Fridae tool name."},
        "arguments": {"type": "object", "description": "Arguments object for that tool."}},
        "required": ["name"]})
}

fn call_labels(_: &Map<String, Value>) -> (Labels, Summary) {
    (Labels { action: "Run", running: "Running", done: "Ran", detail: "a Fridae tool" }, Summary { verb: "Ran", one: "tool", many: "tools" })
}

/// Never reached: `call_tool` hands a `tool_call` to `call` before any handler (its answer is the named tool's own).
fn call_handler(_: &Call, _: &Map<String, Value>) -> Outcome {
    Err(fail("orchestration_error", "tool_call runs through the dispatcher."))
}

/// The deferred tools (tool_search, tool_call).
pub fn defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: TOOL_SEARCH,
            title: "Find Fridae tools",
            description: "Find Fridae tools you do not have yet. If tool_call is available, pass it the returned tool's name and your arguments. Otherwise search loads the tools so you can call them by name. Search by name or purpose: chats (chat_*), projects (project_*), settings (settings_*), themes (theme_*). Exact names work too.",
            schema: search_schema,
            hints: Hints { read_only: true, destructive: false, idempotent: false, open_world: true },
            access: reads_as_caller,
            labels: search_labels,
            always_load: false,
            handler: search_handler,
        },
        ToolDef {
            name: TOOL_CALL,
            title: "Run a Fridae tool",
            description: "Invoke any Fridae tool by exact name (find schemas with tool_search first).",
            schema: call_schema,
            hints: Hints { read_only: false, destructive: true, idempotent: false, open_world: false },
            access: reads_as_caller,
            labels: call_labels,
            always_load: false,
            handler: call_handler,
        },
    ]
}

/// `tool_call`: the named tool run as if called directly (its own access rules, row and answer). Its own refusals
/// are JSON-RPC -32602 errors; bad arguments for the named tool are an error result with that tool's schema.
pub fn call(scope: &keys::Scope, args: Value, cancelled: &AtomicBool, exposure: Mode) -> Result<Value, (i64, String)> {
    let record = args.as_object();
    let name = record.and_then(|r| r.get("name")).and_then(Value::as_str).map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err((-32602, "name is required.".to_string()));
    }
    if name == TOOL_CALL || name == TOOL_SEARCH {
        return Err((-32602, format!("Cannot invoke '{name}' via tool_call.")));
    }
    let inner = match record.and_then(|r| r.get("arguments")) {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(Value::Object(m)) => Value::Object(m.clone()),
        Some(_) => return Err((-32602, "arguments must be a JSON object.".to_string())),
    };
    match super::call_tool(scope, name, inner, cancelled, exposure) {
        Err((-32602, message)) => match super::registry().into_iter().find(|d| d.name == name) {
            Some(def) => Ok(json!({"isError": true, "content": [{"type": "text", "text": format!("Invalid arguments for {name}: {message}\nExpected inputSchema: {}", (def.schema)())}]})),
            None => Err((-32602, message)),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_mcp::{call_tool, drop_chat, fail, fail_result, http::handle_parts, keys, test_chat};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn tool(name: &str, description: &str, schema: Value) -> Searchable {
        Searchable { name: name.to_string(), description: description.to_string(), schema }
    }

    #[test]
    fn scoring_matches_t3() {
        let tools = vec![
            tool("chat_read", "Read a chat's messages.", json!({})),
            tool("project_list", "List projects, with chat counts.", json!({})),
            tool("chat_list", "List chats.", json!({})),
            tool("theme_set", "Set the theme.", json!({})),
        ];
        // exact name +100, name contains +3, description contains +1; split on anything but [a-z0-9_]
        let names = |q: &str| rank(&tools, q, Mode::Full).iter().map(|v| v["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names("chat_list"), vec!["chat_list"], "exact 100, and no other name contains 'chat_list'");
        assert_eq!(names("CHAT"), vec!["chat_read", "chat_list", "project_list"], "name 3 each (stable order), description 1");
        assert_eq!(names("list, projects!"), vec!["project_list", "chat_list"], "project_list 3+1, chat_list 3");
        assert!(names("zzz").is_empty());
        assert!(names("   ").is_empty());
        let full = rank(&tools, "theme", Mode::Full);
        assert_eq!(full, vec![json!({"name": "theme_set", "description": "Set the theme."})], "full mode: no schemas");
        let proxy = rank(&tools, "theme", Mode::DeferredProxy);
        assert_eq!(proxy, vec![json!({"name": "theme_set", "description": "Set the theme.", "inputSchema": {}})]);
        let many: Vec<Searchable> = (0..15).map(|i| tool(&format!("chat_{i}"), "x", json!({}))).collect();
        assert_eq!(rank(&many, "chat", Mode::Full).len(), MAX_RESULTS);
    }

    #[test]
    fn proxy_results_fit_in_8000_bytes() {
        let big = json!({"type": "object", "description": "y".repeat(5000)});
        let tools = vec![tool("chat_a", "first", big.clone()), tool("chat_b", "second", big.clone()), tool("chat_c", "third", big)];
        let got = rank(&tools, "chat", Mode::DeferredProxy);
        assert_eq!(got.len(), 3);
        assert!(got[0].get("inputSchema").is_some());
        assert_eq!(got[1], json!({"name": "chat_b", "description": "second"}), "too big with its schema: name and description");
        assert!(serde_json::to_string(&json!({"tools": got})).unwrap().len() <= MAX_PROXY_SCHEMA_BYTES);
        // a first tool too big even without its schema: a 200-character cut, then nothing more
        let huge = vec![tool("chat_x", &"d".repeat(9000), json!({})), tool("chat_y", "small", json!({}))];
        let got = rank(&huge, "chat", Mode::DeferredProxy);
        assert_eq!(got, vec![json!({"name": "chat_x", "description": format!("{} (Truncated: exceeds response budget.)", "d".repeat(200))})]);
        // once a later tool does not fit even compact, the list stops
        let stop = vec![tool("chat_a", &"a".repeat(7000), json!({})), tool("chat_b", &"b".repeat(2000), json!({})), tool("chat_c", "c", json!({}))];
        assert_eq!(rank(&stop, "chat", Mode::DeferredProxy).len(), 1);
    }

    #[test]
    fn listing_by_mode() {
        for t in ["chat_read", "project_list", "theme_set", "settings_update"] {
            assert!(listed(Mode::Full, t), "{t}");
            assert!(!listed(Mode::DeferredProxy, t), "{t}");
        }
        assert!(!listed(Mode::Full, TOOL_SEARCH) && !listed(Mode::Full, TOOL_CALL));
        for t in ["fridae_capabilities", TOOL_SEARCH, TOOL_CALL] {
            assert!(listed(Mode::DeferredProxy, t), "{t}");
        }
        assert!(listed(Mode::Full, "fridae_capabilities"));
        let names: Vec<&str> = defs().iter().map(|d| d.name).collect();
        assert_eq!(names, vec![TOOL_SEARCH, TOOL_CALL]);
        let search = &defs()[0];
        assert_eq!(search.description, "Find Fridae tools you do not have yet. If tool_call is available, pass it the returned tool's name and your arguments. Otherwise search loads the tools so you can call them by name. Search by name or purpose: chats (chat_*), projects (project_*), settings (settings_*), themes (theme_*). Exact names work too.");
        assert_eq!((search.schema)()["properties"]["query"]["maxLength"], 500);
        assert!(search.hints.read_only && !search.hints.destructive);
        let call = &defs()[1];
        assert_eq!(call.description, "Invoke any Fridae tool by exact name (find schemas with tool_search first).");
        assert_eq!((call.schema)()["properties"]["name"]["description"], "Exact Fridae tool name.");
        assert_eq!((call.schema)()["properties"]["arguments"]["description"], "Arguments object for that tool.");
        assert!(!call.hints.read_only && !call.hints.open_world);
    }

    fn scope_of(a: &crate::agents::Agent) -> keys::Scope {
        keys::Scope { chat: a.id.clone(), ai: a.ai(), subagent: false, namespace: format!("ns-{}", a.id) }
    }

    fn via_tool_call(scope: &keys::Scope, args: Value) -> Result<Value, (i64, String)> {
        call_tool(scope, TOOL_CALL, args, &AtomicBool::new(false), Mode::DeferredProxy)
    }

    #[test]
    fn tool_call_refuses_itself_and_search() {
        let a = test_chat("gemini");
        let s = scope_of(&a);
        assert_eq!(via_tool_call(&s, json!({"name": "tool_call"})), Err((-32602, "Cannot invoke 'tool_call' via tool_call.".to_string())));
        assert_eq!(via_tool_call(&s, json!({"name": " tool_search "})), Err((-32602, "Cannot invoke 'tool_search' via tool_call.".to_string())));
        assert_eq!(via_tool_call(&s, json!({})), Err((-32602, "name is required.".to_string())));
        assert_eq!(via_tool_call(&s, json!({"name": "  "})), Err((-32602, "name is required.".to_string())));
        assert_eq!(via_tool_call(&s, json!({"name": "test_read", "arguments": [1]})), Err((-32602, "arguments must be a JSON object.".to_string())));
        // bad arguments for the named tool: an error result naming it, with its schema
        let bad = via_tool_call(&s, json!({"name": "test_read", "arguments": {"zz": 1}})).unwrap();
        assert_eq!(bad["isError"], true);
        let text = bad["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("test_read") && text.contains("Unknown argument \"zz\"") && text.contains("Expected inputSchema: {"), "{text}");
        // a good call is the named tool's own answer, its row the named tool's
        let ok = via_tool_call(&s, json!({"name": "test_read", "arguments": {}})).unwrap();
        assert_eq!(ok, crate::app_mcp::ok_result(&json!({"chatId": a.id})));
        let rows: Vec<Value> = a.log_snapshot().into_iter().filter(|e| e["kind"] == "apptool").collect();
        assert!(!rows.is_empty() && rows.iter().all(|r| r["tool"] == "test_read"), "{rows:?}");
        assert_eq!(via_tool_call(&s, json!({"name": "nope"})).unwrap_err().0, -32601);
        drop_chat(&a);
    }

    #[test]
    fn tool_call_runs_the_named_tool_with_its_access() {
        // (the plan names chat_update, which is Task 2's: test_write is a writesThreads tool of the same kind)
        let a = test_chat("gemini");
        let got = via_tool_call(&scope_of(&a), json!({"name": "test_write", "arguments": {}})).unwrap();
        assert_eq!(got, fail_result(&fail("parent_not_active", "The calling AI no longer owns an active chat run.")));
        a.set_running_for_test(true);
        assert_eq!(via_tool_call(&scope_of(&a), json!({"name": "test_write"})).unwrap()["isError"], false);
        drop_chat(&a);
    }

    #[test]
    fn tool_search_finds_tools_in_proxy_mode() {
        let a = test_chat("gemini");
        let got = call_tool(&scope_of(&a), TOOL_SEARCH, json!({"query": "tool"}), &AtomicBool::new(false), Mode::DeferredProxy).unwrap();
        assert_eq!(got["isError"], false, "{got}");
        let tools = got["structuredContent"]["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["name"] != TOOL_SEARCH && t["name"] != TOOL_CALL), "{tools:?}");
        drop_chat(&a);
    }

    #[test]
    fn http_lists_three_tools_with_the_header() {
        let a = test_chat("gemini");
        let key = keys::global().prepare(&a.id, &a.ai(), false).raw;
        let heads = |sid: Option<&str>| {
            let mut h = vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Accept".to_string(), "application/json, text/event-stream".to_string()),
                ("Authorization".to_string(), format!("Bearer {key}")),
                ("fridae-tool-exposure".to_string(), "deferred-proxy".to_string()),
            ];
            if let Some(s) = sid {
                h.push(("Mcp-Session-Id".to_string(), s.to_string()));
            }
            h
        };
        let init = handle_parts("POST", &heads(None), br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#);
        let sid = init.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("mcp-session-id")).unwrap().1.clone();
        let r = handle_parts("POST", &heads(Some(&sid)), br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        let names: Vec<String> = body["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        // (fridae_capabilities is Task 2's: listed here once the branches join)
        let want: Vec<String> = crate::app_mcp::registry().iter().map(|d| d.name).filter(|n| ["fridae_capabilities", TOOL_SEARCH, TOOL_CALL].contains(n)).map(str::to_string).collect();
        assert_eq!(names, want);
        assert!(names.contains(&TOOL_SEARCH.to_string()) && names.contains(&TOOL_CALL.to_string()));
        drop_chat(&a);
    }
}
