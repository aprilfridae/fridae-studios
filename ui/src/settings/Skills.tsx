/* Settings, Skills: import and tidy up, the decisions waiting for you (two skills or notes that cover the same thing,
   and vault files that clash with yours), and your library. */
import { useState } from "preact/hooks";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { More } from "../ui/More";
import { Pin } from "../ui/icons";
import { ClashCard, DecisionCard } from "./Cards";
import { SK_APPLIES, SK_OWNER, VT_CLASH_FIRST, changed, sk, skAcceptAll, skCheckNotes, skImport, skOpen, skPinned, vt, vtShowAllClashes } from "./skills-data";

/* Your skills folds away (it is long); the choice is remembered */
function Library() {
  const [open, setOpen] = useState(() => { try { return localStorage.getItem("skills-shown") === "1"; } catch (e) { return false; } });
  const [filter, setFilter] = useState("");
  const fold = () => { try { localStorage.setItem("skills-shown", open ? "0" : "1"); } catch (e) {} setOpen(!open); };
  const all: any[] = sk.data ? (sk.data.skills || []).slice().sort((x: any, y: any) => (+skPinned(y) - +skPinned(x)) || (x.name || x.id || "").localeCompare(y.name || y.id || "")) : [];
  const words = filter.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hay = (s: any) => [s.name, s.id, s.description, SK_OWNER[s.owner], s.owner, SK_APPLIES[s.applies], skPinned(s) ? "pinned" : "", (s.examples || []).join(" ")].join(" ").toLowerCase();
  const list = words.length ? all.filter((s) => { const h = hay(s); return words.every((w) => h.includes(w)); }) : all;
  return (
    <Group id="set-your-skills" find="library pinned" titleCls="sk-h" title={<>Your skills <span id="sk-n">{sk.data ? (list.length === all.length ? String(all.length) : list.length + " of " + all.length) : ""}</span></>}
      act={<button class="pill sm" id="sk-fold" aria-expanded={String(open)} aria-controls="sk-body" onClick={fold}>{open ? "Hide" : "Show"}</button>} boxId="sk-body" boxHidden={!open}>
      <div class="sr"><div class="keyrow"><input type="text" id="sk-filter" value={filter} onInput={(e) => setFilter((e.target as HTMLInputElement).value)} placeholder="Filter: a name, a word, yours, pinned, roblox" autocomplete="off" spellcheck={false} aria-label="Filter your skills" /></div></div>
      <div class="sk-list sg-list" id="sk-list">
        {!sk.data ? <p class="sub">Loading...</p>
          : !list.length ? <p class="sub">{all.length ? "No skill matches “" + words.join(" ") + "”." : "No skills yet."}</p>
          : list.map((s) => <div class="sk-row"><b><span>{s.name || s.id}</span>{skPinned(s) && Pin}</b><em>{[SK_OWNER[s.owner] || s.owner, SK_APPLIES[s.applies] || ""].filter(Boolean).join(" · ")}</em><small title={s.description || undefined}>{s.description || ""}</small></div>)}
      </div>
    </Group>
  );
}
/* Needs your decision holds the decision cards and the clash cards: shown when either has a card, counting both.
   The open cards first (the first 20 clashes unless Show all was pressed), then the ones just decided, one line each. */
function Decisions() {
  const open = skOpen(), clashes = vt.clashes, shownClashes = vt.clashAll ? clashes : clashes.slice(0, VT_CLASH_FIRST);
  const cards = open.length + sk.done.size, clashCards = shownClashes.length + vt.clashDone.size, n = open.length + clashes.length;
  return (
    <section class="st-sec" id="sk-dec-wrap" hidden={!(cards || clashCards)}>
      <div class="st-sec-head"><h2 class="sk-h">Needs your decision <span id="sk-dec-n">{n ? String(n) : ""}</span></h2><div class="st-sec-act">
        <button class="pill sm" id="sk-all" hidden={open.length < 2} disabled={sk.busy} title={open.some((d) => d.recommendation && d.recommendation.choice) ? "Take every suggestion; those without one stay here" : "No suggestions yet"} onClick={skAcceptAll}>Accept all recommendations</button></div></div>
      <p class="sub sk-err" id="sk-all-err" role="status">{sk.allErr}</p>
      <div class="rv-list sk-decs" id="sk-decs">
        {open.map((d) => <DecisionCard key={d.id} d={d} />)}
        {[...sk.done].map(([id, x]) => <DecisionCard key={id} d={x.d} said={x.said} />)}
      </div>
      <div class="rv-list sk-decs" id="vt-clashes">
        {shownClashes.map((c) => <ClashCard key={String(c.unit)} c={c} />)}
        {[...vt.clashDone].map(([unit, x]) => <ClashCard key={unit} c={x.c} said={x.said} />)}
      </div>
      <div class="act" id="vt-clash-more" hidden={vt.clashAll || clashes.length <= VT_CLASH_FIRST}><button class="pill sm" id="vt-clash-all" onClick={vtShowAllClashes}>{vt.clashesRead ? "Show all " + clashes.length : ""}</button></div>
      <p class="sub sk-err" id="vt-clash-err" role="status">{vt.clashErr}</p>
    </section>
  );
}
export function Skills() {
  changed.use();
  return (
    <>
      <div class="st-lede"><p>One library of skills that every AI here uses: Fridae's own, the ones it ships, and the ones found in your other AI tools.</p>
        <More><p>Fridae looks in Claude Code, its plugins, and the skill folders of Codex, Cursor, OpenCode and Gemini now and then (Import now does it at once), and brings in what is new. When a newcomer covers much the same as a skill you already have, it waits here for you, and the one in use keeps working meanwhile. A cheap AI reads both and suggests what to do. Nothing is deleted: the one you don't keep is set aside in the library's .replaced folder. Pinned skills are in every AI's instructions; the rest are found by search when a chat needs them.</p><p>Your notes get the same care: once a week, while no chat is working (Check notes now does it at once), Fridae looks for notes that say the same thing twice or disagree, and asks you here which one holds. The note you don't keep goes to your notes folder's .trash.</p></More></div>
      <Group id="set-import" title="Import and tidy up">
        <SettingsRow id="set-sk-import" title="Import skills" desc="Looks in your other AI tools and brings in what is new."
          ctl={<button class="pill" id="sk-import" disabled={sk.importing} onClick={skImport}>Import now</button>}>
          <p class="sr-status" id="sk-import-note" role="status">{sk.importNote}</p>
        </SettingsRow>
        <SettingsRow id="set-sk-notes" title="Check your notes" desc="Looks for notes that say the same thing twice or disagree."
          ctl={<button class="pill" id="sk-notes" disabled={sk.checking} onClick={skCheckNotes}>Check notes now</button>} after={<p class="sub sk-err" id="sk-err" role="status">{sk.err}</p>}>
          <p class="sr-status" id="sk-notes-note" role="status">{sk.notesNote}</p>
        </SettingsRow>
      </Group>
      <Decisions />
      <Library />
    </>
  );
}
