//! Remote control end to end: the real host (`remote_host`: TLS listener, discovery, pairing, paired devices) and the
//! real client (`remote_client`: finding, pairing, forwarding) talking to each other, on 127.0.0.1 only.
//!
//! ```text
//!  test (the page) --HTTP--> client stand-in server --remote_client::serve_with--> TLS --> remote_host listener
//!                             (checks the client's page token, as server.rs)          --> host stand-in local server
//! ```
//!
//! The host stand-in answers as the page's server would for the few routes used here (it checks the host's page
//! token, as `server.rs` does): `/api/echo` says back what it was handed, `/api/events` waits 2 s, `/api/big` is
//! 3 MB, and `/media/clip` is served by the real `media::serve` (Range honoured). Every port is one the system picks
//! on 127.0.0.1; nothing is broadcast and the user's remote folder is never used (test folders, test keys).

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    remote_client::{self, Book},
    remote_common as rc,
    remote_host::{self, Ctx, Listener, Pairing, cert},
};

const HOST_PAGE_TOKEN: &str = "host-page-token-e2e";
const CLIENT_PAGE_TOKEN: &str = "client-page-token-e2e";
const HOST_ID: &str = "e2e0host0000000000000000000000ab";

/// The host's test key, deleted from Windows' key store when the test ends, passed or failed.
struct TestKey(String);

impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = cert::forget_key(&self.0);
    }
}

fn fresh(name: &str) -> std::path::PathBuf {
    let d = crate::paths::test_dir(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 3 MB that are not all alike.
fn big() -> Vec<u8> {
    (0..3_000_000u32).map(|i| (i.wrapping_mul(31) % 253) as u8).collect()
}

/// What `/media/clip` serves: 100 000 bytes.
fn clip() -> Vec<u8> {
    (0..100_000u32).map(|i| (i % 241) as u8).collect()
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

/// The host's local server, standing in: only its own page token gets in.
fn host_local(media_dir: std::path::PathBuf) -> u16 {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let clip_path = media_dir.join("clip.mp4");
    std::fs::write(&clip_path, clip()).unwrap();
    thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let clip_path = clip_path.clone();
            thread::spawn(move || {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let head = |n: &'static str| req.headers().iter().find(|h| h.field.equiv(n)).map(|h| h.value.as_str().to_string());
                if head("X-Token").as_deref() != Some(HOST_PAGE_TOKEN) {
                    return drop(req.respond(tiny_http::Response::from_string("{\"error\":\"forbidden\"}").with_status_code(403)));
                }
                let url = req.url().to_string();
                let path = url.split('?').next().unwrap_or("").to_string();
                if path == "/media/clip" {
                    let range = head("Range").unwrap_or_default();
                    return drop(req.respond(crate::media::serve(&clip_path, "video/mp4", &range)));
                }
                if path == "/api/big" {
                    let r = tiny_http::Response::from_data(big()).with_header(header("Content-Type", "application/octet-stream"));
                    return drop(req.respond(r));
                }
                if path == "/api/events" {
                    thread::sleep(Duration::from_secs(2));
                }
                let said = json!({
                    "url": url, "method": req.method().as_str(), "token": head("X-Token"), "auth": head("Authorization"),
                    "host": head("Host"), "range": head("Range"), "content_type": head("Content-Type"),
                    "body": body,
                });
                let status = if path == "/api/teapot" { 418 } else { 200 };
                let r = tiny_http::Response::from_string(said.to_string()).with_status_code(status).with_header(header("Content-Type", "application/json"));
                let _ = req.respond(r);
            });
        }
    });
    port
}

/// The client's local server, standing in for `server.rs`: the page token is checked, then `/m/...` goes to the real
/// forwarding (`remote_client::serve_with`, which `server::handle` calls through `serve`).
fn client_local(book: Arc<Book>) -> u16 {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    thread::spawn(move || {
        for req in server.incoming_requests() {
            let book = book.clone();
            thread::spawn(move || {
                let url = req.url().to_string();
                let given = req.headers().iter().find(|h| h.field.equiv("X-Token")).map(|h| h.value.as_str().to_string());
                if given.as_deref() != Some(CLIENT_PAGE_TOKEN) || !url.starts_with("/m/") {
                    return drop(req.respond(tiny_http::Response::from_string("forbidden").with_status_code(403)));
                }
                remote_client::serve_with(&book, req, &url);
            });
        }
    });
    port
}

