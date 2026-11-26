/* An on/off switch. `label` draws the button kind (a track and a knob, its words kept for screen readers); without it
   the switch is a checkbox the row's title labels. After a change it is drawn again from what the app then says. */
import { useAgain } from "../store";
type Props = { id?: string; on: boolean; onChange(on: boolean): unknown; label?: string; hidden?: boolean };
export function Switch(p: Props) {
  const again = useAgain();
  const change = async (on: boolean) => { try { await p.onChange(on); } finally { again(); } };
  if (p.label !== undefined) return (
    <button class="switch" id={p.id} role="switch" aria-checked={p.on ? "true" : "false"} aria-label={p.label} style={p.hidden ? "display: none;" : undefined} onClick={() => change(!p.on)}>
      <span class="track"><span class="knob"></span></span><span class="swl">{p.label}</span>
    </button>
  );
  return <input type="checkbox" class="sw" role="switch" id={p.id} checked={p.on} onChange={(e) => change((e.target as HTMLInputElement).checked)} />;
}
