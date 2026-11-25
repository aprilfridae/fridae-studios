//! A second account for an AI, with rollover at the usage limit.
//! Design: docs/superpowers/specs/2026-11-23-second-accounts-design.md. What was measured: DEVELOPING.md, "A second
//! account: what was measured first".
//!
//! Claude: Claude Code keeps its sign-in per config folder (`CLAUDE_CONFIG_DIR`; measured), so Claude's second
//! account is a config folder of the app's own, beside Fridae's OpenCode copy in the app's data folder, outside every
//! folder an AI is given. The user's own settings, skills and plugins are linked into it (`prepare`), so a turn there
//! loads what a turn on the first account loads (measured from Claude Code's `init` line); who is signed in, and the
//! conversations, are the folder's own.
//!
//! Gemini: agy keeps the user's sign-in in one keychain entry, which a command finds by HOME alone, and its data
//! folder under HOME (measured). So Gemini's second account is a home folder of the app's own, in the same place:
//! every agy command of the second account runs with it as HOME (`home`). The user's keychain cannot be read or
//! written from there (measured), so the second account can never be, or replace, the user's own sign-in. agy
//! keeps a sign-in in the default keychain of its HOME and puts a system alert on the screen when there is none
//! (seen by the user), so the home is given a keychain of its own before agy runs there (`keychain_open`): a file
//! in the home, made and unlocked by the app with a password of the app's own, the home's default by the home's
//! own list file. The app's tool server is given the app's own HOME back.
//!
//! The first account is the user's own sign-in and is never touched. The second is signed in with the tool's own
//! sign-in (`signin`, under the id "<ai>:<folder name>"), by the user's click, and removed by deleting the app's
//! folder, never by any command of the tool. The app reads, copies, moves and stores no token.
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
pub const AIS: [&str; 2] = ["claude", "gemini"];

/// The guarded setting: `{"claude": ["<folder name>", ...], "gemini": [...]}`: each AI's added accounts, in the
/// order they are tried after the user's own (the order they were added). Each name is one the app made
/// (`name_ok`), under `base()`.
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

/// The names in one AI's entry of the setting: a list, or one name alone (as the setting was while an AI could
/// have one added account). Only names the app could have made, each once, in their order.
fn names_in(ai: &str, entry: &Value) -> Vec<String> {
    let listed: Vec<&str> = match entry {
        Value::String(one) => vec![one.as_str()],
        Value::Array(all) => all.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut kept: Vec<String> = Vec::new();
    for name in listed {
        if name_ok(ai, name) && !kept.iter().any(|k| k == name) {
            kept.push(name.to_string());
        }
    }
    // never more than the app keeps for one AI, wherever the list came from (the settings file on a first run,
    // the start-up question, a settings copy): every reader of the list is bounded by this
    kept.truncate(MOST);
    kept
}

/// The place of the account that is `index`-th in the list of added ones: 2 for the first. A count that cannot
/// wrap, whatever the list's length (it is never longer than `MOST`).
fn place(index: usize) -> u8 {
    u8::try_from(index.saturating_add(2)).unwrap_or(u8::MAX)
}

/// The setting as the app keeps it: only AIs that can have added accounts, only names the app could have made.
pub fn checked_setting(v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    Some(Value::Object(m.iter().filter(|(ai, _)| AIS.contains(&ai.as_str())).map(|(ai, entry)| (ai.clone(), json!(names_in(ai, entry)))).collect()))
}

fn list_in(settings: &Map<String, Value>, ai: &str) -> Vec<String> {
    settings.get(SETTING).map(|v| names_in(ai, &v[ai])).unwrap_or_default()
}

/// An AI's added accounts, by their folders' names, in the order they are tried after the user's own (from the
/// app's memory of the guarded settings). The order is this list's and nothing else's.
pub fn list(ai: &str) -> Vec<String> {
    #[cfg(test)]
    if let Some(tags) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.extras.iter().map(|e| e.tag).collect::<Vec<_>>())) {
        return if AIS.contains(&ai) { tags.iter().map(|tag| format!("{ai}-2-{tag}")).collect() } else { Vec::new() };
    }
    list_in(&paths::guarded_settings(), ai)
}

/// Whether the AI has any added account.
pub fn any(ai: &str) -> bool {
    !list(ai).is_empty()
}

/// An account by its place in the order: 1 is the user's own (it has no name), 2 the first added, and so on. The
/// name is what an account is: a place changes when an account before it is removed. So the place is only ever for
/// what the user reads ("account 3"); whatever is *done* for an account (its folder prepared, a command pointed
/// at it, its sign-in asked, its usage marked) goes by its name, which a chat keeps.
#[cfg(test)]
pub fn name_at(ai: &str, account: u8) -> Option<String> {
    (account >= 2).then(|| list(ai).get(account as usize - 2).cloned()).flatten()
}

/// The place of an added account, by its name, as it is now.
pub fn position(ai: &str, name: &str) -> Option<u8> {
    list(ai).iter().position(|n| n == name).map(place)
}

/// An added account's folder, while the AI has that account.
pub fn folder_of(ai: &str, name: &str) -> Option<PathBuf> {
    list(ai).iter().any(|n| n == name).then(|| base().join(name))
}

/// The name of an added account's sign-in (`signin`): "<ai>:<folder name>".
pub fn id_of(ai: &str, name: &str) -> String {
    format!("{ai}:{name}")
}

/// Every added account's sign-in, for every AI.
pub fn ids() -> Vec<String> {
    AIS.iter().flat_map(|ai| list(ai).into_iter().map(move |n| id_of(ai, &n))).collect()
}

/// Whether an added account can take a chat as far as its sign-in goes: its tool last said that an account is
/// signed in there, one of its own (not a key or token from the environment, which is the same for every account,
/// and not an account the user's own or an earlier added one is signed in to: `signin::second_ready`).
pub fn signed_in_of(ai: &str, name: &str) -> bool {
    #[cfg(test)]
    if let Some(yes) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.extras.iter().any(|e| format!("{ai}-2-{}", e.tag) == name && e.signed_in))) {
        return yes;
    }
    crate::signin::second_ready(&id_of(ai, name))
}

/// Whether an added account is known to be signed out (its tool said so; not merely "not known yet").
pub fn signed_out_of(ai: &str, name: &str) -> bool {
    #[cfg(test)]
    if let Some(out) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.extras.iter().any(|e| format!("{ai}-2-{}", e.tag) == name && e.out))) {
        return out;
    }
    crate::signin::known_out(&id_of(ai, name))
}

/// Tests: one test's own accounts in place of the app's one copy (tests run side by side, each on its own thread):
/// which accounts are added, whether each is signed in, and which accounts are used up until when.
#[cfg(test)]
#[derive(Default, Clone)]
pub struct StandIn {
    /// the added accounts, in their order
    pub extras: Vec<Extra>,
    /// by AI and by the account's name ("" for the user's own); shared by every thread the test's chat works on
    /// (`stand_in_here`)
    pub used: std::sync::Arc<Mutex<HashMap<(String, String), f64>>>,
}

/// Tests: one added account of a stand-in.
#[cfg(test)]
#[derive(Clone)]
pub struct Extra {
    /// the end of its folder's name (`<ai>-2-<tag>`)
    pub tag: &'static str,
    pub signed_in: bool,
    /// its tool said that nobody is signed in
    pub out: bool,
}

#[cfg(test)]
thread_local! {
    pub static STAND_IN: std::cell::RefCell<Option<StandIn>> = const { std::cell::RefCell::new(None) };
}

/// Tests: this test's accounts from here on: the user's own and, with `second`, one added account (`<ai>-2-standin`).
#[cfg(test)]
pub fn stand_in(second: bool, signed_in: bool) {
    let one = [("standin", signed_in)];
    stand_in_many(if second { &one } else { &[] });
}

/// Tests: the same with one added account that has a folder of the test's own (`<ai>-2-<tag>`), signed in.
#[cfg(test)]
pub fn stand_in_tagged(tag: &'static str) {
    stand_in_many(&[(tag, true)]);
}

/// Tests: the user's own account and these added ones, in this order: (the end of the folder's name, signed in).
#[cfg(test)]
pub fn stand_in_many(extras: &[(&'static str, bool)]) {
    let extras = extras.iter().map(|(tag, signed_in)| Extra { tag, signed_in: *signed_in, out: false }).collect();
    STAND_IN.with(|s| *s.borrow_mut() = Some(StandIn { extras, used: Default::default() }));
}

/// Tests: every added account's tool says that nobody is signed in (or not known, again).
#[cfg(test)]
pub fn stand_in_out(out: bool) {
    STAND_IN.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            for e in &mut s.extras {
                e.out = out;
                e.signed_in = e.signed_in && !out;
            }
        }
    });
}

/// Tests: this thread's stand-in, to hand to another thread of the same test (a kept process's reader).
#[cfg(test)]
pub fn stand_in_now() -> Option<StandIn> {
    STAND_IN.with(|s| s.borrow().clone())
}

/// Tests: the stand-in another thread of the test handed over, for this thread from here on.
#[cfg(test)]
pub fn stand_in_here(given: &StandIn) {
    STAND_IN.with(|s| *s.borrow_mut() = Some(given.clone()));
}

/// A new added account: a new folder under a new name (never a name used before: on a Mac the tool's sign-in for a
/// folder may outlive the folder in the keychain, and must not come back with it), put last in the AI's list. Its
/// name.
pub fn add(ai: &str) -> Result<String, String> {
    if !AIS.contains(&ai) || cfg!(windows) {
        return Err("Another account is not offered for that AI here.".to_string());
    }
    if list(ai).len() >= MOST {
        return Err(format!("{MOST} added accounts is the most Fridae keeps for one AI. Remove one first."));
    }
    let mut bytes = [0u8; 6];
    let random = fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).is_ok();
    if !random {
        return Err("Fridae could not make a name for the account's folder.".to_string());
    }
    let name = format!("{ai}-2-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    debug_assert!(name_ok(ai, &name));
    paths::update_config(|cfg| {
        let mut all = cfg.get(SETTING).and_then(Value::as_object).cloned().unwrap_or_default();
        let mut names = names_in(ai, all.get(ai).unwrap_or(&Value::Null));
        names.push(name.clone());
        all.insert(ai.to_string(), json!(names));
        cfg.insert(SETTING.into(), Value::Object(all));
    });
    note_left(ai, None);
    if prepare_of(ai, &name).is_none() {
        return Err("Fridae could not make the account's folder.".to_string());
    }
    // (Gemini: with its keychain, so the sign-in that follows finds one and the Mac asks nothing)
    ready_of(ai, &name).map(|_| name).ok_or_else(|| "Fridae could not make a keychain for the account.".to_string())
}

/// How many added accounts one AI may have: far more than anyone signs in to, and few enough that a setting cannot
/// make the app ask a tool for its status without end.
pub const MOST: usize = 20;

