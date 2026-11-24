//! Finished promos. The promo tool drops a card file when a render is done (and rewrites it when the
//! promo is rendered again into the same folder); this tells the window, which puts it in the Promos
//! tab's library with a line in the chat that asked for it, and does what the card's buttons ask.
//! The tab's listing, New promo form and style previews are in promo_tab.rs.
//!
//!   work/promo-cards/<id>.json   {"id", "agent", "folder", "video", "cover", "caption_text",
//!                                 "style", "duration", "check": {"passed", "problems", "notes"}, "created"}
//!
//! Card files stay: they are the only record of which files the window may play.
//!
//! Setup's "Promo engine" row also lives here: `rbx promo status --json` for its state, and
//! `rbx promo install --json-progress` run in the background, its progress lines passed on as
//! `promo_engine` events.

use crate::paths::Start;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use crate::{agents::{self, BUS}, attach, paths};

pub const VIDEO_EXT: [&str; 4] = ["mp4", "mov", "mkv", "webm"];
pub const AUDIO_EXT: [&str; 4] = ["mp3", "wav", "m4a", "ogg"];

fn dir() -> PathBuf {
    paths::work("promo-cards")
}

/// A card id is a plain file name stem: never a path the page could steer elsewhere.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains("..") && !id.contains(['/', '\\', ':', '\0'])
}

fn card_in(dir: &Path, id: &str) -> Option<Value> {
    if !valid_id(id) {
        return None;
    }
    let text = fs::read_to_string(dir.join(format!("{id}.json"))).ok()?;
    serde_json::from_str::<Value>(&text).ok().filter(Value::is_object)
}

fn card(id: &str) -> Option<Value> {
    card_in(&dir(), id)
}

fn path_of(card: &Value, key: &str) -> Option<PathBuf> {
    card[key].as_str().filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// The file `/media/promo/<id>/<what>` may serve, and its type: only what the card names.
fn media_in(dir: &Path, id: &str, what: &str) -> Option<(PathBuf, &'static str)> {
    let card = card_in(dir, id)?;
    match what {
        "video" => Some((path_of(&card, "video")?, "video/mp4")),
        "cover" => Some((path_of(&card, "cover")?, "image/jpeg")),
        _ => None,
    }
}

pub fn media(id: &str, what: &str) -> Option<(PathBuf, &'static str)> {
    media_in(&dir(), id, what)
}

/// Which chat shows the card: the one that made it, or "" for whichever chat is in front.
pub fn card_route_target(wanted: &str, exists: impl Fn(&str) -> bool) -> String {
    if !wanted.is_empty() && exists(wanted) { wanted.to_string() } else { String::new() }
}

/// The command that shows a file selected in the system's file browser.
pub fn reveal_cmd(path: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open".into(), vec!["-R".into(), path.into()])
    } else if cfg!(windows) {
        ("explorer".into(), vec![format!("/select,{path}")])
    } else {
        let parent = Path::new(path).parent().map(|p| p.display().to_string()).unwrap_or_else(|| ".".into());
        ("xdg-open".into(), vec![parent])
    }
}

fn reveal(card: &Value) -> bool {
    let Some(video) = path_of(card, "video").filter(|p| p.exists()) else { return false };
    let (cmd, args) = reveal_cmd(&video.display().to_string());
    let mut c = Command::new(cmd);
    c.args(args);
    paths::quiet(&mut c);
    c.start().is_ok()
}

/// Copy the video to `dest`; with `extras`, the cover and caption beside it under the same base name.
fn copy_out(card: &Value, dest: &Path, extras: bool) -> Result<(), String> {
    let video = path_of(card, "video").ok_or("The card names no video.")?;
    fs::copy(&video, dest).map_err(|e| format!("Could not copy the video: {e}"))?;
    if extras {
        if let Some(cover) = path_of(card, "cover").filter(|p| p.exists()) {
            fs::copy(cover, dest.with_extension("jpg")).map_err(|e| format!("Could not copy the cover: {e}"))?;
        }
        if let Some(folder) = path_of(card, "folder") {
            let caption = folder.join("caption.txt");
            if caption.exists() {
                fs::copy(caption, dest.with_extension("txt")).map_err(|e| format!("Could not copy the caption: {e}"))?;
            }
        }
    }
    Ok(())
}

