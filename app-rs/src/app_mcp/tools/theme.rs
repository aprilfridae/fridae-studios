//! theme_read and theme_set: Fridae's own built-in themes and its light, dark or system mode, applied to the open window
//! at once (port of T3's appearance tools, without theme authoring).

use serde_json::{Map, Value, json};

use super::{opt_choice, reads, words, writes_environment};
use crate::app_mcp::events::{MODES, THEMES};
use crate::app_mcp::{Call, Hints, Labels, Outcome, Summary, ToolDef, access, card::CardSpec, events, fail};

pub fn defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "theme_read",
            title: "Read themes",
            description: "Read Fridae's own themes and the light, dark or system mode. Call before `theme_set`.",
            schema: schema_read,
            hints: Hints::READ,
            access: reads,
            labels: labels_read,
            always_load: false,
            handler: read,
        },
        ToolDef {
            name: "theme_set",
            title: "Set the theme",
            description: "Restyle Fridae itself, only when the user asks. Switch to a built-in `themeId`. `appearance` sets light, dark, or system mode. Its result confirms it.",
            schema: schema_set,
            hints: Hints::WRITE,
            access: writes_environment,
            labels: labels_set,
            always_load: false,
            handler: set,
        },
    ]
}

fn schema_read() -> Value {
    json!({"type": "object", "properties": {}})
}

fn schema_set() -> Value {
    let ids: Vec<&str> = THEMES.iter().map(|(id, _)| *id).collect();
    json!({"type": "object", "properties": {
        "themeId": {"type": "string", "enum": ids, "description": "A built-in theme's id from theme_read."},
        "appearance": {"type": "string", "enum": MODES, "description": "light, dark, or system."}
    }})
}

fn labels_read(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Read", "Reading", "Read", "themes", "Read", "theme list", "theme lists")
}

fn labels_set(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Set", "Setting", "Set", "the theme", "Set", "theme", "themes")
}

fn read(_call: &Call, _args: &Map<String, Value>) -> Outcome {
    let current = events::theme_now().map(|(id, mode)| json!({"themeId": id, "appearance": mode})).unwrap_or(Value::Null);
    let built_in: Vec<Value> = THEMES.iter().map(|(id, name)| json!({"id": id, "name": name})).collect();
    Ok(json!({"current": current, "builtIn": built_in, "appearances": MODES}))
}

/// The card's sentence: only the parts given.
fn sentence(theme: Option<&str>, mode: Option<&str>) -> String {
    let name = theme.and_then(|t| THEMES.iter().find(|(id, _)| *id == t)).map(|(_, n)| *n);
    match (name, mode) {
        (Some(n), Some(m)) => format!("Switch Fridae to {n} in {m} mode."),
        (Some(n), None) => format!("Switch Fridae to {n}."),
        (None, Some(m)) => format!("Switch Fridae to {m} mode."),
        (None, None) => String::new(),
    }
}

fn set(call: &Call, args: &Map<String, Value>) -> Outcome {
    let ids: Vec<&str> = THEMES.iter().map(|(id, _)| *id).collect();
    let theme = opt_choice(args, "themeId", &ids)?;
    let mode = opt_choice(args, "appearance", &MODES)?;
    if theme.is_none() && mode.is_none() {
        return Err(fail("invalid_request", "Pass themeId or appearance."));
    }
    let caller = access::writing_caller(call.scope)?;
    let card = CardSpec { title: "Change the look".into(), text: sentence(theme.as_deref(), mode.as_deref()), allow: "Change it".into() };
    access::environment_gate(call, &caller, card, false)?;
    events::theme_set(theme.as_deref(), mode.as_deref());
    let mut out = Map::new();
    if let Some(t) = theme {
        out.insert("themeId".into(), json!(t));
    }
    if let Some(m) = mode {
        out.insert("appearance".into(), json!(m));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use serde_json::json;

    #[test]
    fn theme_read_lists_six() {
        let _serial = serial();
        let a = caller(false);
        let got = ok(&call(&a, "theme_read", json!({})));
        assert_eq!(got["builtIn"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Fridae", "Graphite", "Moss", "Tide", "Clay", "Lilac"]);
        assert_eq!(got["builtIn"][2], json!({"id": "moss", "name": "Moss"}));
        assert_eq!(got["appearances"], json!(["light", "dark", "system"]));
        assert!(got.get("current").is_some(), "current is there (null before the window has said)");
        crate::app_mcp::drop_chat(&a);
    }

    #[test]
    fn theme_set_needs_one_field_and_announces() {
        let _serial = serial();
        let a = caller(true);
        let (code, msg) = err(&call(&a, "theme_set", json!({})));
        assert_eq!((code.as_str(), msg.as_str()), ("invalid_request", "Pass themeId or appearance."));
        assert_eq!(err(&call(&a, "theme_set", json!({"themeId": "neon"}))).0, "invalid_request");
        assert_eq!(err(&call(&a, "theme_set", json!({"appearance": "dim"}))).0, "invalid_request");
        let n = bus_len();
        let got = ok(&call(&a, "theme_set", json!({"themeId": "moss"})));
        assert_eq!(got, json!({"themeId": "moss"}));
        let ev: Vec<_> = bus_since(n).into_iter().filter(|e| e["kind"] == "theme_set").collect();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["themeId"], "moss");
        assert!(ev[0].get("appearance").is_none());
        assert!(cards(&a).is_empty(), "Full auto: no card");
        crate::app_mcp::drop_chat(&a);
    }

    #[test]
    fn theme_set_below_full_auto_asks() {
        let _serial = serial();
        let a = caller(false);
        let card = answer_card(&a, true);
        let got = ok(&call(&a, "theme_set", json!({"themeId": "tide", "appearance": "dark"})));
        assert_eq!(got, json!({"themeId": "tide", "appearance": "dark"}));
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["text"].as_str(), card["allow"].as_str()), (Some("Change the look"), Some("Switch Fridae to Tide in dark mode."), Some("Change it")));
        crate::app_mcp::drop_chat(&a);
    }
}
