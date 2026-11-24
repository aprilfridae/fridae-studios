//! A second account for an AI, with rollover at the usage limit.
//! Design: docs/superpowers/specs/2026-11-23-second-accounts-design.md. What was measured: DEVELOPING.md, "A second
//! account: what was measured first".
//!
//! Claude only. Claude Code keeps its sign-in per config folder (`CLAUDE_CONFIG_DIR`; measured), so Claude's second
//! account is a config folder of the app's own, beside Fridae's OpenCode copy in the app's data folder, outside every
//! folder an AI is given. The user's own settings, skills and plugins are linked into it (`prepare`), so a turn there
//! loads what a turn on the first account loads (measured from Claude Code's `init` line); who is signed in, and the
//! conversations, are the folder's own. Gemini has none: agy keeps its sign-in in the keychain, one per HOME, whatever
//! folder it is given (measured), so there is nothing the app could give it a second one in.
//!
//! The first account is the user's own sign-in and is never touched. The second is signed in with the tool's own
//! sign-in (`signin`, the id "claude2") and removed by deleting the app's folder, never by any command of the tool.
//! The app reads, copies and stores no token.
//!
//! Which account pays is decided here and nowhere else:
//! - whether a second account exists is a guarded setting (`SETTING`: the folder's name, never a path), changed only
//!   from the page's Setup;
//! - an account that hit its usage limit is "used up" until its reset time (`mark_used`), in the app's memory;
//! - a new chat starts on the first account that is not used up, the user's own first (`pick`); a running chat stays
//!   where it is until its account runs out, then moves to the other one once (`other`), never back and forth.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::paths;

/// The AIs that can have a second account.
pub const AIS: [&str; 1] = ["claude"];

/// The guarded setting: `{"claude": "<folder name>"}`. The name is one the app made (`name_ok`), under `base()`.
pub const SETTING: &str = "second_accounts";

/// Where the second accounts' folders are: the app's own data folder (not Documents, which may be synced, and not
/// any folder an AI is given). Among the places no agent's file tools may write (`paths::app_protected`).
pub fn base() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("accounts");
    #[allow(unreachable_code)]
    crate::opencode::own_store().parent().map(Path::to_path_buf).unwrap_or_default().join("accounts")
}

/// Whether a folder name is one the app makes for an AI's second account: "<ai>-2-" and four to sixteen lower-case
/// letters or digits. Nothing else is ever joined to `base()`, so the setting cannot point anywhere else.
pub fn name_ok(ai: &str, name: &str) -> bool {
    AIS.contains(&ai) && name.strip_prefix(&format!("{ai}-2-")).is_some_and(|rest| (4..=16).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// The setting as the app keeps it: only AIs that can have a second account, only names the app could have made.
pub fn checked_setting(v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    Some(Value::Object(m.iter().filter(|(ai, name)| name.as_str().is_some_and(|n| name_ok(ai, n))).map(|(k, v)| (k.clone(), v.clone())).collect()))
}

fn folder_in(settings: &Map<String, Value>, ai: &str) -> Option<PathBuf> {
    settings.get(SETTING).and_then(|v| v[ai].as_str()).filter(|n| name_ok(ai, n)).map(|n| base().join(n))
}

/// The second account's folder, when the AI has one (from the app's memory of the guarded settings).
pub fn folder(ai: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(second) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.second)) {
        return (second && AIS.contains(&ai)).then(|| base().join(format!("{ai}-2-standin")));
    }
    folder_in(&paths::guarded_settings(), ai)
}

/// Whether the AI's second account can take a chat as far as its sign-in goes: what its tool last said (`signin`).
pub fn second_signed_in(ai: &str) -> bool {
    #[cfg(test)]
    if let Some(yes) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.signed_in)) {
        return yes;
    }
    crate::signin::signed_in(&format!("{ai}2"))
}

/// Tests: one test's own accounts in place of the app's one copy (tests run side by side, each on its own thread):
/// whether there is a second account, whether it is signed in, and which accounts are used up until when.
#[cfg(test)]
#[derive(Default)]
pub struct StandIn {
    pub second: bool,
    pub signed_in: bool,
    pub used: HashMap<(String, u8), f64>,
}

#[cfg(test)]
thread_local! {
    pub static STAND_IN: std::cell::RefCell<Option<StandIn>> = const { std::cell::RefCell::new(None) };
}

