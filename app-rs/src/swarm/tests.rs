use super::*;

/// The swarm's tests share the list of chats and the agent slots: one at a time (team.rs's Studio turn test too).
static SERIAL: Mutex<()> = Mutex::new(());

pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// A git repository with one commit (a.txt), as a game folder.
fn repo(name: &str) -> PathBuf {
    let d = paths::test_dir(&format!("swarm-{name}")).join("game");
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    assert!(git(&d, &["init", "-q"]).0);
    assert!(git_id(&d, &["add", "-A"]).0);
    assert!(git_id(&d, &["commit", "-q", "-m", "start"]).0);
    d
}

fn lead_of(d: &Path, name: &str) -> Arc<Agent> {
    agents::new_background_chat(&format!("Lead: {name}"), Some(d.display().to_string())).unwrap()
}

/// A member as `team::start` makes one: its place, its chat (parked: its first turn is over), joined.
fn member(lead: &Agent, hint: &str, writer: bool) -> (Prepared, Arc<Agent>) {
    let p = prepare(lead, hint, writer).unwrap();
    let a = agents::spawn_helper(Some(&format!("w-{hint}-{}", paths::random_hex(1))), if writer { "worker" } else { "critic" }, hint, p.folder(), None, Some(&lead.id)).unwrap();
    joined(&p, &a, if writer { "coder" } else { "critic" }, &format!("Build the {hint} part."));
    a.mark_result();
    (p, a)
}

fn run_of(lead: &str) -> String {
    runs().iter().find(|r| r.lead == lead).unwrap().id.clone()
}

fn with_run<T>(lead: &str, f: impl FnOnce(&mut Run) -> T) -> T {
    let mut all = runs();
    f(all.iter_mut().find(|r| r.lead == lead).unwrap())
}

fn state(lead: &str, name: &str) -> &'static str {
    with_run(lead, |r| r.members.iter().find(|m| m.name == name).unwrap().state)
}

fn member_id(lead: &str, name: &str) -> String {
    with_run(lead, |r| r.members.iter().find(|m| m.name == name).unwrap().id.clone())
}

/// The verifier of `name` ends its turn with this reply.
fn verdict(lead: &str, name: &str, reply: &str) -> Arc<Agent> {
    let vid = with_run(lead, |r| r.members.iter().rfind(|m| m.verifies.as_deref() == Some(name)).unwrap().id.clone());
    let v = agents::get(&vid).unwrap();
    v.put("user", json!({"text": "go"}));
    v.put("text", json!({"text": reply}));
    turn_ended(&v, false);
    v
}

fn tick_run(lead: &str, now: f64) {
    with_run(lead, |r| watch_run(r, now));
}

fn done(lead: &str) {
    for a in agents::helpers_of(lead) {
        agents::remove_agent(&a.id);
    }
    agents::remove_agent(lead);
    let mut all = runs();
    all.retain(|r| r.lead != lead);
}

fn integration_has(lead: &str, file: &str) -> bool {
    with_run(lead, |r| r.int_tree.join(file).exists())
}

#[test]
fn names_recipients_and_verdicts() {
    assert_eq!(handle("Shop UI!"), "shop-ui");
    assert_eq!(handle("@datastore"), "datastore");
    assert_eq!(handle("  ***  "), "member");
    assert_eq!(handle(&"x".repeat(40)).len(), 24);
    let to = recipients(&["@Shop-UI".into(), "all".into()], "hey @datastore, what's the key? and @shop-ui");
    assert_eq!(to, vec!["@shop-ui", "@all", "@datastore"]);
    assert!(mentions(&["@all".into()], "x", "coder"));
    assert!(mentions(&["@coder".into()], "x", "coder"));
    assert!(!mentions(&["@y".into()], "x", "coder"));
    assert_eq!(verdict_of("**PASS** all checks"), Some(true));
    assert_eq!(verdict_of("FAIL: the shop test breaks"), Some(false));
    assert_eq!(verdict_of("Looks fine"), None);
    assert!(cheap(Some("opencode-go/glm-5.3")) && cheap(Some("opencode-go/deepseek-v4.1-flash")) && !cheap(Some("opus")) && !cheap(None));
    // a verifier never runs on a cheap model
    assert_eq!(verifier_model("opencode", Some("opencode-go/glm-5.3"), None), ("claude".to_string(), None));
    assert_eq!(verifier_model("opencode", None, Some("opencode-go/deepseek-v4.1-flash")), ("claude".to_string(), None));
    assert_eq!(verifier_model("claude", Some("opus"), Some("opus")), ("claude".to_string(), Some("opus".to_string())));
    assert_eq!(clamp_ceiling(None), 4);
    assert_eq!(clamp_ceiling(Some(0)), 1);
    assert_eq!(clamp_ceiling(Some(99)), MAX_AGENTS);
}

