/* The new screens' way in. The old page (ui/index.html) calls fridae.mount("<screen>", element) where it used to
   paint that screen itself, writes what it fetches into fridae.store, and hands over what the screens need of it
   with fridae.setHost. What the old page still needs of a moved screen is here too (settings, usage, ai, skills). */
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
import { UsagePop } from "./usage/UsagePop";
import { loadUsage, loadUsageSoon } from "./usage/usage";
import "./review/review.css";

const screens: Record<string, ComponentType<any>> = { review: Review, lightbox: Lightbox, settings: Settings, usage: UsageWindow, "usage-pop": UsagePop };

function mount(name: string, el: Element, props?: Record<string, unknown>) {
  const Screen = screens[name];
  if (!Screen) throw new Error("No screen called " + name);
  render(<Screen {...(props || {})} />, el);
}

const w = window as any;
w.fridae = {
  mount, store, zoom, setHost, ai,
  settings: { open: openSettings, close: closeSettings, show: showPage, rag: loadRag },
  usage: { load: loadUsage, soon: loadUsageSoon },
  skills: { open: skOpen, count: skCount },
};
/* the old page ran first: whatever it kept for the new screens is handed over now */
if (typeof w.fridaeReady === "function") w.fridaeReady();
setInterval(loadUsage, 60000);
