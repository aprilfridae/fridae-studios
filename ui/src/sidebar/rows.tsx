/* The sidebar's rows. Each is drawn again only when what it shows has changed (memo): a step taken by one chat
   touches that chat's row and nothing else, and a press that is down while a step arrives still lands. */
import { host } from "../host";
import { memo } from "../ui/memo";
import { foldSide } from "./ease";
import { setXpOpen, xpOld, xpOpen } from "./fold";
import { openMenu } from "./ProjectMenu";
import type { ExpertRow, MoreRow } from "./model";
import { AiLogo, AI_NAME } from "../ui/AiLogo";

const CHEVRON = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 1.5l3 3-3 3" /></svg>;
const things = (n: number, one: string, many: string) => n + (n === 1 ? one : many);

export const AllRow = memo(function AllRow({ on, busy }: { on: boolean; busy: boolean }) {
  return (
    <button class={"proj all" + (on ? " on" : "")} title="Every project, its chats and its experts, and what needs you" data-k="all" onClick={() => host.showAll()}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4"><rect x="1" y="1" width="4" height="4" rx="1" /><rect x="7" y="1" width="4" height="4" rx="1" /><rect x="1" y="7" width="4" height="4" rx="1" /><rect x="7" y="7" width="4" height="4" rx="1" /></svg>
      <b>Overview</b><span class="state" key={busy ? "busy" : "idle"} title={busy ? "Agents are working" : undefined}>{busy && <span class="dot live"></span>}</span>
    </button>
  );
});

/* Review, for every project: the count is everything that waits for the user */
export const ReviewRow = memo(function ReviewRow({ on, n, k, strong }: { on: boolean; n: number; k: number; strong: boolean }) {
  const tip = (n ? things(n, " thing waits", " things wait") + " for you" + (k === 1 ? " in one project" : " across " + k + " projects") + (strong ? ", and a run is stopped until you answer" : "") + ". " : "") + "What is happening on every project, and what needs you";
  return (
    <button class={"proj all rev" + (on ? " on" : "")} title={tip} data-k="review" aria-label={n ? "Review, " + things(n, " thing needs you", " things need you") : undefined} aria-current={on ? "page" : undefined} onClick={() => host.setView("review")}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1 3l1.2 1.2L4.4 2M1 8.4l1.2 1.2 2.2-2.2M6.6 3.2H11M6.6 8.6H11" /></svg>
      <b>Review</b><span class="state">{n > 0 && <span class="count" aria-hidden="true">{n}</span>}</span>
    </button>
  );
});

/* the logos of the AIs a project's chats run on */
const Logos = ({ ais }: { ais: string }) => ais ? <span class="p-ais" title={"Runs on " + ais.split(" ").map((a) => AI_NAME[a] || a).join(", ")}>{ais.split(" ").slice(0, 3).map((a) => <AiLogo ai={a} size={12} />)}</span> : null;
/* a new chat in this project: the empty page and the message box (nothing is made until the first message) */
const NewChat = ({ path, name }: { path: string; name: string }) => (
  <span class="p-new" role="button" aria-label={"New chat in " + name} title="New chat" onClick={(ev) => { ev.stopPropagation(); host.newChat(path); }}>
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5.5 1.5v8M1.5 5.5h8" /></svg>
  </span>
);
type ProjProps = { path: string; name: string; cls: string; n: number; strong: boolean; working: boolean; menu: boolean; ais: string };
export const ProjectRow = memo(function ProjectRow({ path, name, cls, n, strong, working, menu, ais }: ProjProps) {
  const review = (ev: Event) => { ev.stopPropagation(); host.openInReview(path); };
  return (
    <button class={"proj" + cls} title={path} data-k={"p:" + path} onClick={() => host.showProject(path)}>
      <span class="p-name"><b>{name}</b><Logos ais={ais} /></span>
      <span class="state" key={!n && working ? "busy" : "idle"} title={!n && working ? "Agents are working on it" : undefined}>
        <NewChat path={path} name={name} />
        {/* the project's menu (Rename, Open folder, Delete): this button, a right-click, or Shift+F10 on the row */}
        <span class="p-more" role="button" aria-label={"Menu for " + name} aria-haspopup="menu" aria-expanded={menu ? "true" : "false"} title="Rename, open folder, delete" onClick={(ev) => { ev.stopPropagation(); openMenu(path, ev.currentTarget as Element); }}>
          <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="currentColor"><circle cx="2" cy="6" r="1.1" /><circle cx="6" cy="6" r="1.1" /><circle cx="10" cy="6" r="1.1" /></svg>
        </span>
        {n > 0 ? <span class="count" role="button" tabIndex={0} title={things(n, " thing waits", " things wait") + " for you in Review" + (strong ? ", and the run is stopped until you answer" : "") + ". Press to see them in Review."}
          onClick={review} onKeyDown={(ev) => { if (ev.key === "Enter") review(ev); }}>{n}</span>
          : working ? <span class="dot live"></span> : null}
      </span>
    </button>
  );
});

