//! Texture requests: a tool asks the user to make an image in their own Gemini
//! chat, the window shows the prompt, and the copied image is handed back.

use std::{collections::HashMap, fs, path::PathBuf, thread, time::Duration};

use image::DynamicImage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    agents::{self, BUS},
    paths,
};

fn dir() -> PathBuf {
    paths::work("texture-requests")
}

/// Requests that are still waiting for an image.
pub fn pending() -> Vec<Value> {
    let mut names: Vec<String> = fs::read_dir(dir()).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| n.strip_suffix(".json"))
        .filter(|stem| !dir().join(format!("{stem}.png")).exists())
        .filter_map(|stem| fs::read_to_string(dir().join(format!("{stem}.json"))).ok())
        .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|r| r["id"].is_string())
        .collect()
}

pub fn find(rid: &str) -> Option<Value> {
    pending().into_iter().find(|r| r["id"] == rid)
}

/// Hand an image to the waiting request.
pub fn fulfil(rid: &str, img: &DynamicImage) -> bool {
    let rid = paths::base_name(rid);
    if !dir().join(format!("{rid}.json")).exists() {
        return false;
    }
    let tmp = dir().join(format!("{rid}.tmp"));
    img.to_rgba8().save_with_format(&tmp, image::ImageFormat::Png).is_ok() && fs::rename(&tmp, dir().join(format!("{rid}.png"))).is_ok()
}

pub fn skip(rid: &str) -> bool {
    fs::write(dir().join(format!("{}.skip", paths::base_name(rid))), "").is_ok()
}

/// Give up on every request a chat is waiting for.
pub fn skip_for(agent_id: &str) {
    for r in pending() {
        if r["agent"] == agent_id {
            skip(r["id"].as_str().unwrap_or(""));
        }
    }
}

// ---------------------------------------------------------------- clipboard
pub fn set_clipboard_text(text: &str) -> bool {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).is_ok()
}

/// The image on the clipboard, if there is one (a copied picture, or a copied image file).
fn clipboard_image() -> Option<DynamicImage> {
    let mut clip = arboard::Clipboard::new().ok()?;
    if let Ok(img) = clip.get_image() {
        let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
        return Some(DynamicImage::ImageRgba8(buf));
    }
    let files = clip.get().file_list().ok()?;
    files
        .iter()
        .filter(|p| {
            let n = p.to_string_lossy().to_lowercase();
            [".png", ".jpg", ".jpeg", ".webp", ".bmp"].iter().any(|e| n.ends_with(e))
        })
        .find_map(|p| image::open(p).ok())
}

/// A fingerprint, to tell a newly copied image from the one that was already there.
fn mark(img: &DynamicImage) -> String {
    let small = img.resize_exact(32, 32, image::imageops::FilterType::Triangle).to_rgb8();
    let hash = Sha256::digest(small.as_raw());
    format!("{}-{}x{}", paths::hex(&hash), img.width(), img.height())
}

/// Show texture requests in the window, and pick the image up when the user copies it.
///
/// The clipboard is only looked at while a request is waiting, and only an
/// image that was not already there when the request appeared is taken.
pub fn watch() {
    let mut known: HashMap<String, Value> = HashMap::new();
    let mut baseline: Option<String> = None;
    loop {
        thread::sleep(Duration::from_secs(1));
        let live: Vec<(String, Value)> = pending().into_iter().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r)).collect();
        for (rid, r) in &live {
            if known.contains_key(rid) {
                continue;
            }
            let mut r = r.clone();
            let wanted = r["agent"].as_str().unwrap_or("").to_string();
            let agent = if agents::get(&wanted).is_some() { wanted } else { agents::agents().first().map(|a| a.id.clone()).unwrap_or_default() };
            r["agent"] = json!(agent);
            BUS.put("texrequest", json!({"agent": agent, "req": rid, "prompt": r["prompt"], "kind_of": r["kind"], "name": r["name"]}));
            known.insert(rid.clone(), r);
            if baseline.is_none() {
                baseline = Some(clipboard_image().map(|i| mark(&i)).unwrap_or_default());
            }
        }
        let gone: Vec<String> = known.keys().filter(|k| !live.iter().any(|(id, _)| id == *k)).cloned().collect();
        for rid in gone {
            let r = known.remove(&rid).unwrap_or_default();
            let skipped = dir().join(format!("{rid}.skip")).exists();
            let agent = r["agent"].as_str().unwrap_or("").to_string();
            BUS.put("texdone", json!({"agent": agent, "req": rid, "ok": !skipped}));
        }
        if live.is_empty() {
            baseline = None;
            continue;
        }
        if let Some(img) = clipboard_image().filter(|i| i.width().min(i.height()) >= 64) {
            let m = mark(&img);
            if baseline.as_deref() != Some(m.as_str()) {
                baseline = Some(m);
                fulfil(&live[0].0, &img);
            }
        }
    }
}

// ------------------------------------------------------- paste into Gemini
#[cfg(windows)]
mod win {
    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn GetForegroundWindow() -> isize;
        pub fn GetWindowTextW(hwnd: isize, text: *mut u16, max: i32) -> i32;
        pub fn GetAsyncKeyState(key: i32) -> i16;
        pub fn keybd_event(key: u8, scan: u8, flags: u32, extra: usize);
    }

    /// The window in front, and its title.
    pub fn front() -> (isize, String) {
        let mut buf = [0u16; 512];
        unsafe {
            let hwnd = GetForegroundWindow();
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            (hwnd, String::from_utf16_lossy(&buf[..n]))
        }
    }

    pub fn key_down(key: i32) -> bool {
        unsafe { GetAsyncKeyState(key) < 0 }
    }

    pub fn press_ctrl_v() {
        const CTRL: u8 = 0x11;
        const V: u8 = 0x56;
        const UP: u32 = 2;
        unsafe {
            keybd_event(CTRL, 0, 0, 0);
            keybd_event(V, 0, 0, 0);
            keybd_event(V, 0, UP, 0);
            keybd_event(CTRL, 0, UP, 0);
        }
    }
}

/// A browser window that is showing the Gemini chat itself: the page has set
/// its title, so this is not the sign-in page or a tab that is still loading.
fn is_gemini(title: &str) -> bool {
    title.contains("Gemini") && !title.contains("google.com") && !title.contains("Sign in") && !title.contains(crate::paths::APP)
}

/// After the Gemini tab opens, paste the prompt into its message box.
///
/// Gemini puts the cursor in the message box by itself, so one Ctrl+V is all it
/// takes. It only happens while a Gemini window is in front and the user is not
/// holding a key, and nothing is sent: pressing Enter stays with the user.
pub fn paste_into_gemini() {
    #[cfg(windows)]
    thread::spawn(|| {
        let mut seen: Option<(isize, u32)> = None; // the window, and how many checks in a row it has been in front
        for _ in 0..120 {
            thread::sleep(Duration::from_millis(250));
            let (hwnd, title) = win::front();
            if !is_gemini(&title) {
                seen = None;
                continue;
            }
            let steady = match seen {
                Some((h, n)) if h == hwnd => n + 1,
                _ => 0,
            };
            seen = Some((hwnd, steady));
            // about two seconds with the chat in front: time for the page to finish and focus its box
            let busy_keys = [0x10, 0x11, 0x12, 0x5B].iter().any(|k| win::key_down(*k)); // shift, ctrl, alt, win
            if steady >= 8 && !busy_keys {
                win::press_ctrl_v();
                return;
            }
        }
    });
}
