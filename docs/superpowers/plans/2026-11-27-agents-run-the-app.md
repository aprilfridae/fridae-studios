# The AI runs Fridae itself (app-control toolbox) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Task 1 is built first, alone.
> Tasks 2, 3, 4 and 5 are then built **at the same time**, each in its own git worktree from the commit Task 1 ends on,
> each owning its files (listed per task); a file another task owns is never changed. Task 6 joins them. Steps use
> checkbox (`- [ ]`) syntax.

**Goal:** In any Fridae chat the user can tell the AI to do things with the app itself (start a chat in a project,
rename a project, move a chat to another AI, change a setting or the theme, stop another chat), and it happens with
the window changing at once, as if the user had clicked it.

**Architecture:** A JSON-RPC MCP server (streamable HTTP, protocol 2025-06-18, stateful sessions, JSON answers only)
lives in the app's existing local server at `POST/DELETE /mcp`, ported from T3 Code's McpHttpServer.ts and Effect's
MCP HTTP transport. Each chat gets an in-memory bearer key (only its SHA-256 kept) naming the chat, its AI and whether
it is a helper. Tools call the same Rust functions the page's routes call, and announce on `agents::BUS` so the page
refreshes itself. Access rules are T3's (McpToolAccess.ts, threadAccess.ts, delegatedTaskModes.ts), plus Fridae's own
Allow/Don't card for invasive changes below Full auto (and always for a few). Each AI CLI that Fridae launches is
handed the server its own way (Claude `--mcp-config` with `${FRIDAE_MCP_KEY}`, Codex `-c` overrides with
`bearer_token_env_var`, OpenCode's per-chat remote server, ACP `session/new`), with `Fridae.exe --mcp-bridge` (a port
of AcpMcpStdioBridge.ts) where only stdio works.

**Tech Stack:** Rust 2024 (app-rs: tiny_http 0.12, serde_json, sha2 0.11, getrandom 0.4, base64 0.23, ureq 3 for the
bridge), Preact/TypeScript (ui/src, bundled by ui/build.py), the old page ui/index.html, Playwright page tests
(tests/remote-page pattern).

**Spec:** `docs/superpowers/specs/2026-11-27-agents-run-the-app-design.md` (binding). Where the spec is silent, T3
Code's source at `H:/t3code-ref` (commit 9ceb8bfb, MIT, Copyright (c) 2026 T3 Tools Inc.) is the authority. Source
notes: `.superpowers/handoff/part1-design.md` … `part4-tools-and-diffs.md`.

## Global Constraints
- **Universal (user rule, 2026-11-27):** every piece works on Windows, macOS and Linux. Anything OS-specific gets a path for each (`cfg(windows)`, `cfg(target_os = "macos")`, Linux via `cfg(target_os = "linux")`/`cfg(unix)`), with no silent do-nothing stubs. Code that cannot be run here must at least compile for each target where the toolchain allows (`cargo check --offline --target x86_64-unknown-linux-gnu` if that target is installed) and be written to the same behaviour.


- Build with `cargo --offline`. **No new crates.** Use what `app-rs/Cargo.toml` already lists: `serde_json`,
  `sha2`, `getrandom`, `base64`, `tiny_http`, `ureq`, `regex`. `serde` derive is in Cargo.lock only transitively: do
  not add it; build JSON with `serde_json::Value`/`json!`. There is no `uuid` crate: make UUIDv4 from 16 random bytes
  (`crate::remote_common::random`, version nibble 4, variant bits 10). If a task truly needs another crate, stop and
  report BLOCKED.
- Build env (Git Bash), every shell:
  `export RUSTUP_HOME=/h/rust/rustup CARGO_HOME=/h/rust/cargo PATH=/h/fridae-venv/Scripts:/h/rust/mingw64/bin:/h/rust/cargo/bin:$PATH TMP=H:/rust/tmp TEMP=H:/rust/tmp PYTHONIOENCODING=utf-8`
- Each worktree uses its own target dir: `cargo test --offline --manifest-path app-rs/Cargo.toml --target-dir 'H:\fridae-target-<name>'`
  with `<name>` = `core` (Task 1), `tools` (2), `wiring` (3), `ui` (4), `credit` (5), `join` (6). C: is nearly full:
  worktrees and everything big go on H: (`H:/fridae-wt/<name>`).
- UI: `python ui/build.py` bundles `ui/src` into `ui/app.js` and `ui/app.css`; both are committed.
- Tests bind `127.0.0.1` only (never `0.0.0.0`), never contact github.com or any outside host, and never start the
  installed `Fridae.exe`. Tests that need a real AI CLI are `#[ignore]` and run only in Task 6, by hand.
- Never modify the user's `~/.claude`, `~/.gemini`, `~/.cursor` or OpenCode config, and never touch the user's own
  `opencode serve` (Fridae's own OpenCode server is fine).
- Never print secrets. A toolbox key never goes in a log, chat text, a BUS event, an error message, a file inside a
  project folder, a file in `WORK/agent-tools`, or git. `Debug` for any type holding a key prints `[redacted]`.
- UI copy: plain English, buttons that say what they do, Fridae's existing look (reuse `.ask`, `.pill`, `.tag`,
  `.steps` styles; no new colours, no gradients, no glows). T3 Code's name, logo or theme never appear in Fridae.
- Commits: `git -c user.name=aprilfridae -c user.email="$(git log -1 --format=%ae 32da380)" commit`, message ending
  with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. No push. Nothing is merged into `main` or any
  remote branch; Task 6 joins the task branches on `feature/app-tools` locally.
- Known flaky test: `swarm::tests::a_conflict_resumes_the_member_whose_branch_conflicts` (passes alone). Re-run it
  alone before calling a run red.
- Fridae vocabulary in agent-facing text: thread → chat, T3 Code → Fridae, provider → AI. Tool descriptions marked
  **exact** below are T3's words with only those swaps and `t3_` dropped; ones marked **adapted** drop or change only
  what names a feature Fridae lacks.

## Review Focus

1. Key secrecy: grep the diff for any `format!`/`println!`/`BUS.put`/`fs::write` that could carry `raw` or
   `Authorization`. Only `keys.rs`, the per-AI wiring (env vars, in-memory ACP/OpenCode bodies) and the bridge touch it.
2. Admission order in `http.rs` matches T3 exactly (Origin 403 → auth 401 → 405 → 415 → 406 → session → parse).
3. Every write tool goes through `access::writing_caller` (live caller), and every cross-chat write through
   `access::target_within`. No tool answers a permission prompt, approves a card, or changes a chat's mode.
4. Kept Claude processes do not restart when a key rotates (`ClaudeLaunch::key()` never contains the key).
5. Nothing of the toolbox is written to a project's `.codex/config.toml` or `.agents/mcp_config.json`.

## Rulings on the spec (made while planning)

1. **Card outcome codes.** T3's failure-code list is kept and three are added for Fridae's own card: `declined`,
   `cancelled`, `timed_out`. Their messages are the spec's notes.
2. **Guarded settings vs project tools.** `projects` is in `paths::GUARDED`, but project tools follow the spec's tool
   table (card below Full auto; always a card for project_delete and adding an existing folder), not the
   "any guarded setting is always a card" rule, which applies to `settings_update` keys.
3. **OpenCode server name.** Fridae already names a chat's servers `fr<8 hex of sha256(chat id)>-<server>` and denies
   all others (`opencode::chat_prefix`, `permissions`). The toolbox uses that: `fr…-fridae` (always under 64 chars),
   which meets the spec's intent (per-chat name, deny others, allow own).
4. **Keys are not revoked at every per-turn process exit.** They are revoked on chat close, AI change, helper-flag
   change and app quit, expire after 24 h idle, and every write needs a live caller anyway. Revoking at each exit would
   rotate the env var every turn for per-message AIs and gain nothing.
5. **Gemini's card wait.** agy cuts any tool call at 3 minutes (measured, `acp::servers_param` docs). A card raised by
   a Gemini chat waits at most 170 s, then closes as `timed_out`.
6. **`chat_wait`'s `next`.** Fridae has no background-completion messages, so `next` is returned only when the waited
   chat is a Studio or overnight Lead (it may never be idle): `{"action":"end_turn","why":"It keeps working on its own.
   Its replies stay in that chat: read them later with chat_read."}`.