#[test]
fn checks_follow_the_project_type() {
    let d = paths::test_dir("swarm-checks");
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    assert!(checks_for(&d).is_empty());
    fs::write(d.join("default.project.json"), "{}").unwrap();
    fs::write(d.join("selene.toml"), "").unwrap();
    let c = checks_for(&d).join("\n");
    assert!(c.contains("rojo build") && c.contains("selene") && c.contains("Luau type check"), "{c}");
    fs::write(d.join("Cargo.toml"), "").unwrap();
    assert!(checks_for(&d).iter().any(|c| c == "cargo test"));
}

#[test]
fn two_writers_get_their_own_copies_and_merge_cleanly() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("two");
    let lead = lead_of(&d, "two");
    let (a, aa) = member(&lead, "Shop UI", true);
    let (b, ba) = member(&lead, "datastore", true);
    let (ta, tb) = (a.tree.clone().unwrap(), b.tree.clone().unwrap());
    assert_ne!(ta, tb);
    assert!(ta.join("a.txt").is_file() && tb.join("a.txt").is_file(), "each copy is a checkout of the project");
    assert_eq!(aa.project().as_deref(), Some(ta.display().to_string().as_str()), "the member works in its own copy");
    assert_eq!(project_of(&aa.id).as_deref(), Some(d.display().to_string().as_str()));
    // the run's index tells the tools who is who
    let index: Value = serde_json::from_str(&fs::read_to_string(index_file()).unwrap()).unwrap();
    assert_eq!(index[&aa.id]["name"], "shop-ui");
    assert_eq!(index[&lead.id]["is_lead"], true);
    fs::write(ta.join("shop.lua"), "return {}\n").unwrap();
    fs::write(tb.join("data.lua"), "return {}\n").unwrap();
    // nothing changes in the project itself meanwhile
    assert!(!d.join("shop.lua").exists() && !d.join("data.lua").exists());
    let said = post_from(&aa.id, &[], "done", "Shop built").unwrap();
    assert!(said.contains("verifier"), "{said}");
    assert_eq!(state(&lead.id, "shop-ui"), "verifying");
    let vid = member_id(&lead.id, "verify-shop-ui");
    assert!(sent_to(&vid)[0].contains("Verify @shop-ui's work"), "the verifier is briefed");
    verdict(&lead.id, "shop-ui", "PASS: built and read the diff");
    assert_eq!(state(&lead.id, "shop-ui"), "merged");
    assert!(integration_has(&lead.id, "shop.lua"));
    assert!(!ta.exists(), "a merged member's copy is removed");
    post_from(&ba.id, &[], "done", "Profiles saved").unwrap();
    verdict(&lead.id, "datastore", "PASS");
    assert_eq!(state(&lead.id, "datastore"), "merged");
    assert!(integration_has(&lead.id, "shop.lua") && integration_has(&lead.id, "data.lua"));
    // the Review item lands it on the project's branch, only when approved
    let run = run_of(&lead.id);
    let item: Value = serde_json::from_str(&fs::read_to_string(d.join(".fridae").join("review").join(format!("swarm-{run}-1.json"))).unwrap()).unwrap();
    assert_eq!(item["status"], "waiting");
    assert!(item["summary"].as_str().unwrap().contains("@shop-ui") && item["summary"].as_str().unwrap().contains("@datastore"));
    assert!(!d.join("shop.lua").exists());
    let said = land(&d.display().to_string(), &item);
    assert!(said.contains("was merged into the project"), "{said}");
    assert!(d.join("shop.lua").exists() && d.join("data.lua").exists());
    // the page sees members and the board
    let page = api_get(&lead.id, None);
    assert_eq!(page["run"], run);
    assert!(page["members"].as_array().unwrap().iter().any(|m| m["name"] == "librarian" && m["status"] == "parked"));
    assert!(page["board"].as_array().unwrap().iter().any(|p| p["kind"] == "merged"));
    let last = page["board"].as_array().unwrap().last().unwrap()["t"].as_f64().unwrap();
    assert_eq!(api_get(&lead.id, Some(last))["board"].as_array().unwrap().len(), 0);
    assert_eq!(api_get("nobody", None), json!({"run": null, "ceiling": 4, "members": [], "board": []}));
    done(&lead.id);
}

