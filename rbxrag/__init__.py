"""Fridae's local knowledge index (RAG): hybrid dense + sparse retrieval with a rerank, over the vault, Fridae's guides
and team docs, the user's notes and the current game's docs. Nothing here makes a network call while answering a
query; downloads happen only in the setup steps the user starts."""
