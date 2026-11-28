//! Per-chat toolbox keys (port of T3 Code's McpSessionRegistry.ts and ProviderSessionManager.ts:614-711).
//!
//! A key is 32 random bytes, base64url. Only its SHA-256 is kept as the way to find it; the raw key of each chat's
//! live credential is held in memory alone (the role of T3's ProviderSessionManager) so a kept AI process can be
//! handed the same key again, and is never written anywhere. App restart: all keys are new.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime},
};

use base64::Engine;
use sha2::{Digest, Sha256};

/// Who a key speaks for. Modes are never in it: they are read live from the chat on every call.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub chat: String,
    pub ai: String,
    pub subagent: bool,
    /// Minted with the key (a UUIDv4): scopes `clientRequestId` replays to this key.
    pub namespace: String,
}

/// A key as handed to the code that launches the chat's AI.
#[derive(Clone)]
pub struct Credential {
    pub raw: String,
    pub scope: Scope,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential").field("raw", &"[redacted]").field("scope", &self.scope).finish()
    }
}

/// Why a request's key was refused (logged as the reason, never with the key).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    Missing,
    Scheme,
    Empty,
    Unknown,
    Expired,
}

impl Refusal {
    /// T3's log words for the reason.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::Missing => "missing_authorization_header",
            Refusal::Scheme => "unsupported_scheme",
            Refusal::Empty => "missing_bearer_token",
            Refusal::Unknown | Refusal::Expired => "unknown_or_expired_token",
        }
    }

    /// Whether a token was presented (T3 then adds `error="invalid_token"` to its challenge).
    pub fn presented(self) -> bool {
        matches!(self, Refusal::Unknown | Refusal::Expired)
    }
}

/// A key unused this long is refused until its chat's next turn touches it.
pub const IDLE: Duration = Duration::from_secs(24 * 3600);

struct Record {
    scope: Scope,
    /// When it was issued (kept as T3 keeps `issuedAt`).
    #[allow(dead_code)]
    issued: (Instant, SystemTime),
    alive: (Instant, SystemTime),
    expired: bool,
}

#[derive(Default)]
struct Inner {
    /// SHA-256 hex of the raw key → its record.
    by_hash: HashMap<String, Record>,
    /// chat → its live credential's scope and raw key (memory only).
    live: HashMap<String, (Scope, String)>,
    /// Raw keys revoked lately (memory only, the newest `RETIRED` of them): a kept AI process holds its key until it
    /// is replaced, and anything it prints is still redacted.
    retired: std::collections::VecDeque<String>,
}

/// How many revoked keys are still redacted.
const RETIRED: usize = 64;

/// What a key is shown as in anything a chat shows or keeps.
pub const REDACTED: &str = "[redacted]";

/// Every chat's toolbox key. One lock around the whole map, so `prepare` is serialised per chat (and across chats).
pub struct Registry {
    clock: fn() -> (Instant, SystemTime),
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Registry([redacted])")
    }
}

/// SHA-256 hex of a raw key: the only form of it the registry and the HTTP sessions keep.
pub fn hash(raw: &str) -> String {
    crate::paths::hex(&Sha256::digest(raw.as_bytes()))
}

fn real_clock() -> (Instant, SystemTime) {
    (Instant::now(), SystemTime::now())
}

/// How long since `then`: the larger of the monotonic and the wall-clock time passed (a wall clock set back counts
/// as no time, never as negative).
fn idle_for(then: (Instant, SystemTime), now: (Instant, SystemTime)) -> Duration {
    let mono = now.0.saturating_duration_since(then.0);
    let wall = now.1.duration_since(then.1).unwrap_or(Duration::ZERO);
    mono.max(wall)
}

