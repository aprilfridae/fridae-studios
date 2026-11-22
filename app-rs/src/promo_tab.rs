//! The Promos tab: the library of finished promos, starting a new promo in a chat of its own, and the
//! style previews for its form. The cards themselves, their media and buttons are in promo.rs.

use crate::paths::Start as _;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Ask},
    paths, projects,
    promo::{self, AUDIO_EXT, VIDEO_EXT},
};

/// The styles the promo tool knows (rbxpromo.recipe.STYLES).
pub const STYLES: [&str; 8] = ["hype", "cinematic", "meme", "countdown", "update-log", "lyric", "showcase", "pov"];
const LENGTHS: [u64; 3] = [15, 30, 60];
const MAX_CLIPS: usize = 60;
const MAX_TEXT: usize = 4000;

/// Seconds since 1970 of an ISO time like "2026-11-21T09:30:00-05:00" (the card's `created`).
fn iso_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b).and_then(|t| t.parse::<i64>().ok());
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    // the zone after the seconds (and any fraction): Z, +HH:MM or -HH:MM; none counts as UTC
    let rest = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let zone = rest.get(1..)?.replace(':', "");
            let (zh, zm) = (zone.get(0..2)?.parse::<i64>().ok()?, zone.get(2..4).and_then(|t| t.parse::<i64>().ok()).unwrap_or(0));
            (zh * 3600 + zm * 60) * if sign == '-' { -1 } else { 1 }
        }
        _ => 0,
    };
    // days from the civil date (Howard Hinnant's algorithm)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// Every card in `dir`, newest first: by `created`, then by when the file was written. Cards that do not
/// read (half-written) are left out.
pub fn list_in(dir: &Path) -> Vec<Value> {
    let mut cards: Vec<(i64, f64, Value)> = promo::card_files(dir)
        .into_iter()
        .filter_map(|(id, _, m)| promo::card_at(dir, &id, m))
        .map(|c| (iso_secs(c["created"].as_str().unwrap_or("")).unwrap_or(0), c["mtime"].as_f64().unwrap_or(0.0), c))
        .collect();
    cards.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)));
    cards.into_iter().map(|(_, _, c)| c).collect()
}

/// GET /api/promos
pub fn list() -> Value {
    json!({"cards": list_in(&paths::work("promo-cards"))})
}

/// What the New promo form asked for, checked.
#[derive(Debug, Default, PartialEq)]
pub struct Start {
    pub clips: Vec<String>,
    pub music: String,
    pub reference: String,
    pub style: String,
    pub length: u64,
    pub voice_text: String,
    pub brief: String,
    pub project: Option<String>,
    /// "Mute game sound (I'll add a TikTok sound)": the agent sets audio.clip_volume 0.
    pub mute_game_sound: bool,
}

/// A file the user chose: it must be there, be a file, and have one of `exts`. Paths go into the message as
/// lines, so one with a line break or other control character is refused.
fn check_file(path: &str, exts: &[&str], what: &str) -> Result<String, String> {
    let name = paths::base_name(path);
    if path.chars().any(char::is_control) {
        return Err(format!("The {what} {name} has an unusual name. Rename it and choose it again."));
    }
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !exts.contains(&ext.as_str()) {
        return Err(format!("{name} is not a {what} this can use ({}).", exts.join(", ")));
    }
    if !p.is_file() {
        return Err(format!("The {what} {name} is no longer there. Choose it again."));
    }
    Ok(path.to_string())
}

fn text(body: &Value, key: &str) -> String {
    body[key].as_str().unwrap_or("").trim().to_string()
}

