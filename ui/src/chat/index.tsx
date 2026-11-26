/* The chat: the bar over the tabs, the bar above the conversation, the conversation itself and the message box.
   `chat` is what the old page (ui/index.html) reaches it through, as fridae.chat. */
import { setChatHost, H } from "./host";
import { calm, closed, draft, event, frontTl, leadLine, opened, poke, showDraft, toLatest, tlOf } from "./chatStore";
import { C, addClips, addFile, attachedHere, closePops, focusBox, moveOpenCodePick, setGeminiMode, setText } from "./compose";
import { ChatHead, TopBar, head } from "./Head";
import { Timeline } from "./Timeline";
import { Dock } from "./Composer";
import "./chat.css";

export { TopBar };
export function ChatMain() { return <><ChatHead /><Timeline /><Dock /></>; }

export const chat = {
  init: setChatHost,
  poke,                                   /* something the chat's bars or the message box show has changed */
  opened, closed, event,                  /* a chat was made; a chat was closed; an agent's event (chatStore.ts) */
  draft: showDraft,                       /* the empty page of a project with no chat yet */
  /* a chat came to the front: it is simply there (nothing fades in), and the keyboard goes to the message box */
  front(focus: boolean) { calm(); poke(); if (focus) focusBox(); },
  calm,
  toLatest() { const tl = frontTl(); if (tl) toLatest(tl); },
  refreshed() { moveOpenCodePick(); poke(); },   /* the app's status was asked for again */
  line: leadLine,                         /* one line in a chat's turn that leads to Promos or Review */
  setGemini: setGeminiMode, addClips, attached: attachedHere, addFile, setBox: (v: string) => setText(v, true), focus: focusBox,
  closePops: () => closePops(), closeConv: () => head.closeConv(), closeRun: () => head.closeRun(),
  /* Escape: the menus close, and one of the message box's that had the keyboard hands it back to its button */
  escape() {
    const m = C.pop && document.getElementById({ add: "add-menu", model: "settings-panel", mode: "mode-menu" }[C.pop as "add"]);
    const had = m && m.contains(document.activeElement) ? C.pop : "";
    closePops(); head.closeConv();
    if (had) document.getElementById(had)!.focus();
  },
  /* for the harness and the curious: how many rows a chat has */
  rows: (id: string) => { const p = H.agents.get(id); return p ? tlOf(p).rows.length : draft.tl ? draft.tl.rows.length : 0; },
};
