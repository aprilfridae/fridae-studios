//! The Fridae vault's Download and Pull newest: the latest release's manifest is read, only the files that changed
//! are fetched (each checked against the manifest's hash and size) into a staging folder, and only when every one is
//! right is the folder handed to `rbx skills vault-apply`, which puts skills, guides and notes into `<home>/Knowledge`
//! without overriding the user: what both sides changed and cannot be combined waits for the user's Keep the vault's
//! or Keep mine (`clashes`, `decide_clash`). Anything wrong or cut off removes the staging folder and changes
//! nothing. No sign-in is needed.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{accounts, paths, vault_git};

/// What a manifest that fails any check says, and what a refused `vault-apply` says (rbxvaultsync.BAD).
const BAD: &str = "This vault update looked wrong, so nothing was changed.";
const BUSY: &str = "The vault is already being updated. Wait a moment.";
const NO_RELEASE: &str = "No vault release yet.";
const NO_ROOM: &str = "Fridae could not save the download in its own folder.";
const NOT_RUN: &str = "Fridae could not run the update.";
/// The code's own data under `Roblox/`, which stays in the app and never travels (rbxvaultsync.CODE_DATA).
const CODE_DATA: [&str; 2] = ["Roblox/_tools/", "Roblox/_data/"];
/// At most 20,000 files, 8 MiB each, 256 MiB in all (rbxvaultsync's limits: the guides are 1,352 files and 21 MB,
/// the largest under 1 MB).
pub(crate) const MAX_FILES: usize = 20_000;
pub(crate) const MAX_FILE: u64 = 8 << 20;
pub(crate) const MAX_TOTAL: u64 = 256 << 20;
/// The config keys that remember the last download and check.
const RELEASE_KEY: &str = "vault_release";
const CHECKED_KEY: &str = "vault_checked_at";
const LAST_KEY: &str = "vault_last";
/// `vault-apply` is given this long (it reads the library once and writes file by file).
const APPLY_LIMIT: Duration = Duration::from_secs(600);
/// The first check after launch, then one a day while the app is open.
const FIRST_WAIT: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct ManFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct Manifest {
    pub release: String,
    pub files: Vec<ManFile>,
}

