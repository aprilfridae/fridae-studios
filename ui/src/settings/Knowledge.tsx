/* Settings, Knowledge: the knowledge index, Fridae's vault (Download, Pull newest and what the last one brought),
   sharing what you make, and the maintainer's submissions. */
import { useState } from "preact/hooks";
import { openSetup } from "../windows";
import { ago } from "../util";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Segmented } from "../ui/Segmented";
import { More } from "../ui/More";
import { PrLink } from "./Cards";
import { Rag } from "./Rag";
import { Submissions } from "./Submissions";
import { changed, poke, sh, shMode, shPrepare, shSend, shSetMode, vt, vtGo, vtGood, vtKind, vtList, vtReportLine, vtReveal, vtShortName } from "./skills-data";

function Vault() {
  const st = vt.st, good = vtGood(st), line = good ? vtReportLine(good) : "", merged = vtList(good, "merged");
  const when = st ? [st.release ? "Release " + st.release : "", st.checked_at ? "checked " + ago(st.checked_at) : ""].filter(Boolean).join(" · ") : "";
  return (
    <Group id="set-vault" find="download guides roblox" title="Knowledge vault">
      <SettingsRow id="set-vault-get" title="Fridae's vault" descId="vt-line"
        desc={!st ? "" : st.downloaded ? (when ? when.charAt(0).toUpperCase() + when.slice(1) : "") : "Skills and Roblox guides from Fridae's vault, checked by Fridae's maintainer. The Roblox guides are not on this computer yet: press Download to get them. Until then, Fridae's searches have no Roblox guides."}
        ctl={<button class="pill" id="vt-go" hidden={!st} disabled={vt.getting} onClick={vtGo}>{st ? (st.downloaded ? "Pull newest" : "Download") : ""}</button>}
        after={<p class="sub sk-err" id="vt-err" role="status">{vt.err}</p>}>
        <p class="sr-status" id="vt-note" role="status">{vt.getting ? "Getting the vault…" : ""}</p>
        <p class="sr-status" id="vt-report" hidden={!line}>{line}</p>
        {/* each file combined with the user's edits, openable */}
        <More id="vt-merged" hidden={!merged.length} word="Show them"><div class="vt-kept" id="vt-kept">
          {merged.map((p) => <p class="sub"><span>{vtShortName(p)}</span><button class="pill sm" onClick={async () => { vt.err = await vtReveal(String(p), "mine"); poke(); }}>Reveal</button></p>)}
        </div></More>
      </SettingsRow>
    </Group>
  );
}
/* a batch to check: each skill or guide with its kind and its files (scripts first, each marked; a guide's one file by
   its full vault path), the text exactly as it would be sent, a mark on each file Fridae hid something in, and a box
   to leave it out; Send finishes, naming what is left out by unit */