/// POST /api/promo-start's body, checked. `known_project` says whether a path is one of the user's projects.
pub fn validate(body: &Value, known_project: impl Fn(&str) -> bool) -> Result<Start, String> {
    let clips: Vec<&Value> = body["clips"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    if clips.is_empty() {
        return Err("Add at least one clip.".into());
    }
    if clips.len() > MAX_CLIPS {
        return Err(format!("Use at most {MAX_CLIPS} clips."));
    }
    let clips = clips.into_iter().map(|c| check_file(c.as_str().unwrap_or(""), &VIDEO_EXT, "clip")).collect::<Result<Vec<_>, _>>()?;
    let optional = |key: &str, exts: &[&str], what: &str| match text(body, key) {
        p if p.is_empty() => Ok(String::new()),
        p => check_file(&p, exts, what),
    };
    let music = optional("music", &AUDIO_EXT, "music file")?;
    let reference = optional("reference", &VIDEO_EXT, "reference video")?;
    let style = text(body, "style");
    if !style.is_empty() && !STYLES.contains(&style.as_str()) {
        return Err(format!("Unknown style {style}."));
    }
    let length = body["length"].as_u64().filter(|l| LENGTHS.contains(l)).ok_or("The length must be 15, 30 or 60 seconds.")?;
    let brief = text(body, "brief");
    if brief.is_empty() {
        return Err("Say what the promo should show.".into());
    }
    let voice_text = text(body, "voice_text");
    if brief.chars().count() > MAX_TEXT || voice_text.chars().count() > MAX_TEXT {
        return Err("That text is too long.".into());
    }
    let project = match text(body, "project") {
        p if p.is_empty() => None,
        p if known_project(&p) => Some(p),
        _ => return Err("That project is no longer in the list. Choose it again.".into()),
    };
    let mute_game_sound = body["mute_game_sound"].as_bool().unwrap_or(false);
    Ok(Start { clips, music, reference, style, length, voice_text, brief, project, mute_game_sound })
}

/// The new promo chat's first message.
pub fn compose(s: &Start) -> String {
    let mut lines = vec![format!("Make a promo video of my game. What it should show: {}", s.brief), String::new()];
    lines.extend(s.clips.iter().map(|c| format!("[clip: {c}]")));
    lines.push(if s.music.is_empty() { "Music: none (no music track; keep the game sound).".into() } else { format!("Music: {}", s.music) });
    if s.mute_game_sound {
        lines.push("Mute the game sound (audio clip_volume 0): I'll add a TikTok sound.".into());
    }
    if !s.reference.is_empty() {
        lines.push(format!("Reference video: {}", s.reference));
    }
    if !s.style.is_empty() {
        lines.push(format!("Style: {}", s.style));
    }
    lines.push(format!("Length: {} seconds", s.length));
    if !s.voice_text.is_empty() {
        lines.push(format!("Voiceover script: {}", s.voice_text));
    }
    lines.push(String::new());
    lines.push("Follow the promo guide (rbx_guide topic promo).".into());
    lines.join("\n")
}

/// "Promo: " and the first five words of the brief.
pub fn title(brief: &str) -> String {
    format!("Promo: {}", brief.split_whitespace().take(5).collect::<Vec<_>>().join(" "))
}

/// POST /api/promo-start: a new chat in the background, in the chosen project, with the composed message.
pub fn start(body: &Value) -> Value {
    let known = projects::list();
    let s = match validate(body, |p| known.iter().any(|k| k["path"].as_str() == Some(p))) {
        Ok(s) => s,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    if let Some(p) = &s.project {
        projects::touch(p);
    }
    let Some(agent) = agents::new_background_chat(&title(&s.brief), s.project.clone()) else {
        return json!({"ok": false, "error": "All eight chats are in use. Close one, then try again."});
    };
    let (id, text) = (agent.id.clone(), compose(&s));
    thread::spawn(move || agent.send(&text, &Ask::default()));
    json!({"ok": true, "agent": id})
}

fn styles_dir() -> PathBuf {
    paths::work("promo-work").join("styles")
}

/// The still `/media/promo-style/<name>` serves: the newest `<name>-<12 hex>.jpg` in `dir`, for a known style only.
fn style_still_in(dir: &Path, name: &str) -> Option<PathBuf> {
    if !STYLES.contains(&name) {
        return None;
    }
    let ours = |f: &str| f.strip_prefix(name).and_then(|r| r.strip_prefix('-')).and_then(|r| r.strip_suffix(".jpg")).is_some_and(|h| h.len() == 12 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| ours(&e.file_name().to_string_lossy()))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

pub fn style_still(name: &str) -> Option<PathBuf> {
    style_still_in(&styles_dir(), name)
}

/// POST /api/promo-styles: each style's name, line and still. The first run renders the stills (they are
/// cached by the tool), so only one runs at a time and a second request waits for it.
pub fn styles() -> Value {
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let out = paths::cli(&["promo", "styles"]).run_output();
    let parsed = match out {
        Ok(o) if o.status.success() => serde_json::from_slice::<Value>(&o.stdout).map_err(|_| "The styles could not be read.".to_string()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("The styles could not be made.").to_string()),
        Err(e) => Err(format!("Could not run the tools: {e}")),
    };
    match parsed {
        Ok(v) => {
            let items: Vec<Value> = v["styles"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    let name = s["name"].as_str().filter(|n| STYLES.contains(n))?;
                    Some(json!({"name": name, "about": s["about"].as_str().unwrap_or(""), "url": format!("/media/promo-style/{name}")}))
                })
                .collect();
            json!({"ok": true, "styles": items})
        }
        Err(e) => json!({"ok": false, "error": e, "styles": []}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::{Duration, SystemTime}};

    fn scratch(name: &str) -> PathBuf {
        let d = crate::paths::test_dir(&format!("promotab-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn set_mtime(path: &Path, secs_ago: u64) {
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn list_cards_newest_first() {
        let d = scratch("list");
        let card = |id: &str, created: &str| json!({"id": id, "agent": "", "video": "/v.mp4", "created": created}).to_string();
        fs::write(d.join("old.json"), card("old", "2026-11-19T10:00:00+00:00")).unwrap();
        // same instant written in another zone: it is newer than "old" by an hour, older than "new"
        fs::write(d.join("mid.json"), card("mid", "2026-11-19T12:00:00+01:00")).unwrap();
        fs::write(d.join("new.json"), card("new", "2026-11-21T09:30:00-05:00")).unwrap();
        // the same created time: the file written later comes first
        fs::write(d.join("twin-a.json"), card("twin-a", "2026-11-18T08:00:00+00:00")).unwrap();
        fs::write(d.join("twin-b.json"), card("twin-b", "2026-11-18T08:00:00+00:00")).unwrap();
        set_mtime(&d.join("twin-a.json"), 100);
        set_mtime(&d.join("twin-b.json"), 10);
        fs::write(d.join("half.json"), "{\"id\": \"half\", \"vid").unwrap();
        fs::write(d.join("list.json"), "[1, 2]").unwrap();
        fs::write(d.join("new.json.tmp"), card("tmp", "2026-11-22T00:00:00+00:00")).unwrap();
        let ids: Vec<String> = list_in(&d).iter().map(|c| c["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["new", "mid", "old", "twin-b", "twin-a"]);
        // each card says when its file last changed, for the page to reload a re-rendered video
        assert!(list_in(&d).iter().all(|c| c["mtime"].as_f64().unwrap_or(0.0) > 0.0));
        let _ = fs::remove_dir_all(&d);
    }

    fn files(d: &Path) -> (String, String, String, String) {
        let clip = d.join("clip 1 José.mp4");
        let clip2 = d.join("B.MOV");
        let song = d.join("song.mp3");
        let reference = d.join("ref.webm");
        for p in [&clip, &clip2, &song, &reference] {
            fs::write(p, b"x").unwrap();
        }
        let s = |p: &PathBuf| p.display().to_string();
        (s(&clip), s(&clip2), s(&song), s(&reference))
    }

    #[test]
    fn compose_start_message() {
        let d = scratch("compose");
        let (clip, clip2, song, reference) = files(&d);
        let full = Start {
            clips: vec![clip.clone(), clip2.clone()],
            music: song.clone(),
            reference: reference.clone(),
            style: "hype".into(),
            length: 15,
            voice_text: "This pet is broken.".into(),
            brief: "the new boss fight and the shop".into(),
            project: None,
            mute_game_sound: false,
        };
        let text = compose(&full);
        assert!(text.contains("the new boss fight and the shop"), "{text}");
        assert!(text.contains(&format!("[clip: {clip}]")) && text.contains(&format!("[clip: {clip2}]")), "{text}");
        assert!(text.contains(&format!("Music: {song}")), "{text}");
        assert!(text.contains(&format!("Reference video: {reference}")), "{text}");
        assert!(text.contains("Style: hype"), "{text}");
        assert!(text.contains("Length: 15 seconds"), "{text}");
        assert!(text.contains("Voiceover script: This pet is broken."), "{text}");
        assert!(text.trim_end().ends_with("Follow the promo guide (rbx_guide topic promo)."), "{text}");

        let bare = Start { clips: vec![clip.clone()], length: 30, brief: "the shop".into(), ..Default::default() };
        let text = compose(&bare);
        assert!(text.contains(&format!("[clip: {clip}]")) && text.contains("Length: 30 seconds"), "{text}");
        assert!(!text.contains("Reference video") && !text.contains("Style:") && !text.contains("Voiceover script"), "{text}");
        // no music means no music track: the game sound stays unless the user mutes it
        assert!(text.contains("Music: none") && text.contains("keep the game sound"), "{text}");
        assert!(!text.contains("silent") && !text.contains("clip_volume"), "{text}");
        let muted = compose(&Start { mute_game_sound: true, ..bare });
        assert!(muted.contains("Mute the game sound") && muted.contains("clip_volume 0"), "{muted}");
        assert!(text.contains("Follow the promo guide (rbx_guide topic promo)."), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn start_validates() {
        let d = scratch("validate");
        let (clip, _, song, reference) = files(&d);
        let known = |p: &str| p == "/games/obby";
        let ok = json!({"clips": [clip], "music": song, "reference": reference, "style": "pov", "length": 60, "voice_text": " hi ", "brief": "  boss fight ", "project": "/games/obby"});
        let s = validate(&ok, known).unwrap();
        assert_eq!((s.brief.as_str(), s.voice_text.as_str(), s.length, s.style.as_str()), ("boss fight", "hi", 60, "pov"));
        assert_eq!(s.project.as_deref(), Some("/games/obby"));
        assert!(!s.mute_game_sound);
        let mut muted = ok.clone();
        muted["mute_game_sound"] = json!(true);
        assert!(validate(&muted, known).unwrap().mute_game_sound);
        let no_project = json!({"clips": [clip], "length": 30, "brief": "x", "project": ""});
        assert_eq!(validate(&no_project, known).unwrap().project, None);

        let bad = |change: Value| {
            let mut b = ok.clone();
            for (k, v) in change.as_object().unwrap() {
                b[k] = v.clone();
            }
            validate(&b, known).unwrap_err()
        };
        bad(json!({"clips": []}));
        bad(json!({"clips": [d.join("missing.mp4").display().to_string()]}));
        bad(json!({"clips": [d.display().to_string()]})); // a folder, not a file
        bad(json!({"clips": [song.clone()]})); // music is not a clip
        bad(json!({"music": clip.clone()}));
        bad(json!({"music": d.join("gone.mp3").display().to_string()}));
        bad(json!({"reference": song.clone()}));
        bad(json!({"brief": "   "}));
        bad(json!({"length": 45}));
        bad(json!({"length": "30"}));
        bad(json!({"style": "vaporwave"}));
        bad(json!({"project": "/somewhere/else"}));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn promo_chat_title() {
        assert_eq!(title("the new boss fight and the shop"), "Promo: the new boss fight and");
        assert_eq!(title("  shop\n update "), "Promo: shop update");
    }

    #[test]
    fn style_still_path_confined() {
        let d = scratch("styles");
        fs::write(d.join("hype-aaaaaaaaaaaa.jpg"), b"old").unwrap();
        fs::write(d.join("hype-bbbbbbbbbbbb.jpg"), b"new").unwrap();
        set_mtime(&d.join("hype-aaaaaaaaaaaa.jpg"), 100);
        fs::write(d.join("hype-extra-cccccccccccc.jpg"), b"other").unwrap();
        fs::write(d.join("pov-dddddddddddd.png"), b"png").unwrap();
        fs::write(d.join("sheet_styles.jpg"), b"sheet").unwrap();
        assert_eq!(style_still_in(&d, "hype"), Some(d.join("hype-bbbbbbbbbbbb.jpg")));
        assert_eq!(style_still_in(&d, "pov"), None); // not a jpg
        assert_eq!(style_still_in(&d, "meme"), None); // no still yet
        for bad in ["", "../hype", "hype/..", "a/b", "..", "sheet_styles", "hype-extra"] {
            assert_eq!(style_still_in(&d, bad), None, "{bad}");
        }
        let _ = fs::remove_dir_all(&d);
    }
}
