/* Settings, Experts: what the Lead may do, how many experts work at once, and one row per roster expert with its AI
   and model (both "Same as the Lead" unless chosen), saved in the app's config as expert_ai. Only AIs installed here
   are offered; a saved choice of one that is gone still shows. */
import { useState } from "preact/hooks";
import { api } from "../api";
import { cell, store, useStore } from "../store";
import { host } from "../host";
import { post } from "../util";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Switch } from "../ui/Switch";
import { Segmented } from "../ui/Segmented";
import { whenShown } from "./state";

const experts = cell<{ list: any; asked: boolean }>({ list: null, asked: false });
async function loadExperts(body?: unknown) {
  const r = await api("/api/experts", body || {});
  if (r && r.ok === false) alert(r.error || "Could not save that.");
  experts.set({ list: (r && r.list) || experts.get().list, asked: true });
}
whenShown(["experts"], () => loadExperts());

function ExpertRow({ x, list }: { x: any; list: any }) {
  const [q, setQ] = useState("");
  const ais: any[] = list.ais || [], pick = (list.table || {})[x.id] || {}, ai: string = pick.ai || "", info = ais.find((a) => a.id === ai);
  const models: [string, string][] = (info && info.models) || [], dflt = info && info.default, dn = (models.find((m) => m[0] === dflt) || [])[1];
  const note = (ai && (list.gaps || {})[ai]) || "", job = (x.job || "").trim();
  /* the model's choices: its default, each model, and a saved one that is not offered now */
  const all: [string, string][] = !ai ? [["", "Same as the Lead"]] : [["", "Its default" + (dn || dflt ? " (" + (dn || dflt) + ")" : "")] as [string, string]].concat(models)
    .concat(pick.model && !models.some((m) => m[0] === pick.model) ? [[pick.model, pick.model + " (not offered now)"]] : []);
  const word = q.trim().toLowerCase(), keep = pick.model || "";
  return (
    <div class="xp-tr"><b>{x.name}</b>
      {/* a long list (Cursor offers hundreds) gets a filter; options are left out rather than hidden: WebKit's native menu ignores hidden options */}
      {ai && models.length > 12 && <input class="r-sel xp-filter" type="search" placeholder="Filter models" aria-label={x.name + ": filter models"} value={q} onInput={(e) => setQ((e.target as HTMLInputElement).value)} />}
      <select class="r-sel" aria-label={x.name + ": AI"} value={ai} onChange={(e) => loadExperts({ expert: x.id, ai: (e.target as HTMLSelectElement).value, model: "" })}>
        <option value="">Same as the Lead</option>
        {ais.filter((a) => a.installed || a.id === ai).map((a) => <option value={a.id}>{a.name + (a.installed ? "" : " (not installed)")}</option>)}
      </select>
      <select class="r-sel" aria-label={x.name + ": model"} disabled={!ai} value={keep} onChange={(e) => loadExperts({ expert: x.id, ai, model: (e.target as HTMLSelectElement).value })}>
        {all.filter(([id, name], i) => i === 0 || !word || id === keep || (name + " " + id).toLowerCase().includes(word)).map(([id, name]) => <option value={id}>{name}</option>)}
      </select>
      <small>{job + (job && !/[.!?]$/.test(job) ? "." : "") + (note ? " " + note : "")}</small>
    </div>
  );
}
/* Most experts working at once, 1 to 8 (4 unless changed). The app's agent limit is 8. */
function Ceiling() {
  const status = useStore((s) => s.status), [want, setWant] = useState(0), [note, setNote] = useState("");
  const v = want || (typeof status.swarm_ceiling === "number" ? status.swarm_ceiling : 4);
  const pick = async (n: number) => {
    if (want) return;
    setWant(n);
    const r = await post("/api/swarm", { action: "ceiling", value: n });
    if (r && r.ok) { if (typeof r.ceiling === "number") host.swarmCeiling(r.ceiling); setNote(""); }
    else setNote((r && r.error) || "Could not change it.");
    setWant(0);
  };
  return (
    <SettingsRow id="set-swarm" find="swarm ceiling parallel at once" title="Most experts working at once" titleId="sw-cap-title" desc="1 means one at a time; more get it done sooner and use up your plan's usage faster."
      ctl={<Segmented id="sw-cap" labelledBy="sw-cap-title" pressed busy={!!want} value={v} onPick={pick}
        options={[1, 2, 3, 4, 5, 6, 7, 8].map((n) => ({ value: n, label: String(n), say: n === 1 ? "One at a time" : n + " at once", data: { "data-n": String(n) } }))} />}>
      <p class="sr-status" id="sw-cap-note" role="status" hidden={!note}>{note}</p>
    </SettingsRow>
  );
}
export function Experts() {
  const status = useStore((s) => s.status), { list, asked } = experts.use(), known = "ai" in status;
  const flip = (path: string) => async (on: boolean) => { await api(path, { on }); await host.refresh(); };
  return (
    <>
      <p class="st-lede">In a studio run, the Lead hands each part of the project to an expert. Pick an AI and model for any of them; the rest run on the Lead's.</p>
      <Group id="set-lead" title="The Lead and its experts">
        <SettingsRow id="set-lead-picks" title="The Lead may pick an AI for a job" labelFor="xp-lead-picks" desc="Now and then the Lead may start an expert on another AI or model; your rows here always win."
          more="Now and then the Lead may start an expert on another AI or model; your rows here always win. An expert on Same as the Lead uses the Lead's model, so a Lead on a big model uses up that plan faster. Once you pick an AI, the expert uses that AI's default model unless you pick one. Only AIs installed on this computer are offered."
          ctl={<Switch id="xp-lead-picks" on={!!list && list.lead_may_pick !== false} onChange={(on) => loadExperts({ lead_may_pick: on })} />} />
        <SettingsRow id="set-summaries" title="Plain-English summary of each expert's work" labelFor="xp-summaries" desc="When an expert finishes, Fridae writes a short line of what it changed in your project."
          more="When an expert finishes, Fridae writes a short, plain-English line of what it changed in your project, so you can follow along without reading reports or code. It is a small job on your OpenCode plan (Settings, Connections). Off: you see the expert's own one-line note instead."
          ctl={<Switch id="xp-summaries" on={known && status.expert_summaries !== false} onChange={flip("/api/expert-summaries")} />} />
        <SettingsRow id="set-strong-lead" title="Keep the Lead on a strong model" labelFor="xp-strong-lead" desc="A run's Lead plans and splits the work, so it never runs on DeepSeek or GLM. Turn off to let it."
          ctl={<Switch id="xp-strong-lead" on={known && status.strong_lead !== false} onChange={flip("/api/strong-lead")} />} />
        <Ceiling />
      </Group>
      <Group id="set-expert-ais" find="model roster" title="Each expert's AI">
        <div id="xp-table" class="sg-list su-block">{list ? (list.experts || []).map((x: any) => <ExpertRow x={x} list={list} />) : asked ? "Loading..." : null}</div>
      </Group>
    </>
  );
}
