# Third-party material in rbxkit

## Design guides from open-design

These folders under `vault/Fridae/Guides/Core/` are copied unchanged (each `SKILL.md` renamed to
`<folder>.md`) from https://github.com/nexu-io/open-design (`skills/`), which is licensed
Apache-2.0 (`licenses/open-design-LICENSE.txt`). Folders that carry their own
`LICENSE` file are under that licence instead (MIT); each folder's
`SOURCES.md` or front matter names its upstream author.

- `brutalist-skill`
- `emil-design-eng`
- `minimalist-skill`
- `motion-design`
- `redesign-skill`
- `reference-design-contract`
- `soft-skill`

## Roblox subject guides from roblox-skills

`vault/Fridae/Guides/Roblox subjects/` is copied unchanged (each `SKILL.md` renamed to `<folder>.md`) from
https://github.com/EL4CTEO/roblox-skills (version 1.1.2), MIT licensed
(`licenses/roblox-skills-LICENSE.txt`). 22 guides (its `roblox-studio-mcp`
guide is left out because rbxkit's own workflow guide covers the same ground): Luau, architecture,
security, networking, data stores, UI, world building, physics, audio, input,
NPC AI, monetization, testing, performance and more.

## Check-UI

The simulator UI look described in `vault/Fridae/Guides/Core/sim-ui` follows Check-UI
Clean Roblox by diggofun / TemporaryLooker22 (MIT). See
`vault/Fridae/Guides/Core/sim-ui/LICENSE.md`.

## LihFolk Vault V2

`vault/Roblox/` (a trimmed copy: 1,308 notes plus the symptom table, the rigs in `18 Rigs and Skeletons`, the Blender build scripts in
`45 Procedural Blender Assets/scripts`, the dated API snapshot) and `vault/Fridae/Guides/LihFolk/` (12 guides with their Luau
modules, examples, scripts and rubrics) are from LihFolk Vault V2 by LihFolk,
https://github.com/lihfolk-cy/v2-vault, licensed CC BY 4.0 (`licenses/lihfolk-vault-LICENSE.md`).
Changes: each guide's `SKILL.md` is now `<folder>.md` inside Fridae's one vault; guide text that pointed at the Claude Code plugin's folders now points at the `rbx_vault` tool;
mentions of LihFolk guides left out (because rbxkit already had one on the subject) point at rbxkit's.
`rbxvault.py` is LihFolk's `_tools/vault.py` search, reading from rbxkit's folders, with rbxkit's section search
added around it. Trimmed from the copy (2026-11): the generated index and router pages `INDEX.md`, `INDEX_SLIM.md`,
`ROUTER.md`, the visual home page `000 Home.md` and the Obsidian `_templates/` (the search replaces them); the LihFolk
plugin's own `Skill - …` notes and `AI Skills Start Here` (they describe that plugin's skills), `Open Questions - Whole
Vault`, and the Concept Sheets note (its images were already left out). Links to removed notes became plain text. The
original is at the link above, and everything removed is in this repository's git history.
Left out: the Obsidian theme, renders and images, the LihFolk UI kit, Figma, asset-pack, council, hook and
benchmark parts, and guides whose subject rbxkit already covered (secure remotes, anti-exploit review,
DataStore safety, performance audit, monetization compliance, UI builder). The LihFolk Asset Pack is not included.

## unslop

`vault/Fridae/Guides/Core/unslop/` (its guide, `unslop.md`, and references) is from unslop by Damir Sokolovsky (@heydamir),
https://github.com/heydamir/unslop, licensed CC BY 4.0 (`licenses/unslop-LICENSE.txt`). Changes: an
"In Roblox" section was added near the top; its Node snapshot and story-video scripts and its HTML examples are
not included.

## Promo renderer (`promo/`)

- **Subtle** by Refs (refs.video), https://github.com/0xpratzyy/subtle, MIT (`licenses/subtle-LICENSE.txt`,
  "Copyright (c) 2026 Refs (refs.video)"). The eight caption looks in `promo/src/captions/` (punch, highlighter,
  karaoke, bottom, impact, glass, ticket, cinema) and the caption rules in `paginate.ts` (numbers as one token,
  one highlight per page, no page across a cut) are adapted from its styles and scripts; each adapted file
  carries Subtle's copyright line. Changes: rewritten as one component over pages laid out in Node from font
  metrics, tuned for 9:16 promos and platform safe areas.
- **Fonts** in `promo/public/fonts/` (Inter, Montserrat, Poppins, Libre Baskerville, Anton), SIL Open Font
  License 1.1, licence texts beside them (`OFL-*.txt`). Taken from Subtle's `fonts/` (Inter and Montserrat are
  static weights Subtle cut from the upstream variable fonts); the others are unchanged Google Fonts files.
