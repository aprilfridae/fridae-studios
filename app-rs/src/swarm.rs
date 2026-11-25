//! The swarm: a Lead's experts working at once, each writer in its own copy of the project, talking on one board.
//! Spec: docs/superpowers/specs/2026-11-24-swarm-design.md. There is no command: a Lead's first `rbx_team spawn`
//! in a project folder starts its run (`prepare`).
//!
//!   WORK/swarm/<run>/board.jsonl       one post per line {"t", "from", "to": ["@name" | "@role" | "@all"], "kind", "text"}
//!                                      written by the app alone (members post through `rbx_team`, see `request`)
//!   WORK/swarm/members.json            {agent id: {"run", "name", "role", "lead", "board", "is_lead"}}: who is who,
//!                                      for `rbxteam.py` (board, wait_board); the app's alone too (`paths::app_protected`)
//!   WORK/swarm-trees/<run>/<name>      a writing member's own git worktree, on branch `fridae/swarm-<run>-<name>`
//!   WORK/swarm-trees/<run>/_integration  the run's integration branch `fridae/swarm-<run>`, where verified work merges
//!
//! - A member that ends its turn is parked: its chat stays, and it holds no agent slot (`agents::holds_slot`). A post
//!   that mentions it resumes it (`pump`), within the user's ceiling (`ceiling`).
//! - A writing member posts `done`: the app commits its copy and starts a verifier on a capable model. PASS merges the
//!   branch into the integration branch, in order; FAIL goes back on the board to the member. A conflict merges the
//!   integration branch into the member's copy (markers left for it) and resumes it to resolve them.
//! - The integration branch reaches the project's own branch only when the user approves the Review item filed for it
//!   (`land`, from `studio_run::decide`).
//! - The watchdog (`tick`) is app code: it wakes the Lead when everyone is idle but work is unfinished, and nudges,
//!   then stops, a member silent for `SILENT` seconds (its Studio turn and copy are released).

use crate::paths::Start;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

#[cfg_attr(test, allow(unused_imports))]
use crate::agents::Ask;
use crate::{
    agents::{self, Agent, MAX_AGENTS},
    paths, projects,
};

/// The setting: most experts working at once (Setup > Experts), 1 to MAX_AGENTS.
pub const CEILING_KEY: &str = "swarm_ceiling";
pub const DEFAULT_CEILING: usize = 4;
/// Seconds without any output before a working member is nudged, and again before it is stopped.
pub const SILENT: f64 = 600.0;
/// How many posts the page gets at once.
const PAGE_LINES: usize = 200;
/// A post's text is cut to this many characters.
const TEXT_MAX: usize = 4000;
/// What a member may post as (the others are the app's own).
const AGENT_KINDS: [&str; 3] = ["msg", "done", "verify"];
/// Handles nobody can take.
const RESERVED: [&str; 6] = ["lead", "all", "user", "you", "librarian", "system"];

pub const NO_RUN: &str = "No swarm is running for this chat.";

#[cfg(test)]
thread_local! {
    static TEST_CEILING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn clamp_ceiling(v: Option<i64>) -> usize {
    v.map(|n| n.clamp(1, MAX_AGENTS as i64) as usize).unwrap_or(DEFAULT_CEILING)
}

/// The user's ceiling on members working at once (default 4, at most MAX_AGENTS). 1 is one at a time.
pub fn ceiling() -> usize {
    #[cfg(test)]
    if let Some(n) = TEST_CEILING.with(|c| c.get()) {
        return n;
    }
    clamp_ceiling(paths::setting(CEILING_KEY).as_i64())
}

/// Save the ceiling (clamped) and return it.
pub fn set_ceiling(v: i64) -> usize {
    let n = clamp_ceiling(Some(v));
    paths::update_config(|cfg| cfg.insert(CEILING_KEY.into(), json!(n)));
    n
}

#[derive(Clone, Debug)]
struct Member {
    /// The member's chat ("" for the Librarian until it is first asked something).
    id: String,
    /// Its handle, without the "@".
    name: String,
    /// The roster expert id, or "worker", "critic", "verifier", "librarian".
    role: String,
    /// "active", "verifying", "merged", "stopped" or "failed" (what the page shows is `shown_status`).
    state: &'static str,
    ai: String,
    model: Option<String>,
    writer: bool,
    branch: Option<String>,
    tree: Option<PathBuf>,
    brief: String,
    /// Posts that mention it, waiting to resume it.
    pending: Vec<String>,
    /// When the watchdog nudged it (silent while working).
    nudged: Option<f64>,
    /// A verifier: the member it checks; the verdict it posted.
    verifies: Option<String>,
    verdict: Option<(bool, String)>,
    /// A writer that posted done and waits for a verifier (a free place under the ceiling).
    verify_wanted: bool,
    /// The Librarian: questions the app is answering for it now (`librarian`), shown as working.
    answering: usize,
}

impl Member {
    fn new(name: &str, role: &str) -> Member {
        Member {
            id: String::new(),
            name: name.to_string(),
            role: role.to_string(),
            state: "active",
            ai: String::new(),
            model: None,
            writer: false,
            branch: None,
            tree: None,
            brief: String::new(),
            pending: Vec::new(),
            nudged: None,
            verifies: None,
            verdict: None,
            verify_wanted: false,
            answering: 0,
        }
    }
}

struct Run {
    id: String,
    lead: String,
    project: String,
    base: String,
    integration: String,
    int_tree: PathBuf,
    members: Vec<Member>,
    /// Posts on the board, and how many there were when the Lead was last woken for an idle run.
    posts: usize,
    woke_at: usize,
    /// Review items filed for landing the integration branch.
    reviews: u32,
}

static RUNS: Mutex<Vec<Run>> = Mutex::new(Vec::new());

fn runs() -> std::sync::MutexGuard<'static, Vec<Run>> {
    RUNS.lock().unwrap_or_else(|e| e.into_inner())
}

fn root() -> PathBuf {
    paths::work("swarm")
}

fn trees() -> PathBuf {
    paths::work("swarm-trees")
}

pub fn board_file(run: &str) -> PathBuf {
    root().join(run).join("board.jsonl")
}

fn index_file() -> PathBuf {
    root().join("members.json")
}

// ------------------------------------------------------------------ git

fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.args(projects::GIT_SAFE).args(args).current_dir(dir).env("GIT_TERMINAL_PROMPT", "0");
    paths::quiet(&mut cmd);
    match cmd.run_output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if !err.is_empty() {
                text = if text.is_empty() { err } else { format!("{text}\n{err}") };
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("Could not run git: {e}")),
    }
}

/// git with a neutral identity when the project has none (never the user's config: `-c` only lasts the one call).
fn git_id(dir: &Path, args: &[&str]) -> (bool, String) {
    let mut all: Vec<&str> = Vec::new();
    if !git(dir, &["config", "user.name"]).0 {
        all.extend(["-c", "user.name=Fridae Studios", "-c", "user.email=fridae-studios@localhost"]);
    }
    all.extend(args);
    git(dir, &all)
}