fn save(card: &Value, extras: bool) -> Value {
    let name = path_of(card, "folder").and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "promo".into());
    let picked = rfd::FileDialog::new()
        .set_title("Save the promo")
        .set_directory(paths::home_dir().join("Downloads"))
        .set_file_name(format!("{name}.mp4"))
        .add_filter("MP4 video", &["mp4"])
        .save_file();
    let Some(mut dest) = picked else { return json!({"ok": false, "cancelled": true}) };
    if dest.extension().is_none() {
        dest.set_extension("mp4");
    }
    match copy_out(card, &dest, extras) {
        Ok(()) => json!({"ok": true, "path": dest.display().to_string()}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// A temporary name beside `path` that no other writer (another request, the promo tool) uses.
fn tmp_beside(path: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// Write `bytes` to `path` through a temporary file, so a reader never finds half of it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> bool {
    let tmp = tmp_beside(path);
    let ok = fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_ok();
    if !ok {
        let _ = fs::remove_file(&tmp);
    }
    ok
}

/// Write the edited post caption back to caption.txt in the card's own folder.
fn write_caption(card: &Value, text: &str) -> bool {
    let Some(folder) = path_of(card, "folder").filter(|f| f.is_dir()) else { return false };
    write_atomic(&folder.join("caption.txt"), text.as_bytes())
}

/// The card's own copy of the caption, so the library shows the edit after a restart. The watcher's note of
/// the file is updated under the same lock, so the app's own edit is never taken for a new render.
fn write_card_caption(dir: &Path, id: &str, text: &str, seen: &mut HashMap<String, SystemTime>) -> bool {
    let Some(mut card) = card_in(dir, id) else { return false };
    card["caption_text"] = json!(text);
    let path = dir.join(format!("{id}.json"));
    let Ok(bytes) = serde_json::to_vec_pretty(&card) else { return false };
    if !write_atomic(&path, &bytes) {
        return false;
    }
    if let Some(m) = mtime(&path) {
        seen.insert(id.to_string(), m);
    }
    true
}

/// POST /api/promo {action: "reveal" | "save" | "caption", id, extras?, text?}
pub fn act(body: &Value) -> Value {
    let Some(card) = card(body["id"].as_str().unwrap_or("")) else { return json!({"ok": false, "error": "That promo is gone."}) };
    match body["action"].as_str().unwrap_or("") {
        "reveal" => json!({"ok": reveal(&card)}),
        "save" => save(&card, body["extras"].as_bool().unwrap_or(false)),
        "caption" => {
            let text = body["text"].as_str().unwrap_or("");
            let on_disk = write_caption(&card, text);
            let in_card = write_card_caption(&dir(), body["id"].as_str().unwrap_or(""), text, &mut SEEN.lock().unwrap());
            json!({"ok": on_disk && in_card})
        }
        _ => json!({"ok": false, "error": "Unknown action."}),
    }
}

/// POST /api/pick-files {kind: "clips" | "music" | "reference"}: paths the user chose, never copied.
pub fn pick_files(kind: &str) -> Vec<String> {
    let dialog = rfd::FileDialog::new();
    let picked = match kind {
        "clips" => dialog.set_title("Add clips").add_filter("Video", &VIDEO_EXT).pick_files(),
        "music" => dialog.set_title("Choose music").add_filter("Audio", &AUDIO_EXT).pick_file().map(|p| vec![p]),
        "reference" => dialog.set_title("Choose a reference video").add_filter("Video", &VIDEO_EXT).pick_file().map(|p| vec![p]),
        _ => None,
    };
    picked.unwrap_or_default().into_iter().map(|p| p.display().to_string()).collect()
}

#[cfg_attr(windows, allow(dead_code))]
fn is_media(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    VIDEO_EXT.contains(&ext.as_str()) || AUDIO_EXT.contains(&ext.as_str())
}

/// Files dropped on the window (not on Windows, see main.rs). Videos and music go to the message box as
/// `[clip: <path>]` lines (by path, not copied). Returns false, leaving the page to handle the drop as before,
/// when nothing dropped is media; other files dropped together with media are attached here, as the page would.
#[cfg_attr(windows, allow(dead_code))]
pub fn dropped(files: Vec<PathBuf>) -> bool {
    let (media, other): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|p| is_media(p));
    if media.is_empty() {
        return false;
    }
    BUS.put("clips", json!({"paths": media.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()}));
    if !other.is_empty() {
        thread::spawn(move || {
            for path in other {
                BUS.put("attached", attach::attach_path(&path));
            }
        });
    }
    true
}

fn show(card: &Value) {
    let target = card_route_target(card["agent"].as_str().unwrap_or(""), |id| agents::get(id).is_some());
    // the card goes under "card": the bus numbers its events with "id"
    BUS.put("promo", json!({"agent": target, "card": card}));
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn secs(t: SystemTime) -> f64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The card files in `dir`: (id, path, when the file last changed).
pub(crate) fn card_files(dir: &Path) -> Vec<(String, PathBuf, SystemTime)> {
    let entries = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path());
    entries
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| {
            let id = p.file_stem()?.to_string_lossy().to_string();
            let m = mtime(&p)?;
            valid_id(&id).then_some((id, p, m))
        })
        .collect()
}