#[test]
fn a_conflict_resumes_the_member_whose_branch_conflicts() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("conflict");
    let lead = lead_of(&d, "conflict");
    let (a, aa) = member(&lead, "alpha", true);
    let (b, ba) = member(&lead, "beta", true);
    fs::write(a.tree.as_ref().unwrap().join("a.txt"), "one\nALPHA\nthree\n").unwrap();
    fs::write(b.tree.as_ref().unwrap().join("a.txt"), "one\nBETA\nthree\n").unwrap();
    post_from(&aa.id, &[], "done", "alpha").unwrap();
    verdict(&lead.id, "alpha", "PASS");
    post_from(&ba.id, &[], "done", "beta").unwrap();
    verdict(&lead.id, "beta", "PASS");
    assert_eq!(state(&lead.id, "beta"), "active", "not merged");
    assert!(sent_to(&ba.id).iter().any(|t| t.contains("conflict markers in: a.txt")), "{:?}", sent_to(&ba.id));
    assert!(!sent_to(&aa.id).iter().any(|t| t.contains("conflict")), "only the member whose branch conflicts");
    let tb = b.tree.clone().unwrap();
    assert!(fs::read_to_string(tb.join("a.txt")).unwrap().contains("<<<<<<<"));
    // it resolves, posts done, and the same verifier checks again
    fs::write(tb.join("a.txt"), "one\nALPHA and BETA\nthree\n").unwrap();
    post_from(&ba.id, &[], "done", "resolved").unwrap();
    let vid = member_id(&lead.id, "verify-beta");
    assert!(sent_to(&vid).last().unwrap().contains("Check again"));
    verdict(&lead.id, "beta", "PASS");
    assert_eq!(state(&lead.id, "beta"), "merged");
    // (git may check it out with CRLF: core.autocrlf, as Git for Windows sets it)
    assert_eq!(with_run(&lead.id, |r| fs::read_to_string(r.int_tree.join("a.txt")).unwrap().replace("\r\n", "\n")), "one\nALPHA and BETA\nthree\n");
    done(&lead.id);
}

#[test]
fn a_branch_that_fails_verification_is_not_merged() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("fail");
    let lead = lead_of(&d, "fail");
    let (a, aa) = member(&lead, "shop", true);
    fs::write(a.tree.as_ref().unwrap().join("shop.lua"), "broken(\n").unwrap();
    post_from(&aa.id, &[], "done", "shop").unwrap();
    // the verdict posted on the board wins over the reply
    let vid = member_id(&lead.id, "verify-shop");
    post_from(&vid, &["@shop".into()], "verify", "FAIL: shop.lua does not parse").unwrap();
    verdict(&lead.id, "shop", "PASS");
    assert_eq!(state(&lead.id, "shop"), "failed");
    assert!(!integration_has(&lead.id, "shop.lua"));
    assert!(a.tree.as_ref().unwrap().exists(), "its copy stays for the fix");
    assert!(sent_to(&aa.id).iter().any(|t| t.contains("failed verification") && t.contains("does not parse")), "{:?}", sent_to(&aa.id));
    done(&lead.id);
}

