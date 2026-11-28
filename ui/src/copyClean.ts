/* What is copied from the window never carries its token. Pictures (a result, an attachment, a promo or Review
   picture) load with "?t=<token>" after their address, and a copy of a message with one in it would put that address
   on the clipboard as HTML. On any copy of the page's own content, the HTML put on the clipboard has every `t` taken
   out of its addresses; the plain text is left as the browser makes it. A copy inside a text box is the browser's. */
import { TOKEN } from "./api";

const ATTRS = ["src", "href", "srcset", "poster", "data-src"];
/* an address without its `t` (and without the token anywhere else in it) */
export function untoken(url: string): string {
  let out = url.replace(/([?&])t=[^&#\s,]*&?/g, (_m, sep: string) => sep).replace(/[?&](#|$|\s|,)/g, "$1").replace(/[?&](?=[,\s])/g, "");
  if (TOKEN) out = out.split(TOKEN).join("");
  return out;
}
const tokened = (v: string) => /[?&]t=/.test(v) || (!!TOKEN && v.includes(TOKEN));

export function installCopyClean() {
  document.addEventListener("copy", (ev: ClipboardEvent) => {
    const a = document.activeElement as HTMLElement | null;
    if (a && (a.tagName === "INPUT" || a.tagName === "TEXTAREA" || a.isContentEditable)) return;
    const sel = window.getSelection();
    if (!sel || !sel.rangeCount || sel.isCollapsed || !ev.clipboardData) return;
    const box = document.createElement("div");
    for (let i = 0; i < sel.rangeCount; i++) box.appendChild(sel.getRangeAt(i).cloneContents());
    if (!tokened(box.innerHTML)) return;
    box.querySelectorAll("*").forEach((n) => { for (const k of ATTRS) { const v = n.getAttribute(k); if (v && tokened(v)) n.setAttribute(k, untoken(v)); } });
    let html = box.innerHTML;
    if (TOKEN) html = html.split(TOKEN).join("");
    ev.clipboardData.setData("text/html", html);
    ev.clipboardData.setData("text/plain", sel.toString());
    ev.preventDefault();
  });
}
