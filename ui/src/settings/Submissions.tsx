/* Settings, Knowledge, Submissions: the vault maintainer's inbox. One block per pull request: its title, who sent it
   and its link; the reason when it could not be read; the files that are not a skill or guide; then a card per skill
   and a card per guide or note, each with Add (asked first), Edit then add, and Reject. Everything in a submission is
   a stranger's: it is only ever drawn as text. */
import { useRef, useState } from "preact/hooks";
import { ScriptsMark } from "../ui/icons";
import { VaultLink } from "./Cards";
import { SB_CREATE_Q, SB_PUBLISH_Q, VAULT_REPO, poke, sb, sbAgainst, sbCreateRepo, sbDecide, sbInboxShown, sbLastSay, sbLoad, sbOwner, sbPublish, sbRole, sbSaid, vt, vtKind, vtList } from "./skills-data";

/* a confirm card in the page's own idiom (End this run): the question, the action's own button and Not now */
function Confirm({ text, go, run, close }: { text: string; go: string; run(): Promise<unknown>; close(): void }) {
  const [busy, setBusy] = useState(false);
  return <div class="ask enter"><p>{text}</p><div class="act"><button class="pill main" disabled={busy || sb.busy} onClick={async () => { setBusy(true); await run(); close(); }}>{go}</button><button class="pill" disabled={busy || sb.busy} onClick={close}>Not now</button></div></div>;
}
/* a file of a submission: a script in its own box, open; any other file folded, SKILL.md open when there is nothing to
   compare it with */