/// A card as the page gets it: the file's contents plus `mtime`, which changes when it is rendered again.
pub fn card_at(dir: &Path, id: &str, m: SystemTime) -> Option<Value> {
    let mut card = card_in(dir, id)?;
    card["mtime"] = json!(secs(m));
    Some(card)
}

/// What is in `dir` now, without reporting any of it.
fn snapshot(dir: &Path) -> HashMap<String, SystemTime> {
    card_files(dir).into_iter().map(|(id, _, m)| (id, m)).collect()
}

/// Cards that are new or were written again since the last look. A card that does not read yet (still being
/// written) is not noted, so it is tried again next time.
fn changed_cards(dir: &Path, seen: &mut HashMap<String, SystemTime>) -> Vec<Value> {
    let mut out = Vec::new();
    for (id, _, m) in card_files(dir) {
        if seen.get(&id) == Some(&m) {
            continue;
        }
        if let Some(card) = card_at(dir, &id, m) {
            seen.insert(id, m);
            out.push(card);
        }
    }
    out
}

/// Which card files the watcher has shown, and as of when.
static SEEN: LazyLock<Mutex<HashMap<String, SystemTime>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Cards already there when the app starts were shown in an earlier run (the Promos tab lists them); new ones,
/// and cards rendered again into the same folder, are shown as they arrive.
pub fn watch() {
    let dir = dir();
    *SEEN.lock().unwrap() = snapshot(&dir);
    loop {
        thread::sleep(Duration::from_millis(700));
        let cards = changed_cards(&dir, &mut SEEN.lock().unwrap());
        for card in cards {
            show(&card);
        }
    }
}

// ---------------------------------------------------------------- the promo engine install (Setup)

/// The install running in the background, or how the last one ended.
struct Install {
    running: bool,
    step: String,
    fraction: f64,
    error: String,
    child: Option<Child>,
    /// The app is closing: a running install is stopped and no new one starts.
    stopping: bool,
}

static INSTALL: Mutex<Install> =
    Mutex::new(Install { running: false, step: String::new(), fraction: 0.0, error: String::new(), child: None, stopping: false });

/// How an install run ended.
#[derive(Debug, PartialEq)]
enum Ending {
    Ready,
    /// Stopped by the app (it is closing), not a failure.
    Stopped,
    Failed(String),
}

