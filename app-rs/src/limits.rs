//! The plan's usage limits, for each account of each AI that reports them, kept in `<work>/limits.json` and shown
//! in the usage popover. Two sources feed it (as in T3 Code, which this follows):
//!
//! - **A probe**, in the background (`watch`): at start and every five minutes (30 seconds after a probe failed),
//!   each Claude account is asked by a short-lived Claude Code process started on that account's config folder,
//!   which is sent one control request and never a message, so no model is called and no token is spent:
//!
//!   ```text
//!   claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence
//!          --strict-mcp-config --mcp-config {"mcpServers":{}} --settings {"disableAllHooks":true}
//!   stdin:  {"type":"control_request","request_id":"fridae_usage","request":{"subtype":"get_usage","skip_behaviors":true}}
//!   stdout: {"type":"control_response","response":{"subtype":"success","request_id":"fridae_usage","response":
//!            {"subscription_type":"max","rate_limits_available":true,"rate_limits":{"five_hour":null,"seven_day":
//!            {"utilization":9,"resets_at":"2026-12-02T18:59:59.957041+00:00",...},"model_scoped":[{"display_name":
//!            "Fable","utilization":0,"resets_at":"..."}],"spend":{...},"extra_usage":{...},...},...}}}
//!   ```
//!
//!   (Measured 2026-11-25 on Claude Code 2.1.326: the subtype is the one the Agent SDK's
//!   `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET` sends, read in the program's text; `total_cost_usd`
//!   0 in the answer; it answers even when stdin closes right after the request. Percentages are 0-100 here. Signed
//!   out, or on a key: `rate_limits_available: false`, and the account is left out.) The user's ChatGPT (Codex)
//!   account is asked by `codex app-server` (JSON-RPC: `initialize`, `initialized`, `account/rateLimits/read` →
//!   `{rateLimits: {primary, secondary: {usedPercent, windowDurationMins, resetsAt}, limitId, planType}}`; signed
//!   out it answers the error "codex account authentication required to read rate limits", and is left out). Codex
//!   must keep its stdin open until it has answered (measured, codex-cli 0.169.1).
//!   Each Gemini account is asked by `agy -p /usage --output-format stream-json`, run as a turn of that account runs
//!   (its home, `accounts::home`): agy answers the slash command itself, with no turn (`num_turns` 0, no tokens;
//!   measured 2026-11-26), in a first line `{"event":"command_result","command":{"name":"usage","data":{"groups":
//!   [{"name":"Gemini Models","buckets":[{"window":"weekly"|"5h","remaining_fraction":0.98,"reset_time":
//!   "2026-12-03T07:17:49Z"},...]},{"name":"Claude and GPT models",...}]}}}` (`tests/fixtures/gemini/usage.json`).
//! - **Each Claude turn's `rate_limit_event`** (`record`), on the account the turn ran on, between probes.
//!
//! Every account is keyed by (ai, account): "own" for the user's own sign-in, an added account's folder name
//! (`accounts`) otherwise. A failed probe keeps the last numbers and says why (`error`).
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};

use crate::paths;

static LOCK: Mutex<()> = Mutex::new(());

/// The user's own sign-in, as an account's id.
pub const OWN: &str = "own";
/// How often every account is asked.
pub const EVERY: Duration = Duration::from_secs(300);
/// How soon every account is asked again after a probe that failed.
pub const RETRY: Duration = Duration::from_secs(30);
/// How long a probe may take.
const PROBE_WAIT: Duration = Duration::from_secs(45);

const HOUR: f64 = 3600.0;
const SESSION: f64 = 5.0 * HOUR;
const WEEK: f64 = 7.0 * 24.0 * HOUR;
const MONTH: f64 = 30.0 * 24.0 * HOUR;

fn file() -> std::path::PathBuf {
    paths::work("limits.json")
}

// ------------------------------------------------------------------ windows

/// One window as the page gets it.
fn window(id: &str, label: &str, used: f64, resets: f64, length: f64) -> Value {
    json!({"id": id, "label": label, "used": used.clamp(0.0, 1.0), "resets": if resets.is_finite() && resets > 0.0 { resets.round() } else { 0.0 }, "length": length})
}

/// The windows Claude Code names, with their plain names and lengths.
fn claude_named(id: &str) -> Option<(&'static str, f64)> {
    Some(match id {
        "five_hour" => ("Session", SESSION),
        "seven_day" => ("Weekly", WEEK),
        "seven_day_opus" => ("Weekly · Opus", WEEK),
        "seven_day_sonnet" => ("Weekly · Sonnet", WEEK),
        _ => return None,
    })
}

/// A model's own weekly window: "seven_day_<model>", "Weekly · <model>".
fn scoped_id(name: &str) -> String {
    let token: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("seven_day_{}", token.trim_matches('_'))
}

/// A time as Claude Code writes it ("2026-12-02T18:59:59.957041+00:00", or with "Z"), in seconds since 1970.
pub fn iso_secs(s: &str) -> Option<f64> {
    let s = s.trim();
    let b = s.as_bytes();
    if !s.is_ascii() || b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut frac = 0.0;
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.chars().take_while(char::is_ascii_digit).count();
        frac = format!("0.{}", &r[..digits]).parse().unwrap_or(0.0);
        rest = &r[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) && &o[3..4] == ":" => {
            let m = o[1..3].parse::<i64>().ok()? * 60 + o[4..6].parse::<i64>().ok()?;
            if o.starts_with('-') { -m * 60 } else { m * 60 }
        }
        _ => return None,
    };
    // days from 1970-01-01 (Howard Hinnant's days_from_civil)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset) as f64 + frac)
}

