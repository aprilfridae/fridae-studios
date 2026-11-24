# Fridae RAG (Off / Light / Heavy) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `rbx_vault` search answers from a local hybrid RAG (dense + sparse fused with RRF, then a cross-encoder rerank) over Fridae's vault, guides, team docs, the user's notes and the current game's docs. The user picks Off, Light or Heavy in Setup, and Heavy has a guided in-app setup.

**Architecture:** A Python package `rbxrag` holds the chunking, indexing, querying and setup steps. One process the app manages, `fridae-cli rag serve`, keeps the models and the index open and answers on 127.0.0.1 with a token. The tool server's `rbx_vault` search asks that process and falls back to today's keyword search on any failure. The Rust side owns the `rag_mode` setting, starts and stops the RAG process (and Qdrant for Heavy), runs the setup steps the way it runs the promo-engine install, and passes the URL and token to the tool servers. Light and Heavy share the code path and differ only in the dense embedder, the reranker model, and whether Qdrant is embedded (local mode, on disk) or a served native binary.

**Tech Stack:** Python 3.14, `fastembed` 0.9.x (ONNX: dense, `Qdrant/bm25` sparse, TextCrossEncoder rerank), `qdrant-client` 1.20.x (local mode and HTTP), Ollama HTTP API, Qdrant native binary; Rust (`ureq`, `tiny_http`, `procs`); `ui/index.html`.

**Spec:** `docs/superpowers/specs/2026-11-23-fridae-rag-design.md`

## Rulings (decisions the spec left open; each with why and the cost if wrong)

