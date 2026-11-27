//! Copying a project to another of the user's computers when a chat moves there (the machine chip,
//! `ui/src/remote/move.ts`): its files go along, so either computer has them all. Only on a move, never on its own.
//!
//! `POST /api/project-copy` on each computer's app:
//! - `list` `{path}`: the project's files (each one's path, size and SHA-256), its name and kind;
//! - `target` `{name, kind}`: this computer's project of that name (found, else made in the usual projects folder),
//!   and its files;
//! - `read` `{path, pieces}` / `write` `{path, pieces}`: parts of files, base64, at most `PIECE` bytes a call, so a
//!   call stays well under what a paired computer may send in one request;
//! - `copy` `{from, path, to}`: run on the page's own computer, which asks the two (either may be itself) and passes
//!   on what differs. The project on the computer it comes from wins: a file that differs is replaced, a file only
//!   on the other one is left alone, nothing is deleted. Folders that are downloaded or built again (`SKIP_DIRS`)
//!   stay behind; the project's git history goes, but never over the other computer's own `.git/config` (its line
//!   endings and file modes are that system's).
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::projects;

/// Folders that are downloaded or built again on each computer: never copied.
const SKIP_DIRS: &[&str] = &["node_modules", ".venv", "venv", "__pycache__", ".pytest_cache", ".mypy_cache", ".gradle", ".next", ".turbo", ".parcel-cache", "build-out"];
/// The system's own notes in a folder.
const SKIP_FILES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];
/// At most this many bytes of files in one read or write.
pub const PIECE: usize = 12 * 1024 * 1024;
/// A project with more files than this is too many to copy this way.
const MOST_FILES: usize = 200_000;
/// What a file being written is called until it is whole.
const PART: &str = ".fridae-copy";

/// One file of a project: its path inside it ('/' between folders), size and SHA-256.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub rel: String,
    pub size: u64,
    pub hash: String,
}

/// Whether a path inside the project stays behind.
fn skipped(rel: &str, is_dir: bool, parent: &Path) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if is_dir {
        // Rust's build output, only where it is that
        return SKIP_DIRS.contains(&name) || (name == "target" && parent.join("Cargo.toml").is_file());
    }
    SKIP_FILES.contains(&name) || name.ends_with(PART) || (rel.starts_with(".git/") && name.ends_with(".lock"))
}

fn hash_of(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(crate::remote_common::hex(&h.finalize()))
}

/// The project's files, but for what stays behind. Links are not followed.
pub fn list(root: &Path) -> Result<Vec<File>, String> {
    if !root.is_dir() {
        return Err(format!("{} isn't there.", root.display()));
    }
    let mut out = Vec::new();
    let mut todo = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = todo.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(kind) = e.file_type() else { continue };
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            let rel = format!("{prefix}{name}");
            if kind.is_symlink() || skipped(&rel, kind.is_dir(), &dir) {
                continue;
            }
            if kind.is_dir() {
                todo.push((e.path(), format!("{rel}/")));
            } else if kind.is_file() {
                if out.len() >= MOST_FILES {
                    return Err("This project has too many files to copy to another computer.".into());
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                let hash = hash_of(&e.path()).map_err(|_| format!("Couldn't read {rel}."))?;
                out.push(File { rel, size, hash });
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// A path inside the project, as another computer named it: plain folder names only, never out of the project.
fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.len() > 1024 || rel.contains('\\') || rel.contains(':') || rel.contains('\0') {
        return None;
    }
    let mut out = root.to_path_buf();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(p) => out.push(p),
            _ => return None,
        }
    }
    // (what never goes is never asked for either)
    (!skipped(rel, false, Path::new(""))).then_some(out)
}

/// Part of a file, `len` bytes from `offset`.
pub fn read_piece(root: &Path, rel: &str, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    let path = inside(root, rel).ok_or("That file isn't in the project.")?;
    if len > PIECE {
        return Err("Too much at once.".into());
    }
    let mut f = fs::File::open(&path).map_err(|_| format!("Couldn't read {rel}."))?;
    f.seek(SeekFrom::Start(offset)).map_err(|_| format!("Couldn't read {rel}."))?;
    let mut buf = Vec::with_capacity(len);
    f.take(len as u64).read_to_end(&mut buf).map_err(|_| format!("Couldn't read {rel}."))?;
    Ok(buf)
}

