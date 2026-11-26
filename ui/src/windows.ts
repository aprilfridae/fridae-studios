/* Settings and Usage are windows over the page: how each is opened and closed. The page behind goes inert and the
   keyboard comes back to where it was (the old page's panelOpened and panelClosed, which its project panel shares). */
import { store } from "./store";
import { host } from "./host";
import { closeSettings, openSettings } from "./settings/state";
import { loadUsage } from "./usage/usage";

const body = () => document.body.classList;
export function openSetup(page?: string) {
  host.closePops(); host.closeProject(); closeUsage();
  body().add("setup"); host.panelOpened(null);
  store.set({ curProj: store.get().nav.proj || "" });
  openSettings(typeof page === "string" ? page : undefined);
  host.refresh(); host.loadEngine(); host.loadGithub();
}
export function closeSetup() {
  if (!body().contains("setup")) return;
  body().remove("setup"); closeSettings(); host.panelClosed();
}
export function openUsage() {
  host.closePops(); host.closeProject(); closeSetup();
  body().add("usage"); host.panelOpened(document.getElementById("us-close"));
  store.set({ usageOpen: true }); loadUsage(); host.refresh();
}
export function closeUsage() {
  if (!body().contains("usage")) return;
  body().remove("usage"); store.set({ usageOpen: false }); host.panelClosed();
}
