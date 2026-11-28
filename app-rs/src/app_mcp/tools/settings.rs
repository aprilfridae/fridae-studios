//! settings_read and settings_update: the app's identity and a short fixed list of app-wide preferences, each changed
//! through the same code its Setup control runs (port of T3's environment_read and environment_preferences_update).

use serde_json::{Map, Value, json};

use super::{ai_name, opt_bool, opt_text, reads, words, writes_environment};
use crate::app_mcp::{Call, Hints, Labels, Outcome, Summary, ToolDef, access, card::CardSpec, events, fail};
use crate::paths;

/// The settings an agent may change: (argument, config key, Setup's words, guarded so always a card).
const SETTABLE: [(&str, &str, &str, bool); 7] = [
    ("newChatAi", "chat_ai", "The AI new chats start on", false),
    ("autoUpdate", "auto_update", "Install updates by itself", false),
    ("expertSummaries", "expert_summaries", "Plain-English summary of each expert's work", true),
    ("strongLead", crate::swarm::STRONG_LEAD_KEY, "Keep the Lead on a strong model", true),
    ("leadPicksAi", crate::expert_ai::LEAD_PICKS, "Lead may pick the AI", true),
    ("geminiImages", "gemini_images", "Agents may generate images with Gemini", true),
    ("geminiPaidTextures", "gemini_use_api", "paid Gemini textures", true),
];

/// What no tool can change, in Setup's words (the user changes these in Setup).
const NEVER_SETTABLE: [&str; 12] = [
    "Gemini API key",
    "programs agents may run",
    "Use API keys from my environment",
    "Studio plugin link",
    "list of games",
    "Experts table",
    "Knowledge index",
    "Most experts working at once",
    "OpenCode's model",
    "Second accounts",
    "Share my skills",
    "Let my other computers connect",
];

pub fn defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "settings_read",
            title: "Read settings",
            description: "Read this app's identity and preferences. AIs and models: `fridae_capabilities`.",
            schema: schema_read,
            hints: Hints::READ,
            access: reads,
            labels: labels_read,
            always_load: false,
            handler: read,
        },
        ToolDef {
            name: "settings_update",
            title: "Change settings",
            description: "Update app-wide preferences. Omitted fields are unchanged. Below Full auto, and always for some, the user is asked first with a card in this chat.",
            schema: schema_update,
            hints: Hints::WRITE,
            access: writes_environment,
            labels: labels_update,
            always_load: false,
            handler: update,
        },
    ]
}

fn schema_read() -> Value {
    json!({"type": "object", "properties": {}})
}

fn schema_update() -> Value {
    let mut props = Map::new();
    for (arg, _, words, guarded) in SETTABLE {
        let ask = if guarded { " The user is always asked first." } else { "" };
        let p = if arg == "newChatAi" {
            json!({"type": "string", "enum": paths::AIS, "description": format!("{words}.{ask}")})
        } else {
            json!({"type": "boolean", "description": format!("{words}.{ask}")})
        };
        props.insert(arg.into(), p);
    }
    json!({"type": "object", "properties": props})
}

fn labels_read(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Read", "Reading", "Read", "settings", "Read", "setting list", "setting lists")
}

fn labels_update(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Change", "Changing", "Changed", "settings", "Changed", "setting", "settings")
}

/// Each settable preference as it is now (never a key or a secret).
fn preferences() -> Value {
    json!({
        "newChatAi": paths::default_ai(),
        "autoUpdate": crate::update::state()["auto"].as_bool().unwrap_or(true),
        "expertSummaries": paths::expert_summaries_on(),
        "strongLead": paths::strong_lead_on(),
        "leadPicksAi": paths::setting(crate::expert_ai::LEAD_PICKS).as_bool().unwrap_or(true),
        "geminiImages": paths::gemini_images_on(),
        "geminiPaidTextures": paths::setting("gemini_use_api").as_bool().unwrap_or(false),
    })
}

fn read(_call: &Call, _args: &Map<String, Value>) -> Outcome {
    let labels: Map<String, Value> = SETTABLE.iter().map(|(arg, _, words, _)| (arg.to_string(), json!(words))).collect();
    Ok(json!({"app": "Fridae", "version": paths::VERSION, "platform": std::env::consts::OS, "preferences": preferences(), "labels": labels, "neverSettable": NEVER_SETTABLE}))
}

/// One change asked for.
enum Change {
    Ai(String),
    Switch(&'static str, &'static str, &'static str, bool),
}

impl Change {
    fn line(&self) -> String {
        match self {
            Change::Ai(ai) => format!("Start new chats on {}.", ai_name(ai)),
            Change::Switch(_, _, words, on) => format!("Turn {} \"{words}\".", if *on { "on" } else { "off" }),
        }
    }

    fn key(&self) -> &'static str {
        match self {
            Change::Ai(_) => "chat_ai",
            Change::Switch(_, key, _, _) => key,
        }
    }

    /// Made through the same code its Setup control's route runs.
    fn apply(&self) {
        match self {
            Change::Ai(ai) => paths::update_config(|cfg| {
                cfg.insert("chat_ai".into(), json!(ai));
            }),
            Change::Switch("autoUpdate", _, _, on) => crate::update::set_auto(*on),
            Change::Switch(_, key, _, on) => paths::update_config(|cfg| {
                cfg.insert(key.to_string(), json!(on));
            }),
        }
    }
}

/// The card for a set of changes: one line each.
fn card_for(changes: &[Change]) -> CardSpec {
    match changes {
        [Change::Ai(_)] => CardSpec { title: "Change a setting".into(), text: changes[0].line(), allow: "Change it".into() },
        [Change::Switch(_, _, _, on)] => CardSpec { title: "Change a setting".into(), text: changes[0].line(), allow: if *on { "Turn it on" } else { "Turn it off" }.into() },
        _ => CardSpec { title: "Change settings".into(), text: changes.iter().map(Change::line).collect::<Vec<_>>().join(" "), allow: "Change them".into() },
    }
}

