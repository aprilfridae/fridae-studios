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

## Stack — recommended: bundled and embedded, no daemons (decision for the build)

Fridae is free, open-source and zero-setup for anyone who installs it, so it should NOT require every user to run and
maintain an always-on Qdrant server + Ollama + launchd like the reference setup. Recommended:

- **Vector store**: Qdrant in its local no-server mode (`QdrantClient(path=...)`) or an equally embedded store
  (sqlite-vec / hnswlib) — a file under the app's data folder, no daemon.
- **Embeddings**: FastEmbed (ONNX, CPU, pip-installable, no Ollama) — a small dense model (e.g. `bge-small-en-v1.5`)
  plus FastEmbed BM25 sparse.
- **Reranker**: a small ONNX cross-encoder via FastEmbed, on demand, no separate process.
- All pip/uv-installable and bundled with the Fridae tools (PyInstaller), so there is nothing for the user to install
  or keep running.

Alternative, only if the user asks for it: the reference doc's exact Qdrant-server + Ollama + launchd setup (more
headroom, but the user maintains the services). Settle this at build time; the user leans to "just do it", default to
the recommended embedded stack.

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
- Requiring a background Qdrant server, Ollama, or launchd on a normal user's machine.
- Indexing a user's whole disk or anything outside Fridae's vault, notes and game folders.
- Sending any of this knowledge to an external service (all local).

## Open for the build
- The embedded store and the exact small models (measure recall on real Roblox queries; the hybrid + rerank shape is
  fixed, the specific models are not).
- Whether `rbx_vault` is upgraded in place or a new `rbx_rag` tool is added and `rbx_vault` points at it.
- Index storage location and size budget; first-build time and progress shown in the app.

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
