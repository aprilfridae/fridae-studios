/* The sidebar's rows. Each is drawn again only when what it shows has changed (memo): a step taken by one chat
   touches that chat's row and nothing else, and a press that is down while a step arrives still lands. */
import { host } from "../host";
import { memo } from "../ui/memo";
import { foldSide } from "./ease";
import { setXpOpen, xpOld, xpOpen } from "./fold";
import { openMenu } from "./ProjectMenu";
import type { ExpertRow, MoreRow } from "./model";
import { AiLogo, AI_NAME } from "../ui/AiLogo";
import { bare } from "../remote/route";

const CHEVRON = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 1.5l3 3-3 3" /></svg>;
/* The project rows are laid out after T3 Code's project folders (MIT; see THIRD_PARTY.md): a plain header row (icon,
   name, and an arrow that turns when it folds) with its chats under it, each lined up with the project's name */
const FOLD = <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M3 4.5l3 3 3-3" /></svg>;
const FOLDER = <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"><path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.5 1.6h4.9A1.5 1.5 0 0 1 14 6.1v5.4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" /></svg>;
const LOOSE = <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"><path d="M2.5 4A1.5 1.5 0 0 1 4 2.5h8A1.5 1.5 0 0 1 13.5 4v5.5A1.5 1.5 0 0 1 12 11H6.5l-3 2.5V11h0A1 1 0 0 1 2.5 10z" /></svg>;
const things = (n: number, one: string, many: string) => n + (n === 1 ? one : many);

export const AllRow = memo(function AllRow({ on, busy }: { on: boolean; busy: boolean }) {
  return (
    <button class={"proj all" + (on ? " on" : "")} title="Every project, its chats and its experts, and what needs you" data-k="all" onClick={() => host.showAll()}>
      <span class="p-ico"><svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4"><rect x="1" y="1" width="4" height="4" rx="1" /><rect x="7" y="1" width="4" height="4" rx="1" /><rect x="1" y="7" width="4" height="4" rx="1" /><rect x="7" y="7" width="4" height="4" rx="1" /></svg></span>
      <b>Overview</b><span class="state" key={busy ? "busy" : "idle"} title={busy ? "Agents are working" : undefined}>{busy && <span class="dot live"></span>}</span>
    </button>
  );
});

/* Review, for every project: the count is everything that waits for the user */
export const ReviewRow = memo(function ReviewRow({ on, n, k, strong }: { on: boolean; n: number; k: number; strong: boolean }) {
  const tip = (n ? things(n, " thing waits", " things wait") + " for you" + (k === 1 ? " in one project" : " across " + k + " projects") + (strong ? ", and a run is stopped until you answer" : "") + ". " : "") + "What is happening on every project, and what needs you";
  return (
    <button class={"proj all rev" + (on ? " on" : "")} title={tip} data-k="review" aria-label={n ? "Review, " + things(n, " thing needs you", " things need you") : undefined} aria-current={on ? "page" : undefined} onClick={() => host.setView("review")}>
      <span class="p-ico"><svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1 3l1.2 1.2L4.4 2M1 8.4l1.2 1.2 2.2-2.2M6.6 3.2H11M6.6 8.6H11" /></svg></span>
      <b>Review</b><span class="state">{n > 0 && <span class="need" aria-hidden="true">{n}</span>}</span>
    </button>
  );
});