/// Tests: this test's accounts from here on (`StandIn`).
#[cfg(test)]
pub fn stand_in(second: bool, signed_in: bool) {
    STAND_IN.with(|s| *s.borrow_mut() = Some(StandIn { second, signed_in, used: HashMap::new() }));
}

/// A new second account: a new folder under a new name (never a name used before: on a Mac the tool's sign-in for a
/// folder may outlive the folder in the keychain, and must not come back with it), noted in the settings.
pub fn add(ai: &str) -> Result<PathBuf, String> {
    if !AIS.contains(&ai) || cfg!(windows) {
        return Err("A second account is not offered for that AI here.".to_string());
    }
    if let Some(have) = folder(ai) {
        return Ok(have);
    }
    let mut bytes = [0u8; 6];
    let random = fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).is_ok();
    if !random {
        return Err("Fridae could not make a name for the second account's folder.".to_string());
    }
    let name = format!("{ai}-2-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    debug_assert!(name_ok(ai, &name));
    paths::update_config(|cfg| {
        let mut all = cfg.get(SETTING).and_then(Value::as_object).cloned().unwrap_or_default();
        all.insert(ai.to_string(), json!(name));
        cfg.insert(SETTING.into(), Value::Object(all));
    });
    prepare(ai).ok_or_else(|| "Fridae could not make the second account's folder.".to_string())
}

/// Remove the second account: its folder is deleted (the app's own, under `base()`, by its checked name) and the
/// setting cleared. Nothing of the tool's is run, and the first account is not touched.
pub fn remove(ai: &str) {
    let dir = folder(ai);
    paths::update_config(|cfg| {
        let mut all = cfg.get(SETTING).and_then(Value::as_object).cloned().unwrap_or_default();
        all.remove(ai);
        cfg.insert(SETTING.into(), Value::Object(all));
    });
    if let Some(dir) = dir.filter(|d| d.parent() == Some(base().as_path())) {
        remove_folder(&dir);
    }
    if let Some(used) = USED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        used.remove(&(ai.to_string(), 2));
    }
}

/// Delete a second account's folder: the links in it first, each as a link (never what it points to), then the rest.
fn remove_folder(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            let _ = fs::remove_file(entry.path());
        }
    }
    let _ = fs::remove_dir_all(dir);
}

/// What of the user's own Claude Code setup a turn on the second account gets as well, by links to it: settings
/// (hooks, permissions, enabled plugins), skills, plugins, agents, commands, memory and rules. Not linked: who is
/// signed in (`.claude.json`, `.credentials.json`), the conversations (`projects`, `sessions`) and other state.
const CLAUDE_LINKED: [&str; 9] = ["settings.json", "skills", "plugins", "agents", "commands", "CLAUDE.md", "rules", "output-styles", "keybindings.json"];

/// The user's own Claude Code config folder: `CLAUDE_CONFIG_DIR` when the app was started with one, else `~/.claude`.
fn users_claude() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| paths::home_dir().join(".claude"))
}

/// The second account's folder, ready for the tool: made (the user's alone), with the user's own setup linked in.
/// Called before every command that runs there, so what the user adds later is linked too, and before the tool's
/// first command there, so the tool never makes a folder of its own where a link belongs. None: no second account.
pub fn prepare(ai: &str) -> Option<PathBuf> {
    let dir = folder(ai)?;
    link_into(&dir, &users_claude(), &CLAUDE_LINKED);
    Some(dir)
}