/// Commit everything in a member's copy (its `.fridae` folder aside), finishing a merge under way. Whether a commit was made.
fn commit_tree(tree: &Path, message: &str) -> bool {
    if !tree.is_dir() {
        return false;
    }
    let _ = git(tree, &["add", "-A", "--", ".", ":(exclude).fridae"]);
    let staged = !git(tree, &["diff", "--cached", "--quiet"]).0;
    let merging = git(tree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).0;
    if !staged && !merging {
        return false;
    }
    git_id(tree, &["commit", "--no-verify", "-q", "-m", message]).0
}

fn remove_tree(project: &str, tree: &Path) {
    let _ = git(Path::new(project), &["worktree", "remove", "--force", &tree.display().to_string()]);
    let _ = fs::remove_dir_all(tree);
    let _ = git(Path::new(project), &["worktree", "prune"]);
}

/// Commits on `branch` that `onto` does not have.
fn ahead(dir: &Path, onto: &str, branch: &str) -> usize {
    git(dir, &["rev-list", "--count", &format!("{onto}..{branch}")]).1.trim().parse().unwrap_or(0)
}

fn new_run(lead: &str, project: &str) -> Result<Run, String> {
    let dir = Path::new(project);
    if !dir.join(".git").exists() && !git(dir, &["init", "-q"]).0 {
        return Err("Could not make the project a git repository.".into());
    }
    // the base: the project as it is now (a checkpoint when anything is uncommitted, or there is no commit yet)
    let dirty = !git(dir, &["status", "--porcelain"]).1.trim().is_empty();
    let has_head = git(dir, &["rev-parse", "-q", "--verify", "HEAD"]).0;
    if (dirty || !has_head) && projects::checkpoint(project, "Before the swarm (Fridae Studios checkpoint)").is_none() {
        return Err("Could not save the project's files before the swarm.".into());
    }
    let (ok, base) = git(dir, &["rev-parse", "HEAD"]);
    if !ok {
        return Err("The project has no commit to start from.".into());
    }
    let id = paths::random_hex(1)[..8].to_string();
    let integration = format!("fridae/swarm-{id}");
    if !git(dir, &["branch", &integration, base.trim()]).0 {
        return Err("Could not make the swarm's integration branch.".into());
    }
    let int_tree = trees().join(&id).join("_integration");
    let _ = fs::create_dir_all(int_tree.parent().unwrap());
    let (ok, why) = git(dir, &["worktree", "add", "-q", &int_tree.display().to_string(), &integration]);
    if !ok {
        return Err(format!("Could not make the swarm's integration copy: {why}"));
    }
    let mut librarian = Member::new("librarian", "librarian");
    let (ai, model) = librarian_model();
    librarian.ai = ai;
    librarian.model = model;
    librarian.brief = "Answers knowledge questions from the vault and files what the swarm learns.".into();
    let _ = fs::create_dir_all(root().join(&id));
    Ok(Run { id, lead: lead.to_string(), project: project.to_string(), base: base.trim().to_string(), integration, int_tree, members: vec![librarian], posts: 0, woke_at: 0, reviews: 0 })
}

// ------------------------------------------------------------------ names and models

/// "Shop UI!" -> "shop-ui": lowercase letters, digits and single dashes, 24 characters at most.
pub fn handle(hint: &str) -> String {
    let mut out = String::new();
    for c in hint.trim().trim_start_matches('@').chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(24).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "member".into() } else { out }
}

fn unique_name(run: &Run, hint: &str) -> String {
    let base = handle(hint);
    let taken = |n: &str| RESERVED.contains(&n) || run.members.iter().any(|m| m.name == n);
    if !taken(&base) {
        return base;
    }
    (2..).map(|i| format!("{base}-{i}")).find(|n| !taken(n)).unwrap()
}

/// The user's cheap-model rule: DeepSeek and GLM only do simple work (the Librarian), never the checking.
pub fn cheap(model: Option<&str>) -> bool {
    model.is_some_and(|m| {
        let m = m.to_lowercase();
        m.contains("deepseek") || m.contains("glm")
    })
}

/// The Librarian runs on a cheap OpenCode model: the first of the app's cheap writers OpenCode offers now.
fn librarian_model() -> (String, Option<String>) {
    let offered = crate::opencode::models().into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    let pick = crate::opencode::WRITERS.iter().find(|w| offered.iter().any(|o| o == *w)).unwrap_or(&crate::opencode::WRITERS[2]);
    ("opencode".into(), Some(pick.to_string()))
}

/// Where a verifier runs: the Lead's AI and model, unless that is a cheap one (then Claude's default).
pub fn verifier_model(lead_ai: &str, lead_model: Option<&str>, default_model: Option<&str>) -> (String, Option<String>) {
    let model = lead_model.or(default_model);
    if cheap(model) {
        return ("claude".into(), None);
    }
    (lead_ai.to_string(), lead_model.map(str::to_string))
}

// ------------------------------------------------------------ a strong Lead

/// The guarded setting "Keep the Lead on a strong model" (`paths::strong_lead_on`): on unless the user turns it off.
pub const STRONG_LEAD_KEY: &str = "strong_lead";

/// A light model (fast and small): never what a Lead is moved to. Whole parts of the name, so "gemini" is not "mini".
const LIGHT: [&str; 7] = ["flash", "haiku", "mini", "nano", "lite", "small", "free"];

/// The families a Lead is moved to, the strongest first (the start of a model's name, after its provider). A family not
/// named here comes after these.
const STRONG: [&str; 8] = ["claude-opus", "gpt", "gemini", "kimi", "qwen", "grok", "minimax", "claude-sonnet"];

/// How strong a model is for a Lead, lower first: None for a cheap one (`cheap`) or a light one (`LIGHT`). Within a
/// family a "pro" or "max" model comes first, then the newer (the first version number in its name).
fn lead_rank(id: &str) -> Option<(usize, bool, std::cmp::Reverse<(u32, u32)>)> {
    if cheap(Some(id)) {
        return None;
    }
    let name = id.rsplit_once('/').map_or(id, |(_, m)| m).to_lowercase();
    let parts = || name.split(['-', '_', '.']);
    if parts().any(|part| LIGHT.contains(&part)) {
        return None;
    }
    let family = STRONG.iter().position(|f| name.starts_with(f)).unwrap_or(STRONG.len());
    let big = parts().any(|part| part == "pro" || part == "max");
    // "gpt-5.6-luna" is 5.6, "kimi-k3" is 3
    let version: String = name.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let mut v = version.split('.').map(|n| n.parse::<u32>().unwrap_or(0));
    Some((family, !big, std::cmp::Reverse((v.next().unwrap_or(0), v.next().unwrap_or(0)))))
}

/// The strongest of `ids` for a Lead (`lead_rank`), the first listed among equals; None when every one is cheap or light.
pub fn strongest(ids: &[String]) -> Option<String> {
    ids.iter().filter_map(|id| lead_rank(id).map(|r| (r, id))).min_by_key(|(r, _)| *r).map(|(_, id)| id.clone())
}

