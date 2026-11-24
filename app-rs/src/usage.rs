//! How much each turn used: tokens from every AI (Claude, Gemini, ChatGPT, Cursor), and Claude Code's own cost estimate.
//!
//! One line per turn is appended to `work/usage.jsonl` ({t, ai, chat, input, output, cache_read, cache_write, cost},
//! plus {session, session_cost} for Claude Code). Claude Code's `total_cost_usd` is the session's running total, so a
//! turn's `cost` is the difference from that session's previous total; its token counts are the turn's own.
//! The window adds them up for "today" in the user's own time zone, so nothing here needs a calendar.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    sync::Mutex,
};

use serde_json::{Value, json};

use crate::paths;

static LOCK: Mutex<()> = Mutex::new(());
/// Each Claude Code session's running cost total as last seen, to turn it into per-turn costs.
static SESSION_COST: Mutex<Option<HashMap<String, f64>>> = Mutex::new(None);
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
    /// This turn's cost: Claude Code's estimate at API prices. The other AIs do not report one.
    pub cost: Option<f64>,
    /// Claude Code only: the session's running total that `cost` was worked out from.
    pub session_cost: Option<f64>,
    /// The AI ran the turn and reported nothing about it (Cursor on its kept connection: measured, no token counts
    /// anywhere over ACP). Logged as a turn with no numbers, so the page says "not reported" instead of zero.
    pub unreported: bool,
}

impl Turn {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    /// Nothing to log: no counts, no cost, and not a turn whose usage went unreported.
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.cost.unwrap_or(0.0) == 0.0 && !self.unreported
    }

    /// A turn that ran and whose usage the AI did not report.
    pub fn not_reported() -> Turn {
        Turn { unreported: true, ..Default::default() }
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
            session_cost: e["total_cost_usd"].as_f64(),
            unreported: false,
        }
    }

    /// From Claude Code's `result` event, with the cost made per-turn from the session's running total.
    pub fn from_claude_session(e: &Value) -> Turn {
        let mut t = Turn::from_claude(e);
        if let (Some(total), Some(session)) = (t.session_cost, e["session_id"].as_str()) {
            t.cost = Some(session_cost_step(session, total));
        }
        t
    }

    /// From Antigravity's `result.usage`.
    pub fn from_antigravity(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Turn { input: n("input_tokens"), output: n("output_tokens") + n("thinking_tokens"), cache_read: n("cache_read_tokens"), ..Default::default() }
    }

    /// From the answer to an ACP prompt as agy gives it (`usage`: measured, `tests/fixtures/gemini/acp-*.wire`).
    pub fn from_agy_acp(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Turn { input: n("inputTokens"), output: n("outputTokens") + n("thoughtTokens"), cache_read: n("cachedReadTokens"), cache_write: n("cachedWriteTokens"), ..Default::default() }
    }

    /// From Codex's `turn.completed.usage`, whose input count includes the cached part.
    pub fn from_codex(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let cached = n("cached_input_tokens");
        Turn { input: n("input_tokens").saturating_sub(cached), output: n("output_tokens") + n("reasoning_output_tokens"), cache_read: cached, ..Default::default() }
    }

    /// From Cursor's `result.usage`. Its input count leaves out the cached part (as Claude Code's does), and
    /// `reasoningTokens` is taken to be part of `outputTokens` (OpenAI's way), so it is not added again.
    pub fn from_cursor(u: &Value) -> Turn {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Turn { input: n("inputTokens"), output: n("outputTokens"), cache_read: n("cacheReadTokens"), cache_write: n("cacheWriteTokens"), ..Default::default() }
    }

    pub fn json(&self) -> Value {
        let mut v = json!({"input": self.input, "output": self.output, "cache_read": self.cache_read, "cache_write": self.cache_write,
                           "total": self.total(), "cost": self.cost});
        if self.unreported {
            v["unreported"] = json!(true);
        }
        v
    }
}

/// A turn's cost from a session's running total and the previous total seen for it (None: none seen). A total lower
/// than before means a new session under the same id, so it counts in full.
pub fn cost_delta(previous: Option<f64>, total: f64) -> f64 {
    match previous {
        Some(p) if total >= p => total - p,
        _ => total,
    }
}

/// The last running total logged for a session (after an app restart the memory is empty).
fn logged_session_cost(session: &str) -> Option<f64> {
    fs::read_to_string(file())
        .unwrap_or_default()
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["session"].as_str() == Some(session))
        .and_then(|v| v["session_cost"].as_f64())
}

/// Remember a session's new running total and return this turn's part of it.
fn session_cost_step(session: &str, total: f64) -> f64 {
    let mut map = SESSION_COST.lock().unwrap();
    let map = map.get_or_insert_with(HashMap::new);
    let previous = match map.get(session) {
        Some(p) => Some(*p),
        None => {
            let _guard = LOCK.lock().unwrap();
            logged_session_cost(session)
        }
    };
    map.insert(session.to_string(), total);
    cost_delta(previous, total)
}

fn file() -> std::path::PathBuf {
    paths::work("usage.jsonl")
}

/// One line of the log: the turn's counts, when, which AI and chat, the game the chat belongs to (Review adds up a
/// game's cost from it), and the Claude Code session with its running total when there is one.
fn line(ai: &str, account: (u8, Option<&str>), chat: &str, project: Option<&str>, turn: &Turn, session: Option<&str>, t: f64) -> Value {
    let mut line = turn.json();
    line["t"] = json!(t);
    line["ai"] = json!(ai);
    // which of the AI's accounts paid, when it was not the user's own (`accounts`)
    // (its place in Setup's list at the time, and its folder's name: the place of an account changes when one before
    // it is removed, the name never does, so a sum per account goes by the name)
    if let (place, Some(name)) = account {
        line["account"] = json!(place);
        line["account_name"] = json!(name);
    }
    line["chat"] = json!(chat);
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        line["project"] = json!(p);
    }
    if let (Some(s), Some(total)) = (session, turn.session_cost) {
        line["session"] = json!(s);
        line["session_cost"] = json!(total);
    }
    line
}

