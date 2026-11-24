# A local RAG for Fridae's own knowledge

Asked for by the user on 2026-11-23, pointing at a family member's corporate local-RAG setup
(`~/Downloads/local-rag-setup.md`, Qdrant + Ollama + hybrid embeddings + cross-encoder rerank): "ignore all of the
hyperspecific stuff that he has added, and make it hold all of my hyperspecific data instead ... all the roblox dev
stuff, all the information ive told the program to save ... i basically just want a rag set up." Queued: the user said
to build it after the features already in flight.

## Intent

Agents retrieve just the relevant passages of Fridae's knowledge on demand, instead of loading whole guides into
context. Replace today's keyword `rbx_vault` lookup with a real semantic + hybrid RAG over the user's own data.

## What it indexes (Fridae's data, not the family member's corporate sources)

- The Roblox knowledge vault already shipped in Fridae (`vault/Roblox`, the LihFolk Vault V2, ~1,300 notes, 26MB) and
  Fridae's own guides and team docs (`vault/Fridae/Guides`, `vault/Fridae/Team`).
- Everything the user told the program to save: house rules, lessons and notes
  (`~/Documents/Fridae Studios/notes/*.md`), and a game's own notes under its `.fridae/notes`.
- Each game's own knowledge: its `GAME.md`, design docs and lessons under the project folder.
- Not indexed (for now): saved chat transcripts (large, lower value — a later option), the Roblox dev skills (already
  loaded as skills).

Drop every corporate source from the reference doc (Confluence, Jira, Databricks, Hoolihan, his git repos, his Obsidian
vault) and all his machine-specific config.

## What to keep from the reference design (the good parts)

- **Hybrid retrieval**: dense embeddings (meaning) + sparse/BM25 (exact identifiers — Luau API names, service names,
  Jira-style ids, acronyms), fused with RRF, then a cross-encoder rerank. This matters for Roblox/Luau exact-name
  recall.
- **Incremental sync** on file change (mtime/hash), not a full rebuild each time.
- **Token-aware chunking** with a safe truncation margin (the reference doc's char-vs-token bug is a lesson: chunk and
  truncate by real tokens).

## Stack — the heavy local stack, offered as an option (decided by the user)

The user chose the reference doc's full stack for SPEED and TOKEN ECONOMY, and does not mind the setup: it is an
**opt-in option** for users who want it, not forced on everyone. Build it as the reference setup does, with Fridae
helping install and manage the pieces the way it already installs OpenCode and the promo engine:

- **Vector store**: Qdrant as a real local service (native binary, the way Fridae runs its other helpers; not Docker).
  Fridae starts/stops it and keeps it under the app's data folder.
- **Embeddings**: Ollama, dense (e.g. `mxbai-embed-large`, 1024 dims) — the user specifically wants Ollama. Fridae
  installs it (official installer) and pulls the model. Plus a sparse/BM25 vector (FastEmbed `Qdrant/bm25`, CPU) for
  exact-identifier recall.
