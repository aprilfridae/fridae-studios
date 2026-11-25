//! Sharing the user's own skills with the vault: a batch is packed (by `rbx skills pack`) into the app's own
//! `outbox` folder, every file is masked (`compact::mask_for_sharing`), the user sees exactly those files, and on
//! their Send, or under their standing choice (Send automatically, at most once a day), those bytes become a branch
//! of the user's fork of the vault and a pull request. Nothing here is reachable by an agent: the routes in
//! `server.rs` are the user's page's, and no tool of the MCP server calls them. The token stays in `github` and
//! `vault_git`; nothing here reads, stores, logs or returns it.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde_json::{Map, Value, json};

use crate::{compact, github, paths, vault_git, vault_sync};

/// The settings: `share_skills` is "off", "review" or "auto" (missing: not asked yet, which counts as off), and
/// it is a guarded setting (an agent's file edit cannot turn it on); the others are seconds and a note.
const MODE_KEY: &str = "share_skills";
const AGREED_KEY: &str = "share_agreed_at";
const LAST_KEY: &str = "share_last_sent";
const HELD_KEY: &str = "share_held";
const OFF: &str = "Turn on sharing first.";
const GONE: &str = "This batch is not there any more. Press Share again to make a new one.";
const BUSY: &str = "Fridae is already getting a batch ready to send. Wait a moment.";
const NO_ROOM: &str = "Fridae could not save what it was about to send in its own folder.";
const NOT_RUN: &str = "Fridae could not get your skills and guides ready to send.";
const NO_LOGIN: &str = "GitHub did not say who is signed in. Disconnect GitHub in Setup, Connections, and connect again.";
const AGREE: &str = "Agree to share your skills and guides under CC BY 4.0 (text) and MIT (scripts) first.";
const HELD_WHY: &str = "Fridae hid something in a script; check it in Setup › Skills before it is sent.";
const LICENCE_LINE: &str = "I made these skills and share them under CC BY 4.0 (text) and MIT (scripts).";
const CHANGES_LINE: &str = "I made these changes and share them under CC BY 4.0 (text) and MIT (scripts).";
/// At most this many skills in a batch.
const BATCH_MAX: usize = 20;
/// One automatic batch in this long.
const DAY: u64 = 24 * 60 * 60;
/// An outbox folder nobody sent is removed after this long.
const KEEP: Duration = Duration::from_secs(7 * DAY);
/// The kit's program is given this long for one call.
const CLI_LIMIT: Duration = Duration::from_secs(300);
/// Files that are scripts by their ending (the others by their folder, `scripts/`, or a `#!` first line).
const SCRIPT_EXT: [&str; 12] = ["py", "sh", "bash", "zsh", "js", "mjs", "ts", "rb", "lua", "applescript", "command", "ps1"];

// ------------------------------------------------------------------------------------------------ small helpers
static RUNNING: Mutex<bool> = Mutex::new(false);

/// The one place a batch is made or sent from; held until dropped.
struct Turn;

impl Turn {
    fn take() -> Option<Turn> {
        let mut g = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if *g {
            return None;
        }
        *g = true;
        Some(Turn)
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }
}

/// The branch a batch is sent on: `skills-<login>-<YYYYMMDD-HHMM>-<4 hex>`. The stamp is cut only when it is long enough
/// (it is plain seconds where the clock cannot be read), and the random end keeps two sends in one minute apart.
fn branch_name(login: &str, stamp: &str) -> String {
    let stamp = stamp.replace('_', "-").to_lowercase();
    format!("skills-{login}-{}-{}", stamp.get(..13).unwrap_or(&stamp), &paths::random_hex(1)[..4])
}

fn fail(why: impl Into<String>) -> Value {
    json!({"ok": false, "error": why.into()})
}

/// "off", "review" or "auto" (anything else, and nothing, is off).
fn mode() -> &'static str {
    match paths::setting(MODE_KEY).as_str() {
        Some("review") => "review",
        Some("auto") => "auto",
        _ => "off",
    }
}

fn config_num(key: &str) -> Option<u64> {
    paths::load_config().get(key).and_then(Value::as_u64)
}

fn login() -> Option<String> {
    github::user()["login"].as_str().filter(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).map(str::to_string)
}

fn outbox() -> PathBuf {
    vault_sync::vault_app().join("outbox")
}

/// A skill's folder name for its name (`rbxskills._safe`).
fn safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 32 { '-' } else { c }).collect();
    let s = s.trim_matches([' ', '.']);
    if s.is_empty() { "skill".to_string() } else { s.to_string() }
}

/// A file written through a temporary one and a rename, never through a link.
fn put(path: &Path, bytes: &[u8]) -> Result<(), String> {
    vault_sync::write_atomic(path, bytes).map_err(|_| NO_ROOM.to_string())
}

// ----------------------------------------------------------------------------------------- the kit's program
/// The command that runs `rbx` with these arguments. In tests, the kit's own code with a scratch home folder.
fn command(args: &[String]) -> std::process::Command {
    #[cfg(test)]
    {
        const WRAP: &str = "import sys, os\nkit, home = sys.argv[1:3]\nsys.path.insert(0, kit)\nimport rbxpaths\nrbxpaths.HOME_DIR = home\nrbxpaths.KNOWLEDGE_DIR = os.path.join(home, 'Knowledge')\nrbxpaths.CONFIG = os.path.join(home, 'config.json')\nimport rbxskills\nrbxskills.HOME = home\nimport rbx\nrbx.main(sys.argv[3:])\n";
        let home = paths::test_dir("vault-home");
        let mut c = std::process::Command::new(paths::python());
        c.args(["-c", WRAP]).arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("..")).arg(&home).args(args);
        c.env("PYTHONIOENCODING", "utf-8").env("FRIDAE_USER_CLAUDE", home.join("claude")).env("CLAUDE_CONFIG_DIR", "");
        paths::quiet(&mut c);
        return c;
    }
    #[allow(unreachable_code)]
    {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        paths::cli(&args)
    }
}

/// Run `rbx skills ...`: what it printed, or a plain sentence (its own last line when it gave one).
fn kit(args: &[&str]) -> Result<String, String> {
    use paths::Start;
    use std::process::Stdio;
    let mut all = vec!["skills".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    let mut cmd = command(&all);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.start().map_err(|_| NOT_RUN.to_string())?;
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let out = match rx.recv_timeout(CLI_LIMIT) {
        Ok(Ok(out)) => out,
        Ok(Err(_)) => return Err(NOT_RUN.to_string()),
        Err(_) => {
            paths::end_trees(&[id]);
            return Err(NOT_RUN.to_string());
        }
    };
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        let code = out.status.code();
        return Err(if said.is_empty() || !matches!(code, Some(1 | 2)) || said.contains("Traceback") { NOT_RUN.to_string() } else { said });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn kit_json(args: &[&str]) -> Result<Value, String> {
    serde_json::from_str(&kit(args)?).map_err(|_| NOT_RUN.to_string())
}

/// One unit a batch may hold: a skill of the user's library (`kind` "skill"), a skill of the vault they edited ("vault
/// skill", `unit` `Skills/<vault name>`) or a guide they changed or added ("guide", `unit` its path).
#[derive(Clone)]
struct Item {
    kind: &'static str,
    name: String,
    unit: String,
    hash: String,
    new: bool,
}

/// What a guide's or vault skill's kind is on the page, from `knowledge_changes`' "note" or "skill".
fn knowledge_kind(kind: &Value) -> &'static str {
    if kind == "skill" { "vault skill" } else { "guide" }
}

/// `rbx skills shareable`'s ready units: the library's skills by name, then what of Knowledge is ready, in the kit's
/// order. A vault skill whose name a library skill of this list has (any capitals) waits for a later batch, since both
/// would be sent at `Skills/<name>`.
fn items(v: &Value) -> Vec<Item> {
    let mut out: Vec<Item> = v["ready"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e["id"].as_str()?.to_string(), e["hash"].as_str().unwrap_or("").to_string())))
        // a name that begins with "-" would be read as an option by the program
        .filter(|(n, _)| !n.is_empty() && !n.starts_with('-'))
        .map(|(name, hash)| Item { kind: "skill", unit: format!("Skills/{}", safe(&name)), name, hash, new: false })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    let taken: Vec<String> = out.iter().map(|i| i.unit.to_lowercase()).collect();
    for e in v["knowledge"]["ready"].as_array().into_iter().flatten() {
        let (Some(unit), Some(hash)) = (e["unit"].as_str(), e["hash"].as_str()) else { continue };
        let kind = knowledge_kind(&e["kind"]);
        let path_ok = if kind == "guide" { !unit.starts_with("Skills/") && vault_sync::check_path(unit) } else { unit.matches('/').count() == 1 && vault_sync::check_path(&format!("{unit}/SKILL.md")) };
        if !path_ok || unit.starts_with('-') || taken.contains(&unit.to_lowercase()) {
            continue;
        }
        let name = e["title"].as_str().unwrap_or(unit).to_string();
        out.push(Item { kind, name, unit: unit.to_string(), hash: hash.to_string(), new: e["new"] == true });
    }
    out
}