/// What a Lead's model is chosen from (`lead_world` for the real computer).
pub struct LeadWorld<'a> {
    /// the AI's own strong model, if it has one (OpenCode: on the user's plan, never billed per use)
    pub strong_of: &'a dyn Fn(&str) -> Option<String>,
    /// the AI is set up on this computer
    pub installed: &'a dyn Fn(&str) -> bool,
    /// Gemini's default model
    pub gemini_default: Option<String>,
}

/// The AI and model a turn of a run's Lead runs on instead, when the one it would run on is cheap (`cheap`): the same
/// AI's strong model; else Claude's Opus when Claude is set up; else Gemini's default when Gemini is. None: it runs as
/// it would (not a Lead, the setting is off, the model is not cheap, or nothing stronger is there).
pub fn lead_model(leads: bool, on: bool, ai: &str, model: Option<&str>, world: &LeadWorld) -> Option<(String, Option<String>)> {
    if !leads || !on || !cheap(model) {
        return None;
    }
    if let Some(m) = (world.strong_of)(ai).filter(|m| !cheap(Some(m))) {
        return Some((ai.to_string(), Some(m)));
    }
    if ai != "claude" && (world.installed)("claude") {
        return Some(("claude".into(), Some("opus".into())));
    }
    if ai != "gemini" && (world.installed)("gemini") && !cheap(world.gemini_default.as_deref()) {
        return Some(("gemini".into(), world.gemini_default.clone()));
    }
    None
}

/// The real computer for `lead_model`: OpenCode's strongest plan model (`opencode::strong_model`); another AI's
/// default when it is not cheap, else its strongest listed model.
pub fn lead_world() -> LeadWorld<'static> {
    LeadWorld {
        strong_of: &|ai| match ai {
            "opencode" => crate::opencode::strong_model(),
            "claude" => Some("opus".into()),
            _ => crate::expert_ai::default_of(ai).filter(|m| !cheap(Some(m))).or_else(|| strongest(&crate::expert_ai::models_of(ai))),
        },
        installed: &|ai| paths::find_ai(ai).is_some(),
        gemini_default: crate::expert_ai::default_of("gemini"),
    }
}

/// "DeepSeek" or "GLM": the cheap model a Lead was moved off, as the user names it.
pub fn cheap_name(model: &str) -> &'static str {
    if model.to_lowercase().contains("deepseek") { "DeepSeek" } else { "GLM" }
}

/// Whether this chat leads a swarm (its experts' board, `Run::lead`).
pub fn leads(agent_id: &str) -> bool {
    !agent_id.is_empty() && runs().iter().any(|r| r.lead == agent_id)
}

/// The checks a verifier runs, by what the copy holds.
pub fn checks_for(tree: &Path) -> Vec<String> {
    let has = |f: &str| tree.join(f).exists();
    let any_ext = |ext: &str| fs::read_dir(tree).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().ends_with(ext));
    let mut out = Vec::new();
    if has("default.project.json") || any_ext(".project.json") {
        out.push("rojo build -o /tmp/check.rbxl (the Rojo project builds)".to_string());
    }
    if has("selene.toml") {
        out.push("selene . (Luau lint)".into());
    }
    if has(".luaurc") || has("default.project.json") {
        out.push("luau-lsp analyze or luau-analyze on the changed .luau/.lua files (Luau type check)".into());
    }
    if has("Cargo.toml") {
        out.push("cargo test".into());
    }
    if has("package.json") {
        out.push("npm test (and npm run build when the package has one)".into());
    }
    if has("pyproject.toml") || has("setup.py") || tree.join("tests").is_dir() {
        out.push("python -m unittest discover tests (or pytest)".into());
    }
    if has("Makefile") {
        out.push("make test, when the Makefile has it".into());
    }
    out
}

// ------------------------------------------------------------------ the board