/// rbxvaultsync._seg_ok: whether one part of a path may be a file's or folder's name on every system. Not empty, not
/// dot-started, at most 255 bytes, and nothing Windows would read as a drive, a stream or a device, or would change:
/// no `:` or `<>"|?*\`, no control character, no trailing dot or space and no leading space, and none of `CON PRN
/// AUX NUL CONIN$ CONOUT$ COM0-9 LPT0-9` (also with `¹²³`), in any case, with or without an extension.
fn seg_ok(s: &str) -> bool {
    const DEVICES: [&str; 6] = ["con", "prn", "aux", "nul", "conin$", "conout$"];
    let stem = s.split('.').next().unwrap_or("").trim_end_matches(' ').to_lowercase();
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|d| d.chars().count() == 1 && d.chars().all(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    let device = DEVICES.contains(&stem.as_str()) || numbered("com") || numbered("lpt");
    !s.is_empty()
        && !s.starts_with('.')
        && s.len() <= 255
        && !device
        && !s.chars().any(|c| c.is_control() || "<>:\"|?*\\".contains(c))
        && !s.ends_with(['.', ' '])
        && !s.starts_with(' ')
}

/// rbxvaultsync.check_path, what travels with the vault: everything in Knowledge but top-level files (`House
/// rules.md`), dot-folders and the code's data (`CODE_DATA`); under `Skills/`, only inside a skill's folder. No
/// leading slash, and every segment a safe name (`seg_ok`). The download, Reveal and the maintainer's inbox all use it.
pub fn check_path(p: &str) -> bool {
    if p.starts_with('/') || CODE_DATA.iter().any(|t| p.starts_with(t)) {
        return false;
    }
    let parts: Vec<&str> = p.split('/').collect();
    let least = if parts[0] == "Skills" { 3 } else { 2 };
    parts.len() >= least && parts.iter().all(|s| seg_ok(s))
}

/// rbxvaultsync.unit_of: what a clash is decided for. A skill is decided whole (`Skills/<name>`), anything else per
/// file.
pub(crate) fn unit_of(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() >= 3 && parts[0] == "Skills" { format!("Skills/{}", parts[1]) } else { p.to_string() }
}

/// A release name that is safe as a folder name (the staging folder is named by it): `vault-`, then letters, digits,
/// dots, dashes or underscores.
pub(crate) fn release_ok(r: &str) -> bool {
    r.len() > "vault-".len() && r.len() <= 100 && r.starts_with("vault-") && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn hash_ok(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn sha(bytes: &[u8]) -> String {
    paths::hex(&Sha256::digest(bytes))
}

/// A release's manifest: format 1, the limits, 64 lower-case hex characters per hash, unique safe paths.
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let bad = || BAD.to_string();
    let v: Value = serde_json::from_slice(bytes).map_err(|_| bad())?;
    if v["format"].as_i64() != Some(1) {
        return Err(bad());
    }
    let release = v["release"].as_str().filter(|r| release_ok(r)).ok_or_else(bad)?.to_string();
    let list = v["files"].as_array().filter(|l| !l.is_empty() && l.len() <= MAX_FILES).ok_or_else(bad)?;
    let (mut files, mut seen, mut total) = (Vec::new(), std::collections::HashSet::new(), 0u64);
    for f in list {
        let (path, hash, size) = (f["path"].as_str().ok_or_else(bad)?, f["sha256"].as_str().ok_or_else(bad)?, f["size"].as_u64().ok_or_else(bad)?);
        if !check_path(path) || !hash_ok(hash) || size > MAX_FILE || !seen.insert(path.to_string()) {
            return Err(bad());
        }
        total += size;
        files.push(ManFile { path: path.to_string(), sha256: hash.to_string(), size });
    }
    if total > MAX_TOTAL {
        return Err(bad());
    }
    Ok(Manifest { release, files })
}

// ------------------------------------------------------------------------------------------------ where things are
/// `<home>/Knowledge`; in tests a scratch folder, because the real CLI runs against it.
fn knowledge() -> PathBuf {
    #[cfg(test)]
    return paths::test_dir("vault-home").join("Knowledge");
    #[allow(unreachable_code)]
    paths::PATHS.home.join("Knowledge")
}

fn base_file() -> PathBuf {
    knowledge().join(".fridae-vault").join("base.json")
}

/// The app's own folder for the vault (`VAULT_APP`).
pub(crate) fn vault_app() -> PathBuf {
    accounts::base().join("fridae-vault")
}

fn staging_root() -> PathBuf {
    vault_app().join("staging")
}

/// `base` joined with a '/'-separated path, segment by segment.
fn join(base: &Path, path: &str) -> PathBuf {
    path.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
}

/// Where a manifest file is staged: under `dir`, or an error (defence in depth: `check_path` already refuses every
/// name that could lead out of it).
fn staged_target(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let target = join(dir, path);
    if check_path(path) && target.starts_with(dir) && target != dir { Ok(target) } else { Err(BAD.to_string()) }
}

/// The base record, whole; empty when there is none or it cannot be read.
fn read_base() -> Value {
    fs::read(base_file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// Whether the user has pressed Download (the base record exists).
pub(crate) fn downloaded() -> bool {
    base_file().is_file()
}

/// A path as this machine keeps it: a skill the user renamed lives in its new folder (rbxvaultsync._local_path).
/// Only skills are renamed: any other path is kept as it is.
fn local_path(path: &str, base: &Value) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 && parts[0] == "Skills" {
        if let Some(to) = base["renamed"][parts[1]].as_str() {
            parts[1] = to;
        }
    }
    parts.join("/")
}

const NOT_SHOWN: &str = "Fridae can only show files in your Knowledge folder.";
const GONE: &str = "That file is not there any more.";
const CHOOSE: &str = "Choose Keep the vault's or Keep mine.";

/// Whether `p` is a skill's unit (`Skills/<name>`), which clash cards and Reveal name instead of one of its files.
fn skill_unit(p: &str) -> bool {
    matches!(p.split('/').collect::<Vec<_>>().as_slice(), ["Skills", name] if seg_ok(name))
}

/// What Reveal shows for a vault path from a report or a clash card (a `check_path` path, or a skill's unit
/// `Skills/<name>`). `which` "mine": the user's file (a renamed skill in its new folder), else its folder. `which`
/// "vault": the vault's copy waiting under `.fridae-vault/clash/`. Never through a link, never outside Knowledge.
pub fn reveal_target(path: &str, which: &str) -> Result<PathBuf, String> {
    if !check_path(path) && !skill_unit(path) {
        return Err(NOT_SHOWN.to_string());
    }
    let know = knowledge();
    let (root, rel) = match which {
        "mine" => (know.clone(), local_path(path, &read_base())),
        "vault" => (know.join(".fridae-vault").join("clash"), path.to_string()),
        _ => return Err(NOT_SHOWN.to_string()),
    };
    if !check_path(&rel) && !skill_unit(&rel) {
        return Err(NOT_SHOWN.to_string());
    }
    // every step from Knowledge down is a real folder, never a link (the clash copy's .fridae-vault/clash included)
    let mut at = know.clone();
    let below = root.strip_prefix(&know).map(Path::to_path_buf).unwrap_or_default();
    let steps: Vec<String> = below.iter().map(|s| s.to_string_lossy().to_string()).chain(rel.split('/').map(str::to_string)).collect();
    let mut steps = steps.iter().peekable();
    while let Some(step) = steps.next() {
        at.push(step);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return Err(NOT_SHOWN.to_string()),
            Ok(m) if steps.peek().is_some() && !m.is_dir() => return Err(GONE.to_string()),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let target = join(&root, &rel);
    if !target.starts_with(&root) || target == root {
        return Err(NOT_SHOWN.to_string());
    }
    if fs::symlink_metadata(&target).is_ok_and(|m| m.is_file() || m.is_dir()) {
        return Ok(target);
    }
    // the vault's copy is shown only when it is there; the user's missing file shows its folder (not a skill gone
    // whole, and never Knowledge or Skills themselves)
    match target.parent() {
        Some(dir) if which == "mine" && dir != know && dir != know.join("Skills") && fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) => {
            Ok(dir.to_path_buf())
        }
        _ => Err(GONE.to_string()),
    }
}

/// POST /api/vault/reveal {path, which}: show a file of the vault's, the user's or the vault's waiting copy, in
/// Finder or Explorer.
pub fn reveal(path: &str, which: &str) -> Value {
    match reveal_target(path, which) {
        Ok(p) => {
            if crate::promo::reveal_path(&p.display().to_string()) { json!({"ok": true}) } else { fail("Fridae could not open the folder.") }
        }
        Err(e) => fail(e),
    }
}

/// GET /api/vault/clashes: what each card waiting for Keep the vault's / Keep mine shows (`vault-clashes`).
pub fn clashes() -> Value {
    match crate::share::kit_json(&["vault-clashes", "--json"]) {
        Ok(Value::Array(list)) => json!({"ok": true, "clashes": list}),
        Ok(_) => fail(NOT_RUN),
        Err(e) => fail(e),
    }
}

/// POST /api/vault/clash {unit, choice}: the user's Keep the vault's ("vault") or Keep mine ("mine") for one waiting
/// unit, never while an update runs (it would rewrite the same record).
pub fn decide_clash(unit: &str, choice: &str) -> Value {
    let Some(_held) = claim() else {
        return json!({"ok": false, "busy": true, "error": BUSY});
    };
    if !matches!(choice, "vault" | "mine") {
        return fail(CHOOSE);
    }
    let (unit, choice) = (format!("--unit={unit}"), format!("--choice={choice}"));
    match crate::share::kit_json(&["vault-clash", &unit, &choice, "--json"]) {
        Ok(v) if v.is_object() => v,
        Ok(_) => fail(NOT_RUN),
        Err(e) => fail(e),
    }
}

/// An ordinary file's bytes (a link or a folder is nothing), when it is there and small enough to matter.
fn read_plain(path: &Path) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.is_file() && meta.len() <= MAX_FILE { fs::read(path).ok() } else { None }
}

fn local_bytes(f: &ManFile, base: &Value, knowledge: &Path) -> Option<Vec<u8>> {
    let local = local_path(&f.path, base);
    if check_path(&local) { read_plain(&join(knowledge, &local)) } else { None }
}

/// The files to fetch: those whose hash differs from both the base record's and the local file's.
pub fn needed(m: &Manifest, base: &Value, knowledge: &Path) -> Vec<ManFile> {
    m.files
        .iter()
        .filter(|f| base["files"][&f.path].as_str() != Some(&f.sha256) && local_bytes(f, base, knowledge).map(|b| sha(&b)).as_deref() != Some(f.sha256.as_str()))
        .cloned()
        .collect()
}

// ------------------------------------------------------------------------------------------------------- the job
static RUNNING: Mutex<bool> = Mutex::new(false);

/// The one running place, held until dropped (so a panic cannot leave it held).
struct Claim;

impl Drop for Claim {
    fn drop(&mut self) {
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }
}

/// Take the one running place; None when an update is already under way.
fn claim() -> Option<Claim> {
    let taken = std::mem::replace(&mut *RUNNING.lock().unwrap_or_else(|e| e.into_inner()), true);
    if taken { None } else { Some(Claim) }
}

pub(crate) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run `update` in the background (one at a time): Download is `first`, Check for updates is not.
pub fn start(first: bool) -> Value {
    let Some(held) = claim() else {
        return json!({"ok": false, "busy": true, "error": BUSY});
    };
    std::thread::spawn(move || {
        let _held = held;
        update(first);
    });
    json!({"ok": true, "running": true})
}

/// `{downloaded, release, checked_at, running, last}`.
pub fn status() -> Value {
    let c = paths::load_config();
    json!({
        "downloaded": downloaded(),
        "release": c.get(RELEASE_KEY).cloned().unwrap_or(Value::Null),
        "checked_at": c.get(CHECKED_KEY).cloned().unwrap_or(Value::Null),
        "running": *RUNNING.lock().unwrap_or_else(|e| e.into_inner()),
        "last": c.get(LAST_KEY).cloned().unwrap_or(Value::Null),
    })
}

/// 60 s after launch, then every 24 h while the app is open: check for updates, once the user has pressed Download.
pub fn watch() {
    std::thread::sleep(FIRST_WAIT);
    loop {
        tick();
        crate::share::auto_tick(); // the user's standing choice to send their skills, at most once a day
        std::thread::sleep(EVERY);
    }
}

fn tick() {
    if !downloaded() {
        return;
    }
    if let Some(_held) = claim() {
        update(false);
    }
}

fn fail(why: impl Into<String>) -> Value {
    json!({"ok": false, "error": why.into()})
}

fn record(out: &Value) {
    paths::update_config(|c: &mut Map<String, Value>| {
        if let (Some(r), true) = (out["release"].as_str(), out["ok"] == true) {
            c.insert(RELEASE_KEY.into(), json!(r));
        }
        c.insert(CHECKED_KEY.into(), json!(now()));
        c.insert(LAST_KEY.into(), out.clone());
    });
}

/// Download the latest release's changes and apply them, or check that there is nothing new. Every end is recorded
/// for `status`. `first`: the user pressed Download; otherwise nothing happens before the first Download.
pub fn update(first: bool) -> Value {
    if !first && !downloaded() {
        return json!({"ok": true, "skipped": true});
    }
    let out = run();
    record(&out);
    out
}

fn run() -> Value {
    let repo = vault_git::vault_repo();
    let tag = match vault_git::latest_tag(&repo) {
        Err(e) => return fail(e),
        Ok(None) => return json!({"ok": true, "release": null, "note": NO_RELEASE}),
        Ok(Some(t)) if release_ok(&t) => t,
        Ok(Some(_)) => return fail(BAD),
    };
    let man_bytes = match vault_git::raw(&repo, &tag, "manifest.json") {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let m = match parse_manifest(&man_bytes) {
        Ok(m) if m.release == tag => m,
        _ => return fail(BAD),
    };
    let (base, know) = (read_base(), knowledge());
    let mut need = needed(&m, &base, &know);
    // a unit waiting for the user's choice on this same release: its vault copy is already kept for the card
    need.retain(|f| base["clashes"][unit_of(&f.path)]["release"].as_str() != Some(tag.as_str()));
    if base["release"].as_str() == Some(&tag) {
        // the same release: what the user deleted, declined or has waiting is no reason to download it again
        let listed = |key: &str, name: &str| base[key].as_array().is_some_and(|l| l.iter().any(|v| v.as_str() == Some(name)));
        // (declined and waiting name skills, so they count only for paths under Skills/)
        need.retain(|f| {
            let skill = f.path.strip_prefix("Skills/").and_then(|r| r.split('/').next());
            !listed("deleted", &f.path) && !skill.is_some_and(|n| listed("declined", n) || base["waiting"].get(n).is_some())
        });
        if need.is_empty() {
            return json!({"ok": true, "current": true, "release": tag});
        }
    }
    let (dir, manifest) = (staging_root().join(&tag), staging_root().join(format!("{tag}.manifest.json")));
    let out = stage_and_apply(&repo, &tag, &m, &man_bytes, &base, &need, &dir, &manifest);
    remove(&dir);
    remove(&manifest);
    out.unwrap_or_else(fail)
}

/// Remove a file or folder this module made (a link is removed itself, never followed).
pub(crate) fn remove(p: &Path) {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(p);
        }
        Ok(_) => {
            let _ = fs::remove_file(p);
        }
        Err(_) => {}
    }
}

/// A folder of ours that is a real folder (made when missing), never a link.
pub(crate) fn real_dir(p: &Path) -> Result<(), String> {
    if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(NO_ROOM.to_string());
    }
    fs::create_dir_all(p).map_err(|_| NO_ROOM.to_string())?;
    if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) { Ok(()) } else { Err(NO_ROOM.to_string()) }
}

