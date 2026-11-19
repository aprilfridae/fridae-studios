//! How much each turn used: tokens from every AI, and Claude Code's own cost estimate.
//!
//! One line per turn is appended to `work/usage.jsonl` ({t, ai, chat, input, output, cache_read, cache_write, cost}).
//! The window adds them up for "today" in the user's own time zone, so nothing here needs a calendar.

use std::{
    fs,
    io::Write,
    sync::Mutex,
};

use serde_json::{Value, json};

use crate::paths;

static LOCK: Mutex<()> = Mutex::new(());
/// Lines older than this are dropped when the file is next tidied.
const KEEP_SECS: f64 = 62.0 * 24.0 * 3600.0;

/// One turn's usage, as each AI reports it.
#[derive(Default, Clone, Copy)]
pub struct Turn {
    /// Fresh input tokens (not read from the cache).
    pub input: u64,
    /// Output tokens, thinking included.
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Claude Code's estimate at API prices. The other AIs do not report one.
    pub cost: Option<f64>,
}

impl Turn {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.cost.unwrap_or(0.0) == 0.0
    }

    /// From Claude Code's `result` event.
    pub fn from_claude(e: &Value) -> Turn {
        let u = &e["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Turn {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
            cost: e["total_cost_usd"].as_f64(),
        }
    }

    /// From Antigravity's `result.usage`.
    pub fn from_antigravity(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Turn { input: n("input_tokens"), output: n("output_tokens") + n("thinking_tokens"), cache_read: n("cache_read_tokens"), ..Default::default() }
    }

    /// From Codex's `turn.completed.usage`, whose input count includes the cached part.
    pub fn from_codex(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let cached = n("cached_input_tokens");
        Turn { input: n("input_tokens").saturating_sub(cached), output: n("output_tokens") + n("reasoning_output_tokens"), cache_read: cached, ..Default::default() }
    }

    pub fn json(&self) -> Value {
        json!({"input": self.input, "output": self.output, "cache_read": self.cache_read, "cache_write": self.cache_write,
               "total": self.total(), "cost": self.cost})
    }
}

fn file() -> std::path::PathBuf {
    paths::work("usage.jsonl")
}

/// Add a turn to the log.
pub fn record(ai: &str, chat: &str, turn: &Turn) {
    if turn.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    let mut line = turn.json();
    line["t"] = json!(paths::now_secs());
    line["ai"] = json!(ai);
    line["chat"] = json!(chat);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(file()) {
        let _ = writeln!(f, "{line}");
    }
}

/// Every turn since `since` (seconds), oldest first.
pub fn since(since: f64) -> Vec<Value> {
    let _guard = LOCK.lock().unwrap();
    fs::read_to_string(file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["t"].as_f64().unwrap_or(0.0) >= since)
        .collect()
}

/// Drop lines older than two months. Called once at start.
pub fn tidy() {
    let _guard = LOCK.lock().unwrap();
    let Ok(text) = fs::read_to_string(file()) else { return };
    let cutoff = paths::now_secs() - KEEP_SECS;
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| serde_json::from_str::<Value>(l).ok().and_then(|v| v["t"].as_f64()).is_some_and(|t| t >= cutoff))
        .collect();
    if kept.len() < text.lines().count() {
        let _ = fs::write(file(), kept.join("\n") + if kept.is_empty() { "" } else { "\n" });
    }
}
