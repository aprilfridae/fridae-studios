/* Settings, Other computers: this computer letting the user's other computers connect. The switch (a guarded setting:
   only this page turns it on), the pairing code with its countdown, this computer's name and port, and the computers
   paired with it, each with Remove. While the page is in front it asks the app every 2 seconds, so a computer that
   just paired, or a code that ran out, shows without a press. */
import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { cell, useStore } from "../store";
import { ago } from "../util";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Switch } from "../ui/Switch";
import { Press } from "../ui/Press";
import { ListRow } from "../ui/Row";

type Device = { id: string; name: string; paired_at: number; last_seen: number };
type Host = { on: boolean; port: number | null; name: string; devices: Device[]; code: { text: string; expires: number } | null; error: string | null };

const host = cell<Host | null>(null);
async function load(body?: unknown) {
  try { host.set(await api("/api/remote/host", body)); } catch (e) {}
}

const day = (ts: number) => new Date(ts * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
/* what is left of a code, as m:ss */
function left(expires: number) {
  const s = Math.max(0, Math.round(expires - Date.now() / 1000));
  return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
}

function Code({ code }: { code: { text: string; expires: number } }) {
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 1000); return () => clearInterval(t); }, []);
  return (
    <div class="rm-pair">
      <div class="rm-code" aria-label={"Pairing code " + code.text.split("").join(" ")}>{code.text}</div>
      <p class="sub">Type this on your other computer. It works once, for <span class="rm-left">{left(code.expires)}</span> more.</p>
      <div class="act"><Press onPress={() => load({ action: "cancel-code" })}>Cancel</Press></div>
    </div>
  );
}

function DeviceRow({ d }: { d: Device }) {
  const [asking, setAsking] = useState(false);
  const used = d.last_seen > d.paired_at ? " · last used " + ago(d.last_seen) : "";
  if (asking) return (
    <ListRow name={d.name} line={"Remove " + d.name + "? It can connect again only after you pair it with a new code."}>
      <div class="act"><Press cls="pill sm main" onPress={() => load({ action: "revoke", device: d.id })}>Remove</Press><button class="pill sm" onClick={() => setAsking(false)}>Cancel</button></div>
    </ListRow>
  );
  return <ListRow name={d.name} line={"Paired " + day(d.paired_at) + used}><button class="pill sm" aria-label={"Remove " + d.name} onClick={() => setAsking(true)}>Remove</button></ListRow>;
}

export function Remote() {
  const s = useStore((st) => st.settings), h = host.use();
  const here = s.open && s.page === "remote";
  useEffect(() => {
    if (!here) return;
    load();
    const t = setInterval(load, 2000);
    return () => clearInterval(t);
  }, [here]);
  const on = !!(h && h.on), devices = (h && h.devices) || [];
  return (
    <Group id="set-remote-host" find="network tailscale pair remote laptop devices" title="This computer">
      <SettingsRow id="set-remote-on" title="Let my other computers connect" labelFor="remote-on"
        desc="Your other computers on this network, or on Tailscale, can run chats here after you pair them."
        ctl={<Switch id="remote-on" on={on} onChange={(v) => load({ action: v ? "on" : "off" })} />}>
        {on && !devices.length && <p class="sr-status">Windows may ask whether Fridae can use the network. Press Allow.</p>}
        {h && h.error && <p class="sr-status rm-error">{h.error}</p>}
      </SettingsRow>
      {on && h && h.port !== null && (
        <SettingsRow id="set-remote-pair" title="Pairing" desc={"This computer is " + h.name + ", on port " + h.port + "."}
          ctl={h.code ? undefined : <Press cls="pill" onPress={() => load({ action: "code" })}>Pair a computer</Press>}
          after={h.code ? <Code code={h.code} /> : undefined} />
      )}
      {on && <div class="sg-list" id="remote-devices">{devices.map((d) => <DeviceRow d={d} />)}</div>}
    </Group>
  );
}