/// A share of a whole: `used` of `limit`, as 0..1.
fn share(used: &Value, limit: &Value) -> Option<f64> {
    let (u, l) = (used.as_f64()?, limit.as_f64()?);
    (u.is_finite() && l.is_finite() && u >= 0.0 && l > 0.0).then(|| (u / l).min(1.0))
}

/// The monthly spend of extra usage, as 0..1, when the account has it turned on.
fn spend_share(r: &Value) -> Option<f64> {
    let s = &r["spend"];
    if s["enabled"] == true
        && s["used"]["currency"].is_string()
        && s["used"]["currency"] == s["limit"]["currency"]
        && s["used"]["exponent"] == s["limit"]["exponent"]
        && let Some(p) = share(&s["used"]["amount_minor"], &s["limit"]["amount_minor"])
    {
        return Some(p);
    }
    let e = &r["extra_usage"];
    if e["is_enabled"] != true {
        return None;
    }
    share(&e["used_credits"], &e["monthly_limit"]).or_else(|| e["utilization"].as_f64().filter(|u| u.is_finite() && *u >= 0.0).map(|u| (u / 100.0).min(1.0)))
}

/// The windows in Claude Code's answer to `get_usage` (the `response` of its `control_response`). None when plan
/// limits do not apply to the account (signed out, or a key).
pub fn claude_usage_windows(resp: &Value) -> Option<Vec<Value>> {
    let r = &resp["rate_limits"];
    if resp["rate_limits_available"] != true || !r.is_object() {
        return None;
    }
    let mut out: Vec<Value> = Vec::new();
    for id in ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"] {
        let w = &r[id];
        let Some(used) = w["utilization"].as_f64().filter(|u| u.is_finite()) else { continue };
        let (label, length) = claude_named(id).unwrap_or_default();
        out.push(window(id, label, used / 100.0, w["resets_at"].as_str().and_then(iso_secs).unwrap_or(0.0), length));
    }
    for m in r["model_scoped"].as_array().into_iter().flatten() {
        let (Some(name), Some(used)) = (m["display_name"].as_str().map(str::trim).filter(|n| !n.is_empty()), m["utilization"].as_f64().filter(|u| u.is_finite())) else { continue };
        let id = scoped_id(name);
        if out.iter().any(|w| w["id"] == id.as_str()) {
            continue;
        }
        let mut w = window(&id, &format!("Weekly · {name}"), used / 100.0, m["resets_at"].as_str().and_then(iso_secs).unwrap_or(0.0), WEEK);
        w["scoped"] = json!(true);
        out.push(w);
    }
    if let Some(p) = spend_share(r) {
        out.push(window("monthly_spend", "Monthly spend", p, 0.0, MONTH));
    }
    Some(out)
}

/// The windows in one `rate_limit_event` (utilization 0..1), named as the probe names them. `known`: the account's
/// windows so far, for the one window an event names only as "seven_day_overage_included" (the model's own weekly
/// window, which only the probe names). Read from `unifiedWindows`, and from the event's own top-level window.
pub fn event_windows(event: &Value, known: &[Value]) -> Vec<Value> {
    let info = &event["rate_limit_info"];
    let mut found: Vec<(String, f64, f64)> = Vec::new();
    for (key, w) in info["unifiedWindows"].as_object().into_iter().flatten() {
        if let Some(u) = w["utilization"].as_f64() {
            found.push((key.clone(), u, w["resetsAt"].as_f64().unwrap_or(0.0)));
        }
    }
    if let (Some(key), Some(u)) = (info["rateLimitType"].as_str(), info["utilization"].as_f64())
        && !found.iter().any(|(k, ..)| k == key)
    {
        found.push((key.to_string(), u, info["resetsAt"].as_f64().unwrap_or(0.0)));
    }
    let mut out = Vec::new();
    for (key, used, resets) in found {
        if !used.is_finite() || used < 0.0 {
            continue;
        }
        if let Some((label, length)) = claude_named(&key) {
            out.push(window(&key, label, used, resets, length));
        } else if key == "seven_day_overage_included"
            && let Some(k) = known.iter().find(|w| w["scoped"] == true)
        {
            let mut w = window(k["id"].as_str().unwrap_or_default(), k["label"].as_str().unwrap_or_default(), used, resets, WEEK);
            w["scoped"] = json!(true);
            out.push(w);
        }
    }
    out
}

