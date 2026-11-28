//! Auto-update for the installed app.
//!
//! Releases live on GitHub (paths::UPDATE_REPO). The app asks GitHub for the latest release and picks the file made
//! for its own system (`pick_asset`): Windows `Fridae.zip` (the release's only .zip: older Windows copies take the
//! first .zip they see), macOS `Fridae-macOS.dmg`, Linux `Fridae-linux-<arch>.AppImage` or `.tar.gz`. When it is newer
//! it is downloaded and checked against the checksum GitHub reports.
//!
//! Installing: Windows and the Linux folder (.tar.gz) unpack it beside the work folder and the new program copies itself
//! over the install folder after the app has closed (splash.rs). A Linux AppImage is downloaded beside itself and
//! renamed over itself (one atomic step), then started again. A Mac cannot swap its own .app from a disk image, so it
//! says a version is out and opens the release page.

use crate::paths::Start;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
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

use crate::paths::{self, EXE_NAME, OLD_EXE_NAME, PATHS, UPDATE_REPO, VERSION};

/// The program in an unpacked update: `Fridae.exe`, or `Fridae Studios.exe` in a release from before the rename.
fn program_in(folder: &std::path::Path) -> Option<PathBuf> {
    [EXE_NAME, OLD_EXE_NAME].iter().map(|n| folder.join(n)).find(|p| p.is_file())
}

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

/// Which build of the app this is, which decides the release file it takes and whether it can install it itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Build {
    Windows,
    Mac,
    /// started from an AppImage (`$APPIMAGE` names it)
    LinuxAppImage,
    /// the plain Linux folder from the .tar.gz
    LinuxTar,
}

/// The AppImage this copy runs from (its runtime sets `$APPIMAGE`).
fn appimage() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_file())
}

pub fn this_build() -> Build {
    if cfg!(windows) {
        Build::Windows
    } else if cfg!(target_os = "macos") {
        Build::Mac
    } else if appimage().is_some() {
        Build::LinuxAppImage
    } else {
        Build::LinuxTar
    }
}

/// Whether a build installs its own updates (a Mac opens the release page instead).
fn installs_itself(build: Build) -> bool {
    build != Build::Mac
}

/// The names one processor goes by in file names.
fn arch_names(arch: &str) -> &'static [&'static str] {
    match arch {
        "x86_64" => &["x86_64", "amd64", "x64"],
        "aarch64" => &["aarch64", "arm64"],
        _ => &[],
    }
}

/// The release file for a build on a processor (`std::env::consts::ARCH`). Windows takes `Fridae.zip`, or else the
/// first .zip that names no other system; a Mac a .dmg, or a file naming macOS; Linux an .AppImage (when it runs from
/// one) or a .tar.gz that names Linux and, when the name gives a processor, its own. A Mac or Linux copy never takes
/// a .zip that is not marked as its own, so it can never pick the Windows `Fridae.zip`.
pub fn pick_asset<'a>(assets: &'a [Value], build: Build, arch: &str) -> Option<&'a Value> {
    let name = |a: &Value| a["name"].as_str().unwrap_or("").to_lowercase();
    let names_other_system = |n: &str| ["mac", "darwin", "linux", "appimage"].iter().any(|w| n.contains(w));
    let all_arches = ["x86_64", "amd64", "x64", "aarch64", "arm64"];
    let fits_arch = |n: &str| arch_names(arch).iter().any(|a| n.contains(a)) || !all_arches.iter().any(|a| n.contains(a));
    match build {
        Build::Windows => assets.iter().find(|a| name(a) == "fridae.zip").or_else(|| assets.iter().find(|a| name(a).ends_with(".zip") && !names_other_system(&name(a)))),
        Build::Mac => assets.iter().find(|a| name(a).ends_with(".dmg")).or_else(|| assets.iter().find(|a| ["macos", "darwin"].iter().any(|w| name(a).contains(w)))),
        Build::LinuxAppImage => assets.iter().find(|a| name(a).ends_with(".appimage") && fits_arch(&name(a))),
        Build::LinuxTar => assets.iter().find(|a| name(a).ends_with(".tar.gz") && name(a).contains("linux") && fits_arch(&name(a))),
    }
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
    /// the asset's file name, which says how it is unpacked
    name: String,
    size: u64,
    sha256: String,
    notes: String,
    /// the release's page on GitHub, for a copy that cannot install it itself
    page: String,
}