- **Remotion** (`remotion`, `@remotion/*`) is installed by the promo engine, not bundled. Its licence is free for
  individuals, non-profits and for-profit teams of up to 3 people; larger companies need a Remotion company
  licence to use the promo maker (https://www.remotion.dev/docs/license).
- **Mediabunny** (MPL-2.0), installed with the engine, reads clip sizes and lengths in `promo/scripts/render.mjs`.

## Promo engine (downloaded at install, not bundled)

`rbx promo install` (Setup, Connections, Promo engine) downloads these into the engine folder
(the per-user app data folder in the app, e.g. `~/Library/Application Support/Fridae Studios/engine`). Each keeps its own licence.

- **Node.js** (MIT, https://github.com/nodejs/node/blob/main/LICENSE): the portable runtime from nodejs.org,
  checked against its `SHASUMS256.txt`.
- **Remotion**: see above (https://www.remotion.dev/docs/license).
- **Chrome Headless Shell**, downloaded by Remotion (`remotion browser ensure`) to render frames. Chromium is
  under a BSD-style licence plus the licences of its components (https://www.chromium.org/chromium-projects/;
  the full list ships with the browser as `chrome://credits`).
- **Whisper** speech recognition, run with **@huggingface/transformers** (Apache-2.0,
  https://github.com/huggingface/transformers.js) and the **onnx-community/whisper-base.en_timestamped** model
  (MIT, converted from OpenAI's Whisper weights, https://github.com/openai/whisper): word timings for voices
  and song lyrics.
- **kokoro-js** (Apache-2.0, https://github.com/hexgrad/kokoro) with the **Kokoro-82M** voice model
  (Apache-2.0, https://huggingface.co/hexgrad/Kokoro-82M; the ONNX build `onnx-community/Kokoro-82M-v1.0-ONNX`):
  the local voice-over.

## Knowledge index (`rbxrag/`)

Bundled in the app's tools (each keeps its own licence; the licence files ship inside the bundle with each
package's metadata):

- **FastEmbed** by Qdrant (Apache-2.0, https://github.com/qdrant/fastembed): dense, sparse (bm25) and reranking
  models run locally.
- **qdrant-client** (Apache-2.0, https://github.com/qdrant/qdrant-client): talks to the Qdrant service in Heavy.
  It brings **grpcio** (Apache-2.0), **protobuf** (BSD-3-Clause), **pydantic** (MIT), **httpx** (BSD-3-Clause),
  **h2** (MIT) and **portalocker** (BSD-3-Clause).
- **ONNX Runtime** by Microsoft (MIT, https://github.com/microsoft/onnxruntime): runs the models on the CPU. Its
  third-party notices ship beside it (`onnxruntime/ThirdPartyNotices.txt`).
- **Tokenizers** by Hugging Face (Apache-2.0, https://github.com/huggingface/tokenizers) and **huggingface_hub**
  (Apache-2.0) with **hf-xet** (Apache-2.0): counting tokens, and fetching the models at setup.
- **py-rust-stemmers** by Qdrant (MIT, https://github.com/qdrant/py-rust-stemmers), **mmh3** (MIT),
  **loguru** (MIT) and **tqdm** (MPL-2.0 and MIT): used by FastEmbed's bm25 and its logging.

Downloaded at setup, only when the user presses that step's button, not bundled:

- **Models** from Hugging Face into the app's own `rag/models` folder: `BAAI/bge-small-en-v1.5` (MIT),
  `Qdrant/bm25` (Apache-2.0), `Xenova/ms-marco-MiniLM-L-6-v2` (Apache-2.0, an ONNX build of
  `cross-encoder/ms-marco-MiniLM-L-6-v2`) for Light; `BAAI/bge-reranker-base` (MIT) for Heavy.
- **Qdrant** (Apache-2.0, https://github.com/qdrant/qdrant): the release binary, pinned with its SHA-256, into the
  app's own `rag/bin` folder (Heavy).
- **Ollama** (MIT, https://github.com/ollama/ollama): the user's own app in `~/Applications`, installed only when
  none is there (Heavy). Fridae never stops, updates or removes it. Its `mxbai-embed-large` model (Apache-2.0,
  by Mixedbread) is pulled through Ollama.

## Ideas

The promo maker borrows ideas, not code, from **OpenMotion** by nexu-io (Apache-2.0,
https://github.com/nexu-io/openmotion): a recipe file for each video (its Motion.md) that the agent writes and
the engine renders; and from **Short Video Maker** by gyoridavid (MIT,
https://github.com/gyoridavid/short-video-maker): scenes as the unit of a video, captions timed with Whisper,
and a free local Kokoro voice.

The OpenCode chats (`app-rs/src/opencode.rs`) follow the design of **t3code**'s OpenCode 2.x provider by T3 Tools Inc.
(MIT, https://github.com/pingdotgg/t3code): one `opencode serve` per app run on 127.0.0.1 behind a random password,
and the order of its calls (session, tool servers, prompt, event stream, interrupt). No code is copied; the Rust
client was written against OpenCode's own API description.