/* a new chat in this project: the empty page and the message box (nothing is made until the first message) */
const NewChat = ({ path, name }: { path: string; name: string }) => (
  <span class="p-new" role="button" aria-label={"New chat in " + name} title="New chat" onClick={(ev) => { ev.stopPropagation(); host.newChat(path); }}>
    <svg aria-hidden="true" width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M7 2H3.5A1.5 1.5 0 0 0 2 3.5v7A1.5 1.5 0 0 0 3.5 12h7a1.5 1.5 0 0 0 1.5-1.5V7" /><path d="M10.6 1.6a1.1 1.1 0 0 1 1.6 1.6L7.4 8 5.5 8.5 6 6.6z" /></svg>
  </span>
);
/* the computer a project or chat of another of the user's computers is on, after its name */
const Where = ({ where }: { where: string }) => where ? <small class="p-where">{where}</small> : null;
type ProjProps = { path: string; name: string; cls: string; n: number; strong: boolean; working: boolean; menu: boolean; ais: string; where: string };
export const ProjectRow = memo(function ProjectRow({ path, name, cls, n, strong, working, menu, ais, where }: ProjProps) {
  const review = (ev: Event) => { ev.stopPropagation(); host.openInReview(path); };
  return (
    <button class={"proj" + cls} title={where ? bare(path) + " on " + where : path} data-k={"p:" + path} onClick={() => host.showProject(path)}>
      <span class="p-ico">{FOLDER}</span>
      <span class="p-name"><b>{name}</b><Where where={where} /></span>
      <span class="state" key={!n && working ? "busy" : "idle"} title={!n && working ? "Agents are working on it" : undefined}>
        <NewChat path={path} name={name} />
        {/* the project's menu (Rename, Open folder, Delete): this button, a right-click, or Shift+F10 on the row */}
        <span class="p-more" role="button" aria-label={"Menu for " + name} aria-haspopup="menu" aria-expanded={menu ? "true" : "false"} title="Rename, open folder, delete" onClick={(ev) => { ev.stopPropagation(); openMenu(path, ev.currentTarget as Element); }}>
          <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="currentColor"><circle cx="2" cy="6" r="1.1" /><circle cx="6" cy="6" r="1.1" /><circle cx="10" cy="6" r="1.1" /></svg>
        </span>
        {n > 0 ? <span class="need" role="button" tabIndex={0} title={things(n, " thing waits", " things wait") + " for you in Review" + (strong ? ", and the run is stopped until you answer" : "") + ". Press to see them in Review."}
          onClick={review} onKeyDown={(ev) => { if (ev.key === "Enter") review(ev); }}>{n}</span>
          : working ? <span class="dot live"></span> : null}
      </span>
      <span class="p-fold">{FOLD}</span>
    </button>
  );
});

/* `path`: "" for this computer's chats of no project, "<machine>~" for another computer's */
export const NoneRow = memo(function NoneRow({ path, cls, busy, ais, where }: { path: string; cls: string; busy: boolean; ais: string; where: string }) {
  return <button class={"proj none" + cls} title={where ? "Chats on " + where.split(" · ")[0] + " that belong to no project" : "Chats that belong to no project"} data-k={path ? "none:" + path.slice(0, -1) : "none"} onClick={() => host.showProject(path)}><span class="p-ico">{LOOSE}</span><span class="p-name"><b>No project</b><Where where={where} /></span><span class="state"><NewChat path={path} name="No project" />{busy && <span class="dot live"></span>}</span><span class="p-fold">{FOLD}</span></button>;
});

export const AddRow = memo(function AddRow({ on }: { on: boolean }) {
  return (
    <button class={"proj add" + (on ? " on" : "")} data-k="add" onClick={() => host.showNewProject()}>
      <span class="p-ico"><svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5.5 1.5v8M1.5 5.5h8" /></svg></span><b>New project</b>
    </button>
  );
});

type ChatProps = { id: string; name: string; on: boolean; live: boolean; count: number; open: boolean; tip: string; ai: string; where: string };
export const ChatRow = memo(function ChatRow({ id, name, on, live, count, open, tip, ai, where }: ChatProps) {
  const many = count === 1 ? "expert" : count + " experts";
  return (
    <div class="s-chat">
      {count > 0 && <button class="s-tw" data-k={"t:" + id} aria-controls={"kids-" + id} aria-expanded={open ? "true" : "false"} aria-label={(open ? "Hide the " : "Show the ") + many + " of " + name} title={(open ? "Hide" : "Show") + " the experts this chat started"}
        onClick={() => foldSide("k:" + id, xpOpen(id), () => setXpOpen(id, !xpOpen(id)))}>{CHEVRON}</button>}
      <button class={"s-row" + (on ? " on" : "")} data-k={"c:" + id} title={tip} aria-current={on ? "true" : undefined} onClick={() => host.openChat(id)}>
        <b>{name}</b>{live && <span class="s-now">Working</span>}{where && <span class="s-where">{where}</span>}<span class={"s-ai" + (live ? " live" : "")} title={AI_NAME[ai] || ai}><AiLogo ai={ai} size={12} /></span>
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
