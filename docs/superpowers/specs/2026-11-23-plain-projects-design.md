# Plain (non-game) projects, with an optional autonomous team

Asked for by the user on 2026-11-23: "eventually this wont just become a game making harness, this is gonna be my main
harness. maybe make it an option to make a game inside of the harness, and then have an option to start a regular
program / other projects." Then, on the shape: "just plain folder for now and make it autonomous team an option."

## Intent

Fridae becomes the user's main AI harness, not only a Roblox game-maker. A project can be a Roblox game (everything
today) or a plain project: any folder the user builds anything in — a web app, a CLI, a script, a library — with all
of Fridae's general features and none of the Roblox-only machinery. A plain project is just a folder you chat and
build in; an autonomous general team is an option you can turn on, off by default.

## What is already general (reused unchanged)

Chats on a folder across Claude, Gemini, Cursor and OpenCode; the model picker; sign-in and multiple accounts with
rollover; Ask me / Full auto / Overnight modes; git; long tool calls; Review; the sidebar; attachments. None of this
is game-specific; a plain project gets all of it.

## What is game-only (a plain project does NOT load)

The studio run's Roblox roster (World builder, Icon maker, etc.), the `rbx_*` tools, Roblox Studio and Blender, the
Promos tab, and the GAME.md bible requirement.

## Behaviour

### Project type
- "New project" (and "Add a folder you already have") asks the type: **Roblox game** (as today) or **Plain project**.
- A project records its type. Existing projects are games (the field defaults to game when absent).
- A plain project: a name, a folder, git optional, a one-line "what is this?" the user can set. No GAME.md is required;
  if the user wants a spec/readme the agents write an ordinary one.

### A plain project with no team (the default)
- Chats on the folder with any AI, every mode, the picker, sign-in/accounts, long calls, git, attachments, Review for
  what a chat produced. The agents get the general tools (read/edit files, run commands per the mode's permission,
  git, web) — never the `rbx_*` tools, Studio or Blender.
- The Promos tab and anything naming Roblox/Studio/GAME.md is hidden for the project.
- The guides an agent loads are the general ones (coding, the repo's own conventions), not the Roblox vault.

### The optional autonomous team
- A plain project has a switch "Autonomous team" (off by default). On, it gains a run like the studio run but general:
  a Lead plans the work, cuts it into tasks, hands each to a general expert, checks the result, files Review items, and
  asks the user at the gates — reusing the existing run/Lead/`rbx_team`/Review machinery with a non-game roster and no
  Studio.
- Starter general roster (refine during build): **Lead**, **Coder**, **Reviewer**, **Tester**, **Architect/Designer**
  (plans structure and interfaces), **Docs**. No Icon/World/Effects/Promo experts.
- General stages, not the game ones: **plan → build → test → review** (or free-form under the Lead); no
  idea/launch/promote. The gates the user approves: the plan, and shipping/finishing.
- The team obeys the same rules as the studio Lead: one writer changing the repo at a time, cheap models (DeepSeek/GLM)
  for simple well-specified jobs and the stronger AI for planning/hard work, read a report before accepting, the
  user's house rules and notes.
- Full auto / Overnight work with the team as they do for a game.

### The Lead's instructions and tools, generalised
- The Lead and expert instruction files are split: the parts that are about running a team (briefing, checking a
  report, filing Review, cheap-model use, never spend/publish without the user) are shared; the Roblox parts (Studio,
  the place, GAME.md, icons/thumbnails) load only for a game.
- The tool server offers a plain project the general tools and the team tools (`rbx_team`, Review/stage, long-call
  jobs), not the Roblox ones.

## Wording adapts to the project type (user request, 2026-11-23)

"add a rewording of all ui functions to be more for generalized projects instead of game dev specifically, but if its
a game project, make it game oriented." So every user-facing label and line is context-aware: generic for a plain
project, game-oriented for a game project.

- Centralise the swappable terms in one small per-project vocabulary keyed on the project's type, rather than scattering
  `if game` through the page — the UI asks that vocabulary for each term and renders it. One place to add a future
  project type later.
- Examples (game term → plain-project term): "game" → "project"; "What should we build or test?" → "What should we
  build?"; "Studio" mode and all Roblox/Studio/GAME.md references → hidden or generic (a plain project has no Studio);
  the Lead's "takes the game through its stages" → "takes the project through its stages"; the game-specific stage
  names (idea/launch/promote) → the general ones (plan/build/test/review) per the team section above; expert role names
  → general ones for a general team; Promos tab → hidden for a plain project. A game project keeps all of today's
  wording unchanged.
- This covers tabs, the sidebar, the message box, mode and picker labels, Setup copy, the Guide, Review, and the
  agent-facing instruction texts (which already split shared vs Roblox-only in "The Lead's instructions and tools"
  above — the shared parts read generically, the Roblox parts load only for a game).
- Default when a project's type is unknown/old: game wording (every existing project is a game).

## Anti-goals
- A plain project loading Studio, Blender, `rbx_*` tools, Promos, or a game roster.
- The autonomous team forced on; it is off until the user turns it on.
- Breaking any existing game project (type defaults to game).

## Not in scope (later)
- Templates/scaffolds for specific stacks.
- A general build/run/preview integration (like Studio) for plain projects.
- Windows.

## Open for the builder
- The exact general roster and stage names (start from the set above; the user may refine).
- Where the "Autonomous team" switch lives (project settings vs the chat's mode menu).
- How much of the studio-run UI (Review's stages, run bar) is shown for a general team vs trimmed.

## Tests
- A project's type is recorded, defaults to game for old projects, and decides which tools/guides/tabs load.
- A plain project's agents never receive `rbx_*`, Studio or Blender; a game's are unchanged.
- The team off: a plain project is chats only. The team on: a run starts with the general roster and general stages,
  through the same run/Review code, with no Roblox expert or Studio call.
- The shared vs Roblox-only split of the Lead/expert instructions loads the right parts per type.

## Sequencing
Build after the in-flight sidebar and multi-account work land (all touch `ui/index.html`). Ship the plain project and
the optional team together, team off by default.