#[test]
fn board_mentions_wake_parked_members_and_only_those() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("mentions");
    let lead = lead_of(&d, "mentions");
    let (_, a) = member(&lead, "alpha", false);
    let (_, b) = member(&lead, "beta", false);
    let (_, c) = member(&lead, "gamma", false);
    post_from(&lead.id, &["@beta".into()], "msg", "what is the profile key?").unwrap();
    assert!(sent_to(&b.id).iter().any(|t| t.contains("@lead (msg): what is the profile key?")));
    assert!(sent_to(&a.id).is_empty() && sent_to(&c.id).is_empty());
    // a mention in the text counts; a working member gets it when its turn ends
    c.set_running_for_test(true);
    post_from(&a.id, &[], "msg", "@gamma which folder is yours?").unwrap();
    assert!(sent_to(&c.id).is_empty(), "not while it works");
    c.set_running_for_test(false);
    turn_ended(&c, false);
    assert!(sent_to(&c.id).iter().any(|t| t.contains("@alpha (msg): @gamma which folder")));
    assert!(sent_to(&b.id).len() == 1);
    // a question to the Librarian is answered by the app, with no agent, while the Librarian shows as working
    let run = run_of(&lead.id);
    post_from(&a.id, &["@librarian".into()], "msg", "how do we stop exploiters firing this remote?").unwrap();
    assert!(member_id(&lead.id, "librarian").is_empty(), "no agent for a question");
    let asked: Vec<_> = LIBRARIAN_ASKED.lock().unwrap().iter().filter(|q| q.0 == run).cloned().collect();
    assert_eq!(asked, vec![(run.clone(), "alpha".to_string(), "how do we stop exploiters firing this remote?".to_string())]);
    let lib_status = |lead: &str| api_get(lead, None)["members"].as_array().unwrap().iter().find(|m| m["name"] == "librarian").unwrap()["status"].clone();
    assert_eq!(lib_status(&lead.id), "working");
    librarian_answered(&run, "alpha", "1. Validate on the server (Roblox security). Ask @gamma too.");
    assert_eq!(lib_status(&lead.id), "parked");
    let answer = read_board(&run).into_iter().last().unwrap();
    assert_eq!((answer["from"].as_str(), answer["kind"].as_str()), (Some("librarian"), Some("msg")));
    assert_eq!(answer["to"], json!(["@alpha"]), "to the asker only: an @ in the answer wakes nobody");
    assert!(sent_to(&a.id).iter().any(|t| t.contains("@librarian (msg): 1. Validate on the server")), "the asker is woken with it");
    assert_eq!(sent_to(&c.id).len(), 1);
    // "remember: ..." still starts the Librarian's agent, on a cheap OpenCode model
    post_from(&a.id, &["@librarian".into()], "msg", "remember: remotes are validated on the server").unwrap();
    assert_eq!(LIBRARIAN_ASKED.lock().unwrap().iter().filter(|q| q.0 == run).count(), 1, "not answered by the app");
    let lib = member_id(&lead.id, "librarian");
    assert!(!lib.is_empty());
    assert!(sent_to(&lib)[0].contains("remember: remotes are validated"));
    assert_eq!(agents::get(&lib).unwrap().ai(), "opencode");
    assert!(cheap(agents::get(&lib).unwrap().model().as_deref()));
    assert!(member_prompt(&lib).contains("not in the knowledge base"));
    // the user posts to the Lead
    assert_eq!(api_post(&json!({"action": "post", "lead": lead.id, "to": ["@alpha"], "text": "softer colours please"})), json!({"ok": true}));
    assert!(sent_to(&a.id).iter().any(|t| t.contains("@you (msg): softer colours please")));
    assert_eq!(api_post(&json!({"action": "post", "lead": "nobody", "to": [], "text": "x"}))["error"], NO_RUN);
    assert!(post_from(&a.id, &[], "merged", "x").is_err(), "the app's own kinds are not a member's");
    done(&lead.id);
}

