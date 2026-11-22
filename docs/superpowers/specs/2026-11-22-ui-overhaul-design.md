# UI overhaul: projects, three tabs, experts under the Lead

Approved by the user on 2026-11-22 ("Yes, build it").

## Why

The window grew one tab per feature (Chats, Board, Review, Promos, Research, Overview) and one chat row per agent, split by AI. The user works on games, not on chats: they talk to one Lead per game and sign work off. The window should be shaped like that.

## Goals

- **Projects are the navigation.** The left side lists the user's games and "New project", nothing else. Picking a game scopes everything on the right to it.
- **Three tabs per project: Chat, Review, Promos.**
- **Chats belong to a project, not to an AI.** Any project's chats can use Claude or Gemini; chats and projects persist as they do today.
- **Experts never get their own chat rows.** They appear collapsed inside the chat of the Lead (or parent chat) that started them: one line each (expert, job, state), opening to show that expert's log.
- **Review is the one place for "what is happening and what needs me"**: Overview and Review merged, with the board's needs cards.

## Non-goals

- No change to how agents, runs, review items, notes or promos work underneath, beyond what the page needs.
- No new visual language: keep the existing tokens, type scale (body 15px or larger; the user has poor eyesight), text-size shortcuts and quiet chrome. Sentence-case labels.

## Anti-goals

- Anything that needs a second click to reach "what needs me".
- More than one row per game in the sidebar while its agents work.

## Removed

- The **Research**, **Board** and **Overview** tabs.
- **ChatGPT** from the page (pickers, setup rows, help text). The backend code may stay, unreachable from the UI.
- The per-AI split of the chat list (`viewAi`).

## Layout

```
┌────────────────┬──────────────────────────────────────────────┐
│ Projects       │  <game name>          Chat | Review | Promos │
│ ▸ Skip Stone … │                                              │
│   Odd One In   │   the page for the chosen tab                │
│   + New project│                                              │
└────────────────┴──────────────────────────────────────────────┘
```

- **All projects** is the first row of the sidebar (added by the user on 2026-11-22): one page covering every game being worked on. Per game, one card: name, run state in words, the stage strip, who is working on what right now, how many things need the user (with the first one named), and cost so far. A card opens that game's Review. Games with nothing going sit below in a short quiet list. The page's main visual is the live graph from the old Overview tab (the user's Obsidian-style graph), kept as they made it: projects, their chats, and experts under their Lead.
- A project row shows the game's name and one state mark: working, needs you (with a count), or idle.
- Chats that have no project (older ones, or a new chat before its folder exists) sit under one "No project" row, shown only when such chats exist.
- Settings, help, the update button and Studio's connection state keep a place in the sidebar's foot or the header.

## Chat tab

- Shows the project's current conversation. A small control at the top lists the project's other conversations and "New chat" (delete, rename and restore stay available there).
- Under the message box: the AI switch (Claude, Gemini) with the model picker, and the existing mode menu (Studio, overnight, and so on).
- **Switching AI in a conversation that has started**: the conversation carries on with the other AI, which is handed the chat so far as text with its first message (the user's and the agent's messages, capped; no tool logs). The page says so in one line when the switch is made. Before a conversation has started, switching just sets its AI as today.
- **Experts**: where the Lead's log shows an expert being started, the chat shows that expert as one collapsible line that updates live (name, job title, working or finished or stopped). Open, it shows the expert's own log read-only, with Stop while it works. Experts are not listed anywhere else.

## Review tab

Top to bottom:

1. **Run line**: the run's state in words with its one button (Pause, Resume), the stage strip, and run settings (gates, experts at once, End, Undo this run) behind one control.
2. **Right now**: who is working on what (the Lead and each expert at work, one line each, live), and the cost so far for this project.
3. **Needs you**: review items that block, plus the board's needs cards for this game, each with its action (Approve, Ask for changes, Done).
4. **To look at**: finished work with pictures or clips, Approve and a box to ask for changes; Approve all.
5. **Tell the run something**: a box that sends a message to the Lead without leaving the page.
6. Earlier decisions, collapsed.

With no run going: one line saying so and how to start one (Studio mode in Chat).

## Promos tab

The existing Promos page, shown for the chosen project where it is per game; engine setup and the library stay as they are.

## New project

"+ New project" opens a page with:

- a name field and Create (makes the folder as today, then opens its Chat);
- **Ideas**: the idea cards that lived in Research, with "Find ideas", and on each card Start this game, Start a studio run, Copy. The charts tables are dropped.

## Approaches considered

1. **Re-skin in place** (keep six tabs, restyle): cheapest, does not fix the shape. Rejected.
2. **Project-first shell over the existing pieces** (chosen): new navigation and a merged Review, reusing the existing chat, review, promo and idea rendering. Most of the value, least rewrite.
3. **Rewrite the page as components**: cleanest code, weeks of regressions for no user-visible gain now. Rejected.

## Left to the builder

- Exact markup and which existing functions are reused or deleted; dead CSS and JS for removed tabs should go.
- Whether small backend additions are needed (for example a route to switch a started chat's AI with the transcript handed over, and per-project cost); keep them small and tested.
- Help text, README, Guide and DEVELOPING entries that name the removed tabs must be updated.