function Batch({ batch }: { batch: any }) {
  const [out, setOut] = useState<Record<string, boolean>>({}), [sending, setSending] = useState(false);
  const skills = vtList(batch, "skills"), unit = (s: any) => String(s.unit || "Skills/" + s.name);
  return (
    <>
      {skills.map((s) => (
        <div class={"sh-skill" + (out[unit(s)] ? " out" : "")} data-unit={unit(s)}>
          <header><b>{String(s.name || "Untitled")}</b>{(s.kind === "guide" || s.kind === "vault skill") && <span class="sh-mark">{s.kind}</span>}<label class="check-row"><input type="checkbox" checked={!!out[unit(s)]} onChange={(e) => setOut({ ...out, [unit(s)]: (e.target as HTMLInputElement).checked })} /> Leave this one out</label></header>
          {s.description && <p class="sub">{s.description}</p>}
          <div class="sh-files">
            {vtList(s, "files").slice().sort((x, y) => (y.script ? 1 : 0) - (x.script ? 1 : 0)).map((f) => (
              <details class="moreinfo"><summary><span>{f.path}</span>{f.script && <span class="sh-mark">Script</span>}{f.masked && <span class="sh-mark">Fridae hid something here</span>}</summary><pre>{f.text || ""}</pre></details>
            ))}
          </div>
        </div>
      ))}
      <div class="act"><button class="pill main" disabled={sending} onClick={async () => { setSending(true); try { await shSend(skills.map(unit).filter((u) => out[u])); } finally { setSending(false); } }}>Send</button></div>
    </>
  );
}
function Share() {
  const st = vt.st, mode = shMode(st), on = mode !== "off", asked = !!(st && st.asked), live = !!st && on && !!st.connected;
  const k = st && st.knowledge && typeof st.knowledge === "object" ? st.knowledge : {};
  const kName = (s: any) => String(s.title || String(s.unit || "").split("/").pop() || "Untitled");
  const kKind = (s: any) => (s.kind === "vault skill" ? "vault skill" : s.kind === "guide" || vtKind(s.unit) === "guide" ? "guide" : vtKind(s.unit) === "skill" ? "vault skill" : "note");
  /* a row: the name, its kind after it (a guide or a vault skill; a skill of the library says none), then where it stands */
  const row = (name: string, tag: string, note: any, kind?: string) => <div class="sk-row"><b><span>{name}</span></b><em>{kind ? kind + " · " + tag : tag}</em><small>{note}</small></div>;
  const sent = (s: any) => { const at = Date.parse(s.at); return <><PrLink url={s.pr} />{isNaN(at) ? "" : " · " + ago(at / 1000)}</>; };
  const rows = live ? [
    ...vtList(st, "ready").map((s) => row(String(s.id), "Ready", s.scripts ? "Has scripts" : "")),
    ...vtList(k, "ready").map((s) => row(kName(s), "Ready", s.new ? "New" : "Changed", kKind(s))),
    ...vtList(st, "held").map((s) => row(String(s.id), "Held", s.why ? String(s.why) : "")),
    ...vtList(k, "held").map((s) => row(kName(s), "Held", s.why ? String(s.why) : "", kKind(s))),
    ...vtList(st, "sent").map((s) => row(String(s.id), "Sent", sent(s))),
    ...vtList(k, "sent").map((s) => row(kName(s), "Sent", sent(s), kKind(s))),
  ] : [];
  const modes = (labels: string[]) => ["off", "review", "auto"].map((m, i) => ({ value: m, label: labels[i], data: { "data-mode": m } }));
  return (
    <Group id="set-share" find="sharing community submit" title="Share your knowledge">
      <SettingsRow id="sh-ask" hidden={!st || asked} title="Share what you make" desc="Share skills you make and guides you change with everyone who uses Fridae?" ctlCls="act"
        ctl={modes(["Off", "Let me check each batch", "Send automatically"]).map((o) => <button class="pill" data-mode={o.value} disabled={sh.busy} onClick={() => shSetMode(o.value)}>{o.label}</button>)} />
      <SettingsRow cls="sh-modes" id="sh-modes" hidden={!st || !asked} title="Sharing" desc="Whether skills you make and guides you change are sent to Fridae's vault."
        ctl={<Segmented label="Sharing" options={modes(["Off", "Check each batch", "Automatic"])} value={st ? mode : null} disabled={sh.busy} onPick={shSetMode} />} />
      <div class="sr sr-notes">
        {/* the licence line shows while sharing is off, and again while an older agreement does not name guides (their
            rows are held until the user presses their sharing choice again) */}
        <p class="sub" id="sh-terms" hidden={!st || (on && st.agreed_knowledge !== false)}>Shared skills and guides are under CC BY 4.0 (text) and MIT (scripts), credited to your GitHub name. Fridae hides keys, emails and your computer's username first. Fridae's maintainer reads every one before anyone gets it.</p>
        <p class="sub" id="sh-gh" hidden={!st || !(sh.wantGh || (on && !st.connected))}>Connect GitHub first in <button class="linkish" id="sh-gh-go" onClick={() => openSetup("connections")}>Settings, Connections</button>.</p>
        <div class="sk-list sh-list" id="sh-list" hidden={!live}>{live && !rows.length ? <p class="sub">Nothing to share yet.</p> : rows}</div>
        <div class="act" id="sh-review" hidden={!(live && mode === "review")}><button class="pill" id="sh-prepare" disabled={sh.busy || (!!st && !((Number(st.shareable) || 0) + vtList(k, "ready").length > 0))} onClick={shPrepare}>Get a batch ready</button></div>
        <div id="sh-batch">{sh.batch && <Batch batch={sh.batch} />}</div>
        <p class="sub" id="sh-note" role="status" hidden={!sh.note}>{!sh.note ? "" : sh.note.nothing ? "Nothing to share yet." : <>Sent: <PrLink url={sh.note.sent.pr_url} />{sh.note.sent.warning ? " " + sh.note.sent.warning : ""}</>}</p>
        <p class="sub sk-err" id="sh-err" role="status">{sh.err}</p>
      </div>
    </Group>
  );
}
export function Knowledge() {
  changed.use();
  return <><Rag /><Vault /><Share /><Submissions /></>;
}
