//! Files the user pastes, drops or picks, and pictures shown in the chat.

use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use base64::Engine;
use regex::Regex;
use serde_json::{Value, json};

use crate::paths;

const MAX_BYTES: usize = 30_000_000;
const IMAGE_EXT: [&str; 9] = [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif", ".tga", ".tif", ".tiff"];
const OFFICE_EXT: [&str; 5] = [".docx", ".pptx", ".xlsx", ".odt", ".rtf"];

/// What a saved attachment is, for the message that goes to the agent.
#[derive(Clone)]
pub struct Info {
    pub name: String,
    pub kind: String,
    pub label: String,
    pub read: String,
    pub note: String,
}

static ATT: Mutex<Option<HashMap<String, Info>>> = Mutex::new(None);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static IMG_COUNT: AtomicUsize = AtomicUsize::new(0);

fn remember(info: &Info) {
    ATT.lock().unwrap().get_or_insert_with(HashMap::new).insert(info.name.clone(), info.clone());
}

/// The details of a saved attachment, if the file is still there.
pub fn info(name: &str) -> Option<Info> {
    let path = paths::work("attachments").join(name);
    if !path.is_file() {
        return None;
    }
    if let Some(i) = ATT.lock().unwrap().as_ref().and_then(|m| m.get(name)) {
        return Some(i.clone());
    }
    let kind = if name.to_lowercase().ends_with(".png") { "image" } else { "doc" };
    Some(Info { name: name.into(), kind: kind.into(), label: name.into(), read: path.display().to_string(), note: "file".into() })
}

/// The bytes of a base64 string or a data: URL.
pub fn decode(data: &str) -> Option<Vec<u8>> {
    let b64 = data.rsplit(',').next().unwrap_or("").trim();
    base64::engine::general_purpose::STANDARD.decode(b64).ok().filter(|b| !b.is_empty())
}

/// Write a picture to the chat-images folder and return its URL.
pub fn save_chat_image(bytes: &[u8]) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    let dir = paths::work("chat-images");
    fs::create_dir_all(&dir).ok()?;
    let name = format!("{}-{:03}.png", paths::stamp(), IMG_COUNT.fetch_add(1, Ordering::SeqCst) + 1);
    img.save(dir.join(&name)).ok()?;
    Some(format!("/img/{name}"))
}

pub fn save_chat_image_b64(data: &str) -> Option<String> {
    save_chat_image(&decode(data)?)
}

/// Store a pasted, dropped or picked file. Returns {"name", "kind", "label"}.
///
/// Images are normalised to PNG. Text files and PDFs are kept as they are (the
/// agent can read both). Word, PowerPoint, Excel, OpenDocument and RTF files
/// also get a plain-text copy, because the agent cannot read those directly.
pub fn save_attachment(data: &str, filename: Option<&str>) -> Option<Value> {
    let raw = decode(data).filter(|r| r.len() <= MAX_BYTES)?;
    let dir = paths::work("attachments");
    fs::create_dir_all(&dir).ok()?;
    let stamp = format!("{}-{:02}", paths::stamp(), COUNT.fetch_add(1, Ordering::SeqCst) + 1);
    let filename = filename.map(|f| paths::base_name(f).trim().to_string()).filter(|f| !f.is_empty());
    let label = filename.clone().unwrap_or_else(|| "pasted image".to_string());
    let ext = label.rfind('.').map(|i| label[i..].to_lowercase()).unwrap_or_default();

    if filename.is_none() || IMAGE_EXT.contains(&ext.as_str()) {
        if let Ok(mut img) = image::load_from_memory(&raw) {
            if img.width().max(img.height()) > 2400 {
                img = img.thumbnail(2400, 2400);
            }
            let name = format!("{stamp}.png");
            let path = dir.join(&name);
            if img.to_rgba8().save(&path).is_ok() {
                remember(&Info { name: name.clone(), kind: "image".into(), label: label.clone(), read: path.display().to_string(), note: "image".into() });
                return Some(json!({"name": name, "kind": "image", "label": label}));
            }
        }
        if filename.is_none() {
            return None; // a paste that was not an image
        }
    }

    let safe: String = Regex::new(r"[^A-Za-z0-9._ -]+").unwrap().replace_all(&label, "_").to_string();
    let safe: String = safe.chars().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
    let name = format!("{stamp}-{}", if safe.is_empty() { "file" } else { &safe });
    let path = dir.join(&name);
    fs::write(&path, &raw).ok()?;
    let shown = path.display().to_string();
    let mut info = Info { name: name.clone(), kind: "doc".into(), label: label.clone(), read: shown.clone(), note: "document".into() };
    if OFFICE_EXT.contains(&ext.as_str()) {
        match extract_office_text(&raw, &ext) {
            Ok(text) if fs::write(format!("{shown}.txt"), &text).is_ok() => {
                info.read = format!("{shown}.txt");
                info.note = format!("text extracted from {label}; the original file is at {shown}");
            }
            _ => info.note = format!("{label} could not be converted to text; it cannot be read directly"),
        }
    } else if ext == ".pdf" {
        info.note = format!("PDF document {label}");
    } else {
        let sample = &raw[..raw.len().min(4096)];
        let looks_text = !sample.contains(&0) || sample.starts_with(&[0xff, 0xfe]) || sample.starts_with(&[0xfe, 0xff]);
        info.note = if looks_text {
            format!("document {label}")
        } else {
            format!("binary file {label}: it cannot be read as text, so say so if its contents matter")
        };
    }
    remember(&info);
    Some(json!({"name": name, "kind": "doc", "label": label}))
}