7. **refuseAgentModePick** has nothing to refuse in Fridae (no model option sets an agent's own mode); the toolbox has
   no mode field at all. The rule is kept as a test that `chat_configure` rejects `runtimeMode`/`full`/`studio` args.
8. **Modes.** A chat's runtime mode is read live from its last message's switches (`State.last_ask`): `full` or
   `studio` → `full-access` (rank 3), else `approval-required` (rank 0). Autopilot and overnight add no rank. Fridae has
   no plan mode: interaction mode is always `default`.

## Decisions left to the user (the plan builds the safe default)

- **Gemini per-message chats in a project folder get no toolbox.** Today Gemini runs a process per message and reads
  tool servers only from the project's `.agents/mcp_config.json`, which the spec forbids for the toolbox. Default: the
  toolbox reaches Gemini on its kept (ACP) process, and per-message Gemini chats with no project (their workspace is
  Fridae's own folder) get a keyless bridge entry only if Task 3's probe shows agy passes its environment to stdio
  servers. Options for the user: allow a keyless entry in the project file, or make Gemini's kept process the default.
- **Cursor** is not offered on Windows (`paths::AIS`). Its wiring is built and unit-tested; its live check runs on the
  Mac only.

## Shared shapes (pinned; every task uses these exactly)

- Endpoint: `http://127.0.0.1:<port>/mcp`, port from `server::local()`.
- Env vars: `FRIDAE_MCP_KEY` (raw key, for `${…}` expansion and Codex `bearer_token_env_var`),
  `FRIDAE_MCP_ENDPOINT`, `FRIDAE_MCP_AUTHORIZATION` (`Bearer <raw>`), `FRIDAE_MCP_TOOL_EXPOSURE` (`deferred-proxy`).
- Header for deferred tools: `fridae-tool-exposure: deferred-proxy`.
- Server name `fridae`; Codex adds `fridae-direct`; OpenCode `fr<8hex>-fridae`. Name forms the page recognises:
  `mcp__fridae__<tool>`, `mcp__fridae-direct__<tool>`, `fridae.<tool>`, `fridae__<tool>`, `fridae-direct.<tool>`,
  `fr<8hex>-fridae_<tool>`.
- Chat id: Fridae's agent id (`c<ms>-<n>`). Project id: `"p" + first 12 hex of sha256(norm(path))` where `norm` is
  `projects::same`'s normalisation (`/`→`\`, trailing `\` trimmed, lower case). `app_mcp::project_id(path)`.
- Links (markdown, quoted verbatim by agents, drawn as chips): `[<title>](fridae://chat/<chatId>)` and
  `[<name>](fridae://project/<projectId>)`; `[`, `]` and `\` in the title are escaped with `\`.
- Run id: `"r" + turn_started * 1000 as u64` (`None` before a chat's first turn).
- Modes on the wire: `runtimeMode` `"approval-required" | "full-access" | "inherit"`; `interactionMode`
  `"default" | "inherit"`.
- BUS events (new):
  - `appcard` `{agent, card, title, text, allow}` and `appcard_done` `{agent, card, title, outcome}` with outcome
    `allowed|declined|cancelled|timed_out` (both via `Agent::put`, so they are in the chat's log).
  - `apptool` `{agent, call, tool, status, label, summary:{verb, one, many}, links:[{kind:"chat"|"project", id,
    title}]}` with status `running|done|failed|declined` (via `Agent::put`; one call id, several events).
  - `projects_changed` `{}`, `settings_changed` `{keys:[…]}`, `accounts_changed` `{}`,
    `theme_set` `{themeId?, appearance?}` (via `BUS.put`).
- Page routes (new): `POST /api/app-card {agent, card, answer:"allow"|"decline"}` → `{ok}`;
  `POST /api/theme-now {id, mode}` → `{ok}`.
- Project descriptions (`projects::describe`) gain `"id": app_mcp::project_id(path)`.

---

## Task 1: Shared core (alone, first)

Worktree `H:/fridae-wt/core`, branch `app-tools/core` from `feature/app-tools`. Target dir `H:\fridae-target-core`.

**Files (owned):**
- Create: `app-rs/src/app_mcp/mod.rs`, `http.rs`, `keys.rs`, `access.rs`, `card.rs`, `events.rs`
- Create stubs (handed over): `app-rs/src/app_mcp/tools/mod.rs` (→ Task 2), `exposure.rs`, `bridge.rs`,
  `instructions.rs` (→ Task 3), `e2e_tests.rs` (→ Task 5)
- Modify: `app-rs/src/main.rs` (one line `mod app_mcp;` after `mod agents;`), `app-rs/src/server.rs` (route table),
  `app-rs/src/remote_host.rs` (`kept_from_devices`), `app-rs/src/projects.rs` (`describe` gains `id`),
  `app-rs/src/agents.rs` (one accessor block, one insertion point, below)

**Interfaces Produces** (used by Tasks 2–5; signatures pinned):

```rust
// app_mcp/mod.rs
pub const SERVER_NAME: &str = "fridae";
pub const PROTOCOL: &str = "2025-06-18";
pub fn endpoint() -> Option<String>;                       // "http://127.0.0.1:<port>/mcp"
pub fn project_id(path: &str) -> String;
pub fn chat_link(chat_id: &str, title: &str) -> String;
pub fn project_link(project_id: &str, name: &str) -> String;
pub fn run_id(turn_started: f64) -> Option<String>;
pub struct Hints { pub read_only: bool, pub destructive: bool, pub idempotent: bool, pub open_world: bool }
pub enum Access { Reads, ReadsAsCaller, WritesThreads, StartsThreads, WritesEnvironment }
pub struct Labels { pub action: &'static str, pub running: &'static str, pub done: &'static str, pub detail: &'static str }
pub struct Summary { pub verb: &'static str, pub one: &'static str, pub many: &'static str }
pub struct ToolDef {
    pub name: &'static str, pub title: &'static str, pub description: &'static str,
    pub schema: fn() -> Value,                              // JSON Schema object of the arguments
    pub hints: Hints,
    pub access: fn(&Map<String, Value>) -> Access,          // per call (chat_configure: read vs set)
    pub labels: fn(&Map<String, Value>) -> (Labels, Summary), // per call (action tools)
    pub always_load: bool,                                  // _meta {"anthropic/alwaysLoad": true}
    pub handler: fn(&Call, &Map<String, Value>) -> Outcome,
}
pub struct Call<'a> { pub scope: &'a keys::Scope, pub call_id: &'a str, pub cancelled: &'a AtomicBool, pub exposure: exposure::Mode }
impl Call<'_> { pub fn links(&self, links: Vec<Value>); }   // chips for this call's apptool event
pub struct Failure { pub code: &'static str, pub message: String }
pub type Outcome = Result<Value, Failure>;
pub fn fail(code: &'static str, message: impl Into<String>) -> Failure;
pub const CODES: [&str; 19];   // T3's 16 + "declined", "cancelled", "timed_out"
pub fn ok_result(v: &Value) -> Value;     // {isError:false, structuredContent:v, content:[{type:"text",text:v.to_string()}]}
pub fn fail_result(f: &Failure) -> Value; // {isError:true, content:[{type:"text",text:"{\"_tag\":\"FridaeToolFailure\",\"code\":…,\"message\":…}"}]}
pub fn crash_result() -> Value;           // {isError:true, content:[{type:"text",text:"Tool execution failed due to an internal server error."}]}
pub fn check_args(def: &ToolDef, args: &Map<String, Value>) -> Result<Map<String, Value>, String>; // nulls dropped
pub fn registry() -> Vec<ToolDef>;        // tools::defs() then exposure::defs()
pub fn call_tool(scope: &keys::Scope, name: &str, args: Value, cancelled: &AtomicBool, exposure: exposure::Mode) -> Result<Value, (i64, String)>;
pub struct Wire { pub url: String, pub raw: String, pub deferred: bool } // Debug = "[redacted]"
impl Wire { pub fn authorization(&self) -> String; }      // "Bearer <raw>"
pub fn wire_for(chat: &str, ai: &str, subagent: bool) -> Option<Wire>; // keys::prepare + endpoint; None if server not up
#[cfg(test)] pub fn set_endpoint_for_test(url: Option<String>);        // endpoint() returns it in tests

// keys.rs
pub struct Scope { pub chat: String, pub ai: String, pub subagent: bool, pub namespace: String }
pub struct Credential { pub raw: String, pub scope: Scope }           // Debug redacts raw
pub enum Refusal { Missing, Scheme, Empty, Unknown, Expired }
pub const IDLE: Duration;                                             // 24 h
pub struct Registry;                                                  // clock injectable for tests
impl Registry {
    pub fn new(clock: fn() -> (Instant, SystemTime)) -> Registry;
    pub fn prepare(&self, chat: &str, ai: &str, subagent: bool) -> Credential;
    pub fn resolve(&self, raw: &str) -> Result<Scope, Refusal>;
    pub fn touch(&self, chat: &str);
    pub fn revoke_chat(&self, chat: &str);
    pub fn revoke_all(&self);
    pub fn hashes_for_test(&self) -> Vec<String>;                    // #[cfg(test)]
}
pub fn global() -> &'static Registry;
pub fn bearer(header: &str) -> Result<&str, Refusal>;                // scheme case-insensitive

// access.rs
pub enum Runtime { ApprovalRequired, FullAccess }   // rank 0 / 3; as_str "approval-required" / "full-access"
pub enum Interaction { Default }                    // as_str "default"
pub struct Limits { pub runtime: Runtime, pub interaction: Interaction }
pub struct Caller { pub agent: Arc<agents::Agent>, pub scope: keys::Scope, pub limits: Limits }
pub fn limits_of(agent: &agents::Agent) -> Limits;
pub fn read_caller(scope: &keys::Scope) -> Result<Caller, Failure>;
pub fn writing_caller(scope: &keys::Scope) -> Result<Caller, Failure>;
pub fn target_within(caller: &Caller, target: &agents::Agent) -> Result<(), Failure>;
pub fn resolve_runtime(parent: Runtime, requested: Option<&str>) -> Result<Runtime, Failure>;
pub fn resolve_interaction(parent: Interaction, requested: Option<&str>) -> Result<Interaction, Failure>;
pub fn resolve_chat(caller: &Caller, given: Option<&str>, default_this: bool) -> Result<Arc<agents::Agent>, Failure>;
pub fn chat_not_found(given: &str) -> Failure;
pub fn environment_gate(call: &Call, caller: &Caller, card: card::CardSpec, always: bool) -> Result<(), Failure>;

// card.rs
pub struct CardSpec { pub title: String, pub text: String, pub allow: String }
pub enum Answer { Allowed, Declined, Cancelled, TimedOut }
pub const POLL: Duration;        // 500 ms
pub const WAIT: Duration;        // 600 s
pub const WAIT_GEMINI: Duration; // 170 s
pub fn ask(agent: &Arc<agents::Agent>, ai: &str, spec: CardSpec, cancelled: &AtomicBool) -> Answer;
pub fn answer(agent_id: &str, card_id: &str, allow: bool) -> bool;
pub fn failure_of(a: &Answer) -> Option<Failure>;

// events.rs
pub fn projects_changed();
pub fn settings_changed(keys: &[&str]);
pub fn accounts_changed();
pub fn theme_set(theme_id: Option<&str>, appearance: Option<&str>);
pub fn report_theme(body: &Value) -> Value;      // /api/theme-now
pub fn theme_now() -> Option<(String, String)>;  // (themeId, mode) as the window last said

// exposure.rs (stub; Task 3 fills)
#[derive(Clone, Copy, PartialEq, Debug)] pub enum Mode { Full, DeferredProxy }
pub fn mode_of(header: &str) -> Mode;            // "deferred-proxy" (trimmed, any case) → DeferredProxy
pub fn listed(mode: Mode, name: &str) -> bool;   // stub: name != "tool_search" && name != "tool_call"
pub fn defs() -> Vec<ToolDef>;                   // stub: vec![]
// tools/mod.rs (stub; Task 2 fills):  pub fn defs() -> Vec<ToolDef> { vec![] }
// bridge.rs (stub; Task 3 fills):     pub fn main() -> i32 { 2 }
// instructions.rs (stub; Task 3):     pub fn sections(deferred: bool) -> Vec<&'static str> { vec![] }
```

`agents.rs` accessor block (the only Task 1 edit there), inserted as a new `impl Agent { … }` immediately above the
doc comment `/// Start another chat the user can give its own task to.` (before `pub fn new_agent`):

```rust
pub fn last_ask(&self) -> Ask;                        // st.last_ask.clone()
pub fn is_subagent(&self) -> bool;                    // is_helper(&st)
pub fn is_closed(&self) -> bool;                      // st.closed
pub fn is_working(&self) -> bool;                     // working(&st) || st.making
pub fn studio_lead_loop(&self) -> bool;               // st.overnight_until > now || st.last_ask.studio
pub fn log_snapshot(&self) -> Vec<Value>;             // st.log.clone()
pub fn queue_items(&self) -> Vec<(String, String)>;   // (qid, text)
pub fn set_mode_for_launch(&self, full: bool);        // st.last_ask.full = full (an idle launched chat keeps its mode)
pub fn parent_id(&self) -> Option<String>;            // st.parent (pub fn parent() may exist: reuse if so)
#[cfg(test)] pub fn set_last_ask_for_test(&self, ask: Ask);
#[cfg(test)] pub fn set_ai_for_test(&self, ai: &str);
```

### 1.1 Keys

- [ ] **Failing tests** in `keys.rs` (`#[cfg(test)] mod tests`, a `Registry` with a test clock in a `static Mutex`):
  - `stores_only_a_hash_and_resolves_the_bearer`: `prepare("c1","claude",false)`; `raw` decodes from base64url-no-pad
    to 32 bytes; `hashes_for_test()` == `[hex(sha256(raw))]` and contains no `raw`; `resolve(&raw)` → scope chat
    `c1`, ai `claude`; `resolve("nope")` → `Unknown`.
  - `reuses_while_the_scope_is_the_same`: two `prepare("c1","claude",false)` give the same `raw`;
    `prepare("c1","gemini",false)` gives a new one and the old raw → `Unknown`; same for the subagent flag.
  - `expires_after_a_day_idle_and_touch_revives`: advance both clocks 24 h + 1 s → `Expired`; `touch("c1")` →
    resolves again with the original scope.
  - `a_backward_wall_clock_neither_extends_nor_revives`: move wall clock back 2 h, monotonic forward 24 h + 1 s →
    `Expired` (idle = max of the two elapsed).
  - `touch_keeps_only_that_chat_alive`, `revoke_chat_and_revoke_all`.
  - `bearer_parses_case_insensitively`: `"bearer abc"` → `abc`; `"Basic x"` → `Scheme`; `""` → `Missing`;
    `"Bearer "` → `Empty`.
  - `debug_never_prints_the_key`: `format!("{:?}", cred)` does not contain `cred.raw`.
- [ ] Run: `cargo test --offline --manifest-path app-rs/Cargo.toml --target-dir 'H:\fridae-target-core' app_mcp::keys` — Expected: fails to compile (module missing).
- [ ] Implement `keys.rs`. Pinned: 32 bytes from `getrandom`, `base64::engine::general_purpose::URL_SAFE_NO_PAD`,
  map key `paths::hex(&Sha256::digest(raw))`, record `{scope, issued: (Instant, SystemTime), alive: (Instant,
  SystemTime), expired: bool}`; `namespace` = a UUIDv4 minted at issue; `prepare` is serialised per chat by one
  `Mutex` around the whole map; raw keys of live credentials are held in a second in-memory map `chat → (Scope, raw)`
  (the role of T3's ProviderSessionManager), never written anywhere. `global()` is a `LazyLock<Registry>` on real clocks.
- [ ] Run the same command — Expected: all `app_mcp::keys` tests pass.
- [ ] Commit `app_mcp: per-chat toolbox keys (port of McpSessionRegistry)`.

### 1.2 Results, argument checks, ids and links

- [ ] **Failing tests** in `mod.rs`:
  - `ok_and_fail_shapes`: `ok_result(&json!({"a":1}))` == `{"isError":false,"structuredContent":{"a":1},"content":[{"type":"text","text":"{\"a\":1}"}]}`;
    `fail_result(&fail("thread_not_found","x"))["content"][0]["text"]` parses to
    `{"_tag":"FridaeToolFailure","code":"thread_not_found","message":"x"}` and has no `structuredContent`.
  - `unknown_arguments_are_named`: a def with properties `a, b, c`; args `{a:1, zz:2}` →
    `Err("Unknown argument \"zz\". Accepted arguments: a, b, c.")`; six unknown args name five then
    `" and 1 more"`; plural form `Unknown arguments "x", "y".`; argument names cut at 64 chars.
  - `null_optional_fields_are_dropped`: `{a:null}` → `Ok({})`.
  - `project_id_is_stable_across_spellings`: `project_id("C:/Games/Shop/")` == `project_id("c:\\games\\shop")`,
    starts with `p`, length 13.
  - `links_escape_brackets`: `chat_link("c1-2","a [b]")` == `"[a \\[b\\]](fridae://chat/c1-2)"`.
  - `run_id`: `run_id(1732.5)` == `Some("r1732500")`, `run_id(0.0)` == `None`.
  - `wire_debug_is_redacted`.
- [ ] Run `… app_mcp::tests` — Expected: compile failure.
- [ ] Implement. `CODES` = `capability_denied, parent_not_active, provider_unavailable, model_unavailable,
  runtime_mode_escalation_denied, interaction_mode_escalation_denied, task_not_found, task_not_cancellable,
  thread_not_found, run_not_found, thread_not_sendable, thread_not_interruptible, invalid_request,
  orchestration_error, thread_credential_required, target_required, declined, cancelled, timed_out`.
  `check_args`: accepted = keys of `schema()["properties"]`; acceptedText `"none"` when empty.
- [ ] Run — Expected: pass. Commit `app_mcp: result shapes, argument checks, ids and links`.

### 1.3 Access rules (port of McpToolAccess.ts, threadAccess.ts, delegatedTaskModes.ts)

Pinned messages (agent-facing):
- missing caller: `thread_not_found` "The calling chat was not found."
- not live: `parent_not_active` "The calling AI no longer owns an active chat run."
- target above caller: `runtime_mode_escalation_denied` `Chat "{id}" runs in {rt}/{im}, above this caller's {crt}/{cim} limit. This call does not create an approval request. Have the user act directly in the target chat.`
- launch broader: `runtime_mode_escalation_denied` `Child runtime mode {resolved} is broader than parent mode {parent}.`; interaction likewise with `interaction_mode_escalation_denied` `Child interaction mode {resolved} is broader than parent mode {parent}.`
- bad mode value: `invalid_request` `Unknown runtime mode "{v}". Use approval-required, full-access or inherit.`
- chat miss: `thread_not_found` `The chat was not found.` + ` Did you mean \`{id}\`?` when an open or saved chat id
  equals the given one trimmed of whitespace/backticks or of a `c` prefix difference; a given
  `fridae://chat/<id>` or `[..](fridae://chat/<id>)` → `` `{given}` is a chat link, not a chat id. Pass `{id}` instead. ``
- live caller = `!agent.is_closed() && agent.ai() == scope.ai && agent.is_working()`.

- [ ] **Failing tests** in `access.rs` (chats made with `agents::new_agent(Some("claude"))`, `set_running_for_test`,
  `set_last_ask_for_test`; each test removes its chats at the end):
  - `ranks_match_t3`: FullAccess rank 3 > ApprovalRequired 0.
  - `resolve_runtime_inherits_and_never_widens`: parent full + None/"inherit" → full; parent approval + "full-access"
    → `runtime_mode_escalation_denied` with the pinned message; parent full + "approval-required" → approval.
  - `a_target_above_the_caller_is_refused_with_t3s_words`: caller approval, target full → message contains
    `above this caller's approval-required/default limit` and `This call does not create an approval request.`
  - `writes_need_a_live_caller`: idle caller → `parent_not_active`; running → Ok; closed (`remove_agent`) → refused;
    key AI `gemini` while chat AI `claude` → refused.
  - `reads_need_no_live_caller`: idle caller `read_caller` → Ok.
  - `chat_miss_suggests_the_id` and `a_chat_link_is_not_an_id`.
  - `default_this_only_when_asked`: `resolve_chat(caller, None, true)` → caller; `(None, false)` →
    `target_required` "Pass chatId."
- [ ] Run `… app_mcp::access` — Expected: compile failure.
- [ ] Implement (environment_gate after 1.4).
- [ ] Run — Expected: pass. Commit `app_mcp: access rules (port of McpToolAccess/threadAccess)`.

### 1.4 The Allow/Don't card (mechanism from T3's request_secret)

- [ ] **Failing tests** in `card.rs` (`POLL`/`WAIT` read through `#[cfg(test)]` overrides of 20 ms / 300 ms):
  - `allow_returns_allowed_and_logs_both_events`: thread A calls `ask`; test reads the chat's log for `appcard`
    (title, text, allow, 32-hex `card`), calls `answer(agent, card, true)` → `true`; `ask` → `Allowed`; log then has
    `appcard_done` outcome `allowed`.
  - `a_card_is_bound_to_its_chat`: `answer("other-chat", card, true)` → `false`, card still pending.
  - `decline`, `the_run_ending_cancels` (`set_running_for_test(false)` → `Cancelled`, outcome `cancelled`),
    `mcp_cancel_cancels` (`cancelled.store(true)`), `timing_out_is_not_declining` (→ `TimedOut`, outcome `timed_out`),
    `gemini_waits_less` (`ai == "gemini"` uses `WAIT_GEMINI`).
  - `failure_notes`: `failure_of(&Declined)` = code `declined`, message "The user declined. Don't ask again in chat; say what's blocked and go on without it.";
    `Cancelled` "The card closed with the run. Ask again with a new clientRequestId only if it is still needed.";
    `TimedOut` "No answer in time. Don't ask in chat; ask again with a new clientRequestId only if it is still needed.";
    `Allowed` → None.
- [ ] Then in `access.rs`: `environment_gate_full_auto_needs_no_card` (caller full, `always=false` → Ok, no
  `appcard` in log), `environment_gate_below_full_auto_asks`, `always_asks_even_in_full_auto`.
- [ ] Run `… app_mcp::card app_mcp::access` — Expected: fail.
- [ ] Implement. Pending cards: `Mutex<HashMap<card_id, (agent_id, Option<bool>)>>`; card id = 16 random bytes hex;
  `ask` puts `appcard`, polls every `POLL`: answered → outcome; `!agent.is_working() || agent.is_closed()` →
  Cancelled; `cancelled` flag → Cancelled; elapsed ≥ wait → TimedOut; always removes the entry and puts
  `appcard_done {agent, card, title, outcome}`.
- [ ] Run — Expected: pass. Commit `app_mcp: Allow/Don't card that holds the call`.

### 1.5 Events, theme report, dispatcher and apptool events

- [ ] **Failing tests** in `mod.rs` / `events.rs`, with a test tool registered through `#[cfg(test)] fn test_defs()`
  appended by `registry()` under `cfg(test)`:
  - `call_tool_puts_running_then_done_with_labels`: a Reads tool with labels `["Read","Reading","Read","a chat"]` →
    the caller's log gets `apptool` `{status:"running", label:"Reading a chat"}` then `{status:"done",
    label:"Read a chat", links:[…]}` with one `call` id; result is `ok_result`.
  - `failed_and_declined_forms`: handler `Err(fail("invalid_request",…))` → `{status:"failed", label:"Failed to read a chat"}`;
    `Err(code "declined")` → `{status:"declined", label:"Declined to read a chat"}`.
  - `a_panic_is_an_internal_error`: handler panics → `crash_result()` (via `catch_unwind`), event `failed`.
  - `unknown_tool_is_method_not_found`: `Err((-32601, "Unknown tool: nope"))`.
  - `bad_args_are_invalid_params`: `Err((-32602, "Unknown argument …"))`.
  - `client_request_id_replays_within_the_namespace`: a counting StartsThreads tool called twice with the same
    `clientRequestId` → handler ran once, same result; another scope namespace → runs again. Applies to
    `chat_launch, chat_send, chat_interrupt, chat_update` by name (`IDEMPOTENT`), cached 1 h, at most 1024 entries.
  - `writes_check_the_live_caller_before_the_handler`: idle caller + WritesThreads tool → `parent_not_active`, handler
    not run.
  - `theme_report_round_trips`: `report_theme(&json!({"id":"moss","mode":"dark"}))` → `{ok:true}`;
    `theme_now()` == `Some(("moss","dark"))`; unknown id or mode → `{ok:false}`, unchanged. Ids:
    `fridae, graphite, moss, tide, clay, lilac` (index.html `THEME_ORDER`); modes `light, dark, system`.
  - `events_put_on_the_bus`: `projects_changed()` → last BUS item kind `projects_changed`.
- [ ] Run — Expected: fail.
- [ ] Implement. Labels: running = `"{running} {detail}"`, done = `"{done} {detail}"`, failed = `"Failed to {action lower-first} {detail}"`,
  declined = `"Declined to {action lower-first} {detail}"`. The apptool event goes to the **calling** chat via
  `agents::get(scope.chat)`.
- [ ] Run — Expected: pass. Commit `app_mcp: dispatcher, idempotency, apptool and app events`.

### 1.6 HTTP transport and admission (port of McpHttpServer.ts + Effect McpServer.layerHttp)

`http.rs` exposes `pub fn serve(req: tiny_http::Request)` and, for tests, `pub(crate) fn handle_parts(method: &str,
headers: &[(String, String)], body: &[u8]) -> Reply` with `pub(crate) struct Reply { pub status: u16, pub headers:
Vec<(String, String)>, pub body: Vec<u8> }`. Sessions: `HashMap<id, Session{key_hash, protocol, exposure,
last_used}>`, at most 512 (oldest dropped), a session answers only the key that made it (else 404). Body cap
4,000,000 bytes (413 above). Every answer carries `cache-control: no-store`.

- [ ] **Failing tests** (`http.rs` tests; a key from `keys::global().prepare` for a test chat):
  - `origin_is_refused_before_credentials`: any `Origin` header, no auth → 403, empty body.
  - `missing_key_401_with_t3s_shape`: body `{"error":"invalid_mcp_credential","message":"A valid Fridae toolbox key is required."}`,
    `www-authenticate: Bearer`; a wrong key → `www-authenticate: Bearer error="invalid_token"`.
  - `the_page_token_never_works`: `X-Token: <page token>` and `?t=<page token>` without Bearer → 401.
  - `methods`: GET/PUT/PATCH/OPTIONS with a good key → 405 `allow: POST, DELETE`.
  - `content_type_415` (POST `text/plain`), `accept_406` (Accept `application/json` only; and missing).
  - `initialize_mints_a_session`: 200, `mcp-session-id` is a UUIDv4, `mcp-protocol-version: 2025-06-18`, result
    `{protocolVersion:"2025-06-18", capabilities:{tools:{listChanged:false}}, serverInfo:{name:"fridae", version:VERSION}}`;
    an unknown offered version is answered with `2025-06-18`.
  - `initialize_with_a_session_id_400`: `{"jsonrpc":"2.0","id":…,"error":{"code":-32600,"message":"initialize must not include an MCP-Session-Id header"}}`.
  - `no_session_400`: "MCP-Session-Id header is required; send initialize to start a session".
  - `unknown_session_404_empty`, `another_keys_session_404`.
  - `bad_protocol_header_400`: `{"code":"UNSUPPORTED_PROTOCOL_VERSION","message":"Unsupported protocol version '1999-01-01'","data":{"supported":["2025-06-18"],"requested":"1999-01-01"}}`.
  - `parse_error_is_200_with_32700`, `batches_400` (empty and non-empty), `notification_only_202`,
    `ping_answers_empty_object`.
  - `delete_ends_the_session`: DELETE → 204; next POST with that id → 404.
  - `tools_list_lists_names_schemas_annotations`: each tool `{name, title, description, inputSchema, annotations:
    {title, readOnlyHint, destructiveHint, idempotentHint, openWorldHint}}`, `_meta {"anthropic/alwaysLoad":true}`
    when `always_load`; `tool_search`/`tool_call` absent without the exposure header (stub rule).
  - `tools_call_round_trip` with the test tool; `notifications/cancelled` with `requestId` sets that call's
    `cancelled` flag (test tool waits on it).
  - `body_too_large_413`.
  - One socket test `serves_over_a_real_port`: `tiny_http::Server::http("127.0.0.1:0")`, a thread calling `serve`,
    `ureq` client: initialize → tools/list → DELETE.
- [ ] In `server.rs` tests: `a_chat_key_never_opens_page_routes` — `Authorization: Bearer <key>` on `GET /api/status`
  without the page token → 403.
- [ ] Run `… app_mcp::http server::` — Expected: fail.
- [ ] Implement `http.rs` (admission order exactly: Origin → bearer/resolve (log the `Refusal` reason only, to the
  existing `FRIDAE_LOG` file when set) → method → content-type → accept → session/protocol → JSON parse → batch →
  dispatch). Exposure mode from `fridae-tool-exposure` on initialize, kept on the session.
- [ ] `server.rs`: at the top of `handle`, before the token check:
  `if path == "/mcp" { if !is_local(&req) { let _ = req.respond(forbidden()); return; } crate::app_mcp::http::serve(req); return; }`.
  In `post`: `"/api/app-card" => send_json(json!({"ok": crate::app_mcp::card::answer(&text_of("agent"), &text_of("card"), text_of("answer") == "allow")})),`
  and `"/api/theme-now" => send_json(crate::app_mcp::events::report_theme(&body)),` (both beside `/api/rename`).
- [ ] `remote_host.rs` `kept_from_devices`: add `|| p == "/mcp" || p.starts_with("/mcp?")`, with a test line in its
  existing devices test asserting `/mcp` → 403.
- [ ] `projects.rs` `describe`: add `"id": crate::app_mcp::project_id(path)`; test `describe_has_an_id`.
- [ ] Run the whole suite `cargo test --offline --manifest-path app-rs/Cargo.toml --target-dir 'H:\fridae-target-core'` — Expected: all pass (bar the known flaky one).
- [ ] Commit `app_mcp: streamable HTTP /mcp with T3's admission order`.

---

## Parallel tasks 2–5: file ownership

| File | Owner |
|---|---|
| `app-rs/src/app_mcp/tools/**` | Task 2 |
| `app-rs/src/app_mcp/exposure.rs`, `bridge.rs`, `instructions.rs` | Task 3 |
| `app-rs/src/agents.rs`, `codex.rs`, `acp.rs`, `cursor.rs`, `opencode.rs`, `antigravity.rs`, `main.rs` | Task 3 |
| `ui/**`, `tests/app-tools-page/**` | Task 4 |
| `THIRD_PARTY.md`, `licenses/**`, `app-rs/src/app_mcp/e2e_tests.rs` | Task 5 |
| `app_mcp/mod.rs`, `http.rs`, `keys.rs`, `access.rs`, `card.rs`, `events.rs`, `server.rs`, `projects.rs`, `remote_host.rs` | frozen (Task 1); a needed change is reported to the controller, not made |

Task 2 needs no edit outside `tools/`: everything it calls is public already or in Task 1's accessor block.

---

## Task 2: Chat, project, settings, theme and capability tools (parallel)

Worktree `H:/fridae-wt/tools`, branch `app-tools/tools`. Target dir `H:\fridae-target-tools`.

**Files:** Create `app-rs/src/app_mcp/tools/{mod.rs (replace stub), caps.rs, chats.rs, projects.rs, settings.rs, theme.rs}`.

**Interfaces Consumes:** Task 1's `ToolDef`, `Call`, `access::*`, `card::CardSpec`, `events::*`, links/ids;
`agents::{agents, get, new_agent, history, open_chat, remove_agent, Agent::{ask, stop, rename, unqueue, set_project,
set_pick, switch_ai, pick_account, ai, name, project, busy, queued, turn_started, last_reply}, EFFORTS, Ask}`,
`studio_undo::pause_if_lead`, `projects::{list, describe, create, add, rename, set_kind, forget, known}`,
`paths::{AIS, default_ai, update_config, find_ai, gemini_api_on, gemini_images_on, expert_summaries_on,
strong_lead_on, setting_name, VERSION}`, `expert_ai::LEAD_PICKS`, `swarm::STRONG_LEAD_KEY`, `update::{state, set_auto}`,
`codex/cursor/opencode/antigravity::{models, default_model}`, `accounts::state`.

**Interfaces Produces:** `tools::defs() -> Vec<ToolDef>` with exactly these 21 names, in this order:
`fridae_capabilities, chat_list, chat_search, chat_read, chat_launch, chat_send, chat_wait, chat_interrupt,
chat_update, chat_organize, chat_configure, chat_queue, project_list, project_read, project_create, project_update,
project_delete, settings_read, settings_update, theme_read, theme_set`.

### Tool table (descriptions, hints, labels, access) — pinned

Hints default (Effect): `read_only false, destructive !read_only, idempotent false, open_world true`; listed values override.

| Tool | Title | Description | Hints | Labels [action, running, done, detail] · summary | Access |
|---|---|---|---|---|---|
| fridae_capabilities | Get AIs and models | **adapted** "List AIs, models, efforts, accounts, and runtime modes for `chat_launch` and `chat_configure`. Short list. `search` finds others. Any catalog model works." | RO, idem, alwaysLoad | Get/Getting/Got/AIs and models · Got,AI list,AI lists | Reads |
| chat_list | List chats | **exact** "List chats newest first, in one project or with `allProjects`. Page with `cursor`." | RO, idem | List/Listing/Listed/chats · Listed,chat list,chat lists | Reads |
| chat_search | Search chats | **adapted** "Search active chats by title, project, then message text. Skips subagent chats. `projectId`: one project, else yours, else all outside a chat. Not exhaustive." | RO | Search/Searching/Searched/chats · Searched,search,searches | Reads |
| chat_read | Read a chat | **adapted** "Read any chat by id. View `messages` (default) or `activity` (all items). Newest first; older: `beforePosition=earlierPosition`; from the start: `afterPosition=-1`, then `nextPosition`. Cut-off text: `itemId`, `textOffset=nextTextOffset`. In your reply, name it as its `link` verbatim." | RO false (T3), idem | Read/Reading/Read/a chat · Read,chat,chats | Reads |
| chat_launch | Start a chat | **adapted** "Create a top-level chat in its workspace before starting. Task in `message`. Omit `projectId`, model and modes to inherit yours (`scratch` for no project); modes cannot exceed yours. On error or lost reply, reuse `clientRequestId`. Follow it with `chat_read` or `chat_wait`. Mention it with its `link`." | destructive, openWorld | Start/Starting/Started/a chat · Started,chat,chats | StartsThreads |
| chat_send | Send to a chat | **adapted** "Send a message to any chat. The target cannot run with broader modes than yours. `mode:'auto'` starts an idle chat or queues behind a running turn. `restart`: stop the running turn and start with this message. In your reply, name it as its `link` verbatim." | destructive, openWorld | Send/Sending/Sent/a message to a chat · Sent,message,messages | WritesThreads |
| chat_wait | Wait for a chat | **exact** "Wait for a chat's run to finish. No `runId`: latest. Idle: at once. On timeout, with `next` end your turn; without, call again." | RO, idem | Wait/Waiting/Waited/for a chat · Waited for,chat,chats | Reads |
| chat_interrupt | Stop a chat | **adapted** "Interrupt a top-level chat's running turn. No `runId`: newest running turn. No effect when idle." | destructive | Stop/Stopping/Stopped/a chat · Stopped,chat,chats | WritesThreads |
| chat_update | Rename a chat | **adapted** "Rename a chat (`action:'rename'` with `title`). Omit `chatId` for this chat. Its project cannot change here." | destructive | Rename/Renaming/Renamed/a chat · Renamed,chat,chats | WritesThreads |
| chat_organize | Organize a chat | **adapted** "Close (archive) or reopen (unarchive) a chat. Omit `chatId` for this chat. Closing this chat applies at turn end. A closed chat stays in your saved chats." | destructive | archive: Close/Closing/Closed/a chat · Closed; unarchive: Reopen/Reopening/Reopened/a chat · Reopened | WritesThreads |
| chat_configure | Configure a chat | **adapted** "Read a chat's AI, model, and modes, or set its AI, model, effort, and account with `modelSelection`. Omit `chatId` for this chat. Permission modes stay. Values from `fridae_capabilities`." | destructive | read: Read/Reading/Read/a chat's settings · Read; set: Change/Changing/Changed/a chat's AI or model · Changed | read: Reads; set: WritesThreads |
| chat_queue | Manage waiting messages | **adapted** "Manage a chat's queued messages. Omit `chatId` for this chat. list: send order, paged by `cursor` and `limit`. read: up to 16,000 chars of one. cancel. Writes fail once the message is no longer queued." | destructive | list: List/Listing/Listed/waiting messages; read: Read/Reading/Read/a waiting message; cancel: Cancel/Cancelling/Cancelled/a waiting message | list/read: Reads; cancel: WritesThreads |
| project_list | List projects | **exact** "List projects. Pages can shift." | RO | List/Listing/Listed/projects · Listed,project list,project lists | Reads |
| project_read | Read a project | **adapted** "Read one project: its folder, kind and chats." | RO | Read/Reading/Read/a project · Read,project,projects | Reads |
| project_create | Add a project | **adapted** "Register a project folder. `createWorkspaceRootIfMissing` creates it; a registered folder is rejected. No `workspaceRoot`: makes a new folder from `title` in your projects folder, with git when `git` is true." | destructive | Add/Adding/Added/a project · Added,project,projects | WritesEnvironment |
| project_update | Update a project | **exact** "Update one project's settings. Omitted fields are unchanged." | destructive | Update/Updating/Updated/a project · Updated,project,projects | WritesEnvironment |
| project_delete | Remove a project | **exact** "Delete one project. Nonempty needs `force: true`. Folder on disk stays." | destructive | Remove/Removing/Removed/a project · Removed,project,projects | WritesEnvironment |
| settings_read | Read settings | **adapted** "Read this app's identity and preferences. AIs and models: `fridae_capabilities`." | RO | Read/Reading/Read/settings · Read,setting list,setting lists | Reads |
| settings_update | Change settings | **adapted** "Update app-wide preferences. Omitted fields are unchanged. Below Full auto, and always for some, the user is asked first with a card in this chat." | destructive | Change/Changing/Changed/settings · Changed,setting,settings | WritesEnvironment |
| theme_read | Read themes | **adapted** "Read Fridae's own themes and the light, dark or system mode. Call before `theme_set`." | RO | Read/Reading/Read/themes · Read,theme list,theme lists | Reads |
| theme_set | Set the theme | **adapted** "Restyle Fridae itself, only when the user asks. Switch to a built-in `themeId`. `appearance` sets light, dark, or system mode." | destructive | Set/Setting/Set/the theme · Set,theme,themes | WritesEnvironment |

Parameter descriptions copied from T3 where they exist: `clientRequestId` "Reuse it to retry the same call. A new
call gets a new key." (max 256); `search` "Match model ids and labels. Replaces the short list." (max 200);
`chat_read.limit` "Max 100.", `maxCharsPerItem` "Max 20000."; `chat_launch.message` "First prompt. Omit it for idle."
(max 120,000); `scratch` **adapted** "No project: the chat works in its own folder. Not with `projectId`."

### Arguments and results (pinned)

- fridae_capabilities `{search?}` → `{ais:[{ai, displayName, installed, models:[{id,label?}], omittedModels?, efforts:[…], accounts:[{account, label, ready}], runtimeModes:["approval-required","full-access"], constraints:[…]}]}`; short list = first 12 models per AI; `search` matches id/label case-insensitively across all; efforts = `agents::EFFORTS` for claude and chatgpt, `[]` otherwise; accounts from `accounts::state()` (never tokens).
- chat_list `{projectId?, allProjects?, statuses?: ["working","idle","closed"] (≤3), titleContains?, includeSubagents?, cursor?, limit? 1..100 default 10}` → `{projectId|null, currentChatId, chats:[{chatId, projectId|null, title, link, status, latestRunId|null, ai, model|null, runtimeMode, interactionMode, parentChatId|null, subagent, itemCount, updatedAt}], nextCursor|null, total}`. Default project = caller's (none → chats with no project). `closed` = saved chats from `agents::history()`.
- chat_search `{query (2..256), limit? 1..50 default 10, projectId?}` → `{currentChatId, matches:[{chatId, projectId|null, title, link, source:"title"|"project"|"user"|"assistant", snippet?}]}`; titles and project names first, then message text of open chats then saved chat files; subagents skipped; snippet ≤ 200 chars around the hit.
- chat_read `{chatId, view?: messages|activity, afterPosition? (≥-1), beforePosition?, limit? ≤100 (default 20), maxCharsPerItem? ≤20000 (default 4000), itemId?, textOffset?}` → `{chat:{…list item…, activeRunId|null, closed}, items:[{position, itemId:"i<position>", type, text|null, textTruncated, nextTextOffset?}], nextPosition|null, hasMore, earlierPosition|null}`. Messages view = log events of kind `user`, `text`, `error`; activity = all. Serialised result capped at **30,000 bytes**: drop oldest items until it fits, `hasMore` true. Works on saved chats too (reads `WORK/chats/<id>.json`).
- chat_launch `{projectId?, scratch?, title, modelSelection?: {ai, model?, effort?, account?}, runtimeMode?, interactionMode?, message?, clientRequestId?}` → `{chatId, link, projectId|null, modelSelection, runId|null, status:"running"|"idle"}`. Steps: resolve modes (`access::resolve_runtime`), AI = given or caller's, `new_agent(Some(ai))` (None → `capability_denied` "Fridae is at its chat limit. Close a chat first."), `rename(title)`, `set_project(path)` (project from id, else caller's unless `scratch`), `set_pick`, `pick_account`, `set_mode_for_launch(full)`, then `ask(message, Ask{full, model, effort, ..})` on a thread when `message` is set.
- chat_send `{chatId, message (≤120000), mode?: auto|restart, clientRequestId?}` → `{chatId, link, runId|null, status, delivery:"started"|"queued"|"restarted"}`. Target `target_within`. `restart`: `stop()` + `pause_if_lead`, wait ≤5 s for idle, then ask. Ask carries the **target's** own mode (`last_ask.full/studio`), never the caller's. A send to itself → `invalid_request` "A chat cannot send to itself."
- chat_wait `{chatId, runId?, timeoutMs? (default 30000, max 45000)}` → `{chatId, runId|null, status:"idle"|"working", timedOut, next?}`; polls `busy()` every 250 ms; `runId` not the latest → returns at once with that chat's state.
- chat_interrupt `{chatId, runId?, reason?, stop?, clientRequestId?}` → `{chatId, runId|null, status:"interrupt_requested"|"no_active_run"}`; calls `stop()` then `studio_undo::pause_if_lead` like `/api/stop`.
- chat_update `{chatId?, action:"rename", title (≤60 after Fridae's trim), clientRequestId?}` → `{chatId, link, title}`.
- chat_organize `{chatId?, action:"archive"|"unarchive"}` → `{chatId, link, closed}` or `{closesWhenTurnEnds:true}` for the caller itself (a thread waits for `!is_working()` then `remove_agent`). unarchive = `open_chat`.
- chat_configure `{chatId?, modelSelection?}` → read: `{chatId, modelSelection:{ai, model|null, effort|null, account}, runtimeMode, interactionMode}`; set: `switch_ai` when `ai` differs (its `Err` → `thread_not_sendable` with Fridae's text), `set_pick`, `pick_account` (`Err` → `invalid_request`). Args `runtimeMode`, `full`, `studio` are not in the schema (ruling 7).
- chat_queue `{chatId?, action:list|read|cancel, queuedId?, cursor?, limit? ≤100}` → list `{items:[{queuedId, text (≤200 chars), truncated}], nextCursor}`; read `{queuedId, text (≤16000), truncated}`; cancel `{chatId, queuedId, cancelled}` (`unqueue`; false → `run_not_found` "That message is no longer queued.").
- project_list `{cursor?, limit? ≤100 default 25}` → `{projects:[{projectId, name, path, kind, git, link}], nextCursor}`.
- project_read `{projectId}` → `{projectId, name, path, kind, git, bible, link, chats:[{chatId, title, link, open}]}`.
- project_create `{title, workspaceRoot?, createWorkspaceRootIfMissing?, git?, kind?: "roblox"|"plain"}` → project + `note?`. No `workspaceRoot`: `projects::create(title, None, git, kind)`. `workspaceRoot` existing → always a card ("Add a project"), then `projects::add`. Missing + `createWorkspaceRootIfMissing` → `projects::create(basename, parent, git, kind)`. Already registered → `invalid_request` "That folder is already a project."
- project_update `{projectId, title?, kind?: "roblox"|"plain"|"auto"}` → project. `rename`, `set_kind`.
- project_delete `{projectId, force?}` → `{projectId, removed:true}`. Nonempty = it has an open or saved chat → without `force`: `invalid_request` "This project has chats. Pass force: true to remove it anyway; its folder and chats stay." A chat working in it → `thread_not_sendable` "A chat is working in this project. Stop it first." Always a card. Calls `projects::forget` only (never `delete`).
- settings_read `{}` → `{app:"Fridae", version, platform, preferences:{newChatAi, autoUpdate, expertSummaries, strongLead, leadPicksAi, geminiImages, geminiPaidTextures}, labels:{<key>: <Setup's words>}, neverSettable:["Gemini API key","programs agents may run","Use API keys from my environment","Studio plugin link","list of games","Experts table","Knowledge index","Most experts working at once","OpenCode's model","Second accounts","Share my skills","Let my other computers connect"]}`.
- settings_update — the fixed list, each through the same code its route runs, then `events::settings_changed(&[keys])`:

  | Arg | Config key | Handler (as the route) | Setup's words | Card |
  |---|---|---|---|---|
  | newChatAi | `chat_ai` | `/api/ai` without agent: must be in `paths::AIS` | "The AI new chats start on" | below Full auto |
  | autoUpdate | `auto_update` | `update::set_auto(on)` | "Install updates by itself" | below Full auto |
  | expertSummaries | `expert_summaries` (guarded) | `/api/expert-summaries` | "Plain-English summary of each expert's work" | always |
  | strongLead | `swarm::STRONG_LEAD_KEY` (guarded) | `/api/strong-lead` | "Keep the Lead on a strong model" | always |
  | leadPicksAi | `expert_ai::LEAD_PICKS` (guarded) | `/api/experts` `lead_may_pick` | "Lead may pick the AI" | always |
  | geminiImages | `gemini_images` (guarded, spends) | `/api/gemini` `images` | "Agents may generate images with Gemini" | always |
  | geminiPaidTextures | `gemini_use_api` (guarded, spends) | `/api/gemini` `use_api` | "paid Gemini textures" | always |

  Any other argument is rejected by `check_args`. More than one changed field → one card listing each line.
- theme_read `{}` → `{current:{themeId, appearance}|null, builtIn:[{id,name}], appearances:["light","dark","system"]}`; names Fridae, Graphite, Moss, Tide, Clay, Lilac.
- theme_set `{themeId?, appearance?}` (one required: else `invalid_request` "Pass themeId or appearance.") → `{themeId?, appearance?}`; then `events::theme_set`.

### Card copy (pinned; allow button / quieter button is always "Don't")

| When | Title | Sentence | Allow |
|---|---|---|---|
| project_create new folder | Make a project | Make a new project folder "{title}" in {parent}. | Make it |
| project_create existing folder | Add a project | Add the folder {path} to your projects. | Add it |
| project_update title | Rename project | Rename "{old}" to "{new}". Its folder keeps its own name. | Rename it |
| project_update kind | Change project type | Make "{name}" a {Roblox / plain} project. | Change it |
| project_delete | Remove project | Take "{name}" off your projects. The folder and its chats stay. | Remove it |
| settings_update (one switch) | Change a setting | Turn {on/off} "{Setup's words}". | Turn it on / Turn it off |
| settings_update newChatAi | Change a setting | Start new chats on {AI name}. | Change it |
| settings_update (several) | Change settings | One line per change, as above, joined by spaces. | Change them |
| theme_set | Change the look | Switch Fridae to {Name} in {light/dark/system} mode. (only the parts given) | Change it |

### Steps

- [ ] **Failing tests**, one per tool family (`#[cfg(test)]` in each file; chats via `new_agent`, caller running via
  `set_running_for_test(true)`, modes via `set_last_ask_for_test`; projects in `H:/rust/tmp/app-tools-<test>`
  registered with `projects::add`; each test forgets its projects and removes its chats; call through
  `app_mcp::call_tool` so access and events are exercised):
  - `defs_are_the_21_in_order_with_pinned_descriptions` (names; `chat_list`, `chat_wait`, `project_list`,
    `project_update`, `project_delete` descriptions equal the **exact** strings above).
  - `capabilities_short_list_and_search`.
  - `chat_list_newest_first_and_pages`, `chat_list_marks_closed_and_hides_subagents_by_default`.
  - `chat_search_titles_before_text_skips_subagents`.
  - `chat_read_pages_and_caps_at_30000_bytes` (a chat with 200 long `text` events: serialised result ≤ 30,000 bytes,
    `hasMore`).
  - `chat_launch_makes_a_sidebar_chat`: BUS gets `agent_new`, `renamed` with the title, `project` with the path;
    result `link` == `chat_link(id, title)`; with `message`, `last_user_text_for_test` == message.
  - `chat_launch_inherits_and_never_widens`: approval caller + `runtimeMode:"full-access"` →
    `runtime_mode_escalation_denied`; full caller, omitted → launched chat's `last_ask().full`.
  - `chat_launch_replays_client_request_id` (one chat made for two calls).
  - `chat_send_refuses_a_broader_target` (T3 words), `chat_send_queues_when_busy` (`delivery:"queued"`, `queued()==1`),
    `chat_send_uses_the_targets_mode`.
  - `chat_wait_times_out_at_the_cap` (`timeoutMs: 999999` returns by 46 s; test uses a 300 ms override) and
    `chat_wait_idle_at_once`, `chat_wait_next_for_a_studio_lead`.
  - `chat_interrupt_stops_and_reports_no_active_run_when_idle`.
  - `chat_update_renames_this_chat_by_default` (BUS `renamed`).
  - `chat_organize_archive_this_chat_waits_for_turn_end`, `chat_organize_unarchive_reopens`.
  - `chat_configure_reads_and_sets`, `chat_configure_has_no_mode_fields` (`runtimeMode` arg → -32602).
  - `chat_queue_list_read_cancel`.
  - `project_tools_full_auto_no_card`: full caller `project_update {title}` → renamed, BUS `projects_changed`, no
    `appcard`.
  - `project_tools_below_full_auto_ask`: approval caller, answer via `card::answer(.., true)` from a helper thread →
    renamed; `false` → result `isError` code `declined`, unchanged.
  - `project_delete_always_asks_and_keeps_the_folder` (full caller still gets a card; folder exists after; nonempty
    without force refused).
  - `project_create_existing_folder_always_asks`.
  - `settings_update_ordinary_and_guarded`: full caller `newChatAi:"chatgpt"` → no card, `paths::setting("chat_ai")`
    == "chatgpt", BUS `settings_changed`; `geminiImages:true` → card even in full auto; unknown arg `geminiKey` →
    -32602.
  - `settings_read_never_shows_secrets` (result text contains no value of `gemini_key` set in the test config).
  - `theme_set_needs_one_field_and_announces` (BUS `theme_set {themeId:"moss"}`), `theme_read_lists_six`.
  - `subagent_follows_the_same_rules`: a helper chat (`spawn_expert`) with an approval-required parent cannot launch
    full-access.
- [ ] Run `cargo test --offline --manifest-path app-rs/Cargo.toml --target-dir 'H:\fridae-target-tools' app_mcp::tools` — Expected: fail (stubs).
- [ ] Implement file by file (caps.rs, chats.rs, projects.rs, settings.rs, theme.rs; `mod.rs` concatenates
  `caps::defs()`, `chats::defs()`, `projects::defs()`, `settings::defs()`, `theme::defs()` in the pinned order).
  Every handler that names a chat or project calls `call.links(...)`.
- [ ] Run — Expected: pass; then the full suite — Expected: pass (bar the known flaky one).
- [ ] Commit `app_mcp: chat, project, settings, theme and capability tools`.

---

## Task 3: Per-AI wiring, stdio bridge, deferred tools, agent instructions (parallel)

Worktree `H:/fridae-wt/wiring`, branch `app-tools/wiring`. Target dir `H:\fridae-target-wiring`.

**Files:** Replace stubs `app-rs/src/app_mcp/{exposure.rs, bridge.rs, instructions.rs}`; modify `agents.rs`,
`codex.rs`, `acp.rs`, `cursor.rs`, `opencode.rs`, `antigravity.rs`, `main.rs`.

**Interfaces Consumes:** `app_mcp::{wire_for, Wire, registry, ToolDef, call_tool, SERVER_NAME}`, `keys::global()`.
**Interfaces Produces:**

```rust
// exposure.rs
pub const TOOL_SEARCH: &str = "tool_search"; pub const TOOL_CALL: &str = "tool_call";
pub const MAX_RESULTS: usize = 10; pub const MAX_PROXY_SCHEMA_BYTES: usize = 8_000;
pub fn listed(mode: Mode, name: &str) -> bool;  // Full: all but tool_search/tool_call; DeferredProxy: fridae_capabilities, tool_search, tool_call
pub fn search(query: &str, mode: Mode) -> Vec<Value>;
pub fn defs() -> Vec<ToolDef>;                   // tool_search, tool_call
// bridge.rs
pub struct BridgeConfig { pub endpoint: String, pub authorization: String, pub exposure: Option<String> } // Debug redacted
pub fn main() -> i32;                            // reads env; missing → stderr line + 2
pub fn run(input: impl BufRead, output: Arc<Mutex<dyn Write + Send>>, cfg: &BridgeConfig, post: &dyn Fn(&BridgeConfig, &Headers, &str) -> Result<HttpAnswer, String>) -> i32;
// instructions.rs
pub fn sections(deferred: bool) -> Vec<&'static str>; // each < 4,000 bytes
pub fn system_text() -> String;                       // for Claude --append-system-prompt and OpenCode/Codex
pub fn prompt_prefix(prompt: &str) -> String;         // ACP / per-message AIs whose instructions ride the prompt
// agents.rs
pub fn is_toolbox_call(name: &str) -> bool;           // the name forms in "Shared shapes"
```

### 3.1 Agent instructions (port of orchestrationInstructions.ts) — pinned text

Section 1:
```
## Fridae toolbox

Ordinary coding tasks need no Fridae tools.

- The `fridae` tools run the Fridae app itself: chats, projects, AIs and models, settings and the theme. Use them when the user asks you to do something with the app.
- Call `fridae_capabilities` once per chat, before you pick an AI or model. Again only after an error or a user-named AI change. AI or model unavailable: say so. Never switch unasked.
- As a subagent or child, do the work yourself. Start no subagents unless your task says to.
```
Section 2:
```
- `chat_launch` makes a new top-level chat, only when the user asks for a new chat. "Subagent", "delegate", and "parallel" never mean one.
- Given a chat id, `chat_read` it before you answer about it.
- In your reply, mention a chat by its `link`, verbatim, never a plain title or bare id. The same for a project.
- Restyle Fridae with `theme_set` only when the user asks.
- Some changes show the user an Allow or Don't card in this chat and wait for the answer. Declined: don't ask again in chat; say what's blocked and go on without it.
- Fridae tools may load late. Try the name once before you call it absent.
```
Deferred-proxy section (only when `deferred`):
```
Deferred Fridae tools: only `fridae_capabilities`, `tool_search`, and `tool_call` are declared directly. For any other Fridae tool (`chat_*`, `project_*`, `settings_*`, `theme_*`), call `tool_search` to get its schema and `tool_call({ name, arguments })` to run it.
```
`prompt_prefix(p)` = `"<fridae_toolbox_instructions>\n{sections joined \n\n}\n</fridae_toolbox_instructions>\n\n<user_request>\n{p}\n</user_request>"`,
and returns `p` unchanged when it starts (after whitespace) with `/`.

- [ ] Failing tests: `every_section_is_under_4000_bytes`, `section_one_opens_with_t3s_line`
  (`"Ordinary coding tasks need no Fridae tools."`), `deferred_adds_the_proxy_section`,
  `slash_commands_are_left_alone`. Run (`… app_mcp::instructions`) — Expected: fail. Implement. Run — pass. Commit
  `app_mcp: agent instructions (port of orchestrationInstructions)`.

### 3.2 Deferred tools (port of McpToolExposure.ts + tool_call)

Descriptions: tool_search **adapted** "Find Fridae tools you do not have yet. If tool_call is available, pass it the
returned tool's name and your arguments. Otherwise search loads the tools so you can call them by name. Search by
name or purpose: chats (chat_*), projects (project_*), settings (settings_*), themes (theme_*). Exact names work too."
(param `query` max 500; RO, not destructive; labels Find/Finding/Found/Fridae tools). tool_call **exact** "Invoke any
Fridae tool by exact name (find schemas with tool_search first)." params `name` "Exact Fridae tool name.", `arguments`
"Arguments object for that tool."; hints `readOnlyHint:false, openWorldHint:false`. tool_call errors (-32602):
"name is required.", `Cannot invoke '{name}' via tool_call.`, "arguments must be a JSON object."; bad arguments for
the named tool → `isError` result whose text names the tool and includes its `inputSchema`. Its apptool event is the
called tool's.

- [ ] Failing tests: `scoring_matches_t3` (exact name +100, name contains +3, description +1; split on
  `[^a-z0-9_]+`), `proxy_results_fit_in_8000_bytes` (falls back to name+description, then a 200-char cut with
  " (Truncated: exceeds response budget.)"), `listing_by_mode`, `tool_call_refuses_itself_and_search`,
  `tool_call_runs_the_named_tool_with_its_access` (an idle caller calling `chat_update` through `tool_call` →
  `parent_not_active`), `http_lists_three_tools_with_the_header` (via `http::handle_parts` with
  `fridae-tool-exposure: deferred-proxy`). Run — fail. Implement. Run — pass. Commit `app_mcp: deferred tools for Antigravity`.

### 3.3 Stdio bridge (exact port of AcpMcpStdioBridge.ts)

Pinned behaviour: stdin split on `\n` only (U+2028/U+2029 stay inside a message); each non-empty line POSTed
verbatim; headers `content-type: application/json`, `accept: application/json, text/event-stream`,
`authorization`, `fridae-tool-exposure` when set, `mcp-session-id` and `mcp-protocol-version` once known (session id
from any response header; version from the initialize result); `initialize` and `notifications/initialized` behind a
barrier (others concurrent, a thread each); lines that are client responses (no `method`) skip the barrier; answers
parsed as JSON or SSE (`data:` lines) and each message written as one line (arrays split); non-2xx with an id →
`{"jsonrpc":"2.0","id":…,"error":{"code":-32603,"message":"Fridae toolbox endpoint responded with HTTP <status>."}}`,
notifications dropped; a failed request → `-32603` "Fridae toolbox bridge request failed: <msg>"; an unparseable line
→ `{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}`; non-object JSON ignored; no
reconnect; stdin EOF → wait for in-flight, DELETE the session, exit 0. Missing `FRIDAE_MCP_ENDPOINT` or
`FRIDAE_MCP_AUTHORIZATION` → stderr "fridae --mcp-bridge needs FRIDAE_MCP_ENDPOINT and FRIDAE_MCP_AUTHORIZATION." and exit 2.

- [ ] Failing tests (with a fake `post`, plus one against a real `tiny_http` stub on 127.0.0.1:0):
  `forwards_and_replays_session_and_version`, `notifications_silent_failures_synthesised`, `parse_errors`,
  `keeps_u2028_inside_a_message`, `sse_messages_forwarded`, `client_responses_skip_the_barrier`,
  `keeps_the_servers_jsonrpc_error`, `forwards_the_exposure_header`, `missing_env_exits_2`,
  `never_prints_the_authorization` (stderr/stdout captures contain no key).
- [ ] Run — fail. Implement with `ureq` (agent with no timeout for the body: card waits are long).
- [ ] `main.rs`: fast path right after the `--board-hook` block: `if args.iter().any(|a| a == "--mcp-bridge") { std::process::exit(app_mcp::bridge::main()); }`.
- [ ] Run — pass. Commit `app_mcp: stdio bridge (port of AcpMcpStdioBridge)`.

### 3.4 Wiring each AI

Pinned per AI (the chat's key comes from `app_mcp::wire_for(&self.id, &kind, self.is_subagent())`, called in
`send` right after `let servers = mcp_servers(...)`; `keys::global().touch(&self.id)` at the same place, every turn):

| AI | How it gets the server | Where the key travels |
|---|---|---|
| Claude | `servers["fridae"] = {"type":"http","url":<endpoint>,"headers":{"Authorization":"Bearer ${FRIDAE_MCP_KEY}"},"timeout":3900000}` in the `--mcp-config` file (WORK/agent-tools, placeholder only); `--allowedTools` gains `mcp__fridae`; `--append-system-prompt` gains `instructions::system_text()` | `FRIDAE_MCP_KEY` env on the process (`ClaudeLaunch.mcp_key`, set in `command()`, **not** in `key()`) |
| Claude (if probe P1 fails) | `servers["fridae"] = {"type":"stdio","command":<current_exe>,"args":["--mcp-bridge"],"env":{"FRIDAE_MCP_ENDPOINT":<endpoint>,"FRIDAE_MCP_AUTHORIZATION":"Bearer ${FRIDAE_MCP_KEY}"}}` | same env var, expanded by Claude in `env` |
| Codex | `codex::command` gains `toolbox: Option<&Wire>`; adds `-c` overrides (never written to `.codex/config.toml`): `mcp_servers.fridae.url="<endpoint>"`, `mcp_servers.fridae.bearer_token_env_var="FRIDAE_MCP_KEY"`, `mcp_servers.fridae.tool_timeout_sec=3900`, `mcp_servers.fridae.disabled_tools=["chat_wait"]`, and the same four for `fridae-direct` with `enabled_tools=["chat_wait"]` instead of `disabled_tools` (plus `omit_tools_from=["code_mode","deferred"]` only if probe P2 shows Codex accepts it); `-c developer_instructions=<system_text()>` | `FRIDAE_MCP_KEY` env on the process |
| OpenCode | `opencode::Spec` gains `toolbox: Option<Wire>`; `servers_config` adds `(server_name(id,"fridae"), {"type":"remote","url":…,"headers":{"Authorization":<authorization>},"oauth":false,"timeout":{"execution":3900000}})` over Fridae's own server's API (existing permission rules already allow `fr…-*` and deny others); `system_text()` appended to `spec.instructions` | in the PUT body to Fridae's own opencode server only |
| Gemini (kept, ACP) | `acp::Spec` gains `toolbox: Option<Wire>`; `servers_param(servers, toolbox, http_ok)` adds `{"type":"http","name":"fridae","url":…,"headers":[{"name":"Authorization","value":…},{"name":"fridae-tool-exposure","value":"deferred-proxy"}]}` when the initialize answer has `agentCapabilities.mcpCapabilities.http == true`, else `{"name":"fridae","command":<current_exe>,"args":["--mcp-bridge"],"env":[ENDPOINT, AUTHORIZATION, FRIDAE_MCP_TOOL_EXPOSURE=deferred-proxy],"timeoutSeconds":3900}`; `prompt_prefix` on the first prompt and whenever `toolbox` presence changes | in memory over the ACP pipe |
| Gemini (per message) | never in the project's `.agents/mcp_config.json` (`antigravity::write_servers` skips `fridae`); no-project chats get the keyless bridge entry `{"command":<current_exe>,"args":["--mcp-bridge"],"env":{"FRIDAE_MCP_ENDPOINT":…,"FRIDAE_MCP_TOOL_EXPOSURE":"deferred-proxy"}}` only if probe P3 shows agy passes its environment to stdio servers | `FRIDAE_MCP_AUTHORIZATION` env on the agy process |
| Cursor (kept, ACP) | `kept_servers` adds `{"type":"http","name":"fridae","url":…,"headers":[Authorization]}` (no exposure header) | in memory over ACP |
| Cursor (per message) | `.cursor/mcp.json` in Fridae's own workspace gets `{"url":…,"headers":{"Authorization":"Bearer ${env:FRIDAE_MCP_KEY}"}}` only if probe P4 (Mac) confirms interpolation; else nothing | `FRIDAE_MCP_KEY` env |

Also: `show_tool` skips the step label when `is_toolbox_call(name)` (the server's apptool events draw those rows; the
tool still counts in `turn_tools`); `remove_agent` and `carry_on_with` call `keys::global().revoke_chat(id)`;
`shut_down` calls `revoke_all()`.

Probes (run once in this task by hand, results written into the commit message and DEVELOPING.md is **not** edited):
- P1 Claude: a test `#[ignore] fn probe_claude_expands_env_in_headers` starts a `tiny_http` stub on 127.0.0.1:0 that
  records the `Authorization` of the first POST, writes a temp `--mcp-config` (in `H:/rust/tmp`) with the placeholder,
  runs `claude -p --strict-mcp-config --mcp-config <file> --max-turns 1 "reply ok"` with `FRIDAE_MCP_KEY=probe123`
  and a 60 s limit; passes if the stub saw `Bearer probe123` (Claude initializes MCP servers before any model call).
- P2 Codex: same stub, `codex exec --json --skip-git-repo-check -c …overrides… -` with stdin "reply ok"; passes if
  the stub saw `Bearer probe123`; a second run adds `omit_tools_from` and passes if Codex does not exit on config error.
- P3 agy: a stdio stub script (python, in `H:/rust/tmp`) that writes `os.environ.get("FRIDAE_MCP_AUTHORIZATION")`
  to a file, listed in a temp workspace's `.agents/mcp_config.json`; passes if the value arrived.
- P4 Cursor: Mac only, by hand in Task 6.

- [ ] Failing tests (`agents.rs` tests module):
  - `claude_config_has_the_placeholder_never_the_key`: written `--mcp-config` file contains `${FRIDAE_MCP_KEY}` and
    not the raw key; `--allowedTools` contains `mcp__fridae`.
  - `rotating_the_key_keeps_a_kept_claude_process`: two `ClaudeLaunch`es differing only in `mcp_key` have equal `key()`.
  - `claude_process_gets_the_key_in_env` (`command(false, None)` envs contain `FRIDAE_MCP_KEY`).
  - `codex_overrides_on_the_command_line_not_in_the_project` (`codex::command` args contain
    `mcp_servers.fridae.bearer_token_env_var="FRIDAE_MCP_KEY"` and the `fridae-direct` lines; the workspace
    `.codex/config.toml` written by `write_servers` has no `fridae`).
  - `opencode_remote_server_named_per_chat` (`servers_config` entry name == `chat_prefix(id) + "fridae"`, type `remote`).
  - `acp_http_when_advertised_else_bridge` (both shapes; exposure header / env present for Gemini, absent for Cursor).
  - `gemini_project_list_never_gets_fridae` (`antigravity::write_servers` output has no `fridae` key).
  - `toolbox_tool_names_hide_the_generic_step` (`is_toolbox_call` true for every name form in "Shared shapes",
    false for `mcp__rbxkit__rbx_team`).
  - `turn_start_touches_the_key` and `closing_a_chat_revokes_its_key`.
- [ ] Run `… agents:: codex:: acp:: opencode:: antigravity:: cursor::` — Expected: new tests fail.
- [ ] Implement, then run probes P1–P3 and pick the Claude/Gemini rows they allow.
- [ ] Run the full suite — Expected: pass (bar the known flaky one).
- [ ] Commit `Wire the Fridae toolbox into Claude, Codex, OpenCode, Gemini and Cursor` (message lists P1–P3 results).

---

## Task 4: The page — approval card, tool rows with chips, live refresh, theme (parallel)

Worktree `H:/fridae-wt/ui`, branch `app-tools/ui`. Build `python ui/build.py`.

**Files:** Create `ui/src/chat/AppCard.tsx`, `ui/src/chat/appTools.ts`, `tests/app-tools-page/{mock.py, page.js,
run.sh, README.md}`; modify `ui/src/chat/chatStore.ts`, `ui/src/chat/Rows.tsx`, `ui/src/chat/md.ts`,
`ui/src/chat/chat.css`, `ui/index.html`, `ui/app.js`, `ui/app.css` (built).

**Interfaces Consumes:** BUS events and page routes in "Shared shapes"; `projects` entries carry `id`.
**Interfaces Produces:** chat parts `appcard` and `apptool`; `onEvent` handling for `projects_changed`,
`settings_changed`, `accounts_changed`, `theme_set`; POST `/api/theme-now` from the page.

Pinned behaviour:
- `appcard` → part `{k:"appcard", card, title, text, allow, said:""}` in the current turn (`p.asks[card]`), `p.now =
  "Waiting for you"`, `p.last = "Needs you"`. Pending look = the `.ask` card: `<h4>{title}</h4><p>{text}</p>` and
  `.act` with `<button class="pill main">{allow}</button><button class="pill">Don't</button>`. A press posts
  `/api/app-card {agent, card, answer}` and disables both buttons.
- `appcard_done` → that part becomes one compact line `"{title} · {allowed|declined|closed|timed out}"` (outcome
  `cancelled` reads "closed"), styled like `.tag`. A pending card in a replayed chat (`p.replay`) with no
  `appcard_done` draws as `"{title} · closed"`.
- `apptool` → part `{k:"apptool", call, status, label, summary, links}` keyed by `call` (later events update it in
  place). Look: one line in the turn's step style with Fridae's mark (`/logo.png`, 12 px, `alt=""`), the label, then
  one chip per link. Failed/declined lines use the existing muted error text colour, nothing new.
- Grouping: when a turn ends (`done`), runs of 2+ consecutive `apptool` parts with the same `summary.verb` and status
  `done` fold into one line `"{verb} {n} {many}"` ("Renamed 2 projects") that opens to the single lines on click.
- Chips: `md.ts` turns `fridae://chat/<id>` links into `<a class="chip app-link" data-chat="<id>" href="#">title</a>`
  and `fridae://project/<pid>` into `data-project="<pid>"`; any other `fridae:` link is plain text. A click opens the
  chat (`H.openChatById`) or the project (the project in the store whose `id` matches); unknown → nothing happens and
  the title stays plain. Chips in `apptool` lines use the same element.
- `index.html` `onEvent`: `projects_changed` → the page's existing projects reload; `settings_changed` and
  `accounts_changed` → the existing status refresh; `theme_set` → `setTheme({id: e.themeId || themeChoice.id, mode:
  e.appearance || themeChoice.mode})` (ignoring unknown ids/modes) and `store.set({theme})`. `applyTheme` posts
  `/api/theme-now {id, mode}` (fire and forget) at load and on every change.

- [ ] **Failing page test** `tests/app-tools-page/page.js` (copy `tests/remote-page`'s runner pattern, mock on
  127.0.0.1:8799, Playwright from `H:/ms-playwright`, node `H:/node/node.exe`) with checks:
  `card_pending_shows_title_text_and_two_buttons` ("Rename it", "Don't"); `allow_posts_and_collapses`
  (mock records `/api/app-card {answer:"allow"}`; after the mock's `appcard_done` the line reads
  "Rename project · allowed"); `replayed_pending_card_reads_closed`; `apptool_running_then_done_updates_one_line`;
  `failed_and_declined_lines`; `chips_open_the_chat_and_the_project`; `grouped_summary_after_done`
  ("Renamed 2 projects"); `projects_changed_refreshes_the_sidebar` (mock serves a renamed project after the event;
  sidebar shows the new name with no reload); `settings_changed_refetches_status`;
  `theme_set_applies_and_reports` (`document.documentElement.dataset.theme == "moss"`, mock got `/api/theme-now`);
  `no_console_errors`.
- [ ] Run `sh tests/app-tools-page/run.sh H:/fridae-wt/ui/ui` — Expected: FAIL counts > 0.
- [ ] Implement; `python ui/build.py`.
- [ ] Run — Expected: `FAIL: 0`. Also run `sh tests/remote-page/run.sh H:/fridae-wt/ui/ui` — Expected: `FAIL: 0`
  (nothing else on the page broke). Screenshots of the card (pending and answered) in light and dark go to
  `H:/rust/tmp/rvwork-apptools/shots` for the controller to look at.
- [ ] Commit `Page: Allow/Don't card, toolbox rows with chips, live refresh and theme from the AI` (with built
  `ui/app.js`, `ui/app.css`).

---

## Task 5: Credit and end-to-end tests (parallel)

Worktree `H:/fridae-wt/credit`, branch `app-tools/credit`. Target dir `H:\fridae-target-credit`.

**Files:** Modify `THIRD_PARTY.md`; `licenses/t3code-LICENSE.txt` already exists (verify it is the full MIT text with
"Copyright (c) 2026 T3 Tools Inc.", unchanged); replace stub `app-rs/src/app_mcp/e2e_tests.rs`.

- [ ] In `THIRD_PARTY.md`'s `## T3 Code` section add a bullet: **App toolbox** (`app-rs/src/app_mcp/`, the toolbox
  rows and approval card in `ui/src/chat/`): a port of T3 Code's app-control MCP server at commit 9ceb8bfb — its
  streamable-HTTP admission (McpHttpServer.ts), per-chat credentials (McpSessionRegistry.ts), access rules
  (McpToolAccess.ts, threadAccess.ts, delegatedTaskModes.ts), deferred tools (McpToolExposure.ts), stdio bridge
  (AcpMcpStdioBridge.ts), tool names and descriptions (toolkits), agent instructions (orchestrationInstructions.ts)
  and tool-row labels (t3McpToolPresentation.ts), with "thread" read as "chat". The Allow/Don't card is Fridae's
  own, its waiting modelled on T3 Code's request_secret. Same licence line as the section (MIT,
  `licenses/t3code-LICENSE.txt`). The section's "Not taken" paragraph stays as it is.
- [ ] Write `e2e_tests.rs` (compiled only under `cfg(test)`, every test `#[ignore]` because it needs Tasks 2 and 3 and
  a real CLI; run in Task 6):
  - `toolbox_over_http_end_to_end` (not ignored once joined; mark `#[ignore = "needs Task 2"]` until then): a
    `tiny_http` server on 127.0.0.1:0 serving `/mcp` via `http::serve`, a running caller chat with a key; initialize
    → tools/list has the 21 tools → `chat_launch {title:"From the toolbox"}` → BUS has `agent_new` and `renamed`
    "From the toolbox" → `chat_list` contains it → DELETE.
  - `bridge_end_to_end`: `bridge::run` over in-memory stdin against that server: initialize, tools/list, call.
  - `every_ai_gets_a_toolbox_entry`: for each of `claude, chatgpt, opencode, gemini, cursor`, the launch pieces Task 3
    builds (Claude config file, Codex args, OpenCode `servers_config`, ACP `servers_param`) name `fridae` and none
    contains the raw key (`#[ignore = "needs Task 3"]`).
  - Live per-AI checks cannot run under `cargo test` (test builds start no AI); they are Task 6's manual checklist.
- [ ] Run `cargo test --offline --manifest-path app-rs/Cargo.toml --target-dir 'H:\fridae-target-credit' app_mcp::e2e_tests` — Expected: compiles; all ignored.
- [ ] Commit `Credit T3 Code for the app toolbox; end-to-end checks`.

---

## Task 6: Join and check (after 2–5)

In `C:/Users/river/fridae studios` on `feature/app-tools` (target dir `H:\fridae-target-join`).

- [ ] `git merge --no-ff app-tools/core` (if not already the base), then `app-tools/tools`, `app-tools/wiring`,
  `app-tools/ui`, `app-tools/credit`, one at a time, running `cargo build --offline` after each. No conflicts are
  expected (ownership is disjoint); if one appears, the owning task's version wins and the controller is told.
- [ ] Remove the `#[ignore = "needs Task 2"]` / `"needs Task 3"` marks in `e2e_tests.rs`
  (`toolbox_over_http_end_to_end`, `bridge_end_to_end`, `every_ai_gets_a_toolbox_entry`).
- [ ] Run the full suite — Expected: all pass (bar the known flaky one, re-run alone).
- [ ] Run the page tests (`tests/app-tools-page/run.sh` and `tests/remote-page/run.sh` against `ui/`) — Expected:
  `FAIL: 0`.
- [ ] Live check per installed AI with the **dev build** (`cargo build --offline … --target-dir 'H:\fridae-target-join'`,
  run `H:\fridae-target-join\debug\fridae.exe`; never the installed Fridae.exe; quit the installed app first only if
  the user agrees, because both want port 47528 — the dev build falls back to a free port otherwise): in a new chat
  on each of Claude, ChatGPT (Codex), OpenCode and Gemini (kept process on, `FRIDAE_GEMINI_KEPT=1`) type "Start a new
  chat called E2E check in this project, with no message." Expected: a chat "E2E check" appears in the sidebar at
  once, the asking chat shows the row "Started a chat" with a chip that opens it. Then, in an Ask-me chat, "Rename
  this project to E2E renamed": the Allow/Don't card appears; Don't leaves the name; asking again and Rename it
  renames it in the sidebar at once. Record per AI: passed / skipped (not installed).
- [ ] Grep the joined diff for key leaks: `git diff main...HEAD | grep -nE 'raw|Authorization|FRIDAE_MCP_KEY'` and
  check every hit is a placeholder, an env-var name, or inside `keys.rs`/bridge/wiring.
- [ ] Commit `Join the app toolbox tasks` (the merge commits are made with the same author line and trailer).
- [ ] Report to the user: what works per AI, probe results P1–P3, the Gemini per-message decision still open, and that
  nothing is pushed or merged into main.