/// What the latest release holds for this copy.
enum Found {
    /// no release, or none that can be read
    Nothing,
    /// a release with this system's file
    Release(Release),
    /// a release without a file for this system (a Mac or Linux copy never takes the Windows zip): named, not offered
    NotForThisSystem { version: String, page: String },
}

/// The status of a newer release with no file for this system: the window says so, and the app keeps checking.
pub const UNAVAILABLE: &str = "unavailable";

/// Latest published release.
fn check() -> Result<Found, String> {
    if UPDATE_REPO.is_empty() {
        return Ok(Found::Nothing);
    }
    let mut resp = http(20)
        .get(format!("https://api.github.com/repos/{UPDATE_REPO}/releases/latest"))
        .header("User-Agent", "fridae-studios-updater")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let rel: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(release_of(&rel, this_build(), std::env::consts::ARCH, &format!("https://github.com/{UPDATE_REPO}/")))
}

/// What a release (GitHub's latest-release answer) holds for a build on a processor. Only files and pages of the
/// project's own releases (`ours`) count.
fn release_of(rel: &Value, build: Build, arch: &str, ours: &str) -> Found {
    let Some(tag) = rel["tag_name"].as_str() else {
        return Found::Nothing;
    };
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let page = rel["html_url"].as_str().filter(|p| p.starts_with(ours)).unwrap_or("").to_string();
    let assets = rel["assets"].as_array().cloned().unwrap_or_default();
    let Some(asset) = pick_asset(&assets, build, arch) else {
        return Found::NotForThisSystem { version, page };
    };
    let url = asset["browser_download_url"].as_str().unwrap_or("").to_string();
    if !url.starts_with(ours) {
        return Found::Nothing; // only ever download from the project's own releases
    }
    let digest = asset["digest"].as_str().unwrap_or("");
    Found::Release(Release {
        version,
        url,
        name: asset["name"].as_str().unwrap_or("").to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        sha256: digest.strip_prefix("sha256:").unwrap_or("").to_string(),
        notes: rel["body"].as_str().unwrap_or("").trim().chars().take(1200).collect(),
        page,
    })
}

/// The version that is downloaded and waiting to be installed, if any.
pub fn staged_version() -> Option<String> {
    let info: Value = serde_json::from_str(&fs::read_to_string(stage().join("ready.json")).ok()?).ok()?;
    let ready = match (info["folder"].as_str(), info["appimage"].as_str()) {
        (Some(folder), _) => program_in(Path::new(folder)).is_some(),
        (None, Some(new)) => Path::new(new).is_file(),
        _ => false,
    };
    ready.then(|| info["version"].as_str().unwrap_or("").to_string()).filter(|v| !v.is_empty())
}