/// Part of a file, written at `offset` of a file of `total` bytes. It is written beside its place (`PART`) and only
/// put in place once whole, so a copy cut short never leaves half a file.
pub fn write_piece(root: &Path, rel: &str, offset: u64, total: u64, data: &[u8]) -> Result<(), String> {
    let path = inside(root, rel).ok_or("That file isn't in the project.")?;
    let parent = path.parent().ok_or("That file isn't in the project.")?;
    fs::create_dir_all(parent).map_err(|_| format!("Couldn't make the folder for {rel}."))?;
    // a folder that is a link could lead out of the project
    let (real_root, real_parent) = (fs::canonicalize(root).map_err(|_| "The project isn't there.")?, fs::canonicalize(parent).map_err(|_| "The project isn't there.")?);
    if !real_parent.starts_with(&real_root) {
        return Err("That file isn't in the project.".into());
    }
    let part = parent.join(format!("{}{PART}", path.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
    let mut f = if offset == 0 {
        fs::File::create(&part)
    } else {
        fs::OpenOptions::new().append(true).open(&part)
    }
    .map_err(|_| format!("Couldn't write {rel}."))?;
    if f.metadata().map(|m| m.len()).unwrap_or(u64::MAX) != offset {
        return Err(format!("{rel} came in the wrong order."));
    }
    f.write_all(data).map_err(|_| format!("Couldn't write {rel}."))?;
    drop(f);
    if offset + data.len() as u64 >= total {
        fs::rename(&part, &path).map_err(|_| format!("Couldn't put {rel} in place. Is it open in another program?"))?;
    }
    Ok(())
}

/// The files as JSON: `[[path, size, hash], ...]`.
fn files_json(files: &[File]) -> Value {
    files.iter().map(|f| json!([f.rel, f.size, f.hash])).collect()
}

fn files_of(v: &Value) -> Vec<File> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| Some(File { rel: f[0].as_str()?.to_string(), size: f[1].as_u64()?, hash: f[2].as_str()?.to_string() }))
        .collect()
}

/// What to send: every file of `from` that `to` lacks or has otherwise, but never over `to`'s own `.git/config`.
pub fn wanted(from: &[File], to: &[File]) -> Vec<File> {
    let there: std::collections::HashMap<&str, &File> = to.iter().map(|f| (f.rel.as_str(), f)).collect();
    from.iter()
        .filter(|f| match there.get(f.rel.as_str()) {
            Some(_) if f.rel == ".git/config" => false,
            Some(t) => t.size != f.size || t.hash != f.hash,
            None => true,
        })
        .cloned()
        .collect()
}

/// This computer's project called `name` (any case), else one made for it in the usual projects folder (or the
/// folder of that name there, if one is waiting).
fn target(name: &str, kind: Option<&str>) -> Result<String, String> {
    let name = name.trim();
    if let Some(p) = projects::list().into_iter().find(|p| p["name"].as_str().is_some_and(|n| n.trim().eq_ignore_ascii_case(name))) {
        return Ok(p["path"].as_str().unwrap_or("").to_string());
    }
    let folder: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let parent = if projects::kind_word(kind) == Some("plain") { projects::default_parent_plain() } else { projects::default_parent() };
    let dir = parent.join(folder.trim());
    let made = if dir.is_dir() { projects::add(&dir.display().to_string(), kind) } else { projects::create(name, Some(&parent.display().to_string()), false, kind) };
    made.map(|p| p["path"].as_str().unwrap_or("").to_string())
}

fn ok(mut v: Value) -> Value {
    v["ok"] = json!(true);
    v
}

fn refused(why: impl Into<String>) -> Value {
    json!({"ok": false, "error": why.into()})
}

/// `POST /api/project-copy` (all but `copy`, which `copy` itself sends to each computer). Only a project on this
/// computer's list is read or written.
pub fn api(body: &Value) -> Value {
    let path = body["path"].as_str().unwrap_or("").to_string();
    let known = !path.is_empty() && projects::known(&path);
    match body["action"].as_str().unwrap_or("") {
        "copy" => copy(body),
        "target" => match target(body["name"].as_str().unwrap_or(""), body["kind"].as_str()) {
            Ok(p) => on(&p, body),
            Err(e) => refused(e),
        },
        _ if !known => refused("That project isn't on this computer's list."),
        _ => on(&path, body),
    }
}

/// `list`, `target` (its answer), `read` and `write` on the project at `path` (checked by the caller).
pub fn on(path: &str, body: &Value) -> Value {
    let root = Path::new(path);
    match body["action"].as_str().unwrap_or("") {
        "list" | "target" => match list(root) {
            Ok(files) => {
                let d = projects::describe(path);
                ok(json!({"path": path, "name": d["name"], "kind": d["kind"], "files": files_json(&files)}))
            }
            Err(e) => refused(e),
        },
        "read" => {
            let mut out = Vec::new();
            let mut total = 0usize;
            for p in body["pieces"].as_array().into_iter().flatten() {
                let (rel, offset, len) = (p[0].as_str().unwrap_or(""), p[1].as_u64().unwrap_or(0), p[2].as_u64().unwrap_or(0) as usize);
                total += len;
                if total > PIECE {
                    return refused("Too much at once.");
                }
                match read_piece(root, rel, offset, len) {
                    Ok(b) => out.push(json!(B64.encode(b))),
                    Err(e) => return refused(e),
                }
            }
            ok(json!({"data": out}))
        }
        "write" => {
            for p in body["pieces"].as_array().into_iter().flatten() {
                let (rel, offset, total) = (p[0].as_str().unwrap_or(""), p[1].as_u64().unwrap_or(0), p[2].as_u64().unwrap_or(0));
                let Ok(data) = B64.decode(p[3].as_str().unwrap_or("")) else { return refused("A piece came broken.") };
                if let Err(e) = write_piece(root, rel, offset, total, &data) {
                    return refused(e);
                }
            }
            ok(json!({}))
        }
        _ => refused("Unknown action."),
    }
}