/// Remove an added account: its folder is deleted (the app's own, under `base()`, by its checked name) and its
/// name taken out of the setting; the accounts after it move up one place. Nothing of the tool's is run, and the
/// user's own account is not touched. The folder that could not be deleted, if any: Setup then says so with its
/// place (`state`), since the sign-in may still be in it.
///
/// Whatever was still running there when it went (a status check on its way, a tool closing down) may write its
/// files back: the folder is looked at again a few seconds later, and once more after the longest a status check
/// may take, and deleted again.
pub fn remove(ai: &str, name: &str) -> Option<PathBuf> {
    let dir = folder_of(ai, name);
    paths::update_config(|cfg| {
        let mut all = cfg.get(SETTING).and_then(Value::as_object).cloned().unwrap_or_default();
        let names: Vec<String> = names_in(ai, all.get(ai).unwrap_or(&Value::Null)).into_iter().filter(|n| n != name).collect();
        all.insert(ai.to_string(), json!(names));
        cfg.insert(SETTING.into(), Value::Object(all));
    });
    if let Some(used) = USED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        used.remove(&(ai.to_string(), name.to_string()));
    }
    if let Some(d) = &dir {
        KEYCHAIN_FAILED.lock().unwrap_or_else(|e| e.into_inner()).retain(|f| f != d);
        KEYCHAIN_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).retain(|f| f != d);
    }
    // only a folder of its checked name, directly in the accounts folder, and only when that folder is the app's
    // own (not a link to somewhere else, where a folder of the same name would be somebody else's)
    let dir = dir.filter(|d| d.parent() == Some(base().as_path()) && real_folder(&base()))?;
    let gone = remove_folder(&dir);
    note_left(ai, (!gone).then(|| dir.clone()));
    if cfg!(not(test)) {
        let (ai, name, again) = (ai.to_string(), name.to_string(), dir.clone());
        std::thread::spawn(move || {
            for wait in [5, 35] {
                std::thread::sleep(std::time::Duration::from_secs(wait));
                // (never one that has been made anew: names are not reused, and a new one would be in the settings)
                if fs::symlink_metadata(&again).is_ok() && folder_of(&ai, &name).is_none() {
                    let gone = remove_folder(&again);
                    if gone {
                        forget_left(&ai, &again);
                    } else {
                        note_left(&ai, Some(again.clone()));
                    }
                }
            }
        });
    }
    (!gone).then_some(dir)
}

/// Removed accounts' folders that could not be deleted, by AI: said in Setup until they are gone, the app closes,
/// or another account is added for that AI.
static LEFT: Mutex<Option<HashMap<String, Vec<PathBuf>>>> = Mutex::new(None);

/// A folder that stayed after its account was removed is noted (Some), or the AI's notes are cleared (None).
fn note_left(ai: &str, left: Option<PathBuf>) {
    let mut g = LEFT.lock().unwrap_or_else(|e| e.into_inner());
    let all = g.get_or_insert_with(HashMap::new);
    match left {
        Some(dir) => {
            let of_ai = all.entry(ai.to_string()).or_default();
            if !of_ai.contains(&dir) {
                of_ai.push(dir);
            }
        }
        None => {
            all.remove(ai);
        }
    }
}

fn forget_left(ai: &str, dir: &Path) {
    if let Some(of_ai) = LEFT.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.get_mut(ai)) {
        of_ai.retain(|d| d != dir);
    }
}

/// The folders of removed accounts of this AI that are still there.
fn left(ai: &str) -> Vec<PathBuf> {
    LEFT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(ai).cloned()).unwrap_or_default().into_iter().filter(|d| fs::symlink_metadata(d).is_ok()).collect()
}

/// Whether a path is a folder itself: there, a directory, and not a link to one. An account's folder that has been
/// swapped for a link (nothing of the app's does that) is never followed: not for a sign-in, a status, a turn or a
/// removal, since everything done "in the second account's folder" would then be done wherever the link leads.
fn real_folder(dir: &Path) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir())
}

/// Where a tool keeps an account's sign-in inside the account's folder: deleted before anything else, so that
/// whatever else in the folder cannot be deleted, the sign-in is not what stays. Claude Code: its credentials file
/// and its account file. agy: its data folder (`.gemini`), where a sign-in made without a keychain is kept, and the
/// home's `Library`, where the account's own keychain is (`KEYCHAIN`), with that keychain's password beside them.
const SIGN_IN_PLACES: [&str; 5] = [".credentials.json", ".claude.json", ".gemini", "Library", KEYCHAIN_KEY];

/// The files in an account's folder that hold its sign-in, for the read rules (`paths::app_secrets`): Claude Code's
/// credentials and account files, agy's sign-in files when it kept one without a keychain, the account's own
/// keychain folder and that keychain's password. Only these: the rest of the folder is the tool's own working place
/// (Claude Code spills large tool outputs, plans and memory there and tells the model to read them), and a deny
/// rule would block that even in bypass mode.
pub const SIGN_IN_FILES: [&str; 8] = [
    ".credentials.json",
    ".claude.json",
    KEYCHAIN_KEY,
    "Library/Keychains",
    ".gemini/oauth_creds.json",
    ".gemini/google_accounts.json",
    ".gemini/mcp-oauth-tokens.json",
    ".gemini/antigravity/oauth_creds.json",
];

/// Those files for every account folder there is: each AI's listed accounts, and any other folder still in
/// `base()` (a removed account's that could not all be deleted).
pub fn sign_in_files() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = AIS.iter().flat_map(|ai| list(ai).into_iter().map(|n| base().join(n))).collect();
    for e in fs::read_dir(base()).into_iter().flatten().flatten() {
        if !dirs.contains(&e.path()) {
            dirs.push(e.path());
        }
    }
    dirs.iter().flat_map(|d| SIGN_IN_FILES.iter().map(move |f| d.join(f))).collect()
}

/// Delete a second account's folder: the links in it first, each as a link (never what it points to), then the
/// sign-in's own place, then the rest. A folder that has been swapped for a link: the link is removed and nothing
/// else, never what it leads to. Something in it that cannot be deleted as it is (a folder a tool left read-only)
/// is made the user's to write and tried once more. Whether it is gone.
fn remove_folder(dir: &Path) -> bool {
    if !real_folder(dir) {
        let _ = fs::remove_file(dir);
        return fs::symlink_metadata(dir).is_err();
    }
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            let _ = fs::remove_file(entry.path());
        }
    }
    for place in SIGN_IN_PLACES {
        remove_all(&dir.join(place));
    }
    remove_all(dir);
    fs::symlink_metadata(dir).is_err()
}

/// A file or a folder and all in it, deleted; what cannot be deleted for its mode is made writable and tried again
/// (links are never followed: a link is removed as a link).
fn remove_all(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else { return };
    if !meta.file_type().is_dir() {
        let _ = fs::remove_file(path);
        return;
    }
    if fs::remove_dir_all(path).is_ok() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fn open_up(dir: &Path) {
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    open_up(&e.path());
                }
            }
        }
        open_up(path);
        let _ = fs::remove_dir_all(path);
    }
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
/// None as well when the folder, or the folder of all second accounts, is not a folder of its own but a link
/// (`real_folder`): nothing is then run or written for the second account at all.
pub fn prepare_of(ai: &str, name: &str) -> Option<PathBuf> {
    let dir = folder_of(ai, name).filter(|d| in_accounts(d))?;
    match ai {
        "gemini" => prepare_in(&dir, &paths::home_dir(), &GEMINI_LINKED).filter(|d| !keychain_changed_in(d) && (!keychain_used() || list_its_own(d))),
        _ => prepare_in(&dir, &users_claude(), &CLAUDE_LINKED),
    }
}

/// Whether a folder is an account's: a full path, directly in the accounts folder. Nothing is prepared, and no
/// command of a tool or of the Mac's keychain program is given a home, that is anything else: an empty or a
/// relative home would let such a program fall back to the user's real one.
fn in_accounts(dir: &Path) -> bool {
    dir.is_absolute() && dir.parent().is_some_and(|p| !p.as_os_str().is_empty() && p == base())
}

/// The same, and ready for a command of the tool to run there now: for Gemini on a Mac, with the account's own
/// keychain made and unlocked (`keychain_open`). None when it is not: the command must then not be run at all (agy
/// started where its keychain is missing or locked puts a system alert or a password box on the screen).
pub fn ready_of(ai: &str, name: &str) -> Option<PathBuf> {
    ready_of_until(ai, name, &|| false)
}

/// The same, given up as soon as `stop` says so (Stop was pressed while the keychain was being opened): None then
/// too, and the caller asks `stop` to tell the two apart.
pub fn ready_of_until(ai: &str, name: &str, stop: &dyn Fn() -> bool) -> Option<PathBuf> {
    let dir = prepare_of(ai, name)?;
    (ai != "gemini" || keychain_open(&dir, stop)).then_some(dir)
}

/// Whether the app uses an added account's folder at all, as far as it knows without running anything: it is a
/// folder of its own (`prepare_of`), and its keychain did not fail to open when that was last tried.
fn usable_of(ai: &str, name: &str) -> bool {
    prepare_of(ai, name).is_some_and(|dir| !KEYCHAIN_FAILED.lock().unwrap_or_else(|e| e.into_inner()).contains(&dir))
}

/// Whether an added Gemini account's keychain was changed from outside the app: what its home has for a keychain
/// is no longer its own (`keychain_its_own`), or its list file is there and no longer says the app's words
/// (`ListFile::Changed`: a command run in that home named another keychain, the user's own it may be). Looked at
/// before every command of the account, at the end of every turn on it, and for Setup. Once found it stays so
/// while the app runs, whatever is put back: the app does not repair it, does not use the account, and says so,
/// since agy may have used another keychain meanwhile.
pub fn keychain_changed(ai: &str, name: &str) -> bool {
    ai == "gemini" && folder_of(ai, name).filter(|d| in_accounts(d) && real_folder(d)).is_some_and(|d| keychain_changed_in(&d))
}

/// Homes found changed (`keychain_changed`), while the app runs.
static KEYCHAIN_CHANGED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn keychain_changed_in(home: &Path) -> bool {
    let mut noted = KEYCHAIN_CHANGED.lock().unwrap_or_else(|e| e.into_inner());
    if noted.iter().any(|h| h == home) {
        return true;
    }
    let changed = !keychain_its_own(home, &paths::home_dir()) || (keychain_used() && list_file(home) == ListFile::Changed);
    if changed {
        noted.push(home.to_path_buf());
    }
    changed
}

// ------------------------------------------------------------------ Gemini: a keychain of the account's own
//
// agy keeps a sign-in with the Mac's `security` program, in the default keychain of its HOME (its program's text:
// `/usr/bin/security -i` fed `add-generic-password -U -s gemini -a antigravity -w ...`, and `find-generic-password`
// to read it; no keychain is named). In a home with no keychain the Mac then puts "Keychain Not Found" on the
// screen (the user saw it at their first sign-in for an added account). So an added Gemini account's home gets a
// keychain of its own before any agy command runs there:
// - a keychain FILE in the home, `Library/Keychains/fridae.keychain-db`, made by `security create-keychain` with a
//   password the app makes up and keeps in the home (`KEYCHAIN_KEY`); before every agy command of the account it
//   is unlocked with that password and then told never to lock by itself (`keychain_open`: each in a run of its
//   own that must end well);
// - made the home's default, and its whole search list, by the home's OWN list file
//   (`Library/Preferences/com.apple.security.plist` inside the home), which the app writes as a plain file when
//   there is none, and never puts right when it says anything else (`keychain_changed`).
// All of it is found through HOME alone: every command is `/usr/bin/security -i` with HOME set to the account's
// home and the keychain given by its bare name, which resolves inside that home. Nothing here names the user's
// keychain, and nothing changes the user's own search list or default keychain (no `list-keychains -s`,
// `default-keychain -s` or `login-keychain -s` is ever run: the list is a file in the account's home).
//
// The name is not `login.keychain-db` on purpose: a keychain under that name in a home's Keychains folder does not
// unlock with the password it was created with (seen in scratch homes on 23 November 2026: exit 51 every time,
// while one under any other name unlocks), so the app could never open it again once it is locked, and agy would
// make the Mac ask the user for a password nobody has.

/// The file name of an added Gemini account's own keychain, in `<home>/Library/Keychains`.
pub const KEYCHAIN: &str = "fridae.keychain-db";

