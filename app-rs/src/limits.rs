//! The plan's usage limits, as Claude Code reports them. Every Claude turn's output carries a `rate_limit_event`
//! with the windows that apply (for example "seven_day": 41% used, resets Wednesday). The newest value of each window
//! is kept in `<work>/limits.json` and shown in the usage popover. Nothing is asked of any server: the numbers are
//! only as fresh as the last Claude turn. Gemini's tool reports no limits, so there are none for it.
use std::{fs, sync::Mutex};

use serde_json::{Map, Value, json};

use crate::paths;

static LOCK: Mutex<()> = Mutex::new(());

fn file() -> std::path::PathBuf {
    paths::work("limits.json")
}

/// The windows in one `rate_limit_event`, as {key: {"used": 0..1, "resets": secs, "status": "..."}}. A window
/// without a usable share is left out.
pub fn windows_of(event: &Value) -> Map<String, Value> {
    let info = &event["rate_limit_info"];
    let mut out = Map::new();
    for (key, w) in info["unifiedWindows"].as_object().into_iter().flatten() {
        let Some(used) = w["utilization"].as_f64().filter(|u| u.is_finite() && *u >= 0.0) else { continue };
        out.insert(key.clone(), json!({"used": used.min(1.0), "resets": w["resetsAt"].as_f64().unwrap_or(0.0), "status": w["status"].as_str().unwrap_or("")}));
    }
    out
}

/// `known` with this event's windows written over it, each stamped `seen`; windows whose reset time has passed go.
pub fn merged(mut known: Map<String, Value>, new: Map<String, Value>, now: f64) -> Map<String, Value> {
    for (key, mut w) in new {
        w["seen"] = json!(now);
        known.insert(key, w);
    }
    known.retain(|_, w| w["resets"].as_f64().is_none_or(|r| r <= 0.0 || r > now));
    known
}

fn read() -> Map<String, Value> {
    fs::read(file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// A Claude turn reported its limits: keep them.
pub fn record(event: &Value) {
    let new = windows_of(event);
    if new.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    let all = merged(read(), new, paths::now_secs());
    if let Some(dir) = file().parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(file(), serde_json::to_vec(&Value::Object(all)).unwrap_or_default());
}

/// What the page shows: {"claude": {window: {...}}}, without windows that have reset since they were seen.
pub fn state() -> Value {
    let _guard = LOCK.lock().unwrap();
    json!({"claude": merged(read(), Map::new(), paths::now_secs())})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_read_merged_and_dropped_once_reset() {
        let e = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "rateLimitType": "seven_day",
            "unifiedWindows": {"seven_day": {"utilization": 0.41, "resetsAt": 2000, "status": "allowed"}, "odd": {"status": "allowed"}}}});
        let w = windows_of(&e);
        assert_eq!(w.len(), 1, "a window with no share is left out");
        assert_eq!(w["seven_day"]["used"], 0.41);
        let known = merged(Map::new(), w, 1000.0);
        assert_eq!(known["seven_day"]["seen"], 1000.0);
        // a later event for another window is added; the first stays
        let five = windows_of(&json!({"rate_limit_info": {"unifiedWindows": {"five_hour": {"utilization": 3.0, "resetsAt": 1500}}}}));
        let known = merged(known, five, 1100.0);
        assert_eq!((known.len(), known["five_hour"]["used"].as_f64()), (2, Some(1.0)), "a share above 1 is capped");
        // past its reset a window is no longer shown
        let later = merged(known, Map::new(), 1600.0);
        assert!(later.contains_key("seven_day") && !later.contains_key("five_hour"));
        assert!(windows_of(&json!({"type": "result"})).is_empty());
    }
}