/// One computer's `/api/project-copy`: this one's own (`m` empty), else the paired computer's, over its connection.
fn call(m: &str, body: Value) -> Value {
    if m.is_empty() {
        return api(&body);
    }
    let raw = body.to_string().into_bytes();
    let heads = [("Content-Type".to_string(), "application/json".to_string())];
    let a = crate::remote_client::forward_within(crate::remote_client::book(), m, "POST", "/api/project-copy", &heads, &raw, std::time::Duration::from_secs(600));
    let mut text = Vec::new();
    let _ = a.body.take(64_000_000).read_to_end(&mut text);
    match serde_json::from_slice::<Value>(&text) {
        Ok(v) if a.status == 200 => v,
        _ => refused(if a.status == 404 { "Update Fridae on that computer to copy projects to it." } else { "Couldn't reach that computer." }),
    }
}

/// `copy` `{from, path, to}`: the project at `path` on computer `from` copied to `to` ("" is this one).
fn copy(body: &Value) -> Value {
    let (from, to) = (body["from"].as_str().unwrap_or("").to_string(), body["to"].as_str().unwrap_or("").to_string());
    if from == to {
        return refused("That's the same computer.");
    }
    let path = body["path"].as_str().unwrap_or("").to_string();
    copy_with(&|v| call(&from, v), &|v| call(&to, v), &path)
}