/// `done` is the last line's (ready, missing), when it came; `code` the exit code (None when killed).
fn ending(stopping: bool, done: Option<(bool, Vec<String>)>, code: Option<i32>, stderr: &str) -> Ending {
    match done {
        Some((true, _)) if code == Some(0) => Ending::Ready,
        _ if stopping => Ending::Stopped,
        Some((_, missing)) if code == Some(0) => Ending::Failed(format!("The install finished, but these parts are still missing: {}.", missing.join(", "))),
        _ => match error_tail(stderr) {
            why if !why.is_empty() => Ending::Failed(why),
            _ => Ending::Failed(format!("The install stopped (exit {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "without a code".into()))),
        },
    }
}

/// The install command in its own process group (macOS), so stopping it also stops the npm, node and
/// browser downloads it started. On Windows the tree is ended with taskkill /T.
fn install_command() -> Command {
    let mut c = paths::cli(&["promo", "install", "--json-progress"]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

/// Stop the install and everything it started.
pub(crate) fn kill_tree(child: &mut Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    let mut kill = {
        let mut k = Command::new("kill");
        k.args(["-KILL", "--", &format!("-{pid}")]);
        k
    };
    #[cfg(windows)]
    let mut kill = {
        let mut k = Command::new("taskkill");
        k.args(["/T", "/F", "/PID", &pid]);
        k
    };
    kill.stdout(Stdio::null()).stderr(Stdio::null());
    paths::quiet(&mut kill);
    let _ = kill.run_status();
    let _ = child.kill(); // in case the group kill could not run
}

/// One line of `rbx promo install --json-progress`.
#[derive(Debug, PartialEq)]
pub(crate) enum EngineLine {
    Progress(String, f64),
    /// The last line: it has no "step".
    Done(bool, Vec<String>),
}

pub(crate) fn engine_line(line: &str) -> Option<EngineLine> {
    let v: Value = serde_json::from_str(line.trim()).ok().filter(Value::is_object)?;
    match v["step"].as_str() {
        Some(step) => Some(EngineLine::Progress(step.to_string(), v["fraction"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0))),
        None => {
            let missing = v["missing"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            Some(EngineLine::Done(v["ready"].as_bool().unwrap_or(false), missing))
        }
    }
}

/// The last lines of what the install printed on failure, short enough for Setup.
pub(crate) fn error_tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    let tail = lines[lines.len().saturating_sub(12)..].join("\n");
    let chars: Vec<char> = tail.chars().collect();
    chars[chars.len().saturating_sub(1500)..].iter().collect()
}

fn install_state() -> Value {
    let i = INSTALL.lock().unwrap();
    json!({"running": i.running, "step": i.step, "fraction": i.fraction, "error": i.error})
}

fn engine_status() -> Value {
    let out = paths::cli(&["promo", "status", "--json"]).run_output();
    let (engine, error) = match out {
        Ok(o) if o.status.success() => match serde_json::from_slice::<Value>(&o.stdout) {
            Ok(v) if v.is_object() => (v, String::new()),
            _ => (Value::Null, "The promo engine's status could not be read.".to_string()),
        },
        Ok(o) => (Value::Null, error_tail(&String::from_utf8_lossy(&o.stderr))),
        Err(e) => (Value::Null, format!("Could not run the tools: {e}")),
    };
    json!({"ok": error.is_empty(), "engine": engine, "error": error, "install": install_state()})
}

/// The install has ended: the state is settled before the page hears of it, so a status asked for then is current.
fn finish(step: &str, end: Ending) {
    {
        let mut i = INSTALL.lock().unwrap();
        i.running = false;
        i.child = None;
        i.error = match &end {
            Ending::Failed(e) => e.clone(),
            _ => String::new(),
        };
    }
    match end {
        Ending::Ready => BUS.put("promo_engine", json!({"done": true, "ready": true})),
        Ending::Stopped => BUS.put("promo_engine", json!({"stopped": true, "step": step})),
        Ending::Failed(error) => BUS.put("promo_engine", json!({"failed": true, "step": step, "error": error})),
    }
}

/// Run the install and pass its progress on; Retry runs this again and the install resumes.
fn run_install() {
    // an install runs for minutes: started one at a time with the AIs' processes (`paths::spawn_lock`), so it never
    // inherits a kept process's pipes
    let spawned = {
        install_command().start()
    };
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return finish("", Ending::Failed(format!("Could not run the tools: {e}"))),
    };
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    {
        let mut i = INSTALL.lock().unwrap();
        if i.stopping {
            // the app began closing while this was starting: stop_install found no child to stop
            kill_tree(&mut child);
        }
        i.child = Some(child);
    }
    // stderr is read beside stdout, so a chatty step can never fill its pipe and stall the install
    let errs = thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut text);
        }
        text
    });
    let (mut step, mut shown, mut done) = (String::new(), -1.0, None);
    for line in stdout.map(BufReader::new).into_iter().flat_map(|r| r.lines()).map_while(Result::ok) {
        match engine_line(&line) {
            Some(EngineLine::Progress(s, f)) => {
                {
                    let mut i = INSTALL.lock().unwrap();
                    i.step = s.clone();
                    i.fraction = f;
                }
                // a new step, its end, or a whole percent more: the bus keeps every event, so not every chunk
                if s != step || f >= 1.0 || f - shown >= 0.01 {
                    BUS.put("promo_engine", json!({"step": s, "fraction": f}));
                    shown = f;
                }
                step = s;
            }
            Some(EngineLine::Done(ready, missing)) => {
                done = Some((ready, missing));
                break;
            }
            None => {}
        }
    }
    let err_text = errs.join().unwrap_or_default();
    let child = INSTALL.lock().unwrap().child.take();
    let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
    let stopping = INSTALL.lock().unwrap().stopping;
    finish(&step, ending(stopping, done, code, &err_text));
}