/// An answer as the page receives it: status, headers, the body (de-chunked), and whether it came chunked.
struct Got {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    chunked: bool,
}

impl Got {
    fn head(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

/// Parse a raw HTTP/1.1 answer read to its end.
fn parse(raw: &[u8]) -> Got {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("an answer's head") + 4;
    let head = String::from_utf8_lossy(&raw[..at]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let chunked = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("Transfer-Encoding") && v.to_ascii_lowercase().contains("chunked"));
    let mut body = Vec::new();
    let mut rest = &raw[at..];
    if chunked {
        loop {
            let eol = rest.windows(2).position(|w| w == b"\r\n").expect("a chunk size");
            let size = usize::from_str_radix(std::str::from_utf8(&rest[..eol]).unwrap().split(';').next().unwrap().trim(), 16).unwrap();
            if size == 0 {
                break;
            }
            body.extend_from_slice(&rest[eol + 2..eol + 2 + size]);
            assert_eq!(&rest[eol + 2 + size..eol + 4 + size], b"\r\n", "each chunk ends with CRLF");
            rest = &rest[eol + 4 + size..];
        }
    } else {
        body = rest.to_vec();
        if let Some(n) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("Content-Length")).and_then(|(_, v)| v.parse::<usize>().ok()) {
            assert_eq!(body.len(), n, "Content-Length matches the body");
        }
    }
    Got { status, headers, body, chunked }
}

/// The page's call to this computer's server, as fetch would make it.
fn page(port: u16, method: &str, target: &str, extra: &[(&str, &str)], body: &[u8]) -> Got {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(90))).unwrap();
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Token: {CLIENT_PAGE_TOKEN}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    parse(&raw)
}

/// One request straight to the host over TLS (as a paired device would send it), the raw answer kept.
fn to_host(port: u16, request: &str) -> Vec<u8> {
    let tls = native_tls::TlsConnector::builder().danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true).build().unwrap();
    let mut s = tls.connect("fridae", TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    raw
}

/// The real host listening on 127.0.0.1 (TLS port and discovery port both chosen by the system) with a test key,
/// in front of the host stand-in.
struct Host {
    listener: Option<Listener>,
    pairing: Arc<Mutex<Pairing>>,
    fp: String,
    port: u16,
    discover: u16,
    dir: std::path::PathBuf,
    /// the computers the host's own app keeps, where a paired computer's offer to be reached in turn goes
    offers: Arc<Book>,
    _key: TestKey,
}

fn host(name: &str) -> Host {
    let dir = fresh(&format!("remote-e2e-host-{name}"));
    let key = TestKey(format!("Fridae remote control test {}", rc::random_hex(4)));
    let cert = cert::load_or_make(&dir, &key.0).expect("a test certificate");
    let local = host_local(dir.clone());
    let pairing = Arc::new(Mutex::new(Pairing::load(&dir)));
    let offers = Arc::new(Book::open(&dir.join("machines.json")));
    let kept = offers.clone();
    let ctx = Arc::new(Ctx {
        pairing: pairing.clone(),
        fp: cert.fp.clone(),
        id: HOST_ID.into(),
        name: "Desk".into(),
        backend: Box::new(move || Some((local, HOST_PAGE_TOKEN.into()))),
        read_limit: Duration::from_secs(10),
        offered: Box::new(move |o| remote_client::take_offer(&kept, o)),
    });
    let (l, warn) = remote_host::listen(ctx, &cert, "127.0.0.1", 0..=0, Some(0)).expect("listening on 127.0.0.1");
    assert_eq!(warn, None);
    let (port, discover) = (l.port, l.discover_port.unwrap());
    assert!(port != rc::LISTEN_PORT && discover != rc::DISCOVER_PORT, "test ports only");
    Host { listener: Some(l), pairing, fp: cert.fp, port, discover, dir, offers, _key: key }
}

fn client_book(name: &str) -> Arc<Book> {
    let d = fresh(&format!("remote-e2e-client-{name}"));
    Arc::new(Book::open(&d.join("machines.json")))
}

