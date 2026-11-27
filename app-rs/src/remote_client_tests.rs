//! The client against a fake host: TLS on 127.0.0.1 with a test-only certificate (tests/remote), a
//! pairing code, and a Bearer-checked `/api/echo` that says back what it was sent.

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::remote_common::{self, DISCOVER_ASK, proof, random_hex, same};

const CODE: &str = "ABCDE-FGHJK";
const HOST_ID: &str = "11112222333344445555666677778888";
const ME: &str = "c0ffee00c0ffee00c0ffee00c0ffee00";
const P12: &[u8] = include_bytes!("../tests/remote/host.p12");
const DER: &[u8] = include_bytes!("../tests/remote/cert.der");

fn fixture_fp() -> String {
    remote_common::hex(&Sha256::digest(DER))
}

#[derive(Default)]
struct HostState {
    token: Option<String>,
    revoked: bool,
    pair_tries: u32,
    /// Requests that got as far as their first line: none may arrive over a certificate the client refused.
    requests: u32,
}

struct FakeHost {
    port: u16,
    state: Arc<Mutex<HostState>>,
}

fn fake_host() -> FakeHost {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let identity = native_tls::Identity::from_pkcs12(P12, "fridae-test").unwrap();
    let acceptor = Arc::new(native_tls::TlsAcceptor::new(identity).unwrap());
    let state = Arc::new(Mutex::new(HostState::default()));
    let st = state.clone();
    thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (acceptor, st) = (acceptor.clone(), st.clone());
            thread::spawn(move || {
                if let Ok(tls) = acceptor.accept(tcp) {
                    host_answer(tls, &st);
                }
            });
        }
    });
    FakeHost { port, state }
}

