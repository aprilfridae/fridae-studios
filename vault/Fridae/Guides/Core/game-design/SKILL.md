---
name: game-design
description: "Game: core loop, first five minutes, feedback, pacing, fair retention, failure modes. Practical game design craft for Roblox - core loop anatomy, the first five minutes, feedback priorities, pacing and reward schedules, difficulty, fair retention for young players, mobile sessions, social hooks, and the common failure modes with how to spot each in a playtest. Use for any design decision, new feature or system."
examples:
  - "Design the core loop for my game"
  - "Why do players quit in the first five minutes?"
  - "Add a reward schedule that is fair to young players"
  - "Review this new feature's design"
applies: game
core: true
---

# Game design for Roblox

Use this when deciding what to build and how it should behave, before
writing the system. Check every decision against the pillars in `GAME.md`
and record it there (`game-bible`). Numbers are designed with
`economy-design`; whether the result works is judged with `player-testing`.

## Core loop anatomy

A loop has four parts. Name each one for the game before building.

1. **Action**: what the player's thumbs do. It must feel good on its own,
   with no reward attached. If the action is dull, no reward fixes it.
2. **Feedback**: the game's immediate answer, within a tenth of a second.
3. **Reward**: something kept: currency, progress, an item, a new place.
4. **Investment**: the reward is spent on something that changes the next
   action (faster, bigger, a new area), which is the reason to repeat.

Loops nest. Seconds: swing, hit, coins. Minutes: fill the bag, sell, buy an
upgrade. Session: unlock the next zone. Days: rebirth, collection, rank.
Each longer loop must be visible from inside the shorter one: the player
swinging a sword can see the locked gate and its price.

A new feature must attach to a loop: it is an action, a reward, or
something to invest in. A feature that attaches to none is decoration; say
so before building it.

## The first five minutes

The player has never seen the game, is on a phone, and may not read.

- **Playing within 10 seconds of spawning.** No menu, no cutscene, no
  choice to make first. Spawn facing the first thing to do.
- **First reward within 30 seconds**, first purchase or upgrade within
  about 2 minutes, first new area or clear milestone within 5.
- **Teach by doing, one thing at a time.** A glowing target, an arrow or
  beam to the next goal, a pulsing button. The player performs the action
  once and the hint goes away.
- **No text walls.** A hint is at most about six words, next to the thing it
  describes. If a mechanic needs a paragraph, simplify the mechanic.
- **Always show the next goal** on screen: what to do and how close it is
  ("Next: Wooden Sword 35/50").
- Hold back everything else. Shops, pets, rebirth, codes and settings
  appear when they first become useful, not all at once at spawn.
- The first minutes are generous: quick wins, a free first upgrade. The
  curve tightens later.
- Returning players skip all of it. Tutorial state is saved.

## Feedback and juice, in priority order

Spend effort in this order. Each level matters more than everything below.

1. **Instant response to input.** The client shows the result at once and
   the server confirms. A button that answers after a network round trip
   feels broken.
2. **Readable outcome.** The player can tell what happened: hit or miss,
   how much was earned, why it failed.
3. **Sound.** A short sound on every core action is the cheapest large
   improvement (`sound-design`).
4. **Number and UI reaction.** The counter pops, the bar fills, a "+5"
   rises from where it was earned.
5. **Motion on the object.** Squash, recoil, a flash, a small knockback.
6. **Particles and camera.** Bursts, a little shake on big moments only.
7. **Celebration** for rare events: level up, rare drop, rebirth. Larger,
   longer, and rare enough to stay special.

Scale feedback to importance. If every coin triggers a full-screen
celebration, nothing is left for the event that matters.

## Pacing and reward schedules

- **Fixed rewards** (every action pays a known amount) make progress
  predictable. Use them for the base loop.
- **Variable rewards** (a chance at something better) create anticipation.
  Use them on top of a fixed base, never instead of it. Publish the odds and
  add a pity rule (`economy-design`).
- **Milestones** (every N actions, each new zone) give medium-term goals.
- Rewards arrive often at the start and spread out gradually. Each gap is
  somewhat longer than the last, never abruptly ten times longer.
- Alternate effort and release: a stretch of work, then a payoff moment
  (the sale, the unlock, the boss dying). Flat intensity is tiring.
- The player should always have one goal reachable in the current session.

## Difficulty

