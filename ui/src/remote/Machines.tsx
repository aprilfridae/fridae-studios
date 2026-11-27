/* The machine chip in the message box, left of the AI chip: which computer the chat runs on. Before the first message
   it is a menu (This PC, each paired computer with a dot for online, Pair a computer…); after it, the chat stays on
   its computer and the chip only names it. */
import { useStore, type Machine } from "../store";
import { C, closePops, togglePop } from "../chat/compose";
import { boxMachine, choose } from "./remote";
import { TROUBLE_LINE, nameOf, troubleOf } from "./route";
import { openPair } from "./Pair";
import "./remote.css";

const MONITOR = <svg class="mc-icon" width="13" height="13" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="1.5" y="2" width="10" height="7" rx="1.2" /><path d="M4.5 11.5h4M6.5 9v2.5" /></svg>;
const DOWN = <svg class="down" width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3.5l2.5 2.5L7 3.5" /></svg>;
const TICK = <span class="tick"><svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 5.5l2.5 2.5 4.5-5.5" /></svg></span>;

export function MachineChip({ p }: { p: any }) {
  const machines = useStore((s) => s.machines);
  const m = boxMachine(), name = nameOf(m), open = C.pop === "machine";
  if (p && p.started) return <span class="chip mc fixed" id="machine" title={"This chat runs on " + name}>{MONITOR}<span>{name}</span></span>;
  /* a machine that needs pairing again (it no longer knows this computer, or its identity changed) is not picked:
     the press opens pairing for it */
  const pick = (to: string) => {
    closePops();
    const x = machines.find((y) => y.id === to);
    if (x && (x.trouble === "unpaired" || x.trouble === "changed")) { openPair(x); return; }
    document.getElementById("machine")!.focus(); choose(to);
  };
  const row = (id: string, label: string, x?: Machine) => {
    const t = x ? troubleOf(id) : "", line = t === "unpaired" || t === "changed" ? TROUBLE_LINE[t] : "";
    return (
      <button class="opt" role="menuitemradio" data-m={id} aria-checked={id === m ? "true" : "false"} onClick={() => pick(id)}>
        {TICK}<span class="mc-row"><span class="mc-t"><b>{label}</b>{line && <small>{line}</small>}</span>{x && <span class={"dot" + (x.online ? " ok" : "")} title={x.online ? "Online" : TROUBLE_LINE[t] || "Offline"} />}</span>
      </button>
    );
  };
  return (
    <span class="pick">
      <button class="chip mc" id="machine" aria-haspopup="menu" aria-expanded={open ? "true" : "false"} title={"Runs on " + name + ". Press to run this chat on another of your computers."}
        onClick={(ev) => { ev.stopPropagation(); togglePop("machine"); }}>{MONITOR}<span>{name}</span>{DOWN}</button>
      <div class={"menu" + (open ? " on" : "")} id="machine-menu" role="menu" aria-label="Which computer it runs on">
        {row("", "This PC")}
        {machines.map((x) => row(x.id, x.name, x))}
        <hr />
        <button class="opt" id="pair-open" role="menuitem" onClick={() => { closePops(); openPair(); }}><span class="tick" /><span><b>Pair a computer…</b></span></button>
      </div>
    </span>
  );
}
