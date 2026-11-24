# Expert team presets, and a general-programming roster

Asked for by the user on 2026-11-23: "make a general programming list of experts, make a few variations of expert
teams that you can pick from. later down the line we can make a custom expert team functionality as well." Builds on
the plain-projects autonomous team (`2026-11-23-plain-projects-design.md`).

## Intent

A plain (non-game) project's optional autonomous team needs a roster that isn't Roblox-specific. Define a
general-programming set of experts, offer a few ready-made team compositions the user picks from when starting an
autonomous run, and leave room to build custom teams later. Game projects keep their existing Roblox roster as one
preset.

## The general-programming expert roster (roles)

Each is a role the Lead can hand a task to; one-line responsibility:

- **Lead** — plans the work, cuts it into tasks, briefs each expert, checks results, files Review, asks the user at the
  gates. Always present; not pickable (every team has exactly one).
- **Architect** — decides structure, modules, interfaces, tech/library choices and data shapes; writes the plan/spec
  the coders build against.
- **Coder** — implements features from a brief; the main builder. The Lead may run several on independent parts (one
  writer per set of files at a time).
- **Reviewer** — reads a change for correctness, clarity and fit to the plan; flags issues for the owning coder.
- **Tester** — writes and runs tests; reports what passes and fails, with repro steps.
- **Debugger** — drives a specific failure to its root cause (for hard bugs; otherwise folded into the Coder).
- **Security reviewer** — audits for vulnerabilities: input validation, secrets, auth, injection, unsafe defaults.
- **Performance** — profiles and improves hot paths, memory and startup.
- **UI/UX designer** — designs screens and flows for anything with a front end (reuses the design skills); hands specs
  to a coder.
- **Docs** — writes the README and usage docs and documents the public interface.
- **Integrator / DevOps** — build, packaging, CI, and run/deploy setup.
- **Researcher** — investigates libraries, approaches and unknowns before building, and reports findings.

All obey the same rules as today's studio Lead and experts: one writer changing the repo at a time; cheap models
(DeepSeek/GLM) for simple well-specified jobs and the stronger AI for planning and hard work; read a report before
accepting; the user's house rules and notes; nothing published or spent without the user.

## Team presets (pick one when an autonomous run starts)

1. **Minimal** — Lead, Coder, Reviewer. Small scripts, one-off tools, quick changes.
2. **Standard** (general default) — Lead, Architect, Coder, Reviewer, Tester, Docs. Most software projects.
3. **App / web** — Lead, Architect, Coder (front end and back end), UI/UX designer, Reviewer, Tester. Projects with a
   user-facing side.
4. **Production / hardened** — Standard plus Security reviewer, Performance, Integrator/DevOps. Shipping something real.
5. **Research / prototype** — Lead, Researcher, Coder, Tester. Exploring or spiking before committing.
6. **Game (Roblox)** — the existing roster (Designer, Coder, World builder, UI builder, Effects and sound, Icon and
   thumbnails, Playtester, Device tester, Security reviewer, Economy and shop, Promo maker). Unchanged; the preset a
   game project uses.

## How picking works

- A preset is just which experts the Lead may spawn and their general instructions. The Lead still decides how many of
  each to run and when; a preset is a roster, not a schedule.
- When a plain project turns on the autonomous team (or starts an autonomous run), it picks a preset — default
  **Standard**. A game project uses **Game** automatically. The choice lives with the project and can be changed.
- Presets are data (a roster list per preset) so a new one is a table entry, and so the later custom-team feature can
  write one.
- Reuse the existing team machinery (`rbx_team`, the expert instruction files, the studio run, Review). The expert
  instruction files split shared vs role-specific the same way the plain-projects spec splits shared vs Roblox-only, so
  a general expert loads its general guide, a game expert its Roblox one.

## Later (deferred by the user): custom teams

- The user builds their own team: pick which experts, name them, set each one's instructions and preferred AI/model.
  Saved as a named team the project can use. A preset is then just a built-in custom team. Not in this phase; the data
  shape above (a named roster) is chosen so it drops in later.

## Anti-goals
- A preset forcing a fixed number of experts or a schedule — it is a roster the Lead draws from.
- Breaking the game roster (it is one preset, unchanged).
- Building custom-team UI now.

## Open for the build
- Final role names and the exact split of shared vs role-specific instruction text.
- Where the preset is picked (project settings vs the autonomous-team switch) — alongside the plain-projects team switch.
- Which general guides each role loads.

## Tests
- Each preset resolves to its roster; the Lead may spawn only those roles; Standard is the plain default, Game the game
  default.
- A general expert loads general instructions, never the Roblox guide/tools; a game expert is unchanged.
- The roster is data (adding a preset is a table entry); the shape supports a future named custom team.

## Sequencing
Part of the plain-projects autonomous-team work; build with it. Custom teams are a later, separate phase.
