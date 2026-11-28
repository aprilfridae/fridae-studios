//! What the toolbox tells the open window, so it changes at once as if the user had clicked (BUS events), and the
//! theme the window last reported.

use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::agents::BUS;

/// Fridae's built-in themes (ids as the page's `THEME_ORDER` in ui/index.html, and their names).
pub const THEMES: [(&str, &str); 6] = [("fridae", "Fridae"), ("graphite", "Graphite"), ("moss", "Moss"), ("tide", "Tide"), ("clay", "Clay"), ("lilac", "Lilac")];
/// The appearance modes.
pub const MODES: [&str; 3] = ["light", "dark", "system"];

/// The project list changed: the page reloads it.
pub fn projects_changed() {
    BUS.put("projects_changed", json!({}));
}

/// Settings changed (config keys): the page asks for the status again.
pub fn settings_changed(keys: &[&str]) {
    BUS.put("settings_changed", json!({"keys": keys}));
}

/// The window switches to this theme and/or appearance and keeps it (both live in the page's own storage).
pub fn theme_set(theme_id: Option<&str>, appearance: Option<&str>) {
    let mut ev = Map::new();
    if let Some(id) = theme_id {
        ev.insert("themeId".into(), json!(id));
    }
    if let Some(mode) = appearance {
        ev.insert("appearance".into(), json!(mode));
    }
    BUS.put("theme_set", Value::Object(ev));
}

static THEME_NOW: Mutex<Option<(String, String)>> = Mutex::new(None);

/// `POST /api/theme-now {id, mode}`: the window says which theme it shows (at load and on every change).
pub fn report_theme(body: &Value) -> Value {
    let (id, mode) = (body["id"].as_str().unwrap_or(""), body["mode"].as_str().unwrap_or(""));
    let ok = THEMES.iter().any(|(t, _)| *t == id) && MODES.contains(&mode);
    if ok {
        *THEME_NOW.lock().unwrap_or_else(|e| e.into_inner()) = Some((id.to_string(), mode.to_string()));
    }
    json!({"ok": ok})
}

/// (themeId, mode) as the window last said; None before it has.
pub fn theme_now() -> Option<(String, String)> {
    THEME_NOW.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_since(n: usize) -> Vec<Value> {
        BUS.after(n, 0.0)
    }

    #[test]
    fn events_put_on_the_bus() {
        let n = BUS.after(0, 0.0).len();
        projects_changed();
        settings_changed(&["chat_ai", "auto_update"]);
        theme_set(Some("moss"), None);
        theme_set(None, Some("dark"));
        let got = kinds_since(n);
        let find = |k: &str| got.iter().filter(|e| e["kind"] == k).cloned().collect::<Vec<_>>();
        assert_eq!(find("projects_changed").len(), 1);
        assert_eq!(find("settings_changed")[0]["keys"], json!(["chat_ai", "auto_update"]));
        let themes = find("theme_set");
        assert_eq!(themes[0]["themeId"], "moss");
        assert!(themes[0].get("appearance").is_none());
        assert_eq!(themes[1]["appearance"], "dark");
        assert!(themes[1].get("themeId").is_none());
    }

    #[test]
    fn theme_report_round_trips() {
        assert_eq!(report_theme(&json!({"id": "moss", "mode": "dark"})), json!({"ok": true}));
        assert_eq!(theme_now(), Some(("moss".to_string(), "dark".to_string())));
        assert_eq!(report_theme(&json!({"id": "neon", "mode": "dark"})), json!({"ok": false}));
        assert_eq!(report_theme(&json!({"id": "tide", "mode": "dim"})), json!({"ok": false}));
        assert_eq!(report_theme(&json!({})), json!({"ok": false}));
        assert_eq!(theme_now(), Some(("moss".to_string(), "dark".to_string())), "unchanged");
        assert_eq!(THEMES.map(|(id, _)| id), ["fridae", "graphite", "moss", "tide", "clay", "lilac"]);
        assert_eq!(MODES, ["light", "dark", "system"]);
    }
}
