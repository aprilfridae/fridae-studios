//! Remote control, the host's side. When the user turns on "Let my other computers connect" (Settings, Other
//! computers), Fridae opens a second, encrypted listener on the network, answers "who is there" on the local network,
//! pairs computers with a one-time code, and serves its own page's requests to the computers it paired.
//!
//! A paired computer's request is handed to the local server (`server.rs`) as a request from this app's own page, so
//! it gets the same answers under the same rules. Only remote control's own routes are kept from it: it can neither
//! pair more computers here nor reach on through this computer to the ones this computer paired with.

use std::{
    fs,
    io::{self, Read, Write},
    net::{IpAddr, Shutdown, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use crate::remote_common::{self as rc, DISCOVER_ASK, DISCOVER_PORT, LISTEN_PORT};

/// The guarded setting that keeps network access on across restarts.
const SETTING: &str = "remote_listen";
/// The name of the certificate's key in Windows' key store.
const KEY_NAME: &str = "Fridae remote control";
/// The largest request body passed on, as the local server reads.
const BODY_LIMIT: usize = 64_000_000;
/// Connections served at once, and from any one address.
const MOST_CONNECTIONS: usize = 64;
const MOST_PER_ADDRESS: usize = 16;
/// How long a connection has to deliver its whole request: the TLS handshake, the head and the body.
const READ_LIMIT: Duration = Duration::from_secs(10);
/// Once a paired device's token has checked, its body may take as long as it needs, as long as it keeps coming: this
/// long without a byte ends it.
const BODY_IDLE: Duration = Duration::from_secs(30);
/// The local server's answer may be a long wait for events (`/api/events?wait=20`), or a long git, GitHub or rebuild
/// step: this long without a byte ends it (as long as the client waits, `remote_client::CALL_WAIT`).
const ANSWER_IDLE: Duration = Duration::from_secs(600);
/// How long a pairing code works, in seconds.
const CODE_LIFE: u64 = 300;
/// Wrong proofs a code takes before it is cancelled.
const MOST_WRONG: u32 = 5;

const NO_CODE: &str = "No pairing code is waiting on that computer. Press Pair a computer there to get one.";
const RAN_OUT: &str = "That code has run out. Press Pair a computer on that computer for a new one.";
const WRONG: &str = "That code is not right. Check it and try again.";
const TOO_MANY: &str = "That code was entered wrong too many times, so it no longer works. Press Pair a computer on that computer for a new one.";
const BAD_ASK: &str = "Fridae on that computer could not read the request. Update Fridae on both computers.";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ------------------------------------------------------------------------------------------------- pairing

struct Code {
    text: String,
    expires: u64,
    wrong: u32,
}

struct Device {
    id: String,
    name: String,
    /// `token_hash` of the device's token: the token itself is never kept here
    hash: String,
    paired_at: u64,
    last_seen: u64,
    /// `last_seen` as last written to the file (it is written at most once a minute per device)
    saved_seen: u64,
}

/// The pairing code that is live (at most one) and the paired devices (`devices.json`).
pub struct Pairing {
    file: PathBuf,
    code: Option<Code>,
    devices: Vec<Device>,
}

impl Pairing {
    pub fn load(dir: &Path) -> Pairing {
        let file = dir.join("devices.json");
        let v: Value = fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let devices = v["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let text = |k: &str| d[k].as_str().map(str::to_string);
                let seen = d["last_seen"].as_u64().unwrap_or(0);
                Some(Device { id: text("id")?, name: text("name").unwrap_or_default(), hash: text("hash").filter(|h| h.len() == 64)?, paired_at: d["paired_at"].as_u64().unwrap_or(0), last_seen: seen, saved_seen: seen })
            })
            .collect();
        Pairing { file, code: None, devices }
    }

    /// Written whole to a temporary file and renamed over the old one, so a reader never sees half a list.
    fn save(&mut self) {
        let list: Vec<Value> = self
            .devices
            .iter_mut()
            .map(|d| {
                d.saved_seen = d.last_seen;
                json!({"id": d.id, "name": d.name, "hash": d.hash, "paired_at": d.paired_at, "last_seen": d.last_seen})
            })
            .collect();
        let tmp = self.file.with_extension("json.tmp");
        if fs::write(&tmp, json!({"devices": list}).to_string()).is_ok() {
            let _ = fs::rename(&tmp, &self.file);
        }
    }

    /// A new code, replacing any code that was live.
    pub fn new_code(&mut self, now: u64) -> String {
        let text = rc::new_code();
        self.code = Some(Code { text: text.clone(), expires: now + CODE_LIFE, wrong: 0 });
        text
    }

    pub fn cancel_code(&mut self) {
        self.code = None;
    }

    fn live_code(&mut self, now: u64) -> Option<&Code> {
        if self.code.as_ref().is_some_and(|c| c.expires <= now) {
            self.code = None;
        }
        self.code.as_ref()
    }

    /// Pair a device with the live code: its proof must be the one made from the code, this host's fingerprint and
    /// its id. A right proof uses the code up and gives the device its token, with this host's confirmation that it
    /// knows the code too (`Proof::confirm`); the fifth wrong one cancels the code. Pairing a device again replaces
    /// its old entry, and so its old token.
    pub fn pair(&mut self, device_id: &str, device_name: &str, proof: &str, fp: &str, now: u64) -> Result<(String, String), String> {
        let id_ok = (1..=64).contains(&device_id.len()) && device_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !id_ok || proof.len() != 64 {
            return Err(BAD_ASK.into());
        }
        let Some(code) = self.code.as_mut() else { return Err(NO_CODE.into()) };
        if code.expires <= now {
            self.code = None;
            return Err(RAN_OUT.into());
        }
        let want = rc::proof(&code.text, fp, device_id);
        if !rc::same(&want.client, &proof.to_ascii_lowercase()) {
            code.wrong += 1;
            if code.wrong >= MOST_WRONG {
                self.code = None;
                return Err(TOO_MANY.into());
            }
            return Err(WRONG.into());
        }
        self.code = None;
        let token = rc::random_hex(32);
        // a name as the page shows it: no control characters, at most 80 letters
        let name: String = device_name.chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string();
        self.devices.retain(|d| d.id != device_id);
        self.devices.push(Device { id: device_id.into(), name: if name.is_empty() { "A computer".into() } else { name }, hash: rc::token_hash(&token), paired_at: now, last_seen: now, saved_seen: now });
        self.save();
        let confirm = want.confirm(&token);
        Ok((token, confirm))
    }

    /// The paired device a token belongs to, noting that it was used now.
    pub fn device_of(&mut self, token: &str, now: u64) -> Option<String> {
        let hash = rc::token_hash(token);
        let d = self.devices.iter_mut().find(|d| rc::same(&d.hash, &hash))?;
        d.last_seen = now;
        let (id, stale) = (d.id.clone(), now.saturating_sub(d.saved_seen) >= 60);
        if stale {
            self.save();
        }
        Some(id)
    }

    pub fn revoke(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        let gone = self.devices.len() != before;
        if gone {
            self.save();
        }
        gone
    }

    fn devices_json(&self) -> Value {
        self.devices.iter().map(|d| json!({"id": d.id, "name": d.name, "paired_at": d.paired_at, "last_seen": d.last_seen})).collect()
    }

    fn code_json(&mut self, now: u64) -> Value {
        match self.live_code(now) {
            Some(c) => json!({"text": rc::show_code(&c.text), "expires": c.expires}),
            None => Value::Null,
        }
    }
}


// ------------------------------------------------------------------------------------------------- certificate

/// The host's certificate, made once and kept. Its private key is a key Windows keeps for this user (CNG, the Microsoft
/// Software Key Storage Provider, under `KEY_NAME`), made so it can never be exported: no file of Fridae's holds it.
/// Only the certificate itself, which is public, is written to `host-cert.der`.
#[cfg(windows)]
pub mod cert {
    use std::{fs, io, path::Path, ptr};

    use schannel::cert_context::CertContext;
    use sha2::{Digest, Sha256};
    use windows_sys::Win32::{Foundation::SYSTEMTIME, Security::Cryptography as wc};

    use crate::remote_common as rc;

    const PROVIDER: &str = "Microsoft Software Key Storage Provider";

    pub struct Cert {
        pub ctx: CertContext,
        /// lowercase hex SHA-256 of the certificate's DER bytes
        pub fp: String,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn check(status: i32, what: &str) -> io::Result<()> {
        if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{what} failed ({status:#x})"))) }
    }

    /// A CNG handle, freed when dropped.
    struct Handle(usize);

    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe { wc::NCryptFreeObject(self.0) };
            }
        }
    }

    fn provider() -> io::Result<Handle> {
        let mut h = 0;
        check(unsafe { wc::NCryptOpenStorageProvider(&mut h, wc::MS_KEY_STORAGE_PROVIDER, 0) }, "opening the key store")?;
        Ok(Handle(h))
    }

    /// The key of that name: None only when Windows says there is no such key; any other failure is an error, so a
    /// key that could not be opened for a moment is never taken for a missing one (and replaced).
    fn open_key(prov: &Handle, name: &str) -> io::Result<Option<Handle>> {
        use windows_sys::Win32::Foundation::{NTE_BAD_KEYSET, NTE_NO_KEY};
        let mut h = 0;
        let name = wide(name);
        match unsafe { wc::NCryptOpenKey(prov.0, &mut h, name.as_ptr(), 0, wc::NCRYPT_SILENT_FLAG) } {
            0 => Ok(Some(Handle(h))),
            NTE_BAD_KEYSET | NTE_NO_KEY => Ok(None),
            status => Err(io::Error::other(format!("opening the key failed ({status:#x})"))),
        }
    }

    /// A new RSA 2048 key under `name` (replacing one of that name). Its export policy is none: it can be used, never
    /// read out.
    fn make_key(prov: &Handle, name: &str) -> io::Result<Handle> {
        let mut h = 0;
        let name = wide(name);
        check(unsafe { wc::NCryptCreatePersistedKey(prov.0, &mut h, wc::BCRYPT_RSA_ALGORITHM, name.as_ptr(), 0, wc::NCRYPT_OVERWRITE_KEY_FLAG) }, "making the key")?;
        let key = Handle(h);
        let set = |prop: windows_sys::core::PCWSTR, v: u32| unsafe { wc::NCryptSetProperty(key.0, prop, v.to_le_bytes().as_ptr(), 4, 0) };
        check(set(wc::NCRYPT_LENGTH_PROPERTY, 2048), "setting the key's length")?;
        check(set(wc::NCRYPT_EXPORT_POLICY_PROPERTY, 0), "setting the key's export policy")?;
        check(unsafe { wc::NCryptFinalizeKey(key.0, wc::NCRYPT_SILENT_FLAG) }, "finishing the key")?;
        Ok(key)
    }

    /// A self-signed certificate for `key` (CN=Fridae, SHA-256), its DER bytes. It is pinned by its fingerprint, never
    /// checked against a name or an authority, so it is made to last: until the end of 2049.
    fn self_signed(key: &Handle) -> io::Result<Vec<u8>> {
        let x500 = wide("CN=Fridae");
        let mut len = 0u32;
        let ok = unsafe { wc::CertStrToNameW(wc::X509_ASN_ENCODING, x500.as_ptr(), wc::CERT_X500_NAME_STR, ptr::null(), ptr::null_mut(), &mut len, ptr::null_mut()) };
        let mut name = vec![0u8; len as usize];
        if ok == 0 || unsafe { wc::CertStrToNameW(wc::X509_ASN_ENCODING, x500.as_ptr(), wc::CERT_X500_NAME_STR, ptr::null(), name.as_mut_ptr(), &mut len, ptr::null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let subject = wc::CRYPT_INTEGER_BLOB { cbData: len, pbData: name.as_mut_ptr() };
        let alg = wc::CRYPT_ALGORITHM_IDENTIFIER { pszObjId: wc::szOID_RSA_SHA256RSA as *mut u8, Parameters: Default::default() };
        let day = |y: u16, m: u16, d: u16| SYSTEMTIME { wYear: y, wMonth: m, wDay: d, ..Default::default() };
        let (from, to) = (day(2025, 1, 1), day(2049, 12, 31));
        let cert = unsafe { wc::CertCreateSelfSignCertificate(key.0, &subject, 0, ptr::null(), &alg, &from, &to, ptr::null()) };
        if cert.is_null() {
            return Err(io::Error::last_os_error());
        }
        let der = unsafe { std::slice::from_raw_parts((*cert).pbCertEncoded, (*cert).cbCertEncoded as usize).to_vec() };
        unsafe { wc::CertFreeCertificateContext(cert) };
        Ok(der)
    }

    /// The certificate with its key: Windows is told where the key is, and the key must match the certificate.
    fn with_key(der: &[u8], key_name: &str) -> io::Result<Cert> {
        let ctx = CertContext::new(der)?;
        ctx.set_key_prov_info().container(key_name).provider(PROVIDER).silent(true).set()?;
        ctx.private_key().silent(true).compare_key(true).acquire()?;
        Ok(Cert { fp: rc::hex(&Sha256::digest(der)), ctx })
    }

    /// The kept certificate. A new key is made only when Windows has no key of that name (the first time, or after a
    /// reinstall), and a new certificate only when there is no certificate file. Anything else that goes wrong while
    /// loading them is an error and nothing is replaced: a new key would make every paired computer see "identity
    /// changed".
    pub fn load_or_make(dir: &Path, key_name: &str) -> Result<Cert, String> {
        let file = dir.join("host-cert.der");
        let couldnt = |e: io::Error| format!("Fridae couldn't read its certificate for other computers, so it stayed off. Turn this off and on again to try again ({e}).");
        let prov = provider().map_err(couldnt)?;
        let Some(key) = open_key(&prov, key_name).map_err(couldnt)? else {
            let make = || -> io::Result<Cert> {
                let der = self_signed(&make_key(&prov, key_name)?)?;
                fs::write(&file, &der)?;
                with_key(&der, key_name)
            };
            return make().map_err(|e| format!("Fridae could not make its certificate for other computers ({e})."));
        };
        match fs::read(&file) {
            Ok(der) => with_key(&der, key_name).map_err(couldnt),
            // the key is there but its certificate file is gone: a new certificate for the same key
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let make = || -> io::Result<Cert> {
                    let der = self_signed(&key)?;
                    fs::write(&file, &der)?;
                    with_key(&der, key_name)
                };
                make().map_err(|e| format!("Fridae could not make its certificate for other computers ({e})."))
            }
            Err(e) => Err(couldnt(e)),
        }
    }

    /// Delete the key (the tests' own keys).
    #[cfg(test)]
    pub fn forget_key(key_name: &str) -> io::Result<()> {
        let prov = provider()?;
        let mut key = open_key(&prov, key_name)?.ok_or_else(|| io::Error::other("no such key"))?;
        check(unsafe { wc::NCryptDeleteKey(key.0, 0) }, "deleting the key")?;
        key.0 = 0; // deleting frees the handle
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------- serving

/// What the listener serves with: this host's pairing, its certificate's fingerprint and identity, and where the
/// local server is (its port and the page token: `server::local`).
pub struct Ctx {
    pub pairing: Arc<Mutex<Pairing>>,
    pub fp: String,
    pub id: String,
    pub name: String,
    pub backend: Box<dyn Fn() -> Option<(u16, String)> + Send + Sync>,
    /// `READ_LIMIT` (shorter in tests)
    pub read_limit: Duration,
}

/// A request's first line and headers.
struct Head {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn get(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

/// A connection read a line or a number of bytes at a time.
struct Conn<'a, S> {
    s: &'a mut S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Conn<'_, S> {
    fn more(&mut self) -> bool {
        let mut chunk = [0u8; 16 * 1024];
        match self.s.read(&mut chunk) {
            Ok(n) if n > 0 => {
                self.buf.extend_from_slice(&chunk[..n]);
                true
            }
            _ => false,
        }
    }

    fn line(&mut self) -> Option<String> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8(self.buf[..i].to_vec()).ok();
                self.buf.drain(..i + 2);
                return line;
            }
            if self.buf.len() > 8 * 1024 || !self.more() {
                return None;
            }
        }
    }

    fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        while self.buf.len() < n {
            if !self.more() {
                return None;
            }
        }
        Some(self.buf.drain(..n).collect())
    }

    fn head(&mut self) -> Option<Head> {
        let first = self.line()?;
        let mut parts = first.split(' ');
        let (method, target, version) = (parts.next()?.to_string(), parts.next()?.to_string(), parts.next()?);
        let method_ok = (1..=16).contains(&method.len()) && method.bytes().all(|b| b.is_ascii_uppercase());
        if !method_ok || !target.starts_with('/') || target.bytes().any(|b| b <= b' ' || b == 0x7f) || !version.starts_with("HTTP/1.") || parts.next().is_some() {
            return None;
        }
        let mut headers = Vec::new();
        loop {
            let line = self.line()?;
            if line.is_empty() {
                return Some(Head { method, target, headers });
            }
            let (k, v) = line.split_once(':')?;
            // a value is passed on to the local server, so it carries no line break or other control character
            let value_ok = !v.bytes().any(|b| (b < b' ' && b != b'\t') || b == 0x7f);
            if headers.len() >= 100 || k.is_empty() || !value_ok || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_!#$%&'*+.^`|~".contains(&b)) {
                return None;
            }
            headers.push((k.to_string(), v.trim().to_string()));
        }
    }

    /// The body, at most `limit` bytes, sent with a length or in chunks. Err: the status to answer.
    fn body(&mut self, head: &Head, limit: usize) -> Result<Vec<u8>, u16> {
        if head.get("Transfer-Encoding").to_ascii_lowercase().contains("chunked") {
            let mut out = Vec::new();
            loop {
                let size = self.line().ok_or(400u16)?;
                let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).map_err(|_| 400u16)?;
                // (a size near the largest number would wrap an unchecked sum back under the limit)
                if out.len().checked_add(size).is_none_or(|total| total > limit) {
                    return Err(413);
                }
                if size == 0 {
                    // the trailer, up to its empty line
                    while !self.line().ok_or(400u16)?.is_empty() {}
                    return Ok(out);
                }
                out.extend(self.take(size).ok_or(400u16)?);
                self.take(2).filter(|crlf| crlf == b"\r\n").ok_or(400u16)?;
            }
        }
        let len = match head.get("Content-Length") {
            "" => 0,
            n => n.parse::<usize>().map_err(|_| 400u16)?,
        };
        if len > limit {
            return Err(413);
        }
        self.take(len).ok_or(400)
    }
}

