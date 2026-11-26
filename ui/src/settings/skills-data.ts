/* What the Skills and Knowledge pages of Settings know, and every press on them. The two pages are one thing: the
   library and its decisions, Fridae's vault, sharing, and (for the vault's maintainer) the submissions.

   GET /api/skills gives the library's index and the open decisions. A decision is about two skills (kind "skill") or
   two of the user's notes (kind "note"): a is the one in use, b the newcomer waiting beside it, with their folders,
   why it was asked (reason: "update" when the user's edited copy's source changed, else an overlap), the newcomer's
   source folder, and a cheap AI's recommendation once written ({choice: keep|replace|merge|null, why, merged, by}).
   A note decision has the two notes' titles and files (a the older), relation ("duplicate" | "conflict") and lines
   ({a, b}: the sentences that differ, or each note's first). Every change is the user's click: POST {action: decide |
   accept_all | import | check_notes}. Replace and Merge are never offered on one of Fridae's own skills (owner app;
   the app refuses them too), and Merge only with the merged text.

   GET /api/vault/status: the vault's {downloaded, release, checked_at, running, last}, where last is the newest
   Download or Pull newest: the vault-apply report ({added, updated, merged, clashes, incoming, errors}; vault paths
   "Skills/<skill>/<file>" for skills and "Roblox/..." for guides; clashes the units now waiting for Keep the vault's /
   Keep mine, a skill as "Skills/<skill>"; incoming the new skills waiting for a decision), {release: null, note},
   {current: true} or {ok: false, error}; merged with Share's {mode: off|review|auto, asked, agreed, agreed_knowledge,
   connected, login, shareable, ready [{id, scripts}], held [{id, why}], sent [{id, pr, at}], knowledge {ready [{unit,
   kind, title, new}], held [{unit, title, why}], sent [{unit, title, pr, at}]}, batch?, last_sent, held_masked} and
   role {connected, login, maintainer, repo_exists}. It runs the skills tool, so it is read when Skills opens and after
   a click, and every few seconds only while an update runs.
   GET /api/vault/clashes: {ok, clashes: [{unit, kind: file|binary|skill, title, release, made, both_new, files,
   mine {path, changed, size, modified}, vault {changed, size}, show {mine, vault}}]}. Every POST is the user's click:
   download, check, clash {unit, choice: vault|mine}, share_mode {mode, agree}, prepare, send {batch, drop: units},
   reveal {path, which: mine|vault}.
   GET /api/vault/inbox: {prs: [{n, url, author, title, error?, not_skill, skills: [{unit, name, files: [{path, text,
   script, note?}], not_skill, same_name, closest {id, score}|null, diff, recommendation|null, decided|null}], notes:
   [{unit, path, title, text, note?, binary, in_vault, diff|null, decided|null}]}]}. Everything in a submission is a
   stranger's: it reaches the page only as text, and its pull request opens through the app by number, never by an
   address. Every POST there is the maintainer's click: inbox_decide {pr, unit, choice: add | edit | reject, text?,
   why?}; publish and create_repo only from their confirm card's button, with confirm: true. */
import { api } from "../api";
import { cell } from "../store";
import { host } from "../host";
import { post } from "../util";
import { shown, whenOpened, whenShown } from "./state";

/* everything below is drawn again when this is poked */
export const changed = cell(0);
const paint = () => changed.poke();
/* the count of open decisions: Settings' Skills button and the Overview's totals line */
export const skillsWaiting = cell(0);
const visible = () => shown("skills", "knowledge");

