//! Which AI and model an expert (or another helper a chat starts with `rbx_team`) runs on.
//! Spec: docs/superpowers/specs/2026-11-22-expert-ai-choice-design.md.
//!
//! 1. The user's table (Setup > Experts, config `expert_ai: {<expert id>: {"ai", "model"}}`) wins.
//! 2. Otherwise the `ai` and `model` the Lead asked for in the spawn.
//! 3. Otherwise the Lead's own AI and model.
//!
//! A choice that cannot be honoured is refused with a reason the Lead can act on, never quietly moved to another AI;
//! only a model the chosen AI does not offer falls back to that AI's default, and the result says so.

use serde_json::{Value, json};

use crate::{agents, antigravity, cursor, paths};

/// An AI and model asked for, by the table or by the Lead. Empty or "lead" means "Same as the Lead".
#[derive(Clone, Copy, Default)]
pub struct Want<'a> {
    pub ai: Option<&'a str>,
    pub model: Option<&'a str>,
}

impl<'a> Want<'a> {
    fn ai(&self) -> Option<&'a str> {
        self.ai.map(str::trim).filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("lead"))
    }
    fn model(&self) -> Option<&'a str> {
        self.model.map(str::trim).filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("lead"))
    }
}

/// Where the expert runs: its AI, its model (None: that AI's default) and, when a model was swapped, why.
#[derive(Debug, PartialEq)]
pub struct Choice {
    pub ai: String,
    pub model: Option<String>,
    pub note: Option<String>,
}

/// What the resolution needs to know about this computer, so it can be tested without one.
pub struct World<'a> {
    /// Whether an AI is installed (and offered here).
    pub installed: &'a dyn Fn(&str) -> bool,
    /// An AI's model ids; empty while its list has not arrived (the model is then passed on as asked).
    pub models: &'a dyn Fn(&str) -> Vec<String>,
    /// An AI's default model.
    pub default_model: &'a dyn Fn(&str) -> Option<String>,
}

/// The AIs an expert may run on: every AI offered here but ChatGPT, which the window no longer offers.
pub fn expert_ais() -> Vec<&'static str> {
    paths::AIS.iter().copied().filter(|a| *a != "chatgpt").collect()
}

pub fn ai_name(ai: &str) -> &'static str {
    match ai {
        "gemini" => "Gemini",
        "cursor" => "Cursor",
        "chatgpt" => "ChatGPT",
        _ => "Claude",
    }
}

/// The one rule: table, then request, then the Lead. `expert` is the expert's name, for the reasons.
pub fn resolve(table: Want, request: Want, lead_ai: &str, lead_model: Option<&str>, expert: &str, world: &World) -> Result<Choice, String> {
    let (ai, model, from_table) = if let Some(ai) = table.ai() {
        (ai, table.model(), true)
    } else if request.ai().is_some() || request.model().is_some() {
        (request.ai().unwrap_or(lead_ai), request.model(), false)
    } else {
        return Ok(Choice { ai: lead_ai.to_string(), model: lead_model.map(str::to_string), note: None });
    };
    let ai = match ai.to_ascii_lowercase().as_str() {
        "antigravity" => "gemini".to_string(),
        other => other.to_string(),
    };
    let known = expert_ais();
    if !known.contains(&ai.as_str()) {
        let list = known.join(", ");
        return Err(if from_table {
            format!("The user's Setup > Experts sets the {expert} to \"{ai}\", which is not an AI this app runs experts on. Ask the user to fix it in Setup.")
        } else {
            format!("\"{ai}\" is not an AI this app runs experts on; use one of {list}, or leave ai out to use your own.")
        });
    }
    // the Lead's own AI is running already
    if ai != lead_ai && !(world.installed)(&ai) {
        let name = ai_name(&ai);
        return Err(if from_table {
            format!("The user set the {expert} to run on {name} (Setup > Experts), but {name} is not installed. Ask the user to install it or change that setting.")
        } else {
            format!("{name} is not installed; run this expert on {} (leave ai out) or ask the user.", ai_name(lead_ai))
        });
    }
    // no model named: on the Lead's own AI the Lead's model, on another AI that AI's default
    let model = match model {
        Some(m) => Some(m.to_string()),
        None if ai == lead_ai && !from_table => lead_model.map(str::to_string),
        None => None,
    };
    let Some(m) = model else { return Ok(Choice { ai, model: None, note: None }) };
    let list = (world.models)(&ai);
    if list.is_empty() || list.iter().any(|x| *x == m) {
        return Ok(Choice { ai, model: Some(m), note: None });
    }
    let dflt = (world.default_model)(&ai);
    let note = format!("{} does not offer the model \"{m}\", so this expert runs on {}'s default ({}).", ai_name(&ai), ai_name(&ai), dflt.as_deref().unwrap_or("its own"));
    Ok(Choice { ai, model: dflt, note: Some(note) })
}

