---
name: economy-design
description: Design and check a Roblox game's numbers - sources and sinks, cost curves, time-to-unlock targets, rebirth multipliers, drop tables and pity, inflation, premium currency - and verify every table with rbx_economy before it ships. Use whenever prices, rewards, drop rates or progression numbers are involved.
---

# Economy design

An economy is a set of numbers that decides how long everything takes. Do
not choose prices by feel. Choose **times**, derive the prices, simulate
them with `rbx_economy`, then record the final table in `GAME.md`
(`game-bible`) and in the game's one config module.

## Method

1. List the **sources**: every way currency or power enters (per action,
   per second, quests, daily reward, drops, purchases).
2. List the **sinks**: every way it leaves (upgrades, unlocks, consumables,
   rerolls, cosmetics, rebirth).
3. Set **time targets**: how many minutes of play each unlock should take.
4. Derive each cost: `cost = income per minute at that point x target minutes`.
5. Run `rbx_economy` on the table. Read the result. Adjust. Run again.
6. Write the final table into `GAME.md` and the config module, and add a
   Decision log line.
7. After it is built, confirm with a playtest that real income matches the
   income assumed (`player-testing`). The simulation is only as good as the
   income figures given to it.

## Sources and sinks

- Every currency needs at least one sink that never runs out (a repeatable
  upgrade, a consumable, rebirth). Otherwise balances grow without limit and
  prices stop meaning anything.
- Count **all** sources. Forgotten ones (an AFK reward, a group bonus, a
  stacking boost, a quest that can be repeated) are what break economies.
- Multipliers multiply each other. Three separate "x2" boosts are x8. List
  every multiplier and compute the largest product a player can reach.

## Cost curves

`n` is the level or purchase number.

| Curve | Formula | Behaviour | Fits |
|---|---|---|---|
| Linear | `a + b*n` | Each step costs the same amount more | Short lists (under ~10 steps), early tutorial upgrades, things with flat income |
| Polynomial | `a * n^k`, k from 1.5 to 3 | Grows steadily, never explodes | XP per level in RPGs, long tracks where income grows roughly linearly |
| Exponential | `a * r^n` | Each step costs r times the last | Repeatable upgrades in simulators and tycoons, where income is multiplied |

- Match the cost curve to the income curve. If upgrades **multiply**
  income, costs must be exponential or the game ends in minutes. If income
  grows by **adding**, exponential costs produce a wall.
- For repeatable upgrades, `r` between 1.07 and 1.15 is slow and long; 1.5
  to 3 suits a few big tiers. The cost ratio between tiers must be larger
  than the income ratio the tier gives, or each step gets quicker than the
  last.
- Round prices to clean numbers (1,500, not 1,487).

## Time-to-unlock targets

Reasonable starting targets for a mobile Roblox game:

| Moment | Target |
|---|---|
| First reward | within 30 seconds |
| First purchase | within 1-2 minutes |
| Next purchases in the first session | 2, 4, 8 minutes apart |
| Mid game | 15-30 minutes per meaningful unlock |
| Late game | 1-3 hours per major unlock, with small ones in between |
| First rebirth | 45-90 minutes |

Each gap is at most about twice the previous one. A step that takes five
times longer than the step before it is a wall; players leave at walls.
Something small should always be affordable within a few minutes, even
late in the game.

## Rebirth and prestige

The player resets progress in exchange for a permanent multiplier.

- **Additive** (`1 + 0.5 * rebirths`): 10 rebirths give x6. Stable and
  easy to balance; each rebirth matters less than the last.
- **Multiplicative** (`2 ^ rebirths`): 10 rebirths give x1,024. Exciting
  and impossible to contain unless costs after rebirth grow just as fast.
- A run after rebirth takes roughly the previous time divided by the
  multiplier gain. With x2 after the first rebirth, a 55-minute run becomes
  about 27. Decide whether runs should shorten (then raise the rebirth
  requirement each time) or whether new content opens to fill them.
- State exactly what resets and what is kept. Never reset anything bought
  with Robux.
- The first rebirth should feel clearly better within a minute of doing it.

## Drop tables and pity

For a chance `p` per try, the chance of at least one success in `n` tries
is `1 - (1 - p)^n`. Players experience the unlucky tail, not the average.

| Chance | Half of players have it by | 95% by | Still without it after 1/p tries |
|---|---|---|---|
| 10% | 7 tries | 29 tries | 35% |
| 1% | 69 tries | 299 tries | 37% |
| 0.1% | 693 tries | 2,995 tries | 37% |

