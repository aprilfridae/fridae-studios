//! Remote control, the client side: the computers this one paired with, finding them on the network, pairing, and
//! passing the page's calls for one of them (`/m/<machine id>/...`) on to it.
//!
//! Every connection to another computer is TLS with that computer's own self-signed certificate. There is no
//! authority to vouch for it, so the certificate's fingerprint is learnt once, at pairing (where the pairing proof
//! ties it to the code the user typed, see `remote_common::proof`), and every later connection must show the same
//! certificate before a single byte is sent. That is why this module speaks HTTP/1.1 itself over a plain
//! `native_tls` stream instead of using ureq: the check has to sit between the handshake and the request, and the
//! answer is passed back to the page as a stream (a video is never read whole).
//!
//! The token each host gave us is kept in `machines.json` encrypted with Windows DPAPI for the current user, so the
//! file alone is no use on another account or computer.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use native_tls::{TlsConnector, TlsStream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::remote_common::{self, DISCOVER_ASK, DISCOVER_PORT, LISTEN_PORT, same};

/// How long to wait for another computer to accept a connection.
const CONNECT_WAIT: Duration = Duration::from_secs(5);
/// How long one read may wait, and how long a forwarded call may take to start its answer: longer than the page's
/// long-poll (`/api/events?wait=20`).
const READ_WAIT: Duration = Duration::from_secs(60);
/// The whole of a small call (hello, pair, the background check), connecting included. Pairing's host works out the
/// slow proof inside it.
const QUICK_WAIT: Duration = Duration::from_secs(15);
/// The longest status, header or chunk-size line read from another computer, and the most header lines: a host
/// that never ends a line cannot fill this computer's memory.
const LINE_LIMIT: u64 = 8 * 1024;
const MAX_HEADERS: usize = 100;
/// The largest request body passed on, as the local server allows.
const BODY_LIMIT: u64 = 64_000_000;
/// How often every saved computer is asked whether it is there.
const CHECK_EVERY: Duration = Duration::from_secs(20);
/// How long finding waits for answers.
const FIND_WAIT: Duration = Duration::from_millis(1500);
/// Mixed into DPAPI's encryption so another program's DPAPI call cannot decrypt the tokens by accident.
const DPAPI_SALT: &[u8] = b"fridae-remote-client-v1";

const UNREACHABLE: &str = "Couldn't reach that computer. Check that it's on, Fridae is open there with network access turned on, and the address is right.";

/// A computer this one paired with. `token` is None once the host has said we are no longer paired.
pub struct Machine {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub fp: String,
    pub token: Option<String>,
    pub online: bool,
    pub last_seen: u64,
}

impl Machine {
    /// What the page sees: everything but the token.
    fn public(&self) -> Value {
        json!({"id": self.id, "name": self.name, "addr": self.addr, "port": self.port, "fp": self.fp,
               "online": self.online, "last_seen": self.last_seen, "paired": self.token.is_some()})
    }
}

/// The saved computers and the file they live in.
pub struct Book {
    pub(crate) path: PathBuf,
    list: Mutex<Vec<Machine>>,
}