/// The real computer: what is installed, and each AI's own models.
pub fn world() -> World<'static> {
    World { installed: &|ai| expert_ais().contains(&ai) && paths::find_ai(ai).is_some(), models: &models_of, default_model: &default_of }
}

pub fn models_of(ai: &str) -> Vec<String> {
    match ai {
        "claude" => agents::MODELS.iter().map(|m| m.to_string()).collect(),
        "gemini" => antigravity::models().iter().map(|(id, _)| id.clone()).collect(),
        "cursor" => cursor::models().into_iter().map(|(id, _)| id).collect(),
        _ => Vec::new(),
    }
}

pub fn default_of(ai: &str) -> Option<String> {
    match ai {
        "claude" => Some("opus".to_string()),
        "gemini" => antigravity::default_model(),
        "cursor" => cursor::default_model(),
        _ => None,
    }
}

/// A model's name as the page shows it ("opus" is "Opus", Cursor's "auto" is "Auto").
pub fn model_name(ai: &str, id: &str) -> String {
    let listed = match ai {
        "gemini" => antigravity::models().iter().find(|(m, _)| m == id).map(|(_, n)| n.clone()),
        "cursor" => cursor::models().into_iter().find(|(m, _)| m == id).map(|(_, n)| n),
        _ => None,
    };
    listed.unwrap_or_else(|| {
        let mut c = id.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    })
}

/// "Cursor · Auto": what an expert runs on, for its line, when that is not what its Lead runs on (else None).
pub fn runs_on(ai: &str, model: Option<&str>, lead_ai: &str, lead_model: Option<&str>) -> Option<String> {
    let mine = model.map(str::to_string).or_else(|| default_of(ai));
    let lead = lead_model.map(str::to_string).or_else(|| default_of(lead_ai));
    if ai == lead_ai && mine == lead {
        return None;
    }
    Some(match mine {
        Some(m) => format!("{} · {}", ai_name(ai), model_name(ai, &m)),
        None => ai_name(ai).to_string(),
    })
}

/// The user's table entry for an expert: (ai, model), both None for "Same as the Lead".
pub fn table_entry(cfg: &serde_json::Map<String, Value>, expert: &str) -> (Option<String>, Option<String>) {
    let e = &cfg.get("expert_ai").cloned().unwrap_or(Value::Null)[expert];
    (e["ai"].as_str().map(str::to_string), e["model"].as_str().map(str::to_string))
}