fn ready() -> Result<Vec<Item>, String> {
    Ok(items(&kit_json(&["shareable"])?))
}

// ------------------------------------------------------------------------------------------------- the files
/// Whether a file is a script: under `scripts/`, one of the script endings, or beginning `#!`.
pub(crate) fn is_script(rel: &str, bytes: &[u8]) -> bool {
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    rel.starts_with("scripts/") || SCRIPT_EXT.contains(&ext.as_str()) || bytes.starts_with(b"#!")
}

/// The ordinary files under a folder, as ('/'-separated path, full path), sorted; links and dot-files left out.
fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut names: Vec<_> = read.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    for n in names.into_iter().filter(|n| !n.starts_with('.')) {
        let (path, shown) = (dir.join(&n), if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") });
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => walk(&path, &shown, out),
            Ok(m) if m.is_file() => out.push((shown, path)),
            _ => {}
        }
    }
}

/// A skill's description from its SKILL.md front matter (one line, at most 200 characters).
fn description(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return String::new();
    }
    let front: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let Some(at) = front.iter().position(|l| l.starts_with("description:")) else { return String::new() };
    let mut value = front[at]["description:".len()..].trim().to_string();
    if value.is_empty() || matches!(value.as_str(), ">" | "|" | ">-" | "|-" | ">+" | "|+") {
        let more: Vec<&str> = front[at + 1..].iter().take_while(|l| l.starts_with([' ', '\t'])).map(|l| l.trim()).collect();
        value = more.join(" ");
    }
    let value = value.trim_matches(['"', '\'']).split_whitespace().collect::<Vec<_>>().join(" ");
    value.chars().take(200).collect()
}

/// The batch's own record, kept beside its files: who sent it and each unit's kind, name, folder or path, and hash.
const META: &str = "batch.json";
/// The folder of a batch that holds what of Knowledge it sends, at the vault's paths.
const KNOWN: &str = "knowledge";

/// `base` joined with a '/'-separated path, part by part.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
}

fn batch_dir(batch: &str) -> Option<PathBuf> {
    let ok = !batch.is_empty() && batch.len() <= 40 && batch.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then(|| outbox().join(batch))
}

/// Outbox folders older than 7 days (by when the batch was made, else by the folder's own time).
fn clean_outbox() {
    let now = vault_sync::now();
    for e in fs::read_dir(outbox()).into_iter().flatten().flatten() {
        let made = fs::read(e.path().join(META)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["made"].as_u64());
        let made = made.or_else(|| e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()));
        if made.is_some_and(|m| now.saturating_sub(m) > KEEP.as_secs()) {
            vault_sync::remove(&e.path());
        }
    }
}

// ------------------------------------------------------------------------------------------------- the routes
/// Choose off, review (each batch on the user's Send) or auto. Review and auto need the agreement (the licence)
/// and a connected GitHub.
pub fn set_mode(mode: &str, agree: bool) -> Value {
    if !matches!(mode, "off" | "review" | "auto") {
        return fail("That is not a way to share.");
    }
    if mode != "off" {
        if !github::connected() {
            return fail(github::NOT_CONNECTED);
        }
        if !agree {
            return fail(AGREE);
        }
    }
    paths::update_config(|c: &mut Map<String, Value>| {
        c.insert(MODE_KEY.into(), json!(mode));
        if mode != "off" {
            c.insert(AGREED_KEY.into(), json!(vault_sync::now()));
        }
    });
    json!({"ok": true, "mode": mode})
}

/// `{mode, asked, agreed, connected, login, shareable, ready, held, sent, knowledge, batch?, last_sent, held_masked}`:
/// while sharing is on and GitHub connected, `rbx skills shareable` is run once for `ready: [{id, scripts}]` (a skill
/// held by the last automatic batch is held, not ready), `held: [{id, why}]` (the kit's, then `held_masked`, each name
/// once), `sent: [{id, pr, at}]` and `knowledge: {ready: [{unit, kind, title, new}], held: [{unit, title, why}], sent:
/// [{unit, title, pr, at}]}` (guides and vault skills); otherwise the lists are empty.
pub fn status() -> Value {
    let c = paths::load_config();
    let (mode, connected) = (mode(), github::connected());
    let held_masked = c.get(HELD_KEY).cloned().unwrap_or(json!([]));
    let listed = if mode != "off" && connected { kit_json(&["shareable"]).ok() } else { None };
    let list = |v: &Value, key: &str| v[key].as_array().cloned().unwrap_or_default();
    let (mut ready, mut held, mut sent, mut shareable) = (Vec::new(), Vec::new(), Vec::new(), 0);
    if let Some(v) = &listed {
        let mut seen = std::collections::HashSet::new();
        for h in list(v, "held").iter().chain(held_masked.as_array().into_iter().flatten()) {
            if let Some(id) = h["id"].as_str().filter(|id| seen.insert(id.to_string())) {
                held.push(json!({"id": id, "why": h["why"].as_str().unwrap_or("")}));
            }
        }
        // the same names `ready()` keeps
        let all: Vec<Value> = list(v, "ready").into_iter().filter(|r| r["id"].as_str().is_some_and(|n| !n.is_empty() && !n.starts_with('-'))).collect();
        shareable = all.len();
        ready = all.iter().filter(|r| !r["id"].as_str().is_some_and(|id| seen.contains(id))).map(|r| json!({"id": r["id"], "scripts": r["scripts"] == true})).collect();
        ready.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        sent = list(v, "sent").iter().map(|s| json!({"id": s["id"], "pr": s["pr"], "at": s["at"]})).collect();
    }
    // guides and vault skills: what of Knowledge is ready, held and sent
    let none = json!({});
    let k = listed.as_ref().map(|v| &v["knowledge"]).unwrap_or(&none);
    let klist = |key: &str| k[key].as_array().cloned().unwrap_or_default();
    let knowledge = json!({
        "ready": klist("ready").iter().map(|e| json!({"unit": e["unit"], "kind": knowledge_kind(&e["kind"]), "title": e["title"], "new": e["new"] == true})).collect::<Vec<_>>(),
        "held": klist("held").iter().map(|e| json!({"unit": e["unit"], "title": e["title"], "why": e["why"]})).collect::<Vec<_>>(),
        "sent": klist("sent").iter().map(|e| json!({"unit": e["unit"], "title": e["title"], "pr": e["pr"], "at": e["at"]})).collect::<Vec<_>>(),
    });
    let mut out = json!({
        "mode": mode,
        "asked": paths::setting(MODE_KEY).is_string(),
        "agreed": c.contains_key(AGREED_KEY),
        "connected": connected,
        "login": github::user()["login"].clone(),
        "shareable": shareable,
        "ready": ready,
        "held": held,
        "sent": sent,
        "knowledge": knowledge,
        "last_sent": c.get(LAST_KEY).cloned().unwrap_or(Value::Null),
        "held_masked": held_masked,
    });
    // a batch made and not sent yet (the newest)
    let newest = fs::read_dir(outbox()).into_iter().flatten().flatten().filter(|e| e.path().join(META).is_file()).max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    if let Some(e) = newest {
        out["batch"] = json!(e.file_name().to_string_lossy());
    }
    out
}