/// Recipients as stored: "@name", "@role" or "@all", lowercase, with every @mention in the text added.
pub fn recipients(to: &[String], text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let words = text.split_whitespace().filter(|w| w.starts_with('@')).map(str::to_string);
    for t in to.iter().cloned().chain(words) {
        let h = t.trim().trim_start_matches('@').trim_end_matches(|c: char| !c.is_ascii_alphanumeric()).to_lowercase();
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let tag = format!("@{h}");
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Whether a post is for this member: by name, by role, or to everyone.
pub fn mentions(to: &[String], name: &str, role: &str) -> bool {
    to.iter().any(|t| {
        let t = t.trim_start_matches('@');
        t == "all" || t == name || (!role.is_empty() && t == role)
    })
}

fn append_line(run: &str, post: &Value) -> bool {
    let file = board_file(run);
    let _ = fs::create_dir_all(file.parent().unwrap());
    let line = format!("{post}\n");
    fs::OpenOptions::new().create(true).append(true).open(&file).and_then(|mut f| f.write_all(line.as_bytes())).is_ok()
}

/// Every post on a run's board, oldest first.
pub fn read_board(run: &str) -> Vec<Value> {
    fs::read_to_string(board_file(run)).unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect()
}

/// Post on the board (under the lock), and mark every parked member it mentions to be resumed. The Lead is woken
/// when the post names it.
fn post_in(run: &mut Run, from: &str, to: &[String], kind: &str, text: &str) -> Value {
    // the app's own posts go to whom they name; a member's also to everyone its text @mentions
    let to = recipients(to, if from == "system" { "" } else { text });
    let text: String = text.trim().chars().take(TEXT_MAX).collect();
    let post = json!({"t": paths::now_secs(), "from": from, "to": to, "kind": kind, "text": text});
    append_line(&run.id, &post);
    run.posts += 1;
    agents::BUS.put("swarm", json!({"lead": run.lead}));
    let line = format!("@{from} ({kind}): {text}");
    let mut ask_librarian = false;
    for m in run.members.iter_mut() {
        // (the app's news to a merged member is not worth waking it for; a question from anyone is)
        if m.name != from && mentions(&to, &m.name, &m.role) && m.state != "stopped" && !(from == "system" && m.state == "merged") {
            // everyone: not the Librarian (it answers questions put to it), nor a verifier of another member's work
            let broadcast_only = !to.iter().any(|t| t.trim_start_matches('@') == m.name || t.trim_start_matches('@') == m.role);
            if broadcast_only && (m.role == "librarian" || m.role == "verifier") {
                continue;
            }
            // a question to the Librarian is answered by the app at once, with no agent ("remember: ..." still goes
            // to its agent, and so does the app's own news)
            if m.role == "librarian" && from != "system" && !librarian::is_remember(&text) {
                m.answering += 1;
                ask_librarian = true;
                continue;
            }
            m.pending.push(line.clone());
        }
    }
    if ask_librarian {
        librarian_asked(run, from, &text);
    }
    if from != "lead" && from != "system" && to.iter().any(|t| t == "@lead") {
        if let Some(lead) = agents::get(&run.lead) {
            wake_lead(&lead, &format!("New on your swarm's board for you: {line}\nRead the board with rbx_team action 'board'."));
        }
    }
    post
}

// ------------------------------------------------------------------ acting on agents (recorded in tests)

#[cfg(test)]
pub static SENT: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Tests: what was sent to this chat, oldest first.
#[cfg(test)]
pub fn sent_to(id: &str) -> Vec<String> {
    SENT.lock().unwrap().iter().filter(|(a, _)| a == id).map(|(_, t)| t.clone()).collect()
}

/// A message to a member: its first task, or a resume. Off this thread (a turn's prompt reads the swarm).
fn deliver(agent: &Arc<Agent>, text: &str, first: bool) {
    #[cfg(test)]
    {
        let _ = first;
        SENT.lock().unwrap().push((agent.id.clone(), text.to_string()));
    }
    #[cfg(not(test))]
    {
        let (a, t) = (agent.clone(), text.to_string());
        thread::spawn(move || if first { a.send(&t, &Ask::default()) } else { a.ask(&t, &Ask { app: true, ..Default::default() }) });
    }
}

fn wake_lead(lead: &Arc<Agent>, text: &str) {
    #[cfg(test)]
    SENT.lock().unwrap().push((lead.id.clone(), text.to_string()));
    #[cfg(not(test))]
    {
        if lead.studio_lead() {
            lead.wake_studio(text);
        } else {
            let (a, t) = (lead.clone(), text.to_string());
            thread::spawn(move || a.ask(&t, &Ask { app: true, ..Default::default() }));
        }
    }
}

fn stop_agent(agent: &Arc<Agent>) {
    crate::team::release(agent);
    #[cfg(test)]
    SENT.lock().unwrap().push((agent.id.clone(), "<stopped>".into()));
    #[cfg(not(test))]
    agent.stop();
}

fn busy(m: &Member) -> bool {
    !m.id.is_empty() && agents::get(&m.id).is_some_and(|a| a.busy())
}

/// Members at work now (holding an agent slot), the Librarian aside: what the ceiling counts.
fn at_work(run: &Run) -> usize {
    run.members.iter().filter(|m| m.role != "librarian" && !m.id.is_empty() && agents::get(&m.id).is_some_and(|a| a.holds_slot())).count()
}

fn shown_status(m: &Member) -> &'static str {
    match m.state {
        "stopped" => "stopped",
        "verifying" => "verifying",
        _ if busy(m) || m.answering > 0 => "working",
        "merged" => "merged",
        "failed" => "failed",
        _ => "parked",
    }
}

// ------------------------------------------------------------------ joining

/// A place made for a new member before its chat starts: its name, and for a writer its branch and copy.
pub struct Prepared {
    pub run: String,
    pub name: String,
    pub tree: Option<PathBuf>,
}

impl Prepared {
    /// The folder the member works in: its own copy, or None to work where its Lead does.
    pub fn folder(&self) -> Option<String> {
        self.tree.as_ref().map(|t| t.display().to_string())
    }
}

/// Make room for a member of `lead`'s run (starting the run on its first member): a name from `hint`, and for a
/// writer its own git worktree and branch, made from the integration branch (the run's base until work merges).
/// None when the Lead has no project folder or git is missing: the member then works as helpers always did.
pub fn prepare(lead: &Agent, hint: &str, writer: bool) -> Option<Prepared> {
    let project = lead.project().filter(|p| Path::new(p).is_dir())?;
    if !projects::git_installed() || is_member(&lead.id) {
        return None;
    }
    let mut all = runs();
    let at = match all.iter().position(|r| r.lead == lead.id && projects::same(&r.project, &project)) {
        Some(i) => i,
        None => {
            let run = new_run(&lead.id, &project).ok()?;
            all.push(run);
            all.len() - 1
        }
    };
    let run = &mut all[at];
    let name = unique_name(run, hint);
    let mut m = Member::new(&name, "worker");
    if writer {
        let branch = format!("{}-{name}", run.integration);
        let tree = trees().join(&run.id).join(&name);
        let (ok, _) = git(Path::new(&run.project), &["worktree", "add", "-q", "-b", &branch, &tree.display().to_string(), &run.integration]);
        if !ok {
            return None;
        }
        m.writer = true;
        m.branch = Some(branch);
        m.tree = Some(tree);
    }
    let prepared = Prepared { run: run.id.clone(), name: name.clone(), tree: m.tree.clone() };
    run.members.push(m);
    Some(prepared)
}

/// The member's chat started: it joins the run.
pub fn joined(p: &Prepared, agent: &Agent, role: &str, brief: &str) {
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.id == p.run) else { return };
    if let Some(m) = run.members.iter_mut().find(|m| m.name == p.name) {
        m.id = agent.id.clone();
        m.role = role.to_string();
        m.ai = agent.ai();
        m.model = agent.model();
        m.brief = brief.chars().take(4000).collect();
    }
    let what = if p.tree.is_some() { "in its own copy" } else { "reading only" };
    post_in(run, "system", &[], "system", &format!("@{} joined as {role}, {what}.", p.name));
    write_index(&all);
}

/// The member's chat could not start: its place and copy go.
pub fn discard(p: Prepared) {
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.id == p.run) else { return };
    if let Some(i) = run.members.iter().position(|m| m.name == p.name && m.id.is_empty()) {
        let m = run.members.remove(i);
        if let (Some(t), Some(b)) = (&m.tree, &m.branch) {
            remove_tree(&run.project, t);
            let _ = git(Path::new(&run.project), &["branch", "-D", b]);
        }
    }
}

/// Whether this chat is a member of a run (not its Lead).
pub fn is_member(agent_id: &str) -> bool {
    !agent_id.is_empty() && runs().iter().any(|r| r.members.iter().any(|m| m.id == agent_id))
}

/// Whether this chat is a run's Librarian (the ceiling does not count it).
pub fn is_librarian(agent_id: &str) -> bool {
    !agent_id.is_empty() && runs().iter().any(|r| r.members.iter().any(|m| m.id == agent_id && m.role == "librarian"))
}

/// The real project of a member (its copy is not where Studio's turn is kept), or None.
pub fn project_of(agent_id: &str) -> Option<String> {
    runs().iter().find(|r| r.lead == agent_id || r.members.iter().any(|m| m.id == agent_id)).map(|r| r.project.clone())
}

fn write_index(all: &[Run]) {
    let mut map = serde_json::Map::new();
    for r in all {
        let board = board_file(&r.id).display().to_string();
        map.insert(r.lead.clone(), json!({"run": r.id, "name": "lead", "role": "lead", "lead": r.lead, "board": board, "is_lead": true}));
        for m in r.members.iter().filter(|m| !m.id.is_empty()) {
            map.insert(m.id.clone(), json!({"run": r.id, "name": m.name, "role": m.role, "lead": r.lead, "board": board, "is_lead": false}));
        }
    }
    let file = index_file();
    let _ = fs::create_dir_all(file.parent().unwrap());
    let _ = crate::promo::write_atomic(&file, Value::Object(map).to_string().as_bytes());
}

