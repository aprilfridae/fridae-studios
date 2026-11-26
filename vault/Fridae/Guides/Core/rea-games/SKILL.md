---
name: rea-games
description: Reverse-engineer a game's behavior and rebuild it in another engine (web, Godot, Bevy, Unity) with proof it matches, frame by frame. Use when the user wants to port, remake or understand how an openly licensed game works, or says "rea this game".
examples:
  - "Port this open-source web game to Godot and prove it plays the same"
  - "Work out exactly how this game's physics behaves"
  - "rea this game"
applies: both
source: https://github.com/erkamyaman/rea-games
licence: MIT
credit: rea-games by Erkam Yaman and contributors (MIT, see LICENSE)
---

# rea-games

You rebuild a game from what it **does**, and prove it. The loop is: record the original, write the spec, build the
port, verify, fix, repeat. Never call a port done until `rea-games verify` says every case is identical.

## Setting up (Fridae)

The tool ships with this skill, in its `tool/` folder (`bin/rea-games.mjs`, `lib/`, `package.json`; no packages to
install). It writes each game next to itself, so never run it inside the skill's folder:

1. Copy the skill's whole `tool/` folder into the user's project as `rea-games/` (once per project; if it is already
   there, use it).
2. Run every command from the project folder as `node rea-games/bin/rea-games.mjs <command>`. Where these
   instructions say `rea-games <command>`, that is what they mean. Games go in `rea-games/games/<game>/`.
3. Check the tools: `node -v` must be 22 or newer. For a browser game, `node -e "require('playwright')"` must work and
   a Chromium must be installed (`npx playwright install chromium`). If Node or Playwright is missing, tell the user
   what is missing and how to install it; do not install software without their yes.

The upstream project's three worked examples (with recorded cases and ports) are not shipped here; they are in the
repository named in `source` above, under `games/`.

## Before you start

1. Find out **what the original is and what licence it has**. A clean-room rebuild of behavior is fine; copying
   its art, music, names, or code is not, unless the licence allows it. Open source or permissive originals are
   the ones to start with. If the licence is unclear, say so and ask before going on. Never use a trademarked name
   for the rebuild, and give it its own art, sounds and name.
2. Check the tools (above).

## The loop

1. **Scaffold**: `rea-games new <game>` makes `games/<game>/` with `game.json`, `record.mjs`, `GAME.md` and a port stub.
   (This `GAME.md` is the rebuild's behavior spec inside `rea-games/games/<game>/`; it is not the project's own
   bible, `PROJECT.md`.)
2. **Make the original deterministic.** Three things vary between runs: time, random numbers, and input timing.
   - Replace the clock and `requestAnimationFrame` with a fake you step by hand (`dt` per step is an input).
   - Replace `Math.random` with a seeded generator and **save every value it returns** in the case as `tape`. The
     port reads those numbers back in the same order. If the original draws random numbers for things that do not
     change play (clouds, particles), give those a separate stream and say so in `GAME.md`.
   - Never edit the original. Wrap the browser, not the game.
3. **Record cases** (`rea-games record <game>`): each case is `{tape, inputs, trace}`. A trace entry is the state
   after one input or frame. Read it from the live game, or, when the state is private, from what it draws.
   Cover: every kind of enemy/obstacle/item, win, lose, restart, the edges (empty, full, maximum speed), odd frame
   times, and at least one saved state that is one step from each ending. Look at what the cases really contain.
4. **Write `GAME.md`** from the recording: constants, the order of operations inside one step, formulas, quirks
   (including bugs the original has: keep them, the port must match), and what is **not** modelled and why that
   cannot change the trace.
5. **Write the port** as pure logic: no DOM, no clock, an injected `rng`. A runner reads a case on stdin and prints
   `{trace, tapeUsed}`. Do not translate the original line by line; write from `GAME.md`, then check.
6. **Verify**: `rea-games verify <game> --port <name>`. The first difference is printed with its path
   (`trace[412].player.y: 46 != 45.5`). Fix the port, not the case. Use the number, not a guess.
7. **Break it on purpose** once (change a constant, an offset, an off-by-one) and see `verify` fail. A check that
   cannot fail proves nothing.
8. **Play it** in the engine: a web page with your own art, then Godot/Bevy/Unity ports that run the same cases.
   An engine port counts as verified only when it runs the cases through a runner; otherwise label it "not verified".
   Godot ports use the binary in `REA_GODOT` when it is set; Rust/Bevy ports need `cargo`.

## Rules

- Traces come only from running the original. Never write a trace by hand.
- Do not loosen a comparison to get green (no rounding, no tolerance) unless `GAME.md` says why and how much.
- Report honestly: cases, inputs, what is covered, what is not, what fails.
- Record each original's licence in a `NOTICE` file in `rea-games/`.