/// Pack up to 20 ready skills into a new outbox folder and mask every file; what the user sees is what is sent.
pub fn prepare() -> Value {
    if mode() == "off" {
        return fail(OFF);
    }
    let Some(_turn) = Turn::take() else { return fail(BUSY) };
    prepare_in().unwrap_or_else(fail)
}

fn prepare_in() -> Result<Value, String> {
    if !github::connected() {
        return Err(github::NOT_CONNECTED.to_string());
    }
    let login = login().ok_or_else(|| NO_LOGIN.to_string())?;
    clean_outbox();
    let mut batch: Vec<Item> = ready()?;
    batch.truncate(BATCH_MAX);
    if batch.is_empty() {
        return Ok(json!({"ok": true, "batch": Value::Null, "skills": []}));
    }
    let id = format!("{}-{}", paths::stamp().replace('_', "-").to_lowercase(), &paths::random_hex(1)[..8]);
    let dir = batch_dir(&id).ok_or_else(|| NOT_RUN.to_string())?;
    let made = (|| {
        vault_sync::real_dir(&outbox()).map_err(|_| NO_ROOM.to_string())?;
        vault_sync::real_dir(&dir).map_err(|_| NO_ROOM.to_string())?;
        // the library's skills under Skills/, what of Knowledge goes under knowledge/ at its vault path
        let (library, known): (Vec<&Item>, Vec<&Item>) = batch.iter().partition(|i| i.kind == "skill");
        let mut hashes = Map::new();
        if !library.is_empty() {
            let out = dir.display().to_string();
            let mut args = vec!["pack", "--out", out.as_str(), "--login", login.as_str()];
            args.extend(library.iter().map(|i| i.name.as_str()));
            hashes.extend(kit_json(&args)?.as_object().cloned().unwrap_or_default());
        }
        if !known.is_empty() {
            let kdir = dir.join(KNOWN);
            vault_sync::real_dir(&kdir).map_err(|_| NO_ROOM.to_string())?;
            let out = kdir.display().to_string();
            let mut args = vec!["pack-knowledge", "--out", out.as_str()];
            args.extend(known.iter().map(|i| i.unit.as_str()));
            hashes.extend(kit_json(&args)?.as_object().cloned().unwrap_or_default());
        }
        let mut skills = Vec::new();
        let mut record = Vec::new();
        for item in &batch {
            let key = if item.kind == "skill" { &item.name } else { &item.unit };
            let hash = hashes.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            let folder = item.unit.strip_prefix("Skills/").unwrap_or("").to_string();
            let mut files = Vec::new();
            match item.kind {
                "skill" => walk(&dir.join("Skills").join(&folder), "", &mut files),
                "vault skill" => walk(&under(&dir.join(KNOWN), &item.unit), "", &mut files),
                _ => files.push((item.unit.clone(), under(&dir.join(KNOWN), &item.unit))),
            }
            let mut shown = Vec::new();
            for (rel, path) in files {
                let bytes = fs::read(&path).map_err(|_| NO_ROOM.to_string())?;
                let text = String::from_utf8_lossy(&bytes).to_string();
                let mut masked_text = compact::mask_for_sharing(&text);
                if rel == "SKILL.md" && item.kind == "skill" {
                    masked_text = restore_credit(&text, &masked_text);
                }
                let masked = masked_text != text;
                if masked {
                    put(&path, masked_text.as_bytes())?;
                }
                shown.push(json!({"path": rel, "script": is_script(&rel, masked_text.as_bytes()), "masked": masked, "text": masked_text}));
            }
            // scripts first
            shown.sort_by_key(|f| (f["script"] != true, f["path"].as_str().unwrap_or("").to_string()));
            let desc = shown.iter().find(|f| f["path"] == "SKILL.md").and_then(|f| f["text"].as_str()).map(description).unwrap_or_default();
            let has_script = shown.iter().any(|f| f["script"] == true);
            let rank = ["skill", "vault skill", "guide"].iter().position(|k| *k == item.kind).unwrap_or(3);
            skills.push(((!has_script, rank), json!({"name": item.name, "unit": item.unit, "kind": item.kind, "description": desc, "files": shown})));
            let mut r = json!({"unit": item.unit, "kind": item.kind, "name": item.name, "hash": hash, "new": item.new});
            if item.kind == "guide" {
                r["path"] = json!(item.unit);
            } else {
                r["folder"] = json!(folder);
            }
            record.push(r);
        }
        skills.sort_by_key(|(order, v)| (*order, v["name"].as_str().unwrap_or("").to_string()));
        put(&dir.join(META), json!({"login": login, "made": vault_sync::now(), "skills": record}).to_string().as_bytes())?;
        Ok::<Value, String>(json!({"ok": true, "batch": id, "skills": skills.into_iter().map(|(_, v)| v).collect::<Vec<_>>()}))
    })();
    if made.is_err() {
        vault_sync::remove(&dir);
    }
    made
}

/// The `credit:` and `source:` lines of a SKILL.md as they were before masking: the sender's GitHub login is the
/// credit the licence asks for, and it is already in the title, the commit and the branch, so hiding it buys nothing.
fn restore_credit(orig: &str, masked: &str) -> String {
    let line_of = |text: &'_ str, key: &str| text.lines().find(|l| l.starts_with(key)).map(str::to_string);
    let mut out = masked.to_string();
    for key in ["credit:", "source:"] {
        if let (Some(was), Some(now)) = (line_of(orig, key), line_of(masked, key)) {
            out = out.replacen(&now, &was, 1);
        }
    }
    out
}

/// The title of the pull request (and its commit's message): "Skills from", "Guides from" or "Skills and guides from"
/// the login, the first three names, then "+N more".
fn title(login: &str, names: &[String], skills: bool, guides: bool) -> String {
    let what = match (skills, guides) {
        (true, true) => "Skills and guides",
        (false, true) => "Guides",
        _ => "Skills",
    };
    let more = names.len().saturating_sub(3);
    format!("{what} from {login}: {}{}", names.iter().take(3).cloned().collect::<Vec<_>>().join(", "), if more > 0 { format!(" +{more} more") } else { String::new() })
}

/// One line of the pull request's words: a skill of the sender's (name, description, has scripts), a change to one of
/// the vault's skills (the same), or a guide (title, path, new).
enum Line {
    Skill(String, String, bool),
    VaultSkill(String, String, bool),
    Guide(String, String, bool),
}

/// The pull request's words: one line per skill or guide, the licence, and where it came from. A batch of only the
/// sender's own skills says they made them; one that changes the vault's skills or guides says they made the changes.
fn body(lines: &[Line]) -> String {
    let mut out: Vec<String> = lines
        .iter()
        .map(|l| match l {
            Line::Skill(name, desc, scripts) => format!("- **{name}**: {desc} (scripts: {})", if *scripts { "yes" } else { "no" }),
            Line::VaultSkill(name, desc, scripts) => format!("- **{name}**: {desc} (scripts: {}; a change to the vault's skill)", if *scripts { "yes" } else { "no" }),
            Line::Guide(title, path, new) => format!("- **{title}** (`{path}`): {} guide", if *new { "new" } else { "changed" }),
        })
        .collect();
    out.push(String::new());
    out.push(if lines.iter().all(|l| matches!(l, Line::Skill(..))) { LICENCE_LINE } else { CHANGES_LINE }.to_string());
    out.push(String::new());
    out.push(format!("Sent from Fridae {}.", paths::VERSION));
    // an "@" in a description must not notify anyone
    out.join("\n").replace('@', "@\u{200b}")
}

/// The user's Send: exactly what is in the outbox folder (not packed again), without the skills named in `drop`.
pub fn send(batch: &str, drop: &[String]) -> Value {
    if mode() == "off" {
        return fail(OFF);
    }
    let Some(_turn) = Turn::take() else { return fail(BUSY) };
    send_in(batch, drop).unwrap_or_else(fail)
}