fn host_answer(tls: native_tls::TlsStream<TcpStream>, st: &Mutex<HostState>) {
    let mut r = BufReader::new(tls);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    if !line.is_empty() {
        st.lock().unwrap().requests += 1;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut heads = Vec::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            heads.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let head = |k: &str| heads.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let mut body = vec![0; head("content-length").parse().unwrap_or(0)];
    r.read_exact(&mut body).unwrap();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
    let out = r.get_mut();
    let send = |out: &mut native_tls::TlsStream<TcpStream>, status: u16, v: Value| {
        let b = v.to_string();
        let _ = write!(out, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len());
    };
    let mut s = st.lock().unwrap();
    if path == "/remote/hello" {
        return send(out, 200, json!({"fridae": 1, "id": HOST_ID, "name": "Desk", "version": "0", "pairing": true}));
    }
    if path == "/remote/pair" {
        s.pair_tries += 1;
        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
        let want = proof(CODE, &fixture_fp(), v["device_id"].as_str().unwrap_or(""));
        if same(&want, v["proof"].as_str().unwrap_or("")) {
            let token = random_hex(32);
            s.token = Some(token.clone());
            return send(out, 200, json!({"ok": true, "token": token, "host_id": HOST_ID}));
        }
        return send(out, 200, json!({"ok": false, "error": "That code isn't right."}));
    }
    let paired = s.token.as_ref().is_some_and(|t| head("authorization") == format!("Bearer {t}")) && !s.revoked;
    if !paired {
        return send(out, 401, json!({"error": "not paired"}));
    }
    if path == "/api/chunked" {
        let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nhello \r\n5;x=1\r\nworld\r\n0\r\n\r\n");
        return;
    }
    let status = query.split('&').find_map(|kv| kv.strip_prefix("status=")).and_then(|v| v.parse().ok()).unwrap_or(200);
    send(
        out,
        status,
        json!({"method": method, "path": path, "query": query, "body": String::from_utf8_lossy(&body),
               "range": head("range"), "x_token": head("x-token"), "content_type": head("content-type")}),
    );
}

fn book(name: &str) -> Book {
    let d = crate::paths::test_dir(&format!("remote-client-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    Book::open(&d.join("machines.json"))
}

fn me() -> Value {
    json!({"id": ME, "name": "Laptop"})
}

fn read(a: Answer) -> (u16, Value) {
    let mut b = Vec::new();
    let status = a.status;
    a.body.take(10_000_000).read_to_end(&mut b).unwrap();
    (status, serde_json::from_slice(&b).unwrap_or_else(|_| json!(String::from_utf8_lossy(&b))))
}

fn get(b: &Book, target: &str) -> (u16, Value) {
    read(forward(b, HOST_ID, "GET", target, &[], b""))
}

fn paired(name: &str) -> (Book, FakeHost) {
    let b = book(name);
    let h = fake_host();
    let r = pair_with(&b, &me(), &json!({"addr": "127.0.0.1", "port": h.port, "code": CODE}));
    assert_eq!(r["ok"], true, "{r}");
    (b, h)
}

/// A port on 127.0.0.1 nothing listens on.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn saved(id: &str, addr: &str, token: Option<&str>) -> Machine {
    Machine { id: id.into(), name: "Desk".into(), addr: addr.into(), port: 47999, fp: fixture_fp(), token: token.map(str::to_string), online: false, last_seen: 0 }
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn saved_computers_keep_the_token_protected() {
    let b = book("store");
    let secret = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    b.add(saved("m1", "10.0.0.5", Some(secret)));
    let raw = fs::read_to_string(&b.path).unwrap();
    assert!(raw.contains("\"m1\"") && raw.contains("10.0.0.5"), "{raw}");
    assert!(!raw.contains(secret), "the token is in the file as plain text");
    assert!(!raw.contains(&B64.encode(secret)), "the token is in the file as plain base64");
    let again = Book::open(&b.path);
    assert_eq!(again.token_of("m1").as_deref(), Some(secret));
    let shown = again.find("m1").unwrap();
    assert_eq!((shown["addr"].as_str(), shown["paired"].as_bool()), (Some("10.0.0.5"), Some(true)));
    assert!(!shown.to_string().contains(secret));
}

#[test]
fn the_page_token_is_left_out_of_the_query() {
    assert_eq!(without_token("a=1&t=SECRET&b=2"), "a=1&b=2");
    assert_eq!(without_token("t=SECRET"), "");
    assert_eq!(without_token("at=1&tt=2&t"), "at=1&tt=2");
    assert_eq!(split_m("/m/abc/api/echo?x=1"), Some(("abc".into(), "/api/echo?x=1".into())));
    assert_eq!(split_m("/m/abc"), Some(("abc".into(), "/".into())));
    assert_eq!(split_m("/m//api"), None);
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn pairing_with_a_wrong_code_is_refused_and_a_right_one_saves_the_computer() {
    let b = book("pair");
    let h = fake_host();
    let bad = pair_with(&b, &me(), &json!({"addr": "127.0.0.1", "port": h.port, "code": "ZZZZZ-ZZZZZ"}));
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"], "That code isn't right.");
    assert!(b.find(HOST_ID).is_none());

    let none = pair_with(&b, &me(), &json!({"addr": "127.0.0.1", "port": h.port, "code": " "}));
    assert_eq!(none["ok"], false);

    let good = pair_with(&b, &me(), &json!({"addr": "127.0.0.1", "port": h.port, "code": "abcde fghjk"}));
    assert_eq!(good["ok"], true, "{good}");
    let m = &good["machine"];
    assert_eq!((m["id"].as_str(), m["name"].as_str(), m["fp"].as_str()), (Some(HOST_ID), Some("Desk"), Some(fixture_fp().as_str())));
    assert_eq!((m["port"].as_u64(), m["online"].as_bool()), (Some(h.port as u64), Some(true)));
    assert!(m.get("token").is_none());
    assert!(b.token_of(HOST_ID).is_some());
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn pairing_refuses_a_certificate_other_than_the_one_found() {
    let b = book("pair-fp");
    let h = fake_host();
    let r = pair_with(&b, &me(), &json!({"addr": "127.0.0.1", "port": h.port, "code": CODE, "fp": "ab".repeat(32)}));
    assert_eq!(r["ok"], false);
    assert_eq!(h.state.lock().unwrap().requests, 0, "a request was sent to a certificate other than the one found");
    assert!(b.find(HOST_ID).is_none());
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn forwarding_passes_status_query_and_body_but_not_the_page_token() {
    let (b, _h) = paired("forward");
    let heads = vec![("Range".to_string(), "bytes=0-9".to_string()), ("X-Token".to_string(), "PAGESECRET".to_string())];
    let (status, v) = read(forward(&b, HOST_ID, "GET", "/api/echo?x=1&t=PAGESECRET&y=a%20b", &heads, b""));
    assert_eq!(status, 200);
    assert_eq!((v["method"].as_str(), v["path"].as_str(), v["query"].as_str()), (Some("GET"), Some("/api/echo"), Some("x=1&y=a%20b")));
    assert_eq!((v["range"].as_str(), v["x_token"].as_str()), (Some("bytes=0-9"), Some("")));

    let heads = vec![("Content-Type".to_string(), "application/json".to_string())];
    let (status, v) = read(forward(&b, HOST_ID, "POST", "/api/echo?status=418", &heads, br#"{"hi":1}"#));
    assert_eq!(status, 418);
    assert_eq!((v["method"].as_str(), v["body"].as_str(), v["content_type"].as_str()), (Some("POST"), Some(r#"{"hi":1}"#), Some("application/json")));

    assert_eq!(get(&b, "/api/chunked"), (200, json!("hello world")));
    assert_eq!(b.find(HOST_ID).unwrap()["online"], true);
    assert_eq!(read(forward(&b, "nobody", "GET", "/api/echo", &[], b"")), (404, json!({"error": "unknown computer"})));
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn a_changed_certificate_is_refused() {
    let (b, _h) = paired("changed");
    b.set_fp(HOST_ID, &"ab".repeat(32));
    assert_eq!(get(&b, "/api/echo"), (502, json!({"error": "identity changed"})));
    assert_eq!(b.find(HOST_ID).unwrap()["online"], false);
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn a_401_from_the_host_marks_the_computer_unpaired() {
    let (b, h) = paired("revoked");
    h.state.lock().unwrap().revoked = true;
    assert_eq!(get(&b, "/api/echo"), (401, json!({"error": "not paired"})));
    assert_eq!(b.find(HOST_ID).unwrap()["paired"], false);
    assert!(b.token_of(HOST_ID).is_none());
    assert!(Book::open(&b.path).token_of(HOST_ID).is_none(), "the forgotten token came back after a restart");
    h.state.lock().unwrap().revoked = false;
    assert_eq!(get(&b, "/api/echo"), (401, json!({"error": "not paired"})));
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn a_computer_that_does_not_answer_is_offline() {
    let (b, _h) = paired("offline");
    b.set_port(HOST_ID, closed_port());
    assert_eq!(get(&b, "/api/echo"), (502, json!({"error": "offline"})));
    assert_eq!(b.find(HOST_ID).unwrap()["online"], false);
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn the_background_check_follows_whether_a_computer_answers() {
    let (b, h) = paired("check");
    b.set_port(HOST_ID, closed_port());
    check_all(&b);
    assert_eq!(b.find(HOST_ID).unwrap()["online"], false);
    b.set_port(HOST_ID, h.port);
    check_all(&b);
    assert_eq!(b.find(HOST_ID).unwrap()["online"], true);
}

#[test]
fn finding_lists_who_answers_and_updates_a_saved_address() {
    let b = book("discover");
    b.add(saved(HOST_ID, "10.9.9.9", Some("t0k")));
    let answer = |id: &'static str| {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = sock.local_addr().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            if let Ok((n, from)) = sock.recv_from(&mut buf)
                && &buf[..n] == DISCOVER_ASK
            {
                let reply = json!({"fridae": 1, "id": id, "name": "Desk", "port": 47999, "fp": fixture_fp()});
                let _ = sock.send_to(reply.to_string().as_bytes(), from);
            }
        });
        at
    };
    let targets = [answer(HOST_ID), answer(ME)];
    let found = discover_on(&b, ME, "127.0.0.1:0", &targets, Duration::from_millis(500));
    let list = found["found"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{found}");
    let f = &list[0];
    assert_eq!((f["id"].as_str(), f["addr"].as_str(), f["port"].as_u64(), f["paired"].as_bool()), (Some(HOST_ID), Some("127.0.0.1"), Some(47999), Some(true)));
    assert_eq!(f["fp"], fixture_fp());
    assert_eq!(b.find(HOST_ID).unwrap()["addr"], "127.0.0.1");
}

/// A TLS host on 127.0.0.1 with the test certificate that reads the start of a request and then misbehaves.
fn hostile_host(misbehave: fn(&mut native_tls::TlsStream<TcpStream>)) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let identity = native_tls::Identity::from_pkcs12(P12, "fridae-test").unwrap();
    let acceptor = Arc::new(native_tls::TlsAcceptor::new(identity).unwrap());
    thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let acceptor = acceptor.clone();
            thread::spawn(move || {
                if let Ok(mut tls) = acceptor.accept(tcp) {
                    let mut buf = [0u8; 1024];
                    let _ = tls.read(&mut buf);
                    misbehave(&mut tls);
                }
            });
        }
    });
    port
}

/// A header line that never ends: the client gives up after a few kilobytes instead of filling its memory.
fn endless_header(tls: &mut native_tls::TlsStream<TcpStream>) {
    let _ = tls.write_all(b"HTTP/1.1 200 OK
X-Endless: ");
    let block = [b'a'; 16 * 1024];
    while tls.write_all(&block).is_ok() {}
}

/// One byte every 300 ms, for ever: each read gets something, so only an overall deadline ends it.
fn trickle(tls: &mut native_tls::TlsStream<TcpStream>) {
    for b in b"HTTP/1.1 200 OK
".iter().chain(std::iter::repeat(&b'a')) {
        if tls.write_all(&[*b]).and_then(|_| tls.flush()).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn an_endless_header_is_refused_before_it_fills_memory() {
    let port = hostile_host(endless_header);
    let t = std::time::Instant::now();
    assert_eq!(call_json("127.0.0.1", port, None, "GET", "/remote/hello", None, Duration::from_secs(10)).err(), Some(Fail::Offline));
    assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
    let r = pair_with(&book("endless"), &me(), &json!({"addr": "127.0.0.1", "port": port, "code": CODE}));
    assert_eq!(r["ok"], false);
}

#[test]
fn a_host_that_trickles_bytes_is_given_up_on_at_the_deadline() {
    let port = hostile_host(trickle);
    let t = std::time::Instant::now();
    assert_eq!(call_json("127.0.0.1", port, None, "GET", "/remote/hello", None, Duration::from_secs(2)).err(), Some(Fail::Offline));
    assert!(t.elapsed() < Duration::from_secs(4), "took {:?}", t.elapsed());
}

#[test]
#[cfg(windows)] // pairing keeps the token with DPAPI, which is Windows only
fn one_slow_computer_does_not_hold_up_the_check_of_the_others() {
    let (b, _h) = paired("check-slow");
    b.add(saved("slow", "127.0.0.1", Some("t0k")));
    b.set_port("slow", hostile_host(trickle));
    let t = std::time::Instant::now();
    check_all_within(&b, Duration::from_secs(2));
    assert!(t.elapsed() < Duration::from_secs(4), "took {:?}", t.elapsed());
    assert_eq!(b.find(HOST_ID).unwrap()["online"], true);
    assert_eq!(b.find("slow").unwrap()["online"], false);
}
