/* Pair a computer: the computers this network has that let others connect (looked for again every 3 s while the
   dialog is open), or one added by its name or address (a Tailscale name works); then the code that computer shows in
   its Settings. Paired, the dialog closes and the message box runs on it. Below, the computers this one is paired
   with, each with Forget. */
import { useEffect, useRef } from "preact/hooks";
import { cell, store, useStore, type Machine } from "../store";
import { host } from "../host";
import { post } from "../util";
import { choose, forget, loadMachines, repaired } from "./remote";
import { TROUBLE_LINE, troubleOf } from "./route";

/* `saved`: for a machine paired before, the certificate it had (its fingerprint) */
type Found = { id: string; name: string; addr: string; port?: number; fp?: string; paired?: boolean; saved?: string };
/* `why`: a machine paired before that needs pairing again, in a few words before the code is asked for;
   `forgetting`: the paired machine whose Forget was pressed, asking first */
type Pairing = { open: boolean; to: Found | null; found: Found[]; looked: boolean; code: string; error: string; busy: boolean; why: string; forgetting: string };
const closed: Pairing = { open: false, to: null, found: [], looked: false, code: "", error: "", busy: false, why: "", forgetting: "" };
const pair = cell<Pairing>(closed);
const put = (patch: Partial<Pairing>) => pair.set({ ...pair.get(), ...patch });

/* "abcdefg" -> "ABCDE-FG": capitals, no spaces, the dash after the fifth letter */
export const showCode = (typed: string) => { const c = typed.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 10); return c.length > 5 ? c.slice(0, 5) + "-" + c.slice(5) : c; };

/* Pair a computer…, or (`again`) a paired machine that needs pairing again: straight to its code. When finding hears
   it with the certificate it had, pairing is held to that certificate there; otherwise it is a fresh pairing at its
   saved address, which the other computer passes only by showing it knows the code too (the app checks that). */
export function openPair(again?: Machine) {
  const why = !again ? "" : again.trouble === "changed" ? again.name + "'s identity changed. Pair it again to be sure it is yours. "
    : again.trouble === "unpaired" ? again.name + " no longer knows this computer. " : "";
  pair.set({ ...closed, open: true, why, to: again ? { id: again.id, name: again.name, addr: again.addr, port: again.port, saved: again.fp } : null });
  host.closePops(); look();
}
function close() { pair.set(closed); }

/* each look takes about a second and a half (the app waits for answers); the next starts 3 s after it ends */
let looking = false, looked: (() => void)[] = [];
async function look() {
  if (looking) return;
  looking = true;
  while (pair.get().open) {
    const r = await post("/api/remote/discover", {});
    if (!pair.get().open) break;
    if (r && Array.isArray(r.found)) put({ found: r.found, looked: true });
    else put({ looked: true });
    looked.splice(0).forEach((f) => f());
    await new Promise((ok) => setTimeout(ok, 3000));
  }
  looking = false;
}
/* the first look's answers, waited for at most `ms` */
const firstLook = (ms: number) => pair.get().looked ? Promise.resolve() : new Promise<void>((ok) => { looked.push(ok); setTimeout(ok, ms); });

async function send() {
  let s = pair.get(); if (!s.to || s.busy) return;
  /* a computer paired before: whether finding hears it with its own certificate is known after the first look */
  if (s.to.saved && !s.looked) { put({ busy: true, error: "" }); await firstLook(4000); s = pair.get(); put({ busy: false }); if (!s.open || !s.to) return; }
  const re = s.to.saved ? s.found.find((f) => f.id === s.to!.id && f.fp === s.to!.saved) : undefined;
  const to: Found = re ? { ...s.to, addr: re.addr, port: re.port, fp: re.fp } : s.to;
  const code = s.code.replace(/-/g, "");
  if (code.length !== 10) { put({ error: "The code has ten letters and numbers." }); return; }
  put({ busy: true, error: "" });
  const body: any = { addr: to.addr, code };
  if (to.port) body.port = to.port;
  if (to.fp) body.fp = to.fp;
  const r = await post("/api/remote/pair", body);
  if (!pair.get().open) return;
  if (!r || !r.ok) { put({ busy: false, error: (r && r.error) || "Could not pair. Try again." }); return; }
  close();
  if (r.machine && r.machine.id) repaired(r.machine.id);
  await loadMachines();
  if (r.machine && r.machine.id) choose(r.machine.id);
}

/* Forget, once asked: this computer stops using that one (its chats stay there) */
async function drop(id: string) {
  put({ busy: true });
  const ok = await forget(id);
  put({ busy: false, forgetting: "", error: ok ? "" : "Couldn't forget that computer. Try again." });
  /* its row is gone: the keyboard stays in the dialog */
  setTimeout(() => document.getElementById("pair-addr")?.focus());
}
/* the row's own Forget again, after Cancel */
const forgetButton = (id: string) => setTimeout(() => (document.querySelector('#pair-machines [data-m="' + id + '"] button') as HTMLElement | null)?.focus());

const needsPairing = (id: string) => { const x = store.get().machines.find((m) => m.id === id); return !!x && (x.trouble === "unpaired" || x.trouble === "changed"); };