#[test]
fn the_watchdog_wakes_the_lead_and_stops_a_silent_member() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("watchdog");
    let lead = lead_of(&d, "watchdog");
    let (_, a) = member(&lead, "alpha", true);
    let now = paths::now_secs();
    tick_run(&lead.id, now);
    assert!(sent_to(&lead.id).iter().any(|t| t.contains("Everyone is parked or idle, but @alpha has not finished")));
    tick_run(&lead.id, now);
    assert_eq!(sent_to(&lead.id).len(), 1, "once per quiet spell");
    // a silent member: nudged once, then stopped with its Studio turn and copy released
    let (b, ba) = member(&lead, "beta", true);
    ba.set_running_for_test(true);
    ba.set_writer(&d.display().to_string());
    tick_run(&lead.id, now + SILENT + 1.0);
    assert_eq!(state(&lead.id, "beta"), "active");
    assert!(read_board(&run_of(&lead.id)).iter().any(|p| p["text"].as_str().unwrap().contains("@beta has shown nothing")));
    tick_run(&lead.id, now + 2.0 * SILENT + 2.0);
    assert_eq!(state(&lead.id, "beta"), "stopped");
    assert!(sent_to(&ba.id).contains(&"<stopped>".to_string()));
    assert!(ba.writer().is_none(), "its Studio turn is released");
    assert!(!b.tree.unwrap().exists(), "its copy is released");
    assert!(read_board(&run_of(&lead.id)).iter().any(|p| p["text"].as_str().unwrap().contains("@beta was silent")));
    let _ = a;
    done(&lead.id);
}

#[test]
fn a_ceiling_of_one_works_one_at_a_time() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(1)));
    let d = repo("ceiling");
    let lead = lead_of(&d, "ceiling");
    let (_, a) = member(&lead, "alpha", false);
    let (_, b) = member(&lead, "beta", false);
    // alpha works: a post to beta waits for it
    a.set_running_for_test(true);
    post_from(&lead.id, &["@beta".into()], "msg", "start on the map").unwrap();
    assert!(sent_to(&b.id).is_empty(), "the ceiling is full");
    a.set_running_for_test(false);
    turn_ended(&a, false);
    assert_eq!(sent_to(&b.id).len(), 1, "resumed once alpha is parked");
    TEST_CEILING.with(|c| c.set(None));
    done(&lead.id);
}

