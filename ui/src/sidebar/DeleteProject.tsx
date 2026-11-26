/* Delete: a dialog that names the project and its folder and says what each choice does. The first choice keeps
   every file; the other moves the folder where it can be put back from. Nothing happens without a press here. */
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { cell, store } from "../store";
import { host } from "../host";
import { projRow } from "./ProjectMenu";

const BIN = /Mac/.test(navigator.userAgent) ? "Trash" : "Recycle Bin";
/* the project being asked about (`on`), or the one last asked about */
const asking = cell<{ path: string; name: string; on: boolean; n: number }>({ path: "", name: "", on: false, n: 0 });
export function askDelete(path: string) {
  const pr = store.get().projects.find((x) => x.path === path); if (!pr) return;
  asking.set({ path, name: pr.name, on: true, n: asking.get().n + 1 });
}

export function DeleteProject() {
  const a = asking.use(), wrap = useRef<HTMLDivElement>(null);
  const [trash, setTrash] = useState(false), [busy, setBusy] = useState(false), [err, setErr] = useState("");
  /* each time it opens: the first choice, which keeps every file */
  useLayoutEffect(() => {
    if (!a.on) return;
    setTrash(false); setBusy(false); setErr("");
    const first = wrap.current && wrap.current.querySelector<HTMLInputElement>('input[value="list"]'); if (first) { first.checked = true; first.focus(); }
  }, [a.n]);
  const close = () => { asking.set({ ...a, on: false }); const row = a.path && projRow(a.path); if (row) row.focus(); };
  const keys = (ev: KeyboardEvent) => {
    ev.stopPropagation();
    if (ev.key === "Escape") { ev.preventDefault(); close(); return; }
    if (ev.key !== "Tab") return;
    /* the keyboard stays in the dialog while it is open */
    const el = wrap.current!, at = document.activeElement as HTMLElement;
    const stops = [el.querySelector<HTMLElement>("input:checked"), document.getElementById("pj-del-no"), document.getElementById("pj-del-go")], i = stops.indexOf(at.matches("input") ? stops[0] : at);
    ev.preventDefault(); stops[(i + (ev.shiftKey ? stops.length - 1 : 1)) % stops.length]!.focus();
  };
  const go = async () => {
    const path = a.path, pr = store.get().projects.find((x) => x.path === path);
    if (!path || !pr) { close(); return; }
    setBusy(true); setErr("");
    let r; try { r = await api("/api/project", { action: "delete", path, trash }); } catch (e) { r = { ok: false, error: "The app did not answer." }; }
    setBusy(false);
    if (!r || !r.ok) { setErr((r && r.error) || "That did not work."); return; }
    asking.set({ ...a, on: false });
    await host.projectDeleted(path);
    host.say(pr.name + (trash ? " is in the " + BIN + "." : " is no longer in Fridae. Its folder is untouched."));
  };
  return (
    <div class="dlg-wrap" id="pj-del" ref={wrap} hidden={!a.on} onKeyDown={keys} onMouseDown={(ev) => { if (ev.target === wrap.current) close(); }}><div class="dlg" role="dialog" aria-modal="true" aria-labelledby="pj-del-h" aria-describedby="pj-del-path">
      <h2 id="pj-del-h">{a.name ? "Delete " + a.name + "?" : ""}</h2>
      <p class="sub" id="pj-del-path">{a.path}</p>
      <div class="dlg-opts" role="radiogroup" aria-label="What happens to it">
        <label class="dlg-opt"><input type="radio" name="pj-del-how" value="list" checked={!trash} onChange={() => setTrash(false)} /><span><b>Remove from Fridae</b><small>It leaves Fridae's list. The folder and every file in it stay where they are.</small></span></label>
        <label class="dlg-opt"><input type="radio" name="pj-del-how" value="trash" checked={trash} onChange={() => setTrash(true)} /><span><b id="pj-del-bin">{a.name ? "Move the folder to the " + BIN : ""}</b><small id="pj-del-bin-note">{a.name ? "It leaves Fridae's list, and its folder goes to the " + BIN + ". You can put it back from there." : ""}</small></span></label>
      </div>
      <p class="sub">Its chats stay in your history. Nothing is deleted for good.</p>
      <p class="rv-err" id="pj-del-err">{err}</p>
      <div class="act"><button class="pill" id="pj-del-no" disabled={busy} onClick={close}>Cancel</button><button class="pill main" id="pj-del-go" disabled={busy} onClick={go}>{a.name ? (trash ? "Move to " + BIN : "Remove from Fridae") : ""}</button></div>
    </div></div>
  );
}