// ------------------------------------------------------------------ requests from members (`rbx_team`)

/// `rbx_team` action "post" through the app: {"op": "post", "from": agent id, "to", "kind", "text"}.
pub fn request(req: &Value) -> Value {
    let from = req["from"].as_str().unwrap_or("");
    let kind = req["kind"].as_str().filter(|k| !k.is_empty()).unwrap_or("msg");
    let text = req["text"].as_str().unwrap_or("").trim();
    let to: Vec<String> = req["to"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    match post_from(from, &to, kind, text) {
        Ok(t) => json!({"status": "done", "text": t}),
        Err(e) => json!({"status": "refused", "reason": e}),
    }
}

/// A post by a member or the Lead (by chat id), with what follows from it: `done` starts verification, `verify`
/// records a verifier's verdict.
pub fn post_from(agent_id: &str, to: &[String], kind: &str, text: &str) -> Result<String, String> {
    if !AGENT_KINDS.contains(&kind) {
        return Err(format!("A post's kind is msg, done or verify, not \"{kind}\"."));
    }
    if text.is_empty() {
        return Err("Give the post's text.".into());
    }
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.lead == agent_id || r.members.iter().any(|m| m.id == agent_id)) else {
        return Err("You are not part of a swarm, so there is no board to post on.".into());
    };
    let at = run.members.iter().position(|m| m.id == agent_id);
    let from = at.map(|i| run.members[i].name.clone()).unwrap_or_else(|| "lead".into());
    post_in(run, &from, to, kind, text);
    let mut said = "Posted.".to_string();
    if let Some(i) = at {
        match kind {
            "done" => said = on_done(run, i, text),
            "verify" if run.members[i].verifies.is_some() => {
                let pass = verdict_of(text).unwrap_or(false);
                run.members[i].verdict = Some((pass, text.to_string()));
                said = "Verdict recorded. End your turn now.".into();
            }
            _ => {}
        }
    }
    pump(run);
    write_index(&all);
    Ok(said)
}

/// The user posts from the page (as "you"; a post for the user names "@you").
pub fn post_as_user(lead: &str, to: &[String], text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Write something to post.".into());
    }
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.lead == lead) else { return Err(NO_RUN.into()) };
    let to = if to.is_empty() { vec!["@lead".to_string()] } else { to.to_vec() };
    post_in(run, "you", &to, "msg", text);
    pump(run);
    Ok(())
}

/// PASS or FAIL as a verdict's first word (markdown and punctuation aside).
pub fn verdict_of(text: &str) -> Option<bool> {
    let first = text.split_whitespace().next()?.trim_matches(|c: char| !c.is_ascii_alphabetic()).to_uppercase();
    match first.as_str() {
        "PASS" | "PASSED" => Some(true),
        "FAIL" | "FAILED" => Some(false),
        _ => None,
    }
}

/// A member said its part is done. A writer's copy is committed and queued for a verifier; a writer with nothing
/// new is done at once; a reader is simply parked.
fn on_done(run: &mut Run, i: usize, text: &str) -> String {
    let m = &run.members[i];
    let (Some(tree), Some(branch)) = (m.tree.clone(), m.branch.clone()) else { return "Noted. End your turn now.".into() };
    if m.state == "merged" || m.state == "stopped" {
        return "Your part was already merged. End your turn now.".into();
    }
    let first: String = text.lines().next().unwrap_or("").chars().take(72).collect();
    commit_tree(&tree, &format!("@{}: {first}", m.name));
    if ahead(Path::new(&run.project), &run.integration, &branch) == 0 {
        let name = m.name.clone();
        finish_member(run, i, "merged");
        post_in(run, "system", &[format!("@{name}"), "@lead".into()], "merged", &format!("@{name} changed no files, so there was nothing to merge."));
        return "Your copy had no changes, so nothing was merged. End your turn now.".into();
    }
    run.members[i].state = "verifying";
    run.members[i].verify_wanted = true;
    "Your work was saved on your branch and a verifier will check it. End your turn now: you are woken if it fails.".into()
}

// ------------------------------------------------------------------ verification and merging

fn verifier_brief(run: &Run, m: &Member, tree: &Path) -> String {
    let checks = checks_for(tree);
    let checks = if checks.is_empty() { "No checks were found: read the diff closely and run whatever the project defines.".to_string() } else { format!("Run these where present:\n- {}", checks.join("\n- ")) };
    let stat = git(tree, &["diff", "--stat", &format!("{}...HEAD", run.integration)]).1;
    format!(
        "Verify @{name}'s work before it is merged. Its branch {branch} is checked out in your folder ({folder}).\n\n\
         Its brief was:\n{brief}\n\nWhat changed (git diff --stat {int}...HEAD):\n{stat}\n\n{checks}\n\n\
         Read the full diff (git diff {int}...HEAD) adversarially against the brief: anything missing, broken, insecure or \
         outside the brief is a FAIL. Change no files. Finish with rbx_team action 'post' kind 'verify', text starting PASS or \
         FAIL followed by what you checked and, for FAIL, exactly what failed and where. Then end your turn; your reply's first \
         word is PASS or FAIL too.",
        name = m.name,
        branch = m.branch.as_deref().unwrap_or(""),
        folder = tree.display(),
        brief = if m.brief.is_empty() { "(no brief recorded)" } else { &m.brief },
        int = run.integration,
    )
}

/// Start (or resume) the verifier of the member at `i`. False when no agent slot is free.
fn start_verifier(run: &mut Run, i: usize) -> bool {
    let target = run.members[i].clone();
    let Some(tree) = target.tree.clone() else { return false };
    let brief = verifier_brief(run, &target, &tree);
    // the one that checked it before checks the fix (it knows what failed)
    if let Some(v) = run.members.iter_mut().find(|v| v.verifies.as_deref() == Some(&target.name) && v.state != "stopped") {
        if let Some(agent) = agents::get(&v.id).filter(|a| !a.busy()) {
            v.verdict = None;
            v.state = "active";
            deliver(&agent, &format!("@{} says it fixed what failed. Check again.\n\n{brief}", target.name), false);
            return true;
        }
    }
    let Some(lead) = agents::get(&run.lead) else { return false };
    let (ai, model) = verifier_model(&lead.ai(), lead.model().as_deref(), crate::expert_ai::default_of(&lead.ai()).as_deref());
    let name = unique_name(run, &format!("verify-{}", target.name));
    let team_id = format!("v{}{}", (paths::now_secs() * 1000.0) as u64, &paths::random_hex(1)[..4]);
    let Some(agent) = agents::spawn_on(&team_id, "critic", &format!("Verify: {}", target.name), Some(tree.display().to_string()), ai, model, Some(&run.lead), None) else {
        return false;
    };
    let mut v = Member::new(&name, "verifier");
    v.id = agent.id.clone();
    v.ai = agent.ai();
    v.model = agent.model();
    v.verifies = Some(target.name.clone());
    v.brief = format!("Checks @{}'s branch before it is merged.", target.name);
    run.members.push(v);
    post_in(run, "system", &[], "system", &format!("@{name} is checking @{}'s work.", target.name));
    deliver(&agent, &brief, true);
    true
}

