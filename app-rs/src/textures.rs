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
    #[cfg(target_os = "linux")]
    if wayland::set_text(text) {
        return true;
    }
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).is_ok()
}

/// A Linux desktop on Wayland: arboard reads only the X11 clipboard (through XWayland), so an image copied in a native
/// Wayland browser is not seen there. `wl-paste`/`wl-copy` (the wl-clipboard package) reach the Wayland clipboard
/// itself; without them arboard is used as before. (arboard's own `wayland-data-control` feature would do this in the
/// process, but needs the wl-clipboard-rs crate, which this build does not carry.)
#[cfg(target_os = "linux")]
mod wayland {
    use crate::paths::Start;
    use std::process::{Command, Stdio};

    fn tool(name: &str) -> Option<std::path::PathBuf> {
        std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty())?;
        crate::paths::which(name)
    }

    pub fn set_text(text: &str) -> bool {
        let Some(copy) = tool("wl-copy") else { return false };
        let Ok(mut child) = Command::new(copy).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).start() else { return false };
        let wrote = child.stdin.take().is_some_and(|mut i| std::io::Write::write_all(&mut i, text.as_bytes()).is_ok());
        wrote && child.wait().is_ok_and(|s| s.success())
    }

    /// The clipboard's image as file bytes, or the copied files' paths (`text/uri-list`).
    pub fn image() -> Option<super::Clip> {
        let paste = tool("wl-paste")?;
        let types = Command::new(&paste).arg("--list-types").stdin(Stdio::null()).stderr(Stdio::null()).run_output().ok()?;
        let mime = super::best_image_type(&String::from_utf8_lossy(&types.stdout))?;
        let out = Command::new(&paste).args(["--no-newline", "--type", mime]).stdin(Stdio::null()).stderr(Stdio::null()).run_output().ok()?;
        if !out.status.success() || out.stdout.is_empty() {
            return None;
        }
        Some(if mime == "text/uri-list" { super::Clip::Files(super::uri_list(&String::from_utf8_lossy(&out.stdout))) } else { super::Clip::Bytes(out.stdout) })
    }
}

/// What a clipboard reader handed over: an image's file bytes, or copied files.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Clip {
    Bytes(Vec<u8>),
    Files(Vec<PathBuf>),
}

/// The clipboard type to read an image from, of those `wl-paste --list-types` lists: a PNG first, then other image
/// types, then a list of copied files.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn best_image_type(listed: &str) -> Option<&'static str> {
    const WANTED: [&str; 6] = ["image/png", "image/jpeg", "image/webp", "image/bmp", "image/gif", "text/uri-list"];
    WANTED.into_iter().find(|w| listed.lines().any(|l| l.trim() == *w))
}

/// The local files of a `text/uri-list` (`file:///home/a/My%20Image.png`, one per line; comments and other schemes
/// left out).
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn uri_list(text: &str) -> Vec<PathBuf> {
    text.lines().filter_map(|l| l.trim().strip_prefix("file://")).filter_map(percent_decode).map(PathBuf::from).collect()
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn image_file(files: &[PathBuf]) -> Option<DynamicImage> {
    files
        .iter()
        .filter(|p| {
            let n = p.to_string_lossy().to_lowercase();
            [".png", ".jpg", ".jpeg", ".webp", ".bmp"].iter().any(|e| n.ends_with(e))
        })
        .find_map(|p| image::open(p).ok())
}

/// The image on the clipboard, if there is one (a copied picture, or a copied image file).
fn clipboard_image() -> Option<DynamicImage> {
    #[cfg(target_os = "linux")]
    match wayland::image() {
        Some(Clip::Bytes(b)) => return image::load_from_memory(&b).ok(),
        Some(Clip::Files(f)) => return image_file(&f),
        None => {}
    }
    let mut clip = arboard::Clipboard::new().ok()?;
    if let Ok(img) = clip.get_image() {
        let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
        return Some(DynamicImage::ImageRgba8(buf));
    }
    let files = clip.get().file_list().ok()?;
    image_file(&files)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wayland_clipboard_image_is_read_as_a_png_first_then_as_copied_files() {
        assert_eq!(best_image_type("text/html\nimage/jpeg\nimage/png\n"), Some("image/png"));
        assert_eq!(best_image_type("text/uri-list\nimage/webp"), Some("image/webp"));
        assert_eq!(best_image_type("text/uri-list\n"), Some("text/uri-list"));
        assert_eq!(best_image_type("text/plain\nUTF8_STRING"), None);
        assert_eq!(uri_list("# copied\nfile:///home/a/My%20Image.png\r\nhttps://x/y.png\nfile:///tmp/%C3%A9.jpg"), [PathBuf::from("/home/a/My Image.png"), PathBuf::from("/tmp/é.jpg")]);
        assert_eq!(percent_decode("bad%2"), None);
    }
}
