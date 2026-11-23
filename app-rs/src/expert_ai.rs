//! Which AI and model an expert (or another helper a chat starts with `rbx_team`) runs on.
//! Spec: docs/superpowers/specs/2026-11-22-expert-ai-choice-design.md.
//!
//! 1. The user's table (Setup > Experts, config `expert_ai: {<expert id>: {"ai", "model"}}`) wins.
//! 2. Otherwise the `ai` and `model` the Lead asked for in the spawn.
//! 3. Otherwise the Lead's own AI and model.
//!
//! A choice that cannot be honoured is refused with a reason the Lead can act on, never quietly moved to another AI;
//! only a model the chosen AI does not offer runs on the nearest that AI has (OpenCode: the same model from another
//! provider, or its free twin) or on that AI's default, and the result says which model runs and why. The Lead is also
//! told when the user's table won over what it asked for, and when OpenCode has only free models because Fridae's
//! copy of it is not signed in. A model swapped later, at a turn (`swap_line`), is said in the chat and to the Lead.

use serde_json::{Value, json};

use crate::{agents, antigravity, cursor, opencode, paths};

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
    /// The model an AI lists that is nearest to one it does not list exactly (AI, model asked for); None: its default.
    pub nearest: &'a dyn Fn(&str, &str) -> Option<String>,
    /// Why an AI offers fewer models than the user may expect, when the app knows (OpenCode that is not signed in),
    /// in words for the Lead.
    pub why: &'a dyn Fn(&str) -> Option<String>,
    /// Setup's "The Lead may pick an AI for a job" (on unless the user switched it off).
    pub lead_may_pick: bool,
}

/// The settings key behind Setup's "The Lead may pick an AI for a job".
pub const LEAD_PICKS: &str = "lead_picks_ai";

pub fn lead_may_pick(cfg: &serde_json::Map<String, Value>) -> bool {
    cfg.get(LEAD_PICKS).and_then(Value::as_bool).unwrap_or(true)
}

/// A model id as an AI's tool takes it on its command line: letters, digits and `. _ : [ ] = , - /` (OpenCode's
/// `provider/model`), not starting with a dash, at most 120 characters. One from an agent is checked before use.
pub fn model_id_ok(id: &str) -> bool {
    id.len() <= 120
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || "._:[]=,-/".contains(c))
}

/// Why a Lead's own pick is refused while the user has switched that off.
pub const LEAD_PICKS_OFF: &str = "The user has switched off \"The Lead may pick an AI for a job\" in Setup, so leave ai and model out: the expert then runs as the user set it in Setup > Experts, or like you.";

/// The AIs an expert may run on: every AI offered here but ChatGPT, which the window no longer offers.
pub fn expert_ais() -> Vec<&'static str> {
    crate::caps::expert_ais()
}

pub fn ai_name(ai: &str) -> &'static str {
    match ai {
        "gemini" => "Gemini",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        "chatgpt" => "ChatGPT",
        _ => "Claude",
    }
}