/// A verifier's turn ended: its verdict (posted, else its reply's first word; none is a FAIL) is acted on.
fn on_verdict(run: &mut Run, vi: usize, reply: &str, stopped: bool) {
    let v = run.members[vi].clone();
    let Some(target) = v.verifies.clone() else { return };
    let Some(ti) = run.members.iter().position(|m| m.name == target) else { return };
    if run.members[ti].state != "verifying" {
        return;
    }
    let (pass, why) = match (&v.verdict, verdict_of(reply)) {
        (Some((p, t)), _) => (*p, t.clone()),
        (None, Some(p)) => (p, reply.trim().chars().take(TEXT_MAX).collect()),
        (None, None) if stopped => (false, "The verifier was stopped before it gave a verdict.".to_string()),
        (None, None) => (false, "The verifier ended without a PASS or FAIL verdict.".to_string()),
    };
    if v.verdict.is_none() {
        post_in(run, &v.name, &[format!("@{target}")], "verify", &if pass { format!("PASS {why}") } else { format!("FAIL {why}") });
    }
    run.members[vi].state = if pass { "merged" } else { "failed" };
    if !pass {
        run.members[ti].state = "failed";
        let failed_why = why.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '*' || c == ':' || c == ' ').to_string();
        post_in(run, "system", &[format!("@{target}")], "failed", &format!("@{target} your work failed verification: {failed_why}\nFix it in your copy, then post done again."));
        return;
    }
    merge(run, ti);
}

/// Merge a verified member's branch into the integration branch. Merged: its copy and branch go, and the Review
/// item for landing is filed or brought up to date. A conflict: the integration branch is merged into its copy
/// (the markers left for it) and it is resumed to resolve them.
fn merge(run: &mut Run, i: usize) {
    let m = run.members[i].clone();
    let (Some(branch), Some(tree)) = (m.branch.clone(), m.tree.clone()) else { return };
    let (ok, _) = git_id(&run.int_tree, &["merge", "--no-ff", "-q", "-m", &format!("Swarm: merge @{} (verified)", m.name), &branch]);
    if ok {
        finish_member(run, i, "merged");
        post_in(run, "system", &[format!("@{}", m.name), "@lead".into()], "merged", &format!("@{}'s verified work was merged into {}.", m.name, run.integration));
        file_review(run);
        return;
    }
    let files = git(&run.int_tree, &["diff", "--name-only", "--diff-filter=U"]).1;
    let _ = git(&run.int_tree, &["merge", "--abort"]);
    let _ = git_id(&tree, &["merge", "--no-edit", "-q", &run.integration]);
    let left = git(&tree, &["diff", "--name-only", "--diff-filter=U"]).1;
    let files = if left.trim().is_empty() { files } else { left };
    run.members[i].state = "active";
    post_in(
        run,
        "system",
        &[format!("@{}", m.name)],
        "failed",
        &format!(
            "@{} your branch conflicts with work already merged. The app merged {} into your copy and left conflict markers in: {}. Resolve them (keep both sides' intent), then post done again.",
            m.name,
            run.integration,
            files.lines().collect::<Vec<_>>().join(", ")
        ),
    );
}

/// A member is finished (merged or stopped): its uncommitted work is kept on its branch, its copy goes, and a merged
/// branch is deleted.
fn finish_member(run: &mut Run, i: usize, state: &'static str) {
    let m = &mut run.members[i];
    m.state = state;
    m.verify_wanted = false;
    m.pending.clear();
    if let Some(tree) = m.tree.take() {
        commit_tree(&tree, &format!("@{}: work in progress (kept when stopped)", m.name));
        remove_tree(&run.project, &tree);
        if state == "merged" {
            if let Some(b) = &m.branch {
                let _ = git(Path::new(&run.project), &["branch", "-D", b]);
            }
        }
    }
}

/// The Review item that lands the integration branch on the project's branch with the user's yes: one per batch of
/// merges, updated while it waits.
fn file_review(run: &mut Run) {
    let dir = Path::new(&run.project);
    let review = crate::studio_run::review_dir(dir);
    let id = |n: u32| format!("swarm-{}-{n}", run.id);
    let waiting = run.reviews > 0 && crate::studio_run::read_obj(&review.join(format!("{}.json", id(run.reviews)))).is_some_and(|i| i["status"] == "waiting");
    if !waiting {
        run.reviews += 1;
    }
    let item_id = id(run.reviews);
    let log = git(&run.int_tree, &["log", "--oneline", &format!("{}..HEAD", run.base)]).1;
    let target = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).1;
    let merged: Vec<String> = run.members.iter().filter(|m| m.writer && m.state == "merged").map(|m| format!("@{}", m.name)).collect();
    let now = paths::now_secs();
    let stage = crate::studio_run::read_run(dir).and_then(|r| r["stage"].as_str().map(str::to_string)).unwrap_or_default();
    let item = json!({
        "id": item_id, "stage": stage, "expert": "", "title": format!("Merge the team's verified work into {}", target.trim()),
        "summary": format!("Verified and merged on {} by {}. Approving merges it into {}.\n\n{}", run.integration, merged.join(", "), target.trim(), log),
        "plain": "", "media": [], "blocking": false, "status": "waiting", "note": "", "created": now, "decided": 0, "replaces": "",
        "by_chat": run.lead, "swarm": run.id, "branch": run.integration,
    });
    if let Ok(_lock) = crate::fridae_lock::acquire(dir) {
        let _ = fs::create_dir_all(&review);
        let _ = crate::studio_run::write_json(&review.join(format!("{item_id}.json")), &item);
    }
}

/// The user approved a swarm's Review item: its integration branch is merged into the project's branch. Returns a
/// line for the Lead.
pub fn land(project: &str, item: &Value) -> String {
    let Some(branch) = item["branch"].as_str().filter(|b| b.starts_with("fridae/swarm-")) else { return String::new() };
    let dir = Path::new(project);
    if !git(dir, &["rev-parse", "-q", "--verify", branch]).0 {
        return format!("The swarm's branch {branch} is gone, so nothing was merged.");
    }
    let _ = projects::checkpoint(project, "Before merging the team's work (Fridae Studios checkpoint)");
    let (ok, _) = git_id(dir, &["merge", "--no-ff", "-q", "-m", &format!("Merge the team's verified work ({branch})"), branch]);
    if ok {
        return format!("The team's verified work ({branch}) was merged into the project.");
    }
    let files = git(dir, &["diff", "--name-only", "--diff-filter=U"]).1;
    let _ = git(dir, &["merge", "--abort"]);
    format!("Merging {branch} into the project conflicted ({}), so nothing was merged. Bring those files in line on the integration branch through a member, then file it again.", files.lines().collect::<Vec<_>>().join(", "))
}