impl Registry {
    pub fn new(clock: fn() -> (Instant, SystemTime)) -> Registry {
        Registry { clock, inner: Mutex::new(Inner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The chat's key: the same one while its chat, AI and subagent flag are unchanged, else the old one is revoked
    /// and a new one issued ("Live providers build MCP clients once; reuse credentials across workspace handoffs.").
    pub fn prepare(&self, chat: &str, ai: &str, subagent: bool) -> Credential {
        let now = (self.clock)();
        let mut inner = self.lock();
        if let Some((scope, raw)) = inner.live.get(chat).cloned() {
            if scope.ai == ai && scope.subagent == subagent {
                if let Some(rec) = inner.by_hash.get_mut(&hash(&raw)) {
                    rec.alive = now;
                    rec.expired = false;
                    return Credential { raw, scope };
                }
            }
        }
        revoke_in(&mut inner, chat);
        let mut bytes = [0u8; 32];
        crate::remote_common::random(&mut bytes);
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let scope = Scope { chat: chat.to_string(), ai: ai.to_string(), subagent, namespace: super::uuid_v4() };
        inner.by_hash.insert(hash(&raw), Record { scope: scope.clone(), issued: now, alive: now, expired: false });
        inner.live.insert(chat.to_string(), (scope.clone(), raw.clone()));
        Credential { raw, scope }
    }

    /// The scope a presented key speaks for. A key idle longer than `IDLE` is marked expired and stays refused until
    /// `touch`; a good one is kept alive.
    pub fn resolve(&self, raw: &str) -> Result<Scope, Refusal> {
        let now = (self.clock)();
        let mut inner = self.lock();
        let rec = inner.by_hash.get_mut(&hash(raw)).ok_or(Refusal::Unknown)?;
        if rec.expired {
            return Err(Refusal::Expired);
        }
        if idle_for(rec.alive, now) > IDLE {
            rec.expired = true;
            return Err(Refusal::Expired);
        }
        rec.alive = now;
        Ok(rec.scope.clone())
    }

    /// A turn of the chat starts: its keys are alive again (an expired one is revived).
    pub fn touch(&self, chat: &str) {
        let now = (self.clock)();
        for rec in self.lock().by_hash.values_mut().filter(|r| r.scope.chat == chat) {
            rec.alive = now;
            rec.expired = false;
        }
    }

    /// The chat was closed, moved to another AI, or its process released: its keys stop working.
    pub fn revoke_chat(&self, chat: &str) {
        revoke_in(&mut self.lock(), chat);
    }

    /// The app is quitting.
    pub fn revoke_all(&self) {
        let mut inner = self.lock();
        inner.by_hash.clear();
        let raws: Vec<String> = inner.live.drain().map(|(_, (_, raw))| raw).collect();
        for raw in raws {
            retire(&mut inner, raw);
        }
    }

    /// `text` with every live or lately revoked key replaced by `REDACTED` ("Bearer <key>" becomes
    /// "Bearer [redacted]"); None when it holds none. An agent's shell tools inherit `FRIDAE_MCP_KEY`, so whatever it
    /// prints passes through here before a chat shows or keeps it.
    #[cfg(test)]
    pub fn redact(&self, text: &str) -> Option<String> {
        let raws = self.raws();
        redact_with(&raws, text)
    }

    /// Every string in `v` redacted in place (see `redact`); true when anything changed.
    pub fn redact_value(&self, v: &mut serde_json::Value) -> bool {
        let raws = self.raws();
        !raws.is_empty() && redact_value_with(&raws, v)
    }

    fn raws(&self) -> Vec<String> {
        let inner = self.lock();
        inner.live.values().map(|(_, raw)| raw.clone()).chain(inner.retired.iter().cloned()).filter(|r| !r.is_empty()).collect()
    }

    #[cfg(test)]
    pub fn hashes_for_test(&self) -> Vec<String> {
        self.lock().by_hash.keys().cloned().collect()
    }
}

fn revoke_in(inner: &mut Inner, chat: &str) {
    inner.by_hash.retain(|_, r| r.scope.chat != chat);
    if let Some((_, raw)) = inner.live.remove(chat) {
        retire(inner, raw);
    }
}

fn retire(inner: &mut Inner, raw: String) {
    inner.retired.push_back(raw);
    while inner.retired.len() > RETIRED {
        inner.retired.pop_front();
    }
}

fn redact_with(raws: &[String], text: &str) -> Option<String> {
    let mut out: Option<String> = None;
    for raw in raws {
        let cur = out.as_deref().unwrap_or(text);
        if cur.contains(raw.as_str()) {
            out = Some(cur.replace(raw.as_str(), REDACTED));
        }
    }
    out
}

fn redact_value_with(raws: &[String], v: &mut serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::String(s) => match redact_with(raws, s) {
            Some(r) => {
                *s = r;
                true
            }
            None => false,
        },
        Value::Array(items) => items.iter_mut().fold(false, |changed, x| redact_value_with(raws, x) | changed),
        Value::Object(map) => map.values_mut().fold(false, |changed, x| redact_value_with(raws, x) | changed),
        _ => false,
    }
}

/// The app's registry, on the real clocks.
pub fn global() -> &'static Registry {
    static GLOBAL: LazyLock<Registry> = LazyLock::new(|| Registry::new(real_clock));
    &GLOBAL
}

/// The key in an `Authorization` header value: `Bearer <key>`, the scheme in any case.
pub fn bearer(header: &str) -> Result<&str, Refusal> {
    let header = header.trim();
    if header.is_empty() {
        return Err(Refusal::Missing);
    }
    let (scheme, rest) = header.split_once(char::is_whitespace).unwrap_or((header, ""));
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(Refusal::Scheme);
    }
    match rest.trim() {
        "" => Err(Refusal::Empty),
        token => Ok(token),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test clock: a fixed start plus offsets the test moves. One test at a time uses it.
    static CLOCK: Mutex<Option<(Instant, SystemTime)>> = Mutex::new(None);
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn test_now() -> (Instant, SystemTime) {
        CLOCK.lock().unwrap().expect("set by the test")
    }

    #[test]
    fn live_and_lately_revoked_keys_are_redacted() {
        let r = Registry::new(real_clock);
        let raw = r.prepare("c-redact", "claude", false).raw;
        let text = format!("FRIDAE_MCP_KEY={raw}
FRIDAE_MCP_AUTHORIZATION=Bearer {raw}
PATH=/bin");
        let got = r.redact(&text).expect("changed");
        assert!(!got.contains(&raw), "{got}");
        assert_eq!(got, "FRIDAE_MCP_KEY=[redacted]
FRIDAE_MCP_AUTHORIZATION=Bearer [redacted]
PATH=/bin");
        assert_eq!(r.redact("nothing secret here"), None);
        let mut v = serde_json::json!({"text": format!("key {raw}"), "items": [{"output": raw.clone()}], "n": 3});
        assert!(r.redact_value(&mut v));
        assert!(!v.to_string().contains(&raw), "{v}");
        assert_eq!(v["items"][0]["output"], REDACTED);
        // a kept process may still show a key revoked a moment ago
        r.revoke_chat("c-redact");
        assert!(!r.redact(&format!("x {raw}")).unwrap().contains(&raw));
        let raw2 = r.prepare("c-redact", "claude", false).raw;
        r.revoke_all();
        assert!(!r.redact(&format!("{raw} {raw2}")).unwrap().contains(&raw2));
    }

    fn fresh() -> (std::sync::MutexGuard<'static, ()>, Registry) {
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        *CLOCK.lock().unwrap() = Some((Instant::now(), SystemTime::now()));
        (guard, Registry::new(test_now))
    }

    fn advance(mono: Duration, wall_forward: bool, wall: Duration) {
        let mut c = CLOCK.lock().unwrap();
        let (i, s) = c.unwrap();
        *c = Some((i + mono, if wall_forward { s + wall } else { s - wall }));
    }

    const DAY_AND_A_SECOND: Duration = Duration::from_secs(24 * 3600 + 1);

    #[test]
    fn stores_only_a_hash_and_resolves_the_bearer() {
        let (_g, reg) = fresh();
        let cred = reg.prepare("c1", "claude", false);
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&cred.raw).unwrap();
        assert_eq!(bytes.len(), 32);
        let hashes = reg.hashes_for_test();
        assert_eq!(hashes, vec![crate::paths::hex(&Sha256::digest(cred.raw.as_bytes()))]);
        assert!(!hashes.iter().any(|h| h.contains(&cred.raw)));
        let scope = reg.resolve(&cred.raw).unwrap();
        assert_eq!((scope.chat.as_str(), scope.ai.as_str(), scope.subagent), ("c1", "claude", false));
        assert_eq!(scope.namespace.len(), 36, "a UUIDv4");
        assert_eq!(reg.resolve("nope"), Err(Refusal::Unknown));
    }

    #[test]
    fn reuses_while_the_scope_is_the_same() {
        let (_g, reg) = fresh();
        let a = reg.prepare("c1", "claude", false);
        let b = reg.prepare("c1", "claude", false);
        assert_eq!(a.raw, b.raw);
        let c = reg.prepare("c1", "gemini", false);
        assert_ne!(c.raw, a.raw);
        assert_eq!(reg.resolve(&a.raw), Err(Refusal::Unknown));
        assert_eq!(reg.resolve(&c.raw).unwrap().ai, "gemini");
        let d = reg.prepare("c1", "gemini", true);
        assert_ne!(d.raw, c.raw);
        assert_eq!(reg.resolve(&c.raw), Err(Refusal::Unknown));
        assert!(reg.resolve(&d.raw).unwrap().subagent);
        assert_eq!(reg.hashes_for_test().len(), 1);
    }

    #[test]
    fn expires_after_a_day_idle_and_touch_revives() {
        let (_g, reg) = fresh();
        let cred = reg.prepare("c1", "claude", false);
        let first = reg.resolve(&cred.raw).unwrap();
        advance(DAY_AND_A_SECOND, true, DAY_AND_A_SECOND);
        assert_eq!(reg.resolve(&cred.raw), Err(Refusal::Expired));
        assert_eq!(reg.resolve(&cred.raw), Err(Refusal::Expired), "stays expired");
        reg.touch("c1");
        assert_eq!(reg.resolve(&cred.raw).unwrap(), first);
    }

    #[test]
    fn a_backward_wall_clock_neither_extends_nor_revives() {
        let (_g, reg) = fresh();
        let cred = reg.prepare("c1", "claude", false);
        advance(DAY_AND_A_SECOND, false, Duration::from_secs(2 * 3600));
        assert_eq!(reg.resolve(&cred.raw), Err(Refusal::Expired));
        // and a wall clock that jumped ahead counts even when the monotonic one did not move
        let reg2 = Registry::new(test_now);
        let other = reg2.prepare("c2", "claude", false);
        advance(Duration::ZERO, true, DAY_AND_A_SECOND);
        assert_eq!(reg2.resolve(&other.raw), Err(Refusal::Expired));
    }

    #[test]
    fn touch_keeps_only_that_chat_alive() {
        let (_g, reg) = fresh();
        let one = reg.prepare("c1", "claude", false);
        let two = reg.prepare("c2", "claude", false);
        advance(Duration::from_secs(20 * 3600), true, Duration::from_secs(20 * 3600));
        reg.touch("c1");
        advance(Duration::from_secs(5 * 3600), true, Duration::from_secs(5 * 3600));
        assert!(reg.resolve(&one.raw).is_ok());
        assert_eq!(reg.resolve(&two.raw), Err(Refusal::Expired));
    }

    #[test]
    fn revoke_chat_and_revoke_all() {
        let (_g, reg) = fresh();
        let one = reg.prepare("c1", "claude", false);
        let two = reg.prepare("c2", "codex", false);
        reg.revoke_chat("c1");
        assert_eq!(reg.resolve(&one.raw), Err(Refusal::Unknown));
        assert!(reg.resolve(&two.raw).is_ok());
        let again = reg.prepare("c1", "claude", false);
        assert_ne!(again.raw, one.raw, "a revoked key is never handed out again");
        reg.revoke_all();
        assert_eq!(reg.resolve(&two.raw), Err(Refusal::Unknown));
        assert_eq!(reg.resolve(&again.raw), Err(Refusal::Unknown));
        assert!(reg.hashes_for_test().is_empty());
    }

    #[test]
    fn bearer_parses_case_insensitively() {
        assert_eq!(bearer("bearer abc"), Ok("abc"));
        assert_eq!(bearer("BEARER  abc "), Ok("abc"));
        assert_eq!(bearer("Basic x"), Err(Refusal::Scheme));
        assert_eq!(bearer(""), Err(Refusal::Missing));
        assert_eq!(bearer("Bearer "), Err(Refusal::Empty));
        assert_eq!(bearer("Bearer"), Err(Refusal::Empty));
    }

    #[test]
    fn debug_never_prints_the_key() {
        let (_g, reg) = fresh();
        let cred = reg.prepare("c1", "claude", false);
        let shown = format!("{cred:?} {reg:?}");
        assert!(!shown.contains(&cred.raw));
        assert!(shown.contains("[redacted]"));
    }
}