/* ---------- the library and its decisions ---------- */
export const sk = {
  data: null as any, err: "", busy: false,            /* busy: a decide or Accept all is on its way; every other decision control waits */
  importNote: "", notesNote: "", allErr: "", importing: false, checking: false,
  done: new Map<string, { said: string; d: any }>(),   /* what was decided, folded to one line until Skills is opened again */
  cardErr: new Map<string, string>(),
};
let skLoading: Promise<void> | null = null, skAgain = false, skAt = 0, skPollT = 0, skPollUntil = 0;
export const SK_OWNER: Record<string, string> = { app: "Fridae's own", shipped: "Shipped with Fridae", community: "Community", user: "Yours" };
export const SK_APPLIES: Record<string, string> = { game: "Roblox games", plain: "Projects", both: "Any project" };
const SK_DONE: Record<string, string> = { keep: "Kept", replace: "Using the new one", merge: "Merged", keep_both: "Kept both" };
const SK_DONE_NOTE: Record<string, string> = { keep: "Kept the first", replace: "Kept the second", merge: "Merged", keep_both: "Kept both" };
export const skOpen = (): any[] => ((sk.data && sk.data.pending) || []).filter((d: any) => !sk.done.has(d.id));
export const skPinned = (s: any) => !!(s.pinned || s.core);
/* where a newcomer came from, told by its source folder (the folders rbxskills._roots looks in) */
export function skFrom(path: string) {
  const p = (path || "").replace(/\\/g, "/");
  if (/\/\.claude\/plugins\/synced\//.test(p)) return "a plugin synced from Claude";
  if (/\/\.claude\/skills\/synced\//.test(p)) return "synced from Claude";
  if (/\/\.claude\/plugins\//.test(p)) return "a Claude Code plugin";
  if (/\/\.claude\/skills\//.test(p)) return "Claude Code";
  if (/\/\.codex\//.test(p)) return "Codex";
  if (/\/\.cursor\//.test(p)) return "Cursor";
  if (/\/opencode\//.test(p)) return "OpenCode";
  if (/\/\.gemini\//.test(p)) return "Gemini";
  if (/\/\.agents\//.test(p)) return "another AI tool";
  return "";
}
export const skHome = (p: string) => (p || "").replace(/^\/(Users|home)\/[^/]+\//, "~/");
/* the library's entry for a side: the one in use by its folder or its name; the newcomer only by its folder (an update
   has the same name as the copy in use) */
function skEntry(name: string, dir: string, byName: boolean) {
  const list: any[] = (sk.data && sk.data.skills) || [];
  return list.find((s) => dir && s.dir === dir) || (byName && name ? list.find((s) => (s.id || "").toLowerCase() === name.toLowerCase()) : null) || null;
}
/* what a side of a skill decision shows: read from its own folder by the skills tool (a_meta / b_meta), else the
   library's entry for it, else nothing more than its name */
export function skSideInfo(d: any, side: "a" | "b") {
  const e = side === "a" ? skEntry(d.a, d.a_dir, true) : skEntry(d.b, d.b_dir, false);
  return Object.assign({ owner: "", description: "", licence: "", source: "", scripts: false, origin: "" }, e || {}, d[side + "_meta"] || {});
}
/* the skill in use once a choice is made: the newcomer after a replace, else the one in use (a merge keeps its name) */
const skNow = (d: any, choice: string) => (choice === "replace" ? d.b : d.a);
export function skLabels(d: any, a: any): Record<string, string> {
  if (d.kind === "note") return { keep: "Keep the first", replace: "Keep the second", merge: "Merge them", keep_both: "Keep both" };
  const owner = a && a.owner;
  return { keep: owner === "app" || owner === "shipped" ? "Keep Fridae's" : owner === "community" ? "Keep the one in use" : "Keep yours", replace: "Use the new one", merge: "Merge them", keep_both: "Keep both" };
}
export function skHeadline(d: any) {
  if (d.kind === "note") return d.relation === "conflict" ? "Two notes disagree" : "Two notes say much the same";
  if (d.reason === "update") return "“" + d.b + "” has a new version, and you changed your copy";
  return (d.a || "").toLowerCase() === (d.b || "").toLowerCase() ? "Two skills called “" + d.a + "”" : "“" + d.b + "” covers much the same as “" + d.a + "”";
}
function skFold(d: any, said: string) { sk.done.set(d.id, { said, d }); }
/* after anything changed: the counts, and a suggestion still being written is looked for again in a while, as long as
   Skills is open */
function skPainted() {
  const n = skOpen().length;
  if (skillsWaiting.get() !== n) skillsWaiting.set(n);
  host.skillsCount();
  clearTimeout(skPollT);
  if (skOpen().some((d) => !d.recommendation) && Date.now() < skPollUntil) skPollT = setTimeout(() => { if (visible()) loadSkills(); }, 15000) as unknown as number;
  paint();
}
export async function skDecide(d: any, choice: string) {
  if (sk.busy) return;
  const said = (d.kind === "note" ? SK_DONE_NOTE : SK_DONE)[choice] + ": “" + skNow(d, choice) + "”";
  sk.busy = true; sk.cardErr.delete(d.id); paint();
  const body: any = { action: "decide", id: d.id, choice };
  if (choice === "merge") body.merged = d.recommendation.merged;
  const r = await post("/api/skills", body);
  sk.busy = false;
  if (!r || !r.ok) {
    /* a refused merge (a side changed since the suggestion) clears the suggestion: show the card as it is now, so its
       old merge is not offered again, and keep the reason on it */
    const msg = (r && r.error) || "That did not work.";
    sk.cardErr.set(d.id, msg); paint();
    if (choice === "merge") { await loadSkills(); sk.cardErr.set(d.id, msg); paint(); }
    return;
  }
  skFold(d, said); skPainted(); loadSkills();
}
export async function skAcceptAll() {
  if (sk.busy) return;
  const open = skOpen();
  sk.busy = true; sk.allErr = ""; paint();
  const r = await post("/api/skills", { action: "accept_all" });
  sk.busy = false;
  if (!r || !r.ok) { sk.allErr = (r && r.error) || "Could not accept them."; paint(); return; }
  const byId = new Map(open.map((d) => [d.id, d]));
  (r.done || []).forEach((id: string) => {
    const d = byId.get(id), c = d && d.recommendation && d.recommendation.choice;
    if (d) skFold(d, "Suggestion taken: " + (skLabels(d, d.kind === "note" ? null : skSideInfo(d, "a"))[c] || c) + " (now “" + skNow(d, c) + "”)");
  });
  const failed = (r.failed || []).map((f: any) => { const d = byId.get(f.id); return (d ? "“" + d.a + "”" : f.id) + ": " + (f.error || "It could not be settled."); });
  if (failed.length) sk.allErr = (failed.length === 1 ? "One was not settled:" : failed.length + " were not settled:") + "\n" + failed.join("\n");
  else if (!(r.done || []).length) sk.allErr = "There are no suggestions to take yet.";
  skPainted(); loadSkills();
}
export async function skImport() {
  sk.importing = true; sk.importNote = "Looking in your other AI tools..."; sk.err = ""; paint();
  const r = await post("/api/skills", { action: "import" });
  sk.importing = false;
  if (!r || !r.ok) { sk.importNote = ""; sk.err = (r && r.error) || "The import could not run."; paint(); return; }
  const f = r.import || {}, n = (k: string) => (Array.isArray(f[k]) ? f[k].length : 0);
  const parts = [n("added") ? n("added") + " new" : "", n("refreshed") ? n("refreshed") + " updated" : "", n("pending") ? n("pending") + (n("pending") === 1 ? " waits for your decision" : " wait for your decision") : ""].filter(Boolean);
  sk.importNote = parts.length ? parts.join(", ") + "." : "Nothing new: your library is up to date.";
  paint(); loadSkills();
}
export async function skCheckNotes() {
  sk.checking = true; sk.notesNote = "Reading your notes..."; sk.err = ""; paint();
  const r = await post("/api/skills", { action: "check_notes" });
  sk.checking = false;
  if (!r || !r.ok) { sk.notesNote = ""; sk.err = (r && r.error) || "Your notes could not be checked."; paint(); return; }
  const added = (r.notes && r.notes.added) || 0, open = (r.notes && r.notes.open) || 0;
  sk.notesNote = added ? added + (added === 1 ? " new question about your notes, below." : " new questions about your notes, below.")
    : open ? "Nothing new. " + open + (open === 1 ? " question about your notes still waits below." : " questions about your notes still wait below.")
    : "Your notes look fine: nothing says the same twice or disagrees.";
  skPollUntil = Date.now() + 180000;
  paint(); loadSkills();
}
/* fresh: Skills was just opened, so the lines of what was decided before go, and suggestions are looked for again */
export async function loadSkills(fresh?: boolean) {
  if (fresh) { sk.done.clear(); skPollUntil = Date.now() + 180000; sk.importNote = ""; sk.notesNote = ""; sk.allErr = ""; sh.note = null; sb.fresh = true; vt.clashDone.clear(); vt.clashAll = false; vtLoad(); }
  if (skLoading) { skAgain = true; return skLoading; }
  skLoading = (async () => {
    do {
      skAgain = false; skAt = Date.now();
      let r; try { r = await api("/api/skills"); } catch (e) { r = null; }
      if (r && Array.isArray(r.skills) && Array.isArray(r.pending)) sk.data = r;
      sk.err = !r ? "Could not reach the app." : r.error || "";
      skPainted();
    } while (skAgain);
  })();
  try { await skLoading; } finally { skLoading = null; }
}
/* the Overview and Settings ask for the count now and then (it runs the skills tool, so not on every repaint) */
export function skCount() { if (!skLoading && Date.now() - skAt > 300000) loadSkills(); }
whenShown(["skills", "knowledge"], () => loadSkills(true));
whenOpened((page, asked) => { if (page !== "skills" && page !== "knowledge") skCount(); else if (!asked) loadSkills(true); });

/* ---------- Fridae's vault, and the clashes waiting for Keep the vault's / Keep mine ---------- */
export const VT_CLASH_FIRST = 20;   /* clash cards shown before "Show all N" */
export const vt = {
  st: null as any, err: "", going: false, getting: false,
  clashes: [] as any[], clashesRead: false, clashAll: false, clashBusy: false, clashErr: "",
  clashDone: new Map<string, { said: string; c: any }>(), clashCardErr: new Map<string, string>(),
};
let vtT = 0, vtLoading = false, vtAgain = false, vtWasRunning = false;
export const vtSkill = (p: any) => String(p || "").split("/")[1] || "";
export const vtList = (o: any, k: string): any[] => (o && Array.isArray(o[k]) ? o[k] : []);
/* what a vault path is: a guide (Roblox/), a skill (Skills/) or another note */
export const vtKind = (p: any) => (/^Roblox\//.test(String(p)) ? "guide" : /^Skills\//.test(String(p)) ? "skill" : "note");
export const vtCount = (n: number, one: string, many: string) => n + " " + (n === 1 ? one : many);
export function vtSize(n: any) {
  n = Number(n) || 0;
  if (n < 1000) return n + (n === 1 ? " byte" : " bytes");
  if (n < 1e6) return (n / 1e3).toFixed(n < 1e4 ? 1 : 0) + " KB";
  return (n / 1e6).toFixed(n < 1e7 ? 1 : 0) + " MB";
}
/* open a file where it lives; what went wrong is handed back */
export async function vtReveal(path: string, which: string) {
  const r = await post("/api/vault/reveal", { path, which });
  return r && r.ok ? "" : (r && r.error) || "That file is not there any more.";
}
export async function vtLoad() {
  clearTimeout(vtT);
  if (vtLoading) { vtAgain = true; return; }
  vtLoading = true;
  let r;
  do {
    vtAgain = false;
    try { r = await api("/api/vault/status"); } catch (e) { r = null; }
  } while (vtAgain);
  vtLoading = false;
  if (!r || typeof r !== "object") { vt.err = "Could not reach the app."; paint(); return; }
  vtSet(r);
  /* Submissions: the inbox asks GitHub, so it is read once each time Skills opens, and only for the maintainer */
  if (sb.fresh && sbInboxShown(r.role) && visible()) { sb.fresh = false; sbLoad(); }
  /* an update that just ended may have added skills, decisions or clashes: they are read again */
  if (vtWasRunning && !r.running) loadSkills();
  if (!r.running) vtLoadClashes();
  vtWasRunning = !!r.running;
  if (r.running) vtT = setTimeout(() => { if (visible()) vtLoad(); }, 2500) as unknown as number;
}
/* a status to show (made-up data shows as it would) */
export function vtSet(st: any) {
  vt.st = st = st && typeof st === "object" ? st : {};
  const last = st.last && typeof st.last === "object" ? st.last : null;
  const good = vtGood(st), failed = vtList(good, "errors").length;
  vt.going = false; vt.getting = !!st.running;
  vt.err = last && last.ok === false ? last.error || "The vault could not be updated."
    : failed ? "Fridae could not put " + failed + (failed === 1 ? " file" : " files") + " in place. The next Pull newest tries again." : "";
  if (st.connected) sh.wantGh = false;
  const mode = shMode(st);
  if (!(mode !== "off" && st.connected && mode === "review")) sh.batch = null;
  const role = sbRole();
  if (!sbInboxShown(role)) { sb.inbox = null; sb.pubConfirm = false; }
  if (!(sbOwner(role) && !role.repo_exists)) sb.createConfirm = false;
  paint();
}
/* the last Download or Pull newest, when it brought something */
export function vtGood(st: any) { const last = st && st.last && typeof st.last === "object" ? st.last : null; return last && last.ok !== false && !last.current && !last.skipped ? last : null; }
/* the last report in one line: new skills (those waiting for a decision among them) and guides, what was updated, then
   a sentence for each kind combined with the user's edits, and the clashes waiting below */
export function vtReportLine(last: any) {
  if (last.note) return last.note;
  const added = vtList(last, "added"), merged = vtList(last, "merged");
  const fresh = new Set(added.filter((p) => /^Skills\/[^/]+\/SKILL\.md$/.test(p)).map(vtSkill));
  const unit = (p: any) => (vtKind(p) === "skill" ? "Skills/" + vtSkill(p) : String(p));
  const newGuides = added.filter((p) => vtKind(p) === "guide").length, newNotes = added.filter((p) => vtKind(p) === "note").length;
  const mergedUnits = new Set(merged.map(unit));
  const touched = new Set([...added.filter((p) => vtKind(p) === "skill"), ...vtList(last, "updated")].map(unit)
    .filter((u) => u !== "Skills/" && !(vtKind(u) === "skill" && fresh.has(vtSkill(u))) && !mergedUnits.has(u)));
  const waiting = vtList(last, "incoming").length, skills = fresh.size + waiting, bits = [];
  if (skills) bits.push(vtCount(skills, "new skill", "new skills") + (waiting ? " (" + waiting + (waiting === 1 ? " waits" : " wait") + " for your decision below)" : ""));
  if (newGuides) bits.push(vtCount(newGuides, "new guide", "new guides"));
  if (newNotes) bits.push(vtCount(newNotes, "new note", "new notes"));
  if (touched.size) bits.push(touched.size + " updated");
  const out = bits.length ? [bits.join(", ") + "."] : [];
  const edited = (kind: string, one: string, many: string) => {
    const n = [...mergedUnits].filter((u) => vtKind(u) === kind).length;
    if (n) out.push("Updated " + vtCount(n, one, many) + " you had edited.");
  };
  edited("guide", "guide", "guides"); edited("skill", "skill", "skills"); edited("note", "note", "notes");
  const clashes = vtList(last, "clashes").length;
  if (clashes) out.push(clashes + (clashes === 1 ? " clash waits" : " clashes wait") + " for your choice below.");
  return out.join(" ");
}
/* a merged file in the report, named the way the user knows it: a skill's name (and its file), a guide by its path */
export function vtShortName(p: any) {
  const parts = String(p).split("/");
  if (parts[0] === "Skills" && parts.length >= 3) return parts[2] === "SKILL.md" && parts.length === 3 ? parts[1] : parts.slice(1).join("/");
  return parts.length > 1 ? parts.slice(1).join("/") : String(p);
}
/* Download, or Pull newest */
export async function vtGo() {
  const first = !(vt.st && vt.st.downloaded);
  vt.going = true; vt.getting = true; vt.err = ""; paint();
  const r = await post(first ? "/api/vault/download" : "/api/vault/check", {});
  if (!r || !r.ok) {
    vt.going = false; vt.getting = false; vt.err = (r && r.error) || "The vault could not be updated."; paint();
    if (!(r && r.busy)) return;
  }
  vtWasRunning = true;
  vtLoad();
}
async function vtLoadClashes() {
  let r; try { r = await api("/api/vault/clashes"); } catch (e) { r = null; }
  if (!r || r.ok === false || !Array.isArray(r.clashes)) { vt.clashErr = (r && r.error) || "Fridae could not read the clashes waiting for your choice."; paint(); return; }
  vt.clashErr = "";
  vtClashSet(r.clashes);
}
export function vtClashSet(list: any) {
  vt.clashesRead = true;
  vt.clashes = (Array.isArray(list) ? list : []).filter((c) => c && typeof c === "object" && c.unit && !vt.clashDone.has(String(c.unit)));
  paint();
}
/* the words for what each side changed: one of four plain sentences */
export function vtClashWords(c: any) {
  const m = c.mine || {}, v = c.vault || {};
  if (c.both_new) return "The vault has a new file with the same name as yours.";
  if (m.changed == null || v.changed == null) return "Pictures and models are never combined. Open each to compare them.";
  const lines = (n: any) => vtCount(Number(n) || 0, "line", "lines");
  if (c.kind === "skill") return "The vault changed " + lines(v.changed) + " in " + vtCount(vtList(c, "files").length, "file", "files") + " of this skill; you changed " + (Number(m.changed) || 0) + ".";
  return "The vault changed " + lines(v.changed) + " since you got it; you changed " + (Number(m.changed) || 0) + ".";
}
export async function vtClashDecide(c: any, choice: string, title: string) {
  if (vt.clashBusy) return;
  const unit = String(c.unit);
  vt.clashBusy = true; vt.clashCardErr.delete(unit); paint();
  const r = await post("/api/vault/clash", { unit: c.unit, choice });
  vt.clashBusy = false;
  if (!r || !r.ok) { vt.clashCardErr.set(unit, (r && r.error) || "That did not work."); paint(); return; }
  vt.clashDone.set(unit, { said: (choice === "vault" ? "Kept the vault's “" : "Kept yours of “") + title + "”", c });
  vtClashSet(vt.clashes.filter((x) => x.unit !== c.unit));
  vtLoadClashes();
}
export function vtShowAllClashes() { vt.clashAll = true; paint(); }

/* ---------- Share your knowledge ---------- */
export const sh = { batch: null as any, busy: false, wantGh: false, err: "", note: null as null | { nothing?: boolean; sent?: any } };
export const shMode = (st: any) => (st && (st.mode === "review" || st.mode === "auto") ? st.mode : "off");
export async function shSetMode(mode: string) {
  if (sh.busy) return;
  sh.err = "";
  if (mode !== "off" && vt.st && !vt.st.connected) { sh.wantGh = true; paint(); return; }
  sh.wantGh = false; sh.busy = true; paint();
  const r = await post("/api/vault/share_mode", { mode, agree: mode !== "off" });
  sh.busy = false;
  if (!r || !r.ok) sh.err = (r && r.error) || "That did not work.";
  paint(); vtLoad();
}
export async function shPrepare() {
  if (sh.busy) return;
  sh.busy = true; sh.err = ""; sh.note = null; paint();
  const r = await post("/api/vault/prepare", {});
  sh.busy = false;
  if (!r || !r.ok) { sh.err = (r && r.error) || "Fridae could not get a batch ready."; paint(); return; }
  shBatchSet(r);
}
/* a batch to check (made-up data shows as it would) */
export function shBatchSet(made: any) {
  sh.batch = made && made.batch && vtList(made, "skills").length ? made : null;
  sh.note = sh.batch ? null : { nothing: true };
  paint();
}
export async function shSend(drop: string[]) {
  if (sh.busy || !sh.batch) return;
  sh.busy = true; sh.err = ""; paint();
  const r = await post("/api/vault/send", { batch: sh.batch.batch, drop });
  sh.busy = false;
  if (!r || !r.ok) { sh.err = (r && r.error) || "The batch could not be sent."; paint(); return; }
  sh.batch = null; sh.note = { sent: r };
  paint(); vtLoad();
}

/* ---------- Submissions (the maintainer's copy) ---------- */
export const VAULT_REPO = "aprilfridae/fridae-vault";
export const SB_OVERLAP = 0.6;   /* vault_inbox's OVERLAP: below it, no suggestion is asked for */
export const SB_CREATE_Q = "Make aprilfridae/fridae-vault on GitHub, public, with the licence, the readmes and the Roblox guides from this copy of Fridae. It takes a few minutes. Nothing is published until you press Publish vault release.";
export const SB_PUBLISH_Q = "Publish a new vault release? Everyone's Fridae will get the skills and guides added since the last one.";
const SB_DONE: Record<string, string> = { add: "Added", edit: "Edited and added", reject: "Rejected" };
export const sb = {
  fresh: false, loading: false, busy: false, inbox: null as any, err: "",
  made: null as any, pubNote: null as any, pubErr: "", createConfirm: false, pubConfirm: false,
};
export const sbRole = () => (vt.st && vt.st.role && typeof vt.st.role === "object" ? vt.st.role : {});
export const sbOwner = (role: any) => !!(role && role.login) && String(role.login).toLowerCase() === VAULT_REPO.split("/")[0].toLowerCase();
export function sbInboxShown(role: any) { return !!(role && role.maintainer && role.repo_exists); }
export async function sbLoad() {
  if (sb.loading) return;
  sb.loading = true; sb.err = ""; paint();
  let r; try { r = await api("/api/vault/inbox"); } catch (e) { r = null; }
  sb.loading = false;
  sbInboxSet(r || { ok: false, error: "Could not reach the app." });
}
/* an inbox to show (made-up data shows as it would) */
export function sbInboxSet(r: any) {
  sb.err = "";
  if (!r || r.ok === false || !Array.isArray(r.prs)) { sb.inbox = { prs: null }; sb.err = (r && r.error) || "The submissions could not be read."; }
  else sb.inbox = r;
  paint();
}
/* the vault skill a submission is compared with: the one of its name, else the closest (when close enough) */
export function sbAgainst(s: any) {
  if (s.same_name) return String(s.name || "");
  const c = s.closest && typeof s.closest === "object" ? s.closest : null;
  return c && c.id && Number(c.score) >= SB_OVERLAP ? String(c.id) : "";
}
export function sbSaid(s: any, d: any) {
  const why = d.choice === "reject" && d.why ? ": " + d.why : "";
  return (SB_DONE[d.choice] || "Decided") + " “" + (s.name || s.title || "Untitled") + "”" + why;
}
/* on a submission's last undecided skill or guide: that the pull request is closed too */
export function sbLastSay(pr: any, item: any) {
  const all = [...vtList(pr, "skills"), ...vtList(pr, "notes")];
  return all.every((x) => x === item || (x.decided && x.decided.choice))
    ? "This is the last one in this submission, so the pull request also gets a thank-you comment and is closed." : "";
}
/* a decision on one skill or guide of a submission; what went wrong is handed back */
export async function sbDecide(pr: any, s: any, choice: string, extra: any): Promise<string> {
  if (sb.busy) return "";
  sb.busy = true; paint();
  const r = await post("/api/vault/inbox_decide", Object.assign({ pr: Number(pr.n), unit: String(s.unit || "Skills/" + s.name), choice }, extra));
  sb.busy = false;
  if (!r || !r.ok) { paint(); return (r && r.error) || "That did not work."; }
  s.decided = { choice, why: extra.why || "" };
  if (r.closed) pr.closedNote = "Everything here is decided, so the pull request was closed with a thank-you.";
  if (r.warning) pr.warning = String(r.warning);
  paint();
  return "";
}
export async function sbCreateRepo() {
  sb.busy = true; paint();
  const r = await post("/api/vault/create_repo", { confirm: true });
  sb.busy = false;
  sbCreatedSet(r);
  if (r && r.ok) { sb.fresh = true; vtLoad(); }
}
export function sbCreatedSet(r: any) {
  if (r && r.ok) { sb.made = r; sb.err = ""; } else { sb.made = null; sb.err = (r && r.error) || "The vault repo could not be made."; }
  paint();
}
export async function sbPublish() {
  sb.busy = true; sb.pubNote = { busy: true }; paint();
  const r = await post("/api/vault/publish", { confirm: true });
  sb.busy = false;
  sbPublishSet(r);
}
/* Published <tag>, or what stops it: the preflight's problems, each starting with the skill's name */
export function sbPublishSet(r: any) {
  if (r && r.ok) { sb.pubNote = { tag: typeof r.tag === "string" && r.tag ? r.tag : "" }; sb.pubErr = ""; paint(); return; }
  sb.pubNote = null;
  const problems = r && Array.isArray(r.problems) ? r.problems.map(String) : [];
  sb.pubErr = problems.length ? "Not published. Fix these first:\n" + problems.join("\n") : (r && r.error) || "The vault release could not be published.";
  paint();
}
export { paint as poke };