/// The one rule: table, then request, then the Lead. `expert` is the expert's name, for the reasons.
pub fn resolve(table: Want, request: Want, lead_ai: &str, lead_model: Option<&str>, expert: &str, world: &World) -> Result<Choice, String> {
    let (ai, model, from_table) = if let Some(ai) = table.ai() {
        (ai, table.model(), true)
    } else if request.ai().is_some() || request.model().is_some() {
        if !world.lead_may_pick {
            return Err(LEAD_PICKS_OFF.to_string());
        }
        if request.model().is_some_and(|m| !model_id_ok(m)) {
            return Err("That model id is not one an AI could take; leave model out to use the AI's default.".to_string());
        }
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
    let name = ai_name(&ai);
    let mut notes: Vec<String> = Vec::new();
    // the user's table won over something else the Lead asked for
    let other_ai = request.ai().is_some_and(|a| !a.eq_ignore_ascii_case(&ai) && !(a.eq_ignore_ascii_case("antigravity") && ai == "gemini"));
    if from_table && (other_ai || request.model().is_some_and(|m| Some(m) != model.as_deref())) {
        notes.push(format!("The user set the {expert} to run on {name} in Setup > Experts, so that was used, not the ai and model you asked for."));
    }
    // a model the AI does not list exactly: the nearest it has, else its default, and the Lead is told which
    let list = (world.models)(&ai);
    let model = match model {
        Some(m) if !list.is_empty() && !list.contains(&m) => {
            let (runs, on) = match (world.nearest)(&ai, &m) {
                Some(n) => (Some(n.clone()), format!("{n}, the nearest it has")),
                None => {
                    let dflt = (world.default_model)(&ai);
                    let on = format!("{name}'s default ({})", dflt.as_deref().unwrap_or("its own"));
                    (dflt, on)
                }
            };
            notes.push(format!("{name} does not offer the model \"{m}\", so this expert runs on {on}."));
            runs
        }
        other => other,
    };
    notes.extend((world.why)(&ai));
    Ok(Choice { ai, model, note: (!notes.is_empty()).then(|| notes.join(" ")) })
}

/// What is said when a turn runs on another model than the one set for it (a model asked for while its AI's list had
/// not arrived, or one that has since gone): for the chat, in the user's words, and for the Lead waiting for the result.
#[derive(Debug, PartialEq)]
pub struct Swap {
    pub chat: String,
    pub lead: String,
}

/// None when the turn runs on what was asked for, when nothing was, or when what was asked for could not be a model id.
/// `why` as `World::why`.
pub fn swap_line(ai: &str, asked: Option<&str>, runs: Option<&str>, why: &dyn Fn(&str) -> Option<String>) -> Option<Swap> {
    let asked = asked.filter(|a| !a.is_empty() && model_id_ok(a) && Some(*a) != runs)?;
    let name = ai_name(ai);
    let (shown, id) = match runs {
        Some(m) => (model_name(ai, m), format!("the model {m}")),
        None => (format!("{name}'s own choice"), format!("{name}'s own choice of model")),
    };
    let reason = why(ai);
    // the Lead reads "The user can sign in"; the user reads "Sign in"
    let to_user = reason.as_deref().map(|r| format!(" {}", r.replace("The user can sign in", "Sign in"))).unwrap_or_default();
    let to_lead = reason.map(|r| format!(" {r}")).unwrap_or_default();
    Some(Swap {
        chat: format!("{name} does not offer the model \"{asked}\", so this ran on {shown}.{to_user}"),
        lead: format!("it ran on {id}, not \"{asked}\" as set: {name} does not offer that one.{to_lead}"),
    })
}

/// The one line an OpenCode chat gets, once, when it runs on a free model because Fridae's OpenCode is not signed in.
pub const FREE_MODEL_LINE: &str = "This chat is on a free model. Sign in under Setup, Connections to use your OpenCode plan.";

/// Why an AI offers fewer models than the user may expect, when the app knows: OpenCode that is not signed in.
pub fn why_fewer(ai: &str) -> Option<String> {
    (ai == "opencode" && opencode::free_only()).then(|| opencode::NOT_SIGNED_IN.to_string())
}

/// The real computer: what is installed, and each AI's own models.
pub fn world() -> World<'static> {
    World {
        installed: &|ai| expert_ais().contains(&ai) && paths::find_ai(ai).is_some(),
        models: &models_of,
        default_model: &default_of,
        nearest: &|ai, model| if ai == "opencode" { opencode::nearest_model(model) } else { None },
        why: &why_fewer,
        lead_may_pick: lead_may_pick(&paths::guarded_settings()),
    }
}

pub fn models_of(ai: &str) -> Vec<String> {
    match ai {
        "claude" => agents::MODELS.iter().map(|m| m.to_string()).collect(),
        "gemini" => antigravity::models().iter().map(|(id, _)| id.clone()).collect(),
        "cursor" => cursor::models().into_iter().map(|(id, _)| id).collect(),
        "opencode" => opencode::models().into_iter().map(|(id, _)| id).collect(),
        _ => Vec::new(),
    }
}

