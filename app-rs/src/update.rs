//! Auto-update for the installed app.
//!
//! Releases live on GitHub (paths::UPDATE_REPO). The app asks GitHub for the
//! latest release, downloads its zip when it is newer, checks the download
//! against the checksum GitHub reports, unpacks it beside the work folder, and
//! swaps it in with a tiny script that runs after the app has closed.

use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::paths::{self, EXE_NAME, PATHS, UPDATE_REPO, VERSION};

fn stage() -> PathBuf {
    PATHS.home.join("update")
}

fn parse(v: &str) -> Vec<u64> {
    let mut out: Vec<u64> = v
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
        .collect();
    while out.len() < 3 {
        out.push(0);
    }
    out
}

pub fn newer(a: &str, b: &str) -> bool {
    parse(a) > parse(b)
}

fn http(seconds: u64) -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(seconds)))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .build()
        .into()
}

struct Release {
    version: String,
    url: String,
    size: u64,
    sha256: String,
    notes: String,
}

/// Latest published release, or None if there is not one.
fn check() -> Result<Option<Release>, String> {
    if UPDATE_REPO.is_empty() {
        return Ok(None);
    }
    let mut resp = http(20)
        .get(format!("https://api.github.com/repos/{UPDATE_REPO}/releases/latest"))
        .header("User-Agent", "fridae-studios-updater")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let rel: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let Some(asset) = rel["assets"].as_array().into_iter().flatten().find(|a| a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".zip")) else {
        return Ok(None);
    };
    let url = asset["browser_download_url"].as_str().unwrap_or("").to_string();
    if !url.starts_with(&format!("https://github.com/{UPDATE_REPO}/")) {
        return Ok(None); // only ever download from the project's own releases
    }
    let digest = asset["digest"].as_str().unwrap_or("");
    Ok(Some(Release {
        version: rel["tag_name"].as_str().unwrap_or("").trim_start_matches(['v', 'V']).to_string(),
        url,
        size: asset["size"].as_u64().unwrap_or(0),
        sha256: digest.strip_prefix("sha256:").unwrap_or("").to_string(),
        notes: rel["body"].as_str().unwrap_or("").trim().chars().take(1200).collect(),
    }))
}

/// The version that is downloaded and waiting to be installed, if any.
pub fn staged_version() -> Option<String> {
    let info: Value = serde_json::from_str(&fs::read_to_string(stage().join("ready.json")).ok()?).ok()?;
    let folder = PathBuf::from(info["folder"].as_str()?);
    folder.join(EXE_NAME).is_file().then(|| info["version"].as_str().unwrap_or("").to_string()).filter(|v| !v.is_empty())
}

pub fn clear_stage() {
    let _ = fs::remove_dir_all(stage());
}

/// Download and unpack a release next to the work folder.
fn download(info: &Release, progress: impl Fn(f64)) -> Result<(), String> {
    let fail = |msg: &str| {
        clear_stage();
        Err(msg.to_string())
    };
    clear_stage();
    fs::create_dir_all(stage()).map_err(|e| e.to_string())?;
    let zpath = stage().join("update.zip");
    let mut resp = http(900).get(&info.url).header("User-Agent", "fridae-studios-updater").call().map_err(|e| e.to_string())?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(info.size);
    let mut reader = resp.body_mut().as_reader();
    let mut file = fs::File::create(&zpath).map_err(|e| e.to_string())?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 18];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        sha.update(&buf[..n]);
        done += n as u64;
        if total > 0 {
            progress(done as f64 / total as f64);
        }
    }
    drop(file);
    if !info.sha256.is_empty() && paths::hex(&sha.finalize()) != info.sha256.to_lowercase() {
        return fail("The download did not match its checksum, so it was thrown away.");
    }
    let out = stage().join("files");
    let mut zip = zip::ZipArchive::new(fs::File::open(&zpath).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            return fail("The update archive contains an unsafe path.");
        };
        let target = out.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
    }
    drop(zip);
    let _ = fs::remove_file(&zpath);
    let mut folder = out.clone();
    if !folder.join(EXE_NAME).is_file() {
        let inner = fs::read_dir(&out).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.join(EXE_NAME).is_file());
        match inner {
            Some(p) => folder = p,
            None => return fail("The update archive does not contain the program."),
        }
    }
    fs::write(stage().join("ready.json"), json!({"version": info.version, "folder": folder.display().to_string()}).to_string()).map_err(|e| e.to_string())
}

