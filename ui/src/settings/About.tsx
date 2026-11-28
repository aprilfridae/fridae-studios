/* Settings, About: the version, checking for a newer one, and whether updates install by themselves. */
import { api } from "../api";
import { useStore } from "../store";
import { host } from "../host";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Switch } from "../ui/Switch";

export function About() {
  const status = useStore((s) => s.status), u = status.update || {}, known = "version" in status || "ai" in status;
  const texts: Record<string, string> = {
    idle: "Version " + (status.version || ""), checking: "Checking for updates...", current: "Version " + (status.version || "") + " is the newest.",
    available: "Version " + u.latest + " is available." + (u.can_install ? " Press Check now to download it." : " Download it from the releases page" + (u.page ? ": press Open releases page." : ".")),
    downloading: "Downloading version " + u.latest + " (" + Math.round((u.progress || 0) * 100) + "%)",
    ready: "Version " + u.latest + " is downloaded. It installs the next time the app starts, or now if you press Restart to update.",
    error: "Could not check for updates: " + (u.error || "no connection"),
  };
  return (
    <Group id="set-about" title="About">
      <SettingsRow id="set-version" find="update upgrade release" title={<>Fridae <span class="ab-ver" id="ab-ver">{status.version || ""}</span></>} desc={known ? texts[u.status] || texts.idle : ""} descId="upd-state"
        ctl={<>
          {/* a copy that cannot install a new version itself (a Mac): the release's page, to download it there */}
          {u.status === "available" && !u.can_install && u.page && <button class="pill" id="upd-page" onClick={() => api("/api/update", { action: "page" })}>Open releases page</button>}
          <button class="pill" id="upd-check" onClick={async () => { await api("/api/update", { action: "check" }); setTimeout(() => host.refresh(), 600); }}>Check now</button>
        </>} />
      <SettingsRow id="set-auto-update" title="Update automatically" desc="New versions are downloaded when they come out. One installs the next time the app starts."
        ctl={<Switch id="upd-auto" label="Update automatically" on={u.auto !== false} onChange={async (on) => { await api("/api/update", { action: "auto", value: on }); await host.refresh(); }} />} />
    </Group>
  );
}