/// See `prepare`. A name that is already there, as a link or as the tool's own, is left as it is; a link whose target
/// is gone is taken away.
fn link_into(dir: &Path, users: &Path, names: &[&str]) {
    let _ = fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        for name in names {
            let (from, to) = (users.join(name), dir.join(name));
            let is_link = fs::symlink_metadata(&to).is_ok_and(|m| m.file_type().is_symlink());
            if is_link && !to.exists() {
                let _ = fs::remove_file(&to);
            }
            if from.exists() && fs::symlink_metadata(&to).is_err() {
                let _ = std::os::unix::fs::symlink(&from, &to);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (users, names);
}

/// The environment that makes a tool's process run on an account: nothing for the first (the user's own), the
/// second's folder for Claude Code. False when the account is the second and there is none (the caller falls back).
pub fn env(cmd: &mut std::process::Command, ai: &str, account: u8) -> bool {
    if account != 2 {
        return true;
    }
    match (ai, prepare(ai)) {
        ("claude", Some(dir)) => {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
            true
        }
        _ => false,
    }
}

// ------------------------------------------------------------------ used up

/// Accounts that hit their usage limit, with when each is back (seconds since 1970). In memory: after a restart the
/// first turn finds out again.
static USED: Mutex<Option<HashMap<(String, u8), f64>>> = Mutex::new(None);

/// When nothing says when a limit resets, the account is tried again after this long.
pub const TRY_AGAIN: f64 = 3600.0;

/// An account hit its usage limit: used up until `until`.
pub fn mark_used(ai: &str, account: u8, until: f64) {
    #[cfg(test)]
    if STAND_IN.with(|s| s.borrow_mut().as_mut().map(|s| s.used.insert((ai.to_string(), account), until)).is_some()) {
        return;
    }
    USED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert((ai.to_string(), account), until);
}

/// When the account is back, if it is used up now.
pub fn used_until(ai: &str, account: u8, now: f64) -> Option<f64> {
    #[cfg(test)]
    if let Some(until) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.used.get(&(ai.to_string(), account)).copied())) {
        return until.filter(|until| *until > now);
    }
    USED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(ai.to_string(), account)).copied()).filter(|until| *until > now)
}

/// What deciding an account needs to know about one AI (so the rule can be tested without the app's state).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Have {
    /// the first account is used up now
    pub first_used: bool,
    /// a second account exists, is signed in, and is not used up now
    pub second_ready: bool,
}

/// What the app knows now. `second_signed_in`: what the tool last said about the second account.
pub fn have(ai: &str, now: f64, second_signed_in: bool) -> Have {
    Have { first_used: used_until(ai, 1, now).is_some(), second_ready: folder(ai).is_some() && second_signed_in && used_until(ai, 2, now).is_none() }
}

/// The account a new chat or a new expert starts on: the user's own whenever it is not used up; the second only while
/// the first is used up and the second is ready. Both used up: the first (it fails as it would without a second).
pub fn pick(have: Have) -> u8 {
    if have.first_used && have.second_ready { 2 } else { 1 }
}

/// The account a chat moves to when `ran_out` has just hit its limit, if the other one can take it: the second when
/// it is ready, the first when it is not used up. None: stay (both are used up, or there is no second account).
pub fn other(have: Have, ran_out: u8) -> Option<u8> {
    match ran_out {
        2 => (!have.first_used).then_some(1),
        _ => have.second_ready.then_some(2),
    }
}

/// When a limit resets, from the tool's own words: "Resets in 2h6m48s" (Gemini's quota line), as seconds from now.
/// Claude Code names a clock time ("resets 12:50am (America/Los_Angeles)"): see `resets_at_clock`.
pub fn resets_in(said: &str) -> Option<f64> {
    let low = said.to_lowercase();
    let rest = low.split("resets in ").nth(1)?;
    let word: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let (mut total, mut number, mut any) = (0f64, String::new(), false);
    for c in word.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: f64 = number.parse().ok()?;
        number.clear();
        total += n * match c {
            'd' => 86_400.0,
            'h' => 3_600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        any = true;
    }
    (any && number.is_empty() && total > 0.0).then_some(total)
}

/// When a limit resets, from a clock time in the tool's words: Claude Code's "resets 12:50am (America/Los_Angeles)"
/// and "Your limit will reset at 3pm", as seconds from now. `day_secs`: the seconds since midnight on this computer's
/// clock (Claude Code names the time in the computer's own zone). The next time the clock shows it, so always within
/// a day. Words that name a day ("resets Nov 28 at 9am") are not read: nothing is guessed about a date.
pub fn resets_at_clock(said: &str, day_secs: f64) -> Option<f64> {
    static CLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\breset(?:s)?(?: at)? (\d{1,2})(?::(\d{2}))?\s?(am|pm)\b").unwrap());
    let low = said.to_lowercase();
    let c = CLOCK.captures(&low)?;
    let (hour, min): (u32, u32) = (c[1].parse().ok()?, c.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?);
    if !(1..=12).contains(&hour) || min > 59 {
        return None;
    }
    let hour = hour % 12 + if &c[3] == "pm" { 12 } else { 0 };
    let ahead = f64::from(hour * 3600 + min * 60) - day_secs;
    Some(if ahead <= 0.0 { ahead + 86_400.0 } else { ahead })
}