#[test]
fn a_runs_lead_never_plans_on_a_cheap_model() {
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // the strongest: a stronger family first, then "pro"/"max", then the newer; never cheap or light ("gemini" is no "mini")
    assert_eq!(strongest(&ids(&["x/kimi-k3", "x/gpt-5.6-luna", "x/gpt-6-luna"])).as_deref(), Some("x/gpt-6-luna"));
    assert_eq!(strongest(&ids(&["x/qwen3.8-max", "x/kimi-k3", "x/kimi-k3.1-pro"])).as_deref(), Some("x/kimi-k3.1-pro"));
    assert_eq!(strongest(&ids(&["x/gemini-4-argon", "x/grok-4.7"])).as_deref(), Some("x/gemini-4-argon"));
    assert_eq!(strongest(&ids(&["x/deepseek-v4-pro", "x/glm-6", "x/gpt-5-mini", "x/claude-haiku-5-5", "x/hy4-flash-free"])), None);
    assert_eq!(strongest(&ids(&["x/deepseek-v4-pro", "x/hy4"])).as_deref(), Some("x/hy4"), "a family not named still beats none");
    // a world: OpenCode's plan has Kimi; Claude and Gemini are set up
    let plan = |ai: &str| (ai == "opencode").then(|| "opencode-go/kimi-k3".to_string());
    let none = |_: &str| None;
    let all = |_: &str| true;
    let only_gemini = |ai: &str| ai == "gemini";
    let nothing = |_: &str| false;
    let world = LeadWorld { strong_of: &plan, installed: &all, gemini_default: Some("gemini-4-pro".into()) };
    let deepseek = Some("opencode-go/deepseek-v4.1-flash");
    let glm = Some("opencode-go/glm-5.3");
    // a Lead on DeepSeek or GLM: the same AI's strong model
    assert_eq!(lead_model(true, true, "opencode", deepseek, &world), Some(("opencode".into(), Some("opencode-go/kimi-k3".into()))));
    assert_eq!(lead_model(true, true, "opencode", glm, &world), Some(("opencode".into(), Some("opencode-go/kimi-k3".into()))));
    // not a Lead, the setting off, or a model that is not cheap: as it is
    assert_eq!(lead_model(false, true, "opencode", deepseek, &world), None);
    assert_eq!(lead_model(true, false, "opencode", deepseek, &world), None);
    assert_eq!(lead_model(true, true, "opencode", Some("opencode-go/kimi-k3"), &world), None);
    assert_eq!(lead_model(true, true, "claude", Some("opus"), &world), None);
    assert_eq!(lead_model(true, true, "opencode", None, &world), None);
    // no strong model on the AI (never one billed per use: `opencode::strong_model`): Claude's Opus, else Gemini's default
    let w = LeadWorld { strong_of: &none, installed: &all, gemini_default: Some("gemini-4-pro".into()) };
    assert_eq!(lead_model(true, true, "opencode", deepseek, &w), Some(("claude".into(), Some("opus".into()))));
    let w = LeadWorld { strong_of: &none, installed: &only_gemini, gemini_default: Some("gemini-4-pro".into()) };
    assert_eq!(lead_model(true, true, "opencode", glm, &w), Some(("gemini".into(), Some("gemini-4-pro".into()))));
    // nothing stronger set up: it runs as it is
    let w = LeadWorld { strong_of: &none, installed: &nothing, gemini_default: None };
    assert_eq!(lead_model(true, true, "opencode", deepseek, &w), None);
    // an AI's "strong" model that is cheap all the same is not taken
    let cheap_too = |_: &str| Some("opencode-go/glm-6".to_string());
    let w = LeadWorld { strong_of: &cheap_too, installed: &nothing, gemini_default: None };
    assert_eq!(lead_model(true, true, "opencode", deepseek, &w), None);
    assert_eq!((cheap_name("opencode-go/deepseek-v4.1-flash"), cheap_name("opencode-go/glm-5.3")), ("DeepSeek", "GLM"));
    // verifiers and the Librarian keep their own rules
    assert_eq!(verifier_model("opencode", Some("opencode-go/glm-5.3"), None), ("claude".to_string(), None));
}

