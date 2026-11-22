---
name: player-testing
description: Test a Roblox game the way a player meets it rather than the way its builder knows it - the recorded new-player run, feel checks with timed captures, performance budgets for mobile, device layouts, multi-player checks, and the report. Use when judging whether something is fun, clear or ready.
---

# Player testing

A mechanic that works is not yet a game that is clear, fair or pleasant.
This guide is about judging the experience. The mechanics of running a
playtest (starting and stopping, clicking by coordinates, keys, proximity
prompts, reading state) are in the `playtest` guide: load it too, and
follow its rules. They are not repeated here.

The builder's problem: you know where everything is and what every button
does. A new player knows nothing. The tests below are built to take that
knowledge away from you.

## The new-player run

The most valuable test. Run it after any change to the start of the game,
the core loop, the HUD or the economy, and before calling a game ready.

1. Reset to a fresh player in edit mode if the game saves data (see the
   `workflow` guide; it deletes saved data, so confirm with the user).
2. Start the playtest.
3. Fetch the `session_recorder` snippet with `rbx_studio_snippet`, read
   its header comment for the exact calls, and start it. It logs what the
   player does and receives over time.
4. Play the first five minutes **as someone who has never seen the game**:
   - Simulated input only: `user_mouse_input`, `user_keyboard_input`,
     `character_navigation`.
   - Decide each action from a `screen_capture` of what is on screen. Go
     where the screen points, press what looks pressable, read only the
     text a player would see.
   - Do not use what you know from the code: no walking straight to an
     object because you know its path, no skipping a step because you know
     it is optional, no reading state to find out what to do next.
   - Do not call remotes, set values, or move the character by script.
   - When the screen gives no clue what to do, that is the finding. Note
     the moment, wander as a player would for a short while, then continue.
5. Stop the recorder so it returns its report, then stop the playtest.
   Always stop the playtest, whatever happened.
6. Read the report and the console output.

What to read from the report:

| Measure | Healthy | Meaning when it is not |
|---|---|---|
| Time to first input that does something | under 10 s | The start is blocked by a menu, a load or a cutscene |
| Time to first reward | under 30 s | The loop starts too slowly or is not found |
| Time to first purchase or upgrade | under 2 min | Prices too high, or the shop was not discovered |
| Longest stretch with no progress | under 45-60 s | The player was stuck or lost; look at what was on screen then |
| Things never found | none of the core ones | A system exists that nothing leads to |
| Deaths or failures and time to recover | seconds | Failure costs too much |
| Errors in the console | none | Something broke during normal play |

Compare the times with the targets in `GAME.md`. Then judge the three
clarity questions at spawn, at one minute and at five: what am I doing,
what do I get, what is next? Answer them only from the screenshots.

One run is one sample. If a result is borderline, run it again with
different choices (go left instead of right, ignore the first prompt).

## Feel checks

The agent cannot feel a game, but it can measure what feel is made of:
what changes on screen, how soon, and for how long.

- **Capture bursts at known moments.** Trigger the action with simulated
  input, then take `screen_capture` images at set times: just before, just
  after, and again about 100, 300 and 600 ms later. Lay them side by side.
  If before and after look alike, the action has no visible feedback.
- For each core action check, in order: does something change at once;
  can the outcome be read (hit, miss, amount); does a number or bar react;
  does the object itself move; does it end cleanly without leftovers.
- **Slow motion for fast effects.** Hits, slashes and bursts are over
  before a normal capture catches them. Use the `vfx_capture` snippet (see
  the `vfx` guide) to step through an effect at exact moments and capture
  each one. Check that the brightest frame lands on the moment of impact.
- **Sound timing** is measured with the `sound_probe` snippet
  (`sound-design`): the loudest point of the sound should land on the
  visual hit.
- **Input delay.** Record the time of the input and the time the state
  changes on the client. Anything a player presses should answer on screen
  within about 0.1 s, before the server replies.
- Rare events (level up, rare drop, rebirth) should look clearly bigger
  than common ones. Capture one of each and compare.

Say what was measured. "The hit shows a flash and a +5 within 100 ms" is a
finding. "It feels good" is not something an agent can report.

## Performance

Fetch the `perf_capture` snippet with `rbx_studio_snippet`, read its header
for the exact calls, and run it during a playtest at the heaviest moments:
the busiest area, the biggest effect, the most enemies or pets on screen,
and after ten minutes of play (to catch growth).

