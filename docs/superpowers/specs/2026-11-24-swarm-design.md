# Swarm: parallel expert teams as Fridae's default way of working

Asked for by the user on 2026-11-24:
- "can we embed our own /swarm feature similar to antigravitys? i know we have a rudementary version of that but i want it to be as good as the swarm feature"
- On where it goes in the queue: "Right after the RAG"
- "dont make it a command tho just make it default behavior unless thats like insane"

Reference: Google Antigravity's `/swarm` (a coordinator, named workers in isolated git worktrees, a shared board with
@mentions, adversarial verifiers, an `@monitor` that wakes only on stalls, idle agents parked at no token cost, a
members roster, and a timeline graph).

## Intent

When a job has parts that can be done at the same time, Fridae's Lead runs them at the same time: each writer in its
own copy of the project, talking on a shared board, each change verified before it is merged, a watchdog for stalls,
and idle members parked. This is how Lead-led work behaves by default. There is no command to type.

**Not insane, with one rule:** the Lead decides. A small or tightly linked job stays a single agent (a swarm would
only burn usage), and a job that splits cleanly fans out. Parallel work uses quota faster, so the user sets a ceiling.

## What exists today (to build on)

`rbx_team` spawn and wait, the Lead and expert rosters (`rbxroster`, `vault/Fridae/Team`), the studio run
(stages, gates, Review), experts as chats under the Lead in the sidebar, plain-English summaries, rollover, cheap
models for simple roles, and Lead wake-ups (`team-nudge`). The gap is that **only one expert may write at a time**
(`team.rs` `writer_gate`). Experts also only talk to the Lead, verification is a reviewer at the end rather than per
change, stalls are not watched, and finished experts end instead of parking.

## Behaviour

- **The Lead fans out by default.** For any Lead-led work (a studio run, a plain project's autonomous team, a chat
  that hands work to experts), the Lead cuts the job into independent parts by file, module or question and starts
  named members (`@shop-ui`, `@datastore`, `@tests`). Each member has a role from the project's team preset.
  - Small or tightly coupled work stays a single agent.
  - The Lead's guide (`studio.md`, `lead.md`) states when to fan out and when not to.