fn send_in(batch: &str, drop: &[String]) -> Result<Value, String> {
    if !github::connected() {
        return Err(github::NOT_CONNECTED.to_string());
    }
    let dir = batch_dir(batch).filter(|d| d.join(META).is_file()).ok_or_else(|| GONE.to_string())?;
    let meta: Value = fs::read(dir.join(META)).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(|| GONE.to_string())?;
    let login = login().ok_or_else(|| NO_LOGIN.to_string())?;
    // the units still going, each with its files: library skills at Skills/<folder>/, the vault's skills at
    // Skills/<vault name>/, guides at their path (a batch made before guides could be sent holds only library skills)
    let (mut files, mut sent, mut lines) = (Vec::new(), Vec::new(), Vec::new());
    for s in meta["skills"].as_array().into_iter().flatten() {
        let kind = s["kind"].as_str().unwrap_or("skill");
        let (name, hash) = (s["name"].as_str().unwrap_or(""), s["hash"].as_str().unwrap_or(""));
        let folder = s["folder"].as_str().unwrap_or("");
        let unit = s["unit"].as_str().map(str::to_string).unwrap_or_else(|| format!("Skills/{folder}"));
        if name.is_empty() || drop.iter().any(|d| *d == unit || (kind == "skill" && d == name)) {
            continue;
        }
        if kind == "guide" {
            let path = s["path"].as_str().filter(|p| !p.starts_with("Skills/") && vault_sync::check_path(p)).ok_or_else(|| GONE.to_string())?;
            let file = under(&dir.join(KNOWN), path);
            if !fs::symlink_metadata(&file).is_ok_and(|m| m.is_file()) {
                return Err(GONE.to_string());
            }
            files.push((path.to_string(), fs::read(&file).map_err(|_| GONE.to_string())?));
            lines.push(Line::Guide(name.to_string(), path.to_string(), s["new"] == true));
            sent.push((kind, name.to_string(), unit, hash.to_string()));
            continue;
        }
        if folder.is_empty() || !vault_sync::check_path(&format!("Skills/{folder}/SKILL.md")) {
            return Err(GONE.to_string());
        }
        let root = if kind == "vault skill" { under(&dir.join(KNOWN), &format!("Skills/{folder}")) } else { dir.join("Skills").join(folder) };
        let mut found = Vec::new();
        walk(&root, "", &mut found);
        if !found.iter().any(|(r, _)| r == "SKILL.md") {
            return Err(GONE.to_string());
        }
        let (mut text, mut scripts) = (String::new(), false);
        for (rel, path) in found {
            let bytes = fs::read(&path).map_err(|_| GONE.to_string())?;
            if rel == "SKILL.md" {
                text = String::from_utf8_lossy(&bytes).to_string();
            }
            scripts |= is_script(&rel, &bytes);
            files.push((format!("Skills/{folder}/{rel}"), bytes));
        }
        lines.push(if kind == "vault skill" { Line::VaultSkill(name.to_string(), description(&text), scripts) } else { Line::Skill(name.to_string(), description(&text), scripts) });
        sent.push((if kind == "vault skill" { "vault skill" } else { "skill" }, name.to_string(), unit, hash.to_string()));
    }
    if sent.is_empty() {
        return Err("There is nothing left to send.".to_string());
    }
    let names: Vec<String> = sent.iter().map(|s| s.1.clone()).collect();
    let (has_skills, has_guides) = (sent.iter().any(|s| s.0 != "guide"), sent.iter().any(|s| s.0 == "guide"));
    let words = title(&login, &names, has_skills, has_guides);
    let upstream = vault_git::vault_repo();
    // the vault's owner gets the vault repo itself: her branch is made there, and her pull request reaches her inbox
    let fork = vault_git::ensure_fork(&upstream)?;
    let branch = branch_name(&login, &paths::stamp());
    let head = vault_git::head_sha(&fork, "main")?;
    vault_git::new_branch(&fork, &branch, &head)?;
    vault_git::commit_files(&fork, &branch, &files, &[], &words)?;
    let owner = fork.split('/').next().unwrap_or(&login);
    let (_, url) = vault_git::open_pr(&upstream, &format!("{owner}:{branch}"), &words, &body(&lines))?;
    // the pull request is open: note each unit as sent (a note that fails is told, the pull request stands)
    let mut unmarked = Vec::new();
    for (kind, name, unit, hash) in &sent {
        let unit_arg = format!("--unit={unit}");
        let args: Vec<&str> = if *kind == "skill" { vec!["mark-shared", name, "--hash", hash, "--pr", &url] } else { vec!["mark-shared", &unit_arg, "--hash", hash, "--pr", &url] };
        if kit(&args).is_err() {
            unmarked.push(name.clone());
        }
    }
    paths::update_config(|c: &mut Map<String, Value>| c.insert(LAST_KEY.into(), json!(vault_sync::now())));
    vault_sync::remove(&dir);
    let mut out = json!({"ok": true, "pr_url": url});
    if !unmarked.is_empty() {
        out["warning"] = json!(format!("Sent, but Fridae could not note {} as sent, so it may offer it again.", unmarked.join(", ")));
    }
    Ok(out)
}

