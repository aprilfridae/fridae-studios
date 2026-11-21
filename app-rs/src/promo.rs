//! Finished promos. The promo tool drops a card file when a render is done; this
//! shows it in the chat that asked for it, and does what the card's buttons ask.
//!
//!   work/promo-cards/<id>.json   {"id", "agent", "folder", "video", "cover", "caption_text",
//!                                 "style", "duration", "check": {"passed", "problems", "notes"}, "created"}
//!
//! Card files stay: they are the only record of which files the window may play.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{agents::{self, BUS}, attach, paths};

const VIDEO_EXT: [&str; 4] = ["mp4", "mov", "mkv", "webm"];
const AUDIO_EXT: [&str; 4] = ["mp3", "wav", "m4a", "ogg"];

fn dir() -> PathBuf {
    paths::work("promo-cards")
}

/// A card id is a plain file name stem: never a path the page could steer elsewhere.
fn valid_id(id: &str) -> bool {
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
    c.spawn().is_ok()
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

/// Write the edited post caption back to caption.txt in the card's own folder.
fn write_caption(card: &Value, text: &str) -> bool {
    let Some(folder) = path_of(card, "folder").filter(|f| f.is_dir()) else { return false };
    let tmp = folder.join("caption.txt.tmp");
    fs::write(&tmp, text).is_ok() && fs::rename(&tmp, folder.join("caption.txt")).is_ok()
}

/// POST /api/promo {action: "reveal" | "save" | "caption", id, extras?, text?}
pub fn act(body: &Value) -> Value {
    let Some(card) = card(body["id"].as_str().unwrap_or("")) else { return json!({"ok": false, "error": "That promo is gone."}) };
    match body["action"].as_str().unwrap_or("") {
        "reveal" => json!({"ok": reveal(&card)}),
        "save" => save(&card, body["extras"].as_bool().unwrap_or(false)),
        "caption" => json!({"ok": write_caption(&card, body["text"].as_str().unwrap_or(""))}),
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

/// Cards already there when the app starts were shown in an earlier run; new ones are shown as they arrive.
pub fn watch() {
    let dir = dir();
    let ids = |d: &Path| -> Vec<String> {
        fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json")).filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string())).collect()
    };
    let mut seen: HashSet<String> = ids(&dir).into_iter().collect();
    loop {
        thread::sleep(Duration::from_millis(700));
        for id in ids(&dir) {
            if seen.contains(&id) {
                continue;
            }
            // a card that does not read yet is tried again on the next look
            if let Some(card) = card_in(&dir, &id) {
                show(&card);
                seen.insert(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fridae-promo-{name}-{}", std::process::id()));
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

    #[test]
    fn caption_is_written_in_the_cards_folder() {
        let d = scratch("caption");
        let card = json!({"folder": d.display().to_string()});
        assert!(write_caption(&card, "New caption\n#roblox"));
        assert_eq!(fs::read_to_string(d.join("caption.txt")).unwrap(), "New caption\n#roblox");
        assert!(!d.join("caption.txt.tmp").exists());
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
}
