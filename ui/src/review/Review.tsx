/* Review: what waits for the user, and nothing else. A tab per project that has something (in the order set when
   the page opened), in a bar that pulls down from the top as a chat's does; in one box, the chosen project's things. A tab goes when its project has nothing left; with no
   tabs the page says so in three words. */
import { useEffect, useRef, useState } from "preact/hooks";
import { store, useStore } from "../store";
import { forget, needsCount, things, type Thing } from "./model";
import { Tabs } from "./Tabs";
import { PullBar } from "../chat/PullBar";
import { ItemsBox } from "./ItemsBox";

/* told to the old page when something is answered, so its counts follow */
export type Answer = (id: string, said: string) => void;

export function Review({ onAnswered }: { onAnswered: Answer }) {
  const projects = useStore((s) => s.projects), review = useStore((s) => s.review), done = useStore((s) => s.reviewDone);
  const order0 = useStore((s) => s.reviewOrder), tab0 = useStore((s) => s.reviewTab), jump = useStore((s) => s.reviewJump);
  const page = useRef<HTMLDivElement>(null);
  const [flash, setFlash] = useState("");

  const have = projects.map((p) => p.path), order = order0.filter((p) => have.includes(p));
  for (const p of have) if (!order.includes(p)) order.push(p);
  const tabs = order.map((p) => ({ path: p, name: projects.find((x) => x.path === p)!.name, things: things(p, review[p], done), n: needsCount(review[p], done) })).filter((t) => t.things.length);
  const tab = tabs.find((t) => t.path === tab0) || tabs[0];
  const on = tab ? tab.path : "";
  forget(new Set(tabs.flatMap((t) => t.things.map((x) => x.key))));

  useEffect(() => { if (on !== tab0) store.set({ reviewTab: on }); }, [on, tab0]);

  /* to a project's tab, and with an id to one thing in it, which is marked for a moment */
  useEffect(() => {
    if (!jump) return;
    const to = tabs.find((t) => t.path === jump.path);
    if (!to) return;
    store.set({ reviewTab: to.path });
    const first = (list: Thing[]) => list.find((x) => x.media.length) || list[0];
    const thing = to.things.find((x) => x.id === jump.id) || first(to.things);
    setFlash(thing.key);
    const t = setTimeout(() => setFlash(""), 1400);
    return () => clearTimeout(t);
  }, [jump && jump.n]);
  useEffect(() => {
    const card = flash && page.current && page.current.querySelector(".rv-card.flash");
    if (card) card.scrollIntoView({ block: jump && jump.id ? "center" : "start" });
  }, [flash]);

  const pick = (path: string) => { store.set({ reviewTab: path }); const box = page.current && page.current.parentElement; if (box) box.scrollTop = 0; };
  const at = tabs.findIndex((t) => t.path === on);
  return (
    <>
    {/* the projects with something waiting, behind the same pull-down tab as a chat's and Overview's */}
    <PullBar keep="rvout" what="the projects" bar="rvbar" cls="rv-fold"><Tabs tabs={tabs} on={on} pick={pick} /></PullBar>
    <div class="rv-page" ref={page}>
      <ItemsBox things={tab ? tab.things : []} labelledBy={at < 0 ? undefined : "rv-tab-" + at} flash={flash} onAnswered={onAnswered} />
      <p class="rv-nothing" id="rv-nothing" hidden={tabs.length > 0}>Nothing needs you</p>
    </div>
    </>
  );
}