- **One RAG process for both stacks, managed by the app.** Qdrant local mode allows one opener per folder, and every expert has its own tool server, so loading the models in each would cost seconds and hundreds of MB per expert. The user still installs nothing for Light. Cost if wrong: a process to supervise, but `procs` already does this.
- **Retrieval units are rbxvault's sections**, vault notes included, parsed by the existing `parse_sections`. A section longer than the model's window is split by real tokens: at most **480 tokens** per chunk (512 window, 32 margin). Each chunk's text starts with `"<title> › <heading>\n"`. Results then map straight onto today's `read` arguments.
- **Models.** Light: dense `BAAI/bge-small-en-v1.5` (384 dims), sparse `Qdrant/bm25`, rerank `Xenova/ms-marco-MiniLM-L-6-v2`. Heavy: dense Ollama `mxbai-embed-large` (1024 dims), sparse `Qdrant/bm25`, rerank `BAAI/bge-reranker-base` (fastembed TextCrossEncoder, kept loaded in the RAG process, which is the spec's "kept reranker").
- **Models are downloaded once, at setup**, into `<work>/rag/models` (fastembed `cache_dir`). The serving process sets `HF_HUB_OFFLINE=1`, so a query makes no network call.
- **`rbx_vault` is upgraded in place.** No new tool, and agents see the same output format.
- **Storage:** `<work>/rag/light/` (Qdrant local path plus `manifest.json`), `<work>/rag/heavy/manifest.json` with Qdrant storage in `<work>/rag/qdrant-storage/`, and the binary in `<work>/rag/bin/qdrant`. Each mode keeps its own index, so switching back is instant, and a stale or missing index is built when its mode starts. All of `<work>/rag` goes on `paths::app_protected()`.
- **Game docs are indexed lazily** on the first query that names a project, under payload `project=<abs path>`. Only `rbxvault.game_files(project)` and that game's team overrides are indexed, never anything else in the folder.
- **Qdrant (Heavy):** the latest stable release asset `qdrant-aarch64-apple-darwin.tar.gz` (or `qdrant-x86_64-apple-darwin.tar.gz`) from `github.com/qdrant/qdrant/releases`, pinned in code. It is started by the app with env `QDRANT__SERVICE__HOST=127.0.0.1`, `QDRANT__SERVICE__HTTP_PORT=<free port>`, `QDRANT__SERVICE__GRPC_PORT=<free port>`, `QDRANT__SERVICE__API_KEY=<token>`, `QDRANT__STORAGE__STORAGE_PATH=<work>/rag/qdrant-storage`, and `QDRANT__TELEMETRY_DISABLED=true`. It never uses 6333 and never touches a Qdrant the user runs themselves.
- **Ollama (Heavy):** used if `http://127.0.0.1:11434/api/version` answers. Otherwise the Install step downloads `https://ollama.com/download/Ollama-darwin.zip`, unzips `Ollama.app` into `~/Applications` and opens it. That only happens when the user presses that step's button. Afterwards it is the user's app: Fridae never stops, updates or removes it. The model is pulled through `POST /api/pull` with streamed progress. Embeds use `POST /api/embed` with `"truncate": false`; on a context error the chunk is halved and retried (the reference doc's silent-truncation lesson).
- **Windows:** not in this plan. Heavy shows "Mac only for now" when `cfg!(windows)`. Light code stays portable.
- **Builders:** these are hard builds and UI, so they run on Claude (no DeepSeek/GLM), per the user's cheap-model rule.

## Global Constraints

- All local: no network call during a query; downloads only in setup steps the user started.
- Indexes only the vault, Fridae's guides and team docs, the user's notes (`rbxnotes.yours_files()`), and the current game's `game_files`/team overrides. Never anything else on disk.
- Degrades gracefully: if the RAG process is off, down, slow (> **3 s**) or errors, `rbx_vault` gives today's keyword answer, never an error.
- Hybrid is fixed: dense + sparse, fused with Qdrant RRF (prefetch **40** each), then rerank the top **30** to `k`.
- Incremental sync: re-embed only files whose `(mtime_ns, size)` changed, and delete the points of removed files.
- Never print or log the RAG token, the `fridae-url.txt` token, or any service password.
- Never modify the user's own Ollama models beyond pulling `mxbai-embed-large`. Never stop the user's Ollama.
- No builder or test runs a real download, a real Ollama or a real Qdrant. Tests use stand-ins (fake embedder, Qdrant `:memory:`, a stub HTTP server). Live checks are the controller's job after install.
- Setting key `rag_mode` with values `"off" | "light" | "heavy"`, default `"off"`, in `paths::GUARDED`.
- Env passed to tool servers: `FRIDAE_RAG_URL` (e.g. `http://127.0.0.1:<port>`) and `FRIDAE_RAG_TOKEN`, only while the RAG process is ready.
- UI copy is plain and adapts to project type (game wording vs generic) through the existing vocabulary if present. Otherwise use neutral wording ("your notes, guides and project docs").

## Review Focus

1. **A user edits a note while a studio run is going**, so the next search must see the edit without a full rebuild. Pinned in Task 2 (`test_sync_reembeds_only_changed`).
2. **The RAG process dies mid-run**, so `rbx_vault` must keep answering (keyword) and the app restarts the process at most every 30 s. Pinned in Task 4 (`test_search_falls_back_when_service_down`) and Task 6 (restart backoff test).
3. **An exact Luau name** (`GetPropertyChangedSignal`, `DataStoreService`) must surface its note even when the dense model ranks it low. Pinned in Task 2 (`test_exact_identifier_found_by_sparse`).
4. **Switching Light ↔ Heavy while a build runs** must cancel or finish cleanly, with no half-index served as complete. Pinned in Task 3 (`test_status_not_ready_while_building`) and Task 6 (mode switch stops the old process first).
5. **A huge or binary-ish note** (a long code dump) must not crash chunking or overflow the model. Pinned in Task 1 (`test_long_code_section_split_under_limit`).

---

### Task 1: Corpus and token-aware chunks

**Files:**
- Create: `rbxrag/__init__.py`, `rbxrag/chunks.py`
- Modify: `rbxvault.py` (add `vault_section_files()` if needed so vault notes yield section docs the same way guides do)
- Test: `tests/test_rag_chunks.py`

**Interfaces:**
- Produces:
  - `Chunk = dict` with keys `id: str` (uuid5 of `f"{shelf}|{path}|{section_index}|{part}"`), `shelf: str` (one of `rbxvault.SHELVES`), `title: str`, `heading: str`, `path: str`, `rel: str`, `i: int` (section index, as in rbxvault docs), `part: int`, `text: str`, `project: str` ("" unless the game shelf).
  - `def corpus_files(project: str | None) -> list[tuple[str, str, str]]` returns `(shelf, title, path)` for the kit vault notes, guides, team (with the game's overrides when `project` is given), yours, and game.
  - `def chunks_for_file(shelf: str, title: str, path: str, count_tokens: Callable[[str], int], limit: int = 480, project: str = "") -> list[Chunk]`
  - `def make_token_counter(tokenizer_json: str | None) -> Callable[[str], int]` uses the `tokenizers` library when a `tokenizer.json` path is given, otherwise a fallback of `ceil(len(text.encode()) / 3)` (pessimistic, for tests and when no model is present).

- [ ] **Step 1: Write the failing tests** in `tests/test_rag_chunks.py` (unittest):
  - `test_chunk_prefix_and_ids_stable`: two calls on the same file give the same ids, and each `text` starts with `"<title> › <heading>\n"`.
  - `test_long_code_section_split_under_limit`: a section of 20,000 chars of code gives several parts, every `count_tokens(part.text) <= 480`, and `part` runs 0..n-1.
  - `test_corpus_files_scoped`: with a temp project holding `GAME.md`, a `.fridae/notes/x.md` and an unrelated `src/big.lua`, the result includes the first two and not `src/big.lua`. With `project=None` there is no `game` shelf.
  - `test_vault_notes_become_sections`: a vault note with two `##` headings gives at least two chunks with `shelf == "vault"` and `i` matching `rbxvault.parse_sections` order.
- [ ] **Step 2: Run them and confirm they fail.** Run `.venv/bin/python -m unittest tests.test_rag_chunks -v`; it fails with ImportError.
- [ ] **Step 3: Implement** `rbxrag/chunks.py`, reusing `rbxvault.parse_sections`, `guide_files`, `team_files`, `yours_files` and `game_files` (no second parser). Split by token count on paragraph and then line boundaries, and hard-split a single over-long line by tokens.
- [ ] **Step 4: Run them and confirm they pass**, plus `tests.test_knowledge` (rbxvault unchanged behaviour).
- [ ] **Step 5: Commit** with `feat(rag): corpus and token-aware chunks`.

### Task 2: Index and hybrid query engine

**Files:**
- Create: `rbxrag/engine.py`
- Test: `tests/test_rag_engine.py`
- Modify: `requirements` / `install.py` dependency list to add `fastembed==0.9.*` and `qdrant-client==1.20.*` (and install them into `.venv`)

**Interfaces:**
- Consumes: Task 1 `corpus_files`, `chunks_for_file`, `Chunk`.
- Produces:
  - `class Embedders(Protocol)`: `dense(texts: list[str]) -> list[list[float]]`, `dense_query(text: str) -> list[float]`, `sparse(texts) -> list[(indices: list[int], values: list[float])]`, `sparse_query(text)`, `rerank(query: str, texts: list[str]) -> list[float]`, `dim: int`, `count_tokens(text) -> int`.
  - `class Engine`: `__init__(client: QdrantClient, emb: Embedders, manifest_path: str, collection: str = "fridae")`, `sync(project: str | None = None, progress: Callable[[str, float], None] = None) -> dict` (returns `{"chunks": int, "files": int, "embedded": int, "removed": int}`), `query(q: str, shelves: list[str], k: int, project: str | None) -> list[dict]` (each hit is a Chunk's payload plus `score: float`), `ready: bool`.
  - The collection has named vectors `dense` (cosine, `emb.dim`) and sparse `sparse` (with `modifier=IDF`). Payload indexes on `shelf`, `project` and `path`.
  - The manifest is JSON `{ "format": 1, "dim": int, "model": str, "files": { path: {"mtime_ns", "size", "ids": [..]} } }`. A changed `format`, `dim` or `model` means a full rebuild.
  - `query` filters on `shelf ∈ shelves` and on `project == project or project == ""`, fuses with `Prefetch(dense, limit=40)` and `Prefetch(sparse, limit=40)` under `FusionQuery(Fusion.RRF)` limited to 30, then reranks to `k`.

- [ ] **Step 1: Write the failing tests** using `QdrantClient(":memory:")` and a `FakeEmbedders`. The fake's dense vector is a 64-dim hashed bag-of-words, its sparse is hashed term counts, and its rerank is term overlap. The tests:
  - `test_sync_then_query_finds_note`: a temp kit with three notes. After `sync()` the stats show `chunks > 0`, and querying for a phrase from note B returns note B first.
  - `test_sync_reembeds_only_changed`: sync, edit one file (bump its mtime), sync again → `embedded` equals that file's chunk count, and a query sees the new text.
  - `test_removed_file_points_deleted`: delete a file and sync → its ids are gone (count by `path` filter == 0).
  - `test_exact_identifier_found_by_sparse`: the fake dense is replaced by a constant vector (useless), and a query `GetPropertyChangedSignal` still returns the note containing it, through sparse.
  - `test_shelf_and_project_filter`: game chunks of project A never return for project B, and `shelves=["yours"]` returns only yours.
  - `test_model_change_rebuilds`: a manifest with another `model` triggers a full rebuild.
- [ ] **Step 2: Run them and confirm they fail** (`.venv/bin/python -m unittest tests.test_rag_engine -v`).
- [ ] **Step 3: Implement** `rbxrag/engine.py`. Upsert in batches of 64, call `progress(step, fraction)` after each batch, and write the manifest atomically (temp file + `os.replace`) only after the upserts succeed.
- [ ] **Step 4: Run them and confirm they pass.**
- [ ] **Step 5: Commit** with `feat(rag): hybrid index with incremental sync and rerank`.

### Task 3: Real embedders, setup steps and the serving process

**Files:**
- Create: `rbxrag/models.py` (LightEmbedders, HeavyEmbedders), `rbxrag/setup.py` (setup steps), `rbxrag/serve.py` (HTTP server), `rbxrag/cli.py` (`rag` subcommand)
- Modify: `rbx.py` (register `"rbxrag.cli": "rag"` beside `"rbxpromo.cli": "promo"`)
- Test: `tests/test_rag_serve.py`

**Interfaces:**
- Consumes: Task 2 `Engine`, `Embedders`.
- Produces:
  - CLI: `rag serve --mode light|heavy --data <work>/rag --port <n> --token-file <path> [--qdrant-url URL --qdrant-key-file PATH]`. The token is read from a file and never put in argv. The process prints one JSON line `{"listening": port}` when bound.
  - CLI: `rag setup <step> --mode light|heavy --data DIR --json-progress`. Steps: `models` (Light: the three fastembed models; Heavy: the sparse model and the reranker), `ollama-install`, `ollama-pull`, `qdrant-install`. Each prints `{"step": str, "fraction": float}` lines and ends with `{"ready": bool, "missing": [..], "error": str}`, the same shape as `rbx promo install --json-progress`. Every step is idempotent: when its part is already there it ends `ready` at once.
  - CLI: `rag check --mode M --data DIR --json` gives `{"models": bool, "ollama": "missing"|"stopped"|"running", "ollama_model": bool, "qdrant_binary": bool}` with no network except `127.0.0.1:11434`.
  - HTTP (every request needs header `Authorization: Bearer <token>`, otherwise 401):
    - `GET /status` returns `{"mode", "ready": bool, "building": bool, "progress": {"step", "fraction"}, "chunks": int, "error": str}`
    - `POST /query` with `{"q", "shelves": [..], "k", "project"}` returns `{"hits": [...]}`. Before searching it runs a quick `(mtime, size)` sync of changed files, throttled to at most once per **10 s**, plus a lazy sync of `project` if new.
    - `POST /rebuild` returns `{"started": true}`.
  - On start the server binds, then builds or syncs in a background thread. `/status.ready` stays false until the first full sync ends, and `/query` returns 503 meanwhile.

- [ ] **Step 1: Write the failing tests:**
  - `test_requires_token`: start `serve` in-process with a fake-embedder factory and Qdrant `:memory:`. No header or a wrong token → 401.
  - `test_status_not_ready_while_building`: with a factory whose sync blocks on an event, `/status` shows `ready: false, building: true` and `/query` returns 503. Release the event → ready, and `/query` returns hits.
  - `test_query_picks_up_edit_after_throttle`: edit a note, advance the injected clock by 11 s, query → the new text is found.
  - `test_setup_lines_shape`: `setup models` with an injected downloader stand-in prints progress lines and a final `{"ready": true, "missing": []}`. With the stand-in raising, it ends `ready: false` with the error.
  - `test_heavy_embed_shrinks_on_context_error`: `HeavyEmbedders` against a stub Ollama HTTP server that rejects inputs over N tokens with a context error. Embedding a long chunk succeeds after halving, and the request body carries `"truncate": false`.
  - `test_no_network_in_query`: patch `socket.create_connection` to fail for any host but 127.0.0.1, then run a Light query on prepared stand-in models → OK.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement** `models.py` (fastembed `TextEmbedding`, `SparseTextEmbedding` and `TextCrossEncoder` with `cache_dir=<data>/models`; Heavy dense through `urllib.request` to Ollama), `setup.py` (fastembed downloads; Ollama zip download, unzip with `ditto -x -k` into `~/Applications` and `open -a`; the Qdrant tar.gz unpacked to `<data>/bin/qdrant` and `chmod 755`; pulling through the Ollama API stream), `serve.py` (`http.server.ThreadingHTTPServer` on 127.0.0.1, setting `HF_HUB_OFFLINE=1` before importing fastembed), and `cli.py`.
- [ ] **Step 4: Run them and confirm they pass.**
- [ ] **Step 5: Commit** with `feat(rag): models, setup steps and the RAG service`.

### Task 4: `rbx_vault` search through the RAG service

**Files:**
- Modify: `rbxvault.py` (`run` search branch, a new `rag_search(q, shelves, k) -> str | None`), `rbxkit_mcp.py` (the `rbx_vault` description: one sentence that search is by meaning when the knowledge index is on)
- Test: `tests/test_rag_vault.py`

**Interfaces:**
- Consumes: Task 3 HTTP `/query`. Env `FRIDAE_RAG_URL` and `FRIDAE_RAG_TOKEN`, plus `RBXKIT_PROJECT`.
- Produces: `rag_search(q, shelves, k)` returns the rendered text in today's format (rank, `[shelf]`, title › heading, size, a snippet built with `snippet()`, a `read:` JSON line with `heading_arg`), or `None` on any failure: no env, timeout over 3 s, non-200, bad JSON or zero hits. `run(action="search")` tries `rag_search` first for every shelf choice (vault-only included) and otherwise uses the existing code unchanged. The vault-only RAG answer keeps the `api_notes` and symptoms lines.

- [ ] **Step 1: Write the failing tests** with a stub HTTP server:
  - `test_search_uses_rag_when_up`: the stub returns two hits, and the output contains `1. [guides]` and a `read:` line with the hit's title.
  - `test_search_falls_back_when_service_down`: env points at a closed port, so the output equals the keyword search output for the same query (compared with env unset).
  - `test_search_falls_back_on_slow_service`: the stub sleeps 4 s, so the fallback answer arrives within 3.5 s.
  - `test_token_sent_never_printed`: the stub sees `Authorization: Bearer t0k`, and `t0k` is not in the output.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement** with `urllib.request` (timeout 3 s), reusing `combined_search`'s rendering helpers. Factor the per-hit rendering out of `combined_search` into a helper both paths call, with no behaviour change for keyword search.
- [ ] **Step 4: Run the new tests plus `tests.test_knowledge` and `tests.test_notes`, and confirm they pass.**
- [ ] **Step 5: Commit** with `feat(rag): rbx_vault searches through the knowledge index when it is on`.

### Task 5: Packaging

**Files:**
- Modify: `rbxkit.spec` (hiddenimports `rbxrag`, `rbxrag.chunks`, `rbxrag.engine`, `rbxrag.models`, `rbxrag.setup`, `rbxrag.serve`, `rbxrag.cli`; `collect_data_files`/`collect_dynamic_libs` for `fastembed`, `onnxruntime`, `tokenizers`, `qdrant_client`, `py_rust_stemmers`), `build_mac.py` if it pins the dependency list, `THIRD_PARTY.md` (fastembed Apache-2.0, qdrant-client Apache-2.0, onnxruntime MIT, tokenizers Apache-2.0, Qdrant Apache-2.0, Ollama MIT)
- Test: `tests/long_calls_proof.py` unchanged; a new smoke command

- [ ] **Step 1: Build** with `.venv/bin/python build_mac.py`.
- [ ] **Step 2: Smoke-test the packaged CLI:** `"$CLI" rag check --mode light --data "$S/ragtest" --json` prints valid JSON with `"models": false`, and `"$CLI" rag serve --mode light --data "$S/ragtest" --port 0 --token-file "$S/tok"` prints `{"listening": <n>}` and answers `/status` with 200 (the models are missing, so it reports `ready: false` with an error naming the missing models, which is the expected state before setup). Here `$S` is the scratchpad.
- [ ] **Step 3: Run** `tests/long_calls_proof.py "$CLI" --mcp` and expect `PROOF HOLDS`. Record the app size before and after in the report.
- [ ] **Step 4: Commit** with `build(rag): package the RAG libraries`.

### Task 6: App side: setting, processes, setup API, env

**Files:**
- Create: `app-rs/src/rag.rs`
- Modify: `app-rs/src/main.rs` (mod, start on launch, stop on quit), `app-rs/src/paths.rs` (`GUARDED` += `"rag_mode"`, `app_protected()` += `(work.join("rag"), true)`, `pub fn rag_mode() -> String`), `app-rs/src/agents.rs` (`mcp_servers_with` env adds `FRIDAE_RAG_URL`/`FRIDAE_RAG_TOKEN` when `rag::endpoint()` is `Some`; leave `guarded_print` keyed on the mode only, never the token), `app-rs/src/server.rs` (routes)

**Interfaces:**
- Consumes: Task 3 CLI (`rag serve`, `rag setup`, `rag check`).
- Produces:
  - `pub fn endpoint() -> Option<(String, String)>` gives (url, token) only while the RAG process is ready.
  - `pub fn apply_mode(mode: &str)` stops the current RAG process (and Qdrant) first, then starts the one for `mode` if its setup is complete. Called on launch and on a setting change.
  - The token is 32 random bytes in hex, written to `<work>/rag/token` with mode 0600, and passed to the processes by file path.
  - Qdrant (Heavy) is started from `<work>/rag/bin/qdrant` with the env in Rulings, on two free ports, and recorded in `procs` under owner `"rag-qdrant"`. The RAG process is owner `"rag"`. Both are started through `paths::Start`.
  - Supervision: if the RAG process exits unexpectedly it is restarted no more often than every **30 s**, and its last error is kept for `/api/rag/status`.
  - Routes: `GET /api/rag/status` returns `{"mode", "check": <rag check json>, "service": <proxied /status or null>, "setup": {"running", "step", "fraction", "error"}, "windows": bool}`. `POST /api/rag/mode {"mode"}` (validated; writes the guarded setting the way other guarded settings are written). `POST /api/rag/setup {"step"}` runs `rag setup <step> --json-progress` in its own process group, one at a time, like the promo install (`promo.rs` `install_command`/`engine_line`; reuse or copy that line parser). `POST /api/rag/test {"q"}` proxies one query and returns the top 3 titles. `POST /api/rag/rebuild`.

- [ ] **Step 1: Write the failing Rust tests** in `rag.rs`:
  - `mode_is_guarded_and_validated`: `"rag_mode"` is in `GUARDED`, and the `"bogus"` mode is refused.
  - `rag_folder_is_protected`: `app_protected()` contains `work/rag`, recursive.
  - `env_has_rag_only_when_ready`: with `endpoint()` None, `mcp_servers_with` has no `FRIDAE_RAG_URL`. With a test hook set, both keys are present and `guarded_print` is unchanged by the token.
  - `restart_backoff_30s`: the supervisor's `may_restart(last, now)` is false at 29 s and true at 30 s.
  - `setup_lines_parse`: the progress and done lines parse the same way as the promo engine's.
- [ ] **Step 2: Run** `cargo test --manifest-path app-rs/Cargo.toml rag` and confirm they fail.
- [ ] **Step 3: Implement** `rag.rs` and the wiring.
- [ ] **Step 4: Run all Rust tests** (`cargo test --manifest-path app-rs/Cargo.toml`) and confirm they pass.
- [ ] **Step 5: Commit** with `feat(rag): app runs the knowledge index and its setup`.

### Task 7: Setup UI: Off / Light / Heavy with a guided Heavy setup

**Files:**
- Modify: `ui/index.html` (Setup → Connections: a new "Knowledge index" section)

**Interfaces:**
- Consumes: Task 6 routes.
- Produces (copy is fixed here; the layout follows the existing Setup section styles):
  - Title: **Knowledge index**, with the subline "Lets the AIs look up just the right bits of your notes, guides and project docs instead of reading whole files. Runs on your Mac; nothing is sent anywhere."
  - A segmented control with three choices and one line each:
    - **Off**: "Simple word search (what Fridae does today)."
    - **Light**: "Works out of the box. Downloads about 200 MB of small models once."
    - **Heavy (Ollama + Qdrant)**: "Fastest and smartest. Needs Ollama and Qdrant, which Fridae can set up for you."
  - Light chosen and not set up: one **Set up** button runs step `models`, with a progress bar and then "Ready, N pieces indexed".
  - Heavy chosen: a numbered list of 5 steps, each with a state dot (green done, amber needs you, red failed, grey not yet), a short line and its button (**Install** / **Download** / **Start** / **Build**). The steps are 1 Install Ollama (`ollama-install`; done when check says running), 2 Download the embedding model (`ollama-pull`), 3 Install Qdrant (`qdrant-install`), 4 Start the reranker (`models`), 5 Build the index (apply mode → service builds; progress from `service.progress`). A failed step shows its error in one plain line plus **Try again**. Every step can be re-run.
  - A **Test it** box: an input and a button calling `/api/rag/test`, which lists the top 3 titles.
  - Windows: Heavy is disabled with "Mac only for now".
  - Status is polled every 1 s while something runs, and every 10 s otherwise only while the section is visible.
- [ ] **Step 1: Implement** the section, matching the existing look of the Setup sections (the same controls and dot styles as Connections' sign-in rows).
- [ ] **Step 2: Check** in the browser pane against a dev copy with the routes stubbed or live. Click through Off → Light → Heavy and confirm no layout break at narrow width and no console errors. Take a screenshot.
- [ ] **Step 3: Commit** with `feat(rag): Setup section for the knowledge index`.

### Task 8: Final review, merge, build, install, live check

- [ ] Whole-branch review (most capable model) against the spec and the Review Focus.
- [ ] `--no-ff` merge into `feature/ui-overhaul`. Then all Rust and Python tests, the page parse, `build_mac.py`, `PROOF HOLDS`, install, and push.
- [ ] Live check by the controller in the installed app: Light setup, then a search from a chat returns RAG-ranked results, then switch Off and get the keyword answer. Heavy steps are left for the user to press (they install Ollama.app on their Mac).