pub fn default_of(ai: &str) -> Option<String> {
    match ai {
        "claude" => Some("opus".to_string()),
        "gemini" => antigravity::default_model(),
        "cursor" => cursor::default_model(),
        "opencode" => opencode::default_model(),
        _ => None,
    }
}

/// A model's name as the page shows it ("opus" is "Opus", Cursor's "auto" is "Auto").
pub fn model_name(ai: &str, id: &str) -> String {
    let listed = match ai {
        "gemini" => antigravity::models().iter().find(|(m, _)| m == id).map(|(_, n)| n.clone()),
        "cursor" => cursor::models().into_iter().find(|(m, _)| m == id).map(|(_, n)| n),
        "opencode" => Some(opencode::model_name(id)),
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
    if !(ai.is_empty() || ai == "lead") && !expert_ais().contains(&ai.as_str()) {
        return Err(format!("{} cannot run experts here.", ai_name(&ai)));
    }
    let model = model.trim();
    if !model.is_empty() && !model_id_ok(model) {
        return Err("Not a model id.".into());
    }
    Ok(paths::update_config(|cfg| {
        let mut table = cfg.get("expert_ai").and_then(Value::as_object).cloned().unwrap_or_default();
        if ai.is_empty() || ai == "lead" {
            table.remove(expert);
        } else {
            let mut row = json!({"ai": ai});
            if !model.is_empty() {
                row["model"] = json!(model);
            }
            table.insert(expert.to_string(), row);
        }
        cfg.insert("expert_ai".into(), Value::Object(table.clone()));
        Value::Object(table)
    }))
}

/// What a chat may ask for with `rbx_team` spawn's `ai` and `model`: each AI experts run on, whether it is installed,
/// its default model, its model ids, and what the app knows about why it has few (`why_fewer`). `team::watch` keeps
/// this in the work folder, where `rbx_team` action "models" reads it.
pub fn for_lead() -> Value {
    let w = world();
    let ais: Vec<Value> = expert_ais()
        .into_iter()
        .map(|ai| json!({"id": ai, "name": ai_name(ai), "installed": (w.installed)(ai), "default": default_of(ai), "models": models_of(ai), "note": why_fewer(ai)}))
        .collect();
    json!({"lead_may_pick": w.lead_may_pick, "ais": ais})
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
    let cfg = paths::guarded_settings();
    let w = world();
    let ais: Vec<Value> = expert_ais()
        .into_iter()
        .map(|ai| {
            let models: Vec<Value> = match ai {
                "claude" => agents::MODELS.iter().map(|m| json!([m, model_name("claude", m)])).collect(),
                "gemini" => antigravity::models().iter().map(|(id, n)| json!([id, n])).collect(),
                "cursor" => cursor::models().into_iter().map(|(id, n)| json!([id, n])).collect(),
                "opencode" => opencode::models().into_iter().map(|(id, n)| json!([id, n])).collect(),
                _ => Vec::new(),
            };
            json!({"id": ai, "name": ai_name(ai), "installed": (w.installed)(ai), "models": models, "default": default_of(ai)})
        })
        .collect();
    json!({"experts": experts, "table": cfg.get("expert_ai").cloned().unwrap_or(json!({})), "ais": ais, "gaps": {"cursor": cursor::EXPERT_GAP}, "lead_may_pick": lead_may_pick(&cfg)})
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
            nearest: &|_, _| None,
            why: &|_| None,
            lead_may_pick: true,
        }
    }

    /// OpenCode as Fridae's copy was on 23 November 2026: never signed in, so it listed free models only.
    fn opencode_not_signed_in() -> World<'static> {
        World {
            models: &|ai| match ai {
                "opencode" => vec!["opencode/beam-free".into(), "opencode/big-pickle".into(), "opencode/deepseek-v4.1-flash-free".into()],
                "claude" => vec!["opus".into(), "sonnet".into()],
                _ => Vec::new(),
            },
            default_model: &|ai| match ai {
                "opencode" => Some("opencode/big-pickle".into()),
                "claude" => Some("opus".into()),
                _ => None,
            },
            nearest: &|ai, m| (ai == "opencode" && m == "opencode/deepseek-v4.1-flash").then(|| "opencode/deepseek-v4.1-flash-free".to_string()),
            why: &|ai| (ai == "opencode").then(|| opencode::NOT_SIGNED_IN.to_string()),
            ..world_with(&["claude", "opencode"])
        }
    }

    #[test]
    fn a_model_opencode_lacks_runs_on_its_twin_and_the_lead_is_told_which_and_why() {
        // last night: the Lead asked for a paid model of a copy that is not signed in
        let w = opencode_not_signed_in();
        let asked = Want { ai: Some("opencode"), model: Some("opencode/deepseek-v4.1-flash") };
        let c = resolve(NONE, asked, "claude", Some("opus"), "Coder", &w).unwrap();
        assert_eq!((c.ai.as_str(), c.model.as_deref()), ("opencode", Some("opencode/deepseek-v4.1-flash-free")));
        assert_eq!(
            c.note.as_deref(),
            Some("OpenCode does not offer the model \"opencode/deepseek-v4.1-flash\", so this expert runs on opencode/deepseek-v4.1-flash-free, the nearest it has. Fridae's OpenCode is not signed in to a plan, so only free models are available. The user can sign in under Setup, Connections.")
        );
        // nothing like it: the default, named, with the same reason
        let c = resolve(NONE, Want { ai: Some("opencode"), model: Some("opencode-go/kimi-k3") }, "claude", None, "Coder", &w).unwrap();
        assert_eq!(c.model.as_deref(), Some("opencode/big-pickle"));
        let note = c.note.unwrap();
        assert!(note.starts_with("OpenCode does not offer the model \"opencode-go/kimi-k3\", so this expert runs on OpenCode's default (opencode/big-pickle).") && note.ends_with("Setup, Connections."), "{note}");
        // no model named: OpenCode's default runs, and the Lead still learns that it is a free one and why
        let c = resolve(NONE, Want { ai: Some("opencode"), model: None }, "claude", Some("opus"), "Coder", &w).unwrap();
        assert_eq!((c.model.as_deref(), c.note.as_deref()), (None, Some(opencode::NOT_SIGNED_IN)));
        // the user's table: the same
        let c = resolve(Want { ai: Some("opencode"), model: Some("opencode/big-pickle") }, NONE, "claude", None, "Coder", &w).unwrap();
        assert_eq!((c.model.as_deref(), c.note.as_deref()), (Some("opencode/big-pickle"), Some(opencode::NOT_SIGNED_IN)));
        // an expert that runs like its Lead is not what the Lead asked about: nothing is added
        assert_eq!(resolve(NONE, NONE, "opencode", Some("opencode/big-pickle"), "Coder", &w).unwrap().note, None);
    }

    #[test]
    fn the_lead_is_told_when_the_users_table_won_over_what_it_asked_for() {
        let w = world_with(&["claude", "cursor"]);
        let table = Want { ai: Some("cursor"), model: Some("auto") };
        let c = resolve(table, Want { ai: Some("claude"), model: Some("haiku") }, "claude", Some("opus"), "World builder", &w).unwrap();
        assert_eq!(c.note.as_deref(), Some("The user set the World builder to run on Cursor in Setup > Experts, so that was used, not the ai and model you asked for."));
        // asked for what the table says anyway, or for nothing: nothing to tell
        assert_eq!(resolve(table, Want { ai: Some("cursor"), model: Some("auto") }, "claude", None, "World builder", &w).unwrap().note, None);
        assert_eq!(resolve(table, Want { ai: Some("Cursor"), model: None }, "claude", None, "World builder", &w).unwrap().note, None);
        assert_eq!(resolve(table, NONE, "claude", None, "World builder", &w).unwrap().note, None);
    }

    #[test]
    fn a_turn_that_runs_on_another_model_than_asked_says_so() {
        let none = |_: &str| None;
        // what was asked for runs, or nothing was asked for: nothing to say
        assert_eq!(swap_line("cursor", Some("auto"), Some("auto"), &none), None);
        assert_eq!(swap_line("cursor", None, Some("auto"), &none), None);
        assert_eq!(swap_line("claude", Some("--yolo"), Some("opus"), &none), None, "not a model id: never shown");
        let s = swap_line("claude", Some("newest"), Some("opus"), &none).unwrap();
        assert_eq!((s.chat.as_str(), s.lead.as_str()), ("Claude does not offer the model \"newest\", so this ran on Opus.", "it ran on the model opus, not \"newest\" as set: Claude does not offer that one."));
        // OpenCode, not signed in: the reason, in the user's words for the chat and the Lead's for the Lead
        let why = |ai: &str| (ai == "opencode").then(|| opencode::NOT_SIGNED_IN.to_string());
        let s = swap_line("opencode", Some("opencode/deepseek-v4.1-flash"), Some("opencode/deepseek-v4.1-flash-free"), &why).unwrap();
        assert_eq!(s.chat, "OpenCode does not offer the model \"opencode/deepseek-v4.1-flash\", so this ran on OpenCode Zen · deepseek-v4.1-flash-free. Fridae's OpenCode is not signed in to a plan, so only free models are available. Sign in under Setup, Connections.");
        assert!(s.lead.ends_with("OpenCode does not offer that one. Fridae's OpenCode is not signed in to a plan, so only free models are available. The user can sign in under Setup, Connections."), "{}", s.lead);
        assert_eq!(FREE_MODEL_LINE, "This chat is on a free model. Sign in under Setup, Connections to use your OpenCode plan.");
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
        let c = resolve(table, asked, "claude", Some("opus"), "World builder", &w).unwrap();
        assert_eq!((c.ai.as_str(), c.model.as_deref()), ("cursor", Some("gpt-5.3-codex")));
        assert!(c.note.is_some_and(|n| n.starts_with("The user set the World builder to run on Cursor")), "the Lead is told its own pick was not used");
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
        assert_eq!(c.note.as_deref(), Some("Cursor does not offer the model \"newest-astra\", so this expert runs on Cursor's default (auto)."));
        // an AI whose list has not arrived: the model is passed on, and that AI itself says if it is wrong
        let w = world_with(&["claude", "gemini"]);
        let c = resolve(NONE, Want { ai: Some("gemini"), model: Some("g-4") }, "claude", None, "Coder", &w).unwrap();
        assert_eq!((c.model.as_deref(), c.note), (Some("g-4"), None));
    }

    #[test]
    fn with_the_switch_off_the_leads_pick_is_refused_but_the_table_still_applies() {
        let mut w = world_with(&["claude", "cursor"]);
        w.lead_may_pick = false;
        let asked = Want { ai: Some("cursor"), model: Some("auto") };
        assert_eq!(resolve(NONE, asked, "claude", Some("opus"), "Coder", &w).unwrap_err(), LEAD_PICKS_OFF);
        assert_eq!(resolve(NONE, Want { ai: None, model: Some("haiku") }, "claude", None, "Coder", &w).unwrap_err(), LEAD_PICKS_OFF);
        let table = Want { ai: Some("cursor"), model: None };
        assert_eq!(resolve(table, asked, "claude", Some("opus"), "World builder", &w).unwrap().ai, "cursor", "the user's own table");
        assert_eq!(resolve(NONE, NONE, "claude", Some("opus"), "Coder", &w).unwrap().model.as_deref(), Some("opus"));
        assert!(lead_may_pick(&serde_json::Map::new()), "on unless switched off");
    }

    #[test]
    fn a_model_id_from_an_agent_is_checked() {
        for ok in ["auto", "gpt-5.3-codex", "opencode-go/glm-5.3", "claude-4.5-sonnet[1m]", "gemini-3.1-pro:high"] {
            assert!(model_id_ok(ok), "{ok}");
        }
        for bad in ["--yolo", "", "a b", "x;rm", "-m", &"a".repeat(121)] {
            assert!(!model_id_ok(bad), "{bad}");
        }
        let w = world_with(&["claude", "gemini"]);
        assert!(resolve(NONE, Want { ai: Some("gemini"), model: Some("--force") }, "claude", None, "Coder", &w).is_err());
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
