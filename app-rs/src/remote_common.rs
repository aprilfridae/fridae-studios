//! What both sides of remote control share: this computer's identity, secure random values, and the pairing proof.
//!
//! A computer that lets others connect ("the host") shows a pairing code. The computer that connects ("the
//! client") never sends the code itself: it sends a proof made from the code, the host's certificate fingerprint as
//! the client saw it, and the client's own id. The host makes the same proof with its own fingerprint; they match
//! only when both know the code and nobody sat in between with a different certificate. The host then answers with
//! its own confirmation, made from the same key and the token it gives, so the client saves only a computer that
//! knew the code as well. The key is slow to make (PBKDF2, 200 000 rounds) so a recorded proof cannot be tried
//! against every code in the five minutes a code lasts.

use std::{fs, path::PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The port the host listens on (TLS); when it is taken, the next free one up to `LISTEN_PORT + 9`.
pub const LISTEN_PORT: u16 = 47530;
/// The UDP port hosts answer "who is there" on.
pub const DISCOVER_PORT: u16 = 47529;
/// What a client broadcasts to find hosts.
pub const DISCOVER_ASK: &[u8] = b"FRIDAE?1";
/// The letters a pairing code is made of (no 0/O, 1/I/L).
pub const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 10;
pub const PROOF_ROUNDS: u32 = 200_000;

/// The folder for remote control's own files (identity, paired devices, saved computers): outside the work
/// folder, the projects and the kit, so no AI is handed it.
pub fn dir() -> PathBuf {
    let d = crate::paths::private_dir().join("remote");
    let _ = fs::create_dir_all(&d);
    d
}

/// Fill `buf` from the system's secure random source.
pub fn random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the system's secure random source failed");
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    random(&mut b);
    hex(&b)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A new pairing code: `CODE_LEN` letters from `CODE_ALPHABET` (shown as two groups of five with a dash).
pub fn new_code() -> String {
    let mut out = String::new();
    while out.len() < CODE_LEN {
        let mut b = [0u8; 16];
        random(&mut b);
        // only bytes below the largest multiple of the alphabet's size, so every letter is as likely
        let lim = 256 - 256 % CODE_ALPHABET.len();
        for x in b {
            if (x as usize) < lim && out.len() < CODE_LEN {
                out.push(CODE_ALPHABET[x as usize % CODE_ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// A code as typed: capitals, with spaces and dashes left out.
pub fn normal_code(typed: &str) -> String {
    typed.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// A code as shown: `ABCDE-FGHJK`.
pub fn show_code(code: &str) -> String {
    let c = normal_code(code);
    if c.len() == CODE_LEN { format!("{}-{}", &c[..5], &c[5..]) } else { c }
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// PBKDF2-HMAC-SHA256 with a 32-byte result (one block).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut s = salt.to_vec();
    s.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &s);
    let mut out = u;
    for _ in 1..rounds {
        u = hmac_sha256(password, &u);
        for (o, x) in out.iter_mut().zip(u) {
            *o ^= x;
        }
    }
    out
}

/// Both halves of pairing, from one slow key: `client` is what the client sends to show it knows the code, and
/// `confirm(token)` what the host sends back with the token to show it knows the code too.
pub struct Proof {
    pub client: String,
    key: [u8; 32],
}

impl Proof {
    /// The host's half (lowercase hex), for the token it gives: HMAC of `host|<token>` with the key.
    pub fn confirm(&self, token: &str) -> String {
        hex(&hmac_sha256(&self.key, format!("host|{token}").as_bytes()))
    }
}

/// The pairing proof. The key is PBKDF2-HMAC-SHA256 of the normalised code, salted with
/// `fridae-pair-v1|<certificate fingerprint>|<client id>`; the client's half is the HMAC of `client` with it
/// (lowercase hex). Each side works out the key once.
pub fn proof(code: &str, fingerprint: &str, client_id: &str) -> Proof {
    let salt = format!("fridae-pair-v1|{}|{}", fingerprint.to_ascii_lowercase(), client_id);
    let key = pbkdf2_sha256(normal_code(code).as_bytes(), salt.as_bytes(), PROOF_ROUNDS);
    Proof { client: hex(&hmac_sha256(&key, b"client")), key }
}

/// Compare two strings in time that does not depend on where they differ.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The SHA-256 of a device token, as the host keeps it (lowercase hex): the token itself is never stored there.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// This computer: `{id, name}`. The id is made once and kept (`identity.json`); the name is the computer's name.
pub fn identity() -> Value {
    let path = dir().join("identity.json");
    let id = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["id"].as_str().map(str::to_string))
        .filter(|s| s.len() == 32)
        .unwrap_or_else(|| {
            let id = random_hex(16);
            let _ = fs::write(&path, json!({"id": id}).to_string());
            id
        });
    json!({"id": id, "name": computer_name()})
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "This computer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_rfc4231_case_2() {
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn hmac_long_key_rfc4231_case_6() {
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn pbkdf2_known_vectors() {
        assert_eq!(hex(&pbkdf2_sha256(b"password", b"salt", 1)), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(hex(&pbkdf2_sha256(b"password", b"salt", 4096)), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
    }

    #[test]
    fn codes_are_ten_letters_from_the_alphabet() {
        for _ in 0..50 {
            let c = new_code();
            assert_eq!(c.len(), CODE_LEN);
            assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        }
        assert_ne!(new_code(), new_code());
    }

    #[test]
    fn codes_are_typed_loosely_and_shown_with_a_dash() {
        assert_eq!(normal_code(" abcde-fghjk "), "ABCDEFGHJK");
        assert_eq!(show_code("abcdefghjk"), "ABCDE-FGHJK");
    }

    #[test]
    fn proof_depends_on_code_fingerprint_and_client() {
        let p = proof("ABCDE-FGHJK", "AA11", "c1").client;
        assert_eq!(p, proof("abcdefghjk", "aa11", "c1").client);
        assert_eq!(p.len(), 64);
        assert_ne!(p, proof("ABCDEFGHJM", "aa11", "c1").client);
        assert_ne!(p, proof("ABCDEFGHJK", "aa12", "c1").client);
        assert_ne!(p, proof("ABCDEFGHJK", "aa11", "c2").client);
    }

    #[test]
    fn one_slow_key_makes_the_clients_proof_and_the_hosts_confirmation() {
        let p = proof("ABCDE-FGHJK", "AA11", "c1");
        let key = pbkdf2_sha256(b"ABCDEFGHJK", b"fridae-pair-v1|aa11|c1", PROOF_ROUNDS);
        assert_eq!(p.client, hex(&hmac_sha256(&key, b"client")));
        let c = p.confirm("tok");
        assert_eq!(c, hex(&hmac_sha256(&key, b"host|tok")));
        // neither gives away the other, nor the key; the confirmation is for this token and this code only
        assert_ne!(c, p.client);
        assert_ne!(p.client, hex(&key));
        assert_ne!(c, p.confirm("tok2"));
        assert_ne!(c, proof("ABCDEFGHJM", "aa11", "c1").confirm("tok"));
    }

    #[test]
    fn same_compares_whole_strings() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }
}
