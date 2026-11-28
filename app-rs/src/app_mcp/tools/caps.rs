//! fridae_capabilities: the AIs, models, efforts, accounts and modes a chat can be launched or configured with (port
//! of T3's orchestrator_capabilities).

use serde_json::{Map, Value, json};

use super::{READ_IDEM, ai_name, max_len, opt_text, reads, words};
use crate::app_mcp::{Call, Labels, Outcome, Summary, ToolDef};
use crate::paths;

/// How many models each AI shows when no `search` is given.
const SHORT: usize = 12;
/// At most this many models per AI for a `search`.
const FOUND: usize = 50;

pub fn defs() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "fridae_capabilities",
        title: "Get AIs and models",
        description: "List AIs, models, efforts, accounts, and runtime modes for `chat_launch` and `chat_configure`. Short list. `search` finds others. Any catalog model works.",
        schema,
        hints: READ_IDEM,
        access: reads,
        labels,
        always_load: true,
        handler,
    }]
}

fn schema() -> Value {
    json!({"type": "object", "properties": {
        "search": {"type": "string", "maxLength": 200, "description": "Match model ids and labels. Replaces the short list."}
    }})
}

fn labels(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Get", "Getting", "Got", "AIs and models", "Got", "AI list", "AI lists")
}

/// The models to show: the first `SHORT`, or those matching `search` (id or label, any case); and how many are left out.
fn short_list(models: &[(String, String)], search: Option<&str>) -> (Vec<Value>, usize) {
    let shown = |(id, label): &(String, String)| if label.is_empty() || label == id { json!({"id": id}) } else { json!({"id": id, "label": label}) };
    match search.map(str::to_lowercase) {
        None => (models.iter().take(SHORT).map(shown).collect(), models.len().saturating_sub(SHORT)),
        Some(q) => {
            let hits: Vec<&(String, String)> = models.iter().filter(|(id, label)| id.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)).collect();
            (hits.iter().take(FOUND).map(|m| shown(m)).collect(), hits.len().saturating_sub(FOUND))
        }
    }
}

/// An AI's catalog as (id, label), best first. Claude's are the app's own short names; the others arrive from their
/// tools after the app starts (empty until then).
fn catalog(ai: &str) -> Vec<(String, String)> {
    match ai {
        "claude" => crate::agents::MODELS.iter().map(|m| (m.to_string(), capitalized(m))).collect(),
        "gemini" => crate::antigravity::models().to_vec(),
        "chatgpt" => crate::codex::models().to_vec(),
        "cursor" => crate::cursor::models(),
        "opencode" => crate::opencode::models(),
        _ => Vec::new(),
    }
}

fn capitalized(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The accounts a chat on `ai` can run on: the user's own, then each added one (names and readiness only, never a
/// token or a folder).
pub(crate) fn accounts_of(ai: &str) -> Vec<Value> {
    let mut out = vec![json!({"account": "", "label": "Your own account", "ready": true})];
    if crate::accounts::AIS.contains(&ai) {
        let state = crate::accounts::state();
        for a in state[ai]["accounts"].as_array().into_iter().flatten() {
            if let Some(name) = a["name"].as_str() {
                out.push(json!({"account": name, "label": format!("Account {}", a["n"].as_u64().unwrap_or(0)), "ready": a["ready"] == true}));
            }
        }
    }
    out
}

/// What a chat on `ai` cannot do, in a few words each.
fn constraints(ai: &str) -> Vec<&'static str> {
    let mut out = vec!["A chat cannot run with broader modes than the chat that starts or changes it."];
    if !matches!(ai, "claude" | "chatgpt") {
        out.push("No effort setting: the AI decides.");
    }
    if !crate::accounts::AIS.contains(&ai) {
        out.push("Only the user's own account.");
    }
    out
}

fn handler(_call: &Call, args: &Map<String, Value>) -> Outcome {
    let search = opt_text(args, "search")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(s) = &search {
        max_len("search", s, 200)?;
    }
    let ais: Vec<Value> = paths::AIS
        .iter()
        .map(|ai| {
            let (models, omitted) = short_list(&catalog(ai), search.as_deref());
            let efforts: &[&str] = if matches!(*ai, "claude" | "chatgpt") { &crate::agents::EFFORTS } else { &[] };
            let mut entry = json!({"ai": ai, "displayName": ai_name(ai), "installed": paths::find_ai(ai).is_some(), "models": models,
                "efforts": efforts, "accounts": accounts_of(ai), "runtimeModes": ["approval-required", "full-access"], "constraints": constraints(ai)});
            if omitted > 0 {
                entry["omittedModels"] = json!(omitted);
            }
            entry
        })
        .collect();
    Ok(json!({"ais": ais}))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use serde_json::json;

    #[test]
    fn capabilities_short_list_and_search() {
        let _serial = serial();
        let many: Vec<(String, String)> = (0..20).map(|i| (format!("model-{i:02}"), format!("Model {i}"))).collect();
        let (short, omitted) = short_list(&many, None);
        assert_eq!((short.len(), omitted), (12, 8));
        assert_eq!(short[0], json!({"id": "model-00", "label": "Model 0"}));
        let (found, omitted) = short_list(&many, Some("MODEL 1"));
        assert_eq!(found.iter().map(|m| m["id"].as_str().unwrap()).collect::<Vec<_>>(), ["model-01", "model-10", "model-11", "model-12", "model-13", "model-14", "model-15", "model-16", "model-17", "model-18", "model-19"]);
        assert_eq!(omitted, 0);

        let a = caller(false);
        let got = ok(&call(&a, "fridae_capabilities", json!({})));
        let ais = got["ais"].as_array().unwrap();
        assert_eq!(ais.iter().map(|x| x["ai"].as_str().unwrap()).collect::<Vec<_>>(), crate::paths::AIS);
        let claude = &ais[0];
        assert_eq!(claude["displayName"], "Claude");
        assert_eq!(claude["models"].as_array().unwrap().len(), crate::agents::MODELS.len());
        assert_eq!(claude["efforts"], json!(crate::agents::EFFORTS));
        assert_eq!(claude["runtimeModes"], json!(["approval-required", "full-access"]));
        assert_eq!(claude["accounts"][0]["account"], "", "the user's own account first");
        let gemini = ais.iter().find(|x| x["ai"] == "gemini").unwrap();
        assert_eq!(gemini["efforts"], json!([]));
        assert!(ais.iter().all(|x| x["models"].as_array().unwrap().len() <= 12));
        let found = ok(&call(&a, "fridae_capabilities", json!({"search": "sonn"})));
        assert_eq!(found["ais"][0]["models"], json!([{"id": "sonnet", "label": "Sonnet"}]));
        assert!(!got.to_string().to_lowercase().contains("token"), "no tokens");
        crate::app_mcp::drop_chat(&a);
    }
}
