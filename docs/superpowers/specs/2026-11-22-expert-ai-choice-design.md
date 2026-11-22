# Experts on a chosen AI and model

Approved by the user on 2026-11-22 ("Yes, build it"). Their example: "3d modeling only happens in newest astra."

## Why

Every expert runs on the AI of the chat that started it. With Claude, Gemini and Cursor all available, the user wants some kinds of work to always go to a particular AI and model, and wants agents to be able to ask for one.

## Behaviour

1. **The user's table (wins).** Setup gets "Experts": one row per roster expert with an AI and a model, both defaulting to "Same as the Lead". Saved in the app's config as `expert_ai: {<expert id>: {"ai": "...", "model": "..."}}`. A job handed to that expert always runs there.
2. **The Lead's request.** `rbx_team` spawn takes optional `ai` and `model`. Used only when the user's table has nothing for that expert, and only while Setup's "The Lead may pick an AI for a job" is on (it is on unless the user switches it off). When it is off, a request with `ai` or `model` is refused with a plain reason; the user's table still applies.
3. **Otherwise** the expert runs on the Lead's AI and the same model as the Lead (the model the Lead's chat was started on, else its last message's). Ruled by the user on 2026-11-22: same model as the Lead. This means a Lead on a big model uses up that plan faster, since every expert left on "Same as the Lead" runs on that model too; Setup and the Guide say so.

Once the table names an AI (even the Lead's own), the expert uses that AI's default model unless the table names a model too; Setup says so.

## Rules

- Only AIs that are installed and can run experts may be chosen; the page offers only those, and the model list is that AI's own list.
- A choice that cannot be honoured (AI missing or signed out, unknown model) never fails the job silently and never falls through to a different AI unannounced: the spawn is refused with a plain reason the Lead can act on ("Gemini is not installed; run this expert on Claude or ask the user"), except that an unknown model on a valid AI falls back to that AI's default model and says so in the result.
- The expert's line in the Lead's chat and in Review's Right now shows its AI and model when they differ from the Lead's.
- Usage is recorded under the AI that actually ran.
- The safety rules of each AI's setup apply unchanged. Cursor's documented gaps apply to an expert run on Cursor; the Experts table says so beside Cursor.
- A model id from an agent is checked before use (letters, digits and `. _ : [ ] = , - /`, not starting with a dash, at most 120 characters); one that fails is refused. An expert chat keeps its model when the user writes in it.
- The Lead's instructions (studio guide / lead text) say the option exists, that the user's table wins, and to use it sparingly: for a kind of work an AI is clearly better at, not per whim.

## Not in scope

- Experts choosing an AI for their own sub-helpers beyond what `rbx_team` already allows them.
- Automatic "best AI for the job" routing.

## Left to the builder

- Where the resolution lives (one pure function: table, request, Lead's default → AI, model, or a refusal), with tests.
- Markup of the Experts table, reusing Setup's existing patterns.
