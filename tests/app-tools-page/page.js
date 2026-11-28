// The app toolbox on the page: the Allow/Don't card an AI's tool call raises, the toolbox rows (one line per call,
// updated in place, with chips for the chats and projects it named), the counted summary a finished turn folds them
// into, links in the agent's words drawn as chips, and the page refreshing itself on projects_changed,
// settings_changed and theme_set. Runs against mock.py on 127.0.0.1.
const { chromium } = require("H:/node/node_modules/playwright");
const OUT = process.env.OUT || "", PORT = process.env.PORT || "8799", errs = [], out = {};
const ok = (name, cond, extra) => { out[name] = cond ? "PASS" : "FAIL " + JSON.stringify(extra === undefined ? null : extra); };
const U = "http://127.0.0.1:" + PORT + "/?t=x";
const A = "H:/games/obby-rush", PID = "p0123456789ab";

(async () => {
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 }, colorScheme: "dark", reducedMotion: "reduce" });
  const pg = await ctx.newPage();
  pg.on("pageerror", (e) => errs.push("pageerror: " + e.message + " " + (e.stack || "").split("\n").slice(1, 3).join(" | ")));
  const outside = [];
  pg.on("request", (r) => { if (!/^(http:\/\/127\.0\.0\.1:|data:|blob:|about:)/.test(r.url())) outside.push(r.url()); });
  pg.on("console", (m) => { if (m.type() === "error" && !/404|Failed to load resource/.test(m.text())) errs.push("console: " + m.text()); });
  const wait = (ms) => pg.waitForTimeout(ms);
  const posts = async (path) => (await pg.evaluate(() => fetch("/mock/posts").then((r) => r.json()))).filter((p) => p[0] === path);
  const gets = async (path) => (await pg.evaluate(() => fetch("/mock/gets").then((r) => r.json())))[path] || 0;
  const mock = (path, body) => pg.evaluate(([path, body]) => fetch(path, { method: "POST", body: JSON.stringify(body) }).then((r) => r.json()), [path, body]);
  const ev = (e) => mock("/mock/push", e);
  const text = (sel) => pg.evaluate((sel) => { const n = document.querySelector(sel); return n ? n.innerText.replace(/\s+/g, " ").trim() : null; }, sel);
  const texts = (sel) => pg.evaluate((sel) => [...document.querySelectorAll(sel)].map((n) => n.innerText.replace(/\s+/g, " ").trim()), sel);
  const until = async (f, ms) => { const end = Date.now() + (ms || 6000); for (;;) { const v = await f(); if (v || Date.now() > end) return v; await wait(120); } };
  const shot = async (name, sel) => {
    await wait(250);
    const box = await pg.evaluate((sel) => { const n = document.querySelector(sel); if (!n) return null; const r = n.getBoundingClientRect(); return { x: Math.max(0, r.x - 24), y: Math.max(0, r.y - 16), width: Math.min(1280, r.width + 48), height: r.height + 32 }; }, sel);
    if (box) await pg.screenshot({ path: OUT + name + ".png", caret: "hide", clip: box });
  };
  const theme = (mode) => pg.evaluate((mode) => setTheme({ id: "fridae", mode }), mode);
  try {
    await pg.request.post("http://127.0.0.1:" + PORT + "/mock/reset", { data: {} });
    await pg.goto(U);
    await until(() => pg.evaluate(() => typeof bootMark !== "undefined" && bootMark !== Infinity && agents.has("a1")), 8000);
    await pg.evaluate(() => { select("a1"); setView("chat"); });
    await until(() => pg.evaluate(() => current === "a1"));

    // ---- a toolbox call shows as its own row: the app sends no step line for it (agents.rs toolbox_step), so the
    // page draws every step it is sent ----
    await ev({ kind: "busy", agent: "a1", value: true });
    await ev({ kind: "tool", agent: "a1", label: "Reading files" });
    await until(() => pg.evaluate(() => document.querySelector("#chat-main").innerText.includes("Reading files")));
    const stepTexts = await texts("#chat-main .step");
    ok("steps_drawn_as_sent", stepTexts.length === 1 && stepTexts[0] === "Reading files", stepTexts);

    // ---- the card ----
    const CARD = { kind: "appcard", agent: "a1", card: "k1", title: "Rename project", text: "Rename \u201cObby Rush\u201d to \u201cLava Run\u201d. Its folder keeps its own name.", allow: "Rename it" };
    await ev(CARD);
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .ask.appcard")));
    const h4 = await text("#chat-main .ask.appcard h4"), para = await text("#chat-main .ask.appcard p"), btns = await texts("#chat-main .ask.appcard .act button");
    ok("card_pending_shows_title_text_and_two_buttons", h4 === "Rename project" && para === CARD.text && JSON.stringify(btns) === JSON.stringify(["Rename it", "Don't"]), [h4, para, btns]);
    ok("card_allow_is_the_main_button", await pg.evaluate(() => { const b = document.querySelectorAll("#chat-main .ask.appcard .act button"); return b[0].classList.contains("main") && !b[1].classList.contains("main"); }));
    ok("card_says_waiting", await pg.evaluate(() => agents.get("a1").now === "Waiting for you" && agents.get("a1").last === "Needs you"), await pg.evaluate(() => [agents.get("a1").now, agents.get("a1").last]));
    await shot("card-pending-dark", "#chat-main .ask.appcard");
    await theme("light"); await shot("card-pending-light", "#chat-main .ask.appcard"); await theme("dark");

    await pg.click("#chat-main .ask.appcard .act button.main");
    const sent = await until(async () => (await posts("/api/app-card")).length > 0);
    const ans = sent && (await posts("/api/app-card"))[0][1];
    ok("allow_posts", !!ans && ans.agent === "a1" && ans.card === "k1" && ans.answer === "allow", ans);
    ok("allow_disables_both_buttons", await pg.evaluate(() => [...document.querySelectorAll("#chat-main .ask.appcard .act button")].every((b) => b.disabled)));
    await ev({ kind: "appcard_done", agent: "a1", card: "k1", title: "Rename project", outcome: "allowed" });
    const line = await until(() => text("#chat-main .appcard-done"));
    ok("allow_posts_and_collapses", line === "Rename project \u00b7 allowed" && !(await pg.evaluate(() => !!document.querySelector("#chat-main .ask.appcard"))), line);
    await shot("card-answered-dark", "#chat-main .turn");
    await theme("light"); await shot("card-answered-light", "#chat-main .turn"); await theme("dark");
    ok("card_back_to_working", await pg.evaluate(() => agents.get("a1").last !== "Needs you"), await pg.evaluate(() => agents.get("a1").last));

    // ---- Don't, and a card the run closed ----
    await ev({ kind: "appcard", agent: "a1", card: "k2", title: "Remove project", text: "Take \u201cLava Run\u201d off your projects. The folder and its chats stay.", allow: "Remove it" });
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .ask.appcard")));
    await pg.click("#chat-main .ask.appcard .act button:not(.main)");
    await until(async () => (await posts("/api/app-card")).length > 1);
    ok("dont_posts_decline", (await posts("/api/app-card"))[1][1].answer === "decline", (await posts("/api/app-card"))[1]);
    await ev({ kind: "appcard_done", agent: "a1", card: "k2", title: "Remove project", outcome: "declined" });
    await ev({ kind: "appcard", agent: "a1", card: "k3", title: "Change the look", text: "Switch Fridae to Moss.", allow: "Change it" });
    await ev({ kind: "appcard_done", agent: "a1", card: "k3", title: "Change the look", outcome: "cancelled" });
    await ev({ kind: "appcard", agent: "a1", card: "k4", title: "Change a setting", text: "Turn on \u201cLead may pick the AI\u201d.", allow: "Turn it on" });
    await ev({ kind: "appcard_done", agent: "a1", card: "k4", title: "Change a setting", outcome: "timed_out" });
    await until(async () => (await texts("#chat-main .appcard-done")).length === 4);
    const lines = await texts("#chat-main .appcard-done");
    ok("outcome_words", JSON.stringify(lines) === JSON.stringify(["Rename project \u00b7 allowed", "Remove project \u00b7 declined", "Change the look \u00b7 closed", "Change a setting \u00b7 timed out"]), lines);

    // ---- toolbox rows ----
    const SUM = { verb: "Updated", one: "project", many: "projects" };
    await ev({ kind: "apptool", agent: "a1", call: "t1", tool: "project_update", status: "running", label: "Updating a project", summary: SUM, links: [] });
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .app-tool")));
    const run1 = await texts("#chat-main .app-tool");
    await ev({ kind: "apptool", agent: "a1", call: "t1", tool: "project_update", status: "done", label: "Updated a project", summary: SUM, links: [{ kind: "project", id: PID, title: "Lava Run" }] });
    await until(async () => ((await texts("#chat-main .app-tool"))[0] || "").startsWith("Updated"));
    const done1 = await texts("#chat-main .app-tool");
    ok("apptool_running_then_done_updates_one_line", JSON.stringify(run1) === JSON.stringify(["Updating a project"]) && done1.length === 1 && done1[0] === "Updated a project Lava Run", [run1, done1]);
    ok("apptool_has_the_mark", await pg.evaluate(() => { const i = document.querySelector("#chat-main .app-tool img"); return !!i && /\/logo\.png$/.test(i.src) && i.getAttribute("alt") === "" && Math.round(i.getBoundingClientRect().width) === 12; }));
    ok("apptool_chip", await pg.evaluate((pid) => !!document.querySelector('#chat-main .app-tool a.chip.app-link[data-project="' + pid + '"]'), PID));

    await ev({ kind: "apptool", agent: "a1", call: "t2", tool: "chat_update", status: "running", label: "Renaming a chat", summary: { verb: "Renamed", one: "chat", many: "chats" }, links: [] });
    await ev({ kind: "apptool", agent: "a1", call: "t2", tool: "chat_update", status: "failed", label: "Failed to rename a chat", summary: { verb: "Renamed", one: "chat", many: "chats" }, links: [] });
    await ev({ kind: "apptool", agent: "a1", call: "t3", tool: "project_delete", status: "declined", label: "Declined to remove a project", summary: { verb: "Removed", one: "project", many: "projects" }, links: [] });
    await until(async () => (await texts("#chat-main .app-tool")).length === 3);
    await wait(200);   /* the page's reduced-motion transitions (1 ms) end */
    const fd = await pg.evaluate(() => {
      const bad = getComputedStyle(document.documentElement).getPropertyValue("--bad-ink").trim();
      const probe = document.createElement("span"); probe.style.color = "var(--bad-ink)"; document.body.appendChild(probe); const want = getComputedStyle(probe).color; probe.remove();
      return [...document.querySelectorAll("#chat-main .app-tool.failed, #chat-main .app-tool.declined")].map((n) => [n.innerText.trim(), getComputedStyle(n.querySelector(".at-label")).color === want, !!bad]);
    });
    ok("failed_and_declined_lines", fd.length === 2 && fd[0][0] === "Failed to rename a chat" && fd[1][0] === "Declined to remove a project" && fd.every((x) => x[1]), fd);
    await shot("apptool-rows-dark", "#chat-main .turn");

    // ---- chips in the agent's words ----
    await ev({ kind: "text", agent: "a1", text: "Done. See [Lava stage notes](fridae://chat/a2) in [Obby \\[old\\] Rush](fridae://project/" + PID + "). Not [this](fridae://other/zz), nor [Gone chat](fridae://chat/c999-9), nor `[code](fridae://chat/a2)`." });
    await ev({ kind: "done", agent: "a1" });
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .prose a.app-link")));
    const chips = await pg.evaluate(() => [...document.querySelectorAll("#chat-main .prose .app-link")].map((n) => [n.tagName, n.className, n.dataset.chat || n.dataset.project || "", n.textContent]));
    const prose = await pg.evaluate(() => [...document.querySelectorAll("#chat-main .prose")].pop().innerText);
    ok("chips_drawn", chips.length === 3 && chips[0][0] === "A" && chips[0][2] === "a2" && chips[0][3] === "Lava stage notes" && chips[1][2] === PID && chips[1][3] === "Obby [old] Rush" && chips[2][0] === "SPAN" && !/chip/.test(chips[2][1]) && chips[2][3] === "Gone chat", chips);
    ok("other_fridae_links_are_plain", prose.includes("Not this, nor") && !prose.includes("fridae://other") && prose.includes("[code](fridae://chat/a2)"), prose);
    await shot("chips-dark", "#chat-main .turn");
    await pg.click('#chat-main .prose a.app-link[data-chat="a2"]');
    const toChat = await until(() => pg.evaluate(() => current === "a2"));
    await pg.evaluate(() => { select("a1"); setView("chat"); }); await wait(300);
    await pg.click('#chat-main .prose a.app-link[data-project="' + PID + '"]');
    const toProj = await until(() => pg.evaluate((a) => curProj === a && view === "chat", A));
    ok("chips_open_the_chat_and_the_project", !!toChat && !!toProj, await pg.evaluate(() => [current, curProj, view]));
    await pg.evaluate(() => { select("a1"); setView("chat"); }); await wait(300);
    const before = await pg.evaluate(() => [current, curProj, view]);
    await pg.click('#chat-main .prose .app-link:not(a)'); await wait(300);
    ok("unknown_chip_does_nothing", JSON.stringify(before) === JSON.stringify(await pg.evaluate(() => [current, curProj, view])), [before, await pg.evaluate(() => [current, curProj, view])]);

    // ---- a finished turn folds the same done calls into one counted line ----
    await ev({ kind: "user", agent: "a1", text: "Rename both" });
    await ev({ kind: "busy", agent: "a1", value: true });
    const RS = { verb: "Renamed", one: "project", many: "projects" };
    await ev({ kind: "apptool", agent: "a1", call: "g1", tool: "project_update", status: "done", label: "Renamed a project", summary: RS, links: [{ kind: "project", id: PID, title: "Lava Run" }] });
    await ev({ kind: "apptool", agent: "a1", call: "g2", tool: "project_update", status: "done", label: "Renamed a project", summary: RS, links: [{ kind: "project", id: "p999999999999", title: "Castle" }] });
    await until(async () => (await texts("#chat-main .turn:last-of-type .app-tool")).length >= 2);
    const beforeDone = await pg.evaluate(() => { const t = [...document.querySelectorAll("#chat-main .turn")].pop(); return t.querySelectorAll(".app-tool").length; });
    await ev({ kind: "done", agent: "a1" });
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .app-tool.group")));
    const g = await text("#chat-main .app-tool.group");
    const shownBefore = await pg.evaluate(() => document.querySelectorAll("#chat-main .app-tool.group .app-tool").length);
    await pg.click("#chat-main .app-tool.group > button");
    const opened = await until(() => pg.evaluate(() => document.querySelectorAll("#chat-main .app-tool.group .app-tool").length === 2));
    ok("grouped_summary_after_done", beforeDone === 2 && g === "Renamed 2 projects" && shownBefore === 0 && !!opened, [beforeDone, g, shownBefore]);
    ok("earlier_turn_not_grouped", (await pg.evaluate(() => document.querySelectorAll("#chat-main .app-tool.group").length)) === 1);
    await shot("grouped-open-dark", "#chat-main .app-tool.group");
    ok("unknown_project_chip_is_plain", await pg.evaluate(() => { const n = [...document.querySelectorAll("#chat-main .app-tool.group .app-link")].find((x) => x.textContent === "Castle"); return !!n && n.tagName === "SPAN"; }));

    // ---- a card in a chat put back from its log, never answered ----
    await ev({ kind: "agent_new", agent: "a3", name: "Old chat", ai: "claude", restored: true });
    await ev({ kind: "project", agent: "a3", path: A, name: "Obby Rush", type: "game", bible: true });
    await ev({ kind: "user", agent: "a3", text: "Remove the project" });
    await ev({ kind: "appcard", agent: "a3", card: "k9", title: "Remove project", text: "Take it off.", allow: "Remove it" });
    await ev({ kind: "done", agent: "a3" });
    await until(() => pg.evaluate(() => agents.has("a3")));
    await pg.evaluate(() => { select("a3"); setView("chat"); });
    const replayed = await until(() => text("#chat-main .appcard-done"));
    ok("replayed_pending_card_reads_closed", replayed === "Remove project \u00b7 closed" && !(await pg.evaluate(() => !!document.querySelector("#chat-main .ask.appcard"))), replayed);
    await pg.evaluate(() => { select("a1"); setView("chat"); });

    // ---- a chat's own project is known by the id its "project" event carries (projectId: the event's id is its number) ----
    const B = "H:/games/castle", PB = "pfedcba987654";
    await ev({ kind: "agent_new", agent: "a4", name: "Castle chat", ai: "claude" });
    await ev({ kind: "project", agent: "a4", path: B, name: "Castle", type: "game", bible: false, projectId: PB });
    await ev({ kind: "text", agent: "a4", text: "Renamed [Castle](fridae://project/" + PB + ")." });
    await ev({ kind: "done", agent: "a4" });
    await until(() => pg.evaluate(() => agents.has("a4")));
    await pg.evaluate(() => { select("a4"); setView("chat"); });
    await until(() => pg.evaluate(() => !!document.querySelector("#chat-main .prose .app-link")));
    const own = await pg.evaluate(() => { const n = document.querySelector("#chat-main .prose .app-link"); return n ? [n.tagName, n.dataset.project || ""] : null; });
    ok("own_project_chip_uses_projectId", !!own && own[0] === "A" && own[1] === PB && await pg.evaluate((pb) => agents.get("a4").projectId === pb, PB), own);
    await pg.evaluate(() => { select("a1"); setView("chat"); });

    // ---- the page refreshes itself ----
    await mock("/mock/projects", { projects: [{ path: A, name: "Lava Run", type: "game", bible: true, id: PID }] });
    await ev({ kind: "projects_changed" });
    const renamed = await until(async () => ((await text('.pbox[data-box="' + A + '"] .p-name')) || "").includes("Lava Run"), 4000);
    ok("projects_changed_refreshes_the_sidebar", !!renamed, await text('.pbox[data-box="' + A + '"] .p-name'));

    // the page asks for its status every 4 s by itself: right after one of those, the event must bring another at once
    const n0 = await gets("/api/status");
    await until(async () => (await gets("/api/status")) > n0, 5000);
    const n1 = await gets("/api/status");
    await ev({ kind: "settings_changed", keys: ["chat_ai"] });
    const refetched = await until(async () => (await gets("/api/status")) > n1, 1500);
    ok("settings_changed_refetches_status", !!refetched, [n1, await gets("/api/status")]);
    const n2 = await gets("/api/status");
    await ev({ kind: "accounts_changed" });
    ok("accounts_changed_refetches_status", !!(await until(async () => (await gets("/api/status")) > n2, 1500)));

    const reportsBefore = (await posts("/api/theme-now")).length;
    ok("theme_reported_at_load", reportsBefore > 0, reportsBefore);
    await ev({ kind: "theme_set", themeId: "moss" });
    const moss = await until(() => pg.evaluate(() => document.documentElement.dataset.theme === "moss"));
    const rep = await until(async () => (await posts("/api/theme-now")).find((p) => p[1].id === "moss"));
    ok("theme_set_applies_and_reports", !!moss && !!rep && rep[1].mode === "dark" && await pg.evaluate(() => fridae.store.get().theme.id === "moss" && JSON.parse(localStorage.getItem("theme")).id === "moss"), [await pg.evaluate(() => document.documentElement.dataset.theme), rep]);
    await ev({ kind: "theme_set", appearance: "light" });
    ok("theme_set_appearance_only", !!(await until(() => pg.evaluate(() => document.documentElement.dataset.theme === "moss" && document.documentElement.dataset.appearance === "light"))));
    await ev({ kind: "theme_set", themeId: "neon", appearance: "sideways" });
    await wait(500);
    ok("theme_set_ignores_unknown", await pg.evaluate(() => document.documentElement.dataset.theme === "moss" && document.documentElement.dataset.appearance === "light"), await pg.evaluate(() => [document.documentElement.dataset.theme, document.documentElement.dataset.appearance]));
    ok("nothing_left_127_0_0_1", outside.length === 0, outside);
  } catch (e) { out.CRASH = String(e && e.stack || e); }
  ok("no_console_errors", errs.length === 0, errs);
  out.errors = errs;
  console.log(JSON.stringify(out, null, 1));
  await b.close();
})();
