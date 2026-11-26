/* What every thing in Review has: its box (answered, it is one line), and under its title either Approve and Ask
   for changes (finished work) or Done (a to-do only the user can do). */
import type { ComponentChildren } from "preact";
import { useRef } from "preact/hooks";
import { api } from "../api";
import { useKept, type Kept, type Thing } from "./model";
import type { Answer } from "./Review";

export type ItemProps = { thing: Thing; flash: boolean; onAnswered: Answer };
/* what a thing's box remembers, and how its parts change it */
type Part = { thing: Thing; kept: Kept; keep: (patch: Partial<Kept>) => void; onAnswered: Answer };

export function Card({ thing, flash, children }: { thing: Thing; flash: boolean; children: (kept: Kept, keep: (patch: Partial<Kept>) => void) => ComponentChildren }) {
  const [kept, keep] = useKept(thing.key);
  const cls = "rv-item rv-card" + (kept.entered ? "" : " enter") + (thing.said !== null ? " done" : "") + (flash ? " flash" : "");
  return (
    <div class={cls} data-id={thing.id} onAnimationEnd={kept.entered ? undefined : () => keep({ entered: true })}>
      {children(kept, keep)}
      <p class="rv-err">{kept.err || null}</p>
      <div class="rv-said">{thing.said}</div>
    </div>
  );
}

/* Approve, or Ask for changes: the box to write in comes when that is pressed */
export function Decide({ thing, kept, keep, onAnswered }: Part) {
  const it = thing.item!, title = it.title || "Untitled";
  const input = useRef<HTMLInputElement>(null), ask = useRef<HTMLButtonElement>(null);
  /* the box opens and takes the keyboard at once, not a frame later: what is typed next lands in it */
  const toggle = () => {
    const el = input.current!, open = !kept.open;
    if (open) { (el.parentElement as HTMLElement).hidden = false; el.focus(); }
    keep({ open });
  };
  const decide = async (body: object, said: string) => {
    keep({ busy: true, err: "" });
    let res; try { res = await api("/api/review", { path: thing.path, id: it.id, ...body }); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
    if (!res.ok) { keep({ busy: false, err: res.error || "That did not work." }); return; }
    onAnswered(it.id, said);
  };
  const sendChange = () => {
    const note = kept.note.trim();
    if (!note) { input.current!.focus(); keep({ err: "Say what to change." }); return; }
    decide({ action: "change", note }, "Changes asked for: " + note);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); sendChange(); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); ask.current!.focus(); keep({ note: "", open: false }); }
  };
  return (
    <>
      <h4 title={it.plain || it.summary || ""}>{title}</h4>
      <div class="act">
        <button class="pill main" disabled={kept.busy} onClick={() => decide({ action: "approve" }, "Approved: " + title)}>Approve</button>
        <button class="pill rv-ask" aria-expanded={kept.open ? "true" : "false"} ref={ask} onClick={toggle}>Ask for changes</button>
      </div>
      <div class="rv-change" hidden={!kept.open}>
        <input class="field" type="text" maxLength={600} placeholder="What should change?" aria-label="What should change" ref={input}
          value={kept.note} onInput={(e) => keep({ note: (e.target as HTMLInputElement).value, err: "" })} onKeyDown={onKey} />
        <button class="pill" disabled={kept.busy || !kept.note.trim()} onClick={sendChange}>Send</button>
      </div>
    </>
  );
}

/* a to-do only the user can do (a board card in the needs column): its title, and Done */
export function DoneButton({ thing, kept, keep, onAnswered }: Part) {
  const c = thing.need!, path = thing.path;
  const done = async () => {
    keep({ busy: true, err: "" });
    let r; try { r = await api("/api/board", { path, action: "move", id: c.id, column: "done" }); } catch (e) { r = { ok: false }; }
    if (!r || !r.ok) { keep({ busy: false, err: (r && r.error) || "Could not move the card." }); return; }
    onAnswered(c.id, "Done: " + (c.title || ""));
    /* the Lead of the project's studio run is told, so a run waiting on it goes on */
    if (path && c.title) api("/api/review", { path, action: "needs_done", title: c.title }).catch(() => {});
  };
  return (
    <>
      <h4 title={c.note || ""}>{c.title || ""}</h4>
      <div class="act"><button class="pill main" disabled={kept.busy} onClick={done}>Done</button></div>
    </>
  );
}
