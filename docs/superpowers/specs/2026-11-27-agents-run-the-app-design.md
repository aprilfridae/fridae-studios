# The AI runs Fridae itself (app-control toolbox) — design

Status: approved by the user 2026-11-27 ("Yes, build it"). Branch feature/app-tools, from feature/remote-switch (26c84fa).
Port of T3 Code's app-control MCP server (github.com/pingdotgg/t3code @ 9ceb8bfb, MIT, Copyright (c) 2026 T3 Tools Inc.).
Detailed source notes: `.superpowers/handoff/part2-t3-notes-1-4.md`, `part3-t3-notes-5-7.md`, `part4-tools-and-diffs.md`
(git-ignored); read-only T3 clone at `H:/t3code-ref`. Where this spec is silent, T3's source is the authority.

## Intent
The user tells the AI in any Fridae chat what to do with the app itself, in plain words ("start a new chat in my Shop
project", "rename that project", "move this chat to Gemini", "turn that setting off", "stop the other chat"), and it
happens, with the screen changing at once as if they had clicked it. User's words: "copy those ... if i ask it to do
basically anything it should be able to do that." and "do it the way t3 does it just copy their swag".

## Goals
- Every chat's AI (Claude, Codex/ChatGPT, Cursor, Antigravity/Gemini, OpenCode) gets a built-in toolbox, server name
  `fridae`, to operate the app: chats, projects, models, accounts, settings, appearance, runs.
- The UI reflects each change at once, with no reload.
- Built T3's way: MCP server inside the app (streamable HTTP), a per-chat key, T3's tool set, descriptions, access rules,
  agent instructions and tool rows, ported to Rust/Preact.
- The user's own addition: invasive actions show an Allow/Decline card in the chat (T3 refuses or has no card).

## Non-goals (this version)
Scheduled tasks, workflows, worktrees, pull requests, issues, preview browser, devices, linked environments, outside
OAuth clients, a command line, request_secret, theme authoring, chat deletion, moving a chat to another project, name
matching, thread transfer (fork/merge), pending user-question answering. Phase 2 (removing Fridae's overlapping tools)
is a separate step after the user approves its list.

## Anti-goals
- An AI deletes, spends, publishes or widens powers without the user's yes.
- A chat gives itself, or a chat it starts or messages, more freedom than it has.
- A key in logs, chat text, a file inside the user's project folder, or git.
- The UI disagreeing with what the AI did, or needing a reload.
- T3's name, logos or branded theme anywhere in Fridae. Never modify the user's own ~/.claude, ~/.gemini, ~/.cursor or
  OpenCode config, and never touch the user's own running `opencode serve`.

## Approaches
1. Add app tools to rbxkit_mcp.py (Python, stdio). Cheapest; rejected by the user.
2. **A separate MCP server built into the app, T3's way. Picked.**
3. The AI writes magic phrases the app acts on. Ruled out (fragile, no access rules).

## 1. Server (port of T3 McpHttpServer.ts + Effect's MCP HTTP transport)
- New Rust module(s) under app-rs/src/app_mcp/ on the existing local server (server.rs): `POST /mcp`, `DELETE /mcp`.
  Endpoint handed to agents: `http://127.0.0.1:<port>/mcp`.
- Protocol 2025-06-18 (stateful, sessions). Methods: initialize, notifications/initialized, ping, tools/list,
  tools/call, notifications/cancelled. Plain JSON responses (no elicitation, so no SSE needed).
- Admission order (T3): any Origin header → 403 empty body before credentials; `Authorization: Bearer <key>`
  (scheme case-insensitive) else 401 with T3's body/headers (message reworded to Fridae: "A valid Fridae toolbox key is
  required."), reason logged never the key; GET/PUT/PATCH/OPTIONS → 405 `allow: POST, DELETE`; POST Content-Type JSON
  else 415; Accept listing both application/json and text/event-stream else 406; Mcp-Session-Id minted at initialize
  (UUIDv4) with mcp-protocol-version header; initialize carrying one → 400; others without → 400; unknown → 404 empty;
  bad MCP-Protocol-Version → 400 UNSUPPORTED_PROTOCOL_VERSION; non-JSON body → 200 with -32700; batches → 400;
  notification-only → 202; DELETE → 204 and the session ends. Body size capped.
- The page token never works on /mcp; a chat key never works on page routes.
- Results: success `{isError:false, structuredContent, content:[{type:"text", text: JSON}]}`; declared failure
  `{isError:true, content:[{type:"text", text: "{\"_tag\":\"FridaeToolFailure\",\"code\":…,\"message\":…}"}]}` with
  T3's codes; crash → "Tool execution failed due to an internal server error."; bad args → -32602 with
  `Unknown argument "x". Accepted arguments: a, b, c.` (up to 5); optional fields accept null; unknown tool → -32601.
- tools/list: name, description, inputSchema, annotations (T3's hints incl. Effect defaults).
- Waits: chat_wait default 30 s, max 45 s, returns `timedOut` + `next`. Results capped as T3 (chat_read 30,000 bytes).
- clientRequestId idempotency on chat_launch, chat_send, chat_interrupt, chat_update, scoped to the key's namespace.

## 2. Key per chat (port of McpSessionRegistry.ts + ProviderSessionManager.ts:614-711)
- 32 random bytes, base64url. Only the SHA-256 hex is kept, mapped to {chat id, AI (provider instance), subagent flag,
  namespace, issued, lastAlive}. Prepares serialized per chat.
- Reused while chat, AI and subagent flag are unchanged; otherwise revoke + issue. touch(chat) at every turn start
  (revives expired). 24 h idle expiry (max of wall and monotonic). Revoked when the AI process is released or the chat
  removed; app restart = all new (memory only).
- Modes are never in the key; read live on every call.
- Delivery in memory only, never in a file inside a project folder, never in logs:
  - Claude: key in the process environment; the --mcp-config file (in work/agent-tools, not a project) refers to it
    via `${VAR}` header expansion; entry `{type:"http", url, headers:{Authorization:"Bearer ${FRIDAE_MCP_KEY}"},
    timeout: 3900000}`. If the CLI cannot expand env in headers, use the stdio bridge entry with env instead.
  - Codex: per-chat config overrides `mcp_servers.fridae` {url, bearer from env var, tool_timeout_sec 3900,
    disabled_tools [chat_wait]} and `mcp_servers.fridae-direct` {enabled_tools [chat_wait], omit from code mode/deferred}
    — as far as Fridae's Codex launch supports; never written into a project's .codex config.
  - Cursor: `{type:"http", url, headers}` per turn over ACP (else stdio bridge).
  - Antigravity: ACP session/new, HTTP if the agent advertises mcpCapabilities.http, else the stdio bridge; tool
    exposure deferred-proxy. Never written to the project's .agents/mcp_config.json.
  - OpenCode: per-chat server name `fridae-<chat id>` (or `fridae-<16 hex sha256>` if >64 chars) added via its API or
    per-session config Fridae owns, with permission rules denying `fridae-*` and allowing its own. Never on the user's
    own opencode server.
- Rotating a key must not change the settings hash that decides whether a kept-alive AI process restarts.

## 3. Wiring and exposure
- Server name `fridae` (tools `mcp__fridae__chat_launch`), pre-allowed like mcp__rbxkit (`mcp__fridae__*`).
- Stdio bridge for ACP agents without HTTP: `Fridae.exe --mcp-bridge` with env FRIDAE_MCP_ENDPOINT and
  FRIDAE_MCP_AUTHORIZATION, an exact port of AcpMcpStdioBridge.ts (split on \n only, POST each line verbatim, carry
  session id + protocol version, initialize barrier, client responses skip it, -32603/-32700 lines, no reconnect, exit
  on stdin EOF after in-flight; missing env → stderr + exit 2). Fast path in main before any startup work.
- Deferred tools for Antigravity only (header `fridae-tool-exposure: deferred-proxy`): tools/list = fridae_capabilities,
  tool_search, tool_call; tool_search results ≤ 8,000 bytes with T3's scoring and fallbacks. Others: full list minus
  tool_search/tool_call.

## 4. Tools (T3 descriptions word for word with `t3_` dropped, thread→chat, T3 Code→Fridae)
| Fridae tool | T3 tool | Fridae call | Access |
|---|---|---|---|
| fridae_capabilities | orchestrator_capabilities | AIs, models, efforts, accounts | reads |
| chat_list | t3_thread_list | agents list | reads |
| chat_search | t3_thread_search | titles + project names, then message text | reads |
| chat_read (chatId required) | t3_thread_read | history, paged, 30,000-byte cap | reads |
| chat_launch | t3_thread_launch | new_agent + set_project + first message | startsThreads |
| chat_send (chatId required) | t3_thread_send | Agent ask (queues when busy) | writesThreads |
| chat_wait (chatId required) | t3_thread_wait | waits on the turn, 30/45 s | reads |
| chat_interrupt (chatId required) | t3_thread_interrupt | stop (+ studio pause_if_lead) | writesThreads |
| chat_update | t3_thread_update | rename | writesThreads |
| chat_organize | t3_thread_organize | close/restore (archive), pin, mark unread — only what Fridae has | writesThreads |
| chat_configure | t3_thread_configure | read; set AI/model/effort/account | writesThreads + refuseAgentModePick |
| chat_queue | t3_queue | list/read/cancel queued messages (edit/reorder only if Fridae supports) | reads / writesThreads |
| project_list, project_read | same | projects list | reads |
| project_create | t3_project_create | projects create (new folder) or add (existing folder → always a card) | writesEnvironment |
| project_update | t3_project_update | rename, kind | writesEnvironment |
| project_delete | t3_project_delete | forget (folder stays; non-empty needs force) | writesEnvironment + always a card |
| settings_read | t3_environment_read | identity + settings in Setup's words, never secrets | reads |
| settings_update | t3_environment_preferences_update | a short fixed list of ordinary settings, each via its own handler; guarded ones only via a card | writesEnvironment |
| theme_read, theme_set | same | built-in themes + light/dark/system, applied to the window; "only when the user asks" | writesEnvironment |
| tool_search, tool_call | same | deferred-proxy only | readsAsCaller |
Chat ids default to "this chat" only where T3 defaults (update, organize, configure, queue).
Ids only; a miss suggests the right id ("Did you mean …"). Results carry a `link` (Fridae chat/project link format,
rendered as chips) that agents quote verbatim.

## 5. Access (port of McpToolAccess.ts, threadAccess.ts, delegatedTaskModes.ts)
- Fridae modes map to T3 ranks: a chat in Full auto or Studio = full-access; Ask-me/default = below (approval-required);
  Autopilot/Overnight add no rank. Interaction mode: plan vs default if Fridae has a plan mode, else all default.
- Unchanged from T3: every write needs a live caller (a turn running, chat not closed, same AI as the key) else
  parent_not_active; writesThreads: target's mode not above the caller's, refused with T3's message; startsThreads:
  launched chat inherits the caller's mode, may ask lower, never higher; refuseAgentModePick: the toolbox can never
  answer a permission prompt, approve a card, or change a chat's mode; subagents (experts) get their own key flagged
  subagent with the same rules and are told "As a subagent or child, do the work yourself."
- The user's spin: writesEnvironment from a chat below Full auto shows an Allow/Decline card (T3 refuses); in Full auto
  it just happens. Always a card, in every mode: project_delete, any guarded setting (paths::GUARDED), adding an
  existing folder as a project, anything that publishes or spends.

## 6. Approval card (Fridae's own; mechanism ported from T3's request_secret)
- The tool records a pending card on the calling chat (a new BUS event, like texrequest), holds the call, polling every
  500 ms. Ends when the user answers; when the run ends or restarts (closed as cancelled); after 10 min (timed_out,
  distinct from declined). Client tool timeouts raised to 65 min.
- Notes adapted from contracts/secretRequest.ts:36-43: declined "The user declined. Don't ask again in chat; say what's
  blocked and go on without it."; cancelled "The card closed with the run. Ask again with a new clientRequestId only if
  it is still needed."; timed_out "No answer in time. Don't ask in chat; ask again with a new clientRequestId only if it
  is still needed."
- The answer goes from the card to the app through a new page route, never through the agent; the card's id is
  unguessable and bound to the chat.
- Look: SecretRequestCard.tsx's layout in Fridae's style — bordered, a title saying the action ("Rename project"), one
  plain sentence, a primary button naming the action ("Rename it") and a quieter "Don't". Answered: one compact row
  "Rename project · allowed".

## 7. Live UI
- Tools call the same Rust functions the routes call; chat changes already reach the page via agents::BUS. Add BUS
  events for projects, settings, accounts and theme, and refresh the page's store on them.
- Theme and appearance live in the page's localStorage, so theme_set sends an event the open window applies and keeps.
- Tool rows: port t3McpToolPresentation.ts — [action, running, done, detail] per tool, failed/declined forms, counted
  summaries ("Renamed 1 project"), with Fridae's mark. Chat and project links render as clickable chips.

## 8. Agent instructions
Port orchestrationInstructions.ts sections (each < 4,000 bytes), adapted: "Ordinary coding tasks need no Fridae
tools."; never switch AI unasked; launch a new chat only when the user asks ("subagent", "delegate" and "parallel" never
mean one); read a chat before answering about it; mention chats by their `link` verbatim; "As a subagent or child, do
the work yourself."; restyle only when asked; "Fridae tools may load late. Try the name once before you call it absent."
Delivery: Claude system-prompt append; Codex developer instructions; OpenCode session instructions; Cursor/ACP prompt
prefix, resent after compaction.

## 9. Phase 2 (only after the user approves the list)
Port delegate_task, task_status, task_cancel; remove rbx_team spawn/status/wait/models. Keep the swarm board, studio_turn,
rbx_board, rbx_job, the vault, skills, guides, images, video and all Roblox tools.

## 10. Credit
THIRD_PARTY.md entry for T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.) and the licence text in licenses/.

## Testing
Port T3's access tests (McpToolAccess.test.ts, threadAccess.test.ts) and bridge tests; HTTP admission tests; key
lifecycle tests; tool tests against the real agents/projects functions; page tests for cards, rows and live updates;
an end-to-end check per installed AI that chat_launch makes a chat appear in the sidebar. Tests bind 127.0.0.1 only and
never contact github.com.
