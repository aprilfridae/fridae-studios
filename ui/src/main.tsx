/* The new screens' way in. The old page (ui/index.html) calls fridae.mount("<screen>", element) where it used to
   paint that screen itself, and writes what it fetches into fridae.store. */
import { render, type ComponentType } from "preact";
import { store } from "./store";
import { Review } from "./review/Review";
import { Lightbox, zoom } from "./review/Lightbox";
import "./review/review.css";

const screens: Record<string, ComponentType<any>> = { review: Review, lightbox: Lightbox };

function mount(name: string, el: Element, props?: Record<string, unknown>) {
  const Screen = screens[name];
  if (!Screen) throw new Error("No screen called " + name);
  render(<Screen {...(props || {})} />, el);
}

const w = window as any;
w.fridae = { mount, store, zoom };
/* the old page ran first: whatever it kept for the new screens is handed over now */
if (typeof w.fridaeReady === "function") w.fridaeReady();