- Difficulty should rise a little slower than the player's skill or power,
  with occasional spikes (a boss, a gate) followed by an easier stretch.
- Failure costs seconds, not minutes. Respawn close by, keep what was
  earned, and make the cause of death obvious.
- In stat-based games, difficulty is a number check: make sure the player
  can see the number they need.
- Design for touch: generous hit areas, forgiving jump timing, some aim
  assistance. What is comfortable with a mouse can be impossible with
  thumbs.
- If one game serves beginners and experts, separate them by zone or mode
  rather than averaging the difficulty.

## Retention that is fair to children

Good reasons to return: a long goal in progress, a collection to complete,
a daily reward that is a bonus, new content, friends who play.

Do not build these, even when asked to "improve retention":

- **Punishing absence**: losing progress, items or a pet for not logging
  in; streaks that reset to zero after one missed day. A streak that pauses
  or keeps its best value is fine.
- **Fake urgency**: countdown timers on offers that are not really ending,
  "only 2 left" on unlimited goods.
- **Pressure popups**: purchase prompts on join, on death or in the middle
  of play; prompts that return after being closed; close buttons that are
  tiny, delayed or hidden.
- **Confusing prices**: several premium currencies, bundles that hide the
  real cost, unclear what is paid with Robux.
- **Paid random rewards with hidden odds.** Show the odds, add pity, and
  respect regions where paid random items are restricted.
- **Guilt and shame**: a sad pet, messages that the player is letting
  others down, public marking of players who have not paid.
- **Pay to remove pain** that the design added on purpose: waits or grind
  tuned to be unbearable so that the skip sells.
- **Bought wins** in competitive modes.

If the user asks for one of these, explain the problem briefly and offer the
fair version (bonus instead of penalty, real limited events, visible odds).

## Sessions on mobile

- A useful session fits in 3-5 minutes and still rewards 20.
- Progress saves continuously. Closing the app at any moment loses nothing.
- Interruptions are normal: no long unpausable sequences, and a player who
  returns after a minute is not dead and lost.
- One thumb moves, the other acts. Few buttons, placed away from the
  thumbstick and jump button, large enough to hit.
- No precise camera control as a requirement. The default camera must be
  enough to play.
- Keep the HUD small: the world is the game, and the screen is small.

## Social hooks

- Other players should be useful to have around: a shared boss, a small
  bonus for playing near friends, visible bases to visit.
- Show status in the world: pets following, auras, a bigger base, titles.
  Seeing what others have is the strongest advertisement for the long loop.
- Give simple things to do together: co-op goals, races, gifting.
- Trading and PvP are large systems with large abuse risks (scams,
  bullying). Add them deliberately and late, with protections.
- Make joining a friend easy and worthwhile the moment they arrive.

## Clarity of goals

At any moment the player can answer three questions without opening a
menu: What am I doing? What do I get? What is next? Check them at spawn,
after the first purchase, and after ten minutes. A locked thing shows what
unlocks it. A number that matters shows its name.

## Failure modes and how to detect them

Check these in a playtest (`player-testing`), not by reading code.

| Failure | What it looks like in a playtest |
|---|---|
| No clear goal | The new-player run wanders; long time before the first reward; nothing on screen names a next step |
| Slow start | More than 30 s to the first reward, or more than 2 min to the first purchase |
| Dead action | The core action produces no sound, no motion and no number within 0.1 s; captures before and after look the same |
| Text wall | A screenshot in the first minute shows a paragraph, or several popups stacked |
| Everything at once | The spawn screenshot shows many buttons and badges before the player has done anything |
| Progression wall | Time-to-afford jumps sharply between two steps; the recorder shows long stretches with no purchase |
| Trivial economy | Several upgrades are affordable at once; the player never has to choose |
| Reward inflation | Every pickup celebrates; a rare event looks the same as a common one |
| Lost player | Long periods without progress, the player far from any objective, returning to spawn repeatedly |
| Punishing failure | Death sends the player far back or removes earnings; time to recover is longer than the attempt |
| Unfair on touch | The task passes with mouse and keyboard and fails with touch-sized input; buttons under the thumbstick |
| Lonely server | Nothing a player does is visible or useful to another |
| Feature without a loop | A system exists that the run never touched and that no goal points to |

When a failure shows up, fix the cause in the design (the goal, the curve,
the first minute) before adding content on top of it.
