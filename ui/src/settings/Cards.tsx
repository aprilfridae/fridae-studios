/* The cards under "Needs your decision" on Settings, Skills: two skills (or two notes) that cover the same thing, and
   files of the vault that clash with yours. A card that was answered folds to one line. */
import type { ComponentChildren } from "preact";
import { api } from "../api";
import { ago } from "../util";
import { ScriptsMark } from "../ui/icons";
import { SK_OWNER, sk, skDecide, skFrom, skHeadline, skHome, skLabels, skSideInfo, vt, vtClashDecide, vtClashWords, vtKind, vtReveal, vtSize, poke } from "./skills-data";

/* one side of a decision: whether it has scripts (first: code that would run on this computer), in use or new, its
   name, whose it is and where it came from, what it does, its folder */
export function Side(p: { tag: string; name: string; who?: string; desc?: string; where?: string; whereTitle?: string; scripts?: boolean; children?: ComponentChildren }) {
  return (
    <div class="sk-side">
      {p.scripts && <p class="sk-scripts" title="This skill carries programs an AI may run on this computer. Read them before you keep it.">{ScriptsMark}<span>Has scripts: runs on your Mac</span></p>}
      <em>{p.tag}</em><b>{p.name || "Untitled"}</b>
      {p.who && <p class="sk-who">{p.who}</p>}{p.desc && <p class="sk-desc">{p.desc}</p>}{p.where && <code title={p.whereTitle || p.where}>{p.where}</code>}
      {p.children}
    </div>
  );
}
const when = (made: any) => { const t = made ? Date.parse(made) : NaN; return isNaN(t) ? "" : ago(t / 1000); };

export function DecisionCard({ d, said }: { d: any; said?: string }) {
  const note = d.kind === "note", a = note ? null : skSideInfo(d, "a"), b = note ? null : skSideInfo(d, "b");
  const app = !note && a.owner === "app", labels = skLabels(d, a), rec = d.recommendation || null;
  const canMerge = !app && !!(rec && typeof rec.merged === "string" && rec.merged.trim());
  const from = note ? "" : skFrom(d.source), lines = d.lines || {};
  const who = (x: any, extra: string) => [SK_OWNER[x.owner] || x.owner, extra, x.licence ? "licence " + x.licence : ""].filter(Boolean).join(" · ");
  /* the suggestion, in the plain-words box Review uses */
  const pick = rec && rec.choice && labels[rec.choice] && !(app && rec.choice !== "keep") && (rec.choice !== "merge" || canMerge) ? rec.choice : "";
  return (
    <div class={"ask rv-item sk-dec enter" + (said ? " done" : "")} data-id={d.id}>
      <div class="meta">{[note ? "Note" : "Skill", when(d.made)].filter(Boolean).join("  ·  ")}</div>
      <h4>{skHeadline(d)}</h4>
      <div class="sk-sides">
        {note ? <><Side tag="First" name={d.a} desc={lines.a || ""} where={skHome(d.a_dir)} whereTitle={d.a_dir} /><Side tag="Second" name={d.b} desc={lines.b || ""} where={skHome(d.b_dir)} whereTitle={d.b_dir} /></>
          : <><Side tag="In use" name={d.a} who={who(a, a.origin === "project" ? "this project's" : "")} desc={a.description} where={a.source || skHome(d.a_dir)} whereTitle={[a.source, d.a_dir].filter(Boolean).join("\n")} scripts={a.scripts} />
            <Side tag="New" name={d.b} who={who(b, from ? "from " + from : "")} desc={b.description} where={b.source || skHome(d.source || d.b_dir)} whereTitle={[b.source, d.source || d.b_dir].filter(Boolean).join("\n")} scripts={b.scripts} /></>}
      </div>
      {!rec ? <div class="plain-words sk-rec none"><b>No suggestion yet</b><p>A cheap AI reads both and suggests one when one is connected (Settings, Connections). You can decide without it.</p><small class="sk-by"></small></div>
        : <div class={"plain-words sk-rec" + (pick ? "" : " none")}><b>{pick ? "Suggested: " + labels[pick] : "No clear pick"}</b>{rec.why && <p>{rec.why}</p>}<small class="sk-by">{rec.by ? "by " + rec.by : ""}</small></div>}
      {canMerge && <details class="moreinfo sk-merged"><summary>See the merged version</summary><pre>{rec.merged}</pre></details>}
      <div class="act">
        {["keep", "replace", "merge", "keep_both"].filter((c) => !(app && (c === "replace" || c === "merge")) && !(c === "merge" && !canMerge)).map((c) => (
          <button class={"pill" + (c === pick ? " main" : "")} disabled={sk.busy} title={c === pick ? "The suggestion" : undefined} onClick={() => skDecide(d, c)}>{labels[c]}</button>
        ))}
      </div>
      <p class="sub sk-own" hidden={!app}>This is one of Fridae's own skills, so it stays as it is: keep it, or keep both.</p>
      <p class="rv-err">{sk.cardErr.get(d.id) || ""}</p>
      <div class="rv-said">{said || ""}</div>
    </div>
  );
}