// ------------------------------------------------------------ office files
fn unescape(s: &str) -> String {
    let re = Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|amp|lt|gt|quot|apos);").unwrap();
    re.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        let ch = match e {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            _ => e[1..].parse::<u32>().ok().and_then(char::from_u32),
        };
        ch.map(String::from).unwrap_or_else(|| c[0].to_string())
    })
    .to_string()
}

/// Pull readable text out of an Office XML part, one line per paragraph.
fn xml_text(xml: &str, para_tags: &[&str], text_tags: &[&str]) -> String {
    let join = |tags: &[&str]| tags.iter().map(|t| regex::escape(t)).collect::<Vec<_>>().join("|");
    let para = Regex::new(&format!("</(?:{})>", join(para_tags))).unwrap();
    let text = Regex::new(&format!(r"<(?:{})(?:\s[^>]*)?>([^<]*)", join(text_tags))).unwrap();
    let lines: Vec<String> = para.split(xml).map(|p| text.captures_iter(p).map(|c| c[1].to_string()).collect::<String>()).collect();
    unescape(&lines.join("\n"))
}

fn last_number(s: &str) -> u32 {
    Regex::new(r"\d+").unwrap().find_iter(s).last().and_then(|m| m.as_str().parse().ok()).unwrap_or(0)
}

/// Plain text from .docx, .pptx, .xlsx, .odt or .rtf.
fn extract_office_text(raw: &[u8], ext: &str) -> Result<String, String> {
    if ext == ".rtf" {
        let text: String = raw.iter().map(|&b| b as char).collect();
        let text = Regex::new(r"\\pard?\b").unwrap().replace_all(&text, "\n");
        let text = Regex::new(r"\\'[0-9a-f]{2}").unwrap().replace_all(&text, "");
        let text = Regex::new(r"\\[a-zA-Z]+-?\d* ?").unwrap().replace_all(&text, "");
        return Ok(text.replace(['{', '}'], "").trim().to_string());
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(raw)).map_err(|e| e.to_string())?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|c| c.to_string())).collect();
    let mut part = |name: &str| -> Result<String, String> {
        let mut bytes = Vec::new();
        zip.by_name(name).map_err(|e| e.to_string())?.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    };
    match ext {
        ".docx" => Ok(xml_text(&part("word/document.xml")?, &["w:p"], &["w:t"])),
        ".odt" => Ok(xml_text(&part("content.xml")?, &["text:p", "text:h"], &["text:p", "text:h", "text:span"])),
        ".pptx" => {
            let is_slide = Regex::new(r"^ppt/slides/slide\d+\.xml$").unwrap();
            let mut slides: Vec<&String> = names.iter().filter(|n| is_slide.is_match(n)).collect();
            slides.sort_by_key(|n| last_number(n));
            let mut parts = Vec::new();
            for (k, n) in slides.iter().enumerate() {
                parts.push(format!("--- Slide {} ---\n{}", k + 1, xml_text(&part(n)?, &["a:p"], &["a:t"])));
            }
            Ok(parts.join("\n\n"))
        }
        ".xlsx" => {
            let t = Regex::new(r"<t[^>]*>([^<]*)</t>").unwrap();
            let mut shared: Vec<String> = Vec::new();
            if names.iter().any(|n| n == "xl/sharedStrings.xml") {
                let xml = part("xl/sharedStrings.xml")?;
                for si in Regex::new(r"(?s)<si>(.*?)</si>").unwrap().captures_iter(&xml) {
                    shared.push(unescape(&t.captures_iter(&si[1]).map(|c| c[1].to_string()).collect::<String>()));
                }
            }
            let is_sheet = Regex::new(r"^xl/worksheets/sheet\d+\.xml$").unwrap();
            let mut sheets: Vec<&String> = names.iter().filter(|n| is_sheet.is_match(n)).collect();
            sheets.sort();
            let row_re = Regex::new(r"(?s)<row[^>]*>(.*?)</row>").unwrap();
            let cell_re = Regex::new(r"(?s)<c([^>]*)>(.*?)</c>").unwrap();
            let v_re = Regex::new(r"<v>([^<]*)</v>").unwrap();
            let mut parts = Vec::new();
            for (k, n) in sheets.iter().enumerate() {
                let xml = part(n)?;
                let mut rows = Vec::new();
                for row in row_re.captures_iter(&xml).take(2000) {
                    let mut cells = Vec::new();
                    for c in cell_re.captures_iter(&row[1]) {
                        let (attrs, inner) = (&c[1], &c[2]);
                        let mut val = v_re.captures(inner).or_else(|| t.captures(inner)).map(|m| m[1].to_string()).unwrap_or_default();
                        if attrs.contains("t=\"s\"") {
                            if let Some(s) = val.parse::<usize>().ok().and_then(|i| shared.get(i)) {
                                val = s.clone();
                            }
                        }
                        cells.push(unescape(&val));
                    }
                    rows.push(cells.join("\t"));
                }
                parts.push(format!("--- Sheet {} ---\n{}", k + 1, rows.join("\n")));
            }
            Ok(parts.join("\n\n"))
        }
        _ => Ok(String::new()),
    }
}