fn answer<W: Write>(w: &mut W, code: u16, v: &Value) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        413 => "Payload Too Large",
        _ => "Bad Gateway",
    };
    let body = v.to_string();
    let _ = write!(w, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = w.flush();
}

/// Headers never passed to the local server: who the request is for and how the connection is kept are its own, and
/// the device's token stays here.
const NOT_PASSED: &[&str] = &["host", "x-token", "authorization", "connection", "keep-alive", "transfer-encoding", "content-length", "te", "upgrade", "proxy-authorization", "proxy-connection"];

/// Hand a paired device's request to the local server as its own page's request (its address, the page token). The
/// answer is then read from the connection returned.
fn forward(ctx: &Ctx, head: &Head, body: &[u8]) -> io::Result<TcpStream> {
    let (port, token) = (ctx.backend)().ok_or_else(|| io::Error::other("the local server is not up"))?;
    let mut local = TcpStream::connect(("127.0.0.1", port))?;
    let mut ask = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Token: {token}\r\nConnection: close\r\nContent-Length: {}\r\n", head.method, head.target, body.len());
    for (k, v) in head.headers.iter().filter(|(k, _)| !NOT_PASSED.contains(&k.to_ascii_lowercase().as_str())) {
        ask.push_str(&format!("{k}: {v}\r\n"));
    }
    ask.push_str("\r\n");
    local.write_all(ask.as_bytes())?;
    local.write_all(body)?;
    Ok(local)
}

