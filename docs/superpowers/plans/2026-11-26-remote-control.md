# Remote control Implementation Plan

> **For agentic workers:** three tasks built at the same time, each in its own git worktree, against the contract
> below. Each task owns its files; a file another task owns is not changed. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fridae on one computer (the laptop) can run chats on another of the user's computers (the host) on the
same network or over Tailscale, picked per chat in the composer, without managing a second interface.

**Architecture:** The host opens a second, encrypted (TLS) listener on the network when the user turns it on, answers
a UDP "who is there", pairs devices with a one-time code and serves its normal `/api/...` to paired devices. The
client keeps the computers it paired with and forwards the page's calls for a machine (`/m/<machineId>/...`) to it
over a pinned TLS connection. The page adds a machine picker to the composer, a pairing dialog, and shows every
machine's projects and chats together, with a machine label on what is not on this computer.

**Tech Stack:** Rust (app-rs: tiny_http, native-tls/schannel, getrandom, sha2, windows-sys), Preact/TS (ui/src),
the old page ui/index.html.

**Spec:** the user's words in this session (2026-11-26): "set this up on another computer and then not manage a
WHOLE other interface from that computer, but for it to just be reachable from my laptop or my pc interface";
"make it easier for local network connections … the tailscale shit should be implemented tho"; "it should be an
option here in the chat thing to pick which machine im running the message on". Approved design: automatic finding
on the local network, pairing code shown on the host (5 minutes, one use), encrypted connection pinned at pairing,
device list with revoke on the host, "Add by address" for Tailscale names or addresses, machine chip in the
composer (free before the first message, fixed after), sidebar labels for other machines' projects, "+" on a
remote project starts the chat on that machine, offline shown plainly.

## Global Constraints

- Plain, non-technical English in every word the user sees. Clean, quiet UI: no new colours; reuse the existing
  chip, menu, dialog and pill styles (`ui/src/chat/chat.css`, `ui/src/settings/settings.css`, `sidebar.css`).
- No downloads: build with `cargo --offline`. Only crates already in `app-rs/Cargo.lock` or the local registry
  cache (`H:/rust/cargo/registry/cache`) may be added; `native-tls 0.2.18`, `schannel 0.1.30`,
  `windows-sys 0.61.2` (any feature), `getrandom 0.4.3`, `sha2` are there. If a task truly needs another crate,
  stop and report BLOCKED.
- Network access is off by default and turned on only by the user in Settings. `remote_listen` is a guarded
  setting (added to `paths::GUARDED`), so no AI can turn it on by editing the settings file.
- Secrets never printed, logged or put in a URL query: device tokens, pairing codes (except on the host's own
  Settings screen), the page token.
- Remote control's files live in `remote_common::dir()` (the private app-data folder), never under the work folder
  or a project.
- Windows is the target. On other systems the code must still compile; the network parts may answer
  `{"ok":false,"error":"Only on Windows for now."}`.
- Never contact github.com or any outside host in tests. Tests use 127.0.0.1 only.
- Never touch the user's AI configs (~/.claude, ~/.gemini, ~/.cursor, OpenCode), git/gh setup, or Windows
  Credential Manager. Never click OS prompts.
- Build env (Git Bash): `export RUSTUP_HOME=/h/rust/rustup CARGO_HOME=/h/rust/cargo
  PATH=/h/fridae-venv/Scripts:/h/rust/mingw64/bin:/h/rust/cargo/bin:/h/node:$PATH TMP=H:/rust/tmp TEMP=H:/rust/tmp
  PYTHONIOENCODING=utf-8`, each task with its own `CARGO_TARGET_DIR` (given in the dispatch). C: is nearly full:
  everything goes on H:.
- Commits: `git -c user.name=aprilfridae -c user.email=<the email of commit 32da380> commit`, message ending with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never merge, push, or touch other checkouts.

## Shared (already on the base branch)

`app-rs/src/remote_common.rs` — use these, do not change them:
`LISTEN_PORT = 47530`, `DISCOVER_PORT = 47529`, `DISCOVER_ASK = b"FRIDAE?1"`, `CODE_LEN = 10`, `dir()`,
`random(&mut [u8])`, `random_hex(n)`, `hex`, `new_code()`, `normal_code`, `show_code`, `proof(code, fingerprint,
client_id) -> String`, `same(a, b)`, `token_hash(token)`, `identity() -> {id, name}`, `computer_name()`.
`paths::private_dir()`.

## The contract

### Wire: client ⇄ host (HTTPS on the host's network port, TLS with the host's self-signed certificate)

The certificate **fingerprint** is the lowercase hex SHA-256 of the certificate's DER bytes.

Discovery (UDP): the client sends `FRIDAE?1` to `255.255.255.255:47529` (and to each IPv4 interface's
directed broadcast). A host with network access on replies to the sender with one JSON datagram:
`{"fridae":1,"id":<host id>,"name":<computer name>,"port":<tls port>,"fp":<fingerprint>}`.