/// Under the standing choice (Send automatically): once 24 hours have passed since the last send, and something is
/// ready, make a batch and send it, leaving out each skill with a script Fridae had to hide something in.
pub fn auto_tick() {
    if mode() != "auto" || !github::connected() || vault_sync::now() < config_num(LAST_KEY).unwrap_or(0) + DAY {
        return;
    }
    let Some(_turn) = Turn::take() else { return };
    if ready().map(|r| r.is_empty()).unwrap_or(true) {
        return;
    }
    let Ok(made) = prepare_in() else { return };
    let Some(batch) = made["batch"].as_str() else { return };
    // (name, unit) of each unit with a script Fridae hid something in
    let held: Vec<(String, String)> = made["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["files"].as_array().is_some_and(|f| f.iter().any(|f| f["script"] == true && f["masked"] == true)))
        .filter_map(|s| Some((s["name"].as_str()?.to_string(), s["unit"].as_str()?.to_string())))
        .collect();
    paths::update_config(|c: &mut Map<String, Value>| c.insert(HELD_KEY.into(), Value::Array(held.iter().map(|(n, _)| json!({"id": n, "why": HELD_WHY})).collect())));
    let held: Vec<String> = held.into_iter().map(|(_, u)| u).collect();
    if held.len() >= made["skills"].as_array().map_or(0, Vec::len) {
        if let Some(dir) = batch_dir(batch) {
            vault_sync::remove(&dir);
        }
        return;
    }
    let _ = send_in(batch, &held);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{STAND_IN, StandIn, TEST_LOCK};
    use base64::Engine;
    use std::sync::{Arc, MutexGuard};

    /// `base` joined with a '/'-separated path, part by part.
    fn join(base: &Path, rel: &str) -> PathBuf {
        rel.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
    }

    const TOKEN: &str = "gho_sharetest0123456789abcdef";

    /// Each request the stub had: (method, path, body).
    type Log = Arc<Mutex<Vec<(String, String, String)>>>;

    struct Stub {
        log: Log,
        /// the pull request answers 422 while this is set
        refuse_pr: Arc<Mutex<bool>>,
    }

    /// A GitHub that knows the fork, the branch and the pull request calls a Send makes.
    fn stub() -> (String, Stub) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let log: Log = Default::default();
        let refuse_pr = Arc::new(Mutex::new(false));
        let (l, refuse) = (log.clone(), refuse_pr.clone());
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let method = req.method().as_str().to_string();
                let path = req.url().to_string();
                let mut body = Vec::new();
                let _ = std::io::Read::read_to_end(req.as_reader(), &mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let blobs = {
                    let mut g = l.lock().unwrap();
                    g.push((method.clone(), path.clone(), body.clone()));
                    g.iter().filter(|(m, p, _)| m == "POST" && p.ends_with("/git/blobs")).count()
                };
                // the user's fork (tester/r), or the vault itself when the maintainer (o) sends
                let (repo, rest) = match path.strip_prefix("/repos/tester/r").map(|r| ("tester", r)).or_else(|| path.strip_prefix("/repos/o/r").map(|r| ("o", r))) {
                    Some(x) => x,
                    None => ("", path.as_str()),
                };
                let (code, text) = match (method.as_str(), repo, rest) {
                    ("GET", "", "/user") => (200, r#"{"login":"tester"}"#.into()),
                    ("GET", "tester", "") => (200, "{}".to_string()),
                    ("POST", "tester", "/merge-upstream") => (200, "{}".into()),
                    ("GET", _, "/git/ref/heads/main") if !repo.is_empty() => (200, r#"{"object":{"sha":"head1"}}"#.into()),
                    ("POST", _, "/git/refs") if !repo.is_empty() => (201, "{}".into()),
                    ("GET", _, p) if !repo.is_empty() && p.starts_with("/git/ref/heads/skills-") => (200, r#"{"object":{"sha":"head1"}}"#.into()),
                    ("GET", _, "/git/commits/head1") if !repo.is_empty() => (200, r#"{"tree":{"sha":"tree0"}}"#.into()),
                    ("POST", _, "/git/blobs") if !repo.is_empty() => (201, format!(r#"{{"sha":"blob{blobs}"}}"#)),
                    ("POST", _, "/git/trees") if !repo.is_empty() => (201, r#"{"sha":"tree1"}"#.into()),
                    ("POST", _, "/git/commits") if !repo.is_empty() => (201, r#"{"sha":"commit2"}"#.into()),
                    ("PATCH", _, p) if !repo.is_empty() && p.starts_with("/git/refs/heads/skills-") => (200, "{}".into()),
                    ("POST", "o", "/pulls") if *refuse.lock().unwrap() => (422, r#"{"message":"Validation Failed"}"#.into()),
                    ("POST", "o", "/pulls") => (201, r#"{"number":7,"html_url":"https://example.test/o/r/pull/7"}"#.into()),
                    _ => (404, "{}".into()),
                };
                let _ = req.respond(tiny_http::Response::from_string(text).with_status_code(code));
            }
        });
        (url, Stub { log, refuse_pr })
    }

    fn library() -> PathBuf {
        paths::test_dir("vault-home").join("skills")
    }

    /// A clean slate: the stub as GitHub, a signed-in user, an empty scratch library, no outbox, no share settings.
    fn setup() -> (MutexGuard<'static, ()>, Stub) {
        let held = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, s) = stub();
        *STAND_IN.lock().unwrap() = Some(StandIn { web: url.clone(), api: url.clone(), raw: url, vault_repo: Some("o/r".into()), ..Default::default() });
        github::disconnect();
        github::save_token(TOKEN).unwrap();
        fs::write(github::folder().join("user.json"), r#"{"login":"tester"}"#).unwrap();
        let _ = fs::remove_dir_all(paths::test_dir("vault-home"));
        let _ = fs::remove_dir_all(vault_sync::vault_app());
        fs::create_dir_all(library()).unwrap();
        paths::update_config(|c| {
            for k in [MODE_KEY, AGREED_KEY, LAST_KEY, HELD_KEY] {
                c.remove(k);
            }
        });
        (held, s)
    }

    fn mode_is(m: &str) {
        paths::update_config(|c| c.insert(MODE_KEY.into(), json!(m)));
    }

    /// A skill of the user's own, with words no other skill has; `extra` are more files.
    fn skill(name: &str, words: &str, extra: &[(&str, &str)]) {
        let dir = library().join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\ndescription: About {name}, a thing to try.\n---\n\n# {name}\n\n{words}\n")).unwrap();
        for (rel, text) in extra {
            let p = join(&dir, rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }

    const ALPHA: &str = "Stack the pumpkins by weight before the harvest festival so the heaviest sit lowest and the lantern carvers can reach the light ones.";
    const BETA: &str = "Rinse the copper kettles after the tea tasting, dry them upside down on the slatted rack and polish the spouts with lemon peel.";

    fn requests(s: &Stub) -> Vec<(String, String, String)> {
        s.log.lock().unwrap().clone()
    }

    /// The paths and bytes of the tree the one commit made, from the blobs and the tree the stub was given.
    fn committed(s: &Stub) -> std::collections::BTreeMap<String, Vec<u8>> {
        let r = requests(s);
        let blobs: Vec<Vec<u8>> = r.iter().filter(|x| x.0 == "POST" && x.1.ends_with("/git/blobs")).map(|x| base64::engine::general_purpose::STANDARD.decode(serde_json::from_str::<Value>(&x.2).unwrap()["content"].as_str().unwrap()).unwrap()).collect();
        let mut out = std::collections::BTreeMap::new();
        for x in r.iter().filter(|x| x.0 == "POST" && x.1.ends_with("/git/trees")) {
            for e in serde_json::from_str::<Value>(&x.2).unwrap()["tree"].as_array().unwrap() {
                let n: usize = e["sha"].as_str().unwrap().trim_start_matches("blob").parse().unwrap();
                out.insert(e["path"].as_str().unwrap().to_string(), blobs[n - 1].clone());
            }
        }
        out
    }

    fn calls(s: &Stub) -> Vec<String> {
        requests(s).iter().map(|x| format!("{} {}", x.0, x.1)).collect()
    }

    fn shared_file(name: &str) -> PathBuf {
        library().join(name).join(".fridae-shared.json")
    }

    #[test]
    fn review_sends_only_on_send() {
        let (_g, s) = setup();
        skill("alpha", ALPHA, &[]);
        mode_is("review");
        let made = prepare();
        assert_eq!(made["ok"], true, "{made}");
        assert!(requests(&s).is_empty(), "preparing asks GitHub for nothing: {:?}", calls(&s));
        let batch = made["batch"].as_str().unwrap().to_string();
        assert_eq!(made["skills"][0]["name"], "alpha");
        assert_eq!(made["skills"][0]["files"][0]["path"], "SKILL.md");
        let out = send(&batch, &[]);
        assert_eq!((out["ok"].as_bool(), out["pr_url"].as_str()), (Some(true), Some("https://example.test/o/r/pull/7")), "{out}");
        let c = calls(&s);
        let at = |needle: &str| c.iter().position(|x| x.contains(needle)).unwrap_or_else(|| panic!("{needle} in {c:?}"));
        let order = ["merge-upstream", "POST /repos/tester/r/git/refs", "git/blobs", "POST /repos/tester/r/git/commits", "PATCH /repos/tester/r/git/refs/heads/skills-", "POST /repos/o/r/pulls"];
        assert!(order.windows(2).all(|w| at(w[0]) < at(w[1])), "{c:?}");
        let pr: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1 == "/repos/o/r/pulls").unwrap().2).unwrap();
        assert_eq!(pr["title"], "Skills from tester: alpha");
        assert_eq!(pr["base"], "main");
        assert!(pr["head"].as_str().unwrap().starts_with("tester:skills-tester-"), "{pr}");
        let text = pr["body"].as_str().unwrap();
        assert!(text.starts_with("- **alpha**: About alpha, a thing to try. (scripts: no)\n"), "{text}");
        assert!(text.contains("I made these skills and share them under CC BY 4.0 (text) and MIT (scripts)."), "{text}");
        assert!(text.ends_with(&format!("Sent from Fridae {}.", paths::VERSION)), "{text}");
        let commit: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1.ends_with("/git/commits")).unwrap().2).unwrap();
        assert_eq!(commit["message"], "Skills from tester: alpha");
        let tree = committed(&s);
        assert_eq!(tree.keys().collect::<Vec<_>>(), vec!["Skills/alpha/SKILL.md"]);
        let front = String::from_utf8(tree["Skills/alpha/SKILL.md"].clone()).unwrap();
        assert!(front.contains("licence: CC-BY-4.0 (text), MIT (scripts)") && front.contains("credit: tester"), "{front}");
        let note: Value = serde_json::from_slice(&fs::read(shared_file("alpha")).unwrap()).unwrap();
        assert_eq!(note["sent"][0]["pr"], "https://example.test/o/r/pull/7");
        assert!(config_num(LAST_KEY).is_some());
        assert!(!outbox().join(&batch).exists(), "a sent batch's folder is removed");
    }

    #[test]
    fn dropped_skill_is_not_sent() {
        let (_g, s) = setup();
        skill("a", ALPHA, &[]);
        skill("b", BETA, &[]);
        mode_is("review");
        let made = prepare();
        assert_eq!(made["skills"].as_array().unwrap().len(), 2, "{made}");
        let out = send(made["batch"].as_str().unwrap(), &["b".to_string()]);
        assert_eq!(out["ok"], true, "{out}");
        let tree = committed(&s);
        assert!(tree.keys().all(|k| k.starts_with("Skills/a/")) && !tree.is_empty(), "{:?}", tree.keys());
        assert!(shared_file("a").is_file() && !shared_file("b").exists());
    }

    #[test]
    fn masked_text_is_what_is_sent() {
        let (_g, s) = setup();
        let home = paths::home_dir().to_string_lossy().to_string();
        let sep = std::path::MAIN_SEPARATOR;
        let words = format!("{ALPHA}\n\nKeep the notes in {home}{sep}notes and mail me.person@example.com when done.");
        skill("alpha", &words, &[]);
        mode_is("review");
        let made = prepare();
        let shown = made["skills"][0]["files"][0]["text"].as_str().unwrap().to_string();
        assert!(shown.contains(&format!("~{sep}notes")) && shown.contains("[email]") && made["skills"][0]["files"][0]["masked"] == true, "{shown}");
        let out = send(made["batch"].as_str().unwrap(), &[]);
        assert_eq!(out["ok"], true, "{out}");
        let sent = String::from_utf8(committed(&s)["Skills/alpha/SKILL.md"].clone()).unwrap();
        assert!(sent.contains(&format!("~{sep}notes")) && sent.contains("[email]"), "{sent}");
        assert!(!sent.contains(&home) && !sent.contains("me.person@example.com"));
        let everything: String = requests(&s).iter().map(|x| x.2.clone()).collect::<Vec<_>>().join("\n");
        assert!(!everything.contains("me.person@example.com") && !everything.contains(&home), "the raw text never reached GitHub");
        // the user's own skill is as it was
        assert!(fs::read_to_string(library().join("alpha").join("SKILL.md")).unwrap().contains("me.person@example.com"));
    }

    #[test]
    fn the_credit_is_never_masked() {
        let (_g, s) = setup();
        // the local user name is the GitHub login, and another login holds it as a word
        let was = std::env::var("USER").ok();
        unsafe { std::env::set_var("USER", "tester") };
        skill("alpha", &format!("{ALPHA}

Made by tester."), &[]);
        mode_is("review");
        let made = prepare();
        let out = send(made["batch"].as_str().unwrap(), &[]);
        match was {
            Some(v) => unsafe { std::env::set_var("USER", v) },
            None => unsafe { std::env::remove_var("USER") },
        }
        assert_eq!(out["ok"], true, "{out}");
        let sent = String::from_utf8(committed(&s)["Skills/alpha/SKILL.md"].clone()).unwrap();
        assert!(sent.contains("credit: tester") && sent.contains("source: shared from Fridae by tester"), "{sent}");
        assert!(sent.contains("Made by [user]."), "the name elsewhere is still hidden: {sent}");
        let keep = |orig: &str, masked: &str| restore_credit(orig, masked);
        assert_eq!(keep("credit: river-dev
x river", "credit: [user]-dev
x [user]"), "credit: river-dev
x [user]");
        assert_eq!(keep("source: shared from Fridae by River
", "source: shared from Fridae by [user]
"), "source: shared from Fridae by River
");
    }

    #[test]
    fn auto_holds_masked_scripts() {
        let (_g, s) = setup();
        skill("tokened", BETA, &[("scripts/go.sh", &format!("#!/bin/sh\nexport TOKEN=ghp_{}\necho hi\n", "a".repeat(30)))]);
        skill("clean", ALPHA, &[("scripts/ok.sh", "#!/bin/sh\necho hi\n")]);
        mode_is("auto");
        auto_tick();
        let tree = committed(&s);
        assert!(!tree.is_empty() && tree.keys().all(|k| k.starts_with("Skills/clean/")), "{:?}", tree.keys());
        assert!(!everything(&s).contains("ghp_"));
        let st = status();
        assert_eq!(st["held_masked"][0]["id"], "tokened", "{st}");
        assert_eq!(st["held_masked"][0]["why"], HELD_WHY);
        assert!(!shared_file("tokened").exists() && shared_file("clean").is_file());
    }

    fn everything(s: &Stub) -> String {
        let r = requests(s);
        let blobs = r.iter().filter(|x| x.1.ends_with("/git/blobs")).map(|x| String::from_utf8_lossy(&base64::engine::general_purpose::STANDARD.decode(serde_json::from_str::<Value>(&x.2).unwrap()["content"].as_str().unwrap()).unwrap()).to_string());
        r.iter().map(|x| x.2.clone()).chain(blobs).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn auto_sends_nothing_when_every_skill_is_held() {
        let (_g, s) = setup();
        skill("tokened", BETA, &[("scripts/go.sh", &format!("export TOKEN=ghp_{}\n", "a".repeat(30)))]);
        mode_is("auto");
        auto_tick();
        assert!(requests(&s).is_empty());
        assert_eq!(status()["held_masked"][0]["id"], "tokened");
        assert!(config_num(LAST_KEY).is_none());
        assert!(fs::read_dir(outbox()).map(|d| d.count() == 0).unwrap_or(true), "the batch nobody sends is removed");
    }

    #[test]
    fn auto_waits_a_day() {
        let (_g, s) = setup();
        skill("alpha", ALPHA, &[]);
        mode_is("auto");
        paths::update_config(|c| c.insert(LAST_KEY.into(), json!(vault_sync::now() - 3600)));
        auto_tick();
        assert!(requests(&s).is_empty());
        paths::update_config(|c| c.insert(LAST_KEY.into(), json!(vault_sync::now() - DAY - 5)));
        auto_tick();
        assert!(!requests(&s).is_empty(), "a day later it sends");
    }

    #[test]
    fn status_lists_ready_held_and_sent() {
        let (_g, _s) = setup();
        skill("alpha", ALPHA, &[]);
        mode_is("review");
        let made = prepare();
        assert_eq!(send(made["batch"].as_str().unwrap(), &[])["ok"], true);
        skill("beta", BETA, &[("scripts/go.sh", "#!/bin/sh\necho hi\n")]);
        skill("gamma", "Fold the linen napkins into swans for the lighthouse keeper's anniversary supper out on the pier.", &[]);
        let owned = library().join("owned");
        fs::create_dir_all(&owned).unwrap();
        fs::write(owned.join("SKILL.md"), "---\nname: owned\ndescription: Owned by a shop.\nlicence: Proprietary\n---\n\nSweep the bakery floor twice before the ovens are lit.\n").unwrap();
        // held by the last automatic batch: gamma, and owned again (listed once, with the kit's reason)
        paths::update_config(|c| c.insert(HELD_KEY.into(), json!([{"id": "gamma", "why": HELD_WHY}, {"id": "owned", "why": HELD_WHY}])));
        let st = status();
        assert_eq!(st["ready"], json!([{"id": "beta", "scripts": true}]), "{st}");
        let held = st["held"].as_array().unwrap();
        let ids: Vec<&str> = held.iter().map(|h| h["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["owned", "gamma"], "{st}");
        assert!(held[0]["why"].as_str().unwrap().contains("Proprietary"), "the kit's own reason: {st}");
        assert_eq!(held[1]["why"], HELD_WHY);
        assert_eq!(st["sent"][0]["id"], "alpha");
        assert_eq!(st["sent"][0]["pr"], "https://example.test/o/r/pull/7");
        assert!(st["sent"][0]["at"].is_string(), "{st}");
        assert_eq!(st["shareable"], 2, "the kit's count stands: {st}");
        // off: the lists are there and empty
        mode_is("off");
        let st = status();
        assert_eq!((st["ready"].clone(), st["held"].clone(), st["sent"].clone()), (json!([]), json!([]), json!([])));
    }

    #[test]
    fn off_or_unasked_never_sends() {
        let (_g, s) = setup();
        skill("alpha", ALPHA, &[]);
        // a batch made while sharing was on, then sharing turned off
        mode_is("review");
        let batch = prepare()["batch"].as_str().unwrap().to_string();
        for state in [None, Some("off")] {
            paths::update_config(|c| match state {
                Some(m) => c.insert(MODE_KEY.into(), json!(m)),
                None => c.remove(MODE_KEY),
            });
            auto_tick();
            assert_eq!(send(&batch, &[])["error"], OFF);
            assert_eq!(prepare()["error"], OFF);
        }
        assert!(requests(&s).is_empty(), "{:?}", calls(&s));
        assert!(config_num(LAST_KEY).is_none());
    }

    #[test]
    fn no_token_in_outbox_or_logs() {
        let (_g, _s) = setup();
        skill("alpha", ALPHA, &[("scripts/go.sh", "#!/bin/sh\necho hi\n")]);
        mode_is("review");
        let made = prepare();
        let batch = made["batch"].as_str().unwrap().to_string();
        let holds = |dir: &Path| {
            fn walk_all(d: &Path, out: &mut Vec<PathBuf>) {
                for e in fs::read_dir(d).into_iter().flatten().flatten() {
                    if e.path().is_dir() { walk_all(&e.path(), out) } else { out.push(e.path()) }
                }
            }
            let mut all = Vec::new();
            walk_all(dir, &mut all);
            assert!(!all.is_empty());
            all.iter().any(|f| fs::read(f).map(|b| String::from_utf8_lossy(&b).contains(TOKEN)).unwrap_or(false))
        };
        assert!(!holds(&outbox()), "nothing in the outbox holds the token");
        assert!(!made.to_string().contains(TOKEN));
        let out = send(&batch, &[]);
        assert_eq!(out["ok"], true, "{out}");
        assert!(!out.to_string().contains(TOKEN) && !status().to_string().contains(TOKEN));
        // this module writes no log line at all
        let source = include_str!("share.rs");
        let production = &source[..source.find("#[cfg(test)]").unwrap()];
        assert!(!production.contains("println!") && !production.contains("log::") && !production.contains("dbg!"));
    }

    #[test]
    fn a_failure_part_way_marks_nothing() {
        let (_g, s) = setup();
        skill("alpha", ALPHA, &[]);
        mode_is("review");
        *s.refuse_pr.lock().unwrap() = true;
        let batch = prepare()["batch"].as_str().unwrap().to_string();
        let out = send(&batch, &[]);
        assert_eq!((out["ok"].as_bool(), out["error"].as_str()), (Some(false), Some("GitHub would not open the pull request. Try again later.")));
        assert!(!shared_file("alpha").exists() && config_num(LAST_KEY).is_none());
        assert!(outbox().join(&batch).is_dir(), "the batch stays, to be sent again");
        *s.refuse_pr.lock().unwrap() = false;
        assert_eq!(send(&batch, &[])["ok"], true);
        assert!(shared_file("alpha").is_file());
    }

    #[test]
    fn modes_need_the_agreement_and_github() {
        let (_g, _s) = setup();
        assert_eq!(set_mode("sometimes", true)["ok"], false);
        assert_eq!(set_mode("review", false)["error"], AGREE);
        assert!(config_num(AGREED_KEY).is_none() && mode() == "off");
        let out = set_mode("review", true);
        assert_eq!((out["ok"].as_bool(), mode()), (Some(true), "review"));
        assert!(config_num(AGREED_KEY).is_some());
        assert_eq!(set_mode("off", false)["ok"], true);
        assert_eq!(mode(), "off");
        github::disconnect();
        assert_eq!(set_mode("auto", true)["error"], "Connect GitHub first: Setup, Connections.");
        assert_eq!(mode(), "off");
    }

    #[test]
    fn scripts_come_first_and_masked_is_flagged() {
        let (_g, _s) = setup();
        skill("alpha", ALPHA, &[("notes.md", "Plain notes."), ("scripts/b.txt", "not much"), ("tool", "#!/bin/sh\necho x\n"), ("run.PY", &format!("key = 'sk-{}'\n", "a".repeat(30)))]);
        skill("plain", BETA, &[]);
        mode_is("review");
        let made = prepare();
        assert_eq!(made["skills"][0]["name"], "alpha", "skills with scripts first: {made}");
        let files: Vec<(String, bool, bool)> = made["skills"][0]["files"].as_array().unwrap().iter().map(|f| (f["path"].as_str().unwrap().to_string(), f["script"] == true, f["masked"] == true)).collect();
        assert_eq!(files, vec![("run.PY".into(), true, true), ("scripts/b.txt".into(), true, false), ("tool".into(), true, false), ("SKILL.md".into(), false, false), ("notes.md".into(), false, false)]);
        assert!(!made["skills"][0]["files"][0]["text"].as_str().unwrap().contains("sk-aaaa"));
    }

    #[test]
    fn titles_and_script_rules() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
        assert_eq!(title("me", &names[..3], true, false), "Skills from me: a, b, c");
        assert_eq!(title("me", &names, true, false), "Skills from me: a, b, c +2 more");
        assert_eq!(title("me", &names[..2], false, true), "Guides from me: a, b");
        assert_eq!(title("me", &names, true, true), "Skills and guides from me: a, b, c +2 more");
        assert_eq!(body(&[Line::Skill("a".into(), "does @someone a thing".into(), true)]).lines().next().unwrap(), "- **a**: does @\u{200b}someone a thing (scripts: yes)");
        let lines = [Line::Guide("Glossary".into(), "Roblox/Glossary.md".into(), false), Line::Guide("New".into(), "Roblox/02 Luau/New.md".into(), true)];
        let text = body(&lines);
        assert!(text.starts_with("- **Glossary** (`Roblox/Glossary.md`): changed guide\n- **New** (`Roblox/02 Luau/New.md`): new guide\n"), "{text}");
        assert!(text.contains("I made these changes and share them under CC BY 4.0 (text) and MIT (scripts)."), "{text}");
        assert!(is_script("scripts/x", b"") && is_script("a/b.PS1", b"") && is_script("x", b"#!/usr/bin/env python") && !is_script("notes.md", b"text") && !is_script("data.json", b"{}"));
        assert_eq!(description("---\nname: a\ndescription: >\n  One\n  two\n---\nbody"), "One two");
        assert_eq!(description("---\ndescription: \"Quoted one\"\n---\n"), "Quoted one");
        assert_eq!(safe("a/b: c"), "a-b- c");
    }

    fn knowledge() -> PathBuf {
        paths::test_dir("vault-home").join("Knowledge")
    }

    /// A guide in the scratch Knowledge folder; the vault never gave it, so it is new and ready.
    fn guide(rel: &str, text: &str) {
        let p = join(&knowledge(), rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// Run with the computer's user name set to `name` (as `the_credit_is_never_masked` does).
    fn as_user<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let (was, was_win) = (std::env::var("USER").ok(), std::env::var("USERNAME").ok());
        unsafe {
            std::env::set_var("USER", name);
            std::env::set_var("USERNAME", name);
        }
        let out = f();
        for (k, v) in [("USER", was), ("USERNAME", was_win)] {
            match v {
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        out
    }

    #[test]
    fn maintainer_send_opens_a_pull_request_in_her_own_repo() {
        let (_g, s) = setup();
        fs::write(github::folder().join("user.json"), r#"{"login":"o"}"#).unwrap();
        guide("Roblox/Glossary.md", "# Glossary\n\nWords.\n");
        mode_is("review");
        let made = prepare();
        assert_eq!(made["ok"], true, "{made}");
        let out = send(made["batch"].as_str().unwrap(), &[]);
        assert_eq!(out["ok"], true, "{out}");
        let c = calls(&s);
        assert!(c.iter().all(|x| !x.contains("/forks") && !x.contains("merge-upstream") && !x.contains("/repos/tester/")), "{c:?}");
        assert!(c.iter().any(|x| x == "POST /repos/o/r/git/refs"), "the branch is made in the vault repo: {c:?}");
        let pr: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1 == "/repos/o/r/pulls").unwrap().2).unwrap();
        assert!(pr["head"].as_str().unwrap().starts_with("o:skills-o-"), "{pr}");
        assert_eq!(pr["title"], "Guides from o: Glossary");
    }

    #[test]
    fn guides_are_masked_and_sent_at_their_paths() {
        let (_g, s) = setup();
        guide("Roblox/02 Luau/Notes.md", "# Notes\n\nAsk river about it, or mail river.person@example.com.\n");
        mode_is("review");
        let (made, out) = as_user("river", || {
            let made = prepare();
            let out = send(made["batch"].as_str().unwrap_or(""), &[]);
            (made, out)
        });
        assert_eq!(made["ok"], true, "{made}");
        let item = &made["skills"][0];
        assert_eq!((item["unit"].as_str(), item["kind"].as_str(), item["name"].as_str()), (Some("Roblox/02 Luau/Notes.md"), Some("guide"), Some("Notes")), "{made}");
        assert_eq!(item["files"][0]["path"], "Roblox/02 Luau/Notes.md");
        assert_eq!(item["files"][0]["masked"], true);
        assert_eq!(out["ok"], true, "{out}");
        let tree = committed(&s);
        assert_eq!(tree.keys().collect::<Vec<_>>(), vec!["Roblox/02 Luau/Notes.md"]);
        let sent = String::from_utf8(tree["Roblox/02 Luau/Notes.md"].clone()).unwrap();
        assert_eq!(sent, "# Notes\n\nAsk [user] about it, or mail [email].\n");
        assert!(!everything(&s).contains("river.person@example.com"));
        let pr: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1 == "/repos/o/r/pulls").unwrap().2).unwrap();
        assert!(pr["body"].as_str().unwrap().starts_with("- **Notes** (`Roblox/02 Luau/Notes.md`): new guide\n"), "{pr}");
        // noted as sent: no longer ready
        let st = status();
        assert_eq!(st["knowledge"]["ready"], json!([]), "{st}");
        assert_eq!(st["knowledge"]["sent"][0]["unit"], "Roblox/02 Luau/Notes.md");
        // the user's own guide is as it was
        assert!(fs::read_to_string(join(&knowledge(), "Roblox/02 Luau/Notes.md")).unwrap().contains("river.person@example.com"));
    }

    #[test]
    fn a_batch_holds_skills_and_guides_up_to_20() {
        let (_g, s) = setup();
        let words = ["pumpkins", "kettles", "napkins", "lanterns", "bicycles", "teapots", "gardens", "ladders", "ribbons", "candles", "marbles", "pebbles", "feathers", "buttons", "anchors"];
        for (i, w) in words.iter().enumerate() {
            skill(&format!("s{i:02}"), &format!("Arrange the {w} by colour on the {w} shelf before the {w} fair opens, then count the {w} twice."), &[]);
        }
        for i in 0..10 {
            guide(&format!("Roblox/G{i:02}.md"), &format!("# Guide {i}\n"));
        }
        mode_is("review");
        let made = prepare();
        let items = made["skills"].as_array().unwrap();
        assert_eq!(items.len(), 20, "{made}");
        assert_eq!(items.iter().filter(|i| i["kind"] == "skill").count(), 15, "every skill first, then guides");
        assert_eq!(items.iter().filter(|i| i["kind"] == "guide").count(), 5);
        let out = send(made["batch"].as_str().unwrap(), &[]);
        assert_eq!(out["ok"], true, "{out}");
        let tree = committed(&s);
        assert_eq!(tree.keys().filter(|k| k.starts_with("Skills/")).count(), 15);
        assert_eq!(tree.keys().filter(|k| k.starts_with("Roblox/")).cloned().collect::<Vec<_>>(), (0..5).map(|i| format!("Roblox/G{i:02}.md")).collect::<Vec<_>>());
        let pr: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1 == "/repos/o/r/pulls").unwrap().2).unwrap();
        assert_eq!(pr["title"], "Skills and guides from tester: s00, s01, s02 +17 more");
        let commit: Value = serde_json::from_str(&requests(&s).iter().find(|x| x.1.ends_with("/git/commits")).unwrap().2).unwrap();
        assert_eq!(commit["message"], pr["title"]);
        // the five left over are ready for the next batch
        let st = status();
        assert_eq!(st["knowledge"]["ready"].as_array().unwrap().len(), 5, "{st}");
    }

    #[test]
    fn status_lists_guides_ready_held_and_sent() {
        let (_g, _s) = setup();
        guide("Roblox/Sent.md", "# Sent\n");
        mode_is("review");
        assert_eq!(send(prepare()["batch"].as_str().unwrap(), &[])["ok"], true);
        guide("Roblox/Ready.md", "# Ready\n");
        guide("Roblox/18 Rigs/R.fbx", "Kaydara FBX");
        let st = status();
        let k = &st["knowledge"];
        assert_eq!(k["ready"], json!([{"unit": "Roblox/Ready.md", "kind": "guide", "title": "Ready", "new": true}]), "{st}");
        assert_eq!(k["held"], json!([{"unit": "Roblox/18 Rigs/R.fbx", "title": "R.fbx", "why": "Pictures and models cannot be shared yet."}]), "{st}");
        assert_eq!((k["sent"][0]["unit"].as_str(), k["sent"][0]["pr"].as_str()), (Some("Roblox/Sent.md"), Some("https://example.test/o/r/pull/7")), "{st}");
        mode_is("off");
        assert_eq!(status()["knowledge"], json!({"ready": [], "held": [], "sent": []}));
    }

    #[test]
    fn a_vault_skill_is_sent_under_its_vault_name() {
        let (_g, s) = setup();
        // a skill the vault never gave (no base record): new, and shared under its folder's name
        let dir = join(&knowledge(), "Skills/tidy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: tidy\ndescription: Put things away.\ncredit: alice\n---\n\nPut things away.\n").unwrap();
        mode_is("review");
        let made = prepare();
        assert_eq!((made["skills"][0]["unit"].as_str(), made["skills"][0]["kind"].as_str()), (Some("Skills/tidy"), Some("vault skill")), "{made}");
        assert_eq!(send(made["batch"].as_str().unwrap(), &[])["ok"], true);
        let sent = String::from_utf8(committed(&s)["Skills/tidy/SKILL.md"].clone()).unwrap();
        assert!(sent.contains("credit: alice") && !sent.contains("licence:"), "the vault keeps its own credit: {sent}");
    }

    #[test]
    fn old_outbox_folders_are_removed() {
        let (_g, _s) = setup();
        let old = outbox().join("old-batch");
        fs::create_dir_all(&old).unwrap();
        let made = |secs: u64| json!({"made": secs}).to_string();
        fs::write(old.join(META), made(vault_sync::now() - KEEP.as_secs() - 3600)).unwrap();
        let fresh = outbox().join("fresh-batch");
        fs::create_dir_all(&fresh).unwrap();
        fs::write(fresh.join(META), made(vault_sync::now() - 3600)).unwrap();
        clean_outbox();
        assert!(!old.exists() && fresh.exists());
    }

    #[test]
    fn no_agent_tool_reaches_a_send() {
        let kit_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for file in ["rbxkit_mcp.py", "rbxshare.py"] {
            let text = fs::read_to_string(kit_dir.join(file)).unwrap();
            for word in ["vault/send", "vault/prepare", "share_mode", "share::"] {
                assert!(!text.contains(word), "{file} reaches {word}");
            }
        }
        assert!(crate::paths::GUARDED.contains(&MODE_KEY), "a file edit cannot turn sharing on");
    }

    #[test]
    fn branch_name_never_panics_and_differs_within_a_minute() {
        let full = branch_name("ada", "20261201-153007");
        assert!(full.starts_with("skills-ada-20261201-1530-") && full.len() == "skills-ada-20261201-1530-".len() + 4, "{full}");
        // where the clock cannot be read the stamp is plain seconds (10 characters)
        assert!(branch_name("ada", "1764600000").starts_with("skills-ada-1764600000-"));
        assert_ne!(branch_name("ada", "20261201-153007"), branch_name("ada", "20261201-153007"));
    }
}