/// Find the host by unicast to its discovery socket, then pair with the code the host shows, through the same
/// function `/api/remote/pair` calls, with the fingerprint from finding (so the pinned hello is used).
fn find_and_pair(h: &Host, book: &Book, me: &Value) -> Value {
    let targets = [SocketAddr::from(([127, 0, 0, 1], h.discover))];
    let found = remote_client::discover_on(book, me["id"].as_str().unwrap(), "127.0.0.1:0", &targets, Duration::from_millis(800));
    let list = found["found"].as_array().unwrap();
    assert_eq!(list.len(), 1, "the host answers finding: {found}");
    let f = &list[0];
    assert_eq!((f["id"].as_str(), f["addr"].as_str(), f["port"].as_u64(), f["fp"].as_str()), (Some(HOST_ID), Some("127.0.0.1"), Some(u64::from(h.port)), Some(h.fp.as_str())));
    assert_eq!(f["paired"], false);

    let code = h.pairing.lock().unwrap().new_code(remote_host_now());
    let shown = rc::show_code(&code);
    // a wrong code is refused with the host's own words, and nothing is saved
    let wrong = remote_client::pair_with(book, me, &json!({"addr": f["addr"], "port": f["port"], "fp": f["fp"], "code": "AAAAA-AAAAA"}));
    assert_eq!(wrong["ok"], false, "{wrong}");
    assert_eq!(wrong["error"], "That code is not right. Check it and try again.");
    assert!(book.find(HOST_ID).is_none());
    // a fingerprint other than the one found: refused before the code is used
    let other = remote_client::pair_with(book, me, &json!({"addr": f["addr"], "port": f["port"], "fp": "00".repeat(32), "code": shown}));
    assert_eq!(other["error"], "That computer's identity didn't match the one found. Try finding it again.");
    // the right code, typed the way it is shown
    let ok = remote_client::pair_with(book, me, &json!({"addr": f["addr"], "port": f["port"], "fp": f["fp"], "code": shown}));
    assert_eq!(ok["ok"], true, "{ok}");
    ok
}