/// Where the password of that keychain is kept: a file in the account's home (the user's alone to read), under the
/// protected accounts folder. Made up by the app, used for nothing else, shown and logged nowhere.
const KEYCHAIN_KEY: &str = ".fridae-keychain-key";

/// The home's keychain list file, inside the account's home.
const KEYCHAIN_LIST_FILE: &str = "com.apple.security.plist";

macro_rules! keychain_entry {
    () => {
        "\t\t<dict>\n\t\t\t<key>DbName</key>\n\t\t\t<string>~/Library/Keychains/fridae.keychain</string>\n\t\t\t<key>GUID</key>\n\t\t\t<string>{87191ca3-0fc9-11d4-849a-000502b52122}</string>\n\t\t\t<key>SubserviceType</key>\n\t\t\t<integer>6</integer>\n\t\t</dict>\n"
    };
}

/// What the home's list file says: the account's own keychain is the one keychain searched and the default, named
/// from the home (`~/`), so the words are the same for every account. Byte for byte what `security` itself writes
/// for it.
const KEYCHAIN_LIST: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    "<plist version=\"1.0\">\n<dict>\n",
    "\t<key>DLDBSearchList</key>\n\t<array>\n",
    keychain_entry!(),
    "\t</array>\n\t<key>DefaultKeychain</key>\n\t<array>\n",
    keychain_entry!(),
    "\t</array>\n</dict>\n</plist>\n"
);

/// Whether an added Gemini account gets a keychain here: on a Mac (and in tests, which never run the real program).
fn keychain_used() -> bool {
    cfg!(target_os = "macos") || cfg!(test)
}

/// Whether a path is a file itself: there, a regular file, and not a link to one.
fn real_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// Whether what a home of the app's own (an added Gemini account's) has for a keychain is its own and nobody
/// else's. The promise that the user's own sign-in cannot be reached from there rests on it: a link at
/// `Library/Keychains` to the user's own brings their keychain back for every command run with that home (measured:
/// the entry is found again), and a sign-in made there would then be written into the user's own entry. So the
/// home is not used for anything (no status, no sign-in, no turn; Setup says so, and Remove takes it away) when:
/// - `Library`, `Library/Keychains` or `Library/Preferences` is there and is not a folder itself (a link);
/// - the home's keychain list file is there and is not a file itself;
/// - anything in `Library/Keychains` is a link, or is a file that is also somewhere else (a second name for the
///   same file: that is how the user's own keychain could be there without a link), or is one of the files in the
///   user's own `Library/Keychains` (`users`: the user's home).
///
/// The app's own keychain there, a regular file made in that home, is what it is for; so is nothing at all yet.
fn keychain_its_own(home: &Path, users: &Path) -> bool {
    let library = home.join("Library");
    let (keychains, preferences) = (library.join("Keychains"), library.join("Preferences"));
    let there = |p: &Path| fs::symlink_metadata(p).is_ok();
    if [&library, &keychains, &preferences].into_iter().any(|d| there(d) && !real_folder(d)) {
        return false;
    }
    let list = preferences.join(KEYCHAIN_LIST_FILE);
    if there(&list) && !real_file(&list) {
        return false;
    }
    if !there(&keychains) {
        return true;
    }
    let Ok(entries) = fs::read_dir(&keychains) else { return false };
    #[cfg(unix)]
    let of_users: Vec<(u64, u64)> = {
        use std::os::unix::fs::MetadataExt;
        fs::read_dir(users.join("Library").join("Keychains")).into_iter().flatten().flatten().filter_map(|e| fs::metadata(e.path()).ok()).map(|m| (m.dev(), m.ino())).collect()
    };
    #[cfg(not(unix))]
    let _ = users;
    for entry in entries.flatten() {
        // (one that is gone by now is passed over: the Mac writes and removes lock and temporary files here while
        // a sign-in is saved, and a file that is not there is nobody's keychain)
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return false,
        };
        if meta.file_type().is_symlink() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.file_type().is_file() && of_users.contains(&(meta.dev(), meta.ino())) {
                return false;
            }
            // a second name for the file: refused when it is still there after a moment (it was not checked
            // whether the Mac's own saving ever gives a keychain file one for an instant; a link someone made
            // stays)
            let shared = |m: &fs::Metadata| m.file_type().is_file() && m.nlink() != 1;
            if shared(&meta) {
                let mut still = true;
                for _ in 0..3 {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    still = fs::symlink_metadata(entry.path()).is_ok_and(|m| shared(&m) || m.file_type().is_symlink());
                    if !still {
                        break;
                    }
                }
                if still {
                    return false;
                }
            }
        }
    }
    true
}

/// One at a time for each home's keychain: making it, unlocking it, its settings. A lock of the home's own, so one
/// account's keychain program that is slow holds up nobody else's turn; looking at an account (Setup's list)
/// takes none.
static KEYCHAIN_LOCKS: Mutex<Option<HashMap<PathBuf, std::sync::Arc<Mutex<()>>>>> = Mutex::new(None);

fn keychain_lock(home: &Path) -> std::sync::Arc<Mutex<()>> {
    KEYCHAIN_LOCKS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).entry(home.to_path_buf()).or_default().clone()
}

/// Homes whose keychain could not be made or opened when that was last tried: Setup says the account is not used.
static KEYCHAIN_FAILED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// For the names of files written beside their place and then put there whole.
static NEW_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn beside(file: &Path) -> PathBuf {
    let n = NEW_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    file.with_file_name(format!(".fridae-new-{}-{n}-{name}", std::process::id()))
}

/// Write a file whole or not at all, and never over anything that is already there under its name: written
/// beside its place (the user's alone to read when `private`), then given its name. Whether it is there now, from
/// this write.
fn place_whole(file: &Path, text: &str, private: bool) -> bool {
    let new = beside(file);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let written = options.open(&new).and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()).and_then(|_| f.sync_all())).is_ok();
    // (a second name for the finished file, which fails when the name is taken; then the first name goes)
    let placed = written && fs::hard_link(&new, file).is_ok();
    let _ = fs::remove_file(&new);
    placed
}

/// What a home's keychain list file is now.
#[derive(PartialEq, Debug)]
enum ListFile {
    /// not there (a new home, or it was deleted): the app writes it
    Missing,
    /// there, a file itself, saying exactly the app's words (`KEYCHAIN_LIST`)
    Ours,
    /// there and anything else: other words, a link, not a file. Never put right by the app (`keychain_changed`)
    Changed,
}

fn list_file(home: &Path) -> ListFile {
    let list = home.join("Library").join("Preferences").join(KEYCHAIN_LIST_FILE);
    match fs::symlink_metadata(&list) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ListFile::Missing,
        Ok(m) if m.file_type().is_file() && fs::read(&list).is_ok_and(|b| b == KEYCHAIN_LIST.as_bytes()) => ListFile::Ours,
        _ => ListFile::Changed,
    }
}

/// The home's own list file says the app's words (`KEYCHAIN_LIST`): the account's keychain is the home's default
/// before any command runs there. A file that is MISSING is written (a new home, or one where it was deleted),
/// whole and never over anything. A file that says anything else is left exactly as it is and the answer is no:
/// the app never puts a changed list right in silence (`keychain_changed`). The home's keychain folders are made,
/// the user's alone. Called after `keychain_its_own` (so none of them is a link). Whether it says them.
fn list_its_own(home: &Path) -> bool {
    let library = home.join("Library");
    let preferences = library.join("Preferences");
    for dir in [&library, &library.join("Keychains"), &preferences] {
        if fs::symlink_metadata(dir).is_err() && fs::create_dir(dir).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
            }
        }
        if !real_folder(dir) {
            return false;
        }
    }
    match list_file(home) {
        ListFile::Ours => true,
        ListFile::Changed => false,
        ListFile::Missing => {
            let _ = place_whole(&preferences.join(KEYCHAIN_LIST_FILE), KEYCHAIN_LIST, false);
            // (written by this call or by another at the same moment: what counts is what it says now)
            list_file(home) == ListFile::Ours
        }
    }
}

/// The kept password of a home's keychain: 48 lower-case hex digits and nothing else (it is put into the text
/// `security` reads, so nothing that is not exactly that is ever used), from a file that is a file itself.
fn keychain_key(home: &Path) -> Option<String> {
    let file = home.join(KEYCHAIN_KEY);
    let key = real_file(&file).then(|| fs::read_to_string(&file).ok()).flatten()?;
    let key = key.trim_end_matches('\n');
    (key.len() == 48 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then(|| key.to_string())
}

/// A new password for a home's keychain, made up from the system's random numbers and kept in the home, the
/// user's alone to read: put there whole or not at all (`place_whole`), so a write that was cut short leaves no
/// file that is no password and would keep the account from ever getting one. None when there is something under
/// that name already, or it could not be made or kept.
fn new_keychain_key(home: &Path) -> Option<String> {
    let file = home.join(KEYCHAIN_KEY);
    if fs::symlink_metadata(&file).is_ok() {
        return None;
    }
    let mut bytes = [0u8; 24];
    fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).ok()?;
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    place_whole(&file, &key, true).then_some(key)
}

/// The Mac's keychain program, told to read its commands from its input (so a password is never on a command line,
/// where other programs can read it), with the account's home as its HOME and nothing else that could name a home:
/// the keychain it is told about by its bare name is then `<home>/Library/Keychains/<name>`.
fn security_command(home: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("/usr/bin/security");
    cmd.arg("-i").current_dir(home);
    self::home(&mut cmd, home);
    cmd
}

/// What `security` is fed to make a home's keychain, with the kept password. One command to a run: the program
/// ends with the result of the last line it read, so each result the app needs is asked for by itself.
fn create_feed(key: &str) -> String {
    format!("create-keychain -p {key} {KEYCHAIN}\n")
}

/// What `security` is fed to unlock a home's keychain with the kept password (it answers at once, right or wrong:
/// with the password given it asks nothing on the screen).
fn unlock_feed(key: &str) -> String {
    format!("unlock-keychain -p {key} {KEYCHAIN}\n")
}

/// What `security` is fed so that a home's keychain never locks by itself (no time limit, not when the Mac
/// sleeps). A keychain is made with the Mac's own settings (seen: it locks after five minutes unused, and on
/// sleep), and a keychain that locked under a turn makes agy's next look at it a password box on the screen. So
/// this is fed after EVERY unlock, in a run of its own whose result counts, never only when the keychain is made,
/// and never before the unlock (a locked keychain asked for this may ask on the screen).
fn settings_feed() -> String {
    format!("set-keychain-settings {KEYCHAIN}\n")
}

/// The longest one run of the keychain program may take (it answers in a fraction of a second).
const SECURITY_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Run `security` for a home with that text to read, given up when `stop` says so; whether it ended well. What it
/// prints is thrown away. Never for a home that is not an account's folder (`in_accounts`).
fn security(home: &Path, feed: String, stop: &dyn Fn() -> bool) -> bool {
    if !in_accounts(home) {
        return false;
    }
    #[cfg(test)]
    return security_stand_in(home, &feed, stop);
    #[allow(unreachable_code)]
    crate::signin::run_fed(security_command(home), SECURITY_LIMIT, Some(feed), "keychain", stop).is_some_and(|(_, _, ok)| ok)
}

/// For tests: every text `security` would have been fed, by home, and the homes where it is to fail ("create",
/// "unlock" or "settings"). Tests never run the real program: a keychain is a file holding the password it was
/// "made" with, and unlocks with that password alone.
#[cfg(test)]
pub static SECURITY_FOR_TEST: Mutex<(Vec<(PathBuf, String)>, SecurityFails)> = Mutex::new((Vec::new(), Vec::new()));

