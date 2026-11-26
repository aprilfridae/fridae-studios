/* An account's limits as bars: each filled to what is left, with the even-pace line in a gap cut from it (kept
   inside at 0 and 100), and when it starts again. `compact` is the pop-up's smaller drawing. */
import { useEffect, useState } from "preact/hooks";
import { PACE, formatResetsAt, formatResetsIn, paceOf, paceRemainingPercent, remainingPercent, type Win } from "./limits";

/* the time in seconds, read again once a minute: what is drawn from it changes no more often, so a tooltip stays put */
export function useNow() {
  const [, setMin] = useState(0);
  useEffect(() => { const t = setInterval(() => setMin(Math.floor(Date.now() / 60000)), 5000); return () => clearInterval(t); }, []);
  return Date.now() / 1000;
}
function Bar({ w, now }: { w: Win; now: number }) {
  const remaining = remainingPercent(w), timeLeft = paceRemainingPercent(w, now), resetsIn = formatResetsIn(w, now), resetsAt = formatResetsAt(w, now);
  const said = w.label + ": " + remaining + "% left" + (timeLeft === null ? "" : ", " + timeLeft + "% of the time left") + (resetsIn ? ", " + resetsIn : "");
  const tip = remaining + "% left" + (timeLeft === null ? "" : " · " + timeLeft + "% of the time left. The line is where even spending would be") + "." + (resetsAt ? " Resets " + resetsAt + (resetsIn ? " · " + resetsIn : "") + "." : "");
  let gap: string | undefined, at = "";
  if (timeLeft !== null) {
    at = "clamp(3px, " + timeLeft + "%, calc(100% - 3px))";
    const left = "calc(" + at + " - 3px)", right = "calc(" + at + " + 3px)", near = "-1px", far = "calc(100% + 1px)";
    gap = "clip-path: polygon(" + [near + " " + near, left + " " + near, left + " " + far, right + " " + far, right + " " + near, far + " " + near, far + " " + far, near + " " + far].join(", ") + ")";
  }
  return (
    <div class={"lw-bar" + (remaining <= 20 ? " low" : "")} role="img" tabindex={0} aria-label={said} title={tip}>
      <div class="lw-clip" style={gap}><div class="lw-track"></div>{remaining > 0 && <div class="lw-fill" style={"width: " + remaining + "%"}></div>}</div>
      {timeLeft !== null && <span class="lw-tick" aria-hidden="true" style={"left: " + at}></span>}
    </div>
  );
}
function Pace({ pace }: { pace: string }) {
  const [label, d] = PACE[pace];
  return <span class="lw-pace" role="img" aria-label={label} title={label}><svg width="14" height="14" viewBox="0 0 15 15" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{d.split("|").map((x) => <path d={x} />)}</svg></span>;
}
export function LimitWindows({ windows, now, compact }: { windows: Win[]; now: number; compact?: boolean }) {
  return (
    <div class={"lw" + (compact ? " compact" : "")}>
      {windows.map((w) => {
        const pace = paceOf(w, now);
        return [
          <span class="lw-name"><span>{w.label}</span><b>{remainingPercent(w) + "% left"}</b></span>,
          <Bar w={w} now={now} />,
          <span class="lw-when">{pace && <Pace pace={pace} />}<span>{formatResetsIn(w, now)}</span></span>,
        ];
      })}
    </div>
  );
}