- `GET /remote/hello` (no auth) → `{"fridae":1,"id","name","version","pairing":bool}` (`pairing`: a code is live).
- `POST /remote/pair` (no auth) body `{"device_id","device_name","proof"}`, `proof = remote_common::proof(code,
  host_fingerprint, device_id)` → `{"ok":true,"token":<32 random bytes, lowercase hex>,"host_id"}` or
  `{"ok":false,"error":<plain words>}`. A code lasts 5 minutes and works once; after 5 wrong proofs it is
  cancelled. The host stores only `token_hash(token)` with `{id: device_id, name, paired_at, last_seen}`. Pairing
  the same device_id again replaces its old entry.
- Any other path, with `Authorization: Bearer <token>` → served exactly as the host's own page would get it
  (`/api/...`, `/img/...`, `/att/...`, `/media/...`), status, content type and body unchanged, `Range` honoured
  for media. Missing/unknown/revoked token → `401 {"error":"not paired"}`. Request body limit as the local server.

### Host's own page API (local server, page token) — Task A

- `GET /api/remote/host` → `{"on":bool,"port":n|null,"id","name","fp","devices":[{"id","name","paired_at",
  "last_seen"}],"code":{"text":"ABCDE-FGHJK","expires":<unix seconds>}|null,"error":<plain words>|null}`
- `POST /api/remote/host` `{"action":"on"|"off"|"code"|"cancel-code"|"revoke","device":<id for revoke>}` → same.

### Client's page API (local server, page token) — Task B

- `GET /api/remote/machines` → `{"this":{"id","name"},"machines":[{"id","name","addr","port","fp","online":bool,
  "last_seen"}]}` (`online` from the last call or check; a background check every 20 s).
- `POST /api/remote/discover` → after ~1.5 s: `{"found":[{"id","name","addr","port","fp","paired":bool}]}`.
- `POST /api/remote/pair` `{"addr","port"?,"code","fp"?}` → connects (port defaults to 47530), reads the
  certificate's fingerprint (if `fp` given from discovery and it differs: refuse), `GET /remote/hello`, sends the
  proof, saves the machine → `{"ok":true,"machine":{...}}` or `{"ok":false,"error":<plain words>}`. `addr` may be
  a name (a Tailscale name such as `desktop` or `desktop.tailnet.ts.net`) or an IP.
- `POST /api/remote/forget` `{"id"}` → `{"ok":true}`.
- Forwarding: any method on `/m/<machineId>/<rest>` (query kept, page token stripped) → the machine's `/<rest>`
  with its Bearer token over TLS pinned to the saved fingerprint; response passed back unchanged; timeout long
  enough for `/api/events?wait=20` (60 s). Unknown machine → `404 {"error":"unknown computer"}`; cannot connect →
  `502 {"error":"offline"}`; fingerprint changed → `502 {"error":"identity changed"}`; `401` from the host →
  machine marked unpaired, `401 {"error":"not paired"}` passed on.
- The token is stored encrypted with Windows DPAPI (`CryptProtectData`, current user) in `machines.json` under
  `remote_common::dir()`.

### Page model — Task C

- A chat on another machine has the page id `<machineId>~<chatId>`; projects from another machine carry
  `machine: <machineId>`, and their paths are that machine's paths. This computer's chats and projects are
  unchanged.

---

### Task A: Host — network access, pairing, devices, Settings

**Files:** Create `app-rs/src/remote_host.rs`, `ui/src/settings/Remote.tsx` (+ its styles in
`ui/src/settings/settings.css`). Modify minimally: `app-rs/src/main.rs` (`mod remote_host;`, start at launch when
`remote_listen` is on), `app-rs/src/server.rs` (route `/api/remote/host`; expose what the forwarder needs to serve
a request as local — keep the change small), `app-rs/src/paths.rs` (`remote_listen` in `GUARDED` and its
`setting_name`: "Let my other computers connect"), `app-rs/Cargo.toml`, `ui/src/settings/Settings.tsx`/`nav.tsx`
(one new section), `ui/app.js`, `ui/app.css` (rebuilt by `python ui/build.py`).

- [ ] Certificate: generated once, kept across restarts, private key protected by Windows (not plaintext on
  disk), e.g. `CertCreateSelfSignedCertificate` + export to a DPAPI-protected PFX in `remote_common::dir()`, loaded
  into `native_tls::Identity`. Tests for: same fingerprint after reload.
- [ ] Listener on `0.0.0.0:47530` (next free up to 47539) with `native_tls::TlsAcceptor`; one thread per
  connection; HTTP/1.1 over the TLS stream (keep-alive optional); serve per the wire contract by passing the
  request to the local server's handling as a local request (e.g. forward to `127.0.0.1:<local port>` with the
  page token, or call the handler directly). Turning off closes the listener and the discovery responder.
- [ ] Discovery responder on UDP 47529 while on.
- [ ] Pairing codes, proofs, devices file (`devices.json`, hashes only), revoke, 5-wrong-tries rule, 5-minute
  expiry. Unit tests for each rule.