fn update(call: &Call, args: &Map<String, Value>) -> Outcome {
    let mut changes = Vec::new();
    let mut always = false;
    for (arg, key, words, guarded) in SETTABLE {
        if !args.contains_key(arg) {
            continue;
        }
        if arg == "newChatAi" {
            let ai = opt_text(args, arg)?.unwrap_or_default();
            let ai = ai.trim();
            if !paths::AIS.contains(&ai) {
                return Err(fail("invalid_request", format!("That AI is not one Fridae offers here. Use one of: {}.", paths::AIS.join(", "))));
            }
            changes.push(Change::Ai(ai.to_string()));
        } else if let Some(on) = opt_bool(args, arg)? {
            changes.push(Change::Switch(arg, key, words, on));
        }
        always |= guarded;
    }
    if changes.is_empty() {
        return Err(fail("invalid_request", "Pass at least one setting to change. settings_read lists them."));
    }
    let caller = access::writing_caller(call.scope)?;
    access::environment_gate(call, &caller, card_for(&changes), always)?;
    for c in &changes {
        c.apply();
    }
    let keys: Vec<&str> = changes.iter().map(Change::key).collect();
    events::settings_changed(&keys);
    let changed: Vec<&str> = SETTABLE.iter().filter(|(arg, ..)| args.contains_key(*arg)).map(|(arg, ..)| *arg).collect();
    Ok(json!({"changed": changed, "preferences": preferences()}))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use crate::paths;
    use serde_json::{Value, json};

    fn config(key: &str) -> Option<Value> {
        paths::load_config().get(key).cloned()
    }

    fn put_back(key: &str, was: Option<Value>) {
        paths::update_config(|c| match was {
            Some(v) => c.insert(key.into(), v),
            None => c.remove(key),
        });
    }

    #[test]
    fn settings_update_ordinary_and_guarded() {
        let _serial = serial();
        let (chat_ai, images, strong) = (config("chat_ai"), config("gemini_images"), config(crate::swarm::STRONG_LEAD_KEY));
        let a = caller(true);
        let n = bus_len();
        let got = ok(&call(&a, "settings_update", json!({"newChatAi": "chatgpt"})));
        assert_eq!(got["preferences"]["newChatAi"].is_string(), true, "{got}");
        assert_eq!(config("chat_ai"), Some(json!("chatgpt")));
        assert!(cards(&a).is_empty(), "an ordinary setting in Full auto: no card");
        let changed: Vec<Value> = bus_since(n).into_iter().filter(|e| e["kind"] == "settings_changed").collect();
        assert_eq!(changed.last().unwrap()["keys"], json!(["chat_ai"]));
        assert_eq!(err(&call(&a, "settings_update", json!({"newChatAi": "nope"}))).0, "invalid_request");
        assert_eq!(err(&call(&a, "settings_update", json!({}))).0, "invalid_request");

        // a guarded setting: a card even in Full auto
        let card = answer_card(&a, false);
        let (code, _) = err(&call(&a, "settings_update", json!({"geminiImages": true})));
        assert_eq!(code, "declined");
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["text"].as_str(), card["allow"].as_str()), (Some("Change a setting"), Some("Turn on \"Agents may generate images with Gemini\"."), Some("Turn it on")));
        let card = answer_card(&a, true);
        ok(&call(&a, "settings_update", json!({"geminiImages": false, "strongLead": true})));
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["allow"].as_str()), (Some("Change settings"), Some("Change them")));
        assert_eq!(card["text"], "Turn on \"Keep the Lead on a strong model\". Turn off \"Agents may generate images with Gemini\".", "one line each, in Setup's order");
        assert!(!paths::gemini_images_on());

        assert_eq!(rpc_error(&a, "settings_update", json!({"geminiKey": "x"})).0, -32602, "never settable");
        crate::app_mcp::drop_chat(&a);
        put_back("chat_ai", chat_ai);
        put_back("gemini_images", images);
        put_back(crate::swarm::STRONG_LEAD_KEY, strong);
    }

    #[test]
    fn settings_update_below_full_auto_asks_for_an_ordinary_one() {
        let _serial = serial();
        let a = caller(false);
        let card = answer_card(&a, false);
        assert_eq!(err(&call(&a, "settings_update", json!({"newChatAi": "gemini"}))).0, "declined");
        let card = card.join().unwrap();
        assert_eq!((card["text"].as_str(), card["allow"].as_str()), (Some("Start new chats on Gemini."), Some("Change it")));
        crate::app_mcp::drop_chat(&a);
    }

    #[test]
    fn settings_read_never_shows_secrets() {
        let _serial = serial();
        let was = config("gemini_key");
        paths::update_config(|c| c.insert("gemini_key".into(), json!("sk-test-NOT-SHOWN-4711")));
        let a = caller(false);
        let r = call(&a, "settings_read", json!({}));
        put_back("gemini_key", was);
        assert!(!r.to_string().contains("NOT-SHOWN-4711"), "{r}");
        let got = ok(&r);
        assert_eq!(got["app"], "Fridae");
        assert_eq!(got["version"], paths::VERSION);
        for k in ["newChatAi", "autoUpdate", "expertSummaries", "strongLead", "leadPicksAi", "geminiImages", "geminiPaidTextures"] {
            assert!(got["preferences"].get(k).is_some(), "{k}");
            assert!(got["labels"][k].is_string(), "{k}");
        }
        assert!(got["neverSettable"].as_array().unwrap().contains(&json!("Gemini API key")));
        crate::app_mcp::drop_chat(&a);
    }
}