- **Ceiling:** Setup → Experts has "Most experts working at once", default 4, at most 8 (the app's agent limit).
  Setting it to 1 means one at a time, which is today's behaviour.
- **Every writer works in its own copy.**
  - For file-based work (scripts, docs, Rojo code, plain projects), each writing member gets its own git worktree
    and branch under the app's work area, made from the run's base commit. Members never edit the same checkout.
  - Read-only members (reviewers, researchers, testers who only read) need no copy.
  - A project that is not a git repo gets one first (Fridae projects already use git).
- **Roblox Studio takes turns.** The live Studio place is one shared thing and cannot be copied, so changes made in
  Studio go through a **Studio turn**: today's writer gate, narrowed to Studio. A member that needs Studio asks for
  the turn, does its Studio part and gives the turn back. File work carries on in parallel meanwhile.
- **The shared board.**
  - Every run has one ordered log (`<work>/swarm/<run>/board.jsonl`).
  - Members post and read through `rbx_team`, with new actions `post`, `board` and `wait_board`. Messages can be
    addressed to `@name`, `@group` (a role) or `@all`.
  - Members can ask each other directly ("@datastore what's the profile key?") instead of everything going through
    the Lead. The Lead sees everything.
- **Parking.** A member that has finished its part, or is waiting on someone, ends its turn and **parks**. Its
  conversation is kept, and it costs nothing while parked. The app resumes it when a board message mentions it or the
  Lead asks for it. Parked members show as parked.
- **Verify before merge.**
  - When a writing member says its part is done, a verifier member checks its branch: it runs the project's checks
    (tests, build, lint, the Luau type check where present) and reads the diff adversarially against the brief.
  - Only a verified branch is merged into the run's integration branch. Verification runs on a capable model, not a
    cheap one (the user's cheap-model rule: cheap models never do the hard checking).
  - A failed check goes back to the member on the board with what failed.
- **Merging.**
  - The app merges verified branches into the run's integration branch, in order.
  - On a conflict, the member whose branch conflicts is resumed to rebase and resolve it, and is verified again.
  - The integration branch reaches the project's main branch only through the existing Review and gate flow, with
    the user's yes.
- **The watchdog is the app itself, not an AI.** It costs no tokens.
  - The app knows when every member is parked or idle but the run isn't finished. It then wakes the Lead with a note
    on the board.
  - A member with no output for N minutes (default 10) gets one nudge. If still silent, it is stopped, its Studio
    turn and copy are released, and the Lead is told. This covers the "expert ended with no report and kept the lock"
    lesson in the user's notes.
- **Cross-AI.** Each member can run on a different AI or model, as today: cheap models for simple, well-specified
  parts and strong ones for plans, hard parts and UI. Rollover keeps members going.
- **Plain-English summaries** carry on per member when it finishes or its branch merges.

## The Librarian: a knowledge expert on the board (user idea, 2026-11-24)

The user: "having an llm agent that indexes the rag and then if the swarm has questions that arent related to
individual agents it can ask the rag trained expert".

- Every swarm has a standing member, **`@librarian`**. Any member, or the Lead, asks it knowledge questions on the
  board: "how do we stop exploiters firing this remote?", "what does the user mean by softer?", "what did we learn
  last time the shop UI broke?".
- The Librarian searches the knowledge index (RAG, or keyword search when RAG is off) over the Fridae Studios vault,
  the user's taste notes, house rules and the game's own notes. It reads the best few passages and answers in a few
  lines, citing each source note so the asker can open it.
- **Why it saves tokens:** today every worker searches and reads whole notes into its own context. With the
  Librarian, one cheap member does the reading and each worker gets a short answer back. Repeat questions in a run
  are answered from its own memory.
- **It answers only from what it found.** When the vault has nothing, it says so ("not in the knowledge base") and
  never makes an answer up. Exact API lookups can still go straight to `rbx_vault`.
- **Model:** a cheap OpenCode model (DeepSeek/GLM), because reading retrieved notes and summarising them is the kind
  of simple job the user's cheap-model rule allows. It escalates to a stronger model only when a question needs real
  judgement.
- **It is also the curator** (with the vault design, `2026-11-24-unified-knowledge-vault-design.md`):
  - When a member discovers something worth keeping, or the user steers taste, it posts `@librarian remember: ...`.
  - The Librarian files it into the vault by topic, updates an existing note instead of duplicating, and links it.
  - This makes the Librarian the one place knowledge goes in and comes out.
- It is parked (free) whenever nobody is asking, and wakes on a mention.
- It isn't "trained": nothing is fine-tuned. It is an agent with the knowledge index as its tool, so it is always as
  current as the vault.

## UI

- **Sidebar tree (exists):** members under the Lead, each with a status (working, parked, verifying, merged, stopped)
  and its AI and model. Click one to open its chat and steer it.
- **Board:** a panel beside the Lead's chat showing the ordered log, with names highlighted, filters (all, mentions,
  one member) and a box for the user to post to `@name` or `@all`.
- **Timeline:** a graph of the run, with members as lanes, their commits, verifier verdicts and merges into the
  integration branch, in the same style as the Overview graph.
- Wording adapts to the project type: game wording for games, generic for plain projects.

## Anti-goals

- Two members editing the same checkout.
- Parallel changes inside the live Studio place.
- A merge to the project's main branch without the user's yes.
- An LLM spending tokens just to watch for stalls.
- Fanning out a tiny task.

## Open for the build

- Where worktrees live and how they are cleaned up (they must be outside folders agents may write to only by
  permission, and removed when merged or stopped).
- How a member's "done" is signalled (a board post of kind `done`, through `rbx_team`).
- How the integration branch is named and how undo (`studio_undo`) covers swarm merges.
- Which checks run per project type (Rojo/Selene/Luau for games; whatever the plain project defines).
- Board size limits and how much of it a member reads at once (tokens).

## Tests

- Two writing members on one project each get their own worktree, and their changes merge cleanly.
- A conflict resumes the right member to fix it.
- A Studio turn is exclusive while file work runs in parallel.
- Board mentions wake parked members and only those.
- The watchdog wakes the Lead when all members are idle, and stops a silent member, releasing its copy and turn.
- A branch that fails verification is not merged.
- A ceiling of 1 gives today's one-at-a-time behaviour.