fn remote_host_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn remote_e2e_a_paired_computer_is_reached_through_the_real_host() {
    let h = host("main");
    let book = client_book("main");
    // this computer's identity, as `/api/remote/pair` passes it (read once: the tests share the test folder, and two
    // tests making the identity file at the same moment could each see a different id)
    let me = rc::identity();
    let ok = find_and_pair(&h, &book, &me);
    let m = &ok["machine"];
    assert_eq!((m["id"].as_str(), m["name"].as_str(), m["fp"].as_str(), m["paired"].as_bool()), (Some(HOST_ID), Some("Desk"), Some(h.fp.as_str()), Some(true)));
    // the host keeps this computer as a device, under its own id and name
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(h.dir.join("devices.json")).unwrap()).unwrap();
    let devices = saved.as_array().or_else(|| saved["devices"].as_array()).cloned().unwrap_or_default();
    assert_eq!(devices.len(), 1, "{saved}");
    assert_eq!(devices[0]["id"], me["id"]);
    assert!(!saved.to_string().contains(&book.token_of(HOST_ID).unwrap()), "the host keeps only the token's hash");
    // the code is used up: a second pairing with it is refused
    let again = remote_client::pair_with(&book, &me, &json!({"addr": "127.0.0.1", "port": h.port, "code": "AAAAA-AAAAA"}));
    // (hello says no code is live, so the client stops before sending a proof)
    assert_eq!(again["error"], "That computer isn't showing a pairing code. Make a new code in its Settings.");

    let port = client_local(book.clone());
    let m = format!("/m/{HOST_ID}");

    // GET with a query: the page token `t` stays here, the rest of the query arrives; the host adds its own page token
    let g = page(port, "GET", &format!("{m}/api/echo?a=1&t={CLIENT_PAGE_TOKEN}&b=two%20words"), &[], b"");
    assert_eq!(g.status, 200);
    assert_eq!(g.head("Content-Type"), Some("application/json"));
    let said = g.json();
    assert_eq!(said["url"], "/api/echo?a=1&b=two%20words");
    assert_eq!(said["method"], "GET");
    assert_eq!(said["token"], HOST_PAGE_TOKEN, "the host's page token, never the client's");
    assert_eq!(said["auth"], Value::Null, "the device token stays on the host");

    // POST with a JSON body, and a status other than 200 passed through
    let body = br#"{"text":"hello from the laptop","n":[1,2,3]}"#;
    let p = page(port, "POST", &format!("{m}/api/teapot"), &[("Content-Type", "application/json")], body);
    assert_eq!(p.status, 418);
    let said = p.json();
    assert_eq!((said["method"].as_str(), said["body"].as_str(), said["content_type"].as_str()), (Some("POST"), Some(std::str::from_utf8(body).unwrap()), Some("application/json")));

    // the long wait for events: the 2 s answer arrives whole
    let at = Instant::now();
    let e = page(port, "GET", &format!("{m}/api/events?after=5&wait=20"), &[], b"");
    assert_eq!(e.status, 200);
    assert!(at.elapsed() >= Duration::from_secs(2), "it waited");
    assert_eq!(e.json()["url"], "/api/events?after=5&wait=20");

    // 3 MB: the host passes the local server's chunks on, the client reads them and the page gets every byte
    let raw = to_host(h.port, &format!("GET /api/big HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n", book.token_of(HOST_ID).unwrap()));
    let direct = parse(&raw);
    assert!(direct.chunked, "the host sends 3 MB chunked, as the local server did");
    let b = page(port, "GET", &format!("{m}/api/big"), &[], b"");
    assert_eq!(b.status, 200);
    assert_eq!(b.head("Content-Type"), Some("application/octet-stream"));
    assert!(b.body == big(), "byte for byte: {} bytes came back", b.body.len());

    // Range on media: 206, the right bytes, Content-Range and Accept-Ranges as the host's server set them
    let r = page(port, "GET", &format!("{m}/media/clip?t={CLIENT_PAGE_TOKEN}"), &[("Range", "bytes=1000-1999")], b"");
    assert_eq!(r.status, 206);
    assert_eq!(r.head("Content-Range"), Some("bytes 1000-1999/100000"));
    assert_eq!(r.head("Accept-Ranges"), Some("bytes"));
    assert_eq!(r.head("Content-Type"), Some("video/mp4"));
    assert!(r.body == clip()[1000..2000], "the asked-for part");
    let whole = page(port, "GET", &format!("{m}/media/clip"), &[], b"");
    assert_eq!((whole.status, whole.body.len()), (200, 100_000));

    // an attachment-sized upload (1.5 MB) arrives whole
    let upload: Vec<u8> = (0..1_500_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let up = page(port, "POST", &format!("{m}/api/echo"), &[("Content-Type", "application/octet-stream")], &upload);
    assert_eq!(up.status, 200);
    assert!(up.json()["body"].as_str().map(str::as_bytes) == Some(&upload[..]), "the upload arrived whole");

    // as many calls at once as a browser makes to one address (6), long waits among them, with the background
    // check alongside: all answered, and the computer stays online (the host lets one machine have 8 connections)
    let calls: Vec<_> = (0..6)
        .map(|i| {
            let m = m.clone();
            let path = if i % 2 == 0 { "/api/events?wait=20".to_string() } else { format!("/api/echo?i={i}") };
            thread::spawn(move || page(port, "GET", &format!("{m}{path}"), &[], b"").status)
        })
        .collect();
    thread::sleep(Duration::from_millis(300));
    remote_client::check_all_within(&book, Duration::from_secs(5));
    let statuses: Vec<u16> = calls.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(statuses, vec![200; 6]);
    assert_eq!(book.find(HOST_ID).unwrap()["online"], true);

    // remote control's own routes are not for a paired computer: 403 from the host, passed on as it is
    for (method, path) in [("GET", "/api/remote/host"), ("POST", "/api/remote/host"), ("GET", "/api/remote/machines"), ("POST", "/api/remote/pair"), ("GET", "/remote/devices"), ("GET", "/remote/pair"), ("GET", "/m/abc/api/history"), ("GET", "/m")] {
        let x = page(port, method, &format!("{m}{path}"), &[("Content-Type", "application/json")], if method == "POST" { b"{}" } else { b"" });
        assert_eq!((x.status, x.json()), (403, json!({"error": "not allowed from another computer"})), "{method} {path}");
    }
    // ... and the paired computer is still paired after them
    assert_eq!(page(port, "GET", &format!("{m}/api/echo"), &[], b"").status, 200);

    // the host removes this computer: the next call is 401, the client marks it unpaired and stops calling
    assert!(h.pairing.lock().unwrap().revoke(me["id"].as_str().unwrap()));
    let x = page(port, "GET", &format!("{m}/api/echo"), &[], b"");
    assert_eq!((x.status, x.json()), (401, json!({"error": "not paired"})));
    assert_eq!(book.find(HOST_ID).unwrap()["paired"], false);
    assert!(book.token_of(HOST_ID).is_none());
    assert!(Book::open(&book.path).token_of(HOST_ID).is_none(), "and stays so after a restart");
    let x = page(port, "GET", &format!("{m}/api/echo"), &[], b"");
    assert_eq!((x.status, x.json()), (401, json!({"error": "not paired"})));

    // finding now shows it as not paired, and pairing again with a new code brings it back
    let code = h.pairing.lock().unwrap().new_code(remote_host_now());
    let again = remote_client::pair_with(&book, &me, &json!({"addr": "127.0.0.1", "port": h.port, "fp": h.fp, "code": code}));
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(page(port, "GET", &format!("{m}/api/echo"), &[], b"").status, 200);

    // an unknown computer
    let u = page(port, "GET", "/m/nobody/api/echo", &[], b"");
    assert_eq!((u.status, u.json()), (404, json!({"error": "unknown computer"})));

    drop(h);
}