export function Pair() {
  const s = pair.use(), machines = useStore((st) => st.machines), addr = useRef<HTMLInputElement>(null), code = useRef<HTMLInputElement>(null);
  /* the keyboard goes to the field of the step in front */
  useEffect(() => { if (!s.open) return; (s.to ? code.current : addr.current)?.focus(); }, [s.open, !!s.to]);
  if (!s.open) return null;
  const keys = (ev: KeyboardEvent) => { if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); close(); } };
  /* "desktop" or "100.64.0.7", and a port after a colon when it is not the usual one ("desktop:47531") */
  const byAddress = () => {
    const v = (addr.current?.value || "").trim(), at = /^([^:]+):(\d{1,5})$/.exec(v);
    if (v) put({ to: at ? { id: "", name: at[1], addr: at[1], port: +at[2] } : { id: "", name: v, addr: v }, code: "", error: "" });
  };
  return (
    <div class="dlg-wrap pair-wrap" onKeyDown={keys} onClick={(ev) => { if (ev.target === ev.currentTarget) close(); }}>
      <div class="dlg pair" id="pair" role="dialog" aria-modal="true" aria-labelledby="pair-h">
        {!s.to ? <>
          <h2 id="pair-h">Pair a computer</h2>
          <p class="sub">On the other computer, open Settings and let other computers connect. It shows a pairing code there.</p>
          <h3 class="pair-h3">Computers on this network</h3>
          <div class="pair-list" id="pair-found">
            {s.found.map((f) => (
              <div class="pair-row">
                <span class="pair-t"><b>{f.name}</b><small>{f.addr}</small></span>
                {f.paired && !needsPairing(f.id) ? <span class="pair-done">Paired</span> : <button class="pill sm" onClick={() => put({ to: f, code: "", error: "" })}>{f.paired || needsPairing(f.id) ? "Pair again" : "Pair"}</button>}
              </div>
            ))}
            {!s.found.length && <p class="pair-none">{s.looked ? "None found yet. A computer shows here once it lets others connect." : "Looking…"}</p>}
          </div>
          <h3 class="pair-h3">Add by address</h3>
          <div class="keyrow">
            <input class="field" id="pair-addr" type="text" ref={addr} placeholder="Computer name or address" aria-label="Computer name or address" autocomplete="off" spellcheck={false}
              onKeyDown={(ev) => { if (ev.key === "Enter") { ev.preventDefault(); byAddress(); } }} />
            <button class="pill" id="pair-next" onClick={byAddress}>Next</button>
          </div>
          <p class="pair-hint">A Tailscale name such as desktop works too.</p>
          {machines.length > 0 && <>
            <h3 class="pair-h3">Paired computers</h3>
            <div class="pair-list" id="pair-machines">
              {machines.map((x) => s.forgetting === x.id ? (
                <div class="pair-row" data-m={x.id}>
                  <span class="pair-q">{"Forget " + x.name + "? Its chats stay on " + x.name + ". You can pair it again any time."}</span>
                  <span class="pair-acts"><button class="pill sm main" disabled={s.busy} onClick={() => drop(x.id)}>Forget</button><button class="pill sm" onClick={() => { put({ forgetting: "" }); forgetButton(x.id); }}>Cancel</button></span>
                </div>
              ) : (
                <div class="pair-row" data-m={x.id}>
                  <span class="pair-t"><b>{x.name}</b><small class="pair-st">{TROUBLE_LINE[troubleOf(x.id)] || "Online"}</small></span>
                  <button class="pill sm" aria-label={"Forget " + x.name} onClick={() => put({ forgetting: x.id, error: "" })}>Forget</button>
                </div>
              ))}
            </div>
            {s.error && <p class="pair-err" role="alert">{s.error}</p>}
          </>}
          <div class="act"><button class="pill" onClick={close}>Cancel</button></div>
        </> : <>
          <h2 id="pair-h">{"Pair with " + s.to.name}</h2>
          <p class="sub">{s.why + "Type the code " + s.to.name + " shows in its Settings. It works once, for five minutes."}</p>
          <input class="field pair-code" id="pair-code" type="text" ref={code} value={s.code} placeholder="ABCDE-FGHJK" aria-label="Pairing code" autocomplete="off" spellcheck={false} maxLength={11}
            onInput={(ev) => { const v = showCode(ev.currentTarget.value); ev.currentTarget.value = v; put({ code: v, error: "" }); }}
            onKeyDown={(ev) => { if (ev.key === "Enter") { ev.preventDefault(); send(); } }} />
          <p class="pair-err" id="pair-err" role="alert">{s.error}</p>
          <div class="act">
            <button class="pill" id="pair-back" onClick={() => put({ to: null, code: "", error: "", busy: false, why: "" })}>Back</button>
            <button class="pill main" id="pair-go" disabled={s.busy || s.code.replace(/-/g, "").length !== 10} onClick={send}>{s.busy ? "Pairing…" : "Pair"}</button>
          </div>
        </>}
      </div>
    </div>
  );
}
