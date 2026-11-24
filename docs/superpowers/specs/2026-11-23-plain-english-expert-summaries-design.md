# Plain-English summaries of what each expert did

Asked for by the user on 2026-11-23: "I want subagents to invoke a cheap model to tell the cheap model to summarize all
of its changes in plain english that a non tech oriented person could understand, and pass it to the lead agent to then
spit out when a subagent finishes its task, that way i can keep track of what is accomplished and understand what is
happening to my program easier."

## Intent

When an expert finishes a task, the user gets a short, plain-English, non-technical account of what it actually changed
— so they can follow what is happening to their game without reading code or expert reports.

## Behaviour

- **Trigger:** an expert (studio-run subagent) finishes its task and hands its report back to the Lead.
- **Who writes it:** a cheap model on the user's OpenCode plan — `opencode-go/deepseek-v4.1-flash`, then GLM — run
  through the app's existing no-tools summary writer (`compact.rs` / `opencode::write_only`, built for the rollover
  hand-over). Costs the user's unlimited plan, not Claude/Gemini. Local-cheap, matches the user's "cheap models for
  simple jobs" rule.
- **What it summarises:** the expert's report plus what it really changed (its changed files / `rbx_project changes` /
  the checkpoint diff for its work), turned into 2–4 plain sentences a non-technical person understands: what it set
  out to do, what changed in the game, and anything the user should know. No file paths, no code, no jargon — "Added a
  shop where players spend coins on hats, and made the buy button bigger" not "Implemented ShopService + GUI."
- **Where it shows (so the user actually sees it):**
  - On the expert's row and chat in the sidebar tree, a "In plain words:" line.
  - Passed to the Lead with the expert's result, so when the Lead reports progress or files the Review item it includes
    the plain summary (the user asked for the Lead to "spit it out when a subagent finishes").
  - On the Review item for that work.
- **Fallback:** if OpenCode is not signed in or the writer fails, fall back to the expert's own one-line summary (or
  skip), never block or fail the run. Bounded time like the rollover writer.
- **Off switch:** tie to a setting (reuse the cheap-model / summary preference) so a user who does not want the extra
  calls can turn it off.

## Why app-generated, not left to the expert
Generating it in the app when the expert finishes (reusing the write-only OpenCode session) is reliable and uniform —
it always happens, in one voice, without each expert remembering to do it or spending its own (possibly expensive) model
on it. The expert's own report still goes to the Lead unchanged; this adds a plain-English layer on top. (An expert may
also be told it can ask for one itself, but the app guarantees it.)

## Anti-goals
- Spending Claude/Gemini plan on the summary (use OpenCode cheap models).
- Blocking or failing a run when the summary cannot be written.
- A summary full of file paths, code or jargon — it is for a non-technical reader.

## Open for the build
- Exactly what the writer is fed (report only, or report + a trimmed changed-files/diff view; keep it small and
  secret-masked like the rollover writer).
- Whether the Lead is instructed to weave the plain summaries into its progress posts, or the app shows them directly
  under each finished expert, or both (lean: both — the app shows it per expert, and the Lead's standing text says to
  include them when it reports).
- Where the off switch lives.

## Tests
- On an expert finishing (stand-in), the app runs the cheap writer over its report+changes and stores a plain summary;
  it appears on the expert's row/chat and in the Review item.
- Secret-masking and the bounded time carry over from the rollover writer.
- OpenCode unavailable → the expert's own one-line summary is used, run not blocked.
- The summary costs the OpenCode plan, never Claude/Gemini.

## Sequencing
Reuses the rollover/compaction summary writer, so it is a small addition on top of the shipped accounts work. Can land
before or alongside the plain-projects and RAG work; independent of the sidebar/accounts UI.