/// The windows in Codex's answer to `account/rateLimits/read`. `primary` is the 5-hour session (monthly on the
/// free plans), `secondary` the week; only the "codex" limit is the plan's.
pub fn codex_windows(result: &Value) -> Vec<Value> {
    let main = &result["rateLimits"];
    let snap = match main["limitId"].as_str() {
        Some(id) if id != "codex" => &result["rateLimitsByLimitId"]["codex"],
        _ => main,
    };
    let monthly_plan = matches!(snap["planType"].as_str(), Some("free" | "go"));
    let mut out: Vec<Value> = Vec::new();
    for (pos, fallback) in [("primary", if monthly_plan { MONTH } else { SESSION }), ("secondary", WEEK)] {
        let w = &snap[pos];
        let Some(used) = w["usedPercent"].as_f64().filter(|u| u.is_finite()) else { continue };
        let length = w["windowDurationMins"].as_f64().filter(|m| *m > 0.0).map(|m| m * 60.0).unwrap_or(fallback);
        let (id, label) = if length >= MONTH - 2.0 * 86_400.0 {
            ("monthly", "Monthly")
        } else if length >= WEEK - 86_400.0 {
            ("seven_day", "Weekly")
        } else if (length - SESSION).abs() < 1.0 {
            ("five_hour", "Session")
        } else {
            ("session", "Session")
        };
        if out.iter().any(|o| o["id"] == id) {
            continue;
        }
        out.push(window(id, label, used / 100.0, w["resetsAt"].as_f64().unwrap_or(0.0), length));
    }
    out
}

/// The windows in agy's answer to `/usage` (its `command_result` line). Each group of models has a 5-hour and a
/// weekly bucket, given as the share that is left; the first group ("Gemini Models") is the plain "Session" and
/// "Weekly", another is named after its models ("Weekly · Claude and GPT"). A group nothing has been used of (every
/// bucket whole, with no reset time) is left out.
pub fn gemini_windows(line: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for g in line["command"]["data"]["groups"].as_array().into_iter().flatten() {
        let buckets: Vec<&Value> = g["buckets"].as_array().into_iter().flatten().collect();
        let resets = |b: &Value| b["reset_time"].as_str().and_then(iso_secs).filter(|t| *t > 0.0).unwrap_or(0.0);
        if buckets.iter().all(|b| b["remaining_fraction"].as_f64().is_none_or(|r| r >= 1.0) && resets(b) == 0.0) {
            continue;
        }
        let name = g["name"].as_str().unwrap_or("").trim();
        let gemini = name.eq_ignore_ascii_case("gemini models");
        let short = name.strip_suffix(" models").or_else(|| name.strip_suffix(" Models")).unwrap_or(name).trim();
        for b in buckets {
            let Some(left) = b["remaining_fraction"].as_f64().filter(|r| r.is_finite()) else { continue };
            let (id, label, length) = match b["window"].as_str() {
                Some("5h") => ("five_hour", "Session", SESSION),
                Some("weekly") => ("seven_day", "Weekly", WEEK),
                _ => continue,
            };
            let (id, label) = if gemini { (id.to_string(), label.to_string()) } else { (format!("{id}_{}", scoped_id(short).trim_start_matches("seven_day_")), format!("{label} · {short}")) };
            if out.iter().any(|o| o["id"] == id.as_str()) {
                continue;
            }
            out.push(window(&id, &label, 1.0 - left, resets(b), length));
        }
    }
    out
}

// ------------------------------------------------------------------ the store

/// What one source said about one account.
#[derive(Debug, Clone)]
pub enum Said {
    /// A probe answered: these are the account's windows now.
    Probed(Vec<Value>),
    /// The account has no plan limits (signed out, or a key): it is left out.
    None,
    /// The probe failed, in a plain sentence: the last numbers stay. Empty: it was stopped (the app is quitting).
    Failed(String),
    /// A turn's `rate_limit_event`: its windows are written over the account's.
    Event(Value),
}

fn key(ai: &str, account: &str) -> String {
    format!("{ai}:{account}")
}

/// `store` (the file's `accounts`) with what was said about (ai, account) at `now`.
pub fn apply(store: &mut Map<String, Value>, ai: &str, account: &str, said: Said, now: f64) {
    let k = key(ai, account);
    let entry = || json!({"ai": ai, "account": account, "checked": 0, "windows": []});
    match said {
        Said::None => {
            store.remove(&k);
        }
        Said::Probed(windows) => {
            store.insert(k, json!({"ai": ai, "account": account, "checked": now.round(), "windows": windows}));
        }
        Said::Failed(why) => {
            store.entry(k).or_insert_with(entry)["error"] = json!(why);
        }
        Said::Event(event) => {
            let known: Vec<Value> = store.get(&k).and_then(|e| e["windows"].as_array().cloned()).unwrap_or_default();
            let new = event_windows(&event, &known);
            if new.is_empty() {
                return;
            }
            let mut all = known;
            for w in new {
                match all.iter_mut().find(|o| o["id"] == w["id"]) {
                    Some(o) => *o = w,
                    None => all.push(w),
                }
            }
            let e = store.entry(k).or_insert_with(entry);
            e["windows"] = json!(all);
            e["checked"] = json!(now.round());
        }
    }
}

fn read() -> Map<String, Value> {
    // (a file in the old shape, one set of windows for every account, is not read: the first probe fills it again)
    std::fs::read(file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["accounts"].as_object().cloned()).unwrap_or_default()
}

fn write(store: &Map<String, Value>) {
    if let Some(dir) = file().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file(), serde_json::to_vec(&json!({"accounts": store})).unwrap_or_default());
}

/// What was said about one account, kept.
pub fn note(ai: &str, account: &str, said: Said) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = read();
    apply(&mut store, ai, account, said, paths::now_secs());
    write(&store);
}