/// Add a turn to the log.
/// With the Claude Code session it belongs to, when there is one.
pub fn record(ai: &str, account: (u8, Option<&str>), chat: &str, project: Option<&str>, turn: &Turn, session: Option<&str>) {
    if turn.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap();
    let line = line(ai, account, chat, project, turn, session, paths::now_secs());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_totals_become_per_turn_costs() {
        // a session's results report 0.50, 1.20, 1.25 in total: the turns cost 0.50, 0.70, 0.05
        let totals = [0.50, 1.20, 1.25];
        let mut prev = None;
        let turns: Vec<f64> = totals.iter().map(|&t| { let c = cost_delta(prev, t); prev = Some(t); c }).collect();
        let sum: f64 = turns.iter().sum();
        assert!((sum - 1.25).abs() < 1e-9, "the turns add up to the session's total, not {sum}");
        assert!((turns[1] - 0.70).abs() < 1e-9);
        // a lower total is a new session: counted in full
        assert!((cost_delta(Some(1.25), 0.10) - 0.10).abs() < 1e-9);
    }

    #[test]
    fn sessions_are_kept_apart() {
        let a = format!("test-a-{}", std::process::id());
        let b = format!("test-b-{}", std::process::id());
        assert!((session_cost_step(&a, 0.4) - 0.4).abs() < 1e-9);
        assert!((session_cost_step(&b, 1.0) - 1.0).abs() < 1e-9);
        assert!((session_cost_step(&a, 0.9) - 0.5).abs() < 1e-9);
        assert!((session_cost_step(&b, 1.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_line_names_the_game_of_the_chat() {
        let turn = Turn { input: 10, output: 5, cost: Some(0.25), ..Default::default() };
        let l = line("claude", (1, None), "c1-1", Some("/games/Odd One In é"), &turn, None, 100.0);
        assert!(l.get("account").is_none(), "the user's own account is not noted");
        let added = line("claude", (3, Some("claude-2-abcdef")), "c1-1", None, &turn, None, 100.0);
        assert_eq!((added["account"].clone(), added["account_name"].clone()), (json!(3), json!("claude-2-abcdef")), "a turn an added account paid for says which: its place then, and its name for good");
        assert_eq!(l["project"], "/games/Odd One In é");
        assert_eq!((l["chat"].as_str(), l["ai"].as_str(), l["t"].as_f64()), (Some("c1-1"), Some("claude"), Some(100.0)));
        assert_eq!(l["cost"], 0.25);
        assert!(l.get("session").is_none(), "no session without a running total");
        // a chat with no game: no project key at all, so older and newer lines read the same way
        assert!(line("gemini", (1, None), "c2", None, &turn, None, 1.0).get("project").is_none());
        assert!(line("gemini", (1, None), "c2", Some(""), &turn, None, 1.0).get("project").is_none());
    }

    #[test]
    fn cursor_turns_are_logged_under_cursor() {
        // a real `result.usage` from cursor-agent 2026.11.22
        let u = json!({"inputTokens": 13200, "outputTokens": 428, "cacheReadTokens": 51840, "cacheWriteTokens": 0, "reasoningTokens": 199});
        let t = Turn::from_cursor(&u);
        assert_eq!((t.input, t.output, t.cache_read, t.cache_write, t.cost), (13200, 428, 51840, 0, None));
        let l = line("cursor", (1, None), "c3", Some("/games/g"), &t, None, 5.0);
        assert_eq!((l["ai"].as_str(), l["total"].as_u64()), (Some("cursor"), Some(13200 + 428 + 51840)));
        assert!(l.get("session").is_none(), "Cursor reports no cost, so no running total");
        assert!(Turn::from_cursor(&json!({})).is_empty());
    }

    #[test]
    fn a_turn_whose_usage_was_not_reported_is_logged_as_such_and_not_as_zero() {
        // Cursor over ACP: the prompt's answer is `{"stopReason": "end_turn"}` and nothing else (recorded)
        let t = Turn::not_reported();
        assert!(!t.is_empty(), "it is logged: the turn did run");
        let l = line("cursor", (1, None), "c4", None, &t, None, 7.0);
        assert_eq!((l["unreported"].as_bool(), l["total"].as_u64(), l["ai"].as_str()), (Some(true), Some(0), Some("cursor")));
        // a turn with numbers says nothing of the kind, and one with nothing at all is still not logged
        assert!(Turn::from_cursor(&json!({"inputTokens": 5})).json().get("unreported").is_none());
        assert!(Turn::default().is_empty());
    }

    #[test]
    fn claude_result_turns_carry_the_turn_cost() {
        let sid = format!("test-c-{}", std::process::id());
        let e = |total: f64| json!({"type": "result", "session_id": sid, "total_cost_usd": total, "usage": {"input_tokens": 10, "output_tokens": 5}});
        let first = Turn::from_claude_session(&e(2.0));
        let second = Turn::from_claude_session(&e(3.5));
        assert_eq!(first.cost, Some(2.0));
        assert_eq!(second.cost, Some(1.5));
        assert_eq!(second.session_cost, Some(3.5));
        assert_eq!((second.input, second.output), (10, 5), "token counts are the turn's own");
    }
}