/// Whether a path is remote control's own, which a paired device may not use: managing this host's network access
/// (`/api/remote/...`) and reaching the computers this one paired with (`/m/...`).
fn kept_from_devices(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("/api/remote") || p.starts_with("/remote/") || p == "/m" || p.starts_with("/m/")
}

/// One request on a connection (which is then closed): the hello and pairing need no token; everything else needs a
/// paired device's token and is passed to the local server.
fn serve<S: Read + Write>(ctx: &Ctx, slot: &Slot, s: &mut S) {
    let mut c = Conn { s, buf: Vec::new() };
    let Some(head) = c.head() else { return answer(c.s, 400, &json!({"error": "bad request"})) };
    let path = head.target.split('?').next().unwrap_or("").to_string();
    match (head.method.as_str(), path.as_str()) {
        ("GET", "/remote/hello") => {
            slot.read();
            let pairing = ctx.pairing.lock().unwrap_or_else(|e| e.into_inner()).live_code(now()).is_some();
            answer(c.s, 200, &json!({"fridae": 1, "id": ctx.id, "name": ctx.name, "version": crate::paths::VERSION, "pairing": pairing}));
        }
        ("POST", "/remote/pair") => {
            let v: Value = c.body(&head, 16 * 1024).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
            slot.read();
            if !slot.live() {
                return;
            }
            let text = |k: &str| v[k].as_str().unwrap_or("").to_string();
            let paired = ctx.pairing.lock().unwrap_or_else(|e| e.into_inner()).pair(&text("device_id"), &text("device_name"), &text("proof"), &ctx.fp, now());
            match paired {
                Ok((token, confirm)) => answer(c.s, 200, &json!({"ok": true, "token": token, "host_id": ctx.id, "confirm": confirm})),
                Err(e) => answer(c.s, 200, &json!({"ok": false, "error": e})),
            }
        }
        _ => {
            let token = head.get("Authorization").strip_prefix("Bearer ").map(str::trim).unwrap_or("");
            let device = (!token.is_empty()).then(|| ctx.pairing.lock().unwrap_or_else(|e| e.into_inner()).device_of(token, now())).flatten();
            let Some(device) = device else { return answer(c.s, 401, &json!({"error": "not paired"})) };
            slot.device(&device);
            // a paired computer's body (a large attachment over Wi-Fi) may take longer than the deadline: from here
            // it only has to keep coming (`BODY_IDLE`)
            slot.read();
            if kept_from_devices(&path) {
                return answer(c.s, 403, &json!({"error": "not allowed from another computer"}));
            }
            let body = c.body(&head, BODY_LIMIT);
            // turned off (or this computer removed) while the request was arriving: nothing is passed on
            if !slot.live() {
                return;
            }
            match body {
                Ok(body) => match forward(ctx, &head, &body) {
                    // the answer copied back as it comes: status, headers and body unchanged
                    Ok(local) => {
                        // long enough for the page's long wait for events (`/api/events?wait=20`)
                        let mut local = slot.watch(local, ANSWER_IDLE);
                        let _ = io::copy(&mut local, c.s);
                        let _ = c.s.flush();
                    }
                    Err(_) => answer(c.s, 502, &json!({"error": "Fridae on that computer did not answer."})),
                },
                Err(code) => answer(c.s, code, &json!({"error": "bad request"})),
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------- connections

/// One connection being served.
struct Entry {
    peer: IpAddr,
    /// its socket, shut to end it from outside (the other side sees it close at once)
    stream: Option<TcpStream>,
    /// until when its request may take to arrive (None once it has)
    until: Option<Instant>,
    device: Option<String>,
    /// ended from outside: its reads stop (`Watched`) and nothing more is done for it
    cut: Arc<AtomicBool>,
}

impl Entry {
    fn shut(&mut self) {
        self.cut.store(true, Ordering::SeqCst);
        if let Some(s) = &self.stream {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

/// The connections being served, so that one machine cannot take every place, a request too slow to arrive is cut
/// off, and turning network access off (or removing a computer) ends what is open at once.
pub struct Conns {
    list: Mutex<std::collections::HashMap<u64, Entry>>,
    next: AtomicU64,
    closed: AtomicBool,
    read_limit: Duration,
}

impl Conns {
    pub fn new(read_limit: Duration) -> Arc<Conns> {
        Arc::new(Conns { list: Mutex::new(Default::default()), next: AtomicU64::new(0), closed: AtomicBool::new(false), read_limit })
    }

    fn list(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, Entry>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A place for a new connection from `peer`, or None when there is no room for it: a few at once is all a
    /// person's computers need, so more is turned away rather than run the host out of threads.
    fn admit(self: &Arc<Self>, peer: IpAddr, stream: Option<TcpStream>) -> Option<Slot> {
        let mut list = self.list();
        if self.closed.load(Ordering::SeqCst) || list.len() >= MOST_CONNECTIONS || list.values().filter(|e| e.peer == peer).count() >= MOST_PER_ADDRESS {
            return None;
        }
        let (id, cut) = (self.next.fetch_add(1, Ordering::SeqCst), Arc::new(AtomicBool::new(false)));
        list.insert(id, Entry { peer, stream, until: Some(Instant::now() + self.read_limit), device: None, cut: cut.clone() });
        Some(Slot { conns: self.clone(), id, cut })
    }

    /// Cut off the connections whose request has not arrived in time.
    fn expire(&self, now: Instant) {
        self.list().values_mut().filter(|e| e.until.is_some_and(|u| u <= now)).for_each(Entry::shut);
    }

    /// Network access is off: end every connection, and admit none.
    fn shut_all(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.list().values_mut().for_each(Entry::shut);
    }

    /// A computer was removed: end what it has open.
    fn shut_device(&self, device: &str) {
        self.list().values_mut().filter(|e| e.device.as_deref() == Some(device)).for_each(Entry::shut);
    }
}

/// One connection's place, given back when it is dropped (also when its thread panics).
pub struct Slot {
    conns: Arc<Conns>,
    id: u64,
    cut: Arc<AtomicBool>,
}

impl Slot {
    fn with(&self, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.conns.list().get_mut(&self.id) {
            f(e);
        }
    }

    /// The whole request has arrived: no deadline from now on.
    fn read(&self) {
        self.with(|e| e.until = None);
    }

    /// The paired device the request came from (removing it ends this connection).
    fn device(&self, id: &str) {
        self.with(|e| e.device = Some(id.to_string()));
    }

    /// Network access is still on, and nothing has ended this connection.
    fn live(&self) -> bool {
        !self.conns.closed.load(Ordering::SeqCst) && !self.cut.load(Ordering::SeqCst)
    }

    /// A socket of this connection, read in short waits so that it stops as soon as the connection is ended, or
    /// once `idle` has passed without a byte.
    fn watch(&self, s: TcpStream, idle: Duration) -> Watched {
        let _ = s.set_read_timeout(None);
        Watched { s, cut: self.cut.clone(), idle, last: Instant::now() }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.conns.list().remove(&self.id);
    }
}

/// A socket read in short waits. Windows does not wake a read that is waiting when another thread shuts the socket,
/// so a connection ended from outside could otherwise hold its thread and its place until data came. The wait is for
/// the socket to be readable (`readable`), never a receive timeout: Windows leaves a socket whose receive timed out in
/// an unknown state, and data could be lost.
struct Watched {
    s: TcpStream,
    cut: Arc<AtomicBool>,
    idle: Duration,
    /// when a byte last came
    last: Instant,
}

impl Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.cut.load(Ordering::SeqCst) {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
            if self.last.elapsed() >= self.idle {
                return Err(io::ErrorKind::TimedOut.into());
            }
            if readable(&self.s, 200)? {
                let done = self.s.read(buf);
                self.last = Instant::now();
                return done;
            }
        }
    }
}

/// Whether the socket can be read now (data, its end, or an error to report), waiting up to `ms` for it.
#[cfg(windows)]
fn readable(s: &TcpStream, ms: i32) -> io::Result<bool> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{POLLERR, POLLHUP, POLLRDNORM, WSAPOLLFD, WSAPoll};
    let mut fd = WSAPOLLFD { fd: s.as_raw_socket() as usize, events: POLLRDNORM, revents: 0 };
    match unsafe { WSAPoll(&mut fd, 1, ms) } {
        n if n < 0 => Err(io::Error::last_os_error()),
        0 => Ok(false),
        _ => Ok(fd.revents & (POLLRDNORM | POLLHUP | POLLERR) != 0),
    }
}

/// (Elsewhere the network side is not used: a read simply waits.)
#[cfg(not(windows))]
fn readable(_: &TcpStream, _: i32) -> io::Result<bool> {
    Ok(true)
}

impl Write for Watched {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.s.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.s.flush()
    }
}

// ------------------------------------------------------------------------------------------------- listener

/// The network listener (TLS) and the discovery responder (UDP), while network access is on.
pub struct Listener {
    pub port: u16,
    /// the UDP port "who is there" is answered on (the tests listen on one the system picks)
    #[cfg_attr(not(test), allow(dead_code))]
    pub discover_port: Option<u16>,
    stop: Arc<AtomicBool>,
    conns: Arc<Conns>,
    wake: (String, u16),
    threads: Vec<thread::JoinHandle<()>>,
}

impl Listener {
    /// Stop listening and answering: when this returns, neither port is open and every connection is shut, a long
    /// wait for events included.
    pub fn close(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.conns.shut_all();
        // a connection of our own wakes the accept that waits
        if let Ok(addr) = format!("{}:{}", self.wake.0, self.wake.1).parse() {
            let _ = TcpStream::connect_timeout(&addr, Duration::from_secs(1));
        }
        for t in self.threads {
            let _ = t.join();
        }
    }
}

/// The first port of `ports` that opens on `ip`. A port taken by another program, or one Windows keeps back (it
/// answers "access denied" for those), is passed over.
#[cfg(windows)]
fn bind_first(ip: &str, ports: std::ops::RangeInclusive<u16>) -> Result<TcpListener, String> {
    let (first, last) = (*ports.start(), *ports.end());
    let mut denied = false;
    for port in ports {
        match TcpListener::bind((ip, port)) {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }
    Err(if denied {
        format!("Windows blocked the ports Fridae needs ({first} to {last}). Check that no firewall or security program is stopping Fridae, then turn this on again.")
    } else {
        format!("Other programs are using the ports Fridae needs ({first} to {last}). Close them, then turn this on again.")
    })
}

/// Listen on `ip` (the first free port of `ports`) and answer "who is there" on UDP `discover`. The second value is a
/// warning when only finding failed: pairing by address still works then.
#[cfg(windows)]
pub fn listen(ctx: Arc<Ctx>, cert: &cert::Cert, ip: &str, ports: std::ops::RangeInclusive<u16>, discover: Option<u16>) -> Result<(Listener, Option<String>), String> {
    use schannel::{schannel_cred::{Direction, SchannelCred}, tls_stream};
    let cred = SchannelCred::builder().cert(cert.ctx.clone()).acquire(Direction::Inbound).map_err(|e| format!("Windows would not use Fridae's certificate ({e})."))?;
    let tcp = bind_first(ip, ports)?;
    let port = tcp.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    let (mut discover_port, mut warning) = (None, None);
    if let Some(dp) = discover {
        match UdpSocket::bind((ip, dp)) {
            Ok(udp) => {
                discover_port = udp.local_addr().ok().map(|a| a.port());
                let reply = json!({"fridae": 1, "id": ctx.id, "name": ctx.name, "port": port, "fp": ctx.fp}).to_string();
                let stop = stop.clone();
                threads.push(thread::spawn(move || discovery(udp, reply, stop)));
            }
            Err(_) => warning = Some("Your other computers cannot find this one by themselves (another program has the port for that). Pair them by its address instead.".to_string()),
        }
    }
    let conns = Conns::new(ctx.read_limit);
    // requests too slow to arrive are cut off, a few times a second
    let (watched, stopped) = (conns.clone(), stop.clone());
    threads.push(thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(200));
            watched.expire(Instant::now());
        }
    }));
    let (stopped, admitted) = (stop.clone(), conns.clone());
    threads.push(thread::spawn(move || {
        for conn in tcp.incoming() {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let (Ok(peer), Ok(handle)) = (conn.peer_addr(), conn.try_clone()) else { continue };
            // no room: the connection is closed as it is dropped
            let Some(slot) = admitted.admit(peer.ip(), Some(handle)) else { continue };
            let (ctx, cred) = (ctx.clone(), cred.clone());
            thread::spawn(move || {
                // (its reads end at the deadline for the request, or when the connection is ended from outside)
                let _ = conn.set_write_timeout(Some(Duration::from_secs(60)));
                if let Ok(mut tls) = tls_stream::Builder::new().accept(cred, slot.watch(conn, BODY_IDLE)) {
                    serve(&ctx, &slot, &mut tls);
                    let _ = tls.shutdown();
                }
                drop(slot);
            });
        }
    }));
    let wake_ip = if ip == "0.0.0.0" { "127.0.0.1" } else { ip };
    Ok((Listener { port, discover_port, stop, conns, wake: (wake_ip.to_string(), port), threads }, warning))
}

/// Answer each "who is there" with this host's line, until stopped (checked twice a second).
fn discovery(udp: UdpSocket, reply: String, stop: Arc<AtomicBool>) {
    let _ = udp.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::SeqCst) {
        if let Ok((n, from)) = udp.recv_from(&mut buf) {
            if &buf[..n] == DISCOVER_ASK {
                let _ = udp.send_to(reply.as_bytes(), from);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------- the page's switch

struct State {
    on: Option<Listener>,
    fp: Option<String>,
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State { on: None, fp: None, error: None });
static PAIRING: LazyLock<Arc<Mutex<Pairing>>> = LazyLock::new(|| Arc::new(Mutex::new(Pairing::load(&rc::dir()))));

fn setting_on() -> bool {
    crate::paths::setting(SETTING).as_bool() == Some(true)
}

/// At launch: listen when the user left network access on.
pub fn start() {
    if setting_on() {
        turn_on(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

fn turn_on(st: &mut State) {
    if st.on.is_some() {
        return;
    }
    st.error = None;
    #[cfg(windows)]
    {
        let cert = match cert::load_or_make(&rc::dir(), KEY_NAME) {
            Ok(c) => c,
            Err(e) => return st.error = Some(e),
        };
        let me = rc::identity();
        let text = |k: &str| me[k].as_str().unwrap_or("").to_string();
        let ctx = Arc::new(Ctx { pairing: PAIRING.clone(), fp: cert.fp.clone(), id: text("id"), name: text("name"), backend: Box::new(crate::server::local), read_limit: READ_LIMIT });
        match listen(ctx, &cert, "0.0.0.0", LISTEN_PORT..=LISTEN_PORT + 9, Some(DISCOVER_PORT)) {
            Ok((l, warning)) => {
                st.on = Some(l);
                st.fp = Some(cert.fp);
                st.error = warning;
            }
            Err(e) => st.error = Some(e),
        }
    }
    #[cfg(not(windows))]
    {
        st.error = Some("Only on Windows for now.".into());
    }
}

fn turn_off(st: &mut State) {
    if let Some(l) = st.on.take() {
        l.close();
    }
    st.error = None;
    PAIRING.lock().unwrap_or_else(|e| e.into_inner()).cancel_code();
}

fn state_json(st: &State) -> Value {
    let me = rc::identity();
    let mut p = PAIRING.lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "on": setting_on(), "port": st.on.as_ref().map(|l| l.port), "id": me["id"], "name": me["name"], "fp": st.fp,
        "devices": p.devices_json(), "code": p.code_json(now()), "error": st.error,
    })
}

/// GET /api/remote/host (the page only: a paired device is never let through to it)
pub fn api_get() -> Value {
    state_json(&STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// POST /api/remote/host `{"action": "on" | "off" | "code" | "cancel-code" | "revoke", "device"}`, from the user's
/// presses in Settings only (the setting is guarded, so no AI turns it on through the settings file).
pub fn api_post(body: &Value) -> Value {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    match body["action"].as_str().unwrap_or("") {
        "on" => {
            crate::paths::update_config(|cfg| cfg.insert(SETTING.into(), json!(true)));
            turn_on(&mut st);
        }
        "off" => {
            crate::paths::update_config(|cfg| cfg.insert(SETTING.into(), json!(false)));
            turn_off(&mut st);
        }
        "code" if st.on.is_some() => {
            PAIRING.lock().unwrap_or_else(|e| e.into_inner()).new_code(now());
        }
        "cancel-code" => PAIRING.lock().unwrap_or_else(|e| e.into_inner()).cancel_code(),
        "revoke" => {
            let device = body["device"].as_str().unwrap_or("");
            PAIRING.lock().unwrap_or_else(|e| e.into_inner()).revoke(device);
            // what that computer has open now ends too
            if let Some(l) = &st.on {
                l.conns.shut_device(device);
            }
        }
        _ => {}
    }
    state_json(&st)
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

    const FP: &str = "ab12";

    #[test]
    fn a_code_pairs_once_and_only_the_tokens_hash_is_kept() {
        let dir = fresh("remote-pair-once");
        let mut p = Pairing::load(&dir);
        let code = p.new_code(1000);
        assert_eq!(code.len(), rc::CODE_LEN);
        let proof = rc::proof(&code, FP, "laptop").client;
        let token = p.pair("laptop", "Laptop", &proof, FP, 1010).expect("the right proof pairs").0;
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(p.pair("laptop", "Laptop", &proof, FP, 1011).unwrap_err(), NO_CODE, "a code works once");
        let file = fs::read_to_string(dir.join("devices.json")).unwrap();
        assert!(!file.contains(&token), "the token is never written");
        assert!(file.contains(&rc::token_hash(&token)));
        assert_eq!(p.device_of(&token, 1020).as_deref(), Some("laptop"));
        assert_eq!(p.device_of(&rc::random_hex(32), 1020), None);
        // kept across a restart
        let mut again = Pairing::load(&dir);
        assert_eq!(again.device_of(&token, 1030).as_deref(), Some("laptop"));
        let list = again.devices_json();
        assert_eq!(list[0]["name"], "Laptop");
        assert_eq!(list[0]["paired_at"], 1010);
        assert!(list[0].get("hash").is_none(), "the page is not handed the hash");
    }

    #[test]
    fn pairing_gives_the_device_the_hosts_confirmation_for_its_token() {
        let mut p = Pairing::load(&fresh("remote-pair-confirm"));
        let code = p.new_code(1000);
        let (token, confirm) = p.pair("laptop", "Laptop", &rc::proof(&code, FP, "laptop").client, FP, 1001).unwrap();
        assert_eq!(confirm, rc::proof(&code, FP, "laptop").confirm(&token), "made from the code: only a host that knows it can say it");
    }

    #[test]
    fn a_code_runs_out_after_five_minutes() {
        let mut p = Pairing::load(&fresh("remote-pair-expiry"));
        let code = p.new_code(1000);
        let proof = rc::proof(&code, FP, "laptop").client;
        assert_eq!(p.pair("laptop", "Laptop", &proof, FP, 1000 + CODE_LIFE).unwrap_err(), RAN_OUT);
        assert_eq!(p.pair("laptop", "Laptop", &proof, FP, 1000 + CODE_LIFE).unwrap_err(), NO_CODE);
    }

    #[test]
    fn the_fifth_wrong_proof_cancels_the_code() {
        let mut p = Pairing::load(&fresh("remote-pair-wrong"));
        let code = p.new_code(1000);
        for _ in 0..4 {
            assert_eq!(p.pair("laptop", "Laptop", &"0".repeat(64), FP, 1001).unwrap_err(), WRONG);
        }
        assert_eq!(p.pair("laptop", "Laptop", &"0".repeat(64), FP, 1001).unwrap_err(), TOO_MANY);
        assert_eq!(p.pair("laptop", "Laptop", &rc::proof(&code, FP, "laptop").client, FP, 1002).unwrap_err(), NO_CODE);
        // the count is the code's: a new device id for each try does not start it again
        let code = p.new_code(2000);
        for i in 0..4 {
            assert_eq!(p.pair(&format!("dev{i}"), "Laptop", &"0".repeat(64), FP, 2001).unwrap_err(), WRONG);
        }
        assert_eq!(p.pair("dev9", "Laptop", &"0".repeat(64), FP, 2001).unwrap_err(), TOO_MANY);
        assert_eq!(p.pair("dev10", "Laptop", &rc::proof(&code, FP, "dev10").client, FP, 2002).unwrap_err(), NO_CODE);
    }

    #[test]
    fn a_proof_made_with_another_fingerprint_or_device_is_wrong() {
        let mut p = Pairing::load(&fresh("remote-pair-mitm"));
        let code = p.new_code(1000);
        assert_eq!(p.pair("laptop", "Laptop", &rc::proof(&code, "ff00", "laptop").client, FP, 1001).unwrap_err(), WRONG);
        assert_eq!(p.pair("laptop", "Laptop", &rc::proof(&code, FP, "other").client, FP, 1001).unwrap_err(), WRONG);
        assert!(p.pair("", "Laptop", &rc::proof(&code, FP, "").client, FP, 1001).is_err(), "a device has an id");
    }

    #[test]
    fn revoking_a_device_ends_its_token_and_pairing_again_replaces_it() {
        let dir = fresh("remote-pair-revoke");
        let mut p = Pairing::load(&dir);
        let code = p.new_code(1000);
        let old = p.pair("laptop", "Laptop", &rc::proof(&code, FP, "laptop").client, FP, 1001).unwrap().0;
        let code = p.new_code(1100);
        let new = p.pair("laptop", "Laptop 2", &rc::proof(&code, FP, "laptop").client, FP, 1101).unwrap().0;
        assert_eq!(p.devices_json().as_array().unwrap().len(), 1, "the same device once");
        assert_eq!(p.device_of(&old, 1102), None, "its old token is gone");
        assert_eq!(p.device_of(&new, 1102).as_deref(), Some("laptop"));
        assert!(p.revoke("laptop"));
        assert_eq!(p.device_of(&new, 1103), None);
        assert_eq!(Pairing::load(&dir).devices_json(), json!([]), "revoked on disk too");
        assert!(!p.revoke("laptop"));
    }

    #[test]
    fn last_used_is_noted() {
        let mut p = Pairing::load(&fresh("remote-pair-seen"));
        let code = p.new_code(1000);
        let token = p.pair("laptop", "Laptop", &rc::proof(&code, FP, "laptop").client, FP, 1001).unwrap().0;
        p.device_of(&token, 5000);
        assert_eq!(p.devices_json()[0]["last_seen"], 5000);
    }

    /// A connection without TLS: what the device sends, and what it is sent back.
    struct Pipe {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// One request served over a pipe: the status, the body as JSON, and how much of what was sent was read.
    fn served_bytes(ctx: &Ctx, raw: &[u8]) -> (u16, Value, u64) {
        let conns = Conns::new(READ_LIMIT);
        let slot = conns.admit("127.0.0.1".parse().unwrap(), None).unwrap();
        let mut pipe = Pipe { input: std::io::Cursor::new(raw.to_vec()), output: Vec::new() };
        serve(ctx, &slot, &mut pipe);
        let out = String::from_utf8_lossy(&pipe.output).to_string();
        let status = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (status, out.split_once("\r\n\r\n").and_then(|(_, b)| serde_json::from_str(b).ok()).unwrap_or(Value::Null), pipe.input.position())
    }

    fn served(ctx: &Ctx, raw: &str) -> (u16, Value) {
        let (status, body, _) = served_bytes(ctx, raw.as_bytes());
        (status, body)
    }

    #[test]
    fn requests_are_read_with_a_length_or_in_chunks_and_within_limits() {
        let dir = fresh("remote-serve");
        let local = stand_in();
        let pairing = Arc::new(Mutex::new(Pairing::load(&dir)));
        let code = pairing.lock().unwrap().new_code(1000);
        let token = pairing.lock().unwrap().pair("laptop1", "Laptop", &rc::proof(&code, FP, "laptop1").client, FP, 1001).unwrap().0;
        let ctx = Ctx { pairing, fp: FP.into(), id: "host1".into(), name: "Desktop".into(), backend: Box::new(move || Some((local, "page-token".into()))), read_limit: READ_LIMIT };
        let (code, said) = served(&ctx, &format!("POST /api/send HTTP/1.1\r\nAuthorization: Bearer {token}\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\n\r\n"));
        assert_eq!((code, said["body"].clone(), said["token"].clone()), (207, json!("hello world"), json!("page-token")));
        // a body too large is refused before it is read; without a token, before anything else
        assert_eq!(served(&ctx, &format!("POST /api/send HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n", BODY_LIMIT + 1)).0, 413);
        assert_eq!(served(&ctx, &format!("POST /api/send HTTP/1.1\r\nContent-Length: {}\r\n\r\n", BODY_LIMIT + 1)).0, 401);
        assert_eq!(served(&ctx, "GET  /api/history HTTP/1.1\r\n\r\n").0, 400);
        assert_eq!(served(&ctx, "GET http://elsewhere/ HTTP/1.1\r\n\r\n").0, 400);
        assert_eq!(served(&ctx, &format!("GET /api/history HTTP/1.1\r\nAuthorization: Bearer {token}\r\nBad Header: x\r\n\r\n")).0, 400);
        // a value with a line break in it would be a second header on the way to the local server
        assert_eq!(served(&ctx, &format!("GET /api/history HTTP/1.1\r\nAuthorization: Bearer {token}\r\nX-A: a\nX-Token: b\r\n\r\n")).0, 400);
        // a chunk size near the largest number is refused at once, not added up past the limit and then read
        // (without a token on the pairing route, and with one on any other)
        let huge = |path: &str, auth: &str| [format!("POST {path} HTTP/1.1\r\n{auth}Transfer-Encoding: chunked\r\n\r\n1\r\nA\r\nffffffffffffffff\r\n").into_bytes(), vec![b'A'; 200_000]].concat();
        let (status, body, read) = served_bytes(&ctx, &huge("/remote/pair", ""));
        assert_eq!((status, body["error"].clone()), (200, json!(BAD_ASK)));
        assert!(read <= 16 * 1024, "only what had arrived with the head was read ({read} bytes)");
        let (status, _, read) = served_bytes(&ctx, &huge("/api/send", &format!("Authorization: Bearer {token}\r\n")));
        assert_eq!(status, 413);
        assert!(read <= 16 * 1024, "only what had arrived with the head was read ({read} bytes)");
        // the local server not up: said plainly
        let down = Ctx { backend: Box::new(|| None), ..ctx };
        assert_eq!(served(&down, &format!("GET /api/history HTTP/1.1\r\nAuthorization: Bearer {token}\r\n\r\n")).0, 502);
    }

    #[test]
    fn one_machine_cannot_take_every_place_and_a_place_is_given_back_even_after_a_panic() {
        let conns = Conns::new(READ_LIMIT);
        let (a, b): (IpAddr, IpAddr) = ("10.0.0.2".parse().unwrap(), "10.0.0.3".parse().unwrap());
        let held: Vec<Slot> = (0..MOST_PER_ADDRESS).map(|_| conns.admit(a, None).expect("room")).collect();
        assert!(conns.admit(a, None).is_none(), "no more from the same machine");
        assert!(conns.admit(b, None).is_some(), "another machine still gets in");
        // a connection's thread that panics gives its place back
        for slot in held {
            assert!(thread::spawn(move || {
                let _slot = slot;
                panic!("a request went wrong");
            })
            .join()
            .is_err());
        }
        let again: Vec<Slot> = (0..MOST_PER_ADDRESS).map(|_| conns.admit(a, None).expect("room again")).collect();
        assert_eq!(again.len(), MOST_PER_ADDRESS);
        // and there is a ceiling over all machines together
        let many: Vec<Slot> = (0..=255u8).filter_map(|i| conns.admit(IpAddr::from([10, 1, 0, i]), None)).collect();
        assert_eq!(many.len() + again.len(), MOST_CONNECTIONS);
    }

    #[test]
    fn a_client_at_its_busiest_fits_from_one_address() {
        // the client opens a new connection for every call, and the last one may still be closing
        let conns = Conns::new(READ_LIMIT);
        let a: IpAddr = "10.0.0.2".parse().unwrap();
        let held: Vec<Slot> = (0..16).filter_map(|_| conns.admit(a, None)).collect();
        assert_eq!(held.len(), 16);
        assert!(conns.admit(a, None).is_none(), "but there is still a limit");
    }

    #[test]
    fn the_page_is_told_where_network_access_stands() {
        let off = api_get();
        assert_eq!((off["on"].clone(), off["port"].clone(), off["code"].clone()), (json!(false), Value::Null, Value::Null));
        assert_eq!(off["id"].as_str().map(str::len), Some(32));
        assert!(off["devices"].is_array());
        // no code while nothing listens
        assert_eq!(api_post(&json!({"action": "code"}))["code"], Value::Null);
    }

    // ---- the certificate and the listener, on 127.0.0.1 only

    /// A test's own key, deleted from Windows' key store when the test ends, passed or failed.
    #[cfg(windows)]
    struct TestKey(String);

    #[cfg(windows)]
    impl Drop for TestKey {
        fn drop(&mut self) {
            let _ = cert::forget_key(&self.0);
        }
    }

    #[cfg(windows)]
    fn test_key() -> TestKey {
        TestKey(format!("Fridae remote control test {}", rc::random_hex(4)))
    }

    #[cfg(windows)]
    #[test]
    fn the_certificate_is_made_once_and_kept() {
        use sha2::Digest;
        let (dir, key) = (fresh("remote-cert"), test_key());
        let first = cert::load_or_make(&dir, &key.0).expect("a certificate is made");
        assert_eq!(first.fp.len(), 64);
        let again = cert::load_or_make(&dir, &key.0).expect("and loaded again");
        assert_eq!(again.fp, first.fp, "the same certificate after a restart");
        assert_eq!(rc::hex(&sha2::Sha256::digest(again.ctx.to_der())), first.fp, "the fingerprint is the SHA-256 of its DER bytes");
        assert!(dir.join("host-cert.der").is_file());
        // only the certificate is on disk: no private key in the folder
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        cert::forget_key(&key.0).unwrap();
        // the key gone (as after a Windows reinstall): a new certificate, not a broken one
        let new = cert::load_or_make(&dir, &key.0).expect("made again");
        assert_ne!(new.fp, first.fp);
    }

    #[cfg(windows)]
    #[test]
    fn a_certificate_that_cannot_be_read_for_a_moment_never_gets_a_new_key() {
        let (dir, key) = (fresh("remote-cert-transient"), test_key());
        let first = cert::load_or_make(&dir, &key.0).expect("a certificate is made");
        let file = dir.join("host-cert.der");
        let der = fs::read(&file).unwrap();
        // the file unreadable for a moment (spoilt, then locked away): it stays off and says why, the key is kept
        fs::write(&file, b"not a certificate").unwrap();
        let e = cert::load_or_make(&dir, &key.0).err().expect("a spoilt certificate is not replaced");
        assert!(e.contains("couldn't read"), "{e}");
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert!(cert::load_or_make(&dir, &key.0).is_err(), "an unreadable certificate is not replaced");
        fs::remove_dir(&file).unwrap();
        fs::write(&file, &der).unwrap();
        let again = cert::load_or_make(&dir, &key.0).expect("loads once it can be read");
        assert_eq!(again.fp, first.fp, "the same key and certificate: every paired computer still knows this one");
    }

    /// One request over its own TLS connection: the status and the body as JSON, and the certificate's fingerprint.
    #[cfg(windows)]
    fn ask(port: u16, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, Value, String) {
        use sha2::Digest;
        let mut s = tls_to(port);
        let fp = rc::hex(&sha2::Sha256::digest(s.peer_certificate().unwrap().unwrap().to_der().unwrap()));
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        write!(s, "{method} {path} HTTP/1.1\r\nHost: desktop:47530\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let raw = String::from_utf8_lossy(&raw).to_string();
        let status = raw.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        (status, serde_json::from_str(body).unwrap_or(Value::Null), fp)
    }

    #[cfg(windows)]
    fn tls_to(port: u16) -> native_tls::TlsStream<TcpStream> {
        let tls = native_tls::TlsConnector::builder().danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true).build().unwrap();
        tls.connect("fridae", TcpStream::connect(("127.0.0.1", port)).unwrap()).expect("TLS")
    }

    /// The local server, standing in: it answers as the page's server would, saying what it was handed. The page's
    /// long wait for events (`/api/events`) waits 20 seconds here.
    fn stand_in() -> u16 {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        thread::spawn(move || {
            for mut req in server.incoming_requests() {
                thread::spawn(move || {
                    let head = |n: &'static str| req.headers().iter().find(|h| h.field.equiv(n)).map(|h| h.value.as_str().to_string());
                    let mut said = json!({"url": req.url(), "method": req.method().as_str(), "token": head("X-Token"), "host": head("Host"), "auth": head("Authorization")});
                    let mut body = String::new();
                    let _ = req.as_reader().read_to_string(&mut body);
                    said["body"] = json!(body);
                    if req.url().starts_with("/api/events") {
                        thread::sleep(Duration::from_secs(20));
                    }
                    if req.url() == "/api/big" {
                        return drop(req.respond(tiny_http::Response::from_data(big())));
                    }
                    let _ = req.respond(tiny_http::Response::from_string(said.to_string()).with_status_code(207));
                });
            }
        });
        port
    }

    /// A host listening on 127.0.0.1 with a test key, and a device ("laptop1") paired with it: its token.
    #[cfg(windows)]
    fn paired_host(name: &str, read_limit: Duration) -> (Listener, String, Arc<Mutex<Pairing>>, TestKey) {
        let (dir, key) = (fresh(name), test_key());
        let cert = cert::load_or_make(&dir, &key.0).unwrap();
        let local = stand_in();
        let pairing = Arc::new(Mutex::new(Pairing::load(&dir)));
        let code = pairing.lock().unwrap().new_code(now());
        let token = pairing.lock().unwrap().pair("laptop1", "Laptop", &rc::proof(&code, &cert.fp, "laptop1").client, &cert.fp, now()).unwrap().0;
        let ctx = Arc::new(Ctx { pairing: pairing.clone(), fp: cert.fp.clone(), id: "host1".into(), name: "Desktop".into(), backend: Box::new(move || Some((local, "page-token".into()))), read_limit });
        let (l, _) = listen(ctx, &cert, "127.0.0.1", 0..=0, None).unwrap();
        (l, token, pairing, key)
    }

    /// A connection the host has closed: reading ends at once (no answer), it does not wait.
    #[cfg(windows)]
    fn closed(s: &mut TcpStream) -> bool {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut b = [0u8; 64];
        !matches!(s.read(&mut b), Ok(n) if n > 0) && !matches!(s.read(&mut b), Err(e) if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock)
    }

    #[cfg(windows)]
    #[test]
    fn slow_requests_are_cut_off_and_one_machine_cannot_crowd_out_the_rest() {
        // (a deadline that a TLS handshake meets even while the other tests keep the processor busy)
        let (l, _token, _pairing, _key) = paired_host("remote-slow", Duration::from_secs(3));
        let port = l.port;
        // a machine opens as many connections as it may and sends nothing
        let mut idle: Vec<TcpStream> = (0..MOST_PER_ADDRESS).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
        thread::sleep(Duration::from_millis(300));
        let mut one_more = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let shut_at = Instant::now();
        assert!(closed(&mut one_more), "a connection over the machine's share is closed");
        assert!(shut_at.elapsed() < Duration::from_millis(900), "at once, not at the deadline");
        // the idle ones are cut at the deadline, so they never hold their places for long
        for s in &mut idle {
            assert!(closed(s));
        }
        // (each place is given back when its thread sees the cut, within a fifth of a second)
        thread::sleep(Duration::from_millis(500));
        assert_eq!(ask(port, "GET", "/remote/hello", None, "").0, 200, "and the next one is served");
        // a request trickled in more slowly than the deadline allows is cut off before it is served
        let mut slow = tls_to(port);
        slow.write_all(b"GET /remote/hel").unwrap();
        thread::sleep(Duration::from_millis(3500));
        let _ = slow.write_all(b"lo HTTP/1.1\r\n\r\n");
        let mut raw = Vec::new();
        let _ = slow.read_to_end(&mut raw);
        assert!(!String::from_utf8_lossy(&raw).contains("200 OK"), "not served: {}", String::from_utf8_lossy(&raw));
        l.close();
    }

    /// What the stand-in answers `/api/big` with: 6 MB that are not all alike.
    fn big() -> Vec<u8> {
        (0..6_000_000u32).map(|i| (i.wrapping_mul(7) % 251) as u8).collect()
    }

    /// Send a request's head, then its body a little at a time (`pieces` of `size` bytes, 250 ms apart), and read the
    /// answer.
    #[cfg(windows)]
    fn trickle(port: u16, head: &str, pieces: usize, size: usize) -> String {
        let mut s = tls_to(port);
        let _ = write!(s, "{head}Content-Length: {}\r\n\r\n", pieces * size);
        for _ in 0..pieces {
            thread::sleep(Duration::from_millis(250));
            if s.write_all(&vec![b'x'; size]).is_err() {
                break;
            }
        }
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        String::from_utf8_lossy(&raw).to_string()
    }

    #[cfg(windows)]
    #[test]
    fn a_paired_computers_slow_upload_gets_through_but_a_strangers_slow_request_is_cut() {
        let (l, token, _pairing, _key) = paired_host("remote-upload", Duration::from_secs(2));
        // 5 seconds of steady sending: past the 2 second deadline for a request to arrive
        let up = trickle(l.port, &format!("POST /api/attach HTTP/1.1\r\nAuthorization: Bearer {token}\r\n"), 20, 1000);
        assert!(up.contains(" 207 "), "a paired computer's upload is passed on: {}", &up[..up.len().min(200)]);
        assert!(up.contains(&format!("\"body\":\"{}\"", "x".repeat(20_000))), "whole");
        let pair = trickle(l.port, "POST /remote/pair HTTP/1.1\r\n", 20, 100);
        assert!(!pair.contains("200 OK"), "a request without a token still has to arrive in time");
        l.close();
    }

    #[cfg(windows)]
    #[test]
    fn a_large_answer_comes_back_byte_for_byte_and_no_socket_read_ever_times_out() {
        let (l, token, _pairing, _key) = paired_host("remote-big", READ_LIMIT);
        let mut s = tls_to(l.port);
        write!(s, "GET /api/big HTTP/1.1\r\nAuthorization: Bearer {token}\r\n\r\n").unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let at = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a head") + 4;
        let head = String::from_utf8_lossy(&raw[..at]).to_string();
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        // the local server sends a large answer in chunks, passed on as they are
        let (mut body, mut rest) = (Vec::new(), &raw[at..]);
        if head.contains("Transfer-Encoding: chunked") {
            loop {
                let eol = rest.windows(2).position(|w| w == b"\r\n").expect("a chunk size");
                let size = usize::from_str_radix(std::str::from_utf8(&rest[..eol]).unwrap().trim(), 16).unwrap();
                if size == 0 {
                    break;
                }
                body.extend_from_slice(&rest[eol + 2..eol + 2 + size]);
                rest = &rest[eol + 2 + size + 2..];
            }
        } else {
            body = rest.to_vec();
        }
        assert!(body == big(), "{} bytes came back", body.len());
        // the sockets are waited on until they can be read; a receive timeout would leave one in an unknown state
        let (a, _b) = {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let a = TcpStream::connect(server.local_addr().unwrap()).unwrap();
            (a, server.accept().unwrap().0)
        };
        let conns = Conns::new(READ_LIMIT);
        let slot = conns.admit("127.0.0.1".parse().unwrap(), None).unwrap();
        let watched = slot.watch(a, BODY_IDLE);
        assert_eq!(watched.s.read_timeout().unwrap(), None);
        l.close();
    }

    /// A paired device's long wait for events, on its own thread: how long it took to end, and its status.
    #[cfg(windows)]
    fn long_wait(port: u16, token: &str) -> thread::JoinHandle<(Duration, u16)> {
        let token = token.to_string();
        thread::spawn(move || {
            let at = Instant::now();
            let (status, _, _) = ask(port, "GET", "/api/events?wait=20", Some(&token), "");
            (at.elapsed(), status)
        })
    }

    #[cfg(windows)]
    #[test]
    fn turning_off_or_removing_a_computer_ends_what_it_has_open() {
        let (l, token, pairing, _key) = paired_host("remote-off", READ_LIMIT);
        let port = l.port;
        // removing the computer ends its long wait at once
        let wait = long_wait(port, &token);
        thread::sleep(Duration::from_millis(700));
        pairing.lock().unwrap().revoke("laptop1");
        l.conns.shut_device("laptop1");
        let (took, status) = wait.join().unwrap();
        assert!(took < Duration::from_secs(3) && status != 207, "ended after {took:?} with {status}");

        let code = pairing.lock().unwrap().new_code(now());
        // (the proof is made with the host's fingerprint, as a device reads it from the connection)
        let (_, _, fp) = ask(port, "GET", "/remote/hello", None, "");
        let token = pairing.lock().unwrap().pair("laptop1", "Laptop", &rc::proof(&code, &fp, "laptop1").client, &fp, now()).unwrap().0;
        let wait = long_wait(port, &token);
        // a connection opened while on, whose request comes after off
        let mut late = tls_to(port);
        thread::sleep(Duration::from_millis(700));
        l.close();
        let (took, status) = wait.join().unwrap();
        assert!(took < Duration::from_secs(3) && status != 207, "turning off ended the long wait after {took:?} with {status}");
        let _ = write!(late, "GET /api/history HTTP/1.1\r\nAuthorization: Bearer {token}\r\n\r\n");
        let mut raw = Vec::new();
        let _ = late.read_to_end(&mut raw);
        assert!(!String::from_utf8_lossy(&raw).contains("207"), "a request after off is not passed on");
    }

    #[cfg(windows)]
    #[test]
    fn a_paired_computer_is_served_as_the_page_would_be_and_nothing_else_is() {
        let (dir, key) = (fresh("remote-listen"), test_key());
        let cert = cert::load_or_make(&dir, &key.0).unwrap();
        let local = stand_in();
        let pairing = Arc::new(Mutex::new(Pairing::load(&dir)));
        let ctx = Arc::new(Ctx { pairing: pairing.clone(), fp: cert.fp.clone(), id: "host1".into(), name: "Desktop".into(), backend: Box::new(move || Some((local, "page-token".into()))), read_limit: READ_LIMIT });
        let (l, warn) = listen(ctx, &cert, "127.0.0.1", 0..=0, Some(0)).expect("listening on 127.0.0.1");
        assert_eq!(warn, None);
        let port = l.port;

        let (code, hello, fp) = ask(port, "GET", "/remote/hello", None, "");
        assert_eq!(code, 200);
        assert_eq!(fp, cert.fp, "the certificate is the one pinned");
        assert_eq!((hello["fridae"].clone(), hello["id"].clone(), hello["name"].clone(), hello["pairing"].clone()), (json!(1), json!("host1"), json!("Desktop"), json!(false)));
        assert!(hello["version"].is_string());

        let text = pairing.lock().unwrap().new_code(now());
        assert_eq!(ask(port, "GET", "/remote/hello", None, "").1["pairing"], true);
        let pair = |proof: &str| ask(port, "POST", "/remote/pair", None, &json!({"device_id": "laptop1", "device_name": "Laptop", "proof": proof}).to_string()).1;
        let wrong = pair(&rc::proof("AAAAAAAAAA", &fp, "laptop1").client);
        assert_eq!((wrong["ok"].clone(), wrong["error"].clone()), (json!(false), json!(WRONG)));
        let right = pair(&rc::proof(&text, &fp, "laptop1").client);
        assert_eq!((right["ok"].clone(), right["host_id"].clone()), (json!(true), json!("host1")));
        let token = right["token"].as_str().unwrap().to_string();
        assert_eq!(right["confirm"].as_str(), Some(rc::proof(&text, &fp, "laptop1").confirm(&token).as_str()), "the host shows it knows the code too");
        assert_eq!(pair(&rc::proof(&text, &fp, "laptop1").client)["error"], NO_CODE, "a used code is refused");

        // served as the page's own request: its token added, the device's taken away, the body passed on
        let (code, said, _) = ask(port, "POST", "/api/send?x=1", Some(&token), r#"{"text":"hi"}"#);
        assert_eq!(code, 207, "the local server's status, unchanged");
        assert_eq!(said["url"], "/api/send?x=1");
        assert_eq!(said["method"], "POST");
        assert_eq!(said["token"], "page-token");
        assert_eq!(said["host"], format!("127.0.0.1:{local}"));
        assert_eq!(said["auth"], Value::Null);
        assert_eq!(said["body"], r#"{"text":"hi"}"#);
        assert_eq!(ask(port, "GET", "/api/history", Some(&token), "").1["url"], "/api/history");

        // remote control's own routes are not for a paired computer
        assert_eq!(ask(port, "GET", "/api/remote/host", Some(&token), "").0, 403);
        assert_eq!(ask(port, "POST", "/m/abc/api/history", Some(&token), "{}").0, 403);

        let (code, body, _) = ask(port, "GET", "/api/history", None, "");
        assert_eq!((code, body), (401, json!({"error": "not paired"})));
        assert_eq!(ask(port, "GET", "/api/history", Some(&rc::random_hex(32)), "").0, 401);

        // who is there, on a 127.0.0.1 socket
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        udp.send_to(rc::DISCOVER_ASK, ("127.0.0.1", l.discover_port.unwrap())).unwrap();
        let mut buf = [0u8; 2048];
        let (n, _) = udp.recv_from(&mut buf).expect("an answer");
        let found: Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(found, json!({"fridae": 1, "id": "host1", "name": "Desktop", "port": port, "fp": cert.fp}));

        assert!(pairing.lock().unwrap().revoke("laptop1"));
        assert_eq!(ask(port, "GET", "/api/history", Some(&token), "").0, 401, "a removed computer is refused");

        l.close();
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "turned off, nothing listens");
        drop(key);
    }
}