// ------------------------------------------------------------------ turn ends, resuming, the watchdog

/// A chat's turn ended (`team::finished`): a verifier's verdict is acted on; parked members with posts waiting are resumed.
pub fn turn_ended(agent: &Agent, stopped: bool) {
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.members.iter().any(|m| m.id == agent.id)) else { return };
    if let Some(vi) = run.members.iter().position(|m| m.id == agent.id && m.verifies.is_some() && m.state == "active") {
        on_verdict(run, vi, &agent.last_reply(), stopped);
    }
    pump(run);
    write_index(&all);
}

/// Resume parked members that posts mention, and start waiting verifiers, while the ceiling allows.
fn pump(run: &mut Run) {
    let ceiling = ceiling();
    for i in 0..run.members.len() {
        let m = &run.members[i];
        if m.verify_wanted && at_work(run) < ceiling && start_verifier(run, i) {
            run.members[i].verify_wanted = false;
        }
        let m = &run.members[i];
        if m.pending.is_empty() || m.state == "stopped" || m.state == "verifying" || busy(m) {
            continue;
        }
        let librarian = m.role == "librarian";
        if !librarian && at_work(run) >= ceiling {
            continue;
        }
        if !librarian && agents::slots_used() >= MAX_AGENTS {
            continue;
        }
        let text = format!(
            "New on the swarm's board for you:\n{}\n\nAnswer or act on it (reply with rbx_team action 'post'), then end your turn.",
            run.members[i].pending.join("\n")
        );
        if m.id.is_empty() {
            if librarian && start_librarian(run, i, &text) {
                run.members[i].pending.clear();
            }
            continue;
        }
        if let Some(agent) = agents::get(&m.id) {
            run.members[i].pending.clear();
            run.members[i].nudged = None;
            deliver(&agent, &text, false);
        }
    }
}

// ------------------------------------------------------------------ the Librarian's quick answers

/// Tests: the questions put to the Librarian, as (run, asker, text); the test answers them (`librarian_answered`).
#[cfg(test)]
pub static LIBRARIAN_ASKED: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

/// A question to the Librarian: answered off this thread (search, one cheap model, see `librarian`), then posted on
/// the board by the Librarian to the asker. The Librarian shows as working meanwhile (`Member::answering`).
fn librarian_asked(run: &Run, asker: &str, text: &str) {
    #[cfg(test)]
    LIBRARIAN_ASKED.lock().unwrap().push((run.id.clone(), asker.to_string(), text.to_string()));
    #[cfg(not(test))]
    {
        let (run_id, asker, text, project) = (run.id.clone(), asker.to_string(), text.to_string(), run.project.clone());
        thread::spawn(move || {
            let began = std::time::Instant::now();
            let said = librarian::answer(&text, &|parts| librarian::search_cli(&project, parts), &librarian::write, &librarian::check);
            eprintln!("librarian: answered @{asker} in {:.1} s", began.elapsed().as_secs_f64());
            librarian_answered(&run_id, &asker, &said);
        });
    }
}

/// The Librarian's answer goes on the board to the asker, as the Librarian's own posts do (wakes and mentions follow).
fn librarian_answered(run_id: &str, asker: &str, said: &str) {
    let mut all = runs();
    let Some(run) = all.iter_mut().find(|r| r.id == run_id) else { return };
    let Some(m) = run.members.iter_mut().find(|m| m.role == "librarian") else { return };
    m.answering = m.answering.saturating_sub(1);
    let name = m.name.clone();
    // an @ in a passage never wakes anyone: only the asker is told
    let said = said.replace('@', "\u{FF20}");
    post_in(run, &name, &[format!("@{asker}")], "msg", &said);
    pump(run);
    write_index(&all);
}

fn start_librarian(run: &mut Run, i: usize, text: &str) -> bool {
    let (ai, model) = (run.members[i].ai.clone(), run.members[i].model.clone());
    let team_id = format!("l{}{}", (paths::now_secs() * 1000.0) as u64, &paths::random_hex(1)[..4]);
    let Some(agent) = agents::spawn_on(&team_id, "worker", "Librarian", Some(run.project.clone()), ai, model, Some(&run.lead), None) else { return false };
    run.members[i].id = agent.id.clone();
    run.members[i].ai = agent.ai();
    run.members[i].model = agent.model();
    deliver(&agent, text, true);
    true
}

/// One look by the watchdog at every run (`now` in seconds). App code only: it costs no tokens.
pub fn tick(now: f64) {
    let mut all = runs();
    let mut i = 0;
    while i < all.len() {
        // a Lead that was closed takes its run with it: copies go (their work kept on their branches)
        if agents::get(&all[i].lead).is_none() {
            let mut run = all.remove(i);
            for j in 0..run.members.len() {
                if run.members[j].tree.is_some() {
                    finish_member(&mut run, j, "stopped");
                }
            }
            remove_tree(&run.project, &run.int_tree);
            continue;
        }
        watch_run(&mut all[i], now);
        i += 1;
    }
    write_index(&all);
}

fn watch_run(run: &mut Run, now: f64) {
    // members whose chat was closed
    for j in 0..run.members.len() {
        let m = &run.members[j];
        if !m.id.is_empty() && agents::get(&m.id).is_none() && !matches!(m.state, "merged" | "stopped") {
            let name = m.name.clone();
            finish_member(run, j, "stopped");
            post_in(run, "system", &["@lead".into()], "system", &format!("@{name}'s chat was closed, so it was stopped. Its work so far is kept on its branch."));
        }
    }
    // silent members: one nudge, then stopped
    for j in 0..run.members.len() {
        let m = &run.members[j];
        let Some(agent) = agents::get(&m.id).filter(|a| a.busy()) else {
            run.members[j].nudged = None;
            continue;
        };
        let last = agent.last_activity();
        let name = m.name.clone();
        let nudged = m.nudged;
        match nudged {
            Some(at) if last > at => run.members[j].nudged = None,
            Some(at) if now - at >= SILENT => {
                stop_agent(&agent);
                finish_member(run, j, "stopped");
                post_in(run, "system", &["@lead".into()], "system", &format!("@{name} was silent for {} minutes after a nudge, so it was stopped. Its Studio turn and copy were released; its work so far is kept on its branch.", (SILENT / 60.0) as u64));
            }
            Some(_) => {}
            None if now - last >= SILENT => {
                run.members[j].nudged = Some(now);
                // the nudge waits behind its turn (`pending`): it is what the member sees first when the turn ends
                post_in(run, "system", &[format!("@{name}")], "system", &format!("@{name} has shown nothing for {} minutes. Post a short progress note on the board, or post done if you are finished. A member that stays silent is stopped.", (SILENT / 60.0) as u64));
            }
            None => {}
        }
    }
    pump(run);
    // everyone parked or idle, but work unfinished: the Lead is woken once per quiet spell
    let lead_busy = agents::get(&run.lead).is_some_and(|a| a.busy());
    let anyone = run.members.iter().any(|m| busy(m) || m.answering > 0 || !m.pending.is_empty() || m.verify_wanted || m.state == "verifying");
    let open: Vec<String> = run.members.iter().filter(|m| m.writer && matches!(m.state, "active" | "failed")).map(|m| format!("@{}", m.name)).collect();
    if !lead_busy && !anyone && !open.is_empty() && run.posts > run.woke_at {
        let text = format!("Everyone is parked or idle, but {} not finished. Decide what happens next: post to a member, or start what is missing.", if open.len() == 1 { format!("{} has", open[0]) } else { format!("{} have", open.join(", ")) });
        post_in(run, "system", &[], "system", &text);
        run.woke_at = run.posts;
        if let Some(lead) = agents::get(&run.lead) {
            wake_lead(&lead, &format!("Fridae Studios (swarm watchdog): {text} Read the board with rbx_team action 'board'."));
        }
    }
}