- **Reranker**: a cross-encoder (e.g. `bge-reranker-v2-m3`), on demand or as a kept reranker process to avoid the
  cold-start load per query (the reference doc's reranker-daemon idea).
- Each point stores named `dense`/`sparse` vectors; a query is embedded both ways and fused with Qdrant's RRF before
  rerank.

Why heavy is the right call here: local Ollama embeddings and rerank cost **no AI-plan tokens**, and retrieving a few
small passages instead of loading whole guides cuts the context tokens an agent spends — the user's SPEED + ECONOMY
goal. It is an option: a user who does not set it up runs on the keyword fallback below, so the heavy stack is never
forced.

### Both stacks ship; the user picks in Setup

The user wants **both** offered as options, chosen in Setup, with a clear setup flow for the heavy one:

- **Light** — bundled and embedded: FastEmbed CPU dense + sparse + a small ONNX reranker, and Qdrant in local
  no-server mode (or sqlite-vec/hnswlib). No Ollama, no daemons, (almost) nothing to install — works out of the box
  for anyone. Lower ceiling, still hybrid + rerank.
- **Heavy** — Ollama + Qdrant service + cross-encoder reranker, as above: more speed and quality, a real setup.
- **Off** — today's keyword lookup.

Both are real builds behind one `rbx_vault`/RAG interface, so the agent side does not care which is on; only retrieval
quality and setup differ. The index is rebuilt when the chosen stack changes.

## Behaviour

- Fridae builds the index on first use and refreshes it incrementally when the vault, the user's notes, or a game's
  docs change (watch mtime; rebuild only changed files). Stored under the app's data folder, per the knowledge set.
- Agents query it through `rbx_vault` (kept, upgraded) or a thin new tool: plain-language query in, top reranked
  passages out, with their source path so the agent can open the full note. Shelves/scopes stay (all-Roblox, this
  game, the user's own notes) as filters on the query.
- A game's own knowledge and the user's notes rank alongside the Roblox vault, so a lesson the user saved surfaces for
  the right task — which is the point of "hold all the information I've told it to save".
- Degrades gracefully: if the RAG index or its deps are unavailable, fall back to today's keyword lookup, never error.
- Respects the app's model/plan rules: embedding and rerank are local/CPU, so they cost no AI plan.

## Anti-goals
- Forcing the heavy stack on a user who did not opt in — it is an option; without it, Fridae uses the keyword fallback.
- Indexing a user's whole disk or anything outside Fridae's vault, notes and game folders.
- Sending any of this knowledge to an external service (all local — Ollama and Qdrant run on the user's machine).

## Setup UI (both options, with a guided heavy setup)
- Setup > Connections gets a "Knowledge index (RAG)" section with the three choices as a clear control: **Off**,
  **Light** (works now), **Heavy (Ollama + Qdrant)**. Each with one plain line on what it gives and costs (Light: works
  out of the box; Heavy: fastest, needs Ollama + Qdrant, all local).
- **Light**: pick it and Fridae builds the index — no external install, a progress bar and a chunk count when done.
- **Heavy — a decently easy, step-by-step setup** the UI walks through, each step with a state dot, Fridae doing what
  it can and showing the exact command/next step for anything it cannot:
  1. Install Ollama (Fridae runs the official installer; if it can't, shows the one command/link).
  2. Pull the embedding model (`mxbai-embed-large`) — progress shown.
  3. Install/locate Qdrant (Fridae installs the native binary and starts it as a managed service; shows status).
  4. Start the reranker.
  5. Build the index — progress and a chunk count; a "test query" button to confirm it works.
  Each step says done/needs-you/failed with a plain fix, re-runnable, idempotent (the reference doc's bootstrap +
  verify, as in-app steps instead of a shell script).
- Fridae manages the Qdrant and reranker processes under its process ledger (start on enable, stop on quit), stored in
  the app's data folder; it does not touch a user's own separate Ollama/Qdrant if they already run one.
- Switching Off / Light / Heavy is a control the user can change any time; changing it rebuilds the index for the new
  stack. When the index is off or a service is down, agents fall back to the keyword lookup, never error.

## Open for the build
- The exact models (the user named Ollama + `mxbai-embed-large`-class dense and a `bge-reranker` cross-encoder from the
  reference doc; confirm recall on real Roblox queries). The hybrid + rerank shape is fixed.
- Whether `rbx_vault` is upgraded in place or a new `rbx_rag` tool is added and `rbx_vault` points at it.
- Index storage location and size budget; first-build time and progress shown in the app.
- Windows (Ollama/Qdrant availability) — later.

## Tests
- Chunking/truncation by real tokens, with the shrink-on-overflow fallback.
- Hybrid retrieval returns the right note for an exact Luau/service name and for a vague task description; the rerank
  improves the order.
- Incremental sync re-embeds only changed files.
- A user-saved note and a game's GAME.md surface for a matching query, scoped correctly.
- Missing/empty index falls back to keyword lookup without erroring.
- All local: no network call during a query.

## Sequencing
After the sidebar tree, the any-number-of-accounts work, and the plain-projects feature land. It is independent of
those, so it can be its own branch when its turn comes.