- [ ] Integration test on 127.0.0.1: start the listener with a test certificate, `GET /remote/hello`, pair with a
  correct proof (gets a token), a wrong proof (refused), a used code (refused), a Bearer request to an `/api/`
  path (served), a revoked token (401). Use a `native_tls::TlsConnector` with invalid certs accepted and check
  the fingerprint by hand.
- [ ] Settings section "Other computers" → "This computer": a switch "Let my other computers connect" with one
  line underneath ("Your other computers on this network, or on Tailscale, can run chats here after you pair
  them."); while on: "Pair a computer" button showing the code large in a mono face with a countdown and "Cancel";
  this computer's name and port in small text; the device list (name, "paired <date>", "last used <time>",
  "Remove" with a confirm); `error` shown plainly (e.g. "Windows blocked the port …"). First-time note: "Windows
  may ask whether Fridae can use the network. Press Allow." Polls `/api/remote/host` every 2 s while the section
  is open.

### Task B: Client — saved computers, finding, pairing, forwarding

**Files:** Create `app-rs/src/remote_client.rs` (+ `app-rs/tests/` fixtures if needed). Modify minimally:
`app-rs/src/main.rs` (`mod remote_client;`), `app-rs/src/server.rs` (routes `/api/remote/machines|discover|pair|
forget` and the `/m/<id>/` forwarding, which must also stream media responses and pass `Range`),
`app-rs/Cargo.toml`.

- [ ] `machines.json` load/save with DPAPI-protected tokens; unit test round-trip (token never in the file as
  plain text).
- [ ] Discovery: broadcast, collect replies for ~1.5 s, mark `paired` for saved ids, update `addr` of saved
  machines whose id answers from a new address.
- [ ] Pinned TLS client (`native_tls::TlsConnector` accepting the self-signed cert, then comparing
  `peer_certificate()` DER SHA-256 to the pinned fingerprint before sending anything).
- [ ] Pair, forget, forwarding, online check every 20 s.
- [ ] Integration test: a fake host in the test (TLS on 127.0.0.1 with a fixture certificate made once with Git's
  `openssl` and committed under `app-rs/tests/fixtures/`, a test-only certificate) that implements hello/pair/a
  Bearer-checked `/api/echo`; test pair (good and bad code), forwarding (body, status, query, POST body),
  fingerprint change refused, 401 marks the machine unpaired, offline → 502.

### Task C: Page — machine picker, pairing dialog, other machines' chats and projects

**Files:** Create `ui/src/remote/` (store slice, `Machines.tsx` picker, `Pair.tsx` dialog, `remote.ts` routing,
`remote.css`). Modify: `ui/src/chat/Composer.tsx` (the chip), `ui/src/sidebar/model.ts`, `rows.tsx`,
`sidebar.css` (labels, remote boxes), `ui/src/api.ts`/`host.ts`/`store.ts` as needed, `ui/index.html` (event
polling, history, sending and other per-chat calls routed by machine), `ui/src/main.tsx`, `ui/app.js`,
`ui/app.css`. Not Settings (Task A owns it).

- [ ] Routing: a call about a chat `<m>~<id>` goes to `/m/<m>/api/...` with the bare chat id; new chats on
  machine M go to `/m/M/api/new`; projects from M come from `/m/M/api/projects`.
- [ ] Events and history: one long-poll per online machine (`/m/<m>/api/events?after=<its cursor>`), ids
  prefixed; backoff 2 s → 30 s while offline; this computer's polling unchanged.
- [ ] Composer chip left of the AI chip: monitor icon + "This PC" (or the machine's name). Menu: "This PC",
  each paired machine with a small online/offline dot, a divider, "Pair a computer…". Changing it on a draft
  re-reads that machine's AIs/models for the AI chip (`/m/<m>/api/status` or what the AI chip reads). After the
  first message the chip shows the name without the arrow and is not pressable (title: "This chat runs on
  <name>").
- [ ] Pair dialog: "Computers on this network" (from discover, refreshed every 3 s while open; each with
  "Pair"), "Add by address" (one field: "Computer name or address", e.g. a Tailscale name), then a code field
  (`ABCDE-FGHJK`, dash added as typed), errors in plain words, success closes and selects that machine.
- [ ] Sidebar: remote projects appear as boxes like local ones with a small machine label after the name; "+"
  starts a chat on that machine; "No project" chats from a remote machine go under that machine's own "No
  project" box labelled with the machine. Offline machine: its boxes dimmed with "offline" in the label.
- [ ] Tests: a mock server (Python, like `H:/rust/tmp/rvwork-preact/mock5.py`) that serves the page and
  implements `/api/remote/*` and `/m/<id>/...` for one fake remote machine; a Playwright script checking: chip
  default, menu lists the machine, pairing flow (bad code error, good code selects), a message on the remote
  machine posts to `/m/<id>/api/send` (or whatever the page posts for sending) with the bare id, remote events
  show in the chat, chip locked after first message, remote project label and "+", offline dimming. The existing
  suites still pass: `H:/rust/tmp/rvwork-preact/run3.sh` (116), `run2.sh`, `H:/rust/tmp/rvwork-chat/run.sh new
  chat.js` (54), with `NEW_UI` pointed at the task's worktree `ui`.
