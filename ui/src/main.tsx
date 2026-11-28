/* The new screens' way in. The old page (ui/index.html) calls fridae.mount("<screen>", element) where it used to
   paint that screen itself, writes what it fetches into fridae.store, and hands over what the screens need of it
   with fridae.setHost. What the old page still needs of a moved screen is here too (windows, settings, usage, ai, skills). */
import { render, type ComponentType } from "preact";
import { store } from "./store";
import { setHost } from "./host";
import * as ai from "./ai";
import { Review } from "./review/Review";
import { Lightbox, zoom } from "./review/Lightbox";
import { Settings } from "./settings/Settings";
import { closeSettings, openSettings, showPage } from "./settings/state";
import { skCount, skOpen } from "./settings/skills-data";
import { loadRag } from "./settings/Rag";
import { UsageWindow } from "./usage/UsageWindow";
import { Sidebar, SideOver } from "./sidebar/Sidebar";
import { Overview } from "./overview/Overview";
import { stats } from "./overview/graph";
import * as windows from "./windows";
import { loadUsage, loadUsageSoon } from "./usage/usage";
import { installCopyClean } from "./copyClean";
import "./review/review.css";
import "./settings/settings.css";
import "./usage/usage.css";
import { ChatMain, TopBar, chat } from "./chat/index";
import { Promos, promos } from "./promos/Promos";
import { onEngine } from "./promos/engine";
import { NewProject, newProject } from "./project/NewProject";
import { ProjectPanel, project } from "./project/Project";
import { WindowBar, setUpFrame } from "./ui/WindowBar";
import "./ui/window.css";
import { api } from "./api";
import * as remoteRoute from "./remote/route";
import { boxStatus, start as startRemote, withHistory, withProjects } from "./remote/remote";

const screens: Record<string, ComponentType<any>> = { review: Review, lightbox: Lightbox, settings: Settings, usage: UsageWindow, sidebar: Sidebar, "side-over": SideOver, overview: Overview,
  chat: ChatMain, "chat-top": TopBar, promos: Promos, newproj: NewProject, project: ProjectPanel, "win-bar": WindowBar };

function mount(name: string, el: Element, props?: Record<string, unknown>) {
  const Screen = screens[name];
  if (!Screen) throw new Error("No screen called " + name);
  render(<Screen {...(props || {})} />, el);
}

const w = window as any;
w.fridae = {
  mount, store, zoom, setHost, ai, windows,
  overview: { stats },
  settings: { open: openSettings, close: closeSettings, show: showPage, rag: loadRag },
  usage: { load: loadUsage, soon: loadUsageSoon },
  skills: { open: skOpen, count: skCount },
  chat,
  promos: { ...promos, onEngine }, project, newProject,
  /* the old page's api() comes here, so a call about another computer's chat or project goes there (remote/route.ts) */
  api,
  remote: { start: startRemote, withProjects, withHistory, boxStatus, machineOf: remoteRoute.machineOf, bare: remoteRoute.bare, homeOf: remoteRoute.homeOf, isHome: remoteRoute.isHome, at: remoteRoute.at },
};
setUpFrame();
installCopyClean();   /* what is copied from the window never carries its token (copyClean.ts) */
/* the old page ran first: whatever it kept for the new screens is handed over now */
if (typeof w.fridaeReady === "function") w.fridaeReady();
loadUsage(); setInterval(loadUsage, 60000);
