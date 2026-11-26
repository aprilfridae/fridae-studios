# Moving Fridae's front end to Preact, one screen at a time

Approved by the user on 2026-11-26.

## Why

`ui/index.html` is one vanilla file of about 7,600 lines. It has outgrown that: every change means wading through it, parallel helpers collide in it, screens are repainted by hand, and nothing catches a misspelled name before a click. The user chose Preact (tiny, React-like, so T3 Code's React code carries over almost line for line) and chose to move one screen at a time so Fridae keeps working throughout. The app must stay as light as possible.

## Goals

- New screens written as Preact components in TypeScript (TSX), one folder per screen under `ui/src/`.
- One bundled file, `ui/app.js`, made by the standalone esbuild binary. No Node, no npm, no `node_modules`; the build never downloads anything.
- The old page and the new screens live side by side until the last screen has moved; then the old code goes.
- Each step is built, tested, installed and usable on its own.

## Non-goals

- No type checking for now (it needs Node; the user declined it for now). esbuild strips types and catches syntax errors only.
- No component libraries, no CSS framework, no second icon set. Our own small components and the page's existing CSS variables and themes.
- No change to the Rust side's API (`/api/*`, the event stream) beyond what a screen strictly needs.

## Anti-goals

- A heavier or slower app. `app.js` stays small (Preact itself is about 4 KB); no runtime downloads.
- A broken in-between state: at every step every screen works, old or new.

## Constraints

- Vendored, checksummed downloads (both MIT, verified against the npm registry's sha512 on 2026-11-26):
  - `tools/esbuild.exe` from `@esbuild/win32-x64` 0.28.2.
  - Preact 11.2.0 under `ui/vendor/preact/` (the package's `dist` builds and type files, plus its LICENSE).
  - Their licences go into THIRD_PARTY.md.
- Mac builds (`build_mac.py`) need esbuild for macOS; until it is vendored, the Mac build uses a committed `ui/app.js` (the bundle is committed after every change so a build never depends on running esbuild).
- `build_app.py` runs `tools/esbuild.exe` before its page check, fails the build if bundling fails, and ships `ui/app.js` next to `index.html` (`server.rs` serves files from `ui/`).

## Approach (decided)

- **Mounting:** `index.html` loads `app.js` (a module script, after the old script). Each moved screen registers a mount function; the old page, where it used to paint that screen, calls `fridae.mount("<screen>", element)` and stops painting it itself.
- **Shared state:** a small store in `ui/src/store.ts` (a minimal hand-written subscribe/notify store with a `useStore` hook, to stay tiny; no extra packages) holding what the page already fetches: projects, chats, limits, review items, settings. The old page writes into it where it already receives data (status poll, the event stream); new screens read from it and call the same `api()` helper. One source of truth while both exist.
- **Styling:** components reuse the page's CSS (classes and variables in `index.html`); a screen's own CSS moves into a `.css` file next to it that esbuild bundles into `app.css`.
- **Order:** 1) setup + Review, 2) Settings and Usage, 3) sidebar (Overview, project menu), 4) chat (message list with only the visible part rendered, and the message box).

## Step 1 in detail (setup + Review)

- Vendor esbuild and Preact as above; add `ui/build.py` (or a function in `build_app.py`) that runs esbuild: entry `ui/src/main.tsx`, `--bundle --format=esm --jsx=automatic --jsx-import-source=preact`, alias `preact` to the vendored copy, minified output `ui/app.js` and `ui/app.css`.
- `ui/src/main.tsx` exposes `window.fridae = { mount, store }`.
- Port the current Review page (tabs per project, picture items left / plain items right, uniform thumbnails, lightbox, Approve / Ask for changes / Done, "Nothing needs you") to `ui/src/review/` with identical look and behaviour, and delete its old code from `index.html`.
- Checks: the existing headless page check in `build_app.py` must pass with `app.js` loaded; the mock-server harness used for Review (`H:/rust/tmp/rvwork`) must pass its Review checks against the new screen; a real install shows the same Review.

## Left to the builder

File layout inside `ui/src/`, store details, and how the old page hands data to the store, as long as the constraints above hold.