/// A file written through a temporary one and a rename, never through a link.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    real_dir(path.parent().ok_or_else(|| NO_ROOM.to_string())?)?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(NO_ROOM.to_string());
    }
    let mut name = path.file_name().ok_or_else(|| NO_ROOM.to_string())?.to_os_string();
    name.push(format!(".{}.tmp", paths::random_hex(2)));
    let tmp = path.with_file_name(name);
    fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, path)).map_err(|_| {
        let _ = fs::remove_file(&tmp);
        NO_ROOM.to_string()
    })
}

fn right(f: &ManFile, bytes: &[u8]) -> bool {
    bytes.len() as u64 == f.size && sha(bytes) == f.sha256
}

/// Put every manifest file under `dir` (vault-apply wants the whole release there): the needed ones fetched and
/// checked, the others copied from where the same bytes already are (the user's file, the base record's copy or the
/// vault's copy waiting for a clash card) and fetched only when none has them. Then apply it.
#[allow(clippy::too_many_arguments)]
fn stage_and_apply(repo: &str, tag: &str, m: &Manifest, man_bytes: &[u8], base: &Value, need: &[ManFile], dir: &Path, manifest: &Path) -> Result<Value, String> {
    real_dir(&vault_app())?;
    real_dir(&staging_root())?;
    remove(dir);
    remove(manifest);
    let know = knowledge();
    let record_copies = know.join(".fridae-vault").join("base");
    let clash_copies = know.join(".fridae-vault").join("clash");
    for f in &m.files {
        let held = if need.contains(f) {
            None
        } else {
            local_bytes(f, base, &know)
                .filter(|b| right(f, b))
                .or_else(|| read_plain(&join(&record_copies, &f.path)).filter(|b| right(f, b)))
                .or_else(|| read_plain(&join(&clash_copies, &f.path)).filter(|b| right(f, b)))
        };
        let bytes = match held {
            Some(b) => b,
            None => {
                let b = vault_git::raw(repo, tag, &f.path)?;
                if !right(f, &b) {
                    return Err(BAD.to_string());
                }
                b
            }
        };
        write_atomic(&staged_target(dir, &f.path)?, &bytes)?;
    }
    write_atomic(manifest, man_bytes)?;
    apply(dir, manifest)
}