pub fn clear_stage() {
    // a downloaded AppImage waits beside the one it replaces (`appimage_beside`), outside the stage folder
    if let Some(new) = fs::read_to_string(stage().join("ready.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v["appimage"].as_str().map(PathBuf::from)) {
        if appimage().is_some_and(|a| new == appimage_beside(&a)) {
            let _ = fs::remove_file(new);
        }
    }
    let _ = fs::remove_dir_all(stage());
}

/// Where a new AppImage is downloaded: beside the running one (the same folder, so the rename over it is one step),
/// hidden: `/apps/.Fridae.AppImage.new` for `/apps/Fridae.AppImage`.
fn appimage_beside(running: &Path) -> PathBuf {
    let name = running.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Fridae.AppImage".into());
    running.with_file_name(format!(".{name}.new"))
}

/// How a release file is unpacked.
#[derive(Debug, PartialEq)]
enum Archive {
    Zip,
    TarGz,
    AppImage,
}

fn archive_of(name: &str) -> Option<Archive> {
    let n = name.to_lowercase();
    if n.ends_with(".zip") {
        Some(Archive::Zip)
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(Archive::TarGz)
    } else if n.ends_with(".appimage") {
        Some(Archive::AppImage)
    } else {
        None
    }
}

/// Download and unpack a release next to the work folder.
fn download(info: &Release, progress: impl Fn(f64)) -> Result<(), String> {
    let fail = |msg: &str| {
        clear_stage();
        Err(msg.to_string())
    };
    clear_stage();
    fs::create_dir_all(stage()).map_err(|e| e.to_string())?;
    let Some(kind) = archive_of(&info.name) else { return fail("This release has no file this computer can install.") };
    let running_appimage = appimage();
    let zpath = match (&kind, &running_appimage) {
        (Archive::AppImage, Some(running)) => appimage_beside(running),
        (Archive::AppImage, None) => return fail("This copy was not started from an AppImage."),
        (Archive::TarGz, _) => stage().join("update.tar.gz"),
        (Archive::Zip, _) => stage().join("update.zip"),
    };
    let mut resp = http(900).get(&info.url).header("User-Agent", "fridae-studios-updater").call().map_err(|e| e.to_string())?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(info.size);
    let mut reader = resp.body_mut().as_reader();
    let mut file = fs::File::create(&zpath).map_err(|e| match kind {
        Archive::AppImage => format!("Fridae can't write beside its AppImage ({e}). Move the AppImage to a folder you own, or download the new version from the releases page."),
        _ => e.to_string(),
    })?;
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
        if kind == Archive::AppImage {
            let _ = fs::remove_file(&zpath);
        }
        return fail("The download did not match its checksum, so it was thrown away.");
    }
    let out = stage().join("files");
    match kind {
        Archive::AppImage => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&zpath, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            return fs::write(stage().join("ready.json"), json!({"version": info.version, "appimage": zpath.display().to_string()}).to_string()).map_err(|e| e.to_string());
        }
        Archive::TarGz => {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            // the system's tar (every Linux has it); GNU tar drops a leading "/" and refuses members with ".."
            let done = Command::new("tar").arg("-xzf").arg(&zpath).arg("-C").arg(&out).arg("--no-same-owner").stdin(Stdio::null()).run_output().map_err(|e| e.to_string())?;
            if !done.status.success() {
                return fail("The update archive could not be unpacked.");
            }
            let _ = fs::remove_file(&zpath);
            return finish_folder(info, &out);
        }
        Archive::Zip => {}
    }
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
    finish_folder(info, &out)
}

/// An unpacked release: the folder holding its program (the top or the one folder inside) is noted as ready.
fn finish_folder(info: &Release, out: &Path) -> Result<(), String> {
    let mut folder = out.to_path_buf();
    if program_in(&folder).is_none() {
        let inner = fs::read_dir(out).into_iter().flatten().flatten().map(|e| e.path()).find(|p| program_in(p).is_some());
        match inner {
            Some(p) => folder = p,
            None => {
                clear_stage();
                return Err("The update archive does not contain the program.".into());
            }
        }
    }
    fs::write(stage().join("ready.json"), json!({"version": info.version, "folder": folder.display().to_string()}).to_string()).map_err(|e| e.to_string())
}

/// Swap the staged update in once this process has exited. The caller must exit right after.
pub fn apply(relaunch: bool) -> bool {
    if !paths::packaged() || !installs_itself(this_build()) {
        return false;
    }
    let Some(info) = fs::read_to_string(stage().join("ready.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
        return false;
    };
    if let Some(new) = info["appimage"].as_str() {
        return apply_appimage(Path::new(new), relaunch);
    }
    let Some(folder) = info["folder"].as_str() else { return false };
    let version = info["version"].as_str().unwrap_or("");
    // the new program installs itself, showing a splash (splash.rs); no console programs, so no console windows
    let Some(program) = program_in(std::path::Path::new(folder)) else { return false };
    let mut cmd = Command::new(program);
    cmd.arg("--apply-update").arg(std::process::id().to_string()).arg(&PATHS.install).arg(version);
    if relaunch {
        cmd.arg("relaunch");
    }
    cmd.current_dir(folder).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.start().is_ok()
}

/// A Linux AppImage: the downloaded one renamed over the running one (the running copy keeps its open file), then,
/// when asked, started again once this process has ended (so it never meets this copy still closing).
fn apply_appimage(new: &Path, relaunch: bool) -> bool {
    let Some(running) = appimage() else { return false };
    if new != appimage_beside(&running) || !new.is_file() || fs::rename(new, &running).is_err() {
        return false;
    }
    let _ = fs::remove_dir_all(stage());
    if relaunch {
        let (program, args) = relaunch_after_exit(std::process::id(), &running);
        let _ = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).start();
    }
    true
}

