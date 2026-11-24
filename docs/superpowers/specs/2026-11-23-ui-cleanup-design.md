# UI clean-up

Decided by the lead after auditing every screen of `ui/index.html` with the user's real data. The user is a graphic
designer who used the app all night and says the UI "feels MESSY... too many menus, too much shit everywhere".

## Principles

- One place for each thing.
- Fewer controls visible at once.
- Every explanation is one short line. Longer detail goes in a `title` tooltip or a small "More" disclosure, or is cut.
- The look stays (dark, monochrome, same fonts and pills) but calmer and more consistent.
- No feature is removed except where this spec says so: things move or fold.
- The JS's behaviour and the server API stay as they are. Element ids are kept where possible so little JS has to
  change.

## 1. Home ("All projects")

Remove the force graph canvas, its legend and its code. The page becomes a grid of the existing project cards (name,
state, stage bar, the needs-you line, cost), full width, plus a "New project" card, the totals line, and "Stop
everything" when something is running. Clicking a card opens the project (its Review if something needs the user, else
Chat).

## 2. Sidebar

Logo, project list with needs-you badges, New project. The footer is one quiet row: usage ("16M today", opens the
existing usage pop-up) and the Setup gear. The three connection dots become ONE dot that only shows when something is
not connected, and clicking it opens Setup on Connections; the separate Connections pop-up goes. The Guide drawer goes:
Guide becomes a section in Setup's left list (its rows still put an example in the message box, closing Setup).

## 3. Chat header

Keep the chat title with its chat-switching menu. Remove "Undo this run / Undo the night" from the header (it stays in
Review, Run settings). The right-hand meta becomes just the state in a few words (e.g. "Waiting for you", linking to
Review when that is where to go); token counts stay in the usage pop-up.

## 4. Message box

One box, modelled on t3code's composer (reference source, read only, MIT: the chat composer and its provider/model
picker). Inside the box's bottom row, left to right:

- one "+" button (menu: "Images or documents", "Video clips"; paste and drop keep working);
- one chip naming the AI and model ("Claude · Opus") that opens ONE picker (AIs down the left, the chosen AI's models on
  the right, effort under them; an AI that is not installed is shown as such);
- one chip naming the mode ("Ask me", "Full auto", "Overnight", "Studio") that opens a small menu of the four with one
  line each, with the two extras (Self-test, Gemini textures) under a "More" divider in that menu;
- Send on the right.

The row of four AI pills and the long hint sentence under the box go (what the hint said becomes the mode menu's line
and tooltips). The `ai-gap` warning line stays, shown only when needed.

Another builder is adding, for OpenCode when it is not signed in, a line "Free models only. Sign in to use your OpenCode
plan." with a button, in the model picker and in Setup, Connections: leave an obvious slot for a notice line at the top
of the models column and of each AI's row in Setup.

## 5. Transcript

- Do not show the Lead's protocol line: a last line of a message that is `STATUS: <WORD>` is hidden in the rendered
  text (the stored chat is unchanged).
- "copy" appears on hover or focus only.
- The "N steps" fold is quieter (small, faint).
- Expert rows are one compact line each (expert, job, AI, state).
- The Lead chat is very long (about 60,000 px in the user's data): make sure it still opens fast and scrolls to the end.

## 6. Review

Top is ONE compact run bar: state dot and word, the one-line reason, the Resume/Pause button, then the stage bar ONCE,
with cost and turns as small text at the right ("$85 · 25 turns", the cost sparkline may stay small) and "Run settings"
as the existing disclosure.

Remove the four big glance cards (Stage, Needs you, To look at, Cost): stage is already in the bar, the counts are
already in the section headings.

- "Right now" shows only while someone is working.
- "Needs you" cards stay as they are, first.
- "To look at" becomes a dense list (one row per item: title, who, when, small thumbnail if any) that expands in place
  to the full card with its actions; "Approve all" stays.
- "Tell the run something" and "Earlier decisions" stay, compact.

## 7. Setup

Left list: General, Connections, Experts, Permissions, Images (the old Textures, same controls), Chats, Guide.

Cut every paragraph to one short sentence; anything longer that the user truly needs goes behind "More". Example: the
60-word environment-keys checkbox label becomes "Use API keys from my environment" plus a More line. The Experts intro
becomes two short lines.

## 8. Promos

Drop the marketing header (eyebrow, big headline, paragraph): the tab starts with the form. Clips, What should it show,
Style and Length are visible; Music, In the style of a TikTok and Voiceover fold under "More options"; the Project
select is hidden when the tab belongs to a project. Library stays at the right.

The New project page loses its big headline the same way: name and Create first, the two links, then Ideas with one
line.

## 9. Consistency pass

- One spacing scale, one set of pill/button sizes, one heading scale.
- No text overlapping or cut off at the app's window size (1180 x 820) and down to 900 px wide.
- Visible keyboard focus everywhere.
- `prefers-reduced-motion` respected for anything animated.
- Text contrast at least 4.5:1 on its background.

## 10. Words

Plain English, short. Never "the board" in anything the user reads (Review holds its cards).

## Changed afterwards, at the user's request (2026-11-23)

Section 1 no longer holds. The page is called **Overview**, and the graph is back as its main part: every game, its
chats and its experts, with drag, zoom, double-click to fit, and a press on a game or chat to open it. The project
cards of section 1 (with the New project card, the totals line and Stop everything) sit in a column at its right,
and go under the graph below about 1000 px. The graph keeps to section 9: its key is behind one small "?", it has a
text alternative, it does not pulse with reduced motion, and it draws nothing while nothing changes.

"Expert rows are one compact line each" (section on the conversation) no longer holds either. Experts are out of the
conversation: the game in front lists its chats under it in the sidebar, and under a chat the experts it started,
newest first, each a chat of its own that opens in the main pane, read only (who, what for, its state and its AI; a
pulsing dot while it works). An arrow beside the chat shows or hides them and is remembered per chat. In the
conversation one short line stays where each expert was started ("Coder · T1 Tricks · Open its chat"), which opens
the same chat and marks it in the sidebar. The menu on the chat's name is unchanged.

Later the same day, also at the user's request: the experts list eases open and shut (not with reduced motion); the
sidebar draws no scrollbar (it still scrolls, as one list); and an expert that has not worked for half an hour is
not listed outright: it is behind one "N earlier" line at the end of its chat's experts, and comes back by itself
when it works again.

## How it is checked

- Every screen opened with a scratch copy of the user's real data at 1180 x 820 and at about 900 wide: Home, Chat (with
  the picker and the mode menu open), Review (run bar, a Needs you card, the To look at list with one item expanded),
  Promos, New project, every Setup section, the usage pop-up, the Project drawer. Before and after screenshots are kept
  under `.superpowers/ui-cleanup/`.
- `cd app-rs && cargo test` and `.venv/bin/python -m pytest tests -q` pass; tests that assert on the page's words change
  only where this spec changed the words.
- The browser console is free of errors on every screen.
- Rust and Python behaviour do not change unless a UI change needs a tiny server addition.
