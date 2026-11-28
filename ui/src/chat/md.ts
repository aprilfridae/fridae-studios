/* A small, safe markdown renderer: everything is escaped first. */
export const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
/* A link to one of Fridae's own chats or projects, as the app toolbox writes them, `[title](fridae://chat/<id>)` or
   `[name](fridae://project/<id>)` (a `[`, `]` or `\` in the title written with a `\` before it): a chip that opens
   it (appTools.ts wires the press, and leaves plain the ones that lead nowhere the page knows). Any other `fridae:`
   link is its title in plain words. Applied to text already escaped; never inside `code`. */
const LINK = /\[((?:\\.|[^\]\\])*)\]\((fridae:[^)\s]*)\)/g;
function links(s: string) {
  return s.replace(LINK, (_all, title: string, url: string) => {
    const t = title.replace(/\\([[\]\\])/g, "$1"), m = /^fridae:\/\/(chat|project)\/([\w.~-]{1,120})$/.exec(url);
    return m ? '<a class="chip app-link" data-' + m[1] + '="' + m[2] + '" href="fridae://' + m[1] + '/' + m[2] + '">' + t + "</a>" : t;
  });
}
export function inline(s: string) {
  return esc(s).split(/(`[^`]+`)/).map((x, i) => (i % 2 ? "<code>" + x.slice(1, -1) + "</code>" : links(x).replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>"))).join("");
}
export function md(src: string) {
  const lines = src.replace(/\r/g, "").split("\n"); let out = "", i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (/^```/.test(line.trim())) { const code: string[] = []; i++; while (i < lines.length && !/^```/.test(lines[i].trim())) code.push(lines[i++]); i++; out += "<pre><code>" + esc(code.join("\n")) + "</code></pre>"; continue; }
    if (/^#{1,4}\s/.test(line)) { out += "<h3>" + inline(line.replace(/^#{1,4}\s+/, "")) + "</h3>"; i++; continue; }
    if (/^\s*\|.*\|\s*$/.test(line) && i + 1 < lines.length && /^\s*\|[\s:|-]+\|\s*$/.test(lines[i + 1])) {
      const cells = (l: string) => l.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      let t = "<table><tr>" + cells(line).map((c) => "<th>" + inline(c) + "</th>").join("") + "</tr>"; i += 2;
      while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) t += "<tr>" + cells(lines[i++]).map((c) => "<td>" + inline(c) + "</td>").join("") + "</tr>";
      out += t + "</table>"; continue;
    }
    const bullet = /^\s*[-*•]\s+/, num = /^\s*\d+[.)]\s+/;
    if (bullet.test(line) || num.test(line)) {
      const ordered = num.test(line), re = ordered ? num : bullet; let items = "";
      while (i < lines.length && re.test(lines[i])) items += "<li>" + inline(lines[i++].replace(re, "")) + "</li>";
      out += (ordered ? "<ol>" : "<ul>") + items + (ordered ? "</ol>" : "</ul>"); continue;
    }
    if (!line.trim()) { i++; continue; }
    const para = [line]; i++;
    while (i < lines.length && lines[i].trim() && !/^(```|#{1,4}\s|\s*[-*•]\s+|\s*\d+[.)]\s+|\s*\|)/.test(lines[i])) para.push(lines[i++]);
    out += "<p>" + inline(para.join(" ")) + "</p>";
  }
  return out;
}
/* sentence case for words that arrive in lower case */
export function cap(s: string) { return s ? String(s).charAt(0).toUpperCase() + String(s).slice(1) : s; }
export const extOf = (name: string) => { const m = /\.([A-Za-z0-9]{1,5})$/.exec(name || ""); return m ? m[1].toLowerCase() : "file"; };
