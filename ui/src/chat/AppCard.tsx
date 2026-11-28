/* The card the app toolbox raises before an AI changes something of the app's below Full auto (and always for a few
   changes): its layout is T3 Code's request card (apps/web/src/components/chat/SecretRequestCard.tsx; MIT, see
   THIRD_PARTY.md) in Fridae's own `.ask` look: a title saying the action, one plain sentence, a main button naming
   it and a quieter "Don't". The answer goes straight to the app (POST /api/app-card), never through the AI. Once the
   app has closed the card it is one short line, "Rename project · allowed", like the turn's other notes. */
import { api } from "../api";
import { H } from "./host";
import { fresh, partChanged, type Part, type Row } from "./chatStore";
import { OUTCOME } from "./appTools";

export function AppCard({ pt, row, p }: { pt: Part; row: Row; p: any }) {
  if (pt.said || pt.shut) return <span class={"tag appcard-done" + (fresh(pt.born) ? " enter" : "")}>{pt.title + " · " + (OUTCOME[pt.said] || "closed")}</span>;
  const answer = async (allow: boolean) => {
    if (pt.sent) return;
    pt.sent = true; partChanged(row, pt);
    let r: any = null;
    try { r = await api("/api/app-card", { agent: p.id, card: pt.card, answer: allow ? "allow" : "decline" }); } catch (e) { r = null; }
    /* answered: the app closes the card (appcard_done) and it becomes its line. A card that had closed already
       says so in the same way; only an answer that did not get there can be given again */
    if (!r) { pt.sent = false; partChanged(row, pt); H.say("Your answer didn't reach the app. Try again."); }
  };
  return (
    <div class={"ask appcard" + (fresh(pt.born) ? " enter" : "")} role="group" aria-label={pt.title}>
      <h4>{pt.title}</h4>
      <p>{pt.text}</p>
      <div class="act">
        <button class="pill main" disabled={pt.sent} onClick={() => answer(true)}>{pt.allow}</button>
        <button class="pill" disabled={pt.sent} onClick={() => answer(false)}>Don't</button>
      </div>
    </div>
  );
}
