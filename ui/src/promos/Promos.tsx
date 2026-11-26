/* The Promos tab (a Roblox game's): the new-promo form on the left, the promos being made under it, and the library
   on the right. `promos` is what the old page (ui/index.html) reaches it through, as fridae.promos. */
import { useEffect, useState } from "preact/hooks";
import { api, TOKEN } from "../api";
import { host } from "../host";
import { store, useStore, type Chat } from "../store";
import { isGame, baseName } from "../project/kind";
import { engineInstall, engineReady, engineKnown, loadEngine } from "./engine";
import { addPaths, chatKnown, form, lib, loadPromos, prChats, prNote, promoHere, promoLen, promoProj, promoWhen, setChats, setForm, styleName, upsert, type Card } from "./lib";

const X = <svg width="8" height="8" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M1.5 1.5l6 6M7.5 1.5l-6 6" /></svg>;
const onWindows = /Windows/.test(navigator.userAgent);   /* no dropping videos by path there: Add clips only */

function Chip({ path, remove }: { path: string; remove(): void }) {
  return <span class="pr-file" title={path}><span>{baseName(path)}</span><button aria-label={"Remove " + baseName(path)} onClick={remove}>{X}</button></span>;
}

/* the row at the top: the engine's state in a few words, and Install while it is missing */
function EngineRow() {
  const e = useStore((s) => s.engine);
  const short = e ? e.short : "Checking the promo engine...";
  return (
    <div class={"eng-row compact" + (e && e.ready ? " ready" : "")} id="pr-engine">
      <p class="sub eng-state" title={e ? e.title : undefined}>{short}</p>
      <div class="act"><button class="pill sm eng-install" hidden={!(e && e.button)} disabled={!!(e && e.running)} onClick={() => engineInstall()}>{(e && e.button) || "Install"}</button></div>
      <div class="eng-bar" hidden={!e || e.bar < 0}><i style={e && e.bar >= 0 ? "width: " + e.bar + "%" : undefined}></i></div>
      <pre class="eng-error">{e ? e.error : ""}</pre>
    </div>
  );
}