export const NoneRow = memo(function NoneRow({ cls, busy, ais }: { cls: string; busy: boolean; ais: string }) {
  return <button class={"proj none" + cls} title="Chats that belong to no project" data-k="none" onClick={() => host.showProject("")}><span class="p-name"><b>No project</b><Logos ais={ais} /></span><span class="state"><NewChat path="" name="No project" /><span class={"dot" + (busy ? " live" : "")}></span></span></button>;
});

export const AddRow = memo(function AddRow({ on }: { on: boolean }) {
  return (
    <button class={"proj add" + (on ? " on" : "")} data-k="add" onClick={() => host.showNewProject()}>
      <svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5.5 1.5v8M1.5 5.5h8" /></svg><b>New project</b>
    </button>
  );
});

type ChatProps = { id: string; name: string; on: boolean; live: boolean; count: number; open: boolean; tip: string; ai: string };
export const ChatRow = memo(function ChatRow({ id, name, on, live, count, open, tip, ai }: ChatProps) {
  const many = count === 1 ? "expert" : count + " experts";
  return (
    <div class="s-chat">
      {count > 0 && <button class="s-tw" data-k={"t:" + id} aria-controls={"kids-" + id} aria-expanded={open ? "true" : "false"} aria-label={(open ? "Hide the " : "Show the ") + many + " of " + name} title={(open ? "Hide" : "Show") + " the experts this chat started"}
        onClick={() => foldSide("k:" + id, xpOpen(id), () => setXpOpen(id, !xpOpen(id)))}>{CHEVRON}</button>}
      <button class={"s-row" + (on ? " on" : "")} data-k={"c:" + id} title={tip} aria-current={on ? "true" : undefined} onClick={() => host.openChat(id)}>
        <span class={"s-ai" + (live ? " live" : "")}><AiLogo ai={ai} size={13} /></span><b>{name}</b><span class="s-n">{count ? String(count) : ""}</span>
      </button>
    </div>
  );
});

/* One expert's row. The row is a button in a plain block (.s-x), and the block draws its piece of the guide line: a
   line drawn from inside a button does not land where it is put. */
export const ExpertRowView = memo(function ExpertRowView(x: Omit<ExpertRow, "t" | "key">) {
  return (
    <div class={"s-x" + (x.of ? "" : " lone")}>
      <button class={"s-row sx" + x.cls} data-k={"x:" + x.id} data-of={x.of ? "c:" + x.of : undefined} title={x.tip} aria-current={x.on ? "true" : undefined} tabIndex={x.tab ? -1 : undefined} onClick={() => host.openChatById(x.id)}>
        <span class={"dot" + x.dot}></span><span class="s-t"><b><em>{x.who}</em>{" "}<span>{x.job}</span></b><small>{x.small}</small></span><span class="s-turn" key={x.turnTip ? "turn" : "none"} title={x.turnTip || undefined}>Studio</span>
      </button>
    </div>
  );
});

/* a chat's earlier experts, behind one line */
export const MoreRowView = memo(function MoreRowView({ id, text, open, tip }: Omit<MoreRow, "t" | "key">) {
  return (
    <div class="s-x">
      <button class="s-more" data-k={"m:" + id} data-of={"c:" + id} aria-controls={"old-" + id} aria-expanded={open ? "true" : "false"} title={tip}
        onClick={() => foldSide("o:" + id, xpOld.has(id), () => { if (!xpOld.delete(id)) xpOld.add(id); })}>{CHEVRON}<span>{text}</span></button>
    </div>
  );
});