/// The watchdog's loop.
pub fn watch() {
    loop {
        thread::sleep(Duration::from_secs(3));
        tick(paths::now_secs());
    }
}

// ------------------------------------------------------------------ what agents are told

/// The standing instructions of a swarm member (or a Lead with a run), or "".
pub fn member_prompt(agent_id: &str) -> String {
    let all = runs();
    if let Some(run) = all.iter().find(|r| r.lead == agent_id) {
        let list: Vec<String> = run.members.iter().map(|m| format!("@{} ({}, {})", m.name, m.role, shown_status(m))).collect();
        return format!(
            "\n\nYOUR SWARM (the experts you started work at once): {}. Every writer has its own copy and branch; the app merges \
             each verified part into {} and files a Review item for the user's yes. Read the board with rbx_team action 'board', \
             post with 'post' (to [\"@name\"], [\"@role\"] or [\"@all\"]). Parked members cost nothing and wake when mentioned. Ask \
             @librarian knowledge questions instead of reading whole notes.",
            list.join(", "),
            run.integration
        );
    }
    let Some((run, m)) = all.iter().find_map(|r| r.members.iter().find(|m| m.id == agent_id).map(|m| (r, m))) else { return String::new() };
    let mut out = format!("\n\nYOU ARE @{} IN A SWARM: experts working at once for the Lead, talking on a shared board.", m.name);
    match m.role.as_str() {
        "librarian" => out += "\n- You are the Librarian, the swarm's knowledge expert. For a question, search with rbx_vault action 'search' (shelf all), read at most the best two or three sections, and answer the asker on the board (rbx_team action 'post', to the asker) in at most five lines, citing each source note's title. Never summarise a guide (shelf guides): name it and tell the asker to load it whole with rbx_guide. When nothing relevant is found, answer exactly \"not in the knowledge base\" and never make an answer up.\n- For \"remember: ...\", file it with rbx_vault action 'note' (scope game: an expert files notes for this project; what holds for every game goes to the Lead in one line). Search the notes first: when a note on that topic exists, save with its exact title and replace true, so it is updated rather than duplicated. When 'note' answers \"Not saved\" nothing was filed and it shows the existing note: if yours replaces it, file again with supersedes naming it, or with force true to keep both; replace true (its exact title) overwrites it, so use that only for a near-copy; when it lists a possible contradiction, decide which holds and file the newer again with supersedes naming the older, or leave both when both hold, and say which in your confirmation. Then confirm in one line on the board.\n- Then end your turn: you are parked at no cost until someone mentions you.",
        "verifier" => out += "\n- You verify one member's branch, checked out in your folder. Change no files. Give your verdict with rbx_team action 'post' kind 'verify' (text starting PASS or FAIL), then end your turn.",
        _ => {
            if let (Some(tree), Some(branch)) = (&m.tree, &m.branch) {
                out += &format!("\n- Your own copy of the project is {} (branch {branch}). Edit files only there; the app commits it. Never merge, rebase onto or switch to another branch, and never touch the project's main branch: the app merges your work after a verifier checks it.", tree.display());
            } else {
                out += "\n- You only read and report: change no project files.";
            }
            out += "\n- The board: rbx_team action 'post' (to: [\"@name\"], [\"@role\"], [\"@all\"] or [\"@lead\"]; text) asks or tells another member directly; 'board' reads the recent posts; 'wait_board' waits for a post that mentions you.\n- Knowledge questions (how we do something, what the user likes, what was learned before): ask @librarian on the board rather than reading whole notes. To keep something worth knowing: post \"@librarian remember: ...\".\n- Roblox Studio is one shared place: before changing it, rbx_team action 'studio_turn' op 'take'; give it back with op 'give' as soon as your Studio part is done. File work needs no turn.\n- When your part is done, post kind 'done' with a one-line summary, then end your turn. When you are waiting on someone, post your question to them and end your turn: you are parked at no cost and woken when a post mentions you.";
        }
    }
    let _ = run;
    out
}

// ------------------------------------------------------------------ the page

/// GET /api/swarm?lead=<chat id>[&since=<t>]
pub fn api_get(lead: &str, since: Option<f64>) -> Value {
    let all = runs();
    let Some(run) = all.iter().find(|r| r.lead == lead) else {
        return json!({"run": null, "ceiling": ceiling(), "members": [], "board": []});
    };
    let members: Vec<Value> = run
        .members
        .iter()
        .map(|m| {
            let studio = !m.id.is_empty() && agents::get(&m.id).is_some_and(|a| a.writer().is_some());
            json!({"id": m.id, "name": m.name, "role": m.role, "status": shown_status(m), "ai": m.ai, "model": m.model, "branch": m.branch, "studio_turn": studio})
        })
        .collect();
    let mut board: Vec<Value> = read_board(&run.id).into_iter().filter(|p| since.is_none_or(|s| p["t"].as_f64().unwrap_or(0.0) > s)).collect();
    if board.len() > PAGE_LINES {
        board.drain(..board.len() - PAGE_LINES);
    }
    json!({"run": run.id, "ceiling": ceiling(), "members": members, "board": board})
}

/// POST /api/swarm {"action": "post", "lead", "to", "text"} | {"action": "ceiling", "value"}
pub fn api_post(body: &Value) -> Value {
    match body["action"].as_str().unwrap_or("") {
        "post" => {
            let to: Vec<String> = body["to"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            match post_as_user(body["lead"].as_str().unwrap_or(""), &to, body["text"].as_str().unwrap_or("")) {
                Ok(()) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "ceiling" => match body["value"].as_i64() {
            Some(v) => json!({"ok": true, "ceiling": set_ceiling(v)}),
            None => json!({"ok": false, "ceiling": ceiling(), "error": "Give the ceiling as a number from 1 to 8."}),
        },
        _ => json!({"ok": false, "error": "Unknown action."}),
    }
}

pub(crate) mod librarian;

#[cfg(test)]
pub(crate) mod tests;