export function ClashCard({ c, said }: { c: any; said?: string }) {
  const unit = String(c.unit || ""), title = String(c.title || unit.split("/").pop() || "Untitled"), m = c.mine || {}, v = c.vault || {};
  const reveal = (which: string) => async () => { const e = await vtReveal(unit, which); if (e) vt.clashCardErr.set(unit, e); else vt.clashCardErr.delete(unit); poke(); };
  const modified = Number(m.modified) > 0 ? "changed " + ago(Number(m.modified)) : "";
  const show = c.show && typeof c.show === "object" ? c.show : {};
  const both: [string, string][] = [["Yours", typeof show.mine === "string" ? show.mine : ""], ["The vault's", typeof show.vault === "string" ? show.vault : ""]];
  const side = (tag: string, who: string, where: string, which: string) => <Side tag={tag} name={title} who={who} where={where}><div class="act"><button class="pill sm" onClick={reveal(which)}>Reveal</button></div></Side>;
  return (
    <div class={"ask rv-item sk-dec enter" + (said ? " done" : "")} data-unit={unit}>
      <div class="meta">{[c.kind === "skill" ? "Skill" : vtKind(unit) === "guide" ? "Guide" : "File", when(c.made)].filter(Boolean).join("  ·  ")}</div>
      <h4>{title}</h4>
      <div class="sk-sides">
        {side("Yours", [vtSize(m.size), modified].filter(Boolean).join(" · "), String(m.path || unit), "mine")}
        {side("The vault's", [vtSize(v.size), c.release ? "release " + c.release : ""].filter(Boolean).join(" · "), unit, "vault")}
      </div>
      <div class="plain-words sk-rec"><b>In plain words</b><p>{vtClashWords(c)}</p></div>
      {(both[0][1] || both[1][1]) && <details class="moreinfo sk-merged"><summary>Show both</summary>{both.filter((x) => x[1]).map(([head, text]) => [<p>{head}</p>, <pre>{text}</pre>])}</details>}
      <div class="act">
        {[["vault", "Keep the vault's"], ["mine", "Keep mine"]].map(([choice, label]) => <button class="pill" data-choice={choice} disabled={vt.clashBusy} onClick={() => vtClashDecide(c, choice, title)}>{label}</button>)}
      </div>
      <p class="sub sk-own">Keep the vault's sets yours aside in the vault's replaced folder; nothing is deleted.</p>
      <p class="rv-err">{vt.clashCardErr.get(unit) || ""}</p>
      <div class="rv-said">{said || ""}</div>
    </div>
  );
}
/* a link-styled button that opens a page of the vault's in the browser: the app builds the address from which page
   (vault, pr or release) and its number or release name; the page never gives an address */
export function VaultLink({ text, what, extra }: { text: string; what: string; extra?: object }) {
  return <button class="linkish" title="Open on GitHub" onClick={() => api("/api/github/open", Object.assign({ what }, extra || {}))}>{text}</button>;
}
/* a pull request of the vault's, named by its number (read from its address); the words alone without one */
export function PrLink({ url }: { url: any }) {
  const n = String(url || "").match(/\/pull\/(\d+)\/?$/);
  return n ? <VaultLink text={"pull request #" + n[1]} what="pr" extra={{ n: Number(n[1]) }} /> : <>pull request</>;
}