/// The seconds since midnight on this computer's clock. None where the app does not read it (Windows: no second
/// accounts there).
pub fn day_secs(now: f64) -> Option<f64> {
    #[cfg(windows)]
    {
        let _ = now;
        None
    }
    #[cfg(not(windows))]
    {
        paths::local_time(now as i64).map(|tm| f64::from(tm.hour * 3600 + tm.min * 60 + tm.sec))
    }
}

/// The reset time in a Claude Code `rate_limit_event` whose limit is reached (its status is not "allowed"): the
/// time of the event itself, else of the first window that is not allowed. None for an event that allows the turn.
pub fn event_reset(event: &Value) -> Option<f64> {
    let info = &event["rate_limit_info"];
    let reached = |v: &Value| v["status"].as_str().is_some_and(|s| s != "allowed" && s != "allowed_warning");
    if reached(info) && let Some(t) = info["resetsAt"].as_f64().filter(|t| *t > 0.0) {
        return Some(t);
    }
    info["unifiedWindows"].as_object().into_iter().flatten().map(|(_, w)| w).filter(|w| reached(w)).filter_map(|w| w["resetsAt"].as_f64()).filter(|t| *t > 0.0).reduce(f64::max)
}

/// When an account that just hit its limit is back, and whether the tool said so: its own time when it gave one that
/// lies ahead (`event`, from `event_reset`; else its words, `resets_in` or `resets_at_clock` with this computer's
/// `day_secs`); otherwise an hour from now, when it is simply tried again.
pub fn back_at(said: &str, event: Option<f64>, now: f64, day_secs: Option<f64>) -> (f64, bool) {
    let told = event.filter(|t| *t > now).or_else(|| resets_in(said).map(|s| now + s)).or_else(|| day_secs.and_then(|d| resets_at_clock(said, d)).map(|s| now + s));
    (told.unwrap_or(now + TRY_AGAIN), told.is_some())
}

/// A reset time as the chat says it: "in about 2 hours", "in about 40 minutes".
pub fn in_words(until: f64, now: f64) -> String {
    let mins = ((until - now) / 60.0).ceil().max(1.0) as u64;
    if mins < 90 { format!("in about {mins} minutes") } else { format!("in about {} hours", (mins + 30) / 60) }
}

/// For the page: which AIs can have a second account, whether each has one, and when a used-up account is back.
pub fn state() -> Value {
    let now = paths::now_secs();
    let mut out = Map::new();
    for ai in AIS {
        out.insert(ai.to_string(), json!({"offered": !cfg!(windows), "exists": folder(ai).is_some(), "used": [used_until(ai, 1, now), used_until(ai, 2, now)]}));
    }
    Value::Object(out)
}