impl Book {
    pub fn open(path: &Path) -> Book {
        let saved: Value = fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let text = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
        let list = saved["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| !text(v, "id").is_empty())
            .map(|v| Machine {
                id: text(v, "id"),
                name: text(v, "name"),
                addr: text(v, "addr"),
                port: v["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(LISTEN_PORT),
                fp: text(v, "fp"),
                token: v["token"].as_str().and_then(|b| B64.decode(b).ok()).and_then(|b| unprotect(&b)).and_then(|b| String::from_utf8(b).ok()),
                online: false,
                last_seen: v["last_seen"].as_u64().unwrap_or(0),
            })
            .collect();
        Book { path: path.to_path_buf(), list: Mutex::new(list) }
    }

    fn save(&self, list: &[Machine]) {
        let out: Vec<Value> = list
            .iter()
            .map(|m| {
                let token = m.token.as_ref().and_then(|t| protect(t.as_bytes())).map(|b| B64.encode(b));
                json!({"id": m.id, "name": m.name, "addr": m.addr, "port": m.port, "fp": m.fp, "token": token, "last_seen": m.last_seen})
            })
            .collect();
        // written beside and renamed over, so a crash mid-write never leaves half a file
        let tmp = self.path.with_extension("json.tmp");
        if fs::write(&tmp, serde_json::to_string_pretty(&json!({"machines": out})).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }

    /// Change the saved computers and write them out.
    fn change(&self, f: impl FnOnce(&mut Vec<Machine>)) {
        let mut list = self.list.lock().unwrap();
        f(&mut list);
        self.save(&list);
    }

    /// Add a computer, replacing one with the same id.
    pub(crate) fn add(&self, m: Machine) {
        self.change(|list| {
            list.retain(|x| x.id != m.id);
            list.push(m);
        });
    }

    #[cfg(test)]
    pub(crate) fn find(&self, id: &str) -> Option<Value> {
        self.list.lock().unwrap().iter().find(|m| m.id == id).map(Machine::public)
    }

    #[cfg(test)]
    pub(crate) fn token_of(&self, id: &str) -> Option<String> {
        self.list.lock().unwrap().iter().find(|m| m.id == id).and_then(|m| m.token.clone())
    }

    /// Where and how to reach a computer: (addr, port, fingerprint, token).
    fn route(&self, id: &str) -> Option<(String, u16, String, Option<String>)> {
        self.list.lock().unwrap().iter().find(|m| m.id == id).map(|m| (m.addr.clone(), m.port, m.fp.clone(), m.token.clone()))
    }

    fn all(&self) -> Vec<Value> {
        self.list.lock().unwrap().iter().map(Machine::public).collect()
    }

    fn ids(&self) -> Vec<String> {
        self.list.lock().unwrap().iter().map(|m| m.id.clone()).collect()
    }

    /// Note whether a computer answered. Only written out when it changes something worth keeping: `last_seen`
    /// moves on every call, and the page calls often.
    fn seen(&self, id: &str, online: bool) {
        let mut list = self.list.lock().unwrap();
        let Some(m) = list.iter_mut().find(|m| m.id == id) else { return };
        m.online = online;
        if online {
            let now = unix_now();
            let worth = now.saturating_sub(m.last_seen) >= 60;
            m.last_seen = now;
            if worth {
                self.save(&list);
            }
        }
    }

    fn unpaired(&self, id: &str) {
        self.change(|list| {
            if let Some(m) = list.iter_mut().find(|m| m.id == id) {
                m.token = None;
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn set_fp(&self, id: &str, fp: &str) {
        self.change(|list| list.iter_mut().filter(|m| m.id == id).for_each(|m| m.fp = fp.into()));
    }

    #[cfg(test)]
    pub(crate) fn set_port(&self, id: &str, port: u16) {
        self.change(|list| list.iter_mut().filter(|m| m.id == id).for_each(|m| m.port = port));
    }

    fn forget(&self, id: &str) {
        self.change(|list| list.retain(|m| m.id != id));
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The app's saved computers. The first use also starts the check that asks each of them every 20 seconds
/// whether it is there; the page asks for the list as soon as it opens.
fn book() -> &'static Book {
    static BOOK: OnceLock<Book> = OnceLock::new();
    BOOK.get_or_init(|| {
        thread::spawn(|| {
            loop {
                check_all(book());
                thread::sleep(CHECK_EVERY);
            }
        });
        Book::open(&remote_common::dir().join("machines.json"))
    })
}

// ---- DPAPI ----

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData},
    };
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr().cast_mut() };
    let salt = CRYPT_INTEGER_BLOB { cbData: DPAPI_SALT.len() as u32, pbData: DPAPI_SALT.as_ptr().cast_mut() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: the in-blobs point at live slices for the call; the out-blob is allocated by the system with
    // LocalAlloc, copied, then freed with LocalFree as the documentation asks.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, null(), &salt, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, null_mut(), &salt, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(out.pbData.cast()) };
    Some(bytes)
}

/// Elsewhere there is nothing to keep a token safe with, so none is kept (pairing says "Only on Windows for now.").
#[cfg(not(windows))]
fn dpapi(_data: &[u8], _encrypt: bool) -> Option<Vec<u8>> {
    None
}

fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    dpapi(plain, true)
}

fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
    dpapi(blob, false)
}

// ---- the pinned connection and a small HTTP/1.1 client ----

/// Why a call to another computer did not get an answer.
#[derive(Debug, PartialEq)]
enum Fail {
    /// It could not be reached, or the connection broke.
    Offline,
    /// It answered with a certificate other than the one pinned.
    Changed,
}

/// The connection under TLS, with an overall deadline: each read and write may wait only until `end` (and never
/// longer than `READ_WAIT`), so a host that sends one byte at a time cannot hold a call open for ever. `end` is
/// None while a forwarded answer's body streams (a video takes as long as it takes, but each read still has 60 s).
struct Timed {
    tcp: TcpStream,
    end: Option<Instant>,
}

impl Timed {
    fn wait(&self) -> io::Result<Duration> {
        let Some(end) = self.end else { return Ok(READ_WAIT) };
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() { Err(io::ErrorKind::TimedOut.into()) } else { Ok(left.min(READ_WAIT)) }
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.tcp.set_read_timeout(Some(self.wait()?))?;
        self.tcp.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tcp.set_write_timeout(Some(self.wait()?))?;
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

/// One line (status, header, chunk size), at most `LINE_LIMIT` bytes, without its line ending.
fn read_line<R: BufRead>(r: &mut R) -> io::Result<String> {
    let mut raw = Vec::new();
    r.take(LINE_LIMIT).read_until(b'\n', &mut raw)?;
    match raw.last() {
        Some(b'\n') => Ok(String::from_utf8_lossy(&raw).trim_end().to_string()),
        None => Err(io::ErrorKind::UnexpectedEof.into()),
        Some(_) => Err(io::ErrorKind::InvalidData.into()),
    }
}

fn fingerprint(tls: &TlsStream<Timed>) -> Option<String> {
    let der = tls.peer_certificate().ok()??.to_der().ok()?;
    Some(remote_common::hex(&Sha256::digest(der)))
}

/// Open TLS to `addr:port`. With `pin`, the certificate must be that one; without (only while pairing), whatever
/// it shows is returned for the caller to check. Nothing is written before this returns.
/// The handshake is inside the deadline `end` too.
fn connect(addr: &str, port: u16, pin: Option<&str>, end: Instant) -> Result<(TlsStream<Timed>, String), Fail> {
    let wait = CONNECT_WAIT.min(end.saturating_duration_since(Instant::now())).max(Duration::from_millis(1));
    let tcp = (addr, port)
        .to_socket_addrs()
        .map_err(|_| Fail::Offline)?
        .find_map(|a| TcpStream::connect_timeout(&a, wait).ok())
        .ok_or(Fail::Offline)?;
    // a self-signed certificate on an address no authority knows: the pin below is the check instead
    let connector = TlsConnector::builder().danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true).build().map_err(|_| Fail::Offline)?;
    let tls = connector.connect(addr, Timed { tcp, end: Some(end) }).map_err(|_| Fail::Offline)?;
    let fp = fingerprint(&tls).ok_or(Fail::Offline)?;
    match pin {
        Some(p) if !same(&fp, &p.to_ascii_lowercase()) => Err(Fail::Changed),
        _ => Ok((tls, fp)),
    }
}

/// An answer from another computer, its body still on the wire.
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
    pub len: Option<u64>,
}

impl Answer {
    fn json(status: u16, v: Value) -> Answer {
        let b = v.to_string().into_bytes();
        let len = b.len() as u64;
        Answer { status, headers: vec![("Content-Type".into(), "application/json".into())], body: Box::new(io::Cursor::new(b)), len: Some(len) }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A body sent with `Transfer-Encoding: chunked`, read back as plain bytes.
struct Chunked<R> {
    inner: R,
    left: u64,
    done: bool,
}

impl<R: BufRead> Chunked<R> {
    fn line(&mut self) -> io::Result<String> {
        read_line(&mut self.inner)
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let size = self.line()?;
            let size = size.split(';').next().unwrap_or("").trim();
            self.left = u64::from_str_radix(size, 16).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            if self.left == 0 {
                // the trailer, if any, up to its empty line
                let mut lines = 0;
                while !self.line()?.is_empty() {
                    lines += 1;
                    if lines > MAX_HEADERS {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                }
                self.done = true;
                return Ok(0);
            }
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= n as u64;
        if self.left == 0 {
            self.line()?; // the CRLF after each chunk
        }
        Ok(n)
    }
}

/// One request over a fresh pinned connection (`Connection: close`); returns the answer and the fingerprint seen.
/// Connecting, sending and the answer's status and headers must all happen within `wait`; with `whole`, reading the
/// body too, else the body only has `READ_WAIT` for each read.
#[allow(clippy::too_many_arguments)]
fn request(addr: &str, port: u16, pin: Option<&str>, method: &str, target: &str, headers: &[(String, String)], body: &[u8], wait: Duration, whole: bool) -> Result<(Answer, String), Fail> {
    let (mut tls, fp) = connect(addr, port, pin, Instant::now() + wait)?;
    let host = if addr.contains(':') { format!("[{addr}]:{port}") } else { format!("{addr}:{port}") };
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() || !matches!(method, "GET" | "HEAD") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    tls.write_all(head.as_bytes()).and_then(|_| tls.write_all(body)).and_then(|_| tls.flush()).map_err(|_| Fail::Offline)?;

    let mut r = BufReader::new(tls);
    let line = read_line(&mut r).map_err(|_| Fail::Offline)?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or(Fail::Offline)?;
    let mut heads = Vec::new();
    loop {
        let h = read_line(&mut r).map_err(|_| Fail::Offline)?;
        if h.is_empty() {
            break;
        }
        if heads.len() >= MAX_HEADERS {
            return Err(Fail::Offline);
        }
        if let Some((k, v)) = h.split_once(':') {
            heads.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    if !whole {
        r.get_mut().get_mut().end = None;
    }
    let mut answer = Answer { status, headers: heads, body: Box::new(io::empty()), len: Some(0) };
    let chunked = answer.header("Transfer-Encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length = answer.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
    if method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status) {
        // no body follows
    } else if chunked {
        answer.body = Box::new(Chunked { inner: r, left: 0, done: false });
        answer.len = None;
    } else if let Some(n) = length {
        answer.body = Box::new(r.take(n));
        answer.len = Some(n);
    } else {
        // neither: the body runs until the host closes the connection
        answer.body = Box::new(r);
        answer.len = None;
    }
    Ok((answer, fp))
}

/// A small JSON call (hello, pair, the check): the answer's status and body, all within `wait`.
fn call_json(addr: &str, port: u16, pin: Option<&str>, method: &str, path: &str, body: Option<&Value>, wait: Duration) -> Result<(u16, Value, String), Fail> {
    let raw = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    let heads = if body.is_some() { vec![("Content-Type".to_string(), "application/json".to_string())] } else { vec![] };
    let (answer, fp) = request(addr, port, pin, method, path, &heads, &raw, wait, true)?;
    let mut text = Vec::new();
    answer.body.take(1_000_000).read_to_end(&mut text).map_err(|_| Fail::Offline)?;
    Ok((answer.status, serde_json::from_slice(&text).unwrap_or_default(), fp))
}

// ---- pairing ----

/// `addr` as typed: a name or IP, maybe with `:port` (only when no port was given, and not an IPv6 address).
fn address(addr: &str, port: Option<u16>) -> (String, u16) {
    let addr = addr.trim().trim_start_matches('[').trim_end_matches(']');
    if port.is_none()
        && addr.matches(':').count() == 1
        && let Some((a, p)) = addr.split_once(':')
        && let Ok(p) = p.parse()
    {
        return (a.to_string(), p);
    }
    (addr.to_string(), port.unwrap_or(LISTEN_PORT))
}

fn failed(why: &str) -> Value {
    json!({"ok": false, "error": why})
}

/// POST /api/remote/pair `{addr, port?, code, fp?}`: connect, check the certificate against the one found (if
/// any), say hello, send the proof, and save the computer with the token it gives.
pub fn pair_with(book: &Book, me: &Value, body: &Value) -> Value {
    if !cfg!(windows) {
        return failed("Only on Windows for now.");
    }
    let code = remote_common::normal_code(body["code"].as_str().unwrap_or(""));
    let port = body["port"].as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0);
    let (addr, port) = address(body["addr"].as_str().unwrap_or(""), port);
    if addr.is_empty() {
        return failed("Type the other computer's name or address.");
    }
    if code.is_empty() {
        return failed("Type the pairing code shown on the other computer.");
    }
    // a computer from finding has told us its certificate: hold it to that before sending anything. Only one typed
    // in by address is met without a pin, and the proof below ties the certificate it shows to the code.
    let found = body["fp"].as_str().filter(|f| !f.is_empty());
    let (status, hello, fp) = match call_json(&addr, port, found, "GET", "/remote/hello", None, QUICK_WAIT) {
        Ok(h) => h,
        Err(Fail::Changed) => return failed("That computer's identity didn't match the one found. Try finding it again."),
        Err(Fail::Offline) => return failed(UNREACHABLE),
    };
    let host_id = hello["id"].as_str().unwrap_or("").to_string();
    if status != 200 || hello["fridae"] != 1 || host_id.is_empty() {
        return failed("That address doesn't answer like Fridae.");
    }
    if hello["pairing"] == false {
        return failed("That computer isn't showing a pairing code. Make a new code in its Settings.");
    }
    let my_id = me["id"].as_str().unwrap_or("");
    let ask = json!({"device_id": my_id, "device_name": me["name"], "proof": remote_common::proof(&code, &fp, my_id)});
    // pinned to the certificate the proof was made for: a different one now means someone sits in between
    let answer = match call_json(&addr, port, Some(&fp), "POST", "/remote/pair", Some(&ask), QUICK_WAIT) {
        Ok((_, v, _)) => v,
        Err(Fail::Changed) => return failed("That computer's identity changed while pairing. Try again."),
        Err(Fail::Offline) => return failed(UNREACHABLE),
    };
    let token = answer["token"].as_str().unwrap_or("");
    if answer["ok"] != true || token.is_empty() {
        return failed(answer["error"].as_str().unwrap_or("That computer didn't accept the pairing. Try a new code."));
    }
    if protect(token.as_bytes()).is_none() {
        return failed("Couldn't save that computer safely on this one.");
    }
    let m = Machine {
        id: host_id.clone(),
        name: hello["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or("Other computer").to_string(),
        addr,
        port,
        fp,
        token: Some(token.to_string()),
        online: true,
        last_seen: unix_now(),
    };
    let shown = m.public();
    book.add(m);
    json!({"ok": true, "machine": shown})
}

// ---- forwarding ----

/// The query without the page token `t`, which never leaves this computer.
pub fn without_token(query: &str) -> String {
    query.split('&').filter(|kv| !kv.is_empty() && *kv != "t" && !kv.starts_with("t=")).collect::<Vec<_>>().join("&")
}

/// `/m/<id>/<rest>?<query>` → (id, `/<rest>?<query>`), the query as the page sent it.
pub fn split_m(url: &str) -> Option<(String, String)> {
    let after = url.strip_prefix("/m/")?;
    let (path, query) = after.split_once('?').map_or((after, None), |(p, q)| (p, Some(q)));
    let (id, rest) = path.split_once('/').unwrap_or((path, ""));
    if id.is_empty() {
        return None;
    }
    Some((id.to_string(), format!("/{rest}{}", query.map(|q| format!("?{q}")).unwrap_or_default())))
}

/// The page's headers that mean something to the host; nothing else (the page token above all) is passed on.
const PASSED_ON: [&str; 4] = ["Content-Type", "Range", "If-Range", "Accept"];

/// Types a browser would run script from if shown as a page.
const SCRIPTABLE: [&str; 8] = ["text/html", "application/xhtml+xml", "image/svg+xml", "text/javascript", "application/javascript", "text/xml", "application/xml", "text/ecmascript"];

/// A host's answer, made harmless to show. The page only ever needs JSON, pictures, video and attachments from
/// another computer, and an answer is served on this computer's own page address: a host that had been taken over
/// must not be able to send a page whose script then drives this computer's local API. So anything that could run
/// script is served as plain text, an answer with no type as bytes, and every answer is sandboxed and never
/// sniffed into something else.
fn defused(mut answer: Answer) -> Answer {
    let given = answer.header("Content-Type").map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
    let shown = match given.as_deref() {
        None | Some("") => Some("application/octet-stream"),
        Some(t) if SCRIPTABLE.contains(&t) || t.ends_with("+xml") || t.contains("script") => Some("text/plain; charset=utf-8"),
        Some(_) => None,
    };
    if let Some(shown) = shown {
        answer.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Type"));
        answer.headers.push(("Content-Type".into(), shown.into()));
    }
    answer.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Content-Security-Policy") && !k.eq_ignore_ascii_case("X-Content-Type-Options"));
    answer.headers.push(("Content-Security-Policy".into(), "sandbox".into()));
    answer.headers.push(("X-Content-Type-Options".into(), "nosniff".into()));
    answer
}

/// Pass one call to a saved computer: `target` is its path and the page's query (the page token is taken out here).
pub fn forward(book: &Book, id: &str, method: &str, target: &str, headers: &[(String, String)], body: &[u8]) -> Answer {
    let Some((addr, port, fp, token)) = book.route(id) else { return Answer::json(404, json!({"error": "unknown computer"})) };
    let Some(token) = token else { return Answer::json(401, json!({"error": "not paired"})) };
    let target = match target.split_once('?') {
        Some((p, q)) if !without_token(q).is_empty() => format!("{p}?{}", without_token(q)),
        Some((p, _)) => p.to_string(),
        None => target.to_string(),
    };
    let mut heads: Vec<(String, String)> = headers.iter().filter(|(k, _)| PASSED_ON.iter().any(|p| p.eq_ignore_ascii_case(k))).cloned().collect();
    heads.push(("Authorization".into(), format!("Bearer {token}")));
    match request(&addr, port, Some(&fp), method, &target, &heads, body, READ_WAIT, false) {
        Ok((answer, _)) => {
            book.seen(id, true);
            if answer.status == 401 {
                // the host revoked this computer (or forgot it): pairing again is the only way back
                book.unpaired(id);
            }
            defused(answer)
        }
        Err(why) => {
            book.seen(id, false);
            Answer::json(502, json!({"error": if why == Fail::Changed { "identity changed" } else { "offline" }}))
        }
    }
}

/// Ask every saved computer whether it is there (its hello, pinned to its certificate).
pub fn check_all(book: &Book) {
    check_all_within(book, QUICK_WAIT)
}

/// Each computer is asked on its own thread, within `wait`, so one slow computer cannot hold up the others or the
/// next round.
pub(crate) fn check_all_within(book: &Book, wait: Duration) {
    thread::scope(|s| {
        for id in book.ids() {
            s.spawn(move || {
                let Some((addr, port, fp, _)) = book.route(&id) else { return };
                let there = matches!(call_json(&addr, port, Some(&fp), "GET", "/remote/hello", None, wait), Ok((200, v, _)) if v["id"] == id.as_str());
                book.seen(&id, there);
            });
        }
    });
}

// ---- finding ----

/// Send "who is there" from `bind` to each of `targets`, collect answers for `wait`, and list them. A saved
/// computer that answers from a new address with the same certificate has its address updated.
pub fn discover_on(book: &Book, me_id: &str, bind: &str, targets: &[SocketAddr], wait: Duration) -> Value {
    let Ok(sock) = UdpSocket::bind(bind) else { return json!({"found": []}) };
    let _ = sock.set_broadcast(true);
    for t in targets {
        let _ = sock.send_to(DISCOVER_ASK, t);
    }
    let saved: Vec<Value> = book.all();
    let mut found: Vec<Value> = Vec::new();
    let end = Instant::now() + wait;
    let mut buf = vec![0u8; 64 * 1024];
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        let _ = sock.set_read_timeout(Some(left));
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(got) => got,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
            Err(_) => continue,
        };
        let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let (id, fp) = (v["id"].as_str().unwrap_or(""), v["fp"].as_str().unwrap_or("").to_ascii_lowercase());
        let port = v["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
        if v["fridae"] != 1 || id.is_empty() || id == me_id || port == 0 || fp.is_empty() || found.iter().any(|f| f["id"] == id) {
            continue;
        }
        let addr = from.ip().to_string();
        let mine = saved.iter().find(|m| m["id"] == id);
        if mine.is_some_and(|m| m["fp"] == fp.as_str() && (m["addr"] != addr.as_str() || m["port"] != port)) {
            // the same certificate is the same computer; another one claiming its id is not believed
            book.change(|list| {
                if let Some(m) = list.iter_mut().find(|m| m.id == id) {
                    m.addr = addr.clone();
                    m.port = port;
                }
            });
        }
        found.push(json!({"id": id, "name": v["name"].as_str().unwrap_or("Other computer"), "addr": addr, "port": port, "fp": fp,
                          "paired": mine.is_some_and(|m| m["paired"] == true)}));
    }
    json!({"found": found})
}

/// Where "who is there" goes on the real network: everyone on every network this computer is on.
fn broadcast_targets() -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::from(([255, 255, 255, 255], DISCOVER_PORT))];
    for b in directed_broadcasts() {
        let a = SocketAddr::from((b, DISCOVER_PORT));
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// Each IPv4 network's own broadcast address (some routers drop 255.255.255.255 between Wi-Fi and cable).
#[cfg(windows)]
fn directed_broadcasts() -> Vec<[u8; 4]> {
    use windows_sys::Win32::{
        Foundation::ERROR_BUFFER_OVERFLOW,
        NetworkManagement::{
            IpHelper::{GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH},
            Ndis::IfOperStatusUp,
        },
        Networking::WinSock::AF_INET,
    };
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64> = Vec::new(); // u64 for the alignment the adapter records need
    // SAFETY: the buffer is as large as `size` says; the system fills a linked list inside it, which is only read
    // while the buffer lives.
    unsafe {
        let mut tries = 0;
        loop {
            buf.resize((size as usize).div_ceil(8), 0);
            let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            let r = GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), first, &mut size);
            if r == 0 {
                break;
            }
            tries += 1;
            if r != ERROR_BUFFER_OVERFLOW || tries > 3 {
                return vec![];
            }
        }
        let mut out = Vec::new();
        let mut a = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !a.is_null() {
            let ad = &*a;
            if ad.OperStatus == IfOperStatusUp && ad.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
                let mut u = ad.FirstUnicastAddress;
                while !u.is_null() {
                    let ua = &*u;
                    let sa = ua.Address.lpSockaddr.cast::<u8>();
                    // a SOCKADDR_IN: family (2 bytes), port (2), then the address (4)
                    if !sa.is_null() && ua.Address.iSockaddrLength >= 8 && *sa.cast::<u16>() == AF_INET {
                        let ip = u32::from_be_bytes([*sa.add(4), *sa.add(5), *sa.add(6), *sa.add(7)]);
                        let len = u32::from(ua.OnLinkPrefixLength);
                        if (1..31).contains(&len) {
                            out.push((ip | (u32::MAX >> len)).to_be_bytes());
                        }
                    }
                    u = ua.Next;
                }
            }
            a = ad.Next;
        }
        out
    }
}

#[cfg(not(windows))]
fn directed_broadcasts() -> Vec<[u8; 4]> {
    vec![]
}

// ---- the page's API and the server's forwarding ----

/// GET /api/remote/machines
pub fn api_machines() -> Value {
    let me = remote_common::identity();
    json!({"this": {"id": me["id"], "name": me["name"]}, "machines": book().all()})
}

/// POST /api/remote/discover | pair | forget
pub fn api_post(path: &str, body: &Value) -> Value {
    match path {
        "/api/remote/discover" => {
            if !cfg!(windows) {
                return json!({"found": [], "error": "Only on Windows for now."});
            }
            let me = remote_common::identity();
            discover_on(book(), me["id"].as_str().unwrap_or(""), "0.0.0.0:0", &broadcast_targets(), FIND_WAIT)
        }
        "/api/remote/pair" => pair_with(book(), &remote_common::identity(), body),
        "/api/remote/forget" => {
            book().forget(body["id"].as_str().unwrap_or(""));
            json!({"ok": true})
        }
        _ => json!({"error": "not found"}),
    }
}

/// Headers that belong to one connection, not to the answer: the local server sets its own.
const NOT_PASSED_BACK: [&str; 7] = ["Connection", "Keep-Alive", "Transfer-Encoding", "Content-Length", "Date", "Server", "Trailer"];

/// Any method on `/m/<machine id>/<rest>`: pass it on and stream the answer back. The caller has checked the page
/// token; `url` is the request's address with its query.
pub fn serve(mut req: tiny_http::Request, url: &str) {
    use tiny_http::{Header, Response, StatusCode};
    let answer = match split_m(url) {
        None => Answer::json(404, json!({"error": "unknown computer"})),
        Some((id, target)) => {
            let headers: Vec<(String, String)> = req.headers().iter().map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string())).collect();
            let mut body = Vec::new();
            let _ = req.as_reader().take(BODY_LIMIT).read_to_end(&mut body);
            let method = req.method().as_str().to_string();
            forward(book(), &id, &method, &target, &headers, &body)
        }
    };
    let headers = answer
        .headers
        .iter()
        .filter(|(k, _)| !NOT_PASSED_BACK.iter().any(|n| n.eq_ignore_ascii_case(k)))
        .filter_map(|(k, v)| Header::from_bytes(k.as_bytes(), v.as_bytes()).ok())
        .collect();
    let len = answer.len.and_then(|n| usize::try_from(n).ok());
    let _ = req.respond(Response::new(StatusCode(answer.status), headers, answer.body, len, None));
}

#[cfg(test)]
#[path = "remote_client_tests.rs"]
mod tests;