/// POST /api/promo-engine {action: "status" | "install"}
pub fn engine(body: &Value) -> Value {
    match body["action"].as_str().unwrap_or("") {
        "status" => engine_status(),
        "install" => {
            let start = {
                let mut i = INSTALL.lock().unwrap();
                let start = !i.running && !i.stopping;
                if start {
                    *i = Install { running: true, step: String::new(), fraction: 0.0, error: String::new(), child: None, stopping: false };
                }
                start
            };
            if start {
                thread::spawn(run_install);
            }
            json!({"ok": true, "install": install_state()})
        }
        _ => json!({"ok": false, "error": "Unknown action."}),
    }
}

/// Closing the app stops an install halfway, with everything it started; the next install resumes from the
/// last finished step.
pub fn stop_install() {
    let mut i = INSTALL.lock().unwrap();
    i.stopping = true;
    if let Some(c) = i.child.as_mut() {
        kill_tree(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = crate::paths::test_dir(&format!("promo-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn routes_to_the_cards_agent_or_the_chat_in_front() {
        let known = |id: &str| id == "a1";
        assert_eq!(card_route_target("a1", known), "a1");
        assert_eq!(card_route_target("", known), "");
        assert_eq!(card_route_target("gone", known), "");
    }

    #[test]
    fn reveal_command_per_system() {
        let (cmd, args) = reveal_cmd("/x/y z/promo.mp4");
        #[cfg(target_os = "macos")]
        assert_eq!((cmd.as_str(), args), ("open", vec!["-R".to_string(), "/x/y z/promo.mp4".to_string()]));
        #[cfg(windows)]
        assert_eq!((cmd.as_str(), args), ("explorer", vec!["/select,/x/y z/promo.mp4".to_string()]));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!((cmd.as_str(), args), ("xdg-open", vec!["/x/y z".to_string()]));
    }

    #[test]
    fn card_ids_are_plain_names() {
        assert!(valid_id("20261121-101010-pet-tycoon"));
        for bad in ["", "..", "../x", "a/b", "a\\b", "C:x", ".hidden", "a..b"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn media_comes_only_from_the_card() {
        let d = scratch("media");
        let card = json!({"id": "p1", "video": "/v/José clip/promo.mp4", "cover": "/v/cover.jpg", "folder": "/v"});
        fs::write(d.join("p1.json"), card.to_string()).unwrap();
        assert_eq!(media_in(&d, "p1", "video"), Some((PathBuf::from("/v/José clip/promo.mp4"), "video/mp4")));
        assert_eq!(media_in(&d, "p1", "cover"), Some((PathBuf::from("/v/cover.jpg"), "image/jpeg")));
        assert_eq!(media_in(&d, "p1", "sheet"), None);
        assert_eq!(media_in(&d, "../p1", "video"), None);
        assert_eq!(media_in(&d, "nope", "video"), None);
        let _ = fs::remove_dir_all(&d);
    }

    fn set_mtime(path: &Path, secs_ago: u64) {
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn watcher_sees_rewritten_card() {
        let d = scratch("watch");
        fs::write(d.join("p1.json"), json!({"id": "p1", "created": "a"}).to_string()).unwrap();
        set_mtime(&d.join("p1.json"), 60);
        // cards there when the app starts were shown in an earlier run
        let mut start = snapshot(&d);
        assert!(changed_cards(&d, &mut start).is_empty());
        let mut seen = HashMap::new();
        let first = changed_cards(&d, &mut seen);
        assert_eq!(first.len(), 1);
        assert!(first[0]["mtime"].as_f64().unwrap() > 0.0);
        assert!(changed_cards(&d, &mut seen).is_empty());
        // rendered again into the same folder: same id, a newer file
        fs::write(d.join("p1.json"), json!({"id": "p1", "created": "b"}).to_string()).unwrap();
        let again = changed_cards(&d, &mut seen);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0]["created"], "b");
        assert!(changed_cards(&d, &mut seen).is_empty());
        // a card still being written is not marked seen, so it is reported once it reads
        fs::write(d.join("p2.json"), "{\"id\": \"p2\", ").unwrap();
        assert!(changed_cards(&d, &mut seen).is_empty());
        fs::write(d.join("p2.json"), json!({"id": "p2"}).to_string()).unwrap();
        assert_eq!(changed_cards(&d, &mut seen).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn caption_edit_goes_into_the_card_too() {
        let d = scratch("cardcap");
        fs::write(d.join("p1.json"), json!({"id": "p1", "caption_text": "old", "style": "hype"}).to_string()).unwrap();
        let mut seen = HashMap::new();
        changed_cards(&d, &mut seen);
        assert!(write_card_caption(&d, "p1", "new #roblox", &mut seen));
        let card = card_in(&d, "p1").unwrap();
        assert_eq!((card["caption_text"].as_str(), card["style"].as_str()), (Some("new #roblox"), Some("hype")));
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "no temporary file is left");
        // the app's own edit is not a new render: the watcher does not report it
        assert!(changed_cards(&d, &mut seen).is_empty());
        assert!(!write_card_caption(&d, "gone", "x", &mut seen));
        assert!(!write_card_caption(&d, "../p1", "x", &mut seen));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn temporary_names_are_unique() {
        let d = Path::new("/x");
        assert_ne!(tmp_beside(&d.join("caption.txt")), tmp_beside(&d.join("caption.txt")));
        assert_eq!(tmp_beside(&d.join("caption.txt")).parent(), Some(d));
    }

    #[test]
    fn caption_is_written_in_the_cards_folder() {
        let d = scratch("caption");
        let card = json!({"folder": d.display().to_string()});
        assert!(write_caption(&card, "New caption\n#roblox"));
        assert_eq!(fs::read_to_string(d.join("caption.txt")).unwrap(), "New caption\n#roblox");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "no temporary file is left");
        assert!(!write_caption(&json!({"folder": d.join("missing").display().to_string()}), "x"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn saving_copies_video_and_extras_with_matching_names() {
        let d = scratch("save");
        let src = d.join("20261121-pet");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("promo.mp4"), b"video").unwrap();
        fs::write(src.join("cover.jpg"), b"cover").unwrap();
        fs::write(src.join("caption.txt"), b"caption").unwrap();
        let card = json!({"folder": src.display().to_string(), "video": src.join("promo.mp4").display().to_string(), "cover": src.join("cover.jpg").display().to_string()});
        let out = d.join("out");
        fs::create_dir_all(&out).unwrap();

        copy_out(&card, &out.join("only.mp4"), false).unwrap();
        assert_eq!(fs::read(out.join("only.mp4")).unwrap(), b"video");
        assert!(!out.join("only.jpg").exists() && !out.join("only.txt").exists());

        copy_out(&card, &out.join("My Promo.mp4"), true).unwrap();
        assert_eq!(fs::read(out.join("My Promo.jpg")).unwrap(), b"cover");
        assert_eq!(fs::read(out.join("My Promo.txt")).unwrap(), b"caption");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn media_files_are_told_apart() {
        for yes in ["a.mp4", "b.MOV", "c.mkv", "d.webm", "e.mp3", "f.wav", "g.m4a", "h.ogg"] {
            assert!(is_media(Path::new(yes)), "{yes}");
        }
        for no in ["a.png", "b.txt", "c", "d.mp4.txt"] {
            assert!(!is_media(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn install_lines_are_progress_until_the_one_without_a_step() {
        assert_eq!(engine_line(r#"{"step": "node", "fraction": 0.42}"#), Some(EngineLine::Progress("node".into(), 0.42)));
        assert_eq!(engine_line(r#"{"step": "voice", "fraction": 3}"#), Some(EngineLine::Progress("voice".into(), 1.0)));
        assert_eq!(engine_line(r#"{"ready": true, "missing": []}"#), Some(EngineLine::Done(true, vec![])));
        assert_eq!(engine_line(r#"{"ready": false, "missing": ["browser", "voice"]}"#), Some(EngineLine::Done(false, vec!["browser".into(), "voice".into()])));
        assert_eq!(engine_line("Downloading about 700 MB"), None);
        assert_eq!(engine_line("[1, 2]"), None);
    }

    #[test]
    fn error_tail_keeps_the_last_lines() {
        let long: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let tail = error_tail(&long);
        assert!(tail.starts_with("line 19") && tail.ends_with("line 30"), "{tail}");
        assert_eq!(error_tail("  one problem \n"), "one problem");
        assert!(error_tail(&"é".repeat(5000)).chars().count() <= 1500);
    }

    #[test]
    fn an_install_ends_ready_stopped_or_failed() {
        assert_eq!(ending(false, Some((true, vec![])), Some(0), ""), Ending::Ready);
        // stopped by the app: killed, no last line, whatever it printed is not shown as a failure
        assert_eq!(ending(true, None, None, "Traceback ... BrokenPipeError"), Ending::Stopped);
        assert_eq!(ending(true, Some((false, vec!["voice".into()])), Some(0), ""), Ending::Stopped);
        // a run that finished just as the app closed still counts as ready
        assert_eq!(ending(true, Some((true, vec![])), Some(0), ""), Ending::Ready);
        assert_eq!(ending(false, None, Some(1), "npm failed\n"), Ending::Failed("npm failed".into()));
        assert_eq!(ending(false, None, None, ""), Ending::Failed("The install stopped (exit without a code).".into()));
        assert_eq!(ending(false, Some((false, vec!["browser".into()])), Some(0), ""), Ending::Failed("The install finished, but these parts are still missing: browser.".into()));
    }

    #[cfg(unix)]
    #[test]
    fn stopping_kills_the_whole_process_group() {
        // a shell in its own group that starts a grandchild, like fridae-cli starting npm
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 30 & echo $!; wait"]);
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let mut child = c.stdout(Stdio::piped()).start().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let grandchild = line.trim().to_string();
        kill_tree(&mut child);
        let _ = child.wait();
        let mut gone = false;
        for _ in 0..50 {
            gone = !Command::new("kill").args(["-0", &grandchild]).stderr(Stdio::null()).run_status().unwrap().success();
            if gone {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(gone, "the grandchild {grandchild} is still running");
    }
}