/// Swap the staged update in once this process has exited. The caller must exit right after.
pub fn apply(relaunch: bool) -> bool {
    if !paths::packaged() || !cfg!(windows) {
        return false;
    }
    let Some(info) = fs::read_to_string(stage().join("ready.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
        return false;
    };
    let Some(folder) = info["folder"].as_str() else { return false };
    let version = info["version"].as_str().unwrap_or("");
    // the new program installs itself, showing a splash (splash.rs); no console programs, so no console windows
    let mut cmd = Command::new(PathBuf::from(folder).join(EXE_NAME));
    cmd.arg("--apply-update").arg(std::process::id().to_string()).arg(&PATHS.install).arg(version);
    if relaunch {
        cmd.arg("relaunch");
    }
    cmd.current_dir(folder).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().is_ok()
}

/// Remove the download folder left by an installed update. The installer may still be closing, so keep trying
/// for a little while in the background.
pub fn clear_stage_later() {
    thread::spawn(|| {
        for _ in 0..30 {
            clear_stage();
            if !stage().exists() {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

// ------------------------------------------------------------------ checker
static STATE: Mutex<Option<Value>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);

fn with_state<T>(f: impl FnOnce(&mut Value) -> T) -> T {
    let mut guard = STATE.lock().unwrap();
    let state = guard.get_or_insert_with(|| {
        let auto = paths::load_config().get("auto_update").and_then(Value::as_bool).unwrap_or(true);
        json!({"current": VERSION, "latest": null, "status": "idle", "progress": 0, "notes": "", "error": "", "auto": auto,
               "can_install": paths::packaged() && cfg!(windows)})
    });
    f(state)
}

/// What the window shows about updates.
pub fn state() -> Value {
    with_state(|s| s.clone())
}

fn set(key: &str, value: Value) {
    with_state(|s| s[key] = value);
}

pub fn set_auto(on: bool) {
    set("auto", json!(on));
    paths::update_config(|cfg| cfg.insert("auto_update".into(), json!(on)));
}

fn auto() -> bool {
    state()["auto"].as_bool().unwrap_or(true)
}

pub fn ready() -> bool {
    state()["status"] == "ready"
}

/// Check for a newer release and, when asked to (or when automatic updates are on), download it.
pub fn run(download_too: Option<bool>) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    set("status", json!("checking"));
    set("error", json!(""));
    let outcome: Result<&str, String> = (|| {
        let info = check()?;
        let Some(info) = info.filter(|i| newer(&i.version, VERSION)) else {
            if staged_version().is_some_and(|v| !newer(&v, VERSION)) {
                clear_stage();
            }
            return Ok("current");
        };
        set("latest", json!(info.version));
        set("notes", json!(info.notes));
        if staged_version().as_deref() == Some(info.version.as_str()) {
            return Ok("ready");
        }
        let can_install = state()["can_install"].as_bool().unwrap_or(false);
        if !can_install || !download_too.unwrap_or_else(auto) {
            return Ok("available");
        }
        set("status", json!("downloading"));
        set("progress", json!(0));
        download(&info, |f| set("progress", json!((f * 1000.0).round() / 1000.0)))?;
        Ok("ready")
    })();
    match outcome {
        Ok("current") => {
            if state()["latest"].is_null() {
                set("latest", json!(VERSION));
            }
            set("status", json!("current"));
        }
        Ok(status) => set("status", json!(status)),
        Err(e) => {
            set("status", json!("error"));
            set("error", json!(e.chars().take(200).collect::<String>()));
        }
    }
    BUSY.store(false, Ordering::SeqCst);
}

/// Check now and then for as long as the app is open.
pub fn run_forever() {
    loop {
        run(None);
        thread::sleep(Duration::from_secs(6 * 3600));
    }
}