/// A Claude turn reported its limits: kept for the account it ran on (`account`: an added account's folder name,
/// None for the user's own).
pub fn record(event: &Value, account: Option<&str>) {
    let info = &event["rate_limit_info"];
    if info["unifiedWindows"].as_object().is_none_or(|m| m.is_empty()) && info["utilization"].as_f64().is_none() {
        return;
    }
    note("claude", account.unwrap_or(OWN), Said::Event(event.clone()));
}

/// An account as the page names it, and where it sorts; None for an added account that is no longer there.
fn place(ai: &str, account: &str) -> Option<(String, u8)> {
    let name = match ai {
        "claude" => "Claude",
        "gemini" => "Gemini",
        "codex" => "ChatGPT",
        _ => return None,
    };
    let n = if account == OWN { 1 } else { crate::accounts::position(ai, account)? };
    Some((format!("{name} · Account {n}"), n))
}

/// The page's shape: {"accounts": [{ai, account, label, checked, error?, windows: [{id, label, used, resets,
/// length}]}]}, Claude's first, each AI's in the order of its accounts. A window whose reset time has passed is
/// left out (its numbers no longer hold).
pub fn shape(store: &Map<String, Value>, now: f64, place: &dyn Fn(&str, &str) -> Option<(String, u8)>) -> Value {
    let mut rows: Vec<(u8, u8, Value)> = Vec::new();
    for e in store.values() {
        let (Some(ai), Some(account)) = (e["ai"].as_str(), e["account"].as_str()) else { continue };
        let Some((label, n)) = place(ai, account) else { continue };
        let windows: Vec<Value> = e["windows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| w["resets"].as_f64().is_none_or(|r| r <= 0.0 || r > now))
            .map(|w| json!({"id": w["id"], "label": w["label"], "used": w["used"], "resets": w["resets"], "length": w["length"]}))
            .collect();
        let mut row = json!({"ai": ai, "account": account, "label": label, "checked": e["checked"].as_f64().unwrap_or(0.0), "windows": windows});
        if let Some(err) = e["error"].as_str().filter(|s| !s.is_empty()) {
            row["error"] = json!(err);
        }
        let order = ["claude", "gemini", "codex"].iter().position(|a| *a == ai).unwrap_or(9) as u8;
        rows.push((order, n, row));
    }
    rows.sort_by_key(|(a, n, _)| (*a, *n));
    json!({"accounts": rows.into_iter().map(|(_, _, r)| r).collect::<Vec<_>>()})
}

/// What the page shows (`shape`).
pub fn state() -> Value {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    shape(&read(), paths::now_secs(), &place)
}

// ------------------------------------------------------------------ the probe

/// Runs `cmd` with `input` written to its stdin (kept open), until a line of its output satisfies `wanted`, at most
/// `limit`, or until the app quits (then `Err("")`). Started without a window and tied to the app's life
/// (`paths::end_with_app`), in the ledger while it runs; its stdin closed and the process ended afterwards.
/// With no `input`, its stdin is empty. When it fails, what it wrote to its error output is handed back too.
fn ask(mut cmd: Command, input: &[String], wanted: &dyn Fn(&Value) -> bool, limit: Duration, what: &str) -> Result<Value, (String, String)> {
    cmd.stdin(if input.is_empty() { Stdio::null() } else { Stdio::piped() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::quiet(&mut cmd);
    paths::own_group(&mut cmd);
    let mut child = paths::end_with_app(&mut cmd).map_err(|_| (format!("{what} could not be started."), String::new()))?;
    let pid = child.id();
    crate::procs::record("limits", pid);
    let mut stdin = child.stdin.take();
    if let Some(pipe) = stdin.as_mut() {
        for line in input {
            let _ = pipe.write_all(line.as_bytes()).and_then(|()| pipe.write_all(b"\n"));
        }
        let _ = pipe.flush();
    }
    let err_text = std::sync::Arc::new(Mutex::new(String::new()));
    if let Some(mut err) = child.stderr.take() {
        let keep = err_text.clone();
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::Read::take(&mut err, 64 * 1024), &mut all);
            *keep.lock().unwrap_or_else(|e| e.into_inner()) = String::from_utf8_lossy(&all).to_string();
            let _ = std::io::copy(&mut err, &mut std::io::sink());
        });
    }
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let until = Instant::now() + limit;
    let answer = loop {
        if crate::agents::shutting() {
            break Err(String::new());
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Err(format!("{what} did not answer in time."));
        }
        match rx.recv_timeout(left.min(Duration::from_millis(200))) {
            Ok(line) => {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && wanted(&v)
                {
                    break Ok(v);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(format!("{what} ended without answering.")),
        }
    };
    drop(stdin); // both end by themselves when their input ends
    let closed = Instant::now() + Duration::from_secs(3);
    while Instant::now() < closed && matches!(child.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    if matches!(child.try_wait(), Ok(None)) {
        paths::end_trees(&[pid]);
    }
    let _ = child.wait();
    crate::procs::forget(pid);
    // (its error output, read to its end once the process has ended, or what came of it in a moment)
    let read_by = Instant::now() + Duration::from_millis(500);
    while Instant::now() < read_by && std::sync::Arc::strong_count(&err_text) > 1 {
        std::thread::sleep(Duration::from_millis(20));
    }
    let err = err_text.lock().unwrap_or_else(|e| e.into_inner()).clone();
    answer.map_err(|why| (why, err))
}

/// The control request that asks Claude Code for the plan's usage (see the top of this file).
pub fn usage_request() -> String {
    json!({"type": "control_request", "request_id": "fridae_usage", "request": {"subtype": "get_usage", "skip_behaviors": true}}).to_string()
}

/// One Claude account asked for its limits (`account`: an added account's folder name, None for the user's own).
/// None when it cannot be asked at all (Claude Code is not installed, or the added account's folder is not there).
pub fn probe_claude(account: Option<&str>) -> Option<Said> {
    let exe = paths::find_agent()?;
    let mut cmd = Command::new(exe);
    cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence", "--strict-mcp-config"]);
    cmd.args(["--mcp-config", r#"{"mcpServers":{}}"#, "--settings", r#"{"disableAllHooks":true}"#]);
    let dir = paths::work("probe");
    let _ = std::fs::create_dir_all(&dir);
    cmd.current_dir(dir);
    paths::child_env(&mut cmd, "claude");
    if let Some(h) = crate::agents::claude_home() {
        cmd.env("HOME", h);
    }
    if !crate::accounts::env_of(&mut cmd, "claude", account) {
        return None;
    }
    let wanted = |v: &Value| v["type"] == "control_response" && v["response"]["request_id"] == "fridae_usage";
    Some(match ask(cmd, &[usage_request()], &wanted, PROBE_WAIT, "Claude Code") {
        Ok(v) if v["response"]["subtype"] == "success" => claude_usage_windows(&v["response"]["response"]).map_or(Said::None, Said::Probed),
        Ok(_) => Said::Failed("Claude Code could not read the usage limits.".into()),
        Err((why, _)) => Said::Failed(why),
    })
}

/// The user's ChatGPT account (Codex) asked for its limits. None when Codex is not installed.
pub fn probe_codex() -> Option<Said> {
    let exe = paths::find_codex()?;
    let mut cmd = Command::new(exe);
    cmd.arg("app-server");
    let dir = paths::work("probe");
    let _ = std::fs::create_dir_all(&dir);
    cmd.current_dir(dir);
    paths::child_env(&mut cmd, "chatgpt");
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "fridae_studios", "title": "Fridae", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"experimentalApi": false}}}).to_string(),
        json!({"jsonrpc": "2.0", "method": "initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read"}).to_string(),
    ];
    let wanted = |v: &Value| v["id"] == 2;
    Some(match ask(cmd, &input, &wanted, PROBE_WAIT, "Codex") {
        Ok(v) if v["result"].is_object() => Said::Probed(codex_windows(&v["result"])),
        // signed out, or on a key: no plan limits
        Ok(v) if v["error"]["message"].as_str().is_some_and(|m| m.to_lowercase().contains("auth")) => Said::None,
        Ok(_) => Said::Failed("Codex could not read the usage limits.".into()),
        Err((why, _)) => Said::Failed(why),
    })
}