/// The command that applies a staged release: the CLI. In tests, the same CLI with a scratch home folder.
fn apply_command(staged: &Path, manifest: &Path) -> std::process::Command {
    let (staged, manifest) = (staged.display().to_string(), manifest.display().to_string());
    let args = ["skills", "vault-apply", "--staged", staged.as_str(), "--manifest", manifest.as_str(), "--json"];
    #[cfg(test)]
    {
        const WRAP: &str = "import sys, os\nkit, home = sys.argv[1:3]\nsys.path.insert(0, kit)\nimport rbxpaths\nrbxpaths.HOME_DIR = home\nrbxpaths.KNOWLEDGE_DIR = os.path.join(home, 'Knowledge')\nrbxpaths.CONFIG = os.path.join(home, 'config.json')\nimport rbx\nrbx.main(sys.argv[3:])\n";
        let home = knowledge().parent().map(Path::to_path_buf).unwrap_or_default();
        let mut c = std::process::Command::new(paths::python());
        c.args(["-c", WRAP]).arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("..")).arg(home).args(args).env("PYTHONIOENCODING", "utf-8");
        paths::quiet(&mut c);
        return c;
    }
    #[allow(unreachable_code)]
    paths::cli(&args)
}

/// Run `vault-apply`: its report with `ok: true`, or the reason it refused.
fn apply(staged: &Path, manifest: &Path) -> Result<Value, String> {
    use std::process::Stdio;
    let mut cmd = apply_command(staged, manifest);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|_| NOT_RUN.to_string())?;
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let out = match rx.recv_timeout(APPLY_LIMIT) {
        Ok(Ok(out)) => out,
        Ok(Err(_)) => return Err(NOT_RUN.to_string()),
        Err(_) => {
            paths::end_trees(&[id]);
            return Err("The vault update took too long, so nothing more was changed.".to_string());
        }
    };
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        return Err(if said.is_empty() || out.status.code() == Some(2) { BAD.to_string() } else { said });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut report: Value = serde_json::from_str(text.trim()).ok().filter(Value::is_object).ok_or_else(|| BAD.to_string())?;
    report["ok"] = json!(true);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{OFFLINE, STAND_IN, StandIn, TEST_LOCK};
    use std::{
        collections::{BTreeMap, HashMap},
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, MutexGuard},
    };

    const T1: &str = "vault-2026.12.01";
    const T2: &str = "vault-2026.12.02";
    const ALPHA: &str = "Skills/vault-alpha/SKILL.md";
    const ALPHA_REF: &str = "Skills/vault-alpha/ref.md";
    const BETA: &str = "Skills/vault-beta/SKILL.md";
    const GLOSSARY: &str = "Roblox/Glossary.md";

    #[derive(Default)]
    struct Served {
        tag: Option<String>,
        files: HashMap<String, Vec<u8>>,
        /// the nth (1-based) file besides the manifest is cut off half way
        cut_on: Option<usize>,
        fetched: usize,
    }

    struct Stub {
        served: Arc<Mutex<Served>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    /// A tiny HTTP server: the latest release at /repos/o/r/releases/latest, raw files at /o/r/<tag>/<path>.
    fn stub() -> (String, Stub) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let served: Arc<Mutex<Served>> = Default::default();
        let log: Arc<Mutex<Vec<String>>> = Default::default();
        let (s, l) = (served.clone(), log.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                l.lock().unwrap().push(path.clone());
                let reply = |code: &str, body: &[u8]| {
                    let mut out = format!("HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                    out.extend_from_slice(body);
                    out
                };
                let out = {
                    let mut g = s.lock().unwrap();
                    if path == "/repos/o/r/releases/latest" {
                        match &g.tag {
                            None => reply("404 Not Found", br#"{"message":"Not Found"}"#),
                            Some(t) => reply("200 OK", format!(r#"{{"tag_name":"{t}"}}"#).as_bytes()),
                        }
                    } else if let Some(rest) = g.tag.clone().and_then(|t| path.strip_prefix(&format!("/o/r/{t}/")).map(str::to_string)) {
                        if rest != "manifest.json" {
                            g.fetched += 1;
                        }
                        if rest != "manifest.json" && g.cut_on == Some(g.fetched) {
                            b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\npartial".to_vec()
                        } else if let Some(b) = g.files.get(&rest) {
                            reply("200 OK", b)
                        } else {
                            reply("404 Not Found", b"")
                        }
                    } else {
                        reply("404 Not Found", b"")
                    }
                };
                let _ = stream.write_all(&out);
            }
        });
        (url, Stub { served, log })
    }

    fn skill(name: &str, body: &str) -> String {
        let (topic, words) = if name.ends_with("alpha") {
            ("Crossing zebra stripes on a quiet road", "pedestrian zebra stripes kerb crossing signal lollipop")
        } else {
            ("Tuning a wooden xylophone before a concert", "xylophone mallet resonator bars pitch tuning concert")
        };
        format!("---
name: {name}
description: {topic}. {body}
---

# {name}

Words: {words}. {body}.
")
    }

    /// What the vault serves from now on: the manifest for `files` and each file's bytes.
    fn release(s: &Stub, tag: &str, files: &[(&str, String)]) {
        let mut g = s.served.lock().unwrap();
        let mut entries: Vec<Value> = Vec::new();
        let mut sorted: Vec<&(&str, String)> = files.iter().collect();
        sorted.sort_by_key(|f| f.0);
        g.files.clear();
        for (p, text) in sorted {
            entries.push(json!({"path": p, "sha256": sha(text.as_bytes()), "size": text.len()}));
            g.files.insert(p.to_string(), text.clone().into_bytes());
        }
        let man = json!({"format": 1, "release": tag, "made": "2026-12-01T00:00:00Z", "files": entries});
        g.files.insert("manifest.json".into(), man.to_string().into_bytes());
        g.tag = Some(tag.to_string());
        g.fetched = 0;
        g.cut_on = None;
    }

    fn base_set() -> Vec<(&'static str, String)> {
        vec![(ALPHA, skill("vault-alpha", "zebra crossings")), (ALPHA_REF, "Alpha reference about zebra crossings.\n".into()), (BETA, skill("vault-beta", "xylophone tuning"))]
    }

    fn setup() -> (MutexGuard<'static, ()>, Stub) {
        let held = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, s) = stub();
        *STAND_IN.lock().unwrap() = Some(StandIn { web: url.clone(), api: url.clone(), raw: url, vault_repo: Some("o/r".into()), ..Default::default() });
        let _ = fs::remove_dir_all(paths::test_dir("vault-home"));
        let _ = fs::remove_dir_all(vault_app());
        paths::update_config(|c| {
            for k in [RELEASE_KEY, CHECKED_KEY, LAST_KEY] {
                c.remove(k);
            }
        });
        (held, s)
    }

    /// Every file and folder under Knowledge with its bytes, to compare before and after.
    fn snapshot() -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                let rel = e.path().strip_prefix(root).unwrap().display().to_string();
                if e.path().is_dir() {
                    out.insert(rel, None);
                    walk(&e.path(), root, out);
                } else {
                    out.insert(rel, fs::read(e.path()).ok());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&knowledge(), &knowledge(), &mut out);
        out
    }

    /// The files fetched besides the manifest.
    fn fetched(s: &Stub) -> Vec<String> {
        s.log.lock().unwrap().iter().filter(|p| p.starts_with("/o/r/") && !p.ends_with("/manifest.json")).cloned().collect()
    }

    fn no_staging_left(tag: &str) {
        assert!(!staging_root().join(tag).exists(), "the staging folder is gone");
        assert!(!staging_root().join(format!("{tag}.manifest.json")).exists());
    }

    fn man(files: Vec<Value>) -> Vec<u8> {
        json!({"format": 1, "release": T1, "made": "x", "files": files}).to_string().into_bytes()
    }

    fn entry(path: &str) -> Value {
        json!({"path": path, "sha256": sha(b"x"), "size": 1})
    }

    #[test]
    fn manifest_rejects_bad_paths_and_limits() {
        assert!(parse_manifest(&man(vec![entry(ALPHA), entry(BETA)])).is_ok());
        // the path rule's cases, the same as tests/test_vaultsync.py's
        let long = format!("Skills/a/{}", "x".repeat(256));
        for bad in [
            "Skills/../x",
            "/etc/x",
            "Skills/a/.git/config",
            "Skills\\a",
            "Skills//a",
            "Skills/README.md",
            "Skills/a/b\0c",
            "Skills/a/..",
            "Skills/a/",
            long.as_str(),
            "House rules.md",
            "Glossary.md",
            ".fridae-vault/base.json",
            "Roblox/.obsidian/x.json",
            "Roblox/_tools/roblox_api.json",
            "Roblox/_data/roblox_facts_2026.json",
            "Roblox",
            "Roblox/",
        ] {
            assert!(!check_path(bad), "{bad:?}");
            assert!(parse_manifest(&man(vec![entry(ALPHA), entry(bad)])).is_err(), "{bad:?}");
        }
        assert!(check_path(&format!("Skills/a/{}", "x".repeat(255))));
        assert!(check_path("Skills/a/b/c.md"));
        // guides and other topic folders travel too
        for good in ["Roblox/Glossary.md", "Roblox/02 Luau Scripting/Buffers and Binary Data.md", "Roblox/18 Rigs and Skeletons/Spider_8Leg/Spider_8Leg.fbx", "Notes/a.md", "Taste/Softer.md", "Inbox/x.md"] {
            assert!(check_path(good), "{good:?}");
            assert!(parse_manifest(&man(vec![entry(ALPHA), entry(good)])).is_ok(), "{good:?}");
        }
        assert_eq!(unit_of("Skills/a/ref/x.md"), "Skills/a");
        assert_eq!(unit_of("Roblox/Glossary.md"), "Roblox/Glossary.md");
        // the limits
        assert_eq!((MAX_FILES, MAX_FILE, MAX_TOTAL), (20_000, 8 << 20, 256 << 20));
        let many: Vec<Value> = (0..20_001).map(|i| entry(&format!("Skills/s{i}/SKILL.md"))).collect();
        assert!(parse_manifest(&man(many.clone())).is_err(), "20,001 files");
        assert!(parse_manifest(&man(many[..20_000].to_vec())).is_ok(), "20,000 files");
        assert!(parse_manifest(&man(vec![json!({"path": ALPHA, "sha256": sha(b"x"), "size": (8 << 20) + 1})])).is_err(), "8 MiB and a byte");
        assert!(parse_manifest(&man(vec![json!({"path": ALPHA, "sha256": sha(b"x"), "size": 8 << 20})])).is_ok(), "8 MiB");
        let big = |n: usize| -> Vec<Value> { (0..n).map(|i| json!({"path": format!("Skills/s{i}/SKILL.md"), "sha256": sha(b"x"), "size": 8 << 20})).collect() };
        assert!(parse_manifest(&man(big(33))).is_err(), "over 256 MiB in all");
        assert!(parse_manifest(&man(big(32))).is_ok(), "256 MiB in all");
        for hash in [sha(b"x").to_uppercase(), "abc".to_string(), "g".repeat(64)] {
            assert!(parse_manifest(&man(vec![json!({"path": ALPHA, "sha256": hash, "size": 1})])).is_err(), "{hash}");
        }
        // the rest of the shape
        assert!(parse_manifest(&man(vec![entry(ALPHA), entry(ALPHA)])).is_err(), "the same path twice");
        assert!(parse_manifest(&man(vec![])).is_err());
        assert!(parse_manifest(b"not json").is_err());
        let other = |k: &str, v: Value| {
            let mut m: Value = serde_json::from_slice(&man(vec![entry(ALPHA)])).unwrap();
            m[k] = v;
            m.to_string().into_bytes()
        };
        assert!(parse_manifest(&other("format", json!(2))).is_err());
        assert!(parse_manifest(&other("release", json!("a/b"))).is_err());
        assert!(parse_manifest(&other("release", json!(""))).is_err());
        assert_eq!(parse_manifest(&other("format", json!(2))).unwrap_err(), BAD);
    }

    /// Names Windows would read as a drive, a stream or a device, or would change: the same table as
    /// tests/test_vaultsync.py's WINDOWS_BAD.
    const WINDOWS_BAD: [&str; 22] = [
        "Skills/a/C:x.md",
        "Skills/C:/x.md",
        "Skills/a/SKILL.md:s",
        "Skills/CON/x.md",
        "Skills/a/nul.txt",
        "Skills/a/com1",
        "Skills/a./x.md",
        "Skills/a /x.md",
        "Skills/a/x.md.",
        "Skills/a/CONIN$",
        "Skills/a/conout$.md",
        "Skills/a/COM\u{b9}",
        "Skills/a/lpt\u{b3}.md",
        "Skills/a/LPT0",
        "Skills/a/aux .md",
        "Skills/ a/x.md",
        "Skills/a/x<y.md",
        "Skills/a/x?.md",
        "Skills/a/x|y.md",
        "Skills/a/x\u{1}y.md",
        "Roblox/CON.md",
        "Roblox/a:b.md",
    ];
    const WINDOWS_GOOD: [&str; 7] = ["Skills/a/console.md", "Skills/a/com10", "Skills/con-tips/SKILL.md", "Skills/a/nul-thing.md", "Skills/a/x.y.md", "Skills/a/comb", "Roblox/con-tips.md"];

    #[test]
    fn path_rule_refuses_what_windows_would_misread() {
        for bad in WINDOWS_BAD {
            assert!(!check_path(bad), "{bad:?}");
            assert!(parse_manifest(&man(vec![entry(ALPHA), entry(bad)])).is_err(), "{bad:?}");
        }
        for good in WINDOWS_GOOD {
            assert!(check_path(good), "{good:?}");
        }
    }

    #[test]
    fn staging_never_writes_outside_its_folder() {
        let dir = Path::new("staging-root").join(T1);
        assert_eq!(staged_target(&dir, ALPHA), Ok(dir.join("Skills").join("vault-alpha").join("SKILL.md")));
        for bad in ["Skills/C:/x.md", "Skills/a/C:x.md", "Skills/../../x.md"] {
            assert!(staged_target(&dir, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn release_names_start_with_vault() {
        assert!(release_ok(T1));
        for bad in ["2026.12.01", "v1", "vault", "x-vault-1"] {
            assert!(!release_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn first_download_installs_and_records() {
        let (_g, s) = setup();
        release(&s, T1, &base_set());
        assert_eq!(status()["downloaded"], false);
        let r = update(true);
        assert_eq!(r["ok"], true, "{r}");
        let mut added: Vec<&str> = r["added"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        added.sort();
        assert_eq!(added, vec![ALPHA, ALPHA_REF, BETA]);
        for (p, text) in base_set() {
            assert_eq!(fs::read_to_string(join(&knowledge(), p)).unwrap(), text);
        }
        let st = status();
        assert_eq!((st["downloaded"].clone(), st["release"].clone(), st["running"].clone()), (json!(true), json!(T1), json!(false)));
        assert!(st["checked_at"].as_u64().unwrap() > 0);
        assert_eq!(st["last"]["added"].as_array().unwrap().len(), 3);
        assert_eq!(read_base()["release"], T1);
        no_staging_left(T1);
        // every file was fetched once, and the manifest too
        assert_eq!(fetched(&s).len(), 3);
    }

    #[test]
    fn second_check_fetches_only_changes() {
        let (_g, s) = setup();
        release(&s, T1, &base_set());
        assert_eq!(update(true)["ok"], true);
        let mut next = base_set();
        next[2].1 = skill("vault-beta", "xylophone tuning, now with a second step");
        release(&s, T2, &next);
        s.log.lock().unwrap().clear();
        let r = update(false);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["updated"], json!([BETA]));
        assert_eq!(fetched(&s), vec![format!("/o/r/{T2}/{BETA}")], "one file besides manifest.json");
        assert!(s.log.lock().unwrap().iter().any(|p| p.ends_with("/manifest.json")));
        assert_eq!(fs::read_to_string(join(&knowledge(), BETA)).unwrap(), next[2].1);
        assert_eq!(status()["release"], T2);
        no_staging_left(T2);
    }

    #[test]
    fn the_same_release_is_current() {
        let (_g, s) = setup();
        release(&s, T1, &base_set());
        assert_eq!(update(true)["ok"], true);
        s.log.lock().unwrap().clear();
        let r = update(false);
        assert_eq!((r["ok"].clone(), r["current"].clone()), (json!(true), json!(true)), "{r}");
        assert!(fetched(&s).is_empty());
        assert_eq!(status()["last"]["current"], true);
    }

    #[test]
    fn the_same_release_ignores_declined_only_for_skills() {
        let (_g, s) = setup();
        let guide = "Roblox/x/y.md";
        // the guide is the only file, so nothing else could be the reason it is fetched
        release(&s, T1, &[(guide, "The vault's guide.\n".to_string())]);
        let k = knowledge();
        fs::create_dir_all(k.join(".fridae-vault")).unwrap();
        fs::write(base_file(), json!({"format": 1, "release": T1, "files": {}, "declined": ["x"]}).to_string()).unwrap();
        fs::create_dir_all(k.join("Roblox").join("x")).unwrap();
        fs::write(join(&k, guide), "The user's own guide.\n").unwrap();
        s.log.lock().unwrap().clear();
        update(false);
        assert!(fetched(&s).contains(&format!("/o/r/{T1}/{guide}")), "{:?}", fetched(&s));
    }

    #[test]
    fn needed_is_what_differs_from_the_base_and_the_local_file() {
        let (_g, _s) = setup();
        let k = knowledge();
        let (a, b) = ("same as base", "edited by the user");
        fs::create_dir_all(k.join("Skills").join("x")).unwrap();
        fs::write(k.join("Skills/x/a.md"), a).unwrap();
        fs::write(k.join("Skills/x/b.md"), b).unwrap();
        fs::write(k.join("Skills/x/c.md"), "already new").unwrap();
        let file = |p: &str, text: &str| ManFile { path: p.into(), sha256: sha(text.as_bytes()), size: text.len() as u64 };
        let m = Manifest {
            release: T1.into(),
            files: vec![file("Skills/x/a.md", "new a"), file("Skills/x/b.md", b), file("Skills/x/c.md", "already new"), file("Skills/x/d.md", "brand new"), file("Skills/x/e.md", "same as base")],
        };
        let base = json!({"files": {"Skills/x/a.md": sha(a.as_bytes()), "Skills/x/b.md": sha(b"older"), "Skills/x/e.md": sha(b"same as base")}});
        let paths: Vec<String> = needed(&m, &base, &k).into_iter().map(|f| f.path).collect();
        // b: upstream's equals the user's file, c: already here, e: upstream did not change
        assert_eq!(paths, vec!["Skills/x/a.md", "Skills/x/d.md"]);
    }

    #[test]
    fn bad_hash_applies_nothing() {
        let (_g, s) = setup();
        // a first download with a wrong file leaves nothing at all
        release(&s, T1, &base_set());
        s.served.lock().unwrap().files.insert(BETA.into(), b"not what the manifest says".to_vec());
        let r = update(true);
        assert_eq!((r["ok"].clone(), r["error"].clone()), (json!(false), json!(BAD)), "{r}");
        assert!(!knowledge().exists());
        no_staging_left(T1);
        assert_eq!(status()["downloaded"], false);
        // an update with a wrong file leaves what was there as it was
        release(&s, T1, &base_set());
        assert_eq!(update(true)["ok"], true);
        let before = snapshot();
        let mut next = base_set();
        next[0].1 = skill("vault-alpha", "zebra crossings and a new step");
        next[2].1 = skill("vault-beta", "a changed beta");
        release(&s, T2, &next);
        s.served.lock().unwrap().files.insert(BETA.into(), skill("vault-beta", "swapped on the way").into_bytes());
        let r = update(false);
        assert_eq!((r["ok"].clone(), r["error"].clone()), (json!(false), json!(BAD)), "{r}");
        assert_eq!(snapshot(), before, "Knowledge and base.json are unchanged");
        no_staging_left(T2);
        assert_eq!(status()["release"], T1);
        assert_eq!(status()["last"]["ok"], false);
    }

    #[test]
    fn cut_off_download_applies_nothing() {
        let (_g, s) = setup();
        release(&s, T1, &base_set());
        assert_eq!(update(true)["ok"], true);
        let before = snapshot();
        let mut next = base_set();
        next[0].1 = skill("vault-alpha", "zebra crossings and a new step");
        next[2].1 = skill("vault-beta", "a changed beta");
        release(&s, T2, &next);
        s.served.lock().unwrap().cut_on = Some(2);
        let r = update(false);
        assert_eq!((r["ok"].clone(), r["error"].clone()), (json!(false), json!(OFFLINE)), "{r}");
        assert_eq!(snapshot(), before);
        no_staging_left(T2);
        assert_eq!(status()["release"], T1);
    }

    #[test]
    fn no_release_is_not_an_error() {
        let (_g, s) = setup();
        let r = update(true);
        assert_eq!(r, json!({"ok": true, "release": null, "note": "No vault release yet."}));
        assert_eq!(status()["last"], r);
        assert_eq!(status()["release"], Value::Null);
        assert_eq!(s.log.lock().unwrap().as_slice(), ["/repos/o/r/releases/latest"]);
        assert!(!knowledge().exists());
    }

    #[test]
    fn offline_says_so_and_changes_nothing() {
        let (_g, _s) = setup();
        STAND_IN.lock().unwrap().as_mut().unwrap().api = "http://127.0.0.1:1".into();
        let r = update(true);
        assert_eq!(r, json!({"ok": false, "error": OFFLINE}));
        assert_eq!(status()["last"]["error"], OFFLINE);
    }

    #[test]
    fn watch_waits_for_first_download() {
        let (_g, s) = setup();
        release(&s, T1, &base_set());
        tick();
        assert!(s.log.lock().unwrap().is_empty(), "no Download yet: no request");
        assert_eq!(update(false)["skipped"], true);
        assert!(s.log.lock().unwrap().is_empty());
        assert_eq!(update(true)["ok"], true);
        s.log.lock().unwrap().clear();
        tick();
        assert!(s.log.lock().unwrap().iter().any(|p| p == "/repos/o/r/releases/latest"), "after Download the watch checks");
    }

    #[test]
    fn only_one_update_runs_at_a_time() {
        let (_g, _s) = setup();
        let held = claim();
        assert!(held.is_some());
        assert_eq!(start(false)["busy"], true);
        assert_eq!(status()["running"], true);
        drop(held);
        assert_eq!(status()["running"], false);
    }

    #[test]
    fn a_panic_never_leaves_the_update_claimed() {
        let (_g, _s) = setup();
        let hurt = std::panic::catch_unwind(|| {
            let _held = claim().unwrap();
            panic!("cut short");
        });
        assert!(hurt.is_err());
        assert_eq!(status()["running"], false);
        assert!(claim().is_some());
    }

    /// A link to a folder (a junction where Windows refuses a symlink); false when this machine allows neither.
    fn link_dir(to: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(to, at).is_ok();
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_dir(to, at).is_ok() {
                return true;
            }
            let ok = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(at).arg(to).output().is_ok_and(|o| o.status.success());
            return ok && fs::symlink_metadata(at).is_ok_and(|m| m.file_type().is_symlink());
        }
        #[allow(unreachable_code)]
        false
    }

    #[test]
    fn reveal_stays_in_the_knowledge_folder() {
        let (_g, _s) = setup();
        let k = knowledge();
        let skills = k.join("Skills");
        fs::create_dir_all(skills.join("vault-alpha")).unwrap();
        fs::write(skills.join("vault-alpha").join("SKILL.md"), "mine").unwrap();
        assert_eq!(reveal_target(ALPHA, "mine"), Ok(skills.join("vault-alpha").join("SKILL.md")));
        // a skill's unit is its folder
        assert_eq!(reveal_target("Skills/vault-alpha", "mine"), Ok(skills.join("vault-alpha")));
        // a missing file: its folder
        assert_eq!(reveal_target(ALPHA_REF, "mine"), Ok(skills.join("vault-alpha")));
        // a guide anywhere in Knowledge
        fs::create_dir_all(k.join("Roblox")).unwrap();
        fs::write(k.join("Roblox").join("Glossary.md"), "mine").unwrap();
        assert_eq!(reveal_target(GLOSSARY, "mine"), Ok(k.join("Roblox").join("Glossary.md")));
        // a skill the user renamed lives in its new folder
        fs::create_dir_all(skills.join("my-beta")).unwrap();
        fs::write(skills.join("my-beta").join("SKILL.md"), "mine").unwrap();
        write_atomic(&base_file(), br#"{"renamed": {"vault-beta": "my-beta"}}"#).unwrap();
        assert_eq!(reveal_target(BETA, "mine"), Ok(skills.join("my-beta").join("SKILL.md")));
        assert_eq!(reveal_target("Skills/vault-beta", "mine"), Ok(skills.join("my-beta")));
        fs::write(k.join("House rules.md"), "rules").unwrap();
        for bad in ["Skills/../x/SKILL.md", "Skills/vault-alpha/../../x.md", "/etc/passwd", "C:/Windows/win.ini", "Skills/C:/x/y.md",
                    "Skills/x:y/SKILL.md", "Skills\\vault-alpha\\SKILL.md", "Skills/vault-gone/SKILL.md", "Skills/vault-gone", "",
                    "House rules.md", "../x", "Roblox/../../x.md", ".fridae-vault/base.json", "Skills", "Roblox"] {
            assert!(reveal_target(bad, "mine").is_err(), "{bad}");
        }
        assert_eq!(reveal_target("House rules.md", "mine"), Err(NOT_SHOWN.to_string()));
        assert_eq!(NOT_SHOWN, "Fridae can only show files in your Knowledge folder.");
        assert!(reveal_target(GLOSSARY, "elsewhere").is_err());
        // a link inside Knowledge is never followed
        let outside = paths::test_dir("vault-outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("SKILL.md"), "elsewhere").unwrap();
        if link_dir(&outside, &skills.join("linked")) && link_dir(&outside, &k.join("Linked")) {
            assert!(reveal_target("Skills/linked/SKILL.md", "mine").is_err());
            assert!(reveal_target("Skills/linked", "mine").is_err());
            assert!(reveal_target("Linked/SKILL.md", "mine").is_err());
        } else {
            eprintln!("no folder links on this machine: the link case was not run");
        }
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn reveal_shows_the_vaults_copy_of_a_clash() {
        let (_g, _s) = setup();
        let k = knowledge();
        let clash = k.join(".fridae-vault").join("clash");
        fs::create_dir_all(k.join("Roblox")).unwrap();
        fs::write(k.join("Roblox").join("Glossary.md"), "mine").unwrap();
        // no copy waiting: nothing to show
        assert_eq!(reveal_target(GLOSSARY, "vault"), Err(GONE.to_string()));
        fs::create_dir_all(clash.join("Roblox")).unwrap();
        fs::write(clash.join("Roblox").join("Glossary.md"), "the vault's").unwrap();
        assert_eq!(reveal_target(GLOSSARY, "vault"), Ok(clash.join("Roblox").join("Glossary.md")));
        assert_eq!(reveal_target(GLOSSARY, "mine"), Ok(k.join("Roblox").join("Glossary.md")));
        // a skill's copy is its folder, by the vault's name even when the user renamed it
        fs::create_dir_all(clash.join("Skills").join("vault-beta")).unwrap();
        fs::write(clash.join("Skills").join("vault-beta").join("SKILL.md"), "the vault's").unwrap();
        write_atomic(&base_file(), br#"{"renamed": {"vault-beta": "my-beta"}}"#).unwrap();
        assert_eq!(reveal_target("Skills/vault-beta", "vault"), Ok(clash.join("Skills").join("vault-beta")));
        assert_eq!(reveal_target(BETA, "vault"), Ok(clash.join("Skills").join("vault-beta").join("SKILL.md")));
        for bad in ["../x", "House rules.md", "Roblox/../../x.md", ""] {
            assert!(reveal_target(bad, "vault").is_err(), "{bad}");
        }
        // a link in place of the clash folder is never followed
        let outside = paths::test_dir("vault-outside");
        fs::create_dir_all(outside.join("Roblox")).unwrap();
        fs::write(outside.join("Roblox").join("Glossary.md"), "elsewhere").unwrap();
        fs::remove_dir_all(&clash).unwrap();
        if link_dir(&outside, &clash) {
            assert!(reveal_target(GLOSSARY, "vault").is_err());
        } else {
            eprintln!("no folder links on this machine: the link case was not run");
        }
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn clash_decisions_wait_for_a_running_update() {
        let (_g, _s) = setup();
        let held = claim();
        let r = decide_clash(GLOSSARY, "vault");
        assert_eq!((r["ok"].clone(), r["busy"].clone(), r["error"].clone()), (json!(false), json!(true), json!(BUSY)), "{r}");
        drop(held);
        for bad in ["", "both", "Vault"] {
            assert_eq!(decide_clash(GLOSSARY, bad), json!({"ok": false, "error": "Choose Keep the vault's or Keep mine."}), "{bad}");
        }
        // nothing waiting: the CLI's own answer, passed on
        let r = decide_clash(GLOSSARY, "mine");
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some_and(|e| !e.is_empty()), "{r}");
        assert_eq!(clashes(), json!({"ok": true, "clashes": []}));
    }

    #[test]
    fn a_waiting_clash_is_no_reason_to_download_again() {
        let (_g, s) = setup();
        let other = "Roblox/Other.md";
        let one = vec![(GLOSSARY, "one\ntwo\nthree\n".to_string()), (other, "other\n".to_string())];
        release(&s, T1, &one);
        assert_eq!(update(true)["ok"], true);
        let k = knowledge();
        fs::write(join(&k, GLOSSARY), "one\nTWO (mine)\nthree\n").unwrap();
        let mut two = one.clone();
        two[0].1 = "one\nTWO (the vault's)\nthree\n".into();
        release(&s, T2, &two);
        let r = update(false);
        assert_eq!(r["clashes"], json!([GLOSSARY]), "{r}");
        assert_eq!(fs::read_to_string(join(&k, GLOSSARY)).unwrap(), "one\nTWO (mine)\nthree\n", "the user's stays in use");
        let waiting = clashes();
        assert_eq!(waiting["ok"], true, "{waiting}");
        assert_eq!(waiting["clashes"][0]["unit"], GLOSSARY, "{waiting}");
        // the daily check on the same release: nothing fetched, nothing asked again
        s.log.lock().unwrap().clear();
        let r = update(false);
        assert_eq!((r["ok"].clone(), r["current"].clone()), (json!(true), json!(true)), "{r}");
        assert!(fetched(&s).is_empty(), "{:?}", fetched(&s));
        // another file of the same release is needed again (it could not be put in place): only it is fetched,
        // the clash's file is staged from the vault's copy kept for the card
        fs::remove_file(join(&k, other)).unwrap();
        let mut b = read_base();
        b["files"].as_object_mut().unwrap().remove(other);
        write_atomic(&base_file(), b.to_string().as_bytes()).unwrap();
        s.log.lock().unwrap().clear();
        let r = update(false);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(fetched(&s), vec![format!("/o/r/{T2}/{other}")]);
        assert_eq!(fs::read_to_string(join(&k, other)).unwrap(), "other\n");
        assert_eq!(clashes()["clashes"][0]["unit"], GLOSSARY);
        // Keep the vault's: the vault's version is in use
        let r = decide_clash(GLOSSARY, "vault");
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(fs::read_to_string(join(&k, GLOSSARY)).unwrap(), two[0].1);
        assert_eq!(clashes(), json!({"ok": true, "clashes": []}));
    }

    #[test]
    fn guides_download_like_skills() {
        let (_g, s) = setup();
        let files = vec![(GLOSSARY, "A glossary of Roblox words.\n".to_string()), (ALPHA, skill("vault-alpha", "zebra crossings"))];
        release(&s, T1, &files);
        let r = update(true);
        assert_eq!(r["ok"], true, "{r}");
        let mut added: Vec<&str> = r["added"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        added.sort();
        assert_eq!(added, vec![GLOSSARY, ALPHA]);
        for (p, text) in &files {
            assert_eq!(&fs::read_to_string(join(&knowledge(), p)).unwrap(), text);
        }
    }
}
