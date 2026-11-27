// Remote control on the page: the machine chip, its menu, pairing, a chat and a project on another computer, and an
// offline computer. Runs against mock.py (which follows the remote-control contract) on 127.0.0.1.
const { chromium } = require("H:/node/node_modules/playwright");
const OUT = process.env.OUT || "", PORT = process.env.PORT || "8798", NOW = +process.env.MOCK_NOW || Date.now() / 1000, SHOTS = process.env.SHOTS !== "0", errs = [], out = {};
const ok = (name, cond, extra) => { out[name] = cond ? "PASS" : "FAIL " + JSON.stringify(extra === undefined ? null : extra); };
const U = "http://127.0.0.1:" + PORT + "/?t=x";
const CASTLE = "m1~D:/games/castle";

(async () => {
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 }, colorScheme: "dark", reducedMotion: "reduce" });
  const pg = await ctx.newPage();
  pg.on("pageerror", (e) => errs.push("pageerror: " + e.message + " " + (e.stack || "").split("\n").slice(1, 3).join(" | ")));
  pg.on("console", (m) => { if (m.type() === "error" && !/404|502|Failed to load resource/.test(m.text())) errs.push("console: " + m.text()); });
  const wait = (ms) => pg.waitForTimeout(ms);
  const posts = async (path) => (await pg.evaluate(() => fetch("/mock/posts").then((r) => r.json()))).filter((p) => p[0] === path);
  const last = async (path) => { const x = (await posts(path)).pop(); return x && x[1]; };
  const mock = (path, body) => pg.evaluate(([path, body]) => fetch(path, { method: "POST", body: JSON.stringify(body) }), [path, body]);
  const text = (sel) => pg.evaluate((sel) => { const n = document.querySelector(sel); return n ? n.innerText.replace(/\s+/g, " ").trim() : null; }, sel);
  const vis = (sel) => pg.evaluate((sel) => { const n = document.querySelector(sel); return !!n && !!n.getClientRects().length && getComputedStyle(n).visibility !== "hidden"; }, sel);
  const box = (key) => '.pbox[data-box="' + key + '"]';
  const shot = async (name, clip) => { if (!SHOTS) return; await wait(250); await pg.screenshot({ path: OUT + name + ".png", caret: "hide", clip }); };
  const until = async (f, ms) => { const end = Date.now() + (ms || 8000); for (;;) { const v = await f(); if (v || Date.now() > end) return v; await wait(150); } };
  try {
    await pg.request.post("http://127.0.0.1:" + PORT + "/mock/reset", { data: {} });
    await pg.goto(U);
    await until(() => pg.evaluate(() => typeof current !== "undefined" && current === "a1"), 6000);
    await until(() => vis(box(CASTLE)), 6000);

    // ---- the chip, on this computer's chat ----
    ok("r01 chip says This PC", (await text("#machine")) === "This PC", await text("#machine"));
    ok("r02 chip is left of the AI chip", await pg.evaluate(() => { const a = document.getElementById("machine"), m = document.getElementById("model"); return !!a && !!m && a.getBoundingClientRect().right <= m.getBoundingClientRect().left; }));
    await shot("01-chip-local", { x: 232, y: 700, width: 1048, height: 160 });

    // ---- the other computer's projects and chats in the sidebar ----
    ok("r03 remote project box, labelled", (await text(box(CASTLE) + " .p-where")) === "Desktop", await text(box(CASTLE)));
    ok("r04 remote project name", (await text(box(CASTLE) + " .p-name b")) === "Castle", await text(box(CASTLE)));
    ok("r05 its own No project box", (await text(box("m1~") + " .p-where")) === "Desktop" && (await text(box("m1~") + " .p-name b")) === "No project", await text(box("m1~")));
    ok("r06 this computer's projects unchanged", (await text(box("H:/games/obby-rush") + " .p-name")) === "Obby Rush", await text(box("H:/games/obby-rush")));
    await shot("02-sidebar-labels", { x: 0, y: 40, width: 260, height: 360 });

    // ---- picking the other computer on an empty page: a new chat there, the AI chip reads its AIs ----
    await pg.evaluate(() => { document.querySelector('.proj[data-k="p:H:/games/obby-rush"] .p-new').click(); }); await wait(400);
    ok("r11 a draft on this computer", await pg.evaluate(() => current === null && draftOn && curProj === "H:/games/obby-rush"));
    // ---- the chip's menu, on an empty page ----
    await pg.click("#machine"); await wait(250);
    ok("r07 menu open", await vis("#machine-menu"));
    const rows = await pg.$$eval("#machine-menu .opt", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r08 menu rows", JSON.stringify(rows) === JSON.stringify(["This PC", "Desktop", "Pair a computer…"]), rows);
    ok("r09 This PC ticked", (await pg.getAttribute("#machine-menu .opt >> nth=0", "aria-checked")) === "true");
    ok("r10 online dot", await pg.evaluate(() => !!document.querySelector('#machine-menu [data-m="m1"] .dot.ok')));
    await shot("03-menu", { x: 232, y: 560, width: 700, height: 300 });

    await pg.click('#machine-menu [data-m="m1"]'); await wait(800);
    ok("r12 draft moved to the other computer", await pg.evaluate(() => draftOn && curProj === "m1~"), await pg.evaluate(() => [draftOn, curProj]));
    ok("r13 chip says Desktop", (await text("#machine")) === "Desktop", await text("#machine"));
    ok("r14 AI chip reads that computer's AIs", /^OpenCode/.test(await text("#model-name")), await text("#model-name"));
    await pg.click("#machine"); await wait(200);
    await pg.click("#machine-menu .opt >> nth=0"); await wait(500);
    ok("r15 back to This PC: the project it came from", await pg.evaluate(() => draftOn && curProj === "H:/games/obby-rush"), await pg.evaluate(() => [draftOn, curProj]));
    ok("r16 AI chip back to this computer's", /^Claude/.test(await text("#model-name")), await text("#model-name"));

    // ---- "+" on the other computer's project: a chat there; the first message goes through /m/m1 with bare ids ----
    await pg.evaluate((k) => { document.querySelector('.proj[data-k="p:' + k + '"] .p-new').click(); }, CASTLE); await wait(700);
    ok("r17 + on a remote project", await pg.evaluate((k) => draftOn && curProj === k, CASTLE), await pg.evaluate(() => [draftOn, curProj]));
    ok("r18 chip says Desktop there", (await text("#machine")) === "Desktop");
    await shot("04-remote-draft", { x: 232, y: 560, width: 1048, height: 300 });
    await pg.fill("#box", "Add a drawbridge"); await pg.press("#box", "Enter");
    const sent = await until(async () => (await posts("/m/m1/api/send")).length > 0);
    ok("r19 new chat made there", (await posts("/m/m1/api/new")).length === 1 && (await posts("/api/new")).length === 0, [await posts("/m/m1/api/new"), await posts("/api/new")]);
    const used = await last("/m/m1/api/project");
    ok("r20 tied to its own path", !!used && used.path === "D:/games/castle" && used.agent === "r101", used);
    const s = await last("/m/m1/api/send");
    ok("r21 sent there with the bare id", !!sent && s.agent === "r101" && s.text === "Add a drawbridge", s);
    ok("r22 no local send", (await posts("/api/send")).length === 0, await posts("/api/send"));
    ok("r23 the chat in front is m1~r101", await until(() => pg.evaluate(() => current === "m1~r101")), await pg.evaluate(() => current));
    await until(() => pg.evaluate(() => document.querySelector("#chat-main").innerText.includes("Add a drawbridge")));
    ok("r24 its own message came back as an event", await pg.evaluate(() => document.querySelector("#chat-main").innerText.includes("Add a drawbridge")));
    await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r101", text: "The drawbridge is down." } });
    ok("r25 remote events show in the chat", await until(() => pg.evaluate(() => document.querySelector("#chat-main").innerText.includes("The drawbridge is down."))));
    await mock("/mock/rpush", { m: "m1", event: { kind: "done", agent: "r101" } }); await wait(400);
    ok("r26 chip locked after the first message", await pg.evaluate(() => { const c = document.getElementById("machine"); return !!c && c.tagName !== "BUTTON" && !c.querySelector("svg.down") && c.title === "This chat runs on Desktop"; }), await pg.evaluate(() => { const c = document.getElementById("machine"); return c && [c.tagName, c.title, c.outerHTML.slice(0, 300)]; }));
    ok("r27 locked chip still names it", (await text("#machine")) === "Desktop");
    await shot("05-chip-locked", { x: 232, y: 560, width: 1048, height: 300 });
    ok("r28 the new chat is in the remote project's box", await pg.evaluate((k) => !!document.querySelector('.pbox[data-box="' + k + '"] [data-k="c:m1~r101"]'), CASTLE));

    // ---- an existing chat on the other computer ----
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300);
    await pg.click('[data-k="c:m1~r1"]'); await wait(400);
    ok("r29 the other computer's chat opens", await pg.evaluate(() => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("The gates are up.")));
    ok("r30 chip locked on it", (await text("#machine")) === "Desktop" && await pg.evaluate(() => document.getElementById("machine").tagName !== "BUTTON"));
    await pg.evaluate(() => fridae.chat.setBox("Paint it red")); await pg.press("#box", "Enter"); await wait(400);
    const s2 = await last("/m/m1/api/send");
    ok("r31 a message on it goes there", !!s2 && s2.agent === "r1" && s2.text === "Paint it red", s2);
    await pg.click('.proj[data-k="p:H:/games/obby-rush"]'); await wait(400);
    await pg.click('[data-k="c:a1"]'); await wait(300);
    await pg.evaluate(() => fridae.chat.setBox("Local words")); await pg.press("#box", "Enter"); await wait(400);
    const s3 = await last("/api/send");
    ok("r32 this computer's chat still sends here", !!s3 && s3.agent === "a1" && s3.text === "Local words", s3);
    ok("r33 its chip says This PC, locked", (await text("#machine")) === "This PC" && await pg.evaluate(() => document.getElementById("machine").tagName !== "BUTTON"));
    ok("r34 history from the other computer", await pg.evaluate(() => (fridae.store.get().saved.chats || []).some((c) => c.id === "m1~s9" && c.project === "m1~D:/games/castle")));

    // ---- pairing: found on the network, a wrong code, the right one ----
    await pg.evaluate(() => { document.querySelector('.proj[data-k="p:H:/games/obby-rush"] .p-new').click(); }); await wait(300);
    await pg.click("#machine"); await wait(200);
    await pg.click("#pair-open"); await wait(300);
    ok("r35 pair dialog open", await vis("#pair"));
    ok("r36 its heading", (await text("#pair h2")) === "Pair a computer", await text("#pair h2"));
    await until(() => pg.evaluate(() => document.querySelectorAll("#pair .pair-row").length >= 2));
    const found = await pg.$$eval("#pair .pair-row", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r37 computers on this network", found.length === 2 && /^Studio laptop/.test(found[0]) && /Pair$/.test(found[0]) && /^Desktop/.test(found[1]) && /Paired$/.test(found[1]), found);
    ok("r38 the two sections", (await text("#pair")).includes("Computers on this network") && (await text("#pair")).includes("Add by address"), await text("#pair"));
    ok("r39 address field", (await pg.getAttribute("#pair-addr", "placeholder")) === "Computer name or address");
    await shot("06-pair-find", { x: 380, y: 140, width: 520, height: 580 });
    const n0 = (await posts("/api/remote/discover")).length;
    await wait(4200);
    ok("r40 looked again while open (every 3 s)", (await posts("/api/remote/discover")).length > n0, [(await posts("/api/remote/discover")).length, n0]);
    await pg.click('#pair .pair-row >> nth=0 >> button'); await wait(300);
    ok("r41 code step", await vis("#pair-code") && (await text("#pair h2")) === "Pair with Studio laptop", await text("#pair h2"));
    await pg.type("#pair-code", "abcdefg");
    ok("r42 dash added as typed", (await pg.inputValue("#pair-code")) === "ABCDE-FG", await pg.inputValue("#pair-code"));
    await pg.fill("#pair-code", ""); await pg.type("#pair-code", "wrongcode1"); await pg.press("#pair-code", "Enter"); await wait(500);
    ok("r43 a wrong code says so in plain words", (await text("#pair-err")) === "That code is not right. Check the code on the other computer and try again.", await text("#pair-err"));
    ok("r44 still open", await vis("#pair"));
    await shot("07-pair-error", { x: 380, y: 200, width: 520, height: 420 });
    await pg.fill("#pair-code", ""); await pg.type("#pair-code", "abcdefghjk");
    ok("r45 whole code shown", (await pg.inputValue("#pair-code")) === "ABCDE-FGHJK", await pg.inputValue("#pair-code"));
    await pg.click("#pair-go"); await wait(900);
    const pr = await last("/api/remote/pair");
    ok("r46 pair posted", !!pr && pr.addr === "192.168.1.31" && pr.port === 47530 && pr.code === "ABCDEFGHJK" && pr.fp === "b2".repeat(32), pr);
    ok("r47 dialog closed", !(await vis("#pair")));
    ok("r48 that computer is selected", (await text("#machine")) === "Studio laptop" && await pg.evaluate(() => draftOn && curProj === "m2~"), [await text("#machine"), await pg.evaluate(() => curProj)]);
    await pg.click("#machine"); await wait(200);
    const rows2 = await pg.$$eval("#machine-menu .opt", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r49 menu lists it", JSON.stringify(rows2) === JSON.stringify(["This PC", "Desktop", "Studio laptop", "Pair a computer…"]), rows2);
    await pg.keyboard.press("Escape"); await wait(200);

    // ---- add by address ----
    await pg.click("#machine"); await wait(200);
    await pg.click("#pair-open"); await wait(300);
    await pg.fill("#pair-addr", "nowhere.tailnet.ts.net"); await pg.press("#pair-addr", "Enter"); await wait(200);
    ok("r50 address goes to the code step", (await text("#pair h2")) === "Pair with nowhere.tailnet.ts.net", await text("#pair h2"));
    await pg.type("#pair-code", "abcdefghjk"); await pg.press("#pair-code", "Enter"); await wait(500);
    const pa = await last("/api/remote/pair");
    ok("r51 address posted as it was typed, no port", !!pa && pa.addr === "nowhere.tailnet.ts.net" && pa.port === undefined && pa.fp === undefined, pa);
    ok("r52 cannot connect, in plain words", /^Could not reach that computer/.test(await text("#pair-err") || ""), await text("#pair-err"));
    await pg.click("#pair-back"); await wait(200);
    ok("r53 back to finding", await vis("#pair-addr"));
    await pg.keyboard.press("Escape"); await wait(300);
    ok("r54 Escape closes it", !(await vis("#pair")));

    // ---- offline ----
    await mock("/mock/off", { m: "m1", off: true });
    ok("r55 offline: its boxes dimmed", await until(() => pg.evaluate((k) => { const b = document.querySelector('.pbox[data-box="' + k + '"]'); return !!b && b.classList.contains("off"); }, CASTLE), 9000));
    ok("r56 offline in the label", (await text(box(CASTLE) + " .p-where")) === "Desktop · offline" && (await text(box("m1~") + " .p-where")) === "Desktop · offline", [await text(box(CASTLE) + " .p-where"), await text(box("m1~") + " .p-where")]);
    ok("r57 still listed while offline", (await text(box(CASTLE) + " .p-name b")) === "Castle");
    await pg.click("#machine"); await wait(200);
    ok("r58 offline dot in the menu", await pg.evaluate(() => { const d = document.querySelector('#machine-menu [data-m="m1"] .dot'); return !!d && !d.classList.contains("ok"); }));
    await shot("08-offline", { x: 0, y: 40, width: 940, height: 820 });
    await pg.keyboard.press("Escape");
    ok("r59 local box not dimmed", await pg.evaluate(() => !document.querySelector('.pbox[data-box="H:/games/obby-rush"]').classList.contains("off")));
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300); await pg.click('[data-k="c:m1~r1"]'); await wait(300);
    await pg.evaluate(() => fridae.chat.setBox("Are you there?")); await pg.press("#box", "Enter"); await wait(300);
    ok("r62 offline: nothing sent, the words stay", (await pg.inputValue("#box")) === "Are you there?", await pg.inputValue("#box"));
    ok("r63 offline: one plain line says why", (await text(".saytip")) === "Desktop can't be reached right now, so nothing was sent. Your message is still here.", await text(".saytip"));
    await pg.evaluate(() => fridae.chat.setBox(""));
    await mock("/mock/off", { m: "m1", off: false });
    ok("r60 back online: undimmed", await until(() => pg.evaluate((k) => { const b = document.querySelector('.pbox[data-box="' + k + '"]'); return !!b && !b.classList.contains("off"); }, CASTLE), 35000));
    await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r1", text: "Back again." } });
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300); await pg.click('[data-k="c:m1~r1"]');
    ok("r61 its events flow again", await until(() => pg.evaluate(() => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("Back again.")), 6000));

  } catch (e) { out.CRASH = String(e && e.stack || e); }
  out.errors = errs;
  console.log(JSON.stringify(out, null, 1));
  await b.close();
})();