#[test]
fn a_member_sees_new_posts_mid_turn_and_is_not_sent_them_again() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("live");
    let lead = lead_of(&d, "live");
    let (_, a) = member(&lead, "alpha", false);
    let (_, b) = member(&lead, "beta", false);
    let run = run_of(&lead.id);
    // alpha works: a post to it waits, and its next step shows it (the hook), marking it seen
    a.set_running_for_test(true);
    post_from(&b.id, &["@alpha".into()], "msg", "the shop uses the coins key").unwrap();
    post_from(&b.id, &["@lead".into()], "msg", "only for the lead").unwrap();
    let said = live::unseen_text(&a.id);
    assert!(said.contains("@beta -> @alpha (msg): the shop uses the coins key"), "{said}");
    assert!(!said.contains("only for the lead"), "{said}");
    assert_eq!(live::unseen_text(&a.id), "", "shown once");
    let out = live::hook_output("claude", &said);
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(out["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("coins key"));
    let g = live::hook_output("gemini", &said);
    assert!(g["injectSteps"][0]["systemMessage"]["systemMessage"].as_str().unwrap().contains("coins key"));
    assert_eq!(live::hook_output("claude", ""), json!({}));
    // its turn ends: what it saw is not sent again as a message
    a.set_running_for_test(false);
    turn_ended(&a, false);
    assert!(sent_to(&a.id).is_empty(), "{:?}", sent_to(&a.id));
    // one it did not see is
    a.set_running_for_test(true);
    post_from(&b.id, &["@alpha".into()], "msg", "and the gems key").unwrap();
    a.set_running_for_test(false);
    turn_ended(&a, false);
    let got = sent_to(&a.id);
    assert_eq!(got.len(), 1);
    assert!(got[0].contains("and the gems key") && !got[0].contains("coins key"));
    // a parked member woken by a post has seen it: its next step does not show it again
    assert_eq!(live::unseen_text(&a.id), "");
    // the Lead hears every post but its own
    let lead_said = live::unseen_text(&lead.id);
    assert!(lead_said.contains("only for the lead") && lead_said.contains("coins key"), "{lead_said}");
    assert!(live::seen(&run, &lead.id) > 0.0);
    done(&lead.id);
}

#[test]
fn a_busy_lead_is_never_queued_board_posts_and_an_idle_one_is_woken_once() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("leadwake");
    let lead = lead_of(&d, "leadwake");
    let (_, a) = member(&lead, "alpha", false);
    let (_, b) = member(&lead, "beta", false);
    lead.set_running_for_test(true);
    post_from(&a.id, &["@lead".into()], "msg", "alpha is half done").unwrap();
    post_from(&b.id, &["@lead".into()], "msg", "beta needs a decision").unwrap();
    tick_run(&lead.id, paths::now_secs());
    assert!(sent_to(&lead.id).is_empty(), "nothing queued behind its turn");
    assert_eq!(lead.queued(), 0);
    // idle: one line for both
    lead.set_running_for_test(false);
    tick_run(&lead.id, paths::now_secs());
    let got = sent_to(&lead.id);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with("2 new posts for you") && got[0].contains("@alpha, @beta"), "{}", got[0]);
    tick_run(&lead.id, paths::now_secs());
    assert_eq!(sent_to(&lead.id).len(), 1, "once");
    // posts it saw at a step wake nothing
    lead.set_running_for_test(true);
    post_from(&a.id, &["@lead".into()], "msg", "alpha done soon").unwrap();
    assert!(live::unseen_text(&lead.id).contains("alpha done soon"));
    lead.set_running_for_test(false);
    tick_run(&lead.id, paths::now_secs());
    assert_eq!(sent_to(&lead.id).len(), 1);
    // the standing instructions: steer, keep the user told, end the turn while experts work
    let p = member_prompt(&lead.id);
    assert!(p.contains("live thread") && p.contains("end your turn") && p.contains("short update"), "{p}");
    done(&lead.id);
}

#[test]
fn the_users_message_to_a_busy_lead_reaches_it_at_its_next_step() {
    let _s = serial();
    TEST_CEILING.with(|c| c.set(Some(4)));
    let d = repo("steer");
    let lead = lead_of(&d, "steer");
    assert!(lead.set_ai("claude"));
    let (_, a) = member(&lead, "alpha", false);
    lead.set_running_for_test(true);
    lead.ask("you gotta update me occasionally lead", &Ask::default());
    assert_eq!(lead.queued(), 0, "not queued behind its turn");
    let said = live::unseen_text(&lead.id);
    assert!(said.contains("THE USER WROTE TO YOU") && said.contains("> you gotta update me occasionally lead"), "{said}");
    assert!(!live::unseen_text(&lead.id).contains("THE USER"), "given once");
    // not given over before the turn ended: it goes first in the queue, as a message already shown
    lead.ask("and keep it short", &Ask::default());
    lead.ask("Review: approved", &Ask { wake: true, ..Default::default() });
    assert_eq!(lead.queued(), 1, "the app's wake is queued");
    steer_left(&lead);
    assert_eq!(lead.queued(), 2);
    // a member, or an AI with no hook, is queued as before (what the user typed still ahead of the app's own)
    a.set_running_for_test(true);
    a.ask("Review: x", &Ask { app: true, ..Default::default() });
    a.ask("typed", &Ask::default());
    assert_eq!(a.queued(), 2);
    a.set_running_for_test(false);
    lead.set_running_for_test(false);
    done(&lead.id);
}