/// One Gemini account asked for its limits, with the environment a turn of that account gets (`account`: an added
/// account's folder name, None for the user's own). None when agy is not installed or the added account's folder
/// is not there.
pub fn probe_gemini(account: Option<&str>) -> Option<Said> {
    let exe = paths::find_antigravity()?;
    let mut cmd = Command::new(exe);
    cmd.args(["-p", "/usage", "--output-format", "stream-json"]);
    let dir = paths::work("probe");
    let _ = std::fs::create_dir_all(&dir);
    cmd.current_dir(dir);
    paths::child_env(&mut cmd, "gemini");
    if account.is_some() {
        if !crate::accounts::env_of(&mut cmd, "gemini", account) {
            return None;
        }
    } else if let Some(h) = crate::antigravity::home_override() {
        cmd.env("HOME", h);
    }
    let home = cmd.get_envs().find(|(k, _)| *k == if cfg!(windows) { "USERPROFILE" } else { "HOME" }).and_then(|(_, v)| v.map(std::path::PathBuf::from)).unwrap_or_else(paths::home_dir);
    let began = std::time::SystemTime::now();
    let wanted = |v: &Value| v["event"] == "command_result" || v["event"] == "result";
    let answer = ask(cmd, &[], &wanted, Duration::from_secs(30), "Gemini");
    if matches!(&answer, Ok(v) if v["event"] == "command_result") {
        return answer.ok().map(|v| Said::Probed(gemini_windows(&v)));
    }
    if matches!(&answer, Err((why, _)) if why.is_empty()) {
        return Some(Said::Failed(String::new())); // the app is quitting
    }
    // Google's "Verify your account to continue" reaches only agy's log (`antigravity::log_asks_to_verify`)
    if crate::antigravity::log_asks_to_verify(&home, began) {
        return Some(Said::Failed(VERIFY.into()));
    }
    Some(match answer {
        Err((why, err)) => gemini_failed(&err).unwrap_or(Said::Failed(why)),
        Ok(_) => Said::Failed("Gemini could not read the usage limits.".into()),
    })
}

/// What a Gemini account is told when Google holds it back.
const VERIFY: &str = "Google wants this Gemini account verified before it can be used.";

/// agy's error output when `/usage` did not answer: signed out (the account is left out), or held back by Google.
fn gemini_failed(err: &str) -> Option<Said> {
    if crate::signin::asks_to_verify(err) {
        return Some(Said::Failed(VERIFY.into()));
    }
    let low = err.to_lowercase();
    (low.contains("please sign in") || low.contains("not authenticated") || low.contains("authentication required") || low.contains("login required")).then_some(Said::None)
}