/// Save one row of the table. An AI of "" or "lead" removes the row (Same as the Lead); a model of "" is the AI's
/// default. Refused for an AI experts cannot run on.
pub fn set_entry(expert: &str, ai: &str, model: &str) -> Result<Value, String> {
    let expert = expert.trim();
    if expert.is_empty() || expert.len() > 40 || !expert.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Not an expert id.".into());
    }
    let ai = ai.trim().to_ascii_lowercase();
    let mut cfg = paths::load_config();
    let mut table = cfg.get("expert_ai").and_then(Value::as_object).cloned().unwrap_or_default();
    if ai.is_empty() || ai == "lead" {
        table.remove(expert);
    } else {
        if !expert_ais().contains(&ai.as_str()) {
            return Err(format!("{} cannot run experts here.", ai_name(&ai)));
        }
        let model = model.trim();
        let mut row = json!({"ai": ai});
        if !model.is_empty() {
            row["model"] = json!(model);
        }
        table.insert(expert.to_string(), row);
    }
    cfg.insert("expert_ai".into(), Value::Object(table.clone()));
    paths::save_config(&cfg);
    Ok(Value::Object(table))
}

/// GET /api/experts: the kit's roster (every expert but the Lead), the user's table, and for each AI experts may run
/// on whether it is installed, its models and its default, with Cursor's documented gaps.
pub fn listing() -> Value {
    let roster: Value = std::fs::read_to_string(paths::kit_team(&paths::PATHS.kit).join("roster.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let experts: Vec<Value> = roster
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["kind"] != "lead" && e["id"].is_string())
        .map(|e| json!({"id": e["id"], "name": e["name"].as_str().or(e["id"].as_str()), "job": e["job"]}))
        .collect();
    let cfg = paths::load_config();
    let w = world();
    let ais: Vec<Value> = expert_ais()
        .into_iter()
        .map(|ai| {
            let models: Vec<Value> = match ai {
                "claude" => agents::MODELS.iter().map(|m| json!([m, model_name("claude", m)])).collect(),
                "gemini" => antigravity::models().iter().map(|(id, n)| json!([id, n])).collect(),
                "cursor" => cursor::models().into_iter().map(|(id, n)| json!([id, n])).collect(),
                _ => Vec::new(),
            };
            json!({"id": ai, "name": ai_name(ai), "installed": (w.installed)(ai), "models": models, "default": default_of(ai)})
        })
        .collect();
    json!({"experts": experts, "table": cfg.get("expert_ai").cloned().unwrap_or(json!({})), "ais": ais, "gaps": {"cursor": cursor::EXPERT_GAP}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with(installed: &'static [&'static str]) -> World<'static> {
        World {
            installed: Box::leak(Box::new(move |ai: &str| installed.contains(&ai))),
            models: &|ai| match ai {
                "cursor" => vec!["auto".into(), "gpt-5.3-codex".into()],
                "claude" => vec!["fable".into(), "opus".into(), "sonnet".into(), "haiku".into()],
                _ => Vec::new(),
            },
            default_model: &|ai| match ai {
                "cursor" => Some("auto".into()),
                "claude" => Some("opus".into()),
                _ => None,
            },
        }
    }

    const NONE: Want = Want { ai: None, model: None };

    #[test]
    fn with_nothing_chosen_the_expert_runs_like_its_lead() {
        let w = world_with(&["claude"]);
        assert_eq!(resolve(NONE, NONE, "claude", Some("sonnet"), "Coder", &w), Ok(Choice { ai: "claude".into(), model: Some("sonnet".into()), note: None }));
        let lead = Want { ai: Some("lead"), model: Some("") };
        assert_eq!(resolve(lead, lead, "claude", None, "Coder", &w).unwrap().ai, "claude", "\"Same as the Lead\" is no choice");
    }

    #[test]
    fn the_users_table_wins_over_the_leads_request() {
        let w = world_with(&["claude", "cursor"]);
        let table = Want { ai: Some("cursor"), model: Some("gpt-5.3-codex") };
        let asked = Want { ai: Some("claude"), model: Some("haiku") };
        assert_eq!(resolve(table, asked, "claude", Some("opus"), "World builder", &w), Ok(Choice { ai: "cursor".into(), model: Some("gpt-5.3-codex".into()), note: None }));
        // the table names an AI only: that AI's default, not the Lead's model
        let table = Want { ai: Some("cursor"), model: None };
        assert_eq!(resolve(table, asked, "claude", Some("opus"), "World builder", &w).unwrap().model, None);
    }

    #[test]
    fn the_leads_request_is_used_when_the_table_has_nothing() {
        let w = world_with(&["claude", "cursor"]);
        let asked = Want { ai: Some("Cursor"), model: None };
        assert_eq!(resolve(NONE, asked, "claude", Some("opus"), "Coder", &w), Ok(Choice { ai: "cursor".into(), model: None, note: None }));
        // a model alone is on the Lead's own AI
        let asked = Want { ai: None, model: Some("haiku") };
        assert_eq!(resolve(NONE, asked, "claude", Some("opus"), "Coder", &w).unwrap(), Choice { ai: "claude".into(), model: Some("haiku".into()), note: None });
        // the Lead's own AI named without a model keeps the Lead's model
        let asked = Want { ai: Some("claude"), model: None };
        assert_eq!(resolve(NONE, asked, "claude", Some("sonnet"), "Coder", &w).unwrap().model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn a_missing_ai_is_refused_with_a_reason_and_never_moved_elsewhere() {
        let w = world_with(&["claude"]);
        let asked = Want { ai: Some("gemini"), model: None };
        let why = resolve(NONE, asked, "claude", None, "Coder", &w).unwrap_err();
        assert_eq!(why, "Gemini is not installed; run this expert on Claude (leave ai out) or ask the user.");
        let table = Want { ai: Some("cursor"), model: Some("auto") };
        let why = resolve(table, NONE, "claude", None, "World builder", &w).unwrap_err();
        assert!(why.contains("The user set the World builder to run on Cursor") && why.contains("Ask the user"), "{why}");
        let why = resolve(NONE, Want { ai: Some("llama"), model: None }, "claude", None, "Coder", &w).unwrap_err();
        assert!(why.starts_with("\"llama\" is not an AI"), "{why}");
        assert!(resolve(NONE, Want { ai: Some("chatgpt"), model: None }, "claude", None, "Coder", &world_with(&["claude", "chatgpt"])).is_err(), "ChatGPT runs no experts");
    }

    #[test]
    fn an_unknown_model_falls_back_to_that_ais_default_and_says_so() {
        let w = world_with(&["claude", "cursor"]);
        let asked = Want { ai: Some("cursor"), model: Some("newest-astra") };
        let c = resolve(NONE, asked, "claude", None, "World builder", &w).unwrap();
        assert_eq!((c.ai.as_str(), c.model.as_deref()), ("cursor", Some("auto")));
        assert!(c.note.unwrap().contains("does not offer the model \"newest-astra\""));
        // an AI whose list has not arrived: the model is passed on, and that AI itself says if it is wrong
        let w = world_with(&["claude", "gemini"]);
        let c = resolve(NONE, Want { ai: Some("gemini"), model: Some("g-4") }, "claude", None, "Coder", &w).unwrap();
        assert_eq!((c.model.as_deref(), c.note), (Some("g-4"), None));
    }

    #[test]
    fn the_line_names_the_ai_only_when_it_differs_from_the_leads() {
        assert_eq!(runs_on("claude", Some("opus"), "claude", None), None, "opus is Claude's default");
        assert_eq!(runs_on("claude", Some("haiku"), "claude", Some("opus")).as_deref(), Some("Claude · Haiku"));
        assert!(runs_on("cursor", Some("gpt-5.3-codex"), "claude", Some("opus")).unwrap().starts_with("Cursor · "));
    }

    #[test]
    fn the_table_reads_and_saves_rows() {
        let mut cfg = serde_json::Map::new();
        cfg.insert("expert_ai".into(), json!({"world": {"ai": "cursor", "model": "auto"}}));
        assert_eq!(table_entry(&cfg, "world"), (Some("cursor".into()), Some("auto".into())));
        assert_eq!(table_entry(&cfg, "coder"), (None, None));
        assert!(set_entry("../x", "cursor", "").is_err());
    }
}
