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
  const outside = [];   /* any request that leaves 127.0.0.1 (a remote event must never send the page anywhere else) */
  pg.on("request", (r) => { if (!/^(http:\/\/127\.0\.0\.1:|data:|blob:|about:)/.test(r.url())) outside.push(r.url()); });
  pg.on("console", (m) => { if (m.type() === "error" && !/404|502|Failed to load resource/.test(m.text())) errs.push("console: " + m.text()); });
  const wait = (ms) => pg.waitForTimeout(ms);
  const posts = async (path) => (await pg.evaluate(() => fetch("/mock/posts").then((r) => r.json()))).filter((p) => p[0] === path);
  const last = async (path) => { const x = (await posts(path)).pop(); return x && x[1]; };
  const mock = (path, body) => pg.evaluate(([path, body]) => fetch(path, { method: "POST", body: JSON.stringify(body) }).then((r) => r.json()), [path, body]);
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

    // ---- a remote project's menu: no Open folder (it would open on the other computer's screen) ----
    const menuItems = async (k) => { await pg.click('.proj[data-k="p:' + k + '"]', { button: "right" }); await wait(250); const t = await pg.$$eval("#proj-menu button", (x) => x.map((n) => n.innerText.trim())); await pg.mouse.click(760, 480); await wait(150); return t; };
    const remoteMenu = await menuItems(CASTLE), localMenu = await menuItems("H:/games/obby-rush");
    ok("r95 remote project menu has no Open folder", remoteMenu.length > 0 && !remoteMenu.includes("Open folder") && remoteMenu.includes("Rename"), remoteMenu);
    ok("r96 this computer's project still has Open folder", localMenu.includes("Open folder"), localMenu);

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
    // ---- video clips are this computer's own files: not for a chat on another computer ----
    await pg.click("#add"); await wait(200);
    ok("r92 Video clips can't be picked for a remote chat", await pg.evaluate(() => { const b = document.getElementById("add-clips"); return !!b && b.disabled && b.innerText.includes("Only for chats on this PC"); }), await pg.evaluate(() => { const b = document.getElementById("add-clips"); return b && [b.disabled, b.innerText]; }));
    await pg.keyboard.press("Escape"); await wait(150);
    await pg.evaluate(() => fridae.chat.addClips(["C:/Users/me/Videos/a.mp4"])); await wait(200);
    ok("r93 a dropped clip is not put in a remote chat", !(await pg.inputValue("#box")).includes("[clip:"), await pg.inputValue("#box"));
    ok("r94 said plainly", (await text(".saytip")) === "Video clips can only be added to chats on this PC.", await text(".saytip"));
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
    ok("r26 after the first message the chip switches the computer it runs on", await pg.evaluate(() => { const c = document.getElementById("machine"); return !!c && c.tagName === "BUTTON" && c.getAttribute("aria-haspopup") === "menu" && c.title === "Runs on Desktop. Press to run this chat on another of your computers."; }), await pg.evaluate(() => { const c = document.getElementById("machine"); return c && [c.tagName, c.title]; }));
    ok("r27 locked chip still names it", (await text("#machine")) === "Desktop");
    await shot("05-chip-locked", { x: 232, y: 560, width: 1048, height: 300 });
    ok("r28 the new chat is in the remote project's box", await pg.evaluate((k) => !!document.querySelector('.pbox[data-box="' + k + '"] [data-k="c:m1~r101"]'), CASTLE));

    // ---- an existing chat on the other computer ----
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300);
    await pg.click('[data-k="c:m1~r1"]'); await wait(400);
    ok("r29 the other computer's chat opens", await pg.evaluate(() => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("The gates are up.")));
    ok("r30 chip names it, and can switch it", (await text("#machine")) === "Desktop" && await pg.evaluate(() => document.getElementById("machine").tagName === "BUTTON"));
    await pg.evaluate(() => fridae.chat.setBox("Paint it red")); await pg.press("#box", "Enter"); await wait(400);
    const s2 = await last("/m/m1/api/send");
    ok("r31 a message on it goes there", !!s2 && s2.agent === "r1" && s2.text === "Paint it red", s2);
    await pg.click('.proj[data-k="p:H:/games/obby-rush"]'); await wait(400);
    await pg.click('[data-k="c:a1"]'); await wait(300);
    await pg.evaluate(() => fridae.chat.setBox("Local words")); await pg.press("#box", "Enter"); await wait(400);
    const s3 = await last("/api/send");
    ok("r32 this computer's chat still sends here", !!s3 && s3.agent === "a1" && s3.text === "Local words", s3);
    ok("r33 its chip says This PC, and can switch it", (await text("#machine")) === "This PC" && await pg.evaluate(() => document.getElementById("machine").tagName === "BUTTON"));
    ok("r34 history from the other computer", await pg.evaluate(() => (fridae.store.get().saved.chats || []).some((c) => c.id === "m1~s9" && c.project === "m1~D:/games/castle")));

    // ---- pairing: found on the network, a wrong code, the right one ----
    await pg.evaluate(() => { document.querySelector('.proj[data-k="p:H:/games/obby-rush"] .p-new').click(); }); await wait(300);
    await pg.click("#machine"); await wait(200);
    await pg.click("#pair-open"); await wait(300);
    ok("r35 pair dialog open", await vis("#pair"));
    ok("r36 its heading", (await text("#pair h2")) === "Pair a computer", await text("#pair h2"));
    await until(() => pg.evaluate(() => document.querySelectorAll("#pair-found .pair-row").length >= 2));
    const found = await pg.$$eval("#pair-found .pair-row", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r37 computers on this network", found.length === 2 && /^Studio laptop/.test(found[0]) && /Pair$/.test(found[0]) && /^Desktop/.test(found[1]) && /Paired$/.test(found[1]), found);
    ok("r38 the two sections", (await text("#pair")).includes("Computers on this network") && (await text("#pair")).includes("Add by address"), await text("#pair"));
    ok("r39 address field", (await pg.getAttribute("#pair-addr", "placeholder")) === "Computer name or address");
    await shot("06-pair-find", { x: 380, y: 140, width: 520, height: 580 });
    const n0 = (await posts("/api/remote/discover")).length;
    await wait(4200);
    ok("r40 looked again while open (every 3 s)", (await posts("/api/remote/discover")).length > n0, [(await posts("/api/remote/discover")).length, n0]);
    await pg.click('#pair-found .pair-row >> nth=0 >> button'); await wait(300);
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
    // (No project is one box for every computer's chats with no project: each chat says where it runs)
    ok("r56 offline in the label", (await text(box(CASTLE) + " .p-where")) === "Desktop · offline", await text(box(CASTLE) + " .p-where"));
    ok("r57 still listed while offline", (await text(box(CASTLE) + " .p-name b")) === "Castle");
    await pg.click("#machine"); await wait(200);
    ok("r58 offline dot in the menu", await pg.evaluate(() => { const d = document.querySelector('#machine-menu [data-m="m1"] .dot'); return !!d && !d.classList.contains("ok"); }));
    await shot("08-offline", { x: 0, y: 40, width: 940, height: 820 });
    await pg.keyboard.press("Escape");
    ok("r59 local box not dimmed", await pg.evaluate(() => !document.querySelector('.pbox[data-box="H:/games/obby-rush"]').classList.contains("off")));
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300); await pg.click('[data-k="c:m1~r1"]'); await wait(300);
    await pg.evaluate(() => fridae.chat.setBox("Are you there?")); await pg.press("#box", "Enter"); await wait(300);
    ok("r62 offline: nothing sent, the words stay", (await pg.inputValue("#box")) === "Are you there?", await pg.inputValue("#box"));
    ok("r63 offline: one plain line says what to check", (await text(".saytip")) === "Couldn't reach Desktop. Make sure Fridae is open there with Let my other computers connect on, and that both are on the same network. Your message is still here.", await text(".saytip"));
    await pg.evaluate(() => fridae.chat.setBox(""));
    await mock("/mock/off", { m: "m1", off: false });
    ok("r60 back online: undimmed", await until(() => pg.evaluate((k) => { const b = document.querySelector('.pbox[data-box="' + k + '"]'); return !!b && !b.classList.contains("off"); }, CASTLE), 35000));
    await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r1", text: "Back again." } });
    await pg.click('.proj[data-k="p:' + CASTLE + '"]'); await wait(300); await pg.click('[data-k="c:m1~r1"]');
    ok("r61 its events flow again", await until(() => pg.evaluate(() => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("Back again.")), 6000), await pg.evaluate(() => [current, document.querySelector("#chat-main").innerText.slice(-300)]));


    // ---- a hostile event: pictures that point anywhere but that computer are dropped ----
    await mock("/mock/rpush", { m: "m1", event: { kind: "images", agent: "r1", urls: ["https://evil.example/x.png", "//evil.example/y.png", "/\\evil.example/z.png", "javascript:alert(1)", "/img/ok.png"] } });
    await mock("/mock/rpush", { m: "m1", event: { kind: "user", agent: "r1", text: "Look", images: ["https://evil.example/a.png", "/img/b.png"] } });
    await until(() => pg.evaluate(() => document.querySelectorAll('#chat-main img[src^="/m/m1/img/"]').length >= 2));
    const srcs = await pg.$$eval("#chat-main .shots img, #chat-main .pics img", (x) => x.map((n) => n.getAttribute("src")));
    ok("r64 only that computer's own pictures, through /m/m1", JSON.stringify(srcs) === JSON.stringify(["/m/m1/img/ok.png?t=x", "/m/m1/img/b.png?t=x"]), srcs);
    await wait(500);
    ok("r65 nothing went to another host", outside.length === 0, outside);

    // ---- the other computer's app restarts: its event ids start again from 1 ----
    await mock("/mock/off", { m: "m1", off: true });
    await until(() => pg.evaluate((k) => document.querySelector('.pbox[data-box="' + k + '"]').classList.contains("off"), CASTLE), 9000);
    await mock("/mock/restart", { m: "m1" });
    await mock("/mock/off", { m: "m1", off: false });
    ok("r66 back after the restart", await until(() => pg.evaluate((k) => !document.querySelector('.pbox[data-box="' + k + '"]').classList.contains("off"), CASTLE), 35000));
    await wait(800);
    await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r1", text: "After the restart." } });
    ok("r67 its new events show without a reload", await until(() => pg.evaluate(() => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("After the restart.")), 8000));
    ok("r68 what it put back is not shown twice", await pg.evaluate(() => document.querySelector("#chat-main").innerText.split("Build the gates").length - 1) === 1, await pg.evaluate(() => document.querySelector("#chat-main").innerText.split("Build the gates").length - 1));

    // ---- restarted again, and by the time the page is back its new ids have reached (or passed) the old cursor: only
    //      the boot tells; nothing it put back is shown as new, and what follows shows once each ----
    const shown = (t) => pg.evaluate((t) => document.querySelector("#chat-main").innerText.split(t).length - 1, t);
    for (const [fill, n, tag] of [["equal", 84, "equal"], [3, 88, "past"]]) {
      await mock("/mock/off", { m: "m1", off: true });
      await until(() => pg.evaluate((k) => document.querySelector('.pbox[data-box="' + k + '"]').classList.contains("off"), CASTLE), 9000);
      const rs = await mock("/mock/restart", { m: "m1", fill });
      await mock("/mock/off", { m: "m1", off: false });
      ok("r" + n + " restart (" + tag + ": new ids " + rs.newest + ", old " + rs.old + "): back", await until(() => pg.evaluate((k) => !document.querySelector('.pbox[data-box="' + k + '"]').classList.contains("off"), CASTLE), 35000));
      await wait(800);
      await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r1", text: "First after " + tag + "." } });
      await mock("/mock/rpush", { m: "m1", event: { kind: "text", agent: "r1", text: "Second after " + tag + "." } });
      ok("r" + (n + 1) + " " + tag + ": new events show without a reload", await until(() => pg.evaluate((tag) => current === "m1~r1" && document.querySelector("#chat-main").innerText.includes("Second after " + tag + "."), tag), 8000), await pg.evaluate(() => document.querySelector("#chat-main").innerText.slice(-300)));
      await wait(600);
      ok("r" + (n + 2) + " " + tag + ": each shown once, none lost", (await shown("First after " + tag + ".")) === 1 && (await shown("Second after " + tag + ".")) === 1, [await shown("First after " + tag + "."), await shown("Second after " + tag + ".")]);
      ok("r" + (n + 3) + " " + tag + ": what it did on starting is not replayed", (await shown("Put back")) === 0 && (await shown("Build the gates")) === 1, [await shown("Put back"), await shown("Build the gates")]);
    }

    // ---- not paired any more (401): said plainly, and pairing again is one press away ----
    await mock("/mock/off", { m: "m1", off: "unpaired" });
    ok("r69 label: needs pairing again", await until(() => pg.evaluate((k) => { const w = document.querySelector('.pbox[data-box="' + k + '"] .p-where'); return !!w && w.textContent === "Desktop · needs pairing again"; }, CASTLE), 9000), await text(box(CASTLE) + " .p-where"));
    ok("r70 its boxes dimmed", await pg.evaluate((k) => document.querySelector('.pbox[data-box="' + k + '"]').classList.contains("off"), CASTLE));
    await pg.evaluate(() => fridae.chat.setBox("Still there?")); await pg.press("#box", "Enter"); await wait(300);
    ok("r71 send: the words stay", (await pg.inputValue("#box")) === "Still there?");
    ok("r72 send: says it needs pairing again", (await text(".saytip")) === "Desktop needs pairing again. Your message is still here. Pair again", await text(".saytip"));
    await pg.click(".saytip button"); await wait(300);
    ok("r73 that button opens pairing for it", (await text("#pair h2")) === "Pair with Desktop", await text("#pair h2"));
    await pg.keyboard.press("Escape"); await wait(200); await pg.evaluate(() => fridae.chat.setBox(""));
    await pg.evaluate(() => { document.querySelector('.proj[data-k="p:H:/games/obby-rush"] .p-new').click(); }); await wait(300);
    await pg.click("#machine"); await wait(200);
    ok("r74 menu row says so", (await text('#machine-menu [data-m="m1"]')) === "Desktop Needs pairing again", await text('#machine-menu [data-m="m1"]'));
    await shot("10-menu-unpaired", { x: 0, y: 40, width: 940, height: 820 });
    await pg.click('#machine-menu [data-m="m1"]'); await wait(300);
    ok("r75 picking it opens pairing, not the machine", (await text("#pair h2")) === "Pair with Desktop" && await pg.evaluate(() => curProj === "H:/games/obby-rush"), [await text("#pair h2"), await pg.evaluate(() => curProj)]);
    await pg.type("#pair-code", "abcdefghjk"); await pg.click("#pair-go"); await wait(900);
    const rp = await last("/api/remote/pair");
    ok("r76 paired again: found with its own certificate, so held to it", !!rp && rp.addr === "192.168.1.20" && rp.port === 47530 && rp.code === "ABCDEFGHJK" && rp.fp === "a1".repeat(32), rp);
    ok("r77 and it is back, selected", await until(() => pg.evaluate((k) => { const w = document.querySelector('.pbox[data-box="' + k + '"] .p-where'); return !!w && w.textContent === "Desktop"; }, CASTLE), 9000) && (await text("#machine")) === "Desktop", [await text(box(CASTLE) + " .p-where"), await text("#machine")]);

    // ---- its identity changed (502 "identity changed"): it cannot be sent to until it is paired again ----
    await mock("/mock/off", { m: "m1", off: "changed" });
    ok("r78 label: identity changed", await until(() => pg.evaluate((k) => { const w = document.querySelector('.pbox[data-box="' + k + '"] .p-where'); return !!w && w.textContent === "Desktop · identity changed"; }, CASTLE), 9000), await text(box(CASTLE) + " .p-where"));
    const newBefore = (await posts("/m/m1/api/new")).length;
    await pg.fill("#box", "Hello"); await pg.press("#box", "Enter"); await wait(400);
    ok("r79 send refused, said plainly", (await text(".saytip")) === "Desktop's identity changed. Pair it again to be sure it is yours. Your message is still here. Pair again" && (await pg.inputValue("#box")) === "Hello", await text(".saytip"));
    ok("r80 nothing made there", (await posts("/m/m1/api/new")).length === newBefore);
    await pg.fill("#box", "");
    await pg.evaluate(() => { document.querySelector('.proj[data-k="p:H:/games/obby-rush"] .p-new').click(); }); await wait(300);
    await pg.click("#machine"); await wait(200);
    ok("r81 menu row says so", (await text('#machine-menu [data-m="m1"]')) === "Desktop Identity changed", await text('#machine-menu [data-m="m1"]'));
    await pg.click('#machine-menu [data-m="m1"]'); await wait(300);
    ok("r82 picking it opens pairing, with why", (await text("#pair h2")) === "Pair with Desktop" && ((await text("#pair .sub")) || "").startsWith("Desktop's identity changed. Pair it again to be sure it is yours."), await text("#pair .sub"));
    ok("r83 still on this computer", await pg.evaluate(() => curProj === "H:/games/obby-rush"));
    await shot("09-identity-changed", { x: 380, y: 200, width: 520, height: 420 });
    // finding now hears "Desktop" only with another certificate, from elsewhere: pairing again is a fresh pairing at
    // its saved address (the other computer must then show it knows the code), never held to the stranger
    await mock("/mock/found", { found: [{ id: "m1", name: "Desktop", addr: "192.168.1.66", port: 47530, fp: "c3".repeat(32) }] });
    await wait(4500);
    await pg.type("#pair-code", "abcdefghjk"); await pg.click("#pair-go"); await wait(900);
    const fresh = await last("/api/remote/pair");
    ok("r98 not re-found with its certificate: a fresh pairing at its saved address", !!fresh && fresh.addr === "192.168.1.20" && fresh.fp === undefined, fresh);
    await pg.focus("#pair-code"); await pg.keyboard.press("Escape"); await wait(200);

    // ---- forgetting a paired computer: its chats and projects leave the page at once ----
    await pg.evaluate(() => { window.__stay = 1; });
    await pg.click("#machine"); await wait(200);
    await pg.click("#pair-open"); await wait(300);
    const paired = await pg.$$eval("#pair-machines .pair-row", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r99 the pair dialog lists the paired computers", paired.length === 2 && /^Desktop/.test(paired[0]) && /Forget$/.test(paired[0]) && /^Studio laptop/.test(paired[1]), paired);
    await pg.click('#pair-machines [data-m="m1"] button'); await wait(200);
    ok("r100 asks first, plainly", ((await text('#pair-machines [data-m="m1"]')) || "").startsWith("Forget Desktop? Its chats stay on Desktop. You can pair it again any time."), await text('#pair-machines [data-m="m1"]'));
    await shot("12-forget-confirm", { x: 380, y: 100, width: 520, height: 700 });
    await pg.click('#pair-machines [data-m="m1"] button.main'); await wait(600);
    const fg = await last("/api/remote/forget");
    ok("r101 forget posted for it", !!fg && fg.id === "m1", fg);
    ok("r102 its projects and chats left the page", await until(() => pg.evaluate((k) => !document.querySelector('.pbox[data-box="' + k + '"]') && !document.querySelector('.pbox[data-box="m1~"]') && !document.querySelector('[data-k="c:m1~r1"]') && !(fridae.store.get().saved.chats || []).some((c) => String(c.id).startsWith("m1~")), CASTLE), 4000),
      await pg.evaluate(() => [...document.querySelectorAll(".pbox")].map((b) => b.dataset.box)));
    ok("r103 without a reload, and only Studio laptop is left", await pg.evaluate(() => window.__stay === 1) && JSON.stringify(await pg.$$eval("#pair-machines .pair-row", (x) => x.map((n) => n.dataset.m))) === JSON.stringify(["m2"]), await pg.$$eval("#pair-machines .pair-row", (x) => x.map((n) => n.dataset.m)));
    await pg.keyboard.press("Escape"); await wait(200);
    await pg.click("#machine"); await wait(200);
    const rows3 = await pg.$$eval("#machine-menu .opt", (x) => x.map((n) => n.innerText.replace(/\s+/g, " ").trim()));
    ok("r104 the chip's menu no longer offers it", JSON.stringify(rows3) === JSON.stringify(["This PC", "Studio laptop", "Pair a computer…"]), rows3);
    await pg.keyboard.press("Escape");

    // (last: opening Settings changes the timing of the long waits the restart checks above depend on)
    // ---- Settings, Other computers: what to check when the other computers can't find this one ----
    await pg.evaluate(() => fridae.windows.openSetup("remote"));
    await until(() => vis("#set-remote-on"));
    await until(async () => ((await text("#set-remote-on")) || "").includes("Windows"), 4000);
    ok("r97 firewall help under the switch while on", ((await text("#set-remote-on")) || "").includes("If your other computers can't find this one, Windows may be blocking Fridae. In Windows Settings › Network, set this network to Private, and allow Fridae when Windows asks."), await text("#set-remote-on"));
    await shot("11-settings-firewall", { x: 0, y: 0, width: 1280, height: 600 });
    await pg.evaluate(() => fridae.windows.closeSetup()); await wait(300);

    // ---- switching the computer a started chat runs on (move.ts, project_copy.rs, agents.rs move_out / move_in / moved_away) ----
    // (from a fresh start: the checks above forgot Desktop)
    await pg.request.post("http://127.0.0.1:" + PORT + "/mock/reset", { data: {} });
    await pg.goto(U);
    await until(() => pg.evaluate(() => typeof current !== "undefined" && current === "a1"), 6000);
    await until(() => vis("#machine"), 6000);
    const boxes = () => pg.$$eval(".pbox", (x) => x.map((n) => n.dataset.box + ":" + ((n.querySelector(".p-name b, .proj b") || {}).innerText || "")));
    ok("r105 in a started chat the chip is a menu", (await pg.getAttribute("#machine", "aria-haspopup")) === "menu");
    await pg.click("#machine"); await wait(300);
    ok("r106 the menu says what a pick does", (await text("#machine-menu .mc-q")) === "Run this chat on another computer. Its project's files are copied there first.", await text("#machine-menu .mc-q"));
    await shot("13-switch-menu", { x: 232, y: 520, width: 1048, height: 340 });
    await pg.click('#machine-menu [data-m="m1"]');
    const moved = await until(() => pg.evaluate(() => current && current.startsWith("m1~") ? current : ""), 8000);
    ok("r107 no question: the same chat now runs on Desktop", moved === "m1~a1", await pg.evaluate(() => current));
    const mcp = await posts("/api/project-copy"), mlp = await posts("/api/chat-move"), mrp = await posts("/m/m1/api/chat-move");
    ok("r108 the project's files copied first, by this window's app", mcp.length === 1 && JSON.stringify(mcp[0][1]) === JSON.stringify({ action: "copy", from: "", path: "H:/games/obby-rush", to: "m1" }), mcp);
    ok("r109 out and away here, in there, with its copy of the project", JSON.stringify(mlp.map((p) => p[1].action)) === '["out","away"]' && mrp.length === 1 && mrp[0][1].action === "in" && mrp[0][1].project === "H:/games/Obby Rush", [mlp, mrp]);
    await wait(800);
    const said = (await text("#chat-main")) || "";
    ok("r110 every message came, and nothing says it moved", said.includes("Build the lava stage") && !/Moved here/.test(said), said.slice(0, 400));
    ok("r111 the old copy is gone from this computer", !(await pg.evaluate(() => agents.has("a1"))));
    ok("r112 the chip names Desktop", (await text("#machine")) === "Desktop", await text("#machine"));
    ok("r113 Obby Rush is still one project in the list", (await boxes()).filter((x) => /Obby Rush/.test(x)).length === 1, await boxes());
    const srows = await pg.$$eval(".pbox.on .s-row", (x) => x.map((r) => r.dataset.k + "|" + r.innerText.replace(/\s+/g, " ").trim()));
    ok("r114 the chat keeps its place, saying where it runs", srows.some((r) => r.startsWith("c:m1~a1|") && r.includes("Desktop")), srows);
    await shot("15-switched", { x: 0, y: 0, width: 1280, height: 860 });
    // back here while it works: it waits for the reply
    await mock("/mock/rpush", { m: "m1", event: { kind: "busy", agent: "a1", value: true } });
    await until(() => pg.evaluate(() => agents.get("m1~a1") && agents.get("m1~a1").busy));
    await pg.click("#machine"); await wait(200);
    await pg.click('#machine-menu [data-m=""]'); await wait(500);
    const held = (await posts("/m/m1/api/chat-move")).slice(1).map((p) => [p[1].action, p[1].on]);
    ok("r115 a working chat is held, not switched", JSON.stringify(held) === '[["hold",true]]', held);
    ok("r116 the chip names both", /^Desktop ?This (PC|Mac)$/.test((await text("#machine")) || ""), await text("#machine"));
    await pg.click("#machine"); await wait(200);
    ok("r117 the row says when", ((await text('#machine-menu [data-m=""]')) || "").includes("Switches here when this reply is over"), await text('#machine-menu [data-m=""]'));
    await shot("16-switch-waiting", { x: 232, y: 480, width: 1048, height: 380 });
    await pg.click("#machine");
    await mock("/mock/rpush", { m: "m1", event: { kind: "done", agent: "a1" } });
    const back = await until(() => pg.evaluate(() => current === "a1"), 10000);
    ok("r118 it switched back when the reply was over, under its own id", !!back, [await pg.evaluate(() => current), (await posts("/m/m1/api/chat-move")).map((p) => p[1].action)]);
    const cps = (await posts("/api/project-copy")).map((p) => p[1]);
    ok("r119 the files came back from Desktop's copy", cps.length === 2 && cps[1].from === "m1" && cps[1].to === "" && cps[1].path === "H:/games/Obby Rush", cps);
    ok("r120 back on this computer, and the chip says so", /^This (PC|Mac)$/.test((await text("#machine")) || ""), await text("#machine"));
  } catch (e) { out.CRASH = String(e && e.stack || e); }
  out.errors = errs;
  console.log(JSON.stringify(out, null, 1));
  await b.close();
})();