/* the styles, as tiles with a still of each: the agent's choice first */
let styles: { name: string; about: string; url: string }[] | null = null, stylesBusy = false, stylesNote = "";
async function loadStyles() {
  if (styles || stylesBusy || !engineReady()) return;
  stylesBusy = true; stylesNote = "Making the style previews. The first time takes a minute or two."; form.poke();
  try {
    const r = await api("/api/promo-styles", {});
    if (r.ok) { styles = r.styles || []; stylesNote = ""; }
    else stylesNote = "Could not make the style previews: " + (r.error || "unknown error") + " You can still let the agent choose.";
  } catch (e) { stylesNote = "Could not make the style previews."; }
  stylesBusy = false; form.poke();
}
function Styles({ style }: { style: string }) {
  const tiles: [string, string, string, string][] = [["", "Let the agent choose", "It picks the style that suits your clips.", ""]];
  for (const x of styles || []) tiles.push([x.name, x.name, x.about, x.url]);
  return (
    <div class="pr-styles" id="pr-styles">
      {tiles.map(([name, label, about, url]) => (
        <button class="pr-style" data-style={name} aria-pressed={name === style ? "true" : "false"} onClick={() => setForm({ style: name })}>
          {url ? <img class="still" loading="lazy" alt="" src={url + (url.includes("?") ? "&" : "?") + "t=" + TOKEN} /> : <span class="still">Agent's choice</span>}
          <b>{name ? styleName(label) : label}</b><small>{about}</small>
        </button>
      ))}
    </div>
  );
}

function Form() {
  const f = form.use(), cur = useStore((s) => s.nav.proj), projects = useStore((s) => s.projects);
  useStore((s) => s.engine);
  const [busy, setBusy] = useState(false);
  /* the tab belongs to a game: its promos are that game's, and there is nothing to choose */
  if (f.project === null || f.forProj !== cur) { f.project = cur || ""; f.forProj = cur; }
  if (f.project && !projects.some((x) => x.path === f.project)) f.project = "";
  if (cur) f.project = cur;
  const ready = engineReady(), brief = f.brief.trim();
  const why = !ready ? (engineKnown() ? "Install the promo engine first." : "Checking the promo engine...") : !f.clips.length ? "Add at least one clip." : !brief ? "Say what it should show." : "";
  /* what is set under More options shows on its line, so nothing chosen there is forgotten while it is folded */
  const set = [f.music ? "music" : "", f.mute ? "game sound muted" : "", f.reference ? "a TikTok's style" : "", f.voice.trim() ? "voiceover" : ""].filter(Boolean);
  const pick = async (kind: string) => { const r = await api("/api/pick-files", { kind }); return (r.paths || []) as string[]; };
  const make = async () => {
    setBusy(true);
    let r: any;
    try {
      r = await api("/api/promo-start", { clips: f.clips, music: f.music || null, reference: f.reference || null, style: f.style, length: f.length, voice_text: f.voice.trim(), brief, project: f.project || null, mute_game_sound: f.mute });
    } catch (e) { r = { ok: false, error: "Could not reach the app." }; }
    setBusy(false);
    if (!r.ok) { prNote(r.error || "Could not start the promo."); return; }
    setChats([...prChats.get(), { agent: r.agent, title: "Promo: " + brief.split(/\s+/).slice(0, 5).join(" "), at: Date.now() }]);
    setForm({ clips: [], music: "", reference: "", brief: "", voice: "", mute: false });
    prNote("Started in a chat of its own. It keeps working while you do other things.");
  };
  return (
    <div class={"pr-form" + (ready ? "" : " off")} id="pr-form">
      <div class="pr-field"><span class="pr-label">Clips</span>
        <div class="pr-files" id="pr-clips">{f.clips.map((c) => <Chip path={c} remove={() => setForm({ clips: form.get().clips.filter((x) => x !== c) })} />)}</div>
        <div class="act"><button class="pill" id="pr-add-clips" onClick={async () => addPaths(await pick("clips"))}>Add clips</button><span class="sub" id="pr-drop-hint" hidden={onWindows}>or drop video files here</span></div></div>
      <div class="pr-field"><label class="pr-label" for="pr-brief">What should it show?</label><textarea class="pr-text" id="pr-brief" rows={2} placeholder="e.g. the new boss fight and the shop" value={f.brief} onInput={(e) => setForm({ brief: (e.target as HTMLTextAreaElement).value })}></textarea></div>
      <div class="pr-field"><span class="pr-label">Style</span><Styles style={f.style} /><p class="sub" id="pr-styles-note">{stylesNote}</p></div>
      <div class="pr-field"><span class="pr-label">Length</span><div class="seg" id="pr-length">{[15, 30, 60].map((n) => <button class={n === f.length ? "on" : undefined} onClick={() => setForm({ length: n })}>{n + " s"}</button>)}</div></div>
      <details class="pr-more" id="pr-more">
        <summary><svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 1.5l3 3-3 3" /></svg>More options<span id="pr-more-n">{set.length ? " · " + set.join(", ") : ""}</span></summary>
        <div>
          <div class="pr-field"><span class="pr-label">Music</span>
            <div class="pr-files" id="pr-music">{f.music && <Chip path={f.music} remove={() => setForm({ music: "" })} />}</div>
            <div class="act"><button class="pill" id="pr-pick-music" onClick={async () => { const p = await pick("music"); if (p[0]) setForm({ music: p[0] }); }}>{f.music ? "Change music" : "Pick music"}</button><span class="sub" id="pr-no-music" hidden={!!f.music}>None. The game sound stays.</span></div>
            <label class="sub check-row"><input type="checkbox" id="pr-mute" checked={f.mute} onChange={(e) => setForm({ mute: (e.target as HTMLInputElement).checked })} /> Mute game sound (I'll add a TikTok sound)</label></div>
          <div class="pr-field"><span class="pr-label">In the style of a TikTok</span>
            <div class="pr-files" id="pr-ref">{f.reference && <Chip path={f.reference} remove={() => setForm({ reference: "" })} />}</div>
            <div class="act"><button class="pill" id="pr-pick-ref" title="TikTok links can't be used: save the video file first, then pick it here." onClick={async () => { const p = await pick("reference"); if (p[0]) setForm({ reference: p[0] }); }}>{f.reference ? "Change video" : "Pick saved video"}</button><span class="sub">Its pacing is copied, not its footage.</span></div></div>
          <div class="pr-field"><label class="pr-label" for="pr-voice">Voiceover script</label><textarea class="pr-text" id="pr-voice" rows={3} placeholder="Leave empty for no voice" title="The agent asks before making claims about your game." value={f.voice} onInput={(e) => setForm({ voice: (e.target as HTMLTextAreaElement).value })}></textarea></div>
        </div>
      </details>
      <div class="pr-field" id="pr-proj-field" hidden={!!cur}><label class="pr-label" for="pr-proj">Project</label>
        <select class="r-sel" id="pr-proj" value={f.project || ""} onChange={(e) => { form.get().project = (e.target as HTMLSelectElement).value; }}>
          <option value="">No project</option>
          {projects.filter((x) => isGame(x)).map((x) => <option value={x.path}>{x.name}</option>)}
        </select></div>
      <div class="pr-go"><button class="pill main" id="pr-make" disabled={!!why || busy} onClick={make}>Make promo</button><span class="r-note" id="pr-why">{Date.now() < f.msgUntil ? f.msg : why}</span></div>
    </div>
  );
}

/* a promo chat's state, from the same events the chat list shows */
function chatState(p?: Chat) {
  if (!p) return "closed";
  if (p.busy) return "working" + (p.now ? " · " + p.now : "");
  if (p.last === "Needs you") return "needs you";
  if (p.last === "Needs attention") return "failed";
  if (p.last === "Stopped") return "stopped";
  return p.started ? "done, no promo yet" : "starting";
}
function Progress() {
  const list = prChats.use(), cards = lib.use(), chats = useStore((s) => s.chats);
  useStore((s) => s.nav.proj); useStore((s) => s.saved);
  /* a chat leaves the list when its promo arrives */
  const madeBy = new Set([...cards.values()].map((c) => c.agent).filter(Boolean));
  useEffect(() => { if (list.some((x) => madeBy.has(x.agent))) setChats(list.filter((x) => !madeBy.has(x.agent))); });
  const here = list.filter((x) => !madeBy.has(x.agent) && promoHere(x.agent).show);
  return (
    <div id="pr-progress-wrap" hidden={!here.length}><h2>In progress</h2><div id="pr-progress">
      {here.slice().reverse().map((x) => {
        const p = chats.find((c) => c.id === x.agent);
        return (
          <div class="pr-row" key={x.agent + x.at}>
            <span class={"dot" + (p && p.busy ? " live" : p ? " ok" : "")}></span>
            <span class="grow"><b>{(p && p.name) || x.title}</b><small>{chatState(p) + (promoHere(x.agent).nogame ? " · no game" : "")}</small></span>
            <button class="pill sm" hidden={!chatKnown(x.agent)} onClick={() => host.openChatById(x.agent)}>Open chat</button>
            <button class="icon" aria-label="Remove from this list" title="Remove from this list" onClick={() => setChats(prChats.get().filter((y) => y !== x))}><svg width="10" height="10" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M1.5 1.5l6 6M7.5 1.5l-6 6" /></svg></button>
          </div>
        );
      })}
    </div></div>
  );
}

/* a finished promo: its video, its caption (saved as you type), the check, and what to do with it */
function PromoCard({ c, nogame }: { c: Card; nogame: boolean }) {
  const [entered, setEntered] = useState(false), [saved, setSaved] = useState(""), [saving, setSaving] = useState(false), [extras, setExtras] = useState(false);
  const [timer] = useState<{ t: number }>({ t: 0 });
  /* the file's mtime in the address: a promo rendered again into the same folder loads its new video */
  const id = encodeURIComponent(c.id || ""), src = (what: string) => "/media/promo/" + id + "/" + what + "?t=" + TOKEN + "&v=" + encodeURIComponent(c.mtime || c.created || "");
  const when = promoWhen(c), ck = c.check || {}, notes = (ck.notes || []).filter(Boolean);
  const check = (ck.passed ? "Checked: safe areas, hook, loudness" : "Problems:\n" + ((ck.problems || []).join("\n") || "the check did not run")) + (notes.length ? "\n" + notes.join("\n") : "");
  const caption = (text: string) => {
    setSaved(""); clearTimeout(timer.t);
    timer.t = setTimeout(async () => {
      const r = await api("/api/promo", { action: "caption", id: c.id, text });
      if (r.ok) c.caption_text = text;
      setSaved(r.ok ? "Caption saved" : "Could not save the caption.");
    }, 600) as unknown as number;
  };
  const download = async () => {
    setSaving(true);
    const r = await api("/api/promo", { action: "save", id: c.id, extras });
    setSaving(false);
    setSaved(r.ok ? "Saved to " + r.path : r.cancelled ? "" : r.error || "Could not save the promo.");
  };
  const reveal = async () => { const r = await api("/api/promo", { action: "reveal", id: c.id }); if (!r.ok) setSaved(r.error || "The video is no longer there."); };
  return (
    <div class={"ask promo" + (entered ? "" : " enter") + (nogame ? " nogame" : "")} data-id={c.id || ""} onAnimationEnd={(e) => { if (e.animationName === "rise") setEntered(true); }}>
      <h4>{c.folder ? baseName(c.folder) : c.id || "Promo"}</h4>
      <video controls playsInline preload="metadata" src={src("video")} poster={src("cover")}></video>
      <div class="meta">{[styleName(c.style), promoLen(c.duration), when ? new Date(when).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : ""].filter(Boolean).join("  ·  ")}</div>
      <textarea aria-label="Post caption" spellcheck={true} defaultValue={c.caption_text || ""} onInput={(e) => caption((e.target as HTMLTextAreaElement).value)}></textarea>
      <div class="saved">{saved}</div>
      <div class={"check" + (ck.passed ? "" : " bad")}>{check}</div>
      <div class="act">
        <button class="pill main" disabled={saving} onClick={download}>Download</button>
        <button class="pill" onClick={reveal}>Show in folder</button>
        <button class="pill" hidden={!chatKnown(c.agent)} onClick={() => c.agent && host.openChatById(c.agent)}>Open chat</button>
        <label><input type="checkbox" checked={extras} onChange={(e) => setExtras((e.target as HTMLInputElement).checked)} /> Include cover and caption</label>
      </div>
      <div class="promo-post"></div>
    </div>
  );
}
function Library() {
  const cards = lib.use();
  useStore((s) => s.nav.proj); useStore((s) => s.chats); useStore((s) => s.saved);
  const items = [...cards.values()].filter((c) => promoHere(c.agent).show);
  items.sort((a, b) => promoWhen(b) - promoWhen(a) || (b.mtime || 0) - (a.mtime || 0));
  const day = (t: number) => { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); };
  const today = day(Date.now()), groups: [string, Card[]][] = [];
  for (const c of items) {
    const t = promoWhen(c), d = t ? day(t) : 0;
    const label = !d ? "Earlier" : d === today ? "Today" : Math.round((today - d) / 86400000) === 1 ? "Yesterday"
      : new Date(d).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short", year: new Date(d).getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
    if (!groups.length || groups[groups.length - 1][0] !== label) groups.push([label, []]);
    groups[groups.length - 1][1].push(c);
  }
  return (
    <section class="pr-lib" aria-label="Library">
      <h2>Library <small id="pr-count">{items.length ? items.length + (items.length === 1 ? " promo" : " promos") : ""}</small></h2>
      <p class="r-note" id="pr-empty" hidden={items.length > 0}>Promos you make appear here, newest first.</p>
      <div id="pr-library">
        {groups.map(([label, xs]) => [<div class="pr-day" key={"d:" + label}>{label}</div>,
          <div class="pr-grid" key={"g:" + label}>{xs.map((c) => <PromoCard key={c.id + "@" + (c.mtime || 0)} c={c} nogame={promoHere(c.agent).nogame} />)}</div>])}
      </div>
    </section>
  );
}

export function Promos() {
  return (
    <div class="r-col pr-page">
      <div class="pr-cols"><section class="pr-make" aria-label="New promo">
        <h2>New promo</h2>
        <EngineRow />
        <Form />
        <Progress />
      </section><Library /></div>
    </div>
  );
}

export const promos = {
  /* the tab came to the front */
  async open() { form.poke(); await Promise.all([loadEngine(), host.loadProjects()]); form.poke(); loadStyles(); },
  load: async () => { await loadPromos(); },
  add: upsert,              /* a finished promo arrived: true when it is new or newer than the one shown */
  addPaths, note: prNote,   /* videos the app was handed while Promos is in front; a passing word beside Make promo */
  /* "Open in Promos" from a chat's line: the promo's game comes to the front, and the promo is shown and marked */
  show(id: string, agent?: string) {
    const k = promoProj(agent || (lib.get().get(id) || {} as Card).agent), cur = store.get().nav.proj;
    if (k && k !== cur) host.showProject(k, "promos"); else host.setView("promos");
    setTimeout(() => {
      const card = [...document.querySelectorAll<HTMLElement>("#pr-library .promo")].find((x) => x.dataset.id === id); if (!card) return;
      card.scrollIntoView({ block: "center" });
      card.classList.add("flash"); setTimeout(() => card.classList.remove("flash"), 1800);
    }, 0);
  },
};