- More than a third of players have not received a "1 in 100" item after
  100 tries. Add **pity**: a guaranteed drop after N failures (commonly
  between 1/p and 2/p tries), or a chance that rises with each failure.
- Weights in a table must be checked as percentages that sum to 100.
  Compute them; do not trust a list of weights by eye.
- Convert every drop into **time**: tries needed x seconds per try. A 0.1%
  drop from a 30-second action is about 25 hours at the 95% mark.
- Roll on the server. Never trust a client's claim of what dropped.
- If a random reward can be bought with Robux, directly or through a
  bought currency, show the odds to the player before purchase and respect
  the policy setting for regions where paid random items are restricted.

## Inflation checks

- Compare what a late-game player earns per hour with what there is left
  to spend it on. If earnings exceed sinks, balances pile up.
- Check the richest realistic case: all boosts, best gear, best pets, max
  rebirth multiplier. Prices must still mean something to that player.
- No loop may produce more than it costs (buy, convert, sell for more).
- AFK income needs a cap or a much lower rate than active play.
- If players can trade, any duplication or runaway source spreads to
  everyone. Tradable items need tighter control than personal currency.

## Premium currency

- Keep a clear line between what is earned by playing and what is bought.
  The simplest fair setup: one soft currency earned in play, and Robux
  products for cosmetics and conveniences. Avoid inventing a second bought
  currency; it hides real prices from young players.
- If bought boosts exist, include them in the simulation. A x2 pass must
  not make the game trivial, and its absence must not make the game
  miserable.
- Never price the base game around the boost. Balance for a non-paying
  player first; the paying player is then simply faster.

## Checking a table with rbx_economy

Always run `rbx_economy` before shipping or changing a table. It needs
numbers, not descriptions. Read the tool's parameter list for the exact
field names, and have these ready:

- **Income**: starting currency per minute (or per action plus actions per
  minute), and how each purchase changes it.
- **The purchases**: name, cost, effect, in the order a player meets them,
  or the formula and number of levels.
- **Multipliers**: rebirth rule, boosts, anything that scales income.
- **For drops**: each item's chance or weight, the pity rule, and the time
  one try takes.
- **A horizon**: how many minutes or hours of play to simulate.

Reading the result:

- **Time to each unlock**: compare with the targets. Flag any step more
  than about twice as long as the one before, and any step under a few
  seconds (it is not a decision).
- **Total time to the end of the table**: is that the amount of content
  intended for this version?
- **Currency balance over time**: a balance that only rises means there
  are too few sinks.
- **Drop results**: read the median and the unlucky end (95th percentile)
  in tries and in hours, not the average.
- Run it twice more: once for a player with every boost, once for a slow
  player earning about half the assumed income. All three must be playable.
- Change one thing at a time between runs.

## Worked example

A sword simulator. Income starts at 60 coins a minute. Targets for the
first session: purchases after about 1, 2, 4, 8, 15 and 25 minutes. Each
cost is the income before the purchase times the target minutes.

| Tier | Cost | Coins/min after | Minutes to afford |
|---|---|---|---|
| Stick (start) | 0 | 60 | - |
| Wooden sword | 50 | 150 | 0.8 |
| Stone sword | 300 | 400 | 2 |
| Iron sword | 1,600 | 1,000 | 4 |
| Gold sword | 8,000 | 2,500 | 8 |
| Diamond sword | 37,500 | 6,000 | 15 |
| Void sword | 150,000 | 14,000 | 25 |

Total: about 55 minutes to the last tier. Costs rise by a factor of 4 to 6
per tier while income rises by about 2.5, so each step takes longer than
the last, by no more than double.

Rebirth at the Void sword: reset swords and coins, gain +1 to the
multiplier (x2 after the first). The second run takes about 27 minutes, the
third about 18. To stop runs collapsing, the rebirth requirement rises by
one tier's worth each time and a new zone opens at rebirth 1 and 3.

Run this table through `rbx_economy` three times (normal, with a x2 pass,
at half income), fix what it flags, then copy the final table into
`GAME.md` and the config module.

## Checklist

- Every currency has a source list, a sink list and an endless sink.
- Costs come from time targets; no step is over twice the previous one.
- Every random drop has published odds, a pity rule and a time in hours.
- `rbx_economy` has been run for a normal, a boosted and a slow player.
- The table in `GAME.md` equals the config module, number for number.