Starting budgets for a mobile-first game. If `GAME.md` sets its own, those
win.

| Measure | Budget | Notes |
|---|---|---|
| Frame time | 16.7 ms (60 fps) typical; never worse than 33 ms (30 fps) | Studio on a PC is far faster than a phone: a PC result near the limit fails on mobile |
| Client memory | under about 1 GB total; under 800 MB is safer | Low-end phones close the app well before a PC would notice |
| Memory growth | flat after the first minutes | A steady rise over ten minutes is a leak: connections or instances not cleaned up |
| Parts in Workspace | under about 20,000 in total, fewer moving | Use StreamingEnabled for larger maps |
| Unanchored, simulated parts | under a few hundred at once | Physics is expensive on phones |
| Triangles in view | under about 500,000 | Watch dense meshes repeated many times |
| Draw calls | under about 500 in a typical view | Many unique meshes and materials raise it; repeated identical ones are cheap |
| Particles alive | a few hundred in normal play, around 1,000 at a peak | Large overlapping transparent particles cost the most |
| Shadow-casting lights | a handful in view | Prefer lights with Shadows off for decoration |
| Server heartbeat | 60 per second, steady | Drops mean server scripts are doing too much per frame |

Features that cost the most to draw, to check first when over budget:
large overlapping transparent surfaces and particles, many shadow-casting
lights, heavy post-processing, many SurfaceGuis and BillboardGuis, Glass
and other reflective materials over large areas, many unique textures,
many Humanoids, very high `Rate` on emitters, and long Beams or Trails with
many segments.

Report the measured numbers, the moment they were taken, and that they
come from Studio on a PC. Real device numbers need the user to test on a
phone; say so rather than implying the game is verified for mobile.

## Device layouts

Check the screens a player actually uses (the `ui-design` guide has the
layout rules):

- **Phone, landscape**: the default case. Nothing under the thumbstick
  (bottom left) or the jump button (bottom right), nothing cut by the notch
  or rounded corners, touch targets at least 44 px, text readable at arm's
  length, the HUD not covering the play area.
- **Tablet**: more room, different aspect ratio. Nothing stretched, nothing
  stranded in a corner.
- **PC**: the layout is not just a giant phone UI; hover states and key
  hints exist where expected.
- **Console**: every screen can be operated with a gamepad alone, a
  selection highlight is visible, text is readable from a sofa, and nothing
  important sits at the very edge of the screen.

For each size, capture the main screens (HUD at spawn, shop, a popup, the
longest list) and look at every capture. If the viewport size cannot be
changed with the tools available, ask the user to switch Studio's device
emulator and tell you when it is set. Do not judge a phone layout from a
desktop-sized capture.

Check input too: every action must be reachable by touch, with no keyboard
or right mouse button required.

## Multi-player checks

A single Studio playtest has one player. Reason about, and where possible
test, what a second player changes:

- What does another player see of my actions (my pets, effects, purchases,
  base)? Is anything meant to be private shown to everyone, or the reverse?
- Shared objects: two players using the same button, pickup, enemy or
  plot at once. Who gets the reward? Can one block the other?
- Per-player state stored in a shared place (one value for the whole
  server where there should be one per player) is a common bug: read the
  code for it.
- Joining late: a player who arrives mid-round or mid-event has something
  to do and sees a correct state.
- Leaving: the game cleans up a leaving player's objects, and others are
  not broken by it.
- Cost with a full server: multiply per-player effects, pets and parts by
  the maximum player count and compare with the budgets above.

When a check needs two real clients, say that it was reasoned from code
and not run, and ask the user to run a local server test with two players
if it matters.

## The report

Write what a player would meet, with evidence, in this shape:

```
NEW-PLAYER RUN: [pass / problems found]
First reward: [time]   First purchase: [time]   Longest stuck: [time, where]
Never found: [list or "nothing"]

Problems (most serious first)
1. [what happens] - at [moment] - evidence: [capture or measure] - fix: [concrete]
2. ...

Feel: [what each core action shows and how soon]
Performance: [numbers, where measured, Studio on PC]
Devices: [sizes checked, problems]
Multi-player: [tested / reasoned from code; findings]
Not tested: [what and why]
```

Separate what was measured from what was inferred. Do not say the game is
fun: say what a new player sees, how long things take, and where they
would get stuck. The user makes the final call by playing it.