/// The command that waits for process `pid` to end, then starts `program` (a POSIX shell: the path is passed as an
/// argument, never put into the script's text).
fn relaunch_after_exit(pid: u32, program: &Path) -> (String, Vec<String>) {
    let script = format!("while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; exec \"$0\"");
    ("/bin/sh".into(), vec!["-c".into(), script, program.display().to_string()])
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
               "can_install": paths::packaged() && installs_itself(this_build()), "page": ""})
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
        let info = match check()? {
            Found::Release(r) => Some(r),
            Found::NotForThisSystem { version, page } if newer(&version, VERSION) => {
                set("latest", json!(version));
                set("page", json!(page));
                return Ok(UNAVAILABLE);
            }
            _ => None,
        };
        let Some(info) = info.filter(|i| newer(&i.version, VERSION)) else {
            if staged_version().is_some_and(|v| !newer(&v, VERSION)) {
                clear_stage();
            }
            return Ok("current");
        };
        set("latest", json!(info.version));
        set("notes", json!(info.notes));
        set("page", json!(info.page));
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

/// Open the latest release's page (a Mac downloads the new version there). Only a page of the project's own releases.
pub fn open_page() -> bool {
    let page = state()["page"].as_str().unwrap_or("").to_string();
    page.starts_with(&format!("https://github.com/{UPDATE_REPO}/")) && paths::open_that(page).is_ok()
}

