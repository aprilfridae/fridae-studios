//! The shared skill vault's Download and Check for updates: the latest release's manifest is read, only the files
//! that changed are fetched (each checked against the manifest's hash and size) into a staging folder, and only when
//! every one is right is the folder handed to `rbx skills vault-apply`, which puts the skills into
//! `<home>/Knowledge/Skills` without overriding the user. Anything wrong or cut off removes the staging folder and
//! changes nothing. No sign-in is needed.
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
/// The folders of the vault a manifest may name (rbxvaultsync.ALLOWED_TOP).
const ALLOWED_TOP: [&str; 1] = ["Skills/"];
/// At most 5,000 files, 1 MiB each, 100 MiB in all.
const MAX_FILES: usize = 5000;
const MAX_FILE: u64 = 1 << 20;
const MAX_TOTAL: u64 = 100 << 20;
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

/// rbxvaultsync.check_path: a safe vault-relative path, under an allowed top folder, inside a skill's folder, with no
/// empty or dot-started segment, no backslash or NUL, no leading slash, and every segment at most 255 bytes.
pub fn check_path(p: &str) -> bool {
    if !ALLOWED_TOP.iter().any(|t| p.starts_with(t)) || p.contains('\\') || p.contains('\0') || p.starts_with('/') {
        return false;
    }
    let parts: Vec<&str> = p.split('/').collect();
    parts.len() >= 3 && parts.iter().all(|s| !s.is_empty() && !s.starts_with('.') && s.len() <= 255)
}

/// A release name that is safe as a folder name (the staging folder is named by it).
fn release_ok(r: &str) -> bool {
    !r.is_empty() && r.len() <= 100 && !r.starts_with('.') && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
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
fn vault_app() -> PathBuf {
    accounts::base().join("fridae-vault")
}

fn staging_root() -> PathBuf {
    vault_app().join("staging")
}

/// `base` joined with a '/'-separated path, segment by segment.
fn join(base: &Path, path: &str) -> PathBuf {
    path.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
}

/// The base record, whole; empty when there is none or it cannot be read.
fn read_base() -> Value {
    fs::read(base_file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// Whether the user has pressed Download (the base record exists).
fn downloaded() -> bool {
    base_file().is_file()
}

/// A path as this machine keeps it: a skill the user renamed lives in its new folder (rbxvaultsync._local_path).
fn local_path(path: &str, base: &Value) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 2 {
        if let Some(to) = base["renamed"][parts[1]].as_str() {
            parts[1] = to;
        }
    }
    parts.join("/")
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

/// Take the one running place; false when an update is already under way.
fn claim() -> bool {
    !std::mem::replace(&mut *RUNNING.lock().unwrap_or_else(|e| e.into_inner()), true)
}

fn release_claim() {
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run `update` in the background (one at a time): Download is `first`, Check for updates is not.
pub fn start(first: bool) -> Value {
    if !claim() {
        return json!({"ok": false, "busy": true, "error": BUSY});
    }
    std::thread::spawn(move || {
        update(first);
        release_claim();
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
        std::thread::sleep(EVERY);
    }
}

fn tick() {
    if downloaded() && claim() {
        update(false);
        release_claim();
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
    if base["release"].as_str() == Some(&tag) {
        // the same release: what the user deleted, declined or has waiting is no reason to download it again
        let listed = |key: &str, name: &str| base[key].as_array().is_some_and(|l| l.iter().any(|v| v.as_str() == Some(name)));
        need.retain(|f| {
            let skill = f.path.split('/').nth(1).unwrap_or("");
            !listed("deleted", &f.path) && !listed("declined", skill) && base["waiting"].get(skill).is_none()
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
fn remove(p: &Path) {
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
fn real_dir(p: &Path) -> Result<(), String> {
    if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(NO_ROOM.to_string());
    }
    fs::create_dir_all(p).map_err(|_| NO_ROOM.to_string())?;
    if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) { Ok(()) } else { Err(NO_ROOM.to_string()) }
}

/// A file written through a temporary one and a rename, never through a link.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
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
/// checked, the others copied from where the same bytes already are (the user's file or the base record's copy) and
/// fetched only when neither has them. Then apply it.
#[allow(clippy::too_many_arguments)]
fn stage_and_apply(repo: &str, tag: &str, m: &Manifest, man_bytes: &[u8], base: &Value, need: &[ManFile], dir: &Path, manifest: &Path) -> Result<Value, String> {
    real_dir(&vault_app())?;
    real_dir(&staging_root())?;
    remove(dir);
    remove(manifest);
    let know = knowledge();
    let record_copies = know.join(".fridae-vault").join("base");
    for f in &m.files {
        let held = if need.contains(f) {
            None
        } else {
            local_bytes(f, base, &know).filter(|b| right(f, b)).or_else(|| read_plain(&join(&record_copies, &f.path)).filter(|b| right(f, b)))
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
        write_atomic(&join(dir, &f.path), &bytes)?;
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
        for bad in ["Skills/../x", "/etc/x", "Skills/a/.git/config", "Skills\\a", "Notes/a.md", "Skills//a", "Skills/README.md", "Skills/a/b\0c", "Skills/a/..", "Skills/a/", long.as_str()] {
            assert!(!check_path(bad), "{bad:?}");
            assert!(parse_manifest(&man(vec![entry(ALPHA), entry(bad)])).is_err(), "{bad:?}");
        }
        assert!(check_path(&format!("Skills/a/{}", "x".repeat(255))));
        assert!(check_path("Skills/a/b/c.md"));
        // the limits
        let many: Vec<Value> = (0..5001).map(|i| entry(&format!("Skills/s{i}/SKILL.md"))).collect();
        assert!(parse_manifest(&man(many.clone())).is_err(), "5,001 files");
        assert!(parse_manifest(&man(many[..5000].to_vec())).is_ok(), "5,000 files");
        assert!(parse_manifest(&man(vec![json!({"path": ALPHA, "sha256": sha(b"x"), "size": 2 * 1024 * 1024})])).is_err(), "2 MiB");
        assert!(parse_manifest(&man(vec![json!({"path": ALPHA, "sha256": sha(b"x"), "size": 1024 * 1024})])).is_ok(), "1 MiB");
        let big: Vec<Value> = (0..101).map(|i| json!({"path": format!("Skills/s{i}/SKILL.md"), "sha256": sha(b"x"), "size": 1024 * 1024})).collect();
        assert!(parse_manifest(&man(big)).is_err(), "over 100 MiB in all");
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
        assert!(claim());
        assert_eq!(start(false)["busy"], true);
        assert_eq!(status()["running"], true);
        release_claim();
        assert_eq!(status()["running"], false);
    }
}
