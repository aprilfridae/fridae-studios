# A second account for Claude and Gemini, with automatic rollover

Asked for by the user on 2026-11-23: "im wondering if its at all possible for you to manage two different accounts of
claude, and gemeni. i want to be able to as soon as usage runs out of one, go to the next on both claude and gemeni",
then "dont do manual just do automatic", and for the hand-over: "you are compacting the chat before sending it to the
next agent right", "make it use one of the opencode models as those are cheap and basically unlimited. either glm or
deepseek 4.1 flash." The user was told once that providers may treat this as working around limits and chose to go
ahead.

## Behaviour

### Accounts
- Claude and Gemini each have the account they have today (the user's own sign-in, used by their tools outside the
  app, never changed by the app) and may have one second account.
- The second account is a sign-in of the app's own: the tool's own official sign-in, kept in a folder of the app's own
  outside every folder an AI is given. The app never reads, copies or stores a token itself.
- Setup, Connections, under Claude and under Gemini: "Second account" with Add (runs that tool's sign-in for the
  second folder, the same one-click flow as the first), then "Signed in as ..." and Remove. Remove deletes only the
  app's own second sign-in. There is still no sign-out for the first account.
- Which account a chat runs on is shown in a few words where the chat's AI and model are shown.

### Rollover
- When a turn is ended by that AI's usage limit, the account it ran on is marked used up until its reset time (taken
  from the tool's own words when it gives one, otherwise checked again after an hour).
- If the AI's other account is signed in and not used up, the chat carries on with it without the user doing
  anything: a new conversation on that account, handed the chat so far (below), and the message that hit the limit is
  sent again. That message is never lost and never sent twice.
- One short line in the chat says it happened: which account ran out, when it resets if known, which one carries on.
- Experts in a studio run do the same. New chats and new experts start on the first account that is not used up,
  the user's own account first whenever it is available again. A running chat stays where it is until its account
  runs out: no switching back mid-chat.
- When both accounts are used up, everything is as today: the run pauses until the reset, or carries on with another
  AI where that already happens.
- A kept process is started under one account, so the account is part of what it was started with.
- Usage records say which account a turn ran on.

### The hand-over is a compaction
- Today the AI a chat is switched to gets the newest part of the chat (the user's messages and each AI's replies,
  about 6,000 words) and the older part is dropped. Now the older part is replaced by a short summary, and the newest
  part stays word for word.
- The summary is written by an OpenCode model on the user's plan: `opencode-go/deepseek-v4.1-flash` first, then
  `opencode-go/glm-5.3-flash`, then `opencode-go/glm-5.3`. If none can be used (OpenCode missing, not signed in, the
  call fails or takes longer than a minute and a half), the cheapest model of the account being switched to writes
  it; if that fails too, the older part is dropped as today. A rollover never waits on a summary for long and never
  fails because of one.
- The model that writes the summary gets the text and no tools, in a conversation that is not one of the user's chats
  and is removed afterwards. What it wrote is handed over marked as a summary written by another model.
- The same compaction applies when the user switches a chat's AI by hand, and to the other same-AI hand-overs.
- The chat shows a step while the summary is written, and its small cost is recorded as usage.

## Anti-goals
- The user's own sign-ins for Claude and Gemini changed, replaced or removed by anything here.
- A message lost or sent twice at a rollover.
- A rollover loop: two accounts bouncing a chat between them.
- An agent able to switch accounts, add one, or point the second account's folder somewhere else.

## Not in scope
- More than two accounts; Cursor and OpenCode; Windows.

## Tests
- The limit messages as the tools really give them (the recorded Claude limit fixtures; Gemini's "Individual quota
  reached. ... Resets in 2h6m48s."), read into "used up until".
- Rollover with stand-ins for the tools: the message is resent once on the other account; the line is said; an expert
  does the same; both used up pauses as today; no bounce; a kept process restarts under the other account.
- New chats pick the first account that is not used up, and go back to the first when its reset time has passed.
- The compaction: the order of models, each fall-back, the time limit, the newest part untouched, nothing over the
  size the hand-over has today.
- The second account's folder is among the places no agent may write, and its settings are guarded.

## Cannot be tested without the user
- A real second account's sign-in, and a real limit reached on a real account.