/// Every account asked once; true when every probe that ran answered.
pub fn probe_all() -> bool {
    let mut ok = true;
    let mut keep = |ai: &str, account: &str, said: Option<Said>| {
        let Some(said) = said else { return };
        if crate::agents::shutting() {
            return;
        }
        if let Said::Failed(why) = &said {
            if why.is_empty() {
                return;
            }
            ok = false;
        }
        note(ai, account, said);
    };
    keep("claude", OWN, probe_claude(None));
    for name in crate::accounts::list("claude") {
        if crate::agents::shutting() {
            return false;
        }
        if crate::accounts::signed_out_of("claude", &name) {
            note("claude", &name, Said::None);
            continue;
        }
        keep("claude", &name, probe_claude(Some(&name)));
    }
    if !crate::agents::shutting() {
        keep("gemini", OWN, probe_gemini(None));
    }
    for name in crate::accounts::list("gemini") {
        if crate::agents::shutting() {
            return false;
        }
        if crate::accounts::signed_out_of("gemini", &name) {
            note("gemini", &name, Said::None);
            continue;
        }
        keep("gemini", &name, probe_gemini(Some(&name)));
    }
    if !crate::agents::shutting() {
        keep("codex", OWN, probe_codex());
    }
    ok
}

/// The background probe: at start, then every `EVERY` (`RETRY` after a failure), until the app quits.
pub fn watch() {
    while !crate::agents::shutting() {
        let wait = if probe_all() { EVERY } else { RETRY };
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if crate::agents::shutting() {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_answer() -> Value {
        json!({"session": {"total_cost_usd": 0}, "subscription_type": "max", "rate_limits_available": true, "rate_limits": {
            "five_hour": null,
            "seven_day": {"utilization": 9, "resets_at": "2026-12-02T18:59:59.957041+00:00", "limit_dollars": null},
            "seven_day_opus": {"utilization": 130, "resets_at": "2026-12-02T18:59:59Z"},
            "seven_day_oauth_apps": null,
            "model_scoped": [{"display_name": "Fable", "utilization": 0, "resets_at": "2026-12-02T18:59:59.957187+00:00"}, {"display_name": "x", "utilization": null, "resets_at": null}],
            "spend": {"used": {"amount_minor": 0, "currency": "USD", "exponent": 2}, "limit": null, "enabled": false},
            "extra_usage": {"is_enabled": false}
        }})
    }

    #[test]
    fn times_are_read_as_claude_code_writes_them() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_secs("2026-12-02T18:59:59+00:00"), Some(1_796_237_999.0));
        assert_eq!(iso_secs("2026-12-02T20:59:59+02:00"), Some(1_796_237_999.0));
        assert!((iso_secs("2026-12-02T18:59:59.5+00:00").unwrap() - 1_796_237_999.5).abs() < 1e-6);
        for bad in ["", "soon", "2026-13-02T18:59:59Z", "2026-12-02T18:59:59+0000", "2026-12-02T18:59:59 UTC"] {
            assert_eq!(iso_secs(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_probe_answer_is_read_into_windows() {
        let w = claude_usage_windows(&probe_answer()).unwrap();
        let ids: Vec<&str> = w.iter().map(|w| w["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["seven_day", "seven_day_opus", "seven_day_fable"], "an empty window and one with no share are left out");
        assert_eq!(w[0], json!({"id": "seven_day", "label": "Weekly", "used": 0.09, "resets": 1_796_238_000.0, "length": 604_800.0}));
        assert_eq!((w[1]["label"].as_str(), w[1]["used"].as_f64()), (Some("Weekly · Opus"), Some(1.0)), "a share above 100% is capped");
        assert_eq!((w[2]["label"].as_str(), w[2]["used"].as_f64(), w[2]["length"].as_f64()), (Some("Weekly · Fable"), Some(0.0), Some(WEEK)));
        // the session window, and spend once it is turned on
        let mut a = probe_answer();
        a["rate_limits"]["five_hour"] = json!({"utilization": 50, "resets_at": "2026-11-25T20:00:00Z"});
        a["rate_limits"]["spend"] = json!({"enabled": true, "used": {"amount_minor": 250, "currency": "USD", "exponent": 2}, "limit": {"amount_minor": 1000, "currency": "USD", "exponent": 2}});
        let w = claude_usage_windows(&a).unwrap();
        assert_eq!(w[0], json!({"id": "five_hour", "label": "Session", "used": 0.5, "resets": 1_795_636_800.0, "length": 18_000.0}));
        assert_eq!((w.last().unwrap()["id"].as_str(), w.last().unwrap()["used"].as_f64()), (Some("monthly_spend"), Some(0.25)));
        // signed out, or a key: no limits
        assert_eq!(claude_usage_windows(&json!({"subscription_type": null, "rate_limits_available": false, "rate_limits": null})), None);
    }

    #[test]
    fn codex_answer_is_read_into_windows() {
        let r = json!({"rateLimits": {"limitId": "codex", "planType": "plus", "primary": {"usedPercent": 12.5, "windowDurationMins": 300, "resetsAt": 1_796_000_000}, "secondary": {"usedPercent": 40, "windowDurationMins": 10080, "resetsAt": 1_796_500_000}}});
        let w = codex_windows(&r);
        assert_eq!(w[0], json!({"id": "five_hour", "label": "Session", "used": 0.125, "resets": 1_796_000_000.0, "length": 18_000.0}));
        assert_eq!(w[1], json!({"id": "seven_day", "label": "Weekly", "used": 0.4, "resets": 1_796_500_000.0, "length": 604_800.0}));
        // a free plan's primary window is the month; another limit's snapshot defers to the codex one
        let free = json!({"rateLimits": {"limitId": "other", "primary": {"usedPercent": 1}}, "rateLimitsByLimitId": {"codex": {"planType": "free", "primary": {"usedPercent": 30}}}});
        assert_eq!(codex_windows(&free), [json!({"id": "monthly", "label": "Monthly", "used": 0.3, "resets": 0.0, "length": MONTH})]);
        assert!(codex_windows(&json!({})).is_empty());
    }

    fn event(key: &str, used: f64, resets: f64) -> Value {
        json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "rateLimitType": key, "resetsAt": resets, "unifiedWindows": {key: {"utilization": used, "resetsAt": resets, "status": "allowed"}}}})
    }

    #[test]
    fn accounts_are_kept_apart_and_an_event_lands_on_its_own_account() {
        let mut s = Map::new();
        apply(&mut s, "claude", OWN, Said::Probed(claude_usage_windows(&probe_answer()).unwrap()), 1000.0);
        apply(&mut s, "claude", "claude-2-abcd", Said::Probed(vec![window("seven_day", "Weekly", 0.7, 3000.0, WEEK)]), 1000.0);
        apply(&mut s, "codex", OWN, Said::Probed(vec![window("five_hour", "Session", 0.2, 3000.0, SESSION)]), 1000.0);
        assert_eq!(s.len(), 3);
        // a turn on the added account changes that account alone
        apply(&mut s, "claude", "claude-2-abcd", Said::Event(event("seven_day", 0.75, 3000.0)), 1100.0);
        apply(&mut s, "claude", "claude-2-abcd", Said::Event(event("five_hour", 0.1, 2000.0)), 1200.0);
        let added = &s["claude:claude-2-abcd"];
        assert_eq!(added["checked"], 1200.0);
        assert_eq!(added["windows"].as_array().unwrap().len(), 2);
        assert_eq!(added["windows"][0]["used"], 0.75);
        assert_eq!(s["claude:own"]["windows"][0]["used"], 0.09, "the user's own account is untouched");
        assert_eq!(s["claude:own"]["checked"], 1000.0);
        // the model's own window, which an event names only as "overage included", goes to the probed one
        apply(&mut s, "claude", OWN, Said::Event(event("seven_day_overage_included", 0.33, 5000.0)), 1300.0);
        let fable = s["claude:own"]["windows"].as_array().unwrap().iter().find(|w| w["id"] == "seven_day_fable").unwrap().clone();
        assert_eq!((fable["used"].as_f64(), fable["label"].as_str()), (Some(0.33), Some("Weekly · Fable")));
        // and is dropped on an account with no probe yet
        apply(&mut s, "claude", "claude-2-new", Said::Event(event("seven_day_overage_included", 0.5, 5000.0)), 1300.0);
        assert!(!s.contains_key("claude:claude-2-new"));
        // an event with its window at the top level only is read too
        let top = json!({"rate_limit_info": {"rateLimitType": "five_hour", "utilization": 0.4, "resetsAt": 2500}});
        apply(&mut s, "claude", OWN, Said::Event(top), 1400.0);
        assert!(s["claude:own"]["windows"].as_array().unwrap().iter().any(|w| w["id"] == "five_hour" && w["used"] == 0.4));
        // a failed probe keeps the numbers and says why; a probe that answers clears it; none: the account goes
        apply(&mut s, "codex", OWN, Said::Failed("Codex did not answer in time.".into()), 1500.0);
        assert_eq!((s["codex:own"]["windows"][0]["used"].as_f64(), s["codex:own"]["error"].as_str()), (Some(0.2), Some("Codex did not answer in time.")));
        apply(&mut s, "codex", OWN, Said::Probed(vec![]), 1600.0);
        assert!(s["codex:own"].get("error").is_none());
        apply(&mut s, "codex", OWN, Said::None, 1700.0);
        assert!(!s.contains_key("codex:own"));
    }

    #[test]
    fn the_page_gets_the_agreed_shape() {
        let mut s = Map::new();
        apply(&mut s, "codex", OWN, Said::Probed(vec![window("five_hour", "Session", 0.2, 3000.0, SESSION)]), 1000.0);
        apply(&mut s, "claude", "claude-2-abcd", Said::Probed(vec![window("seven_day", "Weekly", 0.7, 3000.0, WEEK), window("five_hour", "Session", 0.1, 1500.0, SESSION)]), 1000.0);
        apply(&mut s, "claude", OWN, Said::Probed(vec![window("seven_day", "Weekly", 0.09, 0.0, WEEK)]), 1000.0);
        apply(&mut s, "claude", OWN, Said::Failed("Claude Code did not answer in time.".into()), 1100.0);
        apply(&mut s, "claude", "claude-2-gone", Said::Probed(vec![]), 1000.0);
        let place = |ai: &str, account: &str| -> Option<(String, u8)> {
            let name = if ai == "claude" { "Claude" } else { "ChatGPT" };
            let n = match account {
                OWN => 1,
                "claude-2-abcd" => 2,
                _ => return None,
            };
            Some((format!("{name} · Account {n}"), n))
        };
        let v = shape(&s, 2000.0, &place);
        assert_eq!(
            v,
            json!({"accounts": [
                {"ai": "claude", "account": "own", "label": "Claude · Account 1", "checked": 1000.0, "error": "Claude Code did not answer in time.",
                 "windows": [{"id": "seven_day", "label": "Weekly", "used": 0.09, "resets": 0.0, "length": 604_800.0}]},
                {"ai": "claude", "account": "claude-2-abcd", "label": "Claude · Account 2", "checked": 1000.0,
                 "windows": [{"id": "seven_day", "label": "Weekly", "used": 0.7, "resets": 3000.0, "length": 604_800.0}]},
                {"ai": "codex", "account": "own", "label": "ChatGPT · Account 1", "checked": 1000.0,
                 "windows": [{"id": "five_hour", "label": "Session", "used": 0.2, "resets": 3000.0, "length": 18_000.0}]}
            ]}),
            "Claude's first, by account; a window past its reset and a removed account are left out"
        );
    }

    #[test]
    fn a_turns_event_is_recorded_on_its_account() {
        let _g = crate::accounts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::accounts::stand_in_tagged("lim");
        record(&event("seven_day", 0.41, paths::now_secs() + 9999.0), Some("claude-2-lim"));
        record(&json!({"type": "result"}), None);
        let v = state();
        // (other tests' turns may write the user's own account meanwhile: only the added one is looked at)
        let rows: Vec<&Value> = v["accounts"].as_array().unwrap().iter().filter(|r| r["account"] != OWN).collect();
        assert_eq!(rows.len(), 1, "{v}");
        assert_eq!((rows[0]["account"].as_str(), rows[0]["label"].as_str()), (Some("claude-2-lim"), Some("Claude · Account 2")));
        assert_eq!(rows[0]["windows"][0]["used"], 0.41);
    }

    #[test]
    fn the_gemini_answer_is_read_into_windows() {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("gemini").join("usage.json")).unwrap();
        let line: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let w = gemini_windows(&line);
        assert_eq!(w.len(), 2, "the unused Claude and GPT group is left out: {w:?}");
        assert_eq!((w[0]["id"].as_str(), w[0]["label"].as_str(), w[0]["length"].as_f64(), w[0]["resets"].as_f64()), (Some("seven_day"), Some("Weekly"), Some(WEEK), iso_secs("2026-12-03T07:17:49Z")));
        assert!((w[0]["used"].as_f64().unwrap() - (1.0 - 0.983716607093811)).abs() < 1e-9);
        assert_eq!((w[1]["id"].as_str(), w[1]["label"].as_str(), w[1]["length"].as_f64()), (Some("five_hour"), Some("Session"), Some(SESSION)));
        // once the other group is used it shows, named after its models; an epoch reset is no reset time
        let mut used = line.clone();
        used["command"]["data"]["groups"][1]["buckets"][0]["remaining_fraction"] = json!(0.75);
        let w = gemini_windows(&used);
        let other = w.iter().find(|w| w["label"] == "Weekly · Claude and GPT").unwrap();
        assert_eq!((other["id"].as_str(), other["used"].as_f64(), other["resets"].as_f64()), (Some("seven_day_claude_and_gpt"), Some(0.25), Some(0.0)));
        assert!(w.iter().any(|w| w["label"] == "Session · Claude and GPT" && w["used"] == 0.0));
        assert!(gemini_windows(&json!({"event": "result"})).is_empty());
        // agy's words when it cannot answer
        assert!(matches!(gemini_failed("Error: Please sign in to continue"), Some(Said::None)));
        assert!(matches!(gemini_failed("Verify your account to continue."), Some(Said::Failed(w)) if w == VERIFY));
        assert!(gemini_failed("something else").is_none());
        // and Gemini sorts between Claude and ChatGPT on the page
        let mut s = Map::new();
        for ai in ["codex", "gemini", "claude"] {
            apply(&mut s, ai, OWN, Said::Probed(vec![]), 1.0);
        }
        let v = shape(&s, 2.0, &|ai: &str, _: &str| Some((ai.to_string(), 1)));
        let order: Vec<&str> = v["accounts"].as_array().unwrap().iter().map(|r| r["ai"].as_str().unwrap()).collect();
        assert_eq!(order, ["claude", "gemini", "codex"]);
    }

    #[test]
    fn the_request_is_the_one_claude_code_knows() {
        let r: Value = serde_json::from_str(&usage_request()).unwrap();
        assert_eq!(r["request"], json!({"subtype": "get_usage", "skip_behaviors": true}));
    }

    /// The real thing, once, on this computer: `cargo test -- limits::tests::live_probe --ignored --nocapture`.
    /// Sends no message (no tokens); prints the windows only.
    #[test]
    #[ignore]
    fn live_probe() {
        for (who, said) in [("claude", probe_claude(None)), ("gemini", probe_gemini(None)), ("codex", probe_codex())] {
            match said {
                Some(Said::Probed(w)) => {
                    for w in w {
                        println!("{who}: {} used {:.2} resets {} length {}", w["label"].as_str().unwrap_or(""), w["used"].as_f64().unwrap_or(-1.0), w["resets"], w["length"]);
                    }
                }
                other => println!("{who}: {other:?}"),
            }
        }
    }
}