function File({ f, open }: { f: any; open?: boolean }) {
  const path = String(f.path || ""), text = typeof f.text === "string" ? f.text : "";
  if (f.script) return <div class="sk-side sb-script"><p class="sk-scripts">{ScriptsMark}<span>Runs on everyone's computer: read it</span></p><code>{path}</code>{f.note ? <p class="sb-gone">{String(f.note)}</p> : <pre>{text}</pre>}</div>;
  if (f.note) return <p class="sb-gone"><code>{path}</code><span>{String(f.note)}</span></p>;
  return <details class="moreinfo sb-file" open={!!open}><summary>{path}</summary><pre>{text}</pre></details>;
}
/* one skill (`skill`) or one guide or note of a submission */
function Card({ pr, s, skill }: { pr: any; s: any; skill: boolean }) {
  const [box, setBox] = useState(""), [err, setErr] = useState(""), [text, setText] = useState(""), [why, setWhy] = useState(""), [asking, setAsking] = useState(false), [words, setWords] = useState({ add: "", edit: "", reject: "" });
  const ta = useRef<HTMLTextAreaElement>(null), reason = useRef<HTMLInputElement>(null), addGo = useRef<HTMLButtonElement>(null);
  const files = skill ? vtList(s, "files").slice().sort((x, y) => (y.script ? 1 : 0) - (x.script ? 1 : 0)) : [];
  const md = files.find((f) => !f.script && f.path === "SKILL.md"), rest = files.filter((f) => !f.script && f !== md);
  const against = skill ? sbAgainst(s) : "", diff = typeof s.diff === "string" && s.diff ? s.diff : "";
  const rec = skill && s.recommendation && typeof s.recommendation === "object" ? s.recommendation : null;
  const merged: string = rec && rec.choice === "merge" && typeof rec.merged === "string" && rec.merged.trim() ? rec.merged : "";
  const pick = !rec ? "" : rec.choice === "replace" ? "add" : rec.choice === "keep" ? "reject" : merged ? "edit" : "";
  const title = skill ? s.name || "Untitled" : String(s.title || String(s.path || "").split("/").pop() || "Untitled");
  const own = typeof s.text === "string" ? s.text : "", guide = vtKind(s.unit || s.path) === "guide";
  const last = sbLastSay(pr, s);
  const decide = async (choice: string, extra: any) => { setErr(""); const e = await sbDecide(pr, s, choice, extra); if (e) setErr(e); };
  const press = (choice: string) => {
    setErr("");
    if (choice === "edit" && !skill && s.binary) { setBox(""); setErr("Only text can be edited here."); return; }
    setBox(choice); setAsking(choice === "add");
    setWords({ ...words, [choice]: choice === "add" ? "Add “" + title + "” to the vault?" + (last ? " " + last : "") + (skill ? " Nothing reaches people until you publish a release." : "")
      : choice === "edit" ? last : "Your reason goes in the comment Fridae leaves on the pull request when it is closed, where anyone can read it." + (last ? " " + last : "") });
    if (choice === "edit" && !text) setText(skill ? merged || (md && typeof md.text === "string" ? md.text : "") : own);
    /* the keyboard goes to what just opened, once it is drawn */
    setTimeout(() => { const to = choice === "add" ? addGo.current : choice === "edit" ? ta.current : reason.current; if (to) to.focus(); });
  };
  const d = s.decided && typeof s.decided === "object" && s.decided.choice ? s.decided : null;
  const two = (go: string, run: () => void, ref?: any) => <div class="act"><button class="pill main" ref={ref} disabled={sb.busy} onClick={run}>{go}</button><button class="pill" disabled={sb.busy} onClick={() => setBox("")}>Cancel</button></div>;
  return (
    <div class={"ask rv-item sk-dec" + (d ? " done" : "")}>
      <div class="meta">{skill ? (s.same_name ? "Skill · the vault has one of this name" : against ? "Skill · close to the vault's “" + against + "”" : "Skill · new to the vault") : (guide ? "Guide" : "Note") + " · " + (pr.author || "someone")}</div>
      <h4>{title}</h4>
      {skill ? <div class="sb-scripts">{files.filter((f) => f.script).map((f) => <File f={f} />)}</div> : <p class="sub">{(s.in_vault ? "Changes a " : "New ") + (guide ? "guide" : "note") + (s.in_vault ? " in the vault" : "")}</p>}
      {skill ? <div class="sb-files">
        {md && <File f={md} open={!diff} />}
        {diff && <details class="moreinfo sb-diff" open><summary>{against ? "Changes from the vault's “" + against + "”" : "Changes from the vault's skill"}</summary><pre>{diff}</pre></details>}
        {rest.map((f) => <File f={f} />)}
      </div> : <div class="sb-files">
        <p class="sb-gone"><code>{String(s.path || s.unit || "")}</code></p>
        {s.note ? <p class="sb-gone">{String(s.note)}</p> : (diff || own) ? <details class="moreinfo sb-diff"><summary>Show the change</summary><pre>{s.in_vault && diff ? diff : own}</pre></details> : null}
      </div>}
      {/* the suggestion, worded as the decision cards word theirs */}
      {skill && (!rec ? against && <div class="plain-words sk-rec none"><b>No suggestion yet</b><p>A cheap AI compares it with the vault's skill and suggests what to do when one is connected (Settings, Connections). You can decide without it.</p></div>
        : <div class={"plain-words sk-rec" + (pick ? "" : " none")}><b>{pick === "add" ? "Suggested: Add, replacing " + (against || "the vault's") : pick === "reject" ? "Suggested: Reject, the vault's is better" : pick === "edit" ? "Suggested: Edit then add, merged" : "No clear pick"}</b>{rec.why && <p>{String(rec.why)}</p>}</div>)}
      {merged && <details class="moreinfo sk-merged"><summary>See the merged version</summary><pre>{merged}</pre></details>}
      <div class="act">
        {[["add", "Add"], ["edit", "Edit then add"], ["reject", "Reject"]].map(([choice, label]) => <button class={"pill" + (choice === pick ? " main" : "")} disabled={sb.busy} title={choice === pick ? "The suggestion" : undefined} onClick={() => press(choice)}>{label}</button>)}
      </div>
      {/* what each step does, said plainly; on the submission's last undecided one, that the pull request is closed too */}
      {skill ? <div class="sb-add" hidden={box !== "add"}><p>{words.add}</p>{two("Add it", () => decide("add", {}), addGo)}</div>
        : <div class="sb-add" hidden={box !== "add"}>{box === "add" && asking && <Confirm text={"Add “" + title + "” to the vault? It goes out in the next release." + (last ? " " + last : "")} go="Add it" run={() => decide("add", {})} close={() => setAsking(false)} />}</div>}
      <div class="sb-edit" hidden={box !== "edit"}><p hidden={!words.edit}>{words.edit}</p>
        <textarea ref={ta} spellcheck={false} aria-label={skill ? "SKILL.md, as it will be added" : "The guide, as it will be added"} value={text} onInput={(e) => setText((e.target as HTMLTextAreaElement).value)}></textarea>
        {two("Add this version", () => { if (!text.trim()) { setErr("The text is empty."); return; } decide("edit", { text }); })}</div>
      <div class="sb-reject" hidden={box !== "reject"}><p>{words.reject}</p>
        <div class="keyrow"><input type="text" ref={reason} maxlength={200} placeholder="Why, in one line (optional; the sender sees it on GitHub)" autocomplete="off" aria-label="Why it is rejected (optional; the sender sees it on GitHub)" value={why}
          onInput={(e) => setWhy((e.target as HTMLInputElement).value)} onKeyDown={(e) => { if (e.key === "Enter") decide("reject", { why: why.trim() }); }} /></div>
        {two("Reject it", () => decide("reject", { why: why.trim() }))}</div>
      <p class="rv-err">{err}</p>
      <div class="rv-said">{d ? sbSaid(s, d) : ""}</div>
    </div>
  );
}
function Pr({ pr }: { pr: any }) {
  const n = Number(pr.n), skills = vtList(pr, "skills");
  const gone = [...new Set([...vtList(pr, "not_skill"), ...skills.flatMap((s) => vtList(s, "not_skill"))].map(String))];
  return (
    <div class="sb-pr">
      <p class="sb-pr-h"><b>{pr.title || "Untitled"}</b><span>{"by " + (pr.author || "someone") + " · "}{Number.isInteger(n) && n > 0 ? <VaultLink text={"pull request #" + n} what="pr" extra={{ n }} /> : "pull request"}</span></p>
      <p class="sub sk-err sb-pr-err" role="status">{pr.error ? String(pr.error) : pr.warning || ""}</p>
      <p class="sub sb-pr-note" role="status" hidden={!pr.closedNote}>{pr.closedNote || ""}</p>
      {!pr.error && gone.map((p) => <p class="sb-not"><code>{p}</code><span>Not a skill or guide: will not be added</span></p>)}
      {!pr.error && skills.map((s) => <Card pr={pr} s={s} skill />)}
      {!pr.error && vtList(pr, "notes").map((x) => <Card pr={pr} s={x} skill={false} />)}
    </div>
  );
}
export function Submissions() {
  const st = vt.st, role = sbRole(), create = sbOwner(role) && !role.repo_exists, inbox = sbInboxShown(role);
  const prs: any[] | null = sb.inbox ? sb.inbox.prs : null;
  const waiting = (prs || []).filter((pr) => !pr.error).reduce((n, pr) => n + [...vtList(pr, "skills"), ...vtList(pr, "notes")].filter((x) => !(x.decided && x.decided.choice)).length, 0);
  return (
    <section class="st-sec" id="sb-wrap" hidden={!(role.maintainer || create)}>
      <div class="st-sec-head"><h2 class="sk-h">Submissions <span id="sb-n">{inbox && waiting ? String(waiting) : ""}</span></h2><div class="st-sec-act"><button class="pill sm" id="sb-again" hidden={!inbox} disabled={sb.loading} onClick={() => sbLoad()}>Look again</button></div></div>
      <div class="sg"><div class="sr sr-notes">
        <div id="sb-create-row" hidden={!create}>
          <p class="sub">The vault's repository is not on GitHub yet.</p>
          <div class="act"><button class="pill" id="sb-create" disabled={sb.busy} onClick={() => { if (sb.busy) return; sb.err = ""; sb.createConfirm = true; poke(); }}>Create the vault repo</button></div>
        </div>
        <div id="sb-create-box">{create && sb.createConfirm && <Confirm text={SB_CREATE_Q} go="Create the vault repo" run={sbCreateRepo} close={() => { sb.createConfirm = false; poke(); }} />}</div>
        <p class="sub" id="sb-made" role="status" hidden={!sb.made}>{sb.made && <>Made <VaultLink text={String(sb.made.repo || VAULT_REPO)} what="vault" />.</>}</p>
        <div id="sb-inbox" hidden={st ? !inbox : undefined}>
          {!inbox ? null : !sb.inbox ? (sb.loading ? <p class="sub">Loading...</p> : null) : !prs ? null : !prs.length ? <p class="sub">No submissions waiting.</p> : prs.map((pr) => <Pr pr={pr} />)}
        </div>
        <div class="act" id="sb-pub-row" hidden={!inbox}><button class="pill" id="sb-publish" disabled={sb.busy} onClick={() => { if (sb.busy) return; sb.pubErr = ""; sb.pubNote = null; sb.pubConfirm = true; poke(); }}>Publish vault release</button>
          <span class="sub" id="sb-pub-note" role="status">{!sb.pubNote ? "" : sb.pubNote.busy ? "Publishing…" : <>Published {sb.pubNote.tag ? <VaultLink text={sb.pubNote.tag} what="release" extra={{ tag: sb.pubNote.tag }} /> : "the release"}.</>}</span></div>
        <div id="sb-pub-box">{inbox && sb.pubConfirm && <Confirm text={SB_PUBLISH_Q} go="Publish vault release" run={sbPublish} close={() => { sb.pubConfirm = false; poke(); }} />}</div>
        <p class="sub sk-err" id="sb-pub-err" role="status">{sb.pubErr}</p>
        <p class="sub sk-err" id="sb-err" role="status">{sb.err}</p>
      </div></div>
    </section>
  );
}
