/* Pair a computer: the computers this network has that let others connect (looked for again every 3 s while the
   dialog is open), or one added by its name or address (a Tailscale name works); then the code that computer shows in
   its Settings. Paired, the dialog closes and the message box runs on it. */
import { useEffect, useRef } from "preact/hooks";
import { cell } from "../store";
import { host } from "../host";
import { post } from "../util";
import { choose, loadMachines } from "./remote";

type Found = { id: string; name: string; addr: string; port?: number; fp?: string; paired?: boolean };
type Pairing = { open: boolean; to: Found | null; found: Found[]; looked: boolean; code: string; error: string; busy: boolean };
const closed: Pairing = { open: false, to: null, found: [], looked: false, code: "", error: "", busy: false };
const pair = cell<Pairing>(closed);
const put = (patch: Partial<Pairing>) => pair.set({ ...pair.get(), ...patch });

/* "abcdefg" -> "ABCDE-FG": capitals, no spaces, the dash after the fifth letter */
export const showCode = (typed: string) => { const c = typed.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 10); return c.length > 5 ? c.slice(0, 5) + "-" + c.slice(5) : c; };

export function openPair() { pair.set({ ...closed, open: true }); host.closePops(); look(); }
function close() { pair.set(closed); }

/* each look takes about a second and a half (the app waits for answers); the next starts 3 s after it ends */
let looking = false;
async function look() {
  if (looking) return;
  looking = true;
  while (pair.get().open) {
    const r = await post("/api/remote/discover", {});
    if (!pair.get().open) break;
    if (r && Array.isArray(r.found)) put({ found: r.found, looked: true });
    else put({ looked: true });
    await new Promise((ok) => setTimeout(ok, 3000));
  }
  looking = false;
}

async function send() {
  const s = pair.get(), to = s.to; if (!to || s.busy) return;
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
  await loadMachines();
  if (r.machine && r.machine.id) choose(r.machine.id);
}

export function Pair() {
  const s = pair.use(), addr = useRef<HTMLInputElement>(null), code = useRef<HTMLInputElement>(null);
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
          <div class="pair-list">
            {s.found.map((f) => (
              <div class="pair-row">
                <span class="pair-t"><b>{f.name}</b><small>{f.addr}</small></span>
                {f.paired ? <span class="pair-done">Paired</span> : <button class="pill sm" onClick={() => put({ to: f, code: "", error: "" })}>Pair</button>}
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
          <div class="act"><button class="pill" onClick={close}>Cancel</button></div>
        </> : <>
          <h2 id="pair-h">{"Pair with " + s.to.name}</h2>
          <p class="sub">{"Type the code " + s.to.name + " shows in its Settings. It works once, for five minutes."}</p>
          <input class="field pair-code" id="pair-code" type="text" ref={code} value={s.code} placeholder="ABCDE-FGHJK" aria-label="Pairing code" autocomplete="off" spellcheck={false} maxLength={11}
            onInput={(ev) => { const v = showCode(ev.currentTarget.value); ev.currentTarget.value = v; put({ code: v, error: "" }); }}
            onKeyDown={(ev) => { if (ev.key === "Enter") { ev.preventDefault(); send(); } }} />
          <p class="pair-err" id="pair-err" role="alert">{s.error}</p>
          <div class="act">
            <button class="pill" id="pair-back" onClick={() => put({ to: null, code: "", error: "", busy: false })}>Back</button>
            <button class="pill main" id="pair-go" disabled={s.busy || s.code.replace(/-/g, "").length !== 10} onClick={send}>{s.busy ? "Pairing…" : "Pair"}</button>
          </div>
        </>}
      </div>
    </div>
  );
}
