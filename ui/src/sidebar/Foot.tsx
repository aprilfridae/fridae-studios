/* The sidebar's foot: the update button when an update is on its way or ready, Usage (a pop-up that opens upwards,
   with the way to the Usage window), one dot while something the chat in front needs is not connected, and
   Settings. */
import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { cell, store, useStore } from "../store";
import { openSetup } from "../windows";
import { UsagePop } from "../usage/UsagePop";
import { loadUsage } from "../usage/usage";

/* the usage pop-up is open */
const pop = cell(false);

/* "Restart to update", or how far the download is. With chats at work the first press only asks. */
function Update() {
  const u = useStore((s) => (s.status && s.status.update) || null) || {}, polls = useStore((s) => s.polls);
  const [sure, setSure] = useState(false), [going, setGoing] = useState(false), t = useRef(0), tip = useRef<string | undefined>(undefined), said = useRef("");
  const ready = u.status === "ready", busy = u.status === "downloading";
  /* "Restarting..." stands until the status is next asked for: if the app is still here then, the button is back */
  useEffect(() => { setGoing(false); }, [polls]);
  if (ready) tip.current = "Version " + u.latest + (u.notes ? "\n\n" + u.notes : "");
  const press = async () => {
    if (!ready) return;
    if (store.get().chats.some((p) => p.busy) && !sure) { setSure(true); clearTimeout(t.current); t.current = setTimeout(() => setSure(false), 4000) as unknown as number; return; }
    setGoing(true); await api("/api/update", { action: "install" });
  };
  const words = going ? "Restarting..." : sure && ready ? "Chats are working. Restart anyway?" : ready ? "Restart to update" : busy ? "Updating " + Math.round((u.progress || 0) * 100) + "%" : "";
  if (words) said.current = words;   /* out of sight it keeps its last words, as it keeps its tooltip */
  return <button class={"upd" + (busy ? " busy" : "")} id="b-update" hidden={!ready && !busy} title={tip.current} onClick={press}>{said.current}</button>;
}

export function Foot() {
  const on = pop.use(), off = useStore((s) => s.notConnected), usage = useStore((s) => s.usageOpen), use = useRef<HTMLButtonElement>(null);
  const said = off ? "Not connected: " + off.join(", ") + ". Press to see how to fix it." : undefined;
  useEffect(() => { if (usage) pop.set(false); }, [usage]);
  useEffect(() => {
    const away = (ev: MouseEvent) => { const t = ev.target as Element; if (pop.get() && !t.closest("#use-pop") && !t.closest("#b-use")) pop.set(false); };
    /* Escape closes the pop-up and gives the keyboard back to its button */
    const esc = (ev: KeyboardEvent) => { if (ev.key === "Escape" && pop.get()) { ev.stopPropagation(); pop.set(false); if (use.current) use.current.focus(); } };
    document.addEventListener("click", away); document.addEventListener("keydown", esc, true);
    return () => { document.removeEventListener("click", away); document.removeEventListener("keydown", esc, true); };
  }, []);
  return (
    <div class="side-foot">
      <Update />
      <div class="foot-row">
        <button class="icon" id="b-use" ref={use} aria-expanded={on ? "true" : "false"} aria-controls="use-pop" aria-label="Usage" title="Usage" onClick={(ev) => { ev.stopPropagation(); pop.set(!on); if (!on) loadUsage(); }}>
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 13.5h11M4.5 11V8M8 11V3.5M11.5 11V6" /></svg>
        </button>
        <span class="grow"></span>
        <button class="conn" id="b-conn" hidden={!off || !off.length} title={said} aria-label={said} onClick={() => openSetup("connections")}><span class="dot miss"></span></button>
        <button class="icon" id="b-setup" aria-label="Settings" title="Settings" onClick={() => openSetup()}>
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" /><circle cx="12" cy="12" r="3" /></svg>
        </button>
      </div>
    </div>
  );
}
/* the pop-up itself, which opens upwards from the foot */
export function UsePop() {
  return <div class={"pop" + (pop.use() ? " on" : "")} id="use-pop"><UsagePop /></div>;
}