#[cfg(test)]
type SecurityFails = Vec<(PathBuf, &'static str)>;

#[cfg(test)]
fn security_stand_in(home: &Path, feed: &str, stop: &dyn Fn() -> bool) -> bool {
    let mut g = SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner());
    g.0.push((home.to_path_buf(), feed.to_string()));
    let fails = |what: &str| g.1.iter().any(|(h, w)| h == home && *w == what);
    let file = home.join("Library").join("Keychains").join(KEYCHAIN);
    let key = feed.split_whitespace().nth(2).unwrap_or_default();
    // (as the real program: one command to a run is all the app may feed, and a run that is stopped did not end well)
    if feed.lines().count() != 1 || stop() {
        false
    } else if feed.starts_with("create-keychain ") {
        !fails("create") && fs::symlink_metadata(&file).is_err() && fs::write(&file, key).is_ok()
    } else if feed.starts_with("unlock-keychain ") {
        !fails("unlock") && fs::read_to_string(&file).is_ok_and(|made_with| made_with == key)
    } else if feed.starts_with("set-keychain-settings ") {
        !fails("settings") && real_file(&file)
    } else {
        false
    }
}

/// The account's own keychain, there, unlocked and set never to lock by itself, so that an agy command started in
/// this home now finds a keychain to read and to keep a sign-in in, and the Mac asks nothing, however long the
/// command runs. Made the first time (with a new password, kept in the home); unlocked, and its settings given
/// again, every time. False when it could not be: no keychain and none could be made, a keychain whose password
/// is not there (the app then never asks it anything, since that would put a password box on the screen), one
/// that did not unlock, or one whose settings did not take. The command must then not be run. Given up when
/// `stop` says so (false as well, and not held against the account).
fn keychain_open(home: &Path, stop: &dyn Fn() -> bool) -> bool {
    if !keychain_used() {
        return true;
    }
    let lock = keychain_lock(home);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let open = keychain_open_in(home, stop);
    let mut failed = KEYCHAIN_FAILED.lock().unwrap_or_else(|e| e.into_inner());
    failed.retain(|f| f != home);
    if !open && !stop() {
        failed.push(home.to_path_buf());
    }
    open
}

fn keychain_open_in(home: &Path, stop: &dyn Fn() -> bool) -> bool {
    if !in_accounts(home) {
        return false;
    }
    let file = home.join("Library").join("Keychains").join(KEYCHAIN);
    let there = fs::symlink_metadata(&file).is_ok();
    let key = match (keychain_key(home), there) {
        (Some(key), true) => key,
        (None, true) => return false,
        (kept, false) => {
            // (a password with no keychain: the keychain was never made, or was taken away; the password is used
            // for the new one. Something else under the password's name: nothing is done)
            let Some(key) = kept.or_else(|| new_keychain_key(home)) else { return false };
            // made in a run of its own, whose result is its own
            if !security(home, create_feed(&key), stop) || !real_file(&file) {
                return false;
            }
            key
        }
    };
    // the keychain as it is now, made in this moment or long ago, is checked like everything else in the home;
    // then unlocked; then, unlocked, told never to lock by itself: each in a run of its own, and each must succeed
    real_file(&file) && !keychain_changed_in(home) && security(home, unlock_feed(&key), stop) && security(home, settings_feed(), stop)
}

/// What of the user's own home a Gemini turn on the second account gets as well, by links: the git identity, so a
/// `git commit` a turn runs there is the user's. Nothing of agy's own (its settings, plugins and conversations are
/// the second home's: agy writes them, and its sign-in lives beside them), and none of the user's keys.
const GEMINI_LINKED: [&str; 1] = [".gitconfig"];

fn prepare_in(dir: &Path, users: &Path, names: &[&str]) -> Option<PathBuf> {
    let base = dir.parent()?;
    let _ = fs::create_dir_all(base);
    if !real_folder(base) || (fs::symlink_metadata(dir).is_ok() && !real_folder(dir)) {
        return None;
    }
    link_into(dir, users, names);
    real_folder(dir).then(|| dir.to_path_buf())
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

/// The environment that makes a tool's process run on an account, which is named: nothing for the user's own
/// (None: nothing of its environment is touched), an added account's folder for Claude Code (`point`), its home
/// for agy (`home`), with that home's own keychain open (`ready_of`). False when the added account is not there,
/// its folder is not one the app uses, or its keychain could not be opened: the command must then not be run at
/// all.
pub fn env_of(cmd: &mut std::process::Command, ai: &str, account: Option<&str>) -> bool {
    let Some(name) = account else { return true };
    match (ai, ready_of(ai, name)) {
        ("claude", Some(dir)) => {
            point(cmd, &dir);
            true
        }
        ("gemini", Some(dir)) => {
            home(cmd, &dir);
            true
        }
        _ => false,
    }
}

/// agy told to live in `dir`: its HOME. The keychain entry with the user's own sign-in is found through HOME alone
/// (measured: with another HOME it is not found and cannot be written, whatever else the environment says); the
/// keychain agy finds there instead is the account's own (`keychain_open`). agy's data folder, where it keeps a
/// sign-in it could not put into a keychain, is `<HOME>/.gemini/antigravity-cli` whatever it inherits (measured). The variables that name other folders for agy, and the one that can name another home
/// to the system's own libraries, are taken out all the same: nothing of a second account is ever to resolve to
/// the first's.
///
/// And the keys that could stand in for a sign-in (`KEPT_FROM_SECOND`): agy lists "Gemini API key" and "Application
/// Default Credentials" among its ways to sign in, so a second home is only ever signed in by the sign-in the user
/// made in it, never by a key that happens to be in the app's environment (which would be billed per use). The
/// app's own tool server gets the Gemini keys back in its own entry (its textures may use them).
pub fn home(cmd: &mut std::process::Command, dir: &Path) {
    cmd.env("HOME", dir);
    for k in ["CFFIXED_USER_HOME", "ANTIGRAVITY_APP_DATA_DIR", "ANTIGRAVITY_EXECUTABLE_DATA_DIR", "ANTIGRAVITY_CONFIG_REL_DIR"].into_iter().chain(KEPT_FROM_SECOND) {
        cmd.env_remove(k);
    }
}

/// The keys kept from every agy command of Gemini's second account (`home`).
pub const KEPT_FROM_SECOND: [&str; 3] = ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS"];

/// Claude Code told to keep everything of an account in `dir`: its config folder, and the folder its sign-in is
/// stored under. Claude Code names the keychain entry (and finds the plain credentials file) by
/// `CLAUDE_SECURESTORAGE_CONFIG_DIR` when that is set, over `CLAUDE_CONFIG_DIR` (read in the program's text,
/// 2.1.319; it sets the variable for sessions it starts itself): inherited from the app's environment it would send
/// the second account's sign-in to another folder's entry, or, empty, to the user's own. So both are always set, to
/// the same folder, for every command of a second account, and never for the first.
pub fn point(cmd: &mut std::process::Command, dir: &Path) {
    cmd.env("CLAUDE_CONFIG_DIR", dir);
    cmd.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", dir);
}

// ------------------------------------------------------------------ used up

/// Accounts that hit their usage limit, with when each is back (seconds since 1970). In memory: after a restart the
/// first turn finds out again.
static USED: Mutex<Option<HashMap<(String, String), f64>>> = Mutex::new(None);

/// When nothing says when a limit resets, the account is tried again after this long.
pub const TRY_AGAIN: f64 = 3600.0;

/// What an account is marked by: its name, so the mark stays with it when an account before it is removed ("" for
/// the user's own).
fn used_key(ai: &str, account: Option<&str>) -> (String, String) {
    (ai.to_string(), account.unwrap_or_default().to_string())
}

/// An account hit its usage limit: used up until `until`. By its name (None: the user's own).
pub fn mark_used_of(ai: &str, account: Option<&str>, until: f64) {
    let key = used_key(ai, account);
    #[cfg(test)]
    if STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.used.lock().unwrap().insert(key.clone(), until)).is_some()) {
        return;
    }
    USED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, until);
}

/// When the account is back, if it is used up now. By its name (None: the user's own).
pub fn used_until_of(ai: &str, account: Option<&str>, now: f64) -> Option<f64> {
    let key = used_key(ai, account);
    #[cfg(test)]
    if let Some(until) = STAND_IN.with(|s| s.borrow().as_ref().map(|s| s.used.lock().unwrap().get(&key).copied())) {
        return until.filter(|until| *until > now);
    }
    USED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key).copied()).filter(|until| *until > now)
}

/// Tests: the same by the account's place today.
#[cfg(test)]
pub fn mark_used(ai: &str, account: u8, until: f64) {
    if account < 2 {
        mark_used_of(ai, None, until);
    } else if let Some(name) = name_at(ai, account) {
        mark_used_of(ai, Some(&name), until);
    }
}

#[cfg(test)]
pub fn used_until(ai: &str, account: u8, now: f64) -> Option<f64> {
    if account < 2 { used_until_of(ai, None, now) } else { used_until_of(ai, Some(&name_at(ai, account)?), now) }
}

/// Which of an AI's accounts can take a chat now, in their order: first the user's own (not used up), then each
/// added one of `added` (its folder is one the app uses, an account of its own is signed in there, and it is not
/// used up). `added` is the list as the caller read it (`list`), once: what it then does with a place it reads
/// from that same list. The order is the list's: whoever wants another order reorders that list, nothing here.
pub fn slots(ai: &str, added: &[String], now: f64) -> Vec<bool> {
    let mut out = vec![used_until_of(ai, None, now).is_none()];
    for name in added {
        out.push(usable_of(ai, name) && signed_in_of(ai, name) && used_until_of(ai, Some(name), now).is_none());
    }
    out
}

/// The account a new chat or a new expert starts on: the first that can take a chat, the user's own first whenever
/// it is not used up. None can: the user's own (it fails as it would with one account).
pub fn pick(slots: &[bool]) -> u8 {
    slots.iter().position(|usable| *usable).and_then(|i| u8::try_from(i + 1).ok()).unwrap_or(1)
}