#[test]
fn remote_e2e_pairing_once_lets_each_computer_reach_the_other() {
    let h = host("both-ways");
    let book = client_book("both-ways");
    let me = rc::identity();
    let my_id = me["id"].as_str().unwrap().to_string();
    find_and_pair(&h, &book, &me);
    let (fp, token) = ("ab".repeat(32), "cd".repeat(32));
    // this computer offers the one it paired with a token for its own host, over the paired connection
    let took = remote_client::offer_back(&book, HOST_ID, "Laptop", |id, name| {
        assert_eq!((id, name), (HOST_ID, "Desk"));
        Some((47_999, fp.clone(), token.clone()))
    }, |_| panic!("a taken offer is kept"));
    assert!(took);
    let there = h.offers.find(&my_id).expect("the host's app keeps this computer, under its own id");
    assert_eq!((there["name"].as_str(), there["addr"].as_str(), there["port"].as_u64(), there["fp"].as_str(), there["paired"].as_bool()), (Some("Laptop"), Some("127.0.0.1"), Some(47_999), Some(fp.as_str()), Some(true)), "at the address the offer came from: {there}");
    assert_eq!(h.offers.token_of(&my_id).as_deref(), Some(token.as_str()));
    // nothing to offer (it can reach this one already, or network access is off here): nothing is sent
    assert!(!remote_client::offer_back(&book, HOST_ID, "Laptop", |_, _| None, |_| panic!("nothing was made")));
    // an offer the host can't use is refused, and taken back here
    let back = Mutex::new(String::new());
    assert!(!remote_client::offer_back(&book, HOST_ID, "Laptop", |_, _| Some((0, fp.clone(), token.clone())), |id| *back.lock().unwrap() = id.to_string()));
    assert_eq!(*back.lock().unwrap(), HOST_ID);
    // only a paired computer may offer: one without a token is refused
    let body = format!(r#"{{"name":"Mallory","port":1,"fp":"{fp}","token":"{token}"}}"#);
    let raw = to_host(h.port, &format!("POST /remote/offer HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 401"), "{}", String::from_utf8_lossy(&raw));
    assert!(h.offers.find("Mallory").is_none());
}

#[test]
fn remote_e2e_turning_network_access_off_shows_offline() {
    let mut h = host("off");
    let book = client_book("off");
    find_and_pair(&h, &book, &rc::identity());
    let port = client_local(book.clone());
    let m = format!("/m/{HOST_ID}");
    assert_eq!(page(port, "GET", &format!("{m}/api/echo"), &[], b"").status, 200);
    remote_client::check_all_within(&book, Duration::from_secs(5));
    assert_eq!(book.find(HOST_ID).unwrap()["online"], true);

    // a long wait open while access is turned off ends (no answer from the host: offline), it does not hang
    let wait = {
        let m = m.clone();
        thread::spawn(move || {
            let at = Instant::now();
            let g = page(port, "GET", &format!("{m}/api/events?wait=20"), &[], b"");
            (at.elapsed(), g.status)
        })
    };
    thread::sleep(Duration::from_millis(600));
    h.listener.take().unwrap().close();
    let (took, status) = wait.join().unwrap();
    assert!(took < Duration::from_secs(5) && status != 200, "the open wait ended after {took:?} with {status}");

    let x = page(port, "GET", &format!("{m}/api/echo"), &[], b"");
    assert_eq!((x.status, x.json()), (502, json!({"error": "offline"})));
    assert_eq!(book.find(HOST_ID).unwrap()["online"], false);
    // still paired: off is not a revoke
    assert_eq!(book.find(HOST_ID).unwrap()["paired"], true);
    remote_client::check_all_within(&book, Duration::from_secs(5));
    assert_eq!(book.find(HOST_ID).unwrap()["online"], false);
    // nothing listens any more
    assert!(TcpStream::connect(("127.0.0.1", h.port)).is_err());
}