/// Check now and then for as long as the app is open.
pub fn run_forever() {
    loop {
        run(None);
        thread::sleep(Duration::from_secs(6 * 3600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_may_carry_the_program_under_either_name() {
        let root = paths::test_dir("update-program-in");
        let (new, old, both, none) = (root.join("new"), root.join("old"), root.join("both"), root.join("none"));
        for d in [&new, &old, &both, &none] {
            fs::create_dir_all(d).unwrap();
        }
        let (exe, old_exe) = (paths::EXE_NAME, paths::OLD_EXE_NAME);
        fs::write(new.join(exe), b"").unwrap();
        fs::write(old.join(old_exe), b"").unwrap();
        fs::write(both.join(exe), b"").unwrap();
        fs::write(both.join(old_exe), b"").unwrap();
        assert_eq!(program_in(&new), Some(new.join(exe)));
        assert_eq!(program_in(&old), Some(old.join(old_exe)), "a release from before the rename");
        assert_eq!(program_in(&both), Some(both.join(exe)), "the new name first");
        assert_eq!(program_in(&none), None);
        assert_eq!((paths::APP, paths::DATA_DIR), ("Fridae", "Fridae Studios"));
        assert_eq!(paths::EXE_NAME, if cfg!(windows) { "Fridae.exe" } else { "Fridae" });
    }

    fn release(names: &[&str]) -> Vec<Value> {
        names.iter().map(|n| json!({"name": n})).collect()
    }

    fn picked(assets: &[Value], build: Build, arch: &str) -> Option<String> {
        pick_asset(assets, build, arch).map(|a| a["name"].as_str().unwrap().to_string())
    }

    #[test]
    fn each_system_takes_its_own_release_file() {
        let all = release(&["Fridae-linux-x86_64.tar.gz", "Fridae-macOS.dmg", "Fridae-linux-aarch64.AppImage", "Fridae-linux-x86_64.AppImage", "Fridae.zip"]);
        assert_eq!(picked(&all, Build::Windows, "x86_64").as_deref(), Some("Fridae.zip"), "Windows: Fridae.zip wherever it is in the list");
        assert_eq!(picked(&all, Build::Mac, "aarch64").as_deref(), Some("Fridae-macOS.dmg"));
        assert_eq!(picked(&all, Build::LinuxAppImage, "x86_64").as_deref(), Some("Fridae-linux-x86_64.AppImage"), "its own processor");
        assert_eq!(picked(&all, Build::LinuxAppImage, "aarch64").as_deref(), Some("Fridae-linux-aarch64.AppImage"));
        assert_eq!(picked(&all, Build::LinuxTar, "x86_64").as_deref(), Some("Fridae-linux-x86_64.tar.gz"));
        assert_eq!(picked(&all, Build::LinuxTar, "aarch64"), None, "no .tar.gz for this processor: nothing, never another's");
    }

    #[test]
    fn a_release_without_this_systems_file_is_named_not_offered() {
        let ours = "https://github.com/me/app/";
        let rel = json!({"tag_name": "v9.9.9", "html_url": "https://github.com/me/app/releases/tag/v9.9.9",
                         "assets": [{"name": "Fridae.zip", "browser_download_url": "https://github.com/me/app/releases/download/v9.9.9/Fridae.zip", "size": 5}]});
        for build in [Build::Mac, Build::LinuxAppImage, Build::LinuxTar] {
            match release_of(&rel, build, "x86_64", ours) {
                Found::NotForThisSystem { version, page } => {
                    assert_eq!(version, "9.9.9");
                    assert_eq!(page, "https://github.com/me/app/releases/tag/v9.9.9");
                }
                _ => panic!("{build:?}: the Windows zip is never offered"),
            }
        }
        assert!(matches!(release_of(&rel, Build::Windows, "x86_64", ours), Found::Release(r) if r.name == "Fridae.zip" && r.version == "9.9.9"));
        assert!(matches!(release_of(&json!({"tag_name": "v1", "assets": []}), Build::Windows, "x86_64", ours), Found::NotForThisSystem { .. }));
        assert!(matches!(release_of(&json!({}), Build::Mac, "x86_64", ours), Found::Nothing), "no release at all");
        assert_eq!(UNAVAILABLE, "unavailable");
    }

    #[test]
    fn a_mac_or_linux_copy_never_takes_the_windows_zip() {
        let windows_only = release(&["Fridae.zip"]);
        for build in [Build::Mac, Build::LinuxAppImage, Build::LinuxTar] {
            assert_eq!(picked(&windows_only, build, "x86_64"), None, "{build:?}");
        }
        assert_eq!(picked(&windows_only, Build::Windows, "x86_64").as_deref(), Some("Fridae.zip"));
        // Windows never takes a zip that names another system, even with no Fridae.zip
        assert_eq!(picked(&release(&["Fridae-macOS.zip", "Fridae-linux.zip"]), Build::Windows, "x86_64"), None);
        assert_eq!(picked(&release(&["Fridae-macOS.zip"]), Build::Mac, "x86_64").as_deref(), Some("Fridae-macOS.zip"), "a zip marked as the Mac's is the Mac's");
    }

    #[test]
    fn a_release_file_says_how_it_is_unpacked() {
        assert_eq!(archive_of("Fridae.zip"), Some(Archive::Zip));
        assert_eq!(archive_of("Fridae-linux-x86_64.tar.gz"), Some(Archive::TarGz));
        assert_eq!(archive_of("Fridae-linux-x86_64.AppImage"), Some(Archive::AppImage));
        assert_eq!(archive_of("Fridae-macOS.dmg"), None, "a Mac's disk image is opened by the user");
        assert!(!installs_itself(Build::Mac) && installs_itself(Build::Windows) && installs_itself(Build::LinuxAppImage) && installs_itself(Build::LinuxTar));
    }

    #[test]
    fn a_new_appimage_waits_hidden_beside_the_running_one_and_starts_after_it_ends() {
        assert_eq!(appimage_beside(Path::new("/apps/Fridae.AppImage")), Path::new("/apps/.Fridae.AppImage.new"));
        let (program, args) = relaunch_after_exit(42, Path::new("/apps/my apps/Fridae.AppImage"));
        assert_eq!(program, "/bin/sh");
        assert_eq!(args, ["-c", "while kill -0 42 2>/dev/null; do sleep 0.2; done; exec \"$0\"", "/apps/my apps/Fridae.AppImage"], "the path is an argument, never in the script");
    }
}
