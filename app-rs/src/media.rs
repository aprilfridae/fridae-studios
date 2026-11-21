//! Serving a large file (a rendered promo) to the window's <video>, with byte
//! ranges so the player can seek. The file is streamed, never read whole.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use tiny_http::{Header, Response, StatusCode};

pub type Streamed = Response<Box<dyn Read + Send>>;

/// The first and last byte (inclusive) a `Range` header asks for, in a file of `len` bytes.
/// None when it cannot be met or is not one plain byte range (the caller answers 416).
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        // "bytes=-500": the last 500 bytes
        let n: u64 = b.parse().ok().filter(|n| *n > 0)?;
        return Some((len.saturating_sub(n), len - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
    (start < len && start <= end).then_some((start, end))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn answer(code: u16, headers: Vec<Header>, body: Box<dyn Read + Send>, len: u64) -> Streamed {
    Response::new(StatusCode(code), headers, body, Some(len as usize), None)
}

fn empty(code: u16, mut headers: Vec<Header>) -> Streamed {
    headers.push(header("Content-Type", "text/plain"));
    answer(code, headers, Box::new(std::io::empty()), 0)
}

pub fn missing() -> Streamed {
    empty(404, vec![])
}

/// The file at `path` as `ctype`: whole (200) with no `range`, else the asked-for part (206), or 416.
pub fn serve(path: &Path, ctype: &str, range: &str) -> Streamed {
    let Ok(mut file) = File::open(path) else { return missing() };
    let Some(len) = file.metadata().ok().filter(|m| m.is_file()).map(|m| m.len()) else { return missing() };
    let mut headers = vec![header("Content-Type", ctype), header("Accept-Ranges", "bytes"), header("Cache-Control", "no-store")];
    if range.trim().is_empty() {
        return answer(200, headers, Box::new(file), len);
    }
    let Some((start, end)) = parse_range(range, len) else {
        return empty(416, vec![header("Content-Range", &format!("bytes */{len}"))]);
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return empty(500, vec![]);
    }
    headers.push(header("Content-Range", &format!("bytes {start}-{end}/{len}")));
    let n = end - start + 1;
    answer(206, headers, Box::new(file.take(n)), n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=2000-", 1000), None);
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=990-5000", 1000), Some((990, 999)));
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    fn get(path: &Path, range: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let r = serve(path, "video/mp4", range);
        let code = r.status_code().0;
        let heads = r.headers().iter().map(|h| (h.field.to_string(), h.value.to_string())).collect();
        let mut body = Vec::new();
        r.into_reader().read_to_end(&mut body).unwrap();
        (code, heads, body)
    }

    fn has(heads: &[(String, String)], k: &str, v: &str) -> bool {
        heads.iter().any(|(a, b)| a.eq_ignore_ascii_case(k) && b == v)
    }

    #[test]
    fn serves_whole_part_and_refuses() {
        let dir = crate::paths::test_dir("media");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip 1.mp4");
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();

        let (code, heads, body) = get(&path, "");
        assert_eq!((code, body.len()), (200, 1000));
        assert!(has(&heads, "Accept-Ranges", "bytes") && has(&heads, "Content-Type", "video/mp4"));

        let (code, heads, body) = get(&path, "bytes=900-");
        assert_eq!(code, 206);
        assert_eq!(body, data[900..]);
        assert!(has(&heads, "Content-Range", "bytes 900-999/1000") && has(&heads, "Accept-Ranges", "bytes"));

        let (code, _, body) = get(&path, "bytes=10-19");
        assert_eq!((code, body), (206, data[10..20].to_vec()));

        let (code, heads, body) = get(&path, "bytes=2000-");
        assert_eq!((code, body.len()), (416, 0));
        assert!(has(&heads, "Content-Range", "bytes */1000"));

        assert_eq!(get(&dir.join("missing.mp4"), "").0, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
