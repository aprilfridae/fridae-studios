//! Fridae's own toolbox for the AIs it runs: an MCP server on the app's local server (`POST/DELETE /mcp`) that lets a
//! chat's AI operate the app itself (chats, projects, settings, the theme). A port of T3 Code's app-control MCP
//! server (commit 9ceb8bfb, MIT, Copyright (c) 2026 T3 Tools Inc.): see THIRD_PARTY.md.

pub mod keys;

/// A random UUIDv4 (no `uuid` crate): 16 secure random bytes, version nibble 4, variant bits 10.
pub fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    crate::remote_common::random(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::paths::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}