/// Held by tests that mark accounts used up or add a second account (tests run side by side, and these are the
/// app's one copy).
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_accounts_folder_is_the_apps_own_and_nothing_else() {
        // only names the app makes, under the app's folder: the setting cannot point anywhere else
        assert!(name_ok("claude", "claude-2-0a1b2c3d4e5f"));
        for bad in ["", "claude-2-", "claude-2-abc", "claude-2-../../x", "claude-2-ABCDEF", "../claude-2-abcdef", "/tmp/claude-2-abcdef", "claude-2-abcdef/x", "gemini-2-abcdef", &format!("claude-2-{}", "a".repeat(17))] {
            assert!(!name_ok("claude", bad), "{bad}");
        }
        assert!(!name_ok("gemini", "gemini-2-abcdef"), "Gemini can have none (agy keeps one sign-in per HOME)");
        let settings = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(folder_in(&settings(json!({"second_accounts": {"claude": "claude-2-abcdef"}})), "claude"), Some(base().join("claude-2-abcdef")));
        for bad in [json!({"second_accounts": {"claude": "../../Documents"}}), json!({"second_accounts": {"claude": "/etc"}}), json!({"second_accounts": "claude-2-abcdef"}), json!({})] {
            assert_eq!(folder_in(&settings(bad.clone()), "claude"), None, "{bad}");
        }
        // as a guarded setting: what could not be the app's own is dropped when the file is read
        assert_eq!(checked_setting(&json!({"claude": "claude-2-abcdef", "gemini": "gemini-2-abcdef", "cursor": 1, "x": "/tmp"})), Some(json!({"claude": "claude-2-abcdef"})));
        assert_eq!(checked_setting(&json!("claude-2-abcdef")), None);
        // and no agent's file tools may write there
        assert!(paths::app_protected().iter().any(|(p, dir)| *dir && *p == base()), "the folder of second accounts is among the app's protected places");
        assert!(paths::GUARDED.contains(&SETTING));
    }

    #[cfg(unix)]
    #[test]
    fn the_users_own_setup_is_linked_into_the_second_folder_and_removing_it_leaves_theirs_alone() {
        let root = paths::test_dir("accounts-link");
        let _ = fs::remove_dir_all(&root);
        let (users, second) = (root.join("users"), root.join("second"));
        fs::create_dir_all(users.join("skills").join("mine")).unwrap();
        fs::write(users.join("settings.json"), "{\"hooks\": {}}").unwrap();
        fs::write(users.join("skills").join("mine").join("SKILL.md"), "x").unwrap();
        fs::write(users.join(".claude.json"), "{\"oauthAccount\": {}}").unwrap();
        fs::create_dir_all(users.join("projects")).unwrap();
        link_into(&second, &users, &CLAUDE_LINKED);
        let is_link = |name: &str| fs::symlink_metadata(second.join(name)).is_ok_and(|m| m.file_type().is_symlink());
        assert!(is_link("settings.json") && is_link("skills"), "what a turn loads is the user's own");
        assert_eq!(fs::read_to_string(second.join("skills").join("mine").join("SKILL.md")).unwrap(), "x");
        assert!(!second.join(".claude.json").exists() && !second.join("projects").exists(), "who is signed in and the conversations are never shared");
        assert!(!second.join("plugins").exists(), "nothing is made up for what the user does not have");
        // the user adds plugins later, the tool has made files of its own, and a linked thing has gone
        fs::create_dir_all(users.join("plugins")).unwrap();
        fs::write(second.join(".claude.json"), "{}").unwrap();
        fs::remove_file(users.join("settings.json")).unwrap();
        link_into(&second, &users, &CLAUDE_LINKED);
        assert!(is_link("plugins") && !is_link("settings.json") && fs::symlink_metadata(second.join("settings.json")).is_err(), "new things are linked, a dead link goes");
        assert_eq!(fs::read_to_string(second.join(".claude.json")).unwrap(), "{}", "the tool's own file is left alone");
        // removing the second account deletes the app's folder, never what a link points to
        remove_folder(&second);
        assert!(!second.exists());
        assert!(users.join("skills").join("mine").join("SKILL.md").is_file() && users.join("plugins").is_dir() && users.join(".claude.json").is_file(), "the user's own setup is untouched");
    }

    #[test]
    fn a_new_chat_starts_on_the_first_account_that_is_not_used_up() {
        let none = Have::default();
        assert_eq!(pick(none), 1, "no second account, nothing used up");
        assert_eq!(pick(Have { first_used: false, second_ready: true }), 1, "the user's own first, whenever it is available");
        assert_eq!(pick(Have { first_used: true, second_ready: true }), 2);
        assert_eq!(pick(Have { first_used: true, second_ready: false }), 1, "both used up, or no second: as without one");
        // a chat whose account just ran out moves to the other once, if that one can take it
        assert_eq!(other(Have { first_used: true, second_ready: true }, 1), Some(2));
        assert_eq!(other(Have { first_used: true, second_ready: false }, 1), None, "the second is used up, signed out or not there: it stays");
        assert_eq!(other(Have { first_used: false, second_ready: false }, 2), Some(1), "from the second back to the user's own once that has reset");
        assert_eq!(other(Have { first_used: true, second_ready: false }, 2), None, "both used up: no bounce");
    }

    #[test]
    fn used_up_lasts_until_the_reset_time_and_no_longer() {
        let _one = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        mark_used("claude", 1, 2_000.0);
        assert_eq!(used_until("claude", 1, 1_000.0), Some(2_000.0));
        assert_eq!(used_until("claude", 1, 2_000.0), None, "back at its reset time: new chats go to the user's own again");
        assert_eq!(used_until("claude", 2, 1_000.0), None);
        let h = have("claude", 1_000.0, true);
        assert!(h.first_used && !h.second_ready, "no second account's folder in this test: not ready, whatever its sign-in");
        mark_used("claude", 1, 0.0);
    }

    #[test]
    fn the_tools_own_words_say_when_a_limit_resets() {
        // Gemini's quota line, from the user's chats of 22 November 2026 (as handed to the builder)
        let gemini = "Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 2h6m48s.";
        assert_eq!(resets_in(gemini), Some(2.0 * 3600.0 + 6.0 * 60.0 + 48.0));
        assert_eq!(resets_in("Gemini could not run this message: Individual quota reached. Resets in 45m."), Some(2700.0));
        assert_eq!(resets_in("Resets in 1d2h"), Some(93_600.0));
        assert_eq!(resets_in("Resets in 30s"), Some(30.0));
        // no duration the app can read: none (an hour is then waited, `back_at`)
        for other in ["Resets in a while", "resets 12:50am (America/Los_Angeles)", "Resets in 5 minutes", "Resets in 2x", "Resets in ", "quota reached"] {
            assert_eq!(resets_in(other), None, "{other}");
        }
        assert_eq!(back_at(gemini, None, 1_000.0, None), (1_000.0 + 7_608.0, true));
        // Claude Code's words name a clock time; its event carries the time itself, and goes first
        let claude = "You've hit your 5-hour limit \u{b7} resets 12:50am (America/Los_Angeles)";
        let ten_pm = 22.0 * 3600.0;
        assert_eq!(back_at(claude, Some(9_000.0), 1_000.0, Some(ten_pm)), (9_000.0, true));
        assert_eq!(back_at(claude, None, 1_000.0, Some(ten_pm)), (1_000.0 + 2.0 * 3600.0 + 50.0 * 60.0, true), "the next 12:50am on this computer's clock");
        assert_eq!(back_at(claude, Some(500.0), 1_000.0, Some(ten_pm)).0, 1_000.0 + 2.0 * 3600.0 + 50.0 * 60.0, "an event's time that has passed is no reset time");
        assert_eq!(back_at(claude, None, 1_000.0, None), (1_000.0 + TRY_AGAIN, false), "no clock to read it by: tried again after an hour");
        assert_eq!(back_at("You've hit your usage limit", None, 1_000.0, Some(ten_pm)), (1_000.0 + TRY_AGAIN, false), "no time given: tried again after an hour");
        // the clock times Claude Code is known to write
        assert_eq!(resets_at_clock("Claude usage limit reached. Your limit will reset at 3pm.", 14.5 * 3600.0), Some(1800.0));
        assert_eq!(resets_at_clock("resets 3pm", 15.0 * 3600.0), Some(86_400.0), "the time it is now: tomorrow's");
        assert_eq!(resets_at_clock("resets 12pm", 11.0 * 3600.0), Some(3600.0));
        assert_eq!(resets_at_clock("resets 12:05am", 0.0), Some(300.0));
        // a day is never guessed, nor a time that is none
        for other in ["You've hit your weekly limit \u{b7} resets Nov 28 at 9am", "resets 13pm", "resets 9:75am", "resets soon", "resets 9", "it presets 9am"] {
            assert_eq!(resets_at_clock(other, 0.0), None, "{other}");
        }
        assert!(day_secs(1_795_000_000.0).is_some_and(|d| (0.0..86_400.0).contains(&d)) || cfg!(windows));
        assert_eq!(in_words(1_000.0 + 7_608.0, 1_000.0), "in about 2 hours");
        assert_eq!(in_words(1_000.0 + 2_400.0, 1_000.0), "in about 40 minutes");
    }

    #[test]
    fn a_rate_limit_event_gives_the_reset_time_only_when_the_limit_is_reached() {
        // the one recorded event (tests/fixtures/claude/plain-with-rate-limit.ndjson): the turn was allowed
        let allowed = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1795633200, "rateLimitType": "seven_day", "overageStatus": "rejected",
            "unifiedWindows": {"seven_day": {"utilization": 0.41, "resetsAt": 1795633200, "status": "allowed"}}}});
        assert_eq!(event_reset(&allowed), None);
        // stand-ins (no turn that hit a limit was ever recorded): the same shape with the limit reached
        let rejected = json!({"rate_limit_info": {"status": "rejected", "resetsAt": 1795633200, "rateLimitType": "five_hour"}});
        assert_eq!(event_reset(&rejected), Some(1795633200.0));
        let window = json!({"rate_limit_info": {"status": "allowed", "unifiedWindows": {"five_hour": {"utilization": 1.0, "resetsAt": 1795000000, "status": "rejected"}, "seven_day": {"utilization": 0.4, "resetsAt": 1795633200, "status": "allowed"}}}});
        assert_eq!(event_reset(&window), Some(1795000000.0));
        assert_eq!(event_reset(&json!({"type": "result"})), None);
    }
}
