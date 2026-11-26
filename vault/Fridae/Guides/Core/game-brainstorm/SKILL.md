---
name: game-brainstorm
description: "Game: interview the user about a new idea and write PROJECT.md. Interview the user about a new Roblox game idea and turn it into PROJECT.md - one question at a time, from the core fantasy to the scope of a first playable. Use when the user wants to start a new game or brings a new idea, before building anything."
examples:
  - "I have an idea for a new game"
  - "Help me turn my idea into PROJECT.md"
  - "What should the first playable of my game be?"
applies: game
---

# Game brainstorm

Turns "I want to make a game about X" into an approved `PROJECT.md` (format in
the `game-bible` guide). Nothing gets built during this guide. Its output is
a document the user has read and agreed to.

Skip it when the project already has a `PROJECT.md` and the request is a normal
task. Use it again for a new mode or a major change of direction, limited to
the questions that the change reopens.

## How to ask

- **One question per message.** Never a list of questions.
- **Multiple choice where possible**: 2-4 lettered options, each with a short
  consequence, plus "something else". Put the option you recommend first and
  say why in one line. The user should be able to answer with a letter.
- Read what already exists before asking: the project folder, any notes, the
  open place. Never ask what can be looked up.
- Use the user's own words for their idea. Do not rename it or tidy it into a
  different game.
- If an answer settles later questions, skip them. If an answer is "I don't
  know", choose a sensible default, say it, and move on; record it under Open
  questions.
- Keep each message short. The question, the options, nothing else.
- After every three or four answers, reflect back in two or three sentences
  what the game now is, so a misunderstanding surfaces early.

## Question order

Ask in this order. Earlier answers constrain later ones.

1. **Core fantasy.** What does the player get to be or do? ("Run a pizza
   place", "be the fastest thing on the map", "survive a night in a haunted
   school with friends".) One sentence. Everything else serves this.
2. **Core loop, the 30 seconds.** What does the player do over and over, and
   what do they get each time? Write it as verb -> reward -> reason to repeat:
   "swing sword -> coins -> buy a better sword". If the user cannot say it,
   offer two or three candidate loops for their fantasy and let them choose.
3. **Player and device.** Who is it for (age, what they already play) and on
   what? Default on Roblox: phone first, 9-15 years old, touch controls. A
   game that needs a keyboard or precise aim must be a deliberate choice.
4. **Session length.** How long is one sitting, and what is a satisfying
   stopping point? Default: 5-15 minutes, with something finished inside the
   first two.
5. **Progression and return.** What grows over hours and days (stats, areas,
   collection, rank, a base), and why does the player come back tomorrow?
6. **Social and multiplayer.** Alone among others, cooperative, competitive,
   or trading? What can a player do with a friend that they cannot do alone?
   How many players per server?
7. **Monetisation stance.** None for now, cosmetics only, convenience
   (boosts, extra slots), or power for sale? Record the stance, not a price
   list. Push back on pay-to-win in competitive games and on anything aimed at
   pressuring children (see `game-design`).
8. **Art direction and references.** Which existing games, images or videos
   look like what they want? Offer the three style families (blocky and raw,
   clean and minimal, soft and glossy) and ask for links or screenshots.
9. **Scope of the first playable.** The smallest version that proves the loop
   is fun: one map, one loop, one upgrade path, no saving if it is not needed
   to judge the fun.
10. **Out of scope.** What is explicitly not in the first playable, and what
    is never in the game at all.

## Roblox genre conventions

Name the genre early; it answers many questions by convention. Players
arrive with these expectations, so breaking one should be on purpose.

| Genre | Loop | Players expect |
|---|---|---|
| Simulator | Repeat an action -> currency -> upgrades, pets, new zones | Constant numbers going up, rebirths, eggs or pets, zones gated by price |
| Tycoon | Earn passively -> buy the next piece of the base | A plot per player, buttons on the ground, visible growth, rebirth |
| Obby | Jump through stages -> checkpoint | Instant respawn, stage counter, fair jumps, skips as an optional purchase |
| Horror | Explore -> tension -> escape or survive | Co-op, chapters, sound doing most of the work, short runs |
| RPG | Fight -> loot and XP -> stronger enemies | Classes or builds, quests, bosses, a large saved character |
| Roleplay | Pick a role -> live in the world with others | Houses, vehicles, jobs, avatar customisation; little or no fail state |
| Fighting | Match -> combat -> rank or unlocks | Tight hit feedback, fair matchmaking, abilities with cooldowns |
| Tower defence | Place units -> waves -> earn -> upgrade | Unit collection with rarities, co-op maps, difficulty modes |

Facts that hold across genres:

- Most players are on phones. Small screens, two thumbs, weak hardware,
  interrupted sessions.
- Many players are children. Little reading, no patience for setup, strong
  response to clear rewards and to playing with friends.
- A player decides within a minute or two whether to stay. The loop must be
  running, and rewarding, almost at once.
- Players join mid-session on a server others are already on. The game must
  make sense when entered at any moment.

Load `market-research` to check what comparable live games do before fixing
the direction, when the user is open to it.

## When the idea is vague

If the user gives a theme without a game ("something with dragons"), do not
keep asking open questions. Propose **two or three concrete directions**:

- a name for the direction and its genre,
- the 30-second loop in one line,
- what it costs to build (small, medium, large) and its main risk,
- then one recommendation with the reason.

Example: "A: dragon-raising simulator (hatch, feed, fly; small; risk: looks
like many others). B: co-op dragon boss fights (large; needs good combat).
C: dragon-flight obby (small; one mechanic, easy to make feel good). I'd
start with C: the flying is the fantasy, and it is testable in days."

## Challenge the scope

The first playable must be buildable in **days, not months**. Say so plainly
when the plan is too big, and offer the cut:

- More than one core loop: pick one, list the others under Out of scope.
- "Open world", "many classes", "story campaign", "trading", "PvP and PvE":
  each is a project in itself. One of them, in its smallest form.
- Systems that only matter once people already play (trading, clans,
  seasons, leaderboards across servers): later.
- Content counts: three of something proves the system; thirty is content
  work that follows once the loop is fun.

Ambition belongs in the pillars and the long-term list. The first playable
is a test of whether the loop is fun.

## Before the interview may end

All of these must be known, from the user or as a default the user accepted:

1. The core fantasy, in one sentence.
2. The core loop: action, reward, reason to repeat.
3. Genre, or the two it mixes.
4. Target player and primary device.
5. Session length and a natural stopping point.
6. What progresses over time, and the reason to return the next day.
7. Solo, co-op or competitive, and server size.
8. Monetisation stance.
9. Art direction: a style family and at least one reference.
10. The first playable: exactly what is in it.
11. What is out of scope.
12. Three pillars: the qualities every later decision is checked against
    (propose them from the answers; the user confirms).

If one is missing, ask for it. Do not fill a gap silently.

## Finish

1. Summarise the whole design back in a short message: pitch, pillars, loop,
   player, progression, first playable, out of scope.
2. Ask for explicit approval: "Is this the game? Anything to change before I
   write it down?" A reply about something else is not approval.
3. Write `PROJECT.md` in the project folder using the `game-bible` template.
   Leave unknown sections with a line under Open questions rather than
   inventing content. Numbers that are not designed yet are marked "to be
   set" and are worked out with `economy-design` later.
4. **Stop.** Tell the user the file is written, where it is, and ask them to
   read it. Do not start building in the same turn. Building starts when the
   user says so.