/// The account a chat moves to when `from` has just hit its limit: the first in the order that can take a chat,
/// other than `from` and other than any the message was already sent on (`tried`), so a message is sent once per
/// account at most and never comes back to one. None: stay (all are used up, signed out, or tried).
pub fn next(slots: &[bool], from: u8, tried: &[u8]) -> Option<u8> {
    (1..=u8::try_from(slots.len()).unwrap_or(u8::MAX)).find(|n| slots[*n as usize - 1] && *n != from && !tried.contains(n))
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
    // "resets at 9am on Nov 28": a day follows, so it is no time of today or tomorrow
    let after = low[c.get(0)?.end()..].trim_start();
    if after.starts_with("on ") || after.starts_with(", ") {
        return None;
    }
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

/// For the page, by AI: whether more accounts are offered; when the user's own is back if it is used up (`used`);
/// each added account in its order (`accounts`: its place `n`, the `id` of its sign-in, `broken` when its folder
/// is not one the app will use (swapped for a link, a keychain in it that is not its own, or its own keychain
/// could not be opened), `changed` when that is because its keychain was changed from outside the app
/// (`keychain_changed`), `used` as above); and the
/// folders of removed accounts that could not be deleted (`left`, with their places).
pub fn state() -> Value {
    let now = paths::now_secs();
    let mut out = Map::new();
    for ai in AIS {
        let accounts: Vec<Value> = list(ai)
            .iter()
            .enumerate()
            .map(|(i, name)| json!({"n": place(i), "name": name, "id": id_of(ai, name), "broken": !usable_of(ai, name), "changed": keychain_changed(ai, name), "used": used_until_of(ai, Some(name), now)}))
            .collect();
        let left: Vec<String> = left(ai).iter().map(|d| d.display().to_string()).collect();
        out.insert(ai.to_string(), json!({"offered": !cfg!(windows), "used": used_until_of(ai, None, now), "accounts": accounts, "left": left, "most": MOST}));
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
        for bad in ["", "claude-2-", "claude-2-abc", "claude-2-../../x", "claude-2-ABCDEF", "../claude-2-abcdef", "/tmp/claude-2-abcdef", "claude-2-abcdef/x", "gemini-2-abcdef", "cursor-2-abcdef", &format!("claude-2-{}", "a".repeat(17))] {
            assert!(!name_ok("claude", bad), "{bad}");
        }
        assert!(name_ok("gemini", "gemini-2-abcdef") && !name_ok("gemini", "claude-2-abcdef"), "each AI's own names only");
        assert!(!name_ok("cursor", "cursor-2-abcdef") && !name_ok("opencode", "opencode-2-abcdef"), "no second account for the others");
        let settings = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(list_in(&settings(json!({"second_accounts": {"claude": "claude-2-abcdef"}})), "claude"), ["claude-2-abcdef"]);
        for bad in [json!({"second_accounts": {"claude": "../../Documents"}}), json!({"second_accounts": {"claude": "/etc"}}), json!({"second_accounts": "claude-2-abcdef"}), json!({})] {
            assert!(list_in(&settings(bad.clone()), "claude").is_empty(), "{bad}");
        }
        // as a guarded setting: what could not be the app's own is dropped when the file is read
        assert_eq!(checked_setting(&json!({"claude": "claude-2-abcdef", "gemini": "gemini-2-abcdef", "cursor": "cursor-2-abcdef", "x": "/tmp"})), Some(json!({"claude": ["claude-2-abcdef"], "gemini": ["gemini-2-abcdef"]})));
        assert_eq!(checked_setting(&json!({"claude": "gemini-2-abcdef", "gemini": "../x"})), Some(json!({"claude": [], "gemini": []})));
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
    fn a_new_chat_starts_on_the_first_account_that_can_take_one_and_a_chat_moves_to_the_next_once() {
        // which accounts can take a chat, in their order: the user's own, then the added ones
        assert_eq!(pick(&[true]), 1, "no added account, nothing used up");
        assert_eq!(pick(&[true, true, true]), 1, "the user's own first, whenever it is available");
        assert_eq!(pick(&[false, true, true]), 2);
        assert_eq!(pick(&[false, false, true]), 3, "a used-up or signed-out one is passed over");
        assert_eq!(pick(&[false, false, false]), 1, "all used up: the user's own, as with one account");
        assert_eq!(pick(&[false]), 1);
        // a chat whose account just ran out moves to the first in the order that can take it
        assert_eq!(next(&[false, true, true], 1, &[]), Some(2));
        assert_eq!(next(&[false, false, true], 2, &[1]), Some(3), "then to the one after, when that one ran out too");
        assert_eq!(next(&[false, false, true], 1, &[]), Some(3), "passing over one that is used up, signed out or not usable");
        assert_eq!(next(&[true, false, true], 2, &[]), Some(1), "back to the user's own once it has reset: it is first in the order");
        // never to the one that just ran out, never to one the message was already on, and nowhere when none is left
        assert_eq!(next(&[true, true, true], 2, &[1]), Some(3), "not back to the first: the message ran out there already");
        assert_eq!(next(&[true, true, true], 3, &[1, 2]), None, "it has been on every account: it stays, and ends as with one");
        assert_eq!(next(&[false, false, false], 1, &[]), None, "all used up");
        assert_eq!(next(&[true], 1, &[]), None, "no added account");
        assert_eq!(next(&[false, true], 2, &[]), None, "the same account is no other one");
    }

    #[test]
    #[cfg_attr(windows, ignore = "second accounts are not offered on Windows yet (accounts::add)")]
    fn what_is_done_for_an_account_goes_by_its_name_and_the_list_is_never_longer_than_the_app_keeps() {
        // a command is pointed at the account that is named, wherever it stands: the third's folder for the third
        stand_in_many(&[("b", true), ("c", true), ("d", true)]);
        let get = |cmd: &std::process::Command, key: &str| cmd.get_envs().find(|(k, _)| *k == key).and_then(|(_, v)| v.map(|v| v.to_string_lossy().to_string()));
        let folder = |name: &str| base().join(name).to_string_lossy().to_string();
        let mut third = std::process::Command::new("claude");
        assert!(env_of(&mut third, "claude", Some("claude-2-d")));
        assert_eq!((get(&third, "CLAUDE_CONFIG_DIR"), get(&third, "CLAUDE_SECURESTORAGE_CONFIG_DIR")), (Some(folder("claude-2-d")), Some(folder("claude-2-d"))));
        let mut agy = std::process::Command::new("agy");
        assert!(env_of(&mut agy, "gemini", Some("gemini-2-c")));
        assert_eq!(get(&agy, "HOME"), Some(folder("gemini-2-c")));
        // a mark stays with its account when one BEFORE it is removed: "d" is used up at place 4, and still at place 3
        let now = 1_000.0;
        mark_used_of("claude", Some("claude-2-d"), now + 60.0);
        assert_eq!((used_until("claude", 4, now), used_until("claude", 3, now)), (Some(now + 60.0), None));
        let mut fewer = stand_in_now().unwrap();
        fewer.extras.remove(0);
        stand_in_here(&fewer);
        assert_eq!(list("claude"), ["claude-2-c", "claude-2-d"]);
        assert_eq!((used_until("claude", 3, now), used_until("claude", 2, now)), (Some(now + 60.0), None), "the mark moved up with it, and \"c\" did not inherit it");
        assert_eq!(slots("claude", &list("claude"), now), [true, true, false]);
        // the same account, asked by a name that is no longer in the list: not run at all
        assert!(!env_of(&mut std::process::Command::new("claude"), "claude", Some("claude-2-b")));
        // the list in the setting is never longer than the app keeps, whoever wrote it: 25 well-formed names read as 20
        let many: Vec<String> = (0..25).map(|i| format!("claude-2-{i:06}")).collect();
        assert_eq!(names_in("claude", &json!(many)).len(), MOST);
        assert_eq!(names_in("claude", &json!(many))[..], many[..MOST], "the first twenty, in their order");
        assert_eq!(checked_setting(&json!({"claude": many})).unwrap()["claude"].as_array().map(Vec::len), Some(MOST));
        let settings = json!({"second_accounts": {"gemini": (0..300).map(|i| format!("gemini-2-{i:06}")).collect::<Vec<_>>()}});
        assert_eq!(list_in(settings.as_object().unwrap(), "gemini").len(), MOST, "three hundred names start no three hundred status commands");
        // and a place is a count that cannot wrap
        assert_eq!((place(0), place(18), place(253), place(254), place(100_000)), (2, 20, 255, 255, 255));
        assert_eq!(pick(&[false; 300]), 1);
        assert_eq!(next(&[true; 300], 1, &[]), Some(2));
    }

    #[test]
    fn an_ai_has_any_number_of_added_accounts_in_the_order_they_were_added() {
        // the setting: a list per AI; one name alone (as it was while there could be one) reads as a list of one
        let settings = |v: Value| v.as_object().unwrap().clone();
        let three = settings(json!({"second_accounts": {"claude": ["claude-2-aaaaaa", "claude-2-bbbbbb", "claude-2-cccccc"], "gemini": "gemini-2-dddddd"}}));
        assert_eq!(list_in(&three, "claude"), ["claude-2-aaaaaa", "claude-2-bbbbbb", "claude-2-cccccc"]);
        assert_eq!(list_in(&three, "gemini"), ["gemini-2-dddddd"]);
        assert!(list_in(&three, "cursor").is_empty() && list_in(&settings(json!({})), "claude").is_empty());
        // names the app could not have made, another AI's names and repeats are dropped; the order stays
        let messy = json!({"claude": ["claude-2-aaaaaa", "../../Documents", "gemini-2-dddddd", "claude-2-aaaaaa", 7, "claude-2-bbbbbb"], "gemini": "/etc", "cursor": ["cursor-2-aaaaaa"]});
        assert_eq!(checked_setting(&messy), Some(json!({"claude": ["claude-2-aaaaaa", "claude-2-bbbbbb"], "gemini": []})));
        // the stand-in: three added accounts, by place, name and sign-in
        stand_in_many(&[("first", true), ("second", false), ("third", true)]);
        assert_eq!(list("claude"), ["claude-2-first", "claude-2-second", "claude-2-third"]);
        assert_eq!((name_at("claude", 1), name_at("claude", 2), name_at("claude", 4), name_at("claude", 5)), (None, Some("claude-2-first".to_string()), Some("claude-2-third".to_string()), None));
        assert_eq!((position("claude", "claude-2-third"), position("claude", "claude-2-gone")), (Some(4), None));
        assert_eq!(id_of("gemini", "gemini-2-second"), "gemini:gemini-2-second");
        assert_eq!(ids().len(), 6, "each AI's three");
        assert!(folder_of("claude", "claude-2-gone").is_none() && folder_of("claude", "gemini-2-first").is_none());
        // who can take a chat: the second is not signed in; a used-up one is passed over until its time
        let now = 1_000.0;
        assert_eq!(slots("claude", &list("claude"), now), [true, true, false, true]);
        mark_used("claude", 1, now + 60.0);
        mark_used("claude", 2, now + 60.0);
        assert_eq!(slots("claude", &list("claude"), now), [false, false, false, true]);
        assert_eq!((pick(&slots("claude", &list("claude"), now)), pick(&slots("claude", &list("claude"), now + 61.0))), (4, 1), "the user's own again once its time has passed");
        assert_eq!(slots("gemini", &list("gemini"), now), [true, true, false, true], "each AI's accounts are its own");
        // a mark stays with its account when one before it is removed: it is kept by name
        assert_eq!(used_until("claude", 2, now), Some(now + 60.0));
        let mut fewer = stand_in_now().unwrap();
        fewer.extras.remove(1);
        stand_in_here(&fewer);
        assert_eq!(list("claude"), ["claude-2-first", "claude-2-third"]);
        assert_eq!((used_until("claude", 2, now), used_until("claude", 3, now)), (Some(now + 60.0), None), "the third moved up to the third place, unmarked as it was");
        // the page is told each one, in order
        let st = state();
        let rows = st["claude"]["accounts"].as_array().unwrap();
        assert_eq!(rows.iter().map(|r| (r["n"].as_u64().unwrap(), r["id"].as_str().unwrap().to_string())).collect::<Vec<_>>(), [(2, "claude:claude-2-first".to_string()), (3, "claude:claude-2-third".to_string())]);
        assert!(rows.iter().all(|r| r["broken"] == false), "{rows:?}");
        stand_in(false, false);
        assert!(list("claude").is_empty() && !any("claude") && slots("claude", &list("claude"), now) == [true] && state()["claude"]["accounts"].as_array().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_accounts_folder_swapped_for_a_link_is_never_followed() {
        let root = paths::test_dir("accounts-swapped");
        let _ = fs::remove_dir_all(&root);
        // the user's own Claude folder, with a file and two links of its own (dotfiles kept in a repository)
        let (users, repo, base) = (root.join("users"), root.join("dotfiles"), root.join("accounts"));
        fs::create_dir_all(users.join("projects")).unwrap();
        fs::create_dir_all(repo.join("skills")).unwrap();
        fs::write(repo.join("settings.json"), "{}").unwrap();
        fs::write(users.join(".credentials.json"), "the user's own sign-in").unwrap();
        std::os::unix::fs::symlink(repo.join("skills"), users.join("skills")).unwrap();
        std::os::unix::fs::symlink(repo.join("settings.json"), users.join("settings.json")).unwrap();
        fs::create_dir_all(&base).unwrap();
        // the second account's folder has been replaced by a link to the user's own folder
        let dir = base.join("claude-2-abcdef");
        std::os::unix::fs::symlink(&users, &dir).unwrap();
        // nothing is prepared, so nothing runs there: no sign-in, no status, no turn
        assert_eq!(prepare_in(&dir, &users, &CLAUDE_LINKED), None);
        let listed = |d: &Path| {
            let mut names: Vec<String> = fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            names.sort();
            names
        };
        assert_eq!(listed(&users), [".credentials.json", "projects", "settings.json", "skills"], "nothing was linked into the user's folder");
        // Remove takes the link away and nothing of what it led to, not even that folder's own links
        remove_folder(&dir);
        assert!(fs::symlink_metadata(&dir).is_err(), "the link is gone");
        assert_eq!(listed(&users), [".credentials.json", "projects", "settings.json", "skills"]);
        assert!(fs::symlink_metadata(users.join("skills")).unwrap().file_type().is_symlink() && repo.join("settings.json").exists());
        assert_eq!(fs::read_to_string(users.join(".credentials.json")).unwrap(), "the user's own sign-in");
        // the folder of all second accounts swapped for a link: no account in it is used either
        let linked_base = root.join("accounts-link");
        std::os::unix::fs::symlink(&users, &linked_base).unwrap();
        assert_eq!(prepare_in(&linked_base.join("claude-2-abcdef"), &users, &CLAUDE_LINKED), None);
        assert!(!users.join("claude-2-abcdef").exists());
        // a folder of its own is prepared as before
        let real = base.join("claude-2-fedcba");
        assert_eq!(prepare_in(&real, &users, &CLAUDE_LINKED), Some(real.clone()));
        // Gemini's second home: the same checks, and only the git identity of the user's is linked in
        fs::write(users.join(".gitconfig"), "[user]\n").unwrap();
        fs::create_dir_all(users.join(".ssh")).unwrap();
        fs::create_dir_all(users.join(".gemini")).unwrap();
        let second_home = base.join("gemini-2-abcdef");
        assert_eq!(prepare_in(&second_home, &users, &GEMINI_LINKED), Some(second_home.clone()));
        assert_eq!(listed(&second_home), [".gitconfig"], "none of the user's keys, and nothing of their agy");
        assert!(fs::symlink_metadata(second_home.join(".gitconfig")).unwrap().file_type().is_symlink());
        let swapped = base.join("gemini-2-fedcba");
        std::os::unix::fs::symlink(&users, &swapped).unwrap();
        assert_eq!(prepare_in(&swapped, &users, &GEMINI_LINKED), None, "a home swapped for a link to the user's own is never used");
        remove_folder(&swapped);
        assert!(users.join(".gemini").exists() && fs::symlink_metadata(&swapped).is_err());
        assert!(fs::symlink_metadata(real.join("skills")).unwrap().file_type().is_symlink());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_gemini_homes_keychain_is_its_own_or_the_home_is_not_used_for_anything() {
        let root = paths::test_dir("accounts-keychain");
        let _ = fs::remove_dir_all(&root);
        let (users, base) = (root.join("users"), root.join("accounts"));
        let users_keychains = users.join("Library").join("Keychains");
        fs::create_dir_all(&users_keychains).unwrap();
        fs::write(users_keychains.join("login.keychain-db"), "the user's own keychain").unwrap();
        let home = |name: &str| {
            let h = base.join(name);
            assert_eq!(prepare_in(&h, &users, &GEMINI_LINKED), Some(h.clone()));
            h
        };
        let keychains = |h: &Path| h.join("Library").join("Keychains");
        // a new home with no Library at all, and one where agy and a turn's commands have left their own things
        let clean = home("gemini-2-aaaaaa");
        assert!(keychain_its_own(&clean, &users));
        fs::create_dir_all(clean.join(".gemini").join("antigravity-cli")).unwrap();
        fs::create_dir_all(clean.join("Library").join("Caches")).unwrap();
        assert!(keychain_its_own(&clean, &users));
        // the app's own keychain in it, a regular file made there, with the files the Mac puts beside it, the list
        // file, and even a keychain file of its own under the Mac's usual name: that is what the home is for
        assert!(list_its_own(&clean));
        fs::write(keychains(&clean).join(KEYCHAIN), "its own keychain").unwrap();
        fs::write(keychains(&clean).join(".fl80D2F631"), "").unwrap();
        fs::write(keychains(&clean).join("login.keychain-db"), "another of its own").unwrap();
        assert!(keychain_its_own(&clean, &users), "the app's own freshly made keychain is allowed");
        // the user's keychain folder linked in (what brings their sign-in back for every command there): refused
        let linked = home("gemini-2-bbbbbb");
        fs::create_dir_all(linked.join("Library")).unwrap();
        std::os::unix::fs::symlink(&users_keychains, keychains(&linked)).unwrap();
        assert!(!keychain_its_own(&linked, &users));
        assert!(!list_its_own(&linked), "and nothing is written through the link");
        // a keychain folder of its own with a link in it: to the user's keychain under the app's name, under the
        // Mac's usual name, or under any name
        for name in [KEYCHAIN, "login.keychain-db", "fridae.keychain", "anything"] {
            let own = home("gemini-2-cccccc");
            fs::create_dir_all(keychains(&own)).unwrap();
            assert!(keychain_its_own(&own, &users), "an empty keychain folder of its own is nothing to refuse");
            std::os::unix::fs::symlink(users_keychains.join("login.keychain-db"), keychains(&own).join(name)).unwrap();
            assert!(!keychain_its_own(&own, &users), "{name} as a link");
            assert!(remove_folder(&own));
        }
        // the user's keychain there with no link at all: the same file under a second name
        let shared = home("gemini-2-ffffff");
        fs::create_dir_all(keychains(&shared)).unwrap();
        fs::hard_link(users_keychains.join("login.keychain-db"), keychains(&shared).join(KEYCHAIN)).unwrap();
        assert!(!keychain_its_own(&shared, &users), "a second name for the user's own keychain file");
        fs::remove_file(keychains(&shared).join(KEYCHAIN)).unwrap();
        fs::write(root.join("somewhere-else"), "x").unwrap();
        fs::hard_link(root.join("somewhere-else"), keychains(&shared).join(KEYCHAIN)).unwrap();
        assert!(!keychain_its_own(&shared, &users), "or for any file that is also somewhere else");
        // its whole Library a link to the user's; its Preferences a link; its list file a link
        let library = home("gemini-2-dddddd");
        std::os::unix::fs::symlink(users.join("Library"), library.join("Library")).unwrap();
        assert!(!keychain_its_own(&library, &users));
        assert!(!list_its_own(&library));
        assert!(!users.join("Library").join("Preferences").exists(), "nothing was made in the user's own Library");
        let preferences = home("gemini-2-eeeeee");
        fs::create_dir_all(preferences.join("Library")).unwrap();
        fs::create_dir_all(users.join("Preferences-of-the-user")).unwrap();
        std::os::unix::fs::symlink(users.join("Preferences-of-the-user"), preferences.join("Library").join("Preferences")).unwrap();
        assert!(!keychain_its_own(&preferences, &users));
        assert!(!list_its_own(&preferences));
        assert_eq!(fs::read_dir(users.join("Preferences-of-the-user")).unwrap().count(), 0, "nothing was written where the link leads");
        let listed = home("gemini-2-abcabc");
        fs::create_dir_all(listed.join("Library").join("Preferences")).unwrap();
        fs::write(users.join("the-users-list"), "the user's own list").unwrap();
        std::os::unix::fs::symlink(users.join("the-users-list"), listed.join("Library").join("Preferences").join(KEYCHAIN_LIST_FILE)).unwrap();
        assert!(!keychain_its_own(&listed, &users), "a list file that is a link");
        assert_eq!(fs::read_to_string(users.join("the-users-list")).unwrap(), "the user's own list");
        // a list file of its own that says something else (a turn's command named another keychain in it): it is
        // NOT put right. It stays as it was found, and the answer is no
        let rewritten = home("gemini-2-defdef");
        fs::create_dir_all(rewritten.join("Library").join("Preferences")).unwrap();
        let list = rewritten.join("Library").join("Preferences").join(KEYCHAIN_LIST_FILE);
        let other_words = format!("<plist><string>{}</string></plist>", users_keychains.join("login.keychain-db").display());
        fs::write(&list, &other_words).unwrap();
        assert_eq!(list_file(&rewritten), ListFile::Changed);
        assert!(!list_its_own(&rewritten), "a changed list file is never the app's own");
        assert_eq!(fs::read_to_string(&list).unwrap(), other_words, "and is left exactly as it was found");
        // the app's words with one more keychain named after them, or an empty file: changed all the same
        for words in [KEYCHAIN_LIST.replace("</array>\n</dict>", "<string>/Users/x/Library/Keychains/login.keychain-db</string></array>\n</dict>"), String::new()] {
            fs::write(&list, &words).unwrap();
            assert!(list_file(&rewritten) == ListFile::Changed && !list_its_own(&rewritten), "{words:?}");
        }
        // a list file that is MISSING (a new home, or one where it was deleted) is written, quietly, and whole
        fs::remove_file(&list).unwrap();
        assert_eq!(list_file(&rewritten), ListFile::Missing);
        assert!(list_its_own(&rewritten));
        assert_eq!(fs::read_to_string(&list).unwrap(), KEYCHAIN_LIST);
        assert!(real_file(&list) && real_folder(&keychains(&rewritten)));
        assert_eq!(fs::read_dir(list.parent().unwrap()).unwrap().count(), 1, "nothing is left beside it");
        // and with its whole Preferences folder gone
        fs::remove_dir_all(list.parent().unwrap()).unwrap();
        assert!(list_file(&rewritten) == ListFile::Missing && list_its_own(&rewritten) && list_file(&rewritten) == ListFile::Ours);
        // written whole and never over what is there
        assert!(!place_whole(&list, "other words", false), "a name that is taken is not written over");
        assert_eq!(fs::read_to_string(&list).unwrap(), KEYCHAIN_LIST);
        // Remove still takes every such home away, and nothing of the user's with it
        assert!(remove_folder(&linked) && remove_folder(&shared) && remove_folder(&library) && remove_folder(&preferences) && remove_folder(&listed) && remove_folder(&clean) && remove_folder(&rewritten));
        assert_eq!(fs::read_to_string(users_keychains.join("login.keychain-db")).unwrap(), "the user's own keychain");
        assert_eq!(fs::read_to_string(root.join("somewhere-else")).unwrap(), "x");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_homes_list_file_names_the_accounts_own_keychain_and_no_other() {
        // the one keychain searched and the default, both the account's own, named from the home: no path of the
        // user's, nothing of the Mac's usual login keychain, and the same words for every account
        assert_eq!(KEYCHAIN_LIST.matches("<string>~/Library/Keychains/fridae.keychain</string>").count(), 2);
        assert_eq!(KEYCHAIN_LIST.matches("DbName").count(), 2, "one keychain in the search list, one as the default");
        assert!(KEYCHAIN_LIST.contains("<key>DLDBSearchList</key>") && KEYCHAIN_LIST.contains("<key>DefaultKeychain</key>"));
        assert!(!KEYCHAIN_LIST.contains("login") && !KEYCHAIN_LIST.contains("/Users/") && !KEYCHAIN_LIST.contains(&paths::home_dir().to_string_lossy().to_string()));
        assert_eq!(KEYCHAIN_LIST.len(), 735, "byte for byte the file the Mac's own program wrote for this keychain");
        assert!(KEYCHAIN.starts_with("fridae.keychain") && KEYCHAIN != "login.keychain-db");
    }

    #[test]
    fn the_keychain_commands_are_the_accounts_alone_by_its_home_and_carry_no_password_on_the_command_line() {
        // (built and looked at; never run)
        let home = paths::test_dir("accounts-keychain-command").join("gemini-2-abcdef");
        let cmd = security_command(&home);
        assert_eq!(cmd.get_program(), "/usr/bin/security");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["-i"], "its commands come on its input, so no password is ever an argument");
        let get = |key: &str| cmd.get_envs().find(|(k, _)| *k == key).map(|(_, v)| v.map(|v| v.to_string_lossy().to_string()));
        assert_eq!(get("HOME"), Some(Some(home.to_string_lossy().to_string())), "the account's home, and so its keychain folder");
        assert_eq!(get("CFFIXED_USER_HOME"), Some(None), "nothing else that could name a home");
        assert_eq!(cmd.get_current_dir(), Some(home.as_path()));
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(create_feed(key), format!("create-keychain -p {key} fridae.keychain-db\n"));
        assert_eq!(unlock_feed(key), format!("unlock-keychain -p {key} fridae.keychain-db\n"));
        assert_eq!(settings_feed(), "set-keychain-settings fridae.keychain-db\n", "no time limit and not on sleep: no -l, -u or -t");
        assert!(SECURITY_LIMIT <= std::time::Duration::from_secs(5));
        for feed in [create_feed(key), unlock_feed(key), settings_feed()] {
            // one command to a run: the program ends with the result of the last line it read, and each of these
            // results is one the app needs
            assert_eq!(feed.lines().count(), 1, "{feed}");
            // the keychain by its bare name (found in the home's own keychain folder), never by a path; and nothing
            // that changes which keychains anyone searches, the user's own least of all
            assert!(!feed.contains('/') && !feed.contains('~') && !feed.contains("login"), "{feed}");
            for never in ["list-keychains", "default-keychain", "login-keychain", "delete-keychain", "add-generic-password", "find-generic-password", "delete-generic-password", "import", " -s", " -d "] {
                assert!(!feed.contains(never), "{never} in {feed}");
            }
            assert!(feed.lines().all(|l| l.starts_with("create-keychain -p ") || l.starts_with("unlock-keychain -p ") || l == "set-keychain-settings fridae.keychain-db"), "{feed}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_gemini_account_has_its_own_keychain_made_once_and_unlocked_before_every_command() {
        use std::os::unix::fs::PermissionsExt;
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        stand_in_many(&[("kkkkaaaa", true), ("kkkkbbbb", true)]);
        let (first, second) = (base().join("gemini-2-kkkkaaaa"), base().join("gemini-2-kkkkbbbb"));
        for h in [&first, &second] {
            let _ = fs::remove_dir_all(h);
        }
        let fed = |h: &Path| SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.iter().filter(|(home, _)| home == h).map(|(_, f)| f.clone()).collect::<Vec<_>>();
        let forget = |h: &Path| SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.retain(|(home, _)| home != h);
        forget(&first);
        forget(&second);
        // looking at the account (Setup's list does, over and over) runs nothing: it only keeps the home's list file
        assert_eq!(prepare_of("gemini", "gemini-2-kkkkaaaa"), Some(first.clone()));
        assert!(state()["gemini"]["accounts"].as_array().unwrap().iter().all(|a| a["broken"] == false));
        assert!(fed(&first).is_empty() && fed(&second).is_empty(), "no keychain command for a look");
        assert_eq!(fs::read_to_string(first.join("Library").join("Preferences").join(KEYCHAIN_LIST_FILE)).unwrap(), KEYCHAIN_LIST);
        for dir in [first.join("Library"), first.join("Library").join("Keychains"), first.join("Library").join("Preferences")] {
            assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700, "{dir:?} is the user's alone");
        }
        // the first agy command of the account: the keychain is made with a password of the app's own, then unlocked
        let mut agy = std::process::Command::new("agy");
        assert!(env_of(&mut agy, "gemini", Some("gemini-2-kkkkaaaa")));
        let key = fs::read_to_string(first.join(KEYCHAIN_KEY)).unwrap();
        assert!(key.len() == 48 && key.bytes().all(|b| b.is_ascii_hexdigit()), "a password of 48 hex digits");
        assert_eq!(fs::metadata(first.join(KEYCHAIN_KEY)).unwrap().permissions().mode() & 0o777, 0o600, "the user's alone to read");
        // made, in a run of its own; unlocked; and, unlocked, told never to lock by itself
        assert_eq!(fed(&first), [create_feed(&key), unlock_feed(&key), settings_feed()]);
        assert!(real_file(&first.join("Library").join("Keychains").join(KEYCHAIN)));
        assert_eq!(fs::read_dir(&first).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("fridae-new")).count(), 0, "the password was put in place whole");
        // every later one: unlocked again with the kept password and told again never to lock, never made again
        assert!(env_of(&mut std::process::Command::new("agy"), "gemini", Some("gemini-2-kkkkaaaa")));
        assert_eq!(ready_of("gemini", "gemini-2-kkkkaaaa"), Some(first.clone()));
        assert_eq!(fed(&first), [create_feed(&key), unlock_feed(&key), settings_feed(), unlock_feed(&key), settings_feed(), unlock_feed(&key), settings_feed()]);
        // each account has a keychain and a password of its own
        assert!(env_of(&mut std::process::Command::new("agy"), "gemini", Some("gemini-2-kkkkbbbb")));
        let other = fs::read_to_string(second.join(KEYCHAIN_KEY)).unwrap();
        assert!(other != key && fed(&second) == [create_feed(&other), unlock_feed(&other), settings_feed()]);
        // Claude's accounts and the user's own Gemini account have none of this
        stand_in_many(&[("kkkkaaaa", true)]);
        let claude = base().join("claude-2-kkkkaaaa");
        let _ = fs::remove_dir_all(&claude);
        forget(&claude);
        assert!(env_of(&mut std::process::Command::new("claude"), "claude", Some("claude-2-kkkkaaaa")));
        assert!(fed(&claude).is_empty() && !claude.join("Library").exists() && !claude.join(KEYCHAIN_KEY).exists());
        let before = SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.len();
        assert!(env_of(&mut std::process::Command::new("agy"), "gemini", None));
        assert_eq!(SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.len(), before, "nothing for the user's own account");
        // Remove takes the keychain and its password away with the home
        assert!(remove_folder(&first) && remove_folder(&second));
        let _ = fs::remove_dir_all(&claude);
        stand_in(false, false);
    }

    #[cfg(unix)]
    #[test]
    fn a_list_file_changed_from_outside_is_never_put_right_and_the_account_is_not_used_again() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        stand_in_many(&[("kkkklist", true), ("kkkkgone", true)]);
        let (home, other) = (base().join("gemini-2-kkkklist"), base().join("gemini-2-kkkkgone"));
        for h in [&home, &other] {
            let _ = fs::remove_dir_all(h);
            SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.retain(|(at, _)| at != h);
            KEYCHAIN_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).retain(|at| at != h);
        }
        let fed = |h: &Path| SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.iter().filter(|(at, _)| at == h).count();
        let row = |name: &str, key: &str| state()["gemini"]["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap()[key].clone();
        let run = |name: &str| env_of(&mut std::process::Command::new("agy"), "gemini", Some(name));
        let list_of = |h: &Path| h.join("Library").join("Preferences").join(KEYCHAIN_LIST_FILE);
        assert!(run("gemini-2-kkkklist") && run("gemini-2-kkkkgone"));
        assert!(!keychain_changed("gemini", "gemini-2-kkkklist") && row("gemini-2-kkkklist", "changed") == false);
        // a command run in the account's home names the user's keychain in the home's list (what `security
        // list-keychains -s` or `default-keychain -s` run there would write)
        let theirs = "<plist><dict><key>DefaultKeychain</key><string>/Users/someone/Library/Keychains/login.keychain-db</string></dict></plist>";
        fs::write(list_of(&home), theirs).unwrap();
        let asked = fed(&home);
        // found, and said: not usable, not put right, nothing run
        assert!(keychain_changed("gemini", "gemini-2-kkkklist"));
        assert_eq!(prepare_of("gemini", "gemini-2-kkkklist"), None);
        assert!(!run("gemini-2-kkkklist"), "no agy command for it");
        assert_eq!((row("gemini-2-kkkklist", "changed"), row("gemini-2-kkkklist", "broken")), (json!(true), json!(true)));
        assert_eq!(fs::read_to_string(list_of(&home)).unwrap(), theirs, "the file is left as it was found: nothing is repaired in silence");
        assert_eq!(fed(&home), asked, "and the keychain program is not run");
        assert_eq!(slots("gemini", &list("gemini"), paths::now_secs()), [true, false, true], "no chat goes there");
        // put back by whoever changed it: it stays not used while the app runs (agy may have used the other
        // keychain meanwhile, and the app cannot know)
        fs::write(list_of(&home), KEYCHAIN_LIST).unwrap();
        assert!(keychain_changed("gemini", "gemini-2-kkkklist") && !run("gemini-2-kkkklist") && fed(&home) == asked);
        // the other account, whose list file was only deleted: written again without a word, and used as before
        fs::remove_file(list_of(&other)).unwrap();
        assert!(!keychain_changed("gemini", "gemini-2-kkkkgone"));
        assert!(run("gemini-2-kkkkgone") && row("gemini-2-kkkkgone", "changed") == false && row("gemini-2-kkkkgone", "broken") == false);
        assert_eq!(fs::read_to_string(list_of(&other)).unwrap(), KEYCHAIN_LIST);
        // Claude's accounts and the user's own are never "changed"
        assert!(!keychain_changed("claude", "claude-2-kkkklist") && !keychain_changed("gemini", "gemini-2-nosuchone"));
        // Remove forgets it with the folder
        stand_in(false, false);
        for h in [&home, &other] {
            KEYCHAIN_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).retain(|at| at != h);
            assert!(remove_folder(h));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_keychain_that_cannot_be_opened_means_no_command_runs_and_nothing_is_asked_of_it() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        stand_in_many(&[("kkkkcccc", true), ("kkkkdddd", true)]);
        let (home, other) = (base().join("gemini-2-kkkkcccc"), base().join("gemini-2-kkkkdddd"));
        for h in [&home, &other] {
            let _ = fs::remove_dir_all(h);
            SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.retain(|(at, _)| at != h);
        }
        let fed = |h: &Path| SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.iter().filter(|(at, _)| at == h).count();
        let fails = |h: &Path, what: Option<&'static str>| {
            let mut g = SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner());
            g.1.retain(|(at, _)| at != h);
            if let Some(what) = what {
                g.1.push((h.to_path_buf(), what));
            }
        };
        let broken = |name: &str| state()["gemini"]["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap()["broken"] == true;
        let now = paths::now_secs();
        let run = |name: &str| env_of(&mut std::process::Command::new("agy"), "gemini", Some(name));
        // it cannot be made: no agy command, Setup is told, and a chat is not sent there
        fails(&home, Some("create"));
        assert!(!run("gemini-2-kkkkcccc"));
        assert!(broken("gemini-2-kkkkcccc") && !broken("gemini-2-kkkkdddd"));
        assert_eq!(slots("gemini", &list("gemini"), now), [true, false, true]);
        // made now, but it does not unlock: the same
        fails(&home, Some("unlock"));
        assert!(!run("gemini-2-kkkkcccc") && broken("gemini-2-kkkkcccc"));
        // it unlocks, but the "never lock by itself" setting does not take: the same (a keychain that may lock under
        // a turn would have the Mac ask for its password), and the setting is tried again every time, not only when
        // the keychain is made
        fails(&home, Some("settings"));
        let texts = |h: &Path| SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.iter().filter(|(at, _)| at == h).map(|(_, f)| f.split(' ').next().unwrap_or_default().to_string()).collect::<Vec<_>>();
        let before = texts(&home).len();
        assert!(!run("gemini-2-kkkkcccc") && broken("gemini-2-kkkkcccc"));
        assert_eq!(texts(&home)[before..], ["unlock-keychain", "set-keychain-settings"], "the keychain is there already: not made again, its settings asked for again");
        let before = texts(&home).len();
        assert!(!run("gemini-2-kkkkcccc"));
        assert_eq!(texts(&home)[before..], ["unlock-keychain", "set-keychain-settings"], "and again the next time");
        // it opens again: used again
        fails(&home, None);
        let before = texts(&home).len();
        assert!(run("gemini-2-kkkkcccc") && !broken("gemini-2-kkkkcccc"));
        assert_eq!(texts(&home)[before..], ["unlock-keychain", "set-keychain-settings"]);
        assert_eq!(slots("gemini", &list("gemini"), now), [true, true, true]);
        // Stop while it is being opened: nothing is run to its end, and it is not held against the account
        fails(&home, None);
        assert_eq!(ready_of_until("gemini", "gemini-2-kkkkcccc", &|| true), None);
        assert!(!broken("gemini-2-kkkkcccc"), "a stopped try is no failure of the keychain");
        assert!(run("gemini-2-kkkkcccc"));
        // the program is never run for a home that is not an account's folder: an empty or a relative path would
        // let it fall back to the user's real home
        let all = SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.len();
        for not_a_home in [PathBuf::new(), PathBuf::from("gemini-2-kkkkcccc"), PathBuf::from("."), paths::home_dir(), base(), base().join("gemini-2-kkkkcccc").join("deeper"), paths::test_dir("accounts-elsewhere").join("gemini-2-kkkkcccc")] {
            assert!(!in_accounts(&not_a_home), "{not_a_home:?}");
            assert!(!security(&not_a_home, settings_feed(), &|| false) && !keychain_open_in(&not_a_home, &|| false), "{not_a_home:?}");
        }
        assert_eq!(SECURITY_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).0.len(), all, "nothing was fed for any of them");
        assert!(in_accounts(&home));
        // its password is gone: the keychain is asked nothing at all (with no password it would ask on the screen),
        // and none is made over it
        let asked = fed(&home);
        let key = fs::read_to_string(home.join(KEYCHAIN_KEY)).unwrap();
        fs::remove_file(home.join(KEYCHAIN_KEY)).unwrap();
        assert!(!run("gemini-2-kkkkcccc") && broken("gemini-2-kkkkcccc"));
        assert_eq!(fed(&home), asked, "nothing was run");
        assert!(!home.join(KEYCHAIN_KEY).exists(), "no new password for a keychain that has another");
        // a password file that is not a password of the app's (it would be put into the program's input): not used
        for bad in ["wrong", "0123456789abcdef0123456789abcdef0123456789abcdeX", &format!("{key}\nlist-keychains -s x"), &format!("{key} {key}")] {
            fs::write(home.join(KEYCHAIN_KEY), bad).unwrap();
            assert!(!run("gemini-2-kkkkcccc"), "{bad:?}");
            assert_eq!(fed(&home), asked, "nothing was run for {bad:?}");
        }
        // or one that is a link
        fs::remove_file(home.join(KEYCHAIN_KEY)).unwrap();
        fs::write(base().join("a-password-elsewhere"), &key).unwrap();
        std::os::unix::fs::symlink(base().join("a-password-elsewhere"), home.join(KEYCHAIN_KEY)).unwrap();
        assert!(!run("gemini-2-kkkkcccc") && fed(&home) == asked);
        // the right one back: it opens
        fs::remove_file(home.join(KEYCHAIN_KEY)).unwrap();
        fs::write(home.join(KEYCHAIN_KEY), &key).unwrap();
        assert!(run("gemini-2-kkkkcccc") && !broken("gemini-2-kkkkcccc"));
        // the keychain swapped for a link to the user's own after it was made: the home is not used, nothing is run
        let asked = fed(&home);
        let keychain = home.join("Library").join("Keychains").join(KEYCHAIN);
        fs::remove_file(&keychain).unwrap();
        fs::write(base().join("the-users-keychain"), &key).unwrap();
        std::os::unix::fs::symlink(base().join("the-users-keychain"), &keychain).unwrap();
        assert!(!run("gemini-2-kkkkcccc") && broken("gemini-2-kkkkcccc"));
        assert_eq!(fed(&home), asked, "a keychain that is a link is never unlocked, and none is made through it");
        assert_eq!(fs::read_to_string(base().join("the-users-keychain")).unwrap(), key);
        assert!(keychain_changed("gemini", "gemini-2-kkkkcccc") && !keychain_changed("gemini", "gemini-2-kkkkdddd"), "and Setup is told that it was changed from outside");
        KEYCHAIN_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).retain(|h| h != &home);
        for f in ["a-password-elsewhere", "the-users-keychain"] {
            let _ = fs::remove_file(base().join(f));
        }
        assert!(remove_folder(&home));
        let _ = fs::remove_dir_all(&other);
        stand_in(false, false);
    }

    #[cfg(unix)]
    #[test]
    fn removing_an_account_deletes_its_sign_in_first_and_says_when_the_folder_stays() {
        use std::os::unix::fs::PermissionsExt;
        let root = paths::test_dir("accounts-remove");
        let _ = fs::remove_dir_all(&root);
        // a home a turn's commands have written into: a folder left read-only (as a module cache is) holds a file
        let dir = root.join("gemini-2-abcdef");
        fs::create_dir_all(dir.join(".gemini").join("antigravity-cli")).unwrap();
        fs::write(dir.join(".gemini").join("antigravity-cli").join("the-sign-in"), "kept in a file").unwrap();
        fs::create_dir_all(dir.join("go").join("pkg")).unwrap();
        fs::write(dir.join("go").join("pkg").join("module"), "x").unwrap();
        fs::set_permissions(dir.join("go").join("pkg"), fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(dir.join("go"), fs::Permissions::from_mode(0o500)).unwrap();
        assert!(fs::remove_dir_all(dir.join("go")).is_err(), "as it is, it cannot be deleted");
        assert!(remove_folder(&dir), "made the user's to write, then deleted");
        assert!(fs::symlink_metadata(&dir).is_err());
        // something that cannot be deleted at all (a file marked unchangeable): the sign-in is gone all the same,
        // and the folder is reported, not passed over in silence
        #[cfg(target_os = "macos")]
        {
            let dir = root.join("claude-2-abcdef");
            fs::create_dir_all(dir.join("projects")).unwrap();
            fs::write(dir.join(".credentials.json"), "the sign-in").unwrap();
            fs::write(dir.join(".claude.json"), "the account").unwrap();
            fs::write(dir.join("projects").join("stuck"), "x").unwrap();
            let flag = |on: &str| {
                use crate::paths::Start;
                std::process::Command::new("/usr/bin/chflags").arg(on).arg(dir.join("projects").join("stuck")).start().and_then(|mut c| c.wait()).is_ok_and(|s| s.success())
            };
            if flag("uchg") {
                assert!(!remove_folder(&dir), "still there");
                assert!(!dir.join(".credentials.json").exists() && !dir.join(".claude.json").exists(), "the sign-in went first");
                note_left("claude", Some(dir.clone()));
                assert_eq!(left("claude"), std::slice::from_ref(&dir), "Setup is told where it is");
                assert!(flag("nouchg"));
                assert!(remove_folder(&dir));
                assert!(left("claude").is_empty(), "and no longer once it is gone");
                note_left("claude", None);
            }
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg_attr(windows, ignore = "second accounts are not offered on Windows yet (accounts::add)")]
    fn a_second_accounts_commands_keep_its_sign_in_in_its_own_folder_whatever_the_app_inherited() {
        stand_in(true, true);
        let get = |cmd: &std::process::Command, key: &str| cmd.get_envs().find(|(k, _)| *k == key).map(|(_, v)| v.map(|v| v.to_string_lossy().to_string()));
        // the second account: the config folder and the folder its sign-in is stored under are both its own, set
        // explicitly, so a `CLAUDE_SECURESTORAGE_CONFIG_DIR` the app inherited (empty: the user's own keychain
        // entry) can never decide
        let mut second = std::process::Command::new("claude");
        assert!(env_of(&mut second, "claude", Some("claude-2-standin")));
        let dir = base().join("claude-2-standin").to_string_lossy().to_string();
        assert_eq!(get(&second, "CLAUDE_CONFIG_DIR"), Some(Some(dir.clone())));
        assert_eq!(get(&second, "CLAUDE_SECURESTORAGE_CONFIG_DIR"), Some(Some(dir)));
        // the first account: nothing of its environment is set or removed by the app
        let mut first = std::process::Command::new("claude");
        assert!(env_of(&mut first, "claude", None));
        assert_eq!(first.get_envs().count(), 0);
        // Gemini's second account: its own HOME, and nothing that could name another home or another data folder
        let mut agy = std::process::Command::new("agy");
        assert!(env_of(&mut agy, "gemini", Some("gemini-2-standin")));
        let home_dir = base().join("gemini-2-standin").to_string_lossy().to_string();
        assert_eq!(get(&agy, "HOME"), Some(Some(home_dir)));
        for gone in ["CFFIXED_USER_HOME", "ANTIGRAVITY_APP_DATA_DIR", "ANTIGRAVITY_EXECUTABLE_DATA_DIR", "ANTIGRAVITY_CONFIG_REL_DIR"] {
            assert_eq!(get(&agy, gone), Some(None), "{gone} is taken out");
        }
        // nor a key that could stand in for a sign-in: only the sign-in made in that home signs it in
        for key in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS"] {
            assert_eq!(get(&agy, key), Some(None), "{key} is kept from the second account's agy");
        }
        let mut agy_first = std::process::Command::new("agy");
        assert!(env_of(&mut agy_first, "gemini", None));
        assert_eq!(agy_first.get_envs().count(), 0, "the user's own account: untouched");
        // no second account: the command is not to be run at all
        stand_in(false, false);
        assert!(!env_of(&mut std::process::Command::new("claude"), "claude", Some("claude-2-standin")));
        assert!(!env_of(&mut std::process::Command::new("agy"), "gemini", Some("gemini-2-standin")));
        // an AI that has none
        stand_in(true, true);
        assert!(!env_of(&mut std::process::Command::new("cursor-agent"), "cursor", Some("cursor-2-standin")));
    }

    #[test]
    fn used_up_lasts_until_the_reset_time_and_no_longer() {
        let _one = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        mark_used("claude", 1, 2_000.0);
        assert_eq!(used_until("claude", 1, 1_000.0), Some(2_000.0));
        assert_eq!(used_until("claude", 1, 2_000.0), None, "back at its reset time: new chats go to the user's own again");
        assert_eq!(used_until("claude", 2, 1_000.0), None);
        assert_eq!(slots("claude", &list("claude"), 1_000.0), [false], "the user's own is used up, and there is no added account in this test");
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
        for other in ["You've hit your weekly limit \u{b7} resets Nov 28 at 9am", "resets at 9am on Nov 28", "resets 9am, Nov 28", "resets 13pm", "resets 9:75am", "resets soon", "resets 9", "it presets 9am"] {
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
