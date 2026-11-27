/* The machine chip in the message box, left of the AI chip: which computer the chat runs on, and a menu of the user's
   computers (This Mac or This PC, each paired computer with a dot for online, Pair a computer…). Before the first
   message a pick starts the chat there; after it, a pick moves the chat there with everything said so far
   (./move.ts): at once, or when the reply it is writing is over. Then the chip names both ("Laptop → Desktop") until
   it has moved, and picking where it is takes the move back. When the other computer has no project of the chat's
   project's name, the menu asks which of its projects (or none). An expert's chat stays with the chat that started
   it: its chip only names the computer. */
import { useStore, type Machine } from "../store";
import { C, closePops, togglePop } from "../chat/compose";
import { H } from "../chat/host";
import { boxMachine, choose } from "./remote";
import { HERE, TROUBLE_LINE, nameOf, troubleOf } from "./route";
import { asking, moveChat, movingTo } from "./move";
import { openPair } from "./Pair";
import "./remote.css";

const MONITOR = <svg class="mc-icon" width="13" height="13" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="1.5" y="2" width="10" height="7" rx="1.2" /><path d="M4.5 11.5h4M6.5 9v2.5" /></svg>;
const DOWN = <svg class="down" width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3.5l2.5 2.5L7 3.5" /></svg>;
const TICK = <span class="tick"><svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 5.5l2.5 2.5 4.5-5.5" /></svg></span>;

const ARROW = <svg class="mc-to" width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2 5.5h7M6.5 3l2.5 2.5L6.5 8" /></svg>;

export function MachineChip({ p }: { p: any }) {
  const machines = useStore((s) => s.machines);
  const m = boxMachine(), name = nameOf(m), open = C.pop === "machine";
  const started = !!(p && p.started);
  if (started && H.isHelper(p)) return <span class="chip mc fixed" id="machine" title={"This chat runs on " + name}>{MONITOR}<span>{name}</span></span>;
  const to = started ? movingTo(p.id) : null, ask = started && asking.id === p.id ? asking : null;
  /* a machine that needs pairing again (it no longer knows this computer, or its identity changed) is not picked:
     the press opens pairing for it */
  const pick = (id: string) => {
    closePops();
    const x = machines.find((y) => y.id === id);
    if (x && (x.trouble === "unpaired" || x.trouble === "changed")) { openPair(x); return; }
    document.getElementById("machine")!.focus();
    if (started) { moveChat(p.id, id); return; }
    choose(id);
  };
  const row = (id: string, label: string, x?: Machine) => {
    const t = x ? troubleOf(id) : "", line = t === "unpaired" || t === "changed" ? TROUBLE_LINE[t] : id === to ? "Moves here when this reply is over" : "";
    return (
      <button class="opt" role="menuitemradio" data-m={id} aria-checked={id === m ? "true" : "false"} onClick={() => pick(id)}>
        {TICK}<span class="mc-row"><span class="mc-t"><b>{label}</b>{line && <small>{line}</small>}</span>{x && <span class={"dot" + (x.online ? " ok" : "")} title={x.online ? "Online" : TROUBLE_LINE[t] || "Offline"} />}</span>
      </button>
    );
  };
  const title = to !== null ? "Moves to " + nameOf(to) + " when this reply is over. Press to change." : started ? "Runs on " + name + ". Press to move this chat to another of your computers." : "Runs on " + name + ". Press to run this chat on another of your computers.";
  const answer = (path: string | null) => { closePops(); moveChat(p.id, ask!.to, path); };
  return (
    <span class="pick">
      <button class={"chip mc" + (to !== null ? " moving" : "")} id="machine" aria-haspopup="menu" aria-expanded={open ? "true" : "false"} title={title}
        onClick={(ev) => { ev.stopPropagation(); if (!togglePop("machine") && ask) asking.id = ""; }}>
        {MONITOR}<span>{name}</span>{to !== null && <>{ARROW}<span>{nameOf(to)}</span></>}{DOWN}
      </button>
      {ask ? (
        <div class={"menu" + (open ? " on" : "")} id="machine-menu" role="menu" aria-label={"Which project on " + nameOf(ask.to)}>
          <p class="mc-q">{nameOf(ask.to) + " has no project called “" + ((p.project && p.project.name) || "") + "”. Which of its projects should the chat use?"}</p>
          {ask.list.map((x) => <button class="opt" role="menuitem" data-proj={x.path} onClick={() => answer(x.path)}><span class="tick" /><span><b>{x.name}</b></span></button>)}
          <button class="opt" role="menuitem" id="mc-no-project" onClick={() => answer(null)}><span class="tick" /><span><b>No project</b></span></button>
          <hr />
          <button class="opt" role="menuitem" id="mc-cancel" onClick={() => { asking.id = ""; closePops(); }}><span class="tick" /><span><b>Cancel</b></span></button>
        </div>
      ) : (
        <div class={"menu" + (open ? " on" : "")} id="machine-menu" role="menu" aria-label={started ? "Move this chat to" : "Which computer it runs on"}>
          {started && <p class="mc-q">Move this chat to another computer. Everything said so far goes with it.</p>}
          {row("", "This " + HERE)}
          {machines.map((x) => row(x.id, x.name, x))}
          <hr />
          <button class="opt" id="pair-open" role="menuitem" onClick={() => { closePops(); openPair(); }}><span class="tick" /><span><b>Pair a computer…</b></span></button>
        </div>
      )}
    </span>
  );
}