/// The copy itself, with each side's `/api/project-copy` as `src` and `dst`. Its answer: the project's path on `dst`,
/// and how many files and bytes went.
pub fn copy_with(src: &dyn Fn(Value) -> Value, dst: &dyn Fn(Value) -> Value, path: &str) -> Value {
    let have = src(json!({"action": "list", "path": path}));
    if have["ok"] != true {
        return have;
    }
    let there = dst(json!({"action": "target", "name": have["name"], "kind": have["kind"]}));
    if there["ok"] != true {
        return there;
    }
    let to_path = there["path"].as_str().unwrap_or("").to_string();
    let send = wanted(&files_of(&have["files"]), &files_of(&there["files"]));
    // in pieces of at most PIECE bytes, a large file in several
    let mut batch: Vec<(String, u64, usize, u64)> = Vec::new();
    let mut size = 0usize;
    let mut bytes = 0u64;
    let flush = |batch: &mut Vec<(String, u64, usize, u64)>, size: &mut usize| -> Result<(), Value> {
        if batch.is_empty() {
            return Ok(());
        }
        let ask: Vec<Value> = batch.iter().map(|(r, o, n, _)| json!([r, o, n])).collect();
        let got = src(json!({"action": "read", "path": path, "pieces": ask}));
        if got["ok"] != true {
            return Err(got);
        }
        let data = got["data"].as_array().cloned().unwrap_or_default();
        if data.len() != batch.len() {
            return Err(refused("A piece went missing on the way."));
        }
        let put: Vec<Value> = batch.iter().zip(data).map(|((r, o, _, t), d)| json!([r, o, t, d])).collect();
        let wrote = dst(json!({"action": "write", "path": to_path, "pieces": put}));
        if wrote["ok"] != true {
            return Err(wrote);
        }
        batch.clear();
        *size = 0;
        Ok(())
    };
    for f in &send {
        let mut offset = 0u64;
        loop {
            let n = ((f.size - offset) as usize).min(PIECE);
            if size + n > PIECE {
                if let Err(e) = flush(&mut batch, &mut size) {
                    return e;
                }
            }
            batch.push((f.rel.clone(), offset, n, f.size));
            size += n;
            offset += n as u64;
            if offset >= f.size {
                break;
            }
        }
        bytes += f.size;
    }
    if let Err(e) = flush(&mut batch, &mut size) {
        return e;
    }
    ok(json!({"path": to_path, "files": send.len(), "bytes": bytes}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> PathBuf {
        let d = crate::paths::test_dir(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, text: &[u8]) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// Each side's answers, as `api` gives them, on folders of this test's own (no project list).
    fn side(root: PathBuf) -> impl Fn(Value) -> Value {
        move |mut v: Value| {
            if v["action"] == "target" {
                v["action"] = json!("list");
            }
            let mut a = on(&root.display().to_string(), &v);
            if a["ok"] == true && a.get("name").is_some() {
                a["name"] = json!("Castle");
            }
            a
        }
    }

    #[test]
    fn a_project_is_copied_whole_but_for_what_is_built_again() {
        let (a, b) = (fresh("copy-a"), fresh("copy-b"));
        put(&a, "src/main.luau", b"print('hi')");
        put(&a, "assets/logo.png", &[0, 1, 2, 255]);
        put(&a, "empty.txt", b"");
        put(&a, ".git/HEAD", b"ref: refs/heads/main\n");
        put(&a, ".git/config", b"[core]\n\tfilemode = true\n");
        put(&a, ".git/index.lock", b"x");
        put(&a, "node_modules/x/index.js", b"big");
        put(&a, "target/debug/app", b"built");
        put(&a, "Cargo.toml", b"[package]");
        put(&a, ".DS_Store", b"x");
        // the other computer's own: kept
        put(&b, "notes.txt", b"only there");
        put(&b, ".git/config", b"[core]\n\tautocrlf = true\n");
        put(&b, "src/main.luau", b"old");
        let got = copy_with(&side(a.clone()), &side(b.clone()), &a.display().to_string());
        assert_eq!(got["ok"], true, "{got}");
        assert_eq!(got["path"], b.display().to_string());
        assert_eq!(fs::read(b.join("src/main.luau")).unwrap(), b"print('hi')", "a file that differs: the computer it comes from wins");
        assert_eq!(fs::read(b.join("assets/logo.png")).unwrap(), [0, 1, 2, 255]);
        assert_eq!(fs::read(b.join("empty.txt")).unwrap(), b"");
        assert_eq!(fs::read(b.join(".git/HEAD")).unwrap(), b"ref: refs/heads/main\n", "its history goes");
        assert_eq!(fs::read_to_string(b.join(".git/config")).unwrap(), "[core]\n\tautocrlf = true\n", "never over the other computer's own git settings");
        assert_eq!(fs::read(b.join("notes.txt")).unwrap(), b"only there", "nothing is deleted");
        for left in [".git/index.lock", "node_modules", "target", ".DS_Store"] {
            assert!(!b.join(left).exists(), "{left} stays behind");
        }
        assert!(list(&b).unwrap().iter().all(|f| !f.rel.ends_with(PART)), "no half-written file is left");
        // again: nothing differs, nothing goes
        let again = copy_with(&side(a.clone()), &side(b.clone()), &a.display().to_string());
        assert_eq!((again["files"].as_u64(), again["bytes"].as_u64()), (Some(0), Some(0)));
    }

    #[test]
    fn a_large_file_goes_in_pieces() {
        let (a, b) = (fresh("copy-big-a"), fresh("copy-big-b"));
        let big: Vec<u8> = (0..PIECE * 2 + 1234).map(|i| (i * 7 % 251) as u8).collect();
        put(&a, "video.mp4", &big);
        put(&a, "small.txt", b"small");
        let (calls, most) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let dst = side(b.clone());
        let got = copy_with(&side(a.clone()), &|v| {
            if v["action"] == "write" {
                calls.set(calls.get() + 1);
                let sent: usize = v["pieces"].as_array().unwrap().iter().map(|p| B64.decode(p[3].as_str().unwrap()).unwrap().len()).sum();
                most.set(most.get().max(sent));
            }
            dst(v)
        }, &a.display().to_string());
        assert_eq!(got["ok"], true, "{got}");
        assert_eq!(fs::read(b.join("video.mp4")).unwrap(), big);
        assert_eq!(fs::read(b.join("small.txt")).unwrap(), b"small");
        assert!(most.get() <= PIECE, "no write is over a piece's size");
        assert!(calls.get() >= 3);
    }

    #[test]
    fn nothing_is_written_outside_the_project() {
        let root = fresh("copy-inside");
        for bad in ["../x", "/etc/x", "a/../../x", "C:/x", "a\\..\\x", "", "a/./b/../../../x"] {
            assert!(write_piece(&root, bad, 0, 1, b"x").is_err(), "{bad}");
            assert!(read_piece(&root, bad, 0, 1).is_err(), "{bad}");
        }
        assert!(write_piece(&root, "ok/file.txt", 0, 2, b"hi").is_ok());
        assert_eq!(fs::read(root.join("ok/file.txt")).unwrap(), b"hi");
        assert!(write_piece(&root, "ok/file.txt", 5, 9, b"x").is_err(), "a piece out of order is refused");
        assert!(read_piece(&root, "ok/file.txt", 0, PIECE + 1).is_err());
    }
}
