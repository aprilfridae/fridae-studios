//! The event log the window reads, the agents (one headless Claude Code process
//! per chat), and saved chats.

use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{
    antigravity::{self, Out},
    attach, board, codex,
    paths::{self, PATHS},
    plugin, projects, research, team, textures, usage,
};

pub const MAX_AGENTS: usize = 8;
pub const MAX_ATTACH: usize = 10;
/// Aliases the agent CLI resolves to the latest model of each family.
pub const MODELS: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];
/// The effort levels a message can ask for. Every AI here takes these five.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The app's standing instructions, with the guide routing table (shared with the tool server) filled in.
static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| include_str!("text/system.txt").replace("{routing}", include_str!("../../routing.txt")));
const SELF_TEST_PROMPT: &str = include_str!("text/selftest.txt");
const GEMINI_PROMPT: &str = include_str!("text/gemini.txt");
const PROJECT_PROMPT: &str = include_str!("text/project.txt");
const KICKOFF_NEW: &str = include_str!("text/kickoff_new.txt");
const KICKOFF_EXISTING: &str = include_str!("text/kickoff_existing.txt");
const WORKER_PROMPT: &str = include_str!("text/worker.txt");
const CRITIC_PROMPT: &str = include_str!("text/critic.txt");
const RESEARCH_PROMPT: &str = include_str!("text/research.txt");
const OTHER_AI_PROMPT: &str = include_str!("text/other_ai.txt");
const AUTOPILOT_PROMPT: &str = include_str!("text/autopilot.txt");
const FULL_PROMPT: &str = include_str!("text/full.txt");
const OVERNIGHT_PROMPT: &str = include_str!("text/overnight.txt");
const STUDIO_PROMPT: &str = include_str!("text/studio.txt");
const KICKOFF_STUDIO: &str = include_str!("text/kickoff_studio.txt");
/// How long overnight mode keeps a chat going after the user's last message.
const OVERNIGHT_SECS: f64 = 24.0 * 3600.0;
/// How many times in a row autopilot may tell an agent to carry on before the user is needed again.
const AUTOPILOT_TURNS: u32 = 12;

/// What goes with a message: the switches under the message box, the model, files, and how the turn began.
#[derive(Default, Clone)]
pub struct Ask {
    pub selftest: bool,
    pub gemini: bool,
    /// Keep working without being asked, for as long as the agent says there is more it can do alone.
    pub autopilot: bool,
    /// Overnight: keep going for up to 24 hours, past "done" and "needs you", until there is nothing left to do alone.
    pub overnight: bool,
    /// Full auto: every tool and shell command runs without a permission check, and it keeps going like autopilot.
    pub full: bool,
    pub model: Option<String>,
    /// How hard the model thinks: low, medium, high, xhigh or max. None leaves it to the AI.
    pub effort: Option<String>,
    pub attachments: Vec<String>,
    /// "new", "existing" or "studio" for a project's first message; "auto" for autopilot's own "carry on".
    pub kickoff: Option<String>,
    /// Studio mode: the game's chat is the Lead, moving the game through its stages with named experts. It implies
    /// full auto and overnight's loop, and may also wait for the user at a gate (STATUS: WAITING).
    pub studio: bool,
}

/// What kind of turn just ended, for deciding what happens next.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Stop was pressed during the turn.
    Stopped,
    /// The user sent messages while it worked; they go next instead of any carry-on.
    Held,
    Overnight,
    Studio,
    /// A Studio run whose 24 hours ran out during the turn: autopilot decides, but it can still wait at a gate.
    StudioLate,
    /// Neither night nor studio clock is running: autopilot (or nothing) decides.
    Autopilot,
}

/// The loop's counters, copied out of the chat's state and back after each turn.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct LoopState {
    autopilot_left: u32,
    /// Failed turns in a row, and "done"s in a row, in overnight or studio mode.
    fails: u32,
    done: u32,
    /// Whether the overnight (or studio) clock is still running; set false when the night or run ends or waits.
    night: bool,
}

/// What the app does after a turn.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    /// Send this message as the next turn.
    CarryOn(&'static str),
    /// Studio mode at a gate: send nothing and wait for the user's answer.
    Wait,
    /// The night or run is over; show this.
    End(&'static str),
    /// Nothing more happens.
    Idle,
}

fn loop_mode(stopped: bool, held: bool, night: bool, studio: bool) -> Mode {
    if stopped {
        Mode::Stopped
    } else if held {
        Mode::Held
    } else if night && studio {
        Mode::Studio
    } else if night {
        Mode::Overnight
    } else if studio {
        Mode::StudioLate
    } else {
        Mode::Autopilot
    }
}

const STUDIO_NEEDS_PROJECT: &str = "Studio mode needs a game project. Start a folder for this game first, then turn Studio on.";

/// Why a message cannot run in Studio mode, if it cannot: Studio runs belong to a game project's folder.
fn studio_refusal(studio: bool, has_project: bool) -> Option<&'static str> {
    (studio && !has_project).then_some(STUDIO_NEEDS_PROJECT)
}

/// The settings a message runs with. Studio mode is full auto with overnight's loop, and a Studio run waiting at a
/// gate goes on in Studio mode whatever the message itself asked for (a reply, or a decision from Review).
fn effective_ask(ask: &Ask, waiting: bool, last: &Ask) -> Ask {
    let (studio, overnight, full) = if waiting && last.studio { (true, last.overnight, last.full) } else { (ask.studio, ask.overnight, ask.full) };
    Ask { studio, overnight: overnight || studio, full: full || studio, ..ask.clone() }
}

/// The bookkeeping at the start of a message. None when Stop was pressed between two turns of the loop. Returns
/// whether a Studio run was waiting; a waiting run stays waiting (clock stopped) until a turn really starts.
fn begin_send(st: &mut State, ask: &Ask, auto: bool, now: f64) -> Option<bool> {
    // a message from the user sets autopilot's allowance afresh; its own "carry on" spends from it
    if auto && std::mem::take(&mut st.halted) {
        return None;
    }
    st.halted = false;
    let waiting = st.studio_waiting;
    if !auto {
        st.autopilot_left = if ask.autopilot || ask.full { AUTOPILOT_TURNS } else { 0 };
        if !waiting {
            st.overnight_until = if ask.overnight { now + OVERNIGHT_SECS } else { 0.0 };
        }
        st.overnight_fails = 0;
        st.overnight_done = 0;
    }
    st.last_ask = Ask { attachments: Vec::new(), kickoff: None, ..ask.clone() };
    Some(waiting)
}

/// A waiting Studio run goes on: the clock restarts with the full 24 hours. Returns the seconds that were left when
/// it stopped, so a turn that cannot start after all can put the wait back; None when it was not waiting.
fn resume_from_wait(st: &mut State, now: f64) -> Option<f64> {
    if !std::mem::take(&mut st.studio_waiting) {
        return None;
    }
    let left = std::mem::take(&mut st.overnight_left);
    st.overnight_until = now + OVERNIGHT_SECS;
    Some(left)
}

/// Undo `resume_from_wait` when the turn did not start: waiting again, clock stopped.
fn back_to_wait(st: &mut State, left: f64) {
    st.studio_waiting = true;
    st.overnight_left = left;
    st.overnight_until = 0.0;
}

const NEXT_STEP: &str = "Carry on with the next step.";
const TURN_FAILED: &str = "The last turn failed. Find out what went wrong, fix or work around it, then carry on.";

/// The whole of the after-turn decision. `said` is the reply's last line in capitals.
fn next_step(mode: Mode, got_result: bool, said: &str, state: &mut LoopState) -> Step {
    let studio = mode == Mode::Studio;
    match mode {
        Mode::Stopped | Mode::Held => {
            state.autopilot_left = 0;
            if mode == Mode::Stopped {
                state.night = false;
            }
            Step::Idle
        }
        // overnight: keep going past "done" and "needs you"; stop on repeated failure or when twice there is nothing left
        Mode::Overnight | Mode::Studio => {
            if !got_result {
                state.fails += 1;
                if state.fails >= 3 {
                    state.night = false;
                    Step::End(if studio { "Studio run stopped: three turns in a row failed." } else { "Overnight mode stopped: three turns in a row failed." })
                } else {
                    Step::CarryOn(TURN_FAILED)
                }
            } else if studio && said.starts_with("STATUS: WAITING") {
                // everything left depends on the user's answer in Review: stop the clock until they give it
                state.fails = 0;
                state.done = 0;
                state.night = false;
                Step::Wait
            } else if said.starts_with("STATUS: DONE") {
                state.fails = 0;
                state.done += 1;
                if state.done >= 2 {
                    state.night = false;
                    Step::End(if studio {
                        "Studio run finished: twice in a row there was nothing more it could do without you. See Review and the board for what needs you."
                    } else {
                        "Overnight mode finished: twice in a row there was nothing more it could do without you. See the board for what needs you."
                    })
                } else {
                    Step::CarryOn(if studio {
                        "Studio run: call rbx_stage decisions and act on every answer, then, within any limits I set in this chat, hand the next job the stages need to the right expert. If everything left waits on my answer in Review, end with STATUS: WAITING. If the run is truly finished, say so and end with STATUS: DONE."
                    } else {
                        "Overnight mode: within any limits I set in this chat, pick the next most valuable thing you can do without me (the board's todo cards first, then what GAME.md says is missing, then bugs, polish and playtesting) and do it. If nothing inside my limits is left, say so and end with STATUS: DONE."
                    })
                }
            } else if said.starts_with("STATUS: NEEDS YOU") {
                state.fails = 0;
                state.done = 0;
                Step::CarryOn(if studio {
                    "I am away. Put what you need from me on the board under needs if it is not there yet (or file it with rbx_stage review, blocking, if the run cannot pass a gate without it), then carry on with the next job that does not depend on it."
                } else {
                    "I am away. Put what you need from me on the board under needs if it is not there yet, then carry on with the next most valuable thing you can do without me."
                })
            } else {
                state.fails = 0;
                state.done = 0;
                Step::CarryOn(if studio { "Carry on with the next step. Call rbx_stage decisions first and act on any answer." } else { NEXT_STEP })
            }
        }
        Mode::Autopilot | Mode::StudioLate => {
            if mode == Mode::StudioLate && got_result && said.starts_with("STATUS: WAITING") {
                state.fails = 0;
                state.done = 0;
                state.night = false;
                return Step::Wait;
            }
            let go = got_result && state.autopilot_left > 0 && said.starts_with("STATUS: CONTINUE");
            state.autopilot_left = if go { state.autopilot_left - 1 } else { 0 };
            if go { Step::CarryOn(NEXT_STEP) } else { Step::Idle }
        }
    }
}

static FRIENDLY: LazyLock<Map<String, Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("text/friendly.json")).unwrap_or_default());
/// What the user can say to turn Gemini textures on.
static GEMINI_PHRASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\bgemini\b.*\b(textur\w*|images?|sprites?|decals?)\b|\b(textur\w*|images?|sprites?|decals?)\b.*\bgemini\b").unwrap()
});
/// Commands that make a texture with no agent involved.
static TEXTURE_COMMAND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)^/(texture|surface|particle|decal)\b[:\s]*(.*)$").unwrap());
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9]+").unwrap());

// ------------------------------------------------------------------- events
/// Append-only event log the page long-polls.
pub struct Bus {
    items: Mutex<Vec<Value>>,
    cond: Condvar,
}

pub static BUS: Bus = Bus { items: Mutex::new(Vec::new()), cond: Condvar::new() };

impl Bus {
    pub fn put(&self, kind: &str, data: Value) {
        let mut ev = data.as_object().cloned().unwrap_or_default();
        let mut items = self.items.lock().unwrap();
        ev.insert("kind".into(), json!(kind));
        ev.insert("id".into(), json!(items.len() + 1));
        items.push(Value::Object(ev));
        self.cond.notify_all();
    }

    pub fn after(&self, n: usize, wait: f64) -> Vec<Value> {
        let mut items = self.items.lock().unwrap();
        if items.len() <= n && wait > 0.0 {
            items = self.cond.wait_timeout(items, Duration::from_secs_f64(wait.min(60.0))).unwrap().0;
        }
        items[n.min(items.len())..].to_vec()
    }
}

// ------------------------------------------------------------------- agents
#[derive(Default)]
struct State {
    name: String,
    named: bool,
    saved_at: f64,
    notes: Vec<String>,
    making: bool,
    running: bool,
    stopped: bool,
    pid: Option<u32>,
    log: Vec<Value>,
    session: Option<String>,
    tools: HashMap<String, String>,
    /// The game project folder this chat belongs to.
    project: Option<String>,
    /// "worker" or "critic" for an agent another agent started, with the id of that request.
    role: Option<String>,
    team: Option<String>,
    /// Studio mode: the roster id of the expert this helper was started as, the instructions that came with it,
    /// and, for an expert that changes the game, the project whose one writer place it holds.
    expert: Option<String>,
    expert_text: Option<String>,
    writer_of: Option<String>,
    /// When the current (or latest) turn began, and the newest Studio output line the agent has been shown.
    turn_started: f64,
    log_seq: u64,
    /// Autopilot: how many more times this chat may be told to carry on, and what its last message asked for.
    autopilot_left: u32,
    last_ask: Ask,
    /// Overnight mode: when it ends (seconds since 1970, 0 when off), failed turns in a row, and "done"s in a row.
    overnight_until: f64,
    /// Stop was pressed between two of autopilot's or the night's turns: the next carry-on is not sent.
    halted: bool,
    overnight_fails: u32,
    overnight_done: u32,
    /// Studio mode stopped at a gate (STATUS: WAITING): no carry-on until the user answers, and the clock is stopped
    /// with this many seconds of it left.
    studio_waiting: bool,
    overnight_left: f64,
    /// Which AI this chat runs on ("claude", "gemini" or "chatgpt"). Open until the first message; a
    /// conversation then stays with the AI it began on, because its memory lives with that AI.
    ai: Option<String>,
    /// Messages the user sent while this chat was working, with their own settings. They go together as the next turn.
    queue: Vec<(String, String, Ask)>,
}

/// One chat. One headless Claude Code turn at a time; events go to the bus.
pub struct Agent {
    pub id: String,
    st: Mutex<State>,
    save_lock: Mutex<()>,
}

static AGENTS: Mutex<Vec<Arc<Agent>>> = Mutex::new(Vec::new());
static COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn agents() -> Vec<Arc<Agent>> {
    AGENTS.lock().unwrap().clone()
}

pub fn get(id: &str) -> Option<Arc<Agent>> {
    AGENTS.lock().unwrap().iter().find(|a| a.id == id).cloned()
}

fn is_user(e: &Value) -> bool {
    e["kind"] == "user"
}

impl Agent {
    fn new(id: String, name: &str) -> Arc<Agent> {
        Arc::new(Agent { id, st: Mutex::new(State { name: name.to_string(), ..Default::default() }), save_lock: Mutex::new(()) })
    }

    pub fn name(&self) -> String {
        self.st.lock().unwrap().name.clone()
    }

    pub fn project(&self) -> Option<String> {
        self.st.lock().unwrap().project.clone()
    }

    /// The request this agent was started for, and its role, if another agent started it.
    pub fn team(&self) -> Option<(String, String)> {
        let st = self.st.lock().unwrap();
        Some((st.team.clone()?, st.role.clone().unwrap_or_default()))
    }

    /// The roster id of the Studio expert this helper was started as (the id after `expert:`), if any.
    pub fn expert(&self) -> Option<String> {
        self.st.lock().unwrap().expert.clone()
    }

    /// Remember that this helper holds the project's writer place.
    pub fn set_writer(&self, project: &str) {
        self.st.lock().unwrap().writer_of = Some(project.to_string());
    }

    /// Give up the writer place: the project it was held for, once, then nothing (so a second release cannot free another helper's).
    pub fn take_writer(&self) -> Option<String> {
        self.st.lock().unwrap().writer_of.take()
    }

    /// Tie this chat to a game project. Only possible before the conversation has started,
    /// because the agent's memory of a conversation belongs to the folder it began in.
    pub fn set_project(&self, path: &str) -> bool {
        {
            let mut st = self.st.lock().unwrap();
            if st.session.is_some() || st.running {
                return false;
            }
            st.project = Some(path.to_string());
        }
        projects::touch(path);
        self.put("project", projects::describe(path));
        true
    }

    /// What the agent said in its latest turn.
    pub fn last_reply(&self) -> String {
        let st = self.st.lock().unwrap();
        let start = st.log.iter().rposition(is_user).map(|i| i + 1).unwrap_or(0);
        let pick = |kind: &str| st.log[start..].iter().filter(|e| e["kind"] == kind).filter_map(|e| e["text"].as_str()).collect::<Vec<_>>().join("\n\n");
        let text = pick("text");
        if text.is_empty() { pick("error") } else { text }
    }

    /// When this chat's latest turn began (seconds since 1970), or 0 if it has not had one.
    pub fn turn_started(&self) -> f64 {
        self.st.lock().unwrap().turn_started
    }

    /// Undo this chat's latest overnight run: the agent's steps in Studio since it began, every script as it was,
    /// and the project's files as they were at the checkpoint. Answers with what happened, for the window.
    pub fn undo_night(&self) -> Value {
        if self.busy() {
            return json!({"ok": false, "text": "Stop the agent first, then undo the night."});
        }
        let night = {
            let st = self.st.lock().unwrap();
            let last = st.log.iter().rposition(|e| e["kind"] == "night");
            match last {
                Some(i) if !st.log[i..].iter().any(|e| e["kind"] == "night_undone") => Some(st.log[i].clone()),
                _ => None,
            }
        };
        let Some(night) = night else {
            return json!({"ok": false, "text": "There is no overnight run in this chat to undo."});
        };
        let start = night["start"].as_f64().unwrap_or(0.0);
        let mut lines: Vec<String> = Vec::new();
        let mut ok = true;
        if start > 0.0 && plugin::connected() {
            let r = plugin::revert_within(start, 600);
            ok &= r["ok"] == true;
            lines.push(format!("Studio: {}", r["text"].as_str().unwrap_or("")));
        } else if night["studio"] == true {
            ok = false;
            lines.push("Studio: not connected. Open the same place in Studio (without restarting it if you can, since a restart clears its undo history), then try again.".to_string());
        } else {
            lines.push("Studio: it was not connected when the night began, so there was nothing there to undo.".to_string());
        }
        match (night["project"].as_str(), night["commit"].as_str()) {
            (Some(p), Some(c)) => match projects::restore_to(p, c) {
                Ok(t) => lines.push(format!("Project: {t}")),
                Err(e) => {
                    ok = false;
                    lines.push(format!("Project: {e}"));
                }
            },
            (Some(_), None) => lines.push("Project: its folder has no git repository, so its files could not be checkpointed and were left as they are.".to_string()),
            _ => {}
        }
        lines.push("The board's cards from the night were kept so you can still read them.".to_string());
        let text = lines.join("\n");
        // marked done only when something was put back, so a failed try can be repeated
        if ok {
            self.put("night_undone", json!({"text": text}));
        } else {
            self.put("error", json!({"text": format!("Undo the night did not finish.\n{text}")}));
        }
        json!({"ok": ok, "text": text})
    }

    /// The AI this chat runs on: its own, or the one new chats start on.
    pub fn ai(&self) -> String {
        self.st.lock().unwrap().ai.clone().unwrap_or_else(paths::default_ai)
    }

    /// Choose this chat's AI. Only possible before its conversation has started.
    pub fn set_ai(&self, kind: &str) -> bool {
        {
            let mut st = self.st.lock().unwrap();
            if !paths::AIS.contains(&kind) || st.session.is_some() || st.running || st.log.iter().any(is_user) {
                return false;
            }
            st.ai = Some(kind.to_string());
        }
        self.put("ai", json!({"ai": kind}));
        true
    }

    pub fn role(&self) -> Option<String> {
        self.st.lock().unwrap().role.clone()
    }

    pub fn busy(&self) -> bool {
        let st = self.st.lock().unwrap();
        st.making || st.running
    }

    fn has_messages(&self) -> bool {
        self.st.lock().unwrap().log.iter().any(is_user)
    }

    pub fn put(&self, kind: &str, data: Value) {
        let mut ev = data.as_object().cloned().unwrap_or_default();
        let mut shown = ev.clone();
        shown.insert("agent".into(), json!(self.id));
        BUS.put(kind, Value::Object(shown));
        let stale = {
            let mut st = self.st.lock().unwrap();
            if !matches!(kind, "agent_new" | "agent_removed" | "busy" | "model" | "queued") {
                ev.insert("kind".into(), json!(kind));
                st.log.push(Value::Object(ev));
            }
            paths::now_secs() - st.saved_at > 3.0
        };
        // saved as the chat goes, so nothing is lost if the window is closed or the app is killed
        if matches!(kind, "user" | "text" | "images" | "error" | "done" | "renamed" | "texdone" | "night" | "night_undone" | "studio_waiting") || (kind == "tool" && stale) {
            self.save();
        }
    }

    /// Write this chat to disk so it can be reopened later. Empty chats are not kept.
    pub fn save(&self) {
        let data = {
            let st = self.st.lock().unwrap();
            if !st.log.iter().any(is_user) {
                return;
            }
            json!({"id": self.id, "name": st.name, "session": st.session, "project": st.project, "role": st.role, "expert": st.expert, "ai": st.ai, "updated": paths::now_secs(), "events": st.log})
        };
        {
            let _guard = self.save_lock.lock().unwrap();
            let dir = paths::work("chats");
            let _ = fs::create_dir_all(&dir);
            let tmp = dir.join(format!("{}.tmp", self.id));
            if fs::write(&tmp, data.to_string()).is_ok() {
                let _ = fs::rename(&tmp, dir.join(format!("{}.json", self.id)));
            }
            self.st.lock().unwrap().saved_at = paths::now_secs();
        }
        remember_open();
    }

    /// The user's own name for this chat. It is kept, and no longer replaced by one made from the first message.
    pub fn rename(&self, name: &str) -> bool {
        let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
        if name.is_empty() {
            return false;
        }
        {
            let mut st = self.st.lock().unwrap();
            st.named = true;
            st.name = name;
        }
        self.put("renamed", json!({"name": self.name()}));
        true
    }

    /// A message from the user: sent now, or, while the chat is working, held and sent when it finishes.
    pub fn ask(self: &Arc<Self>, text: &str, ask: &Ask) {
        let queued = {
            let mut st = self.st.lock().unwrap();
            if st.making || st.running {
                let qid = paths::random_hex(1)[..8].to_string();
                st.queue.push((qid, text.to_string(), ask.clone()));
                true
            } else {
                false
            }
        };
        if queued {
            self.show_queue();
        } else {
            self.send(text, ask);
        }
    }

    /// A Studio run goes on: a decision from Review, or Resume. Sent as a message in Studio mode (full auto, overnight's
    /// loop) with the model and effort of the chat's last message; held like any message while the chat is working.
    /// Returns at once: the turn runs on its own thread.
    pub fn wake_studio(self: &Arc<Self>, text: &str) {
        let ask = {
            let st = self.st.lock().unwrap();
            Ask { studio: true, full: true, overnight: true, model: st.last_ask.model.clone(), effort: st.last_ask.effort.clone(), ..Default::default() }
        };
        let (me, text) = (self.clone(), text.to_string());
        thread::spawn(move || me.ask(&text, &ask));
    }

    /// Take back a held message before it is sent.
    pub fn unqueue(&self, qid: &str) -> bool {
        let removed = {
            let mut st = self.st.lock().unwrap();
            let before = st.queue.len();
            st.queue.retain(|(id, _, _)| id != qid);
            st.queue.len() < before
        };
        self.show_queue();
        removed
    }

    /// Whether overnight mode is keeping this chat going.
    pub fn overnight(&self) -> bool {
        self.st.lock().unwrap().overnight_until > paths::now_secs()
    }

    /// Whether a Studio run in this chat is stopped at a gate, waiting for the user's answer in Review.
    pub fn studio_waiting(&self) -> bool {
        self.st.lock().unwrap().studio_waiting
    }

    /// Note what a turn used: in the usage log, and as an event the window adds up for this chat.
    fn used(&self, turn: &usage::Turn) {
        if turn.is_empty() {
            return;
        }
        usage::record(&self.ai(), &self.id, turn);
        self.put("usage", turn.json());
    }

    fn show_queue(&self) {
        let items: Vec<Value> = self.st.lock().unwrap().queue.iter().map(|(id, text, _)| json!({"qid": id, "text": text})).collect();
        self.put("queued", json!({"items": items}));
    }

    fn name_from(&self, title: String) {
        let fresh = {
            let mut st = self.st.lock().unwrap();
            if st.named {
                false
            } else {
                st.named = true;
                st.name = title.chars().take(34).collect();
                true
            }
        };
        if fresh {
            self.put("renamed", json!({"name": self.name()}));
        }
    }

    /// /texture and friends: straight to the Gemini hand-off, with no agent turn.
    fn make_texture(&self, kind: &str, what: &str) {
        let kind = if kind == "texture" { "surface" } else { kind };
        self.put("user", json!({"text": format!("/{kind} {what}")}));
        if what.trim().is_empty() {
            self.put("error", json!({"text": "Say what the texture shows, for example: /particle jagged impact star"}));
            self.put("done", json!({}));
            return;
        }
        self.name_from(format!("Texture: {what}"));
        self.put("busy", json!({"value": true}));
        self.put("tool", json!({"label": "Making the texture"}));
        self.st.lock().unwrap().making = true;
        let name: String = WORDS.find_iter(what).take(4).map(|w| capitalize(w.as_str())).collect();
        let name = if name.is_empty() { "Texture".to_string() } else { name };
        // a command typed by the user always goes through their own Gemini chat, never the paid API
        let out = paths::cli(&["gen", what, "--kind", kind, "--name", &name, "--folder", "chat", "--agent", &self.id, "--no-api"]).output();
        let result = match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                text.find('{').and_then(|i| serde_json::from_str::<Value>(&text[i..]).ok()).ok_or_else(|| "The texture tool gave no result.".to_string())
            }
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(format!("Could not start the texture tool: {e}")),
        };
        match result {
            Ok(r) => {
                let file = r["file"].as_str().unwrap_or("");
                let mut urls: Vec<String> = [r["look_at"].as_str(), r["file"].as_str()]
                    .into_iter()
                    .flatten()
                    .filter_map(|p| fs::read(p).ok())
                    .filter_map(|bytes| attach::save_chat_image(&bytes))
                    .collect();
                if !urls.is_empty() {
                    let pick = if kind == "surface" { urls.remove(0) } else { urls.pop().unwrap() };
                    self.put("images", json!({"urls": [pick]}));
                }
                let kind_made = r["kind"].as_str().unwrap_or(kind);
                let mut lines = vec![format!("**{name}** is saved as `{file}` ({kind_made}, {}).", r["size"].as_str().unwrap_or(""))];
                let preview = r["preview_id"].as_str();
                if let Some(p) = preview {
                    lines.push(format!("Free local preview id for Studio: `{p}`"));
                }
                if let Some(problems) = r["check"]["problems"].as_array().filter(|p| !p.is_empty()) {
                    lines.push(format!("Check found: {}", problems.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")));
                }
                lines.push("It is not uploaded. Tell me what to use it for and I will take it from here.".into());
                self.put("text", json!({"text": lines.join("\n\n")}));
                let note = format!(
                    "The user generated a {kind_made} texture themselves: {file} (what it shows: {what}){}",
                    preview.map(|p| format!("; local preview id {p}")).unwrap_or_default()
                );
                self.st.lock().unwrap().notes.push(note);
            }
            Err(e) => {
                let short = e.split("\n\nPROMPT:").next().unwrap_or("").trim().to_string();
                let stopped = std::mem::take(&mut self.st.lock().unwrap().stopped);
                self.put("error", json!({"text": if stopped { "Stopped.".to_string() } else { short }}));
            }
        }
        self.st.lock().unwrap().making = false;
        self.put("done", json!({}));
    }

    pub fn send(self: &Arc<Self>, text: &str, ask: &Ask) {
        if self.busy() {
            return;
        }
        let ask = &{
            let st = self.st.lock().unwrap();
            effective_ask(ask, st.studio_waiting, &st.last_ask)
        };
        let has_project = self.project().is_some_and(|p| std::path::Path::new(&p).is_dir());
        if let Some(why) = studio_refusal(ask.studio, has_project) {
            self.put("error", json!({"text": why}));
            self.put("done", json!({}));
            return;
        }
        let (selftest, gemini, autopilot) = (ask.selftest, ask.gemini, ask.autopilot || ask.full);
        let (model, attachments, kickoff) = (ask.model.as_deref(), ask.attachments.as_slice(), ask.kickoff.as_deref());
        let auto = kickoff == Some("auto");
        let Some(resumed) = begin_send(&mut self.st.lock().unwrap(), ask, auto, paths::now_secs()) else {
            return;
        };
        if let Some(c) = TEXTURE_COMMAND.captures(text.trim()) {
            return self.make_texture(&c[1].to_lowercase(), c[2].trim());
        }
        // a turn starts here, so a Studio run waiting at a gate goes on (put back below if no AI can be found)
        let was_left = resume_from_wait(&mut self.st.lock().unwrap(), paths::now_secs());
        if was_left.is_some() {
            self.put("studio_waiting", json!({"value": false, "left": 0}));
        }
        let mut gemini = gemini;
        if !gemini && GEMINI_PHRASE.is_match(text) {
            gemini = true;
            BUS.put("mode", json!({"gemini": true})); // flip the switch in the window too
        }
        self.name_from(text.split_whitespace().collect::<Vec<_>>().join(" "));
        if let Some(p) = self.project() {
            let task = if auto { None } else { Some(board::gist(text, 90)) };
            board::track(&p, &self.id, &self.name(), "doing", task.as_deref(), if auto { None } else { Some("") });
        }
        // each AI has its own models: Claude's four families, or whatever Antigravity lists
        // each AI starts on a set model rather than whatever its own program defaults to
        let model: Option<String> = match self.ai().as_str() {
            "claude" => Some(model.filter(|m| MODELS.contains(m)).unwrap_or("opus").to_string()),
            "gemini" => model.filter(|m| antigravity::models().iter().any(|(id, _)| id == m)).map(str::to_string).or_else(antigravity::default_model),
            _ => model.filter(|m| codex::models().iter().any(|(id, _)| id == m)).map(str::to_string).or_else(codex::default_model),
        };
        let model = model.as_deref();
        let effort = ask.effort.as_deref().filter(|e| EFFORTS.contains(e));
        let infos: Vec<attach::Info> = attachments.iter().take(MAX_ATTACH).map(|n| paths::base_name(n)).filter_map(|n| attach::info(&n)).collect();
        self.put(
            "user",
            json!({
                "text": text, "selftest": selftest, "model": model, "gemini": gemini, "autopilot": autopilot || ask.overnight, "auto": auto,
                "overnight": ask.overnight, "full": ask.full, "until": self.st.lock().unwrap().overnight_until,
                "studio": ask.studio, "studio_waiting": false,
                "images": infos.iter().filter(|i| i.kind == "image").map(|i| format!("/att/{}", i.name)).collect::<Vec<_>>(),
                "files": infos.iter().filter(|i| i.kind != "image").map(|i| i.label.clone()).collect::<Vec<_>>(),
            }),
        );
        let mut text = text.to_string();
        if !infos.is_empty() {
            text += &format!("\n\n[The user attached {} file(s). Read each one with the Read tool before answering:\n", infos.len());
            text += &infos.iter().map(|i| format!("- {}  ({})", i.read, i.note)).collect::<Vec<_>>().join("\n");
            text += "]";
        }
        match kickoff {
            Some("new") => text += KICKOFF_NEW,
            Some("existing") => text += KICKOFF_EXISTING,
            Some("studio") => text += KICKOFF_STUDIO,
            _ => {}
        }
        // what is selected in Studio and any new errors there, when the Studio plugin is connected
        {
            let seen = self.st.lock().unwrap().log_seq;
            let (note, newest) = plugin::context_note(seen);
            text += &note;
            let mut st = self.st.lock().unwrap();
            st.log_seq = newest;
            st.turn_started = paths::now_secs();
            plugin::begin(st.turn_started);
        }
        // an overnight run starts with a checkpoint, so the whole night can be undone in the morning
        // (a Studio run going on after a gate is the same run, so it keeps its first checkpoint)
        if ask.overnight && !auto && !resumed {
            let start = self.turn_started();
            plugin::pin(start);
            let project = self.project().filter(|p| std::path::Path::new(p).is_dir());
            let commit = project.as_deref().and_then(projects::night_checkpoint);
            self.put("night", json!({"start": start, "commit": commit, "project": project, "studio": plugin::connected()}));
        }
        let notes = std::mem::take(&mut self.st.lock().unwrap().notes);
        if !notes.is_empty() {
            text += &format!("\n\n[{}]", notes.join("\n"));
        }
        self.put("busy", json!({"value": true}));
        let kind = self.ai();
        let Some(exe) = paths::find_ai(&kind) else {
            let missing = match kind.as_str() {
                "gemini" => "This chat is set to Gemini, but Google's Antigravity command-line tool (agy) was not found. Install it from https://antigravity.google, run `agy` once to sign in, then reopen Fridae Studios.",
                "chatgpt" => "This chat is set to ChatGPT, but OpenAI's command-line tool (codex) was not found. Install it (npm install -g @openai/codex), run `codex login` once, then reopen Fridae Studios.",
                _ => "This chat is set to Claude, but Claude Code is not installed. Install it from https://claude.com/claude-code, sign in once, then reopen Fridae Studios. Or pick another AI under the message box.",
            };
            self.put("error", json!({"text": missing}));
            self.put("done", json!({}));
            team::release(self);
            if let Some(left) = was_left {
                back_to_wait(&mut self.st.lock().unwrap(), left);
                self.put("studio_waiting", json!({"value": true, "left": left.round()}));
            }
            return;
        };
        if self.st.lock().unwrap().ai.is_none() {
            self.st.lock().unwrap().ai = Some(kind.clone());
            self.put("ai", json!({"ai": kind}));
        }
        // what the two switches under the message box add
        let switches = [(selftest, SELF_TEST_PROMPT), (gemini, GEMINI_PROMPT), (autopilot || ask.overnight, AUTOPILOT_PROMPT), (ask.overnight, OVERNIGHT_PROMPT), (ask.full, FULL_PROMPT), (ask.studio, STUDIO_PROMPT)].iter().filter(|(on, _)| *on).map(|(_, text)| *text).collect::<String>().replace("{work}", &PATHS.work.display().to_string());
        let mut prompt = SYSTEM_PROMPT.to_string() + &switches;
        let (project, role, expert_text) = {
            let st = self.st.lock().unwrap();
            (st.project.clone().filter(|p| std::path::Path::new(p).is_dir()), st.role.clone(), st.expert_text.clone())
        };
        if let Some(p) = &project {
            prompt += &PROJECT_PROMPT.replace("{path}", p);
        }
        match role.as_deref() {
            Some("critic") => prompt += CRITIC_PROMPT,
            Some("research") => prompt += RESEARCH_PROMPT,
            Some(_) => prompt += WORKER_PROMPT,
            None => {}
        }
        if ask.studio && role.is_none() {
            prompt += &studio_lead_text(project.as_deref());
        }
        if let Some(text) = expert_text.filter(|t| !t.trim().is_empty()) {
            prompt += &format!("\n\n{}", text.trim());
        }
        prompt += &team_prompt(self);
        let _ = fs::create_dir_all(&PATHS.work);
        let servers = mcp_servers(&self.id, &self.name(), project.as_deref(), role.as_deref());
        let session = self.st.lock().unwrap().session.clone();
        // every AI but Claude Code takes its standing instructions inside the conversation, and works in a folder
        // of its own (the project, or one made for this chat) because that is where it looks for its tool servers
        let workspace = project.as_ref().map(std::path::PathBuf::from).unwrap_or_else(|| paths::work(&kind).join(&self.id));
        if kind != "claude" {
            let _ = fs::create_dir_all(&workspace);
            prompt += OTHER_AI_PROMPT;
            text = match &session {
                None => antigravity::with_instructions(&prompt, &text),
                Some(_) if !switches.is_empty() => antigravity::with_instructions(&switches, &text),
                Some(_) => text,
            };
        }
        let mut cmd = match kind.as_str() {
            "gemini" => {
                text = antigravity::message_line(&text);
                antigravity::command(&exe, &workspace, &servers, session.as_deref(), model, effort, ask.full)
            }
            "chatgpt" => codex::command(&exe, &workspace, &servers, session.as_deref(), model, effort, ask.full),
            _ => {
                let mut cmd = Command::new(&exe);
                cmd.args(["-p", "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config"])
                    .arg(write_mcp_config(&self.id, &servers))
                    .args(["--allowedTools", "mcp__rbxkit", "mcp__Roblox_Studio", "Read", "Write", "Edit", "Glob", "Grep", "WebSearch", "WebFetch"])
                    // the programs the user allowed from a "blocked" card or Setup, through either shell
                    .args(paths::allowed_commands().iter().flat_map(|p| [format!("Bash({p}:*)"), format!("PowerShell({p}:*)")]))
                    // full auto skips every permission check, so nothing is ever refused
                    .args(["--permission-mode", if ask.full { "bypassPermissions" } else { "acceptEdits" }, "--append-system-prompt", &prompt, "--add-dir"])
                    .arg(&PATHS.kit);
                if let Some(p) = &project {
                    cmd.arg("--add-dir").arg(p);
                }
                if let Some(m) = model {
                    cmd.args(["--model", m]);
                }
                if let Some(e) = effort {
                    cmd.args(["--effort", e]);
                }
                if let Some(s) = &session {
                    cmd.args(["--resume", s]);
                }
                // the conversation lives in the project folder when there is one, so the agent works among the game's files
                cmd.current_dir(project.as_ref().map(std::path::PathBuf::from).unwrap_or_else(|| PATHS.work.clone()));
                cmd
            }
        };
        // a texture request may wait minutes for the user
        cmd.env("MCP_TOOL_TIMEOUT", "900000").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        paths::quiet(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.put("error", json!({"text": format!("Could not start the agent: {e}")}));
                self.put("done", json!({}));
                team::release(self);
                return;
            }
        };
        {
            let mut st = self.st.lock().unwrap();
            st.running = true;
            st.stopped = false;
            st.pid = Some(child.id());
        }
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let me = self.clone();
        thread::spawn(move || me.read(child, &kind));
    }

    /// The label shown while a tool runs: a plain phrase for the tools we know, the tool's own name otherwise.
    fn show_tool(&self, name: &str) {
        let known = FRIENDLY.iter().filter(|(k, _)| name == k.as_str() || name.ends_with(&format!("_{k}"))).max_by_key(|(k, _)| k.len());
        let label = match known {
            Some((_, v)) => v.as_str().map(str::to_string),
            None => Some(name.replace('_', " ")),
        };
        if let Some(label) = label.filter(|l| !l.is_empty()) {
            self.put("tool", json!({"label": label}));
        }
    }

    fn read(self: &Arc<Self>, mut child: std::process::Child, kind: &str) {
        let stderr = child.stderr.take();
        let err_reader = thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(mut e) = stderr {
                let _ = e.read_to_end(&mut s);
            }
            String::from_utf8_lossy(&s).trim().to_string()
        });
        let mut got_result = false;
        let (mut gemini, mut chatgpt) = (antigravity::Turn::default(), codex::Turn::default());
        if let Some(out) = child.stdout.take() {
            for raw in BufReader::new(out).split(b'\n').map_while(Result::ok) {
                let Ok(e) = serde_json::from_slice::<Value>(&raw) else { continue };
                if kind != "claude" {
                    let outs = if kind == "gemini" { gemini.feed(&e) } else { chatgpt.feed(&e) };
                    for out in outs {
                        match out {
                            Out::Conversation(id) => self.st.lock().unwrap().session = Some(id),
                            Out::Model(name) => self.put("model", json!({"name": name})),
                            Out::Tool(name) => self.show_tool(&name),
                            Out::Text(text) => self.put("text", json!({"text": text})),
                            Out::Pictures(paths) => {
                                let urls: Vec<String> = paths.iter().filter_map(|p| fs::read(p).ok()).filter_map(|bytes| attach::save_chat_image(&bytes)).collect();
                                if !urls.is_empty() {
                                    self.put("images", json!({"urls": urls}));
                                }
                            }
                            Out::Error(text) => self.put("error", json!({"text": text})),
                            Out::Usage(turn) => self.used(&turn),
                            Out::Finished => got_result = true,
                        }
                    }
                    continue;
                }
                if let Some(s) = e["session_id"].as_str().filter(|s| !s.is_empty()) {
                    self.st.lock().unwrap().session = Some(s.to_string());
                }
                match e["type"].as_str().unwrap_or("") {
                    "system" if e["subtype"] == "init" => {
                        if let Some(m) = e["model"].as_str().filter(|m| !m.is_empty()) {
                            self.put("model", json!({"name": pretty_model(m)}));
                        }
                        let bad: Vec<&str> = e["mcp_servers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|s| s["status"] != "connected")
                            .filter_map(|s| s["name"].as_str())
                            .collect();
                        if !bad.is_empty() {
                            self.put("error", json!({"text": format!("Could not connect to: {}. Open Setup to see what is missing.", bad.join(", "))}));
                        }
                    }
                    "assistant" => {
                        for b in e["message"]["content"].as_array().into_iter().flatten() {
                            match b["type"].as_str().unwrap_or("") {
                                "text" => {
                                    if let Some(t) = b["text"].as_str().filter(|t| !t.trim().is_empty()) {
                                        self.put("text", json!({"text": t}));
                                    }
                                }
                                "tool_use" => {
                                    let short = b["name"].as_str().unwrap_or("").rsplit("__").next().unwrap_or("").to_string();
                                    if let Some(id) = b["id"].as_str() {
                                        self.st.lock().unwrap().tools.insert(id.to_string(), short.clone());
                                    }
                                    self.show_tool(&short);
                                }
                                _ => {}
                            }
                        }
                    }
                    "user" => {
                        for b in e["message"]["content"].as_array().into_iter().flatten() {
                            let Some(parts) = b["content"].as_array().filter(|_| b["type"] == "tool_result") else { continue };
                            let tool = b["tool_use_id"].as_str().and_then(|id| self.st.lock().unwrap().tools.get(id).cloned()).unwrap_or_default();
                            // only pictures from our own tools and Studio screenshots are shown
                            if !(tool.starts_with("rbx_") || tool.starts_with("screen_capture")) {
                                continue;
                            }
                            let urls: Vec<String> = parts
                                .iter()
                                .filter(|c| c["type"] == "image" && c["source"]["type"] == "base64")
                                .filter_map(|c| c["source"]["data"].as_str())
                                .filter_map(attach::save_chat_image_b64)
                                .take(4)
                                .collect();
                            if !urls.is_empty() {
                                self.put("images", json!({"urls": urls}));
                            }
                        }
                    }
                    "result" => {
                        got_result = true;
                        self.used(&usage::Turn::from_claude(&e));
                        // commands the agent was not allowed to run: the page offers to allow each program
                        let mut blocked: Vec<Value> = Vec::new();
                        for d in e["permission_denials"].as_array().into_iter().flatten() {
                            let Some(cmd) = d["tool_input"]["command"].as_str() else { continue };
                            if let Some(name) = paths::program_name(cmd).filter(|n| !blocked.iter().any(|b| b["name"] == *n)) {
                                blocked.push(json!({"name": name, "command": cmd.chars().take(200).collect::<String>()}));
                            }
                        }
                        if !blocked.is_empty() {
                            self.put("blocked", json!({"commands": blocked}));
                        }
                        if e["is_error"].as_bool().unwrap_or(false) {
                            let msg = e["result"].as_str().or(e["subtype"].as_str()).filter(|s| !s.is_empty()).unwrap_or("The agent stopped with an error.");
                            self.put("error", json!({"text": msg}));
                        }
                    }
                    _ => {}
                }
            }
        }
        let err = err_reader.join().unwrap_or_default();
        let status = child.wait().ok();
        let stopped = {
            let mut st = self.st.lock().unwrap();
            st.running = false;
            st.pid = None;
            std::mem::take(&mut st.stopped)
        };
        if !got_result && err.contains("No conversation found") {
            self.st.lock().unwrap().session = None;
            self.put("error", json!({"text": "The memory of this earlier chat is no longer on this computer, so the agent has lost its context. Send your message again and it will carry on from what you tell it."}));
        } else if stopped {
            self.put("error", json!({"text": "Stopped."}));
        } else if let Some(why) = match kind {
            "gemini" => antigravity::explain(&err, got_result),
            "chatgpt" => codex::explain(&err).filter(|_| !got_result).map(str::to_string),
            _ => None,
        } {
            self.put("error", json!({"text": why}));
        } else if !got_result && !status.map(|s| s.success()).unwrap_or(true) {
            let tail: String = err.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
            let code = status.and_then(|s| s.code()).unwrap_or(-1);
            self.put("error", json!({"text": if tail.is_empty() { format!("The agent exited unexpectedly (code {code}).") } else { tail }}));
        }
        team::finished(self);
        research::finished(self);
        self.put("done", json!({}));
        // autopilot: the agent ends each reply by saying whether there is more it can do alone; if so, it is told to carry on
        let reply = self.last_reply();
        let last_line = reply.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        let said = last_line.to_uppercase();
        let held = std::mem::take(&mut self.st.lock().unwrap().queue);
        let mut night_over: Option<&str> = None;
        let mut waiting: Option<f64> = None;
        let carry_on: Option<(Ask, &str)> = {
            let mut st = self.st.lock().unwrap();
            let next = Ask { kickoff: Some("auto".to_string()), ..st.last_ask.clone() };
            let now = paths::now_secs();
            let night = st.overnight_until > now;
            let mut ls = LoopState { autopilot_left: st.autopilot_left, fails: st.overnight_fails, done: st.overnight_done, night };
            let step = next_step(loop_mode(stopped, !held.is_empty(), night, st.last_ask.studio), got_result, &said, &mut ls);
            st.autopilot_left = ls.autopilot_left;
            st.overnight_fails = ls.fails;
            st.overnight_done = ls.done;
            if step == Step::Wait {
                st.studio_waiting = true;
                st.overnight_left = (st.overnight_until - now).max(0.0);
                waiting = Some(st.overnight_left);
            }
            if stopped || (night && !ls.night) {
                st.overnight_until = 0.0;
            }
            match step {
                Step::CarryOn(say) => Some((next, say)),
                Step::End(why) => {
                    night_over = Some(why);
                    None
                }
                Step::Wait | Step::Idle => None,
            }
        };
        if let Some(left) = waiting {
            self.put("studio_waiting", json!({"value": true, "left": left.round()}));
        }
        if let Some(why) = night_over {
            self.put("error", json!({"text": why}));
        }
        // the board follows the chat: still working, waiting on the user, or finished
        if let Some(p) = self.project() {
            let first = board::gist(reply.trim_start(), 200);
            let (column, note) = if !held.is_empty() || carry_on.is_some() {
                ("doing", None)
            } else if stopped {
                ("todo", Some("Stopped before it finished.".to_string()))
            } else if waiting.is_some() {
                ("needs", Some("Waiting for your answer in Review".to_string()))
            } else if said.starts_with("STATUS: NEEDS YOU") {
                ("needs", Some(last_line.splitn(2, ':').nth(1).and_then(|r| r.splitn(2, ':').nth(1)).unwrap_or("").trim().to_string()))
            } else if !got_result {
                ("needs", Some(format!("It stopped with an error: {first}")))
            } else {
                ("done", Some(first))
            };
            board::track(&p, &self.id, &self.name(), column, None, note.as_deref());
        }
        if !held.is_empty() {
            // what the user sent while it worked goes next, in one message, with the newest settings
            self.show_queue();
            let text = held.iter().map(|(_, t, _)| t.as_str()).collect::<Vec<_>>().join("\n\n");
            let ask = Ask { kickoff: None, attachments: held.iter().flat_map(|(_, _, a)| a.attachments.clone()).collect(), ..held.last().unwrap().2.clone() };
            let me = self.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(400));
                me.send(&text, &ask);
            });
        } else if let Some((ask, say)) = carry_on {
            let me = self.clone();
            let say = say.to_string();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(800));
                me.send(&say, &ask);
            });
        }
    }

    pub fn stop(&self) {
        team::release(self); // a stopped expert no longer holds the game
        let (pid, making) = {
            let mut st = self.st.lock().unwrap();
            if !(st.running || st.making) {
                // between two turns of full auto or the night: end it there
                st.overnight_until = 0.0;
                st.autopilot_left = 0;
                st.halted = true;
                let was_waiting = std::mem::take(&mut st.studio_waiting);
                st.overnight_left = 0.0;
                drop(st);
                if was_waiting {
                    self.put("studio_waiting", json!({"value": false, "left": 0}));
                }
                return;
            }
            st.stopped = true;
            (st.pid.filter(|_| st.running), st.making)
        };
        if let Some(pid) = pid {
            let mut kill = Command::new("taskkill");
            kill.args(["/T", "/F", "/PID", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
            paths::quiet(&mut kill);
            let _ = kill.status();
        }
        if making {
            textures::skip_for(&self.id); // the waiting texture tool then ends by itself
        }
    }
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// claude-fable-5-2 -> Claude Fable 5.2
pub fn pretty_model(model_id: &str) -> String {
    let (mut words, mut nums) = (Vec::new(), Vec::new());
    for part in model_id.split('[').next().unwrap_or("").split('-') {
        let digits = !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
        if digits && part.len() < 4 {
            nums.push(part.to_string());
        } else if !digits && !part.is_empty() {
            words.push(capitalize(part));
        }
    }
    let mut out = words.join(" ");
    if !nums.is_empty() {
        out += &format!(" {}", nums.join("."));
    }
    let out = out.trim().to_string();
    if out.is_empty() { "Claude".into() } else { out }
}

/// Start another chat the user can give its own task to.
pub fn new_agent(ai: Option<&str>) -> Option<Arc<Agent>> {
    let agent = {
        let mut all = AGENTS.lock().unwrap();
        if all.len() >= MAX_AGENTS {
            return None;
        }
        let n = COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        let agent = Agent::new(format!("c{}-{n}", (paths::now_secs() * 1000.0) as u64), "New chat");
        all.push(agent.clone());
        agent
    };
    // a chat made for one AI's list says so from the start; one made without a choice is claimed by the window
    let ai = ai.filter(|a| paths::AIS.contains(a));
    agent.st.lock().unwrap().ai = ai.map(str::to_string);
    agent.put("agent_new", json!({"name": agent.name(), "ai": ai}));
    Some(agent)
}

/// Start an agent with a role of its own: a worker or reviewer for another agent (`team_id` is that
/// request), or the Research tab's researcher.
pub fn spawn_helper(team_id: Option<&str>, role: &str, name: &str, project: Option<String>, ai: Option<String>, parent: Option<&str>) -> Option<Arc<Agent>> {
    spawn_named(team_id, Some(role), name, project, ai, parent, None)
}

/// A named Studio expert: a helper whose chat also gets the expert's own instructions (`id`, display `name`, `text`).
pub struct Expert {
    pub id: String,
    pub name: String,
    pub text: String,
}

pub fn spawn_expert(team_id: &str, role: &str, title: &str, project: Option<String>, ai: Option<String>, parent: Option<&str>, expert: Expert) -> Option<Arc<Agent>> {
    spawn_named(Some(team_id), Some(role), title, project, ai, parent, Some(expert))
}

/// Start a chat of the user's own (no role) in the background, already named and in its project: the Promos tab's.
pub fn new_background_chat(name: &str, project: Option<String>) -> Option<Arc<Agent>> {
    spawn_named(None, None, name, project, None, None, None)
}

fn spawn_named(team_id: Option<&str>, role: Option<&str>, name: &str, project: Option<String>, ai: Option<String>, parent: Option<&str>, expert: Option<Expert>) -> Option<Arc<Agent>> {
    let agent = {
        let mut all = AGENTS.lock().unwrap();
        if all.len() >= MAX_AGENTS {
            return None;
        }
        let n = COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        let title: String = name.chars().take(34).collect();
        let agent = Agent::new(format!("c{}-{n}", (paths::now_secs() * 1000.0) as u64), if title.trim().is_empty() { "Helper" } else { &title });
        {
            let mut st = agent.st.lock().unwrap();
            st.named = true;
            st.role = role.map(str::to_string);
            st.team = team_id.map(str::to_string);
            if let Some(x) = &expert {
                st.expert = Some(x.id.clone());
                st.expert_text = Some(x.text.clone());
            }
            st.ai = Some(ai.unwrap_or_else(paths::default_ai));
            st.project = project;
        }
        all.push(agent.clone());
        agent
    };
    let mut event = json!({"agent": agent.id, "name": agent.name(), "role": role, "background": true, "ai": agent.ai(), "parent": parent});
    if let Some(x) = &expert {
        event["expert"] = json!(x.name);
    }
    BUS.put("agent_new", event);
    if let Some(p) = agent.project() {
        agent.put("project", projects::describe(&p));
    }
    Some(agent)
}

/// While any chat is in overnight mode, keep Windows from going to sleep (the screen may still turn off).
pub fn keep_awake() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetThreadExecutionState(flags: u32) -> u32;
        }
        const CONTINUOUS: u32 = 0x8000_0000;
        const SYSTEM_REQUIRED: u32 = 0x0000_0001;
        thread::spawn(|| {
            let mut was = false;
            loop {
                let now = agents().iter().any(|a| a.overnight());
                if now != was {
                    // SAFETY: a plain Windows call with constant flags; it only changes this thread's sleep request
                    unsafe { SetThreadExecutionState(if now { CONTINUOUS | SYSTEM_REQUIRED } else { CONTINUOUS }) };
                    was = now;
                }
                thread::sleep(Duration::from_secs(20));
            }
        });
    }
}

/// Note which chats are open, so they come back when the app is started again.
pub fn remember_open() {
    let dir = paths::work("chats");
    let _ = fs::create_dir_all(&dir);
    let ids: Vec<String> = agents().iter().filter(|a| a.has_messages()).map(|a| a.id.clone()).collect();
    let _ = fs::write(dir.join("_open.txt"), ids.join("\n"));
}

/// Reopen the chats that were open last time.
pub fn restore_open() -> usize {
    let Ok(text) = fs::read_to_string(paths::work("chats").join("_open.txt")) else { return 0 };
    text.lines().map(str::trim).filter(|l| !l.is_empty()).take(MAX_AGENTS).filter(|id| open_chat(id).is_some()).count()
}

fn chat_file(chat_id: &str) -> std::path::PathBuf {
    paths::work("chats").join(format!("{}.json", paths::base_name(chat_id)))
}

/// Saved chats that are not open right now, newest first.
pub fn history() -> Vec<Value> {
    let open: Vec<String> = agents().iter().map(|a| a.id.clone()).collect();
    saved_in(&paths::work("chats"), &open)
}

/// Where deleted chats are kept. Deleting only moves a chat here, so it can be put back.
fn deleted_dir() -> std::path::PathBuf {
    paths::work("chats").join("deleted")
}

/// Chats that were deleted and can still be put back, newest first.
pub fn deleted() -> Vec<Value> {
    saved_in(&deleted_dir(), &[])
}

fn saved_in(dir: &std::path::Path, open: &[String]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else { continue };
        if open.iter().any(|id| id == stem) {
            continue;
        }
        let Some(d) = fs::read_to_string(entry.path()).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { continue };
        out.push(json!({
            "id": d["id"].as_str().unwrap_or(stem),
            "name": d["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("Chat"),
            "updated": d["updated"].as_f64().unwrap_or(0.0),
            "messages": d["events"].as_array().map(|e| e.iter().filter(|e| is_user(e)).count()).unwrap_or(0),
            "ai": d["ai"].as_str().unwrap_or("claude"),
        }));
    }
    out.sort_by(|a, b| b["updated"].as_f64().partial_cmp(&a["updated"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(200);
    out
}

/// Reopen a saved chat: its messages come back, and the agent picks up where it left off.
pub fn open_chat(chat_id: &str) -> Option<Arc<Agent>> {
    let chat_id = paths::base_name(chat_id);
    if let Some(a) = get(&chat_id) {
        return Some(a);
    }
    let d: Value = serde_json::from_str(&fs::read_to_string(chat_file(&chat_id)).ok()?).ok()?;
    let agent = Agent::new(chat_id, d["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("Chat"));
    {
        let mut all = AGENTS.lock().unwrap();
        if all.len() >= MAX_AGENTS {
            return None;
        }
        all.push(agent.clone());
    }
    BUS.put("agent_new", json!({"agent": agent.id, "name": agent.name(), "restored": true, "ai": d["ai"].as_str().unwrap_or("claude")}));
    let events = d["events"].as_array().cloned().unwrap_or_default();
    for e in &events {
        let Some(kind) = e["kind"].as_str() else { continue };
        let mut shown = e.as_object().cloned().unwrap_or_default();
        shown.remove("kind");
        shown.insert("agent".into(), json!(agent.id));
        BUS.put(kind, Value::Object(shown));
    }
    let settled = events.last().map(|e| e["kind"] == "done").unwrap_or(false);
    {
        let mut st = agent.st.lock().unwrap();
        st.named = true;
        st.session = d["session"].as_str().map(str::to_string);
        st.project = d["project"].as_str().map(str::to_string);
        st.role = d["role"].as_str().map(str::to_string);
        st.expert = d["expert"].as_str().map(str::to_string);
        // chats saved before there was a choice were all Claude's
        st.ai = Some(d["ai"].as_str().unwrap_or("claude").to_string());
        st.log = events.into_iter().filter(|e| e["kind"].is_string()).collect();
    }
    if !settled {
        BUS.put("done", json!({"agent": agent.id})); // it was closed mid-reply: settle the view
    }
    // an idle fresh chat that was only a placeholder makes way for the reopened one
    let placeholders: Vec<Arc<Agent>> = agents().into_iter().filter(|o| o.id != agent.id && o.st.lock().unwrap().log.is_empty() && !o.busy()).collect();
    for other in placeholders {
        AGENTS.lock().unwrap().retain(|a| a.id != other.id);
        BUS.put("agent_removed", json!({"agent": other.id}));
    }
    Some(agent)
}

/// Delete a saved chat. Nothing is erased: its file moves to the deleted folder, from where it can be put back.
pub fn delete_chat(chat_id: &str) -> bool {
    let chat_id = paths::base_name(chat_id);
    if get(&chat_id).is_some() {
        remove_agent(&chat_id);
    }
    let _ = fs::create_dir_all(deleted_dir());
    let moved = fs::rename(chat_file(&chat_id), deleted_dir().join(format!("{chat_id}.json"))).is_ok();
    remember_open();
    moved
}

/// Put a deleted chat back among the saved ones.
pub fn restore_chat(chat_id: &str) -> bool {
    let chat_id = paths::base_name(chat_id);
    fs::rename(deleted_dir().join(format!("{chat_id}.json")), chat_file(&chat_id)).is_ok()
}

pub fn remove_agent(agent_id: &str) {
    let found = {
        let mut all = AGENTS.lock().unwrap();
        let hit = all.iter().find(|a| a.id == agent_id).cloned();
        all.retain(|a| a.id != agent_id);
        hit
    };
    if let Some(agent) = found {
        agent.stop();
        team::release(&agent);
        agent.save();
        BUS.put("agent_removed", json!({"agent": agent.id}));
        remember_open();
    }
    if AGENTS.lock().unwrap().is_empty() {
        new_agent(None);
    }
}

/// Stop every agent and save every chat: called when the window closes.
pub fn shut_down() {
    crate::promo::stop_install();
    for a in agents() {
        a.stop();
        a.save();
    }
}

/// The Lead's instructions for a Studio run: the game's own `.fridae/team/experts/lead.md` when it has one, else the
/// kit's `team/experts/lead.md`. Empty when neither can be read (the studio prompt still applies).
fn studio_lead_text(project: Option<&str>) -> String {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = project {
        files.push(std::path::Path::new(p).join(".fridae").join("team").join("experts").join("lead.md"));
    }
    files.push(PATHS.kit.join("team").join("experts").join("lead.md"));
    files.iter().find_map(|f| fs::read_to_string(f).ok()).filter(|t| !t.trim().is_empty()).map(|t| format!("\n\n{}", t.trim())).unwrap_or_default()
}

/// Extra instructions when several agents share one Studio place.
fn team_prompt(me: &Agent) -> String {
    let others: Vec<String> = agents()
        .iter()
        .filter(|a| a.id != me.id && (a.st.lock().unwrap().named || a.busy()))
        .map(|a| format!("\"{}\"", a.name()))
        .collect();
    if others.is_empty() {
        return String::new();
    }
    [
        "".to_string(),
        "".to_string(),
        format!("YOU ARE ONE OF SEVERAL AGENTS working at the same time in the same Roblox Studio place. You are \"{}\".", me.name()),
        format!("The others are working on: {}.", others.join("; ")),
        "- Stay strictly inside your own task. Do not edit, move or delete anything you did not create unless your task names it.".to_string(),
        "- Give what you create distinct names, and use your own model names in Blender, so work does not collide.".to_string(),
        "- Studio has one playtest and one camera for everyone. Before start_stop_play, a VFX capture, the icon booth, or any".to_string(),
        "  bulk change to the place, call rbx_studio_lock with action \"acquire\"; call it with \"release\" as soon as you finish.".to_string(),
        "  If another agent holds it, do work that does not need it, then try again. Never stop a playtest you did not start.".to_string(),
        "- Say in your reply if another agent's work blocked or changed what you were doing.".to_string(),
    ]
    .join("\n")
}

/// The tool servers an agent gets: ours, and Roblox Studio's when its connector is on.
fn mcp_servers(agent_id: &str, agent_name: &str, project: Option<&str>, role: Option<&str>) -> Value {
    let cmd = paths::mcp_command();
    let mut servers = json!({
        "rbxkit": {"type": "stdio", "command": cmd[0], "args": cmd[1..], "env": {
            "RBXKIT_AGENT_ID": agent_id, "RBXKIT_AGENT_NAME": agent_name,
            "RBXKIT_PROJECT": project.unwrap_or(""), "RBXKIT_AGENT_ROLE": role.unwrap_or(""),
        }}
    });
    // FRIDAE_NO_STUDIO (testing only): a copy of the app that cannot reach the user's open Studio place
    if let Some(studio) = paths::studio_mcp().filter(|_| std::env::var_os("FRIDAE_NO_STUDIO").is_none()) {
        servers["Roblox_Studio"] = if cfg!(windows) {
            json!({"type": "stdio", "command": "cmd.exe", "args": ["/c", "cd /d %LOCALAPPDATA%\\Roblox && .\\mcp.bat"]})
        } else {
            json!({"type": "stdio", "command": studio.display().to_string(), "args": ["--stdio"]})
        };
    }
    servers
}

fn write_mcp_config(agent_id: &str, servers: &Value) -> std::path::PathBuf {
    let path = PATHS.work.join(format!("agent-tools-{agent_id}.json"));
    let _ = fs::write(&path, json!({"mcpServers": servers}).to_string());
    path
}

/// Run the setup check and show its report in the window.
pub fn run_setup() {
    let text = match paths::cli(&["doctor"]).output() {
        Ok(o) => {
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if out.is_empty() { String::from_utf8_lossy(&o.stderr).trim().to_string() } else { out }
        }
        Err(e) => format!("Could not run the setup check: {e}"),
    };
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("In Claude")).map(str::to_string).collect();
    let row = |found: Option<std::path::PathBuf>, name: &str, missing: &str| {
        format!("  [{}] {:<18} {}", if found.is_some() { "ok" } else { "!!" }, name, found.map(|p| p.display().to_string()).unwrap_or(missing.to_string()))
    };
    let at = lines.len().saturating_sub(1);
    lines.insert(at, row(paths::find_agent(), "Claude Code", "not found: install it and sign in"));
    let at = lines.len().saturating_sub(1);
    lines.insert(at, row(paths::studio_mcp(), "Studio MCP", "off: turn it on in Studio's Assistant settings"));
    BUS.put("setup", json!({"text": lines.join("\n")}));
}

/// Add the rbxkit tool server to other agents found on this computer.
pub fn connect_other_agents() -> String {
    let home = paths::home_dir();
    let cmd = paths::mcp_command();
    let entry = json!({"command": cmd[0], "args": cmd[1..]});
    let mut done: Vec<&str> = Vec::new();
    // (a folder that shows the app is installed, its settings file, its name)
    let gemini = home.join(".gemini");
    let antigravity = ["antigravity", "antigravity-cli", "config"].iter().any(|d| gemini.join(d).is_dir());
    let targets = [
        (home.join(".cursor"), home.join(".cursor").join("mcp.json"), "Cursor"),
        (home.join(".codeium").join("windsurf"), home.join(".codeium").join("windsurf").join("mcp_config.json"), "Windsurf"),
        (gemini.clone(), gemini.join("settings.json"), "Gemini CLI"),
        // Antigravity (the app, the editor and the `agy` command) reads one shared file; older versions read their own
        (if antigravity { gemini.clone() } else { gemini.join("no-antigravity") }, gemini.join("config").join("mcp_config.json"), "Antigravity"),
        (gemini.join("antigravity"), gemini.join("antigravity").join("mcp_config.json"), "Antigravity (older versions)"),
        (gemini.join("antigravity-cli"), gemini.join("antigravity-cli").join("mcp_config.json"), "Antigravity CLI (older versions)"),
    ];
    for (folder, path, label) in targets {
        if !folder.is_dir() {
            continue;
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut cfg = json!({});
        if path.exists() {
            match fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object) {
                Some(v) => cfg = v,
                None => continue, // a settings file we cannot read is left alone
            }
        }
        if !cfg["mcpServers"].is_object() {
            cfg["mcpServers"] = json!({});
        }
        cfg["mcpServers"]["rbxkit"] = entry.clone();
        if fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap_or_default()).is_ok() {
            done.push(label);
        }
    }
    let codex = home.join(".codex").join("config.toml");
    if home.join(".codex").is_dir() {
        let text = fs::read_to_string(&codex).unwrap_or_default();
        if !text.contains("[mcp_servers.rbxkit]") {
            let add = format!("\n[mcp_servers.rbxkit]\ncommand = {}\nargs = {}\n", json!(cmd[0]), json!(cmd[1..]));
            let _ = fs::write(&codex, text + &add);
        }
        done.push("Codex");
    }
    let head = if done.is_empty() {
        "No other agents were found on this computer.".to_string()
    } else {
        format!("Connected to: {}. Restart them to pick it up.", done.join(", "))
    };
    format!(
        "{head}\n\nThose apps can now use the building tools from their own chat. The chat in this window only runs on Claude Code, \
         so the agent dot here stays empty without it.\nIn Antigravity, reload the servers after this: type /mcp in the CLI, or open MCP \
         Servers from the agent panel's menu. Also connect Roblox Studio there (Studio's Assistant settings list how).\n\n\
         For any other agent that supports MCP servers, add:\n{}",
        serde_json::to_string_pretty(&json!({"mcpServers": {"rbxkit": entry}})).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIGHT_NEXT: &str = "Overnight mode: within any limits I set in this chat, pick the next most valuable thing you can do without me (the board's todo cards first, then what GAME.md says is missing, then bugs, polish and playtesting) and do it. If nothing inside my limits is left, say so and end with STATUS: DONE.";
    const NIGHT_AWAY: &str = "I am away. Put what you need from me on the board under needs if it is not there yet, then carry on with the next most valuable thing you can do without me.";
    const FAILED: &str = "The last turn failed. Find out what went wrong, fix or work around it, then carry on.";
    const NEXT: &str = "Carry on with the next step.";

    fn night() -> LoopState {
        LoopState { autopilot_left: AUTOPILOT_TURNS, fails: 0, done: 0, night: true }
    }

    #[test]
    fn mode_is_chosen_as_before() {
        assert_eq!(loop_mode(true, true, true, true), Mode::Stopped);
        assert_eq!(loop_mode(false, true, true, true), Mode::Held);
        assert_eq!(loop_mode(false, false, true, false), Mode::Overnight);
        assert_eq!(loop_mode(false, false, true, true), Mode::Studio);
        assert_eq!(loop_mode(false, false, false, true), Mode::StudioLate);
        assert_eq!(loop_mode(false, false, false, false), Mode::Autopilot);
    }

    #[test]
    fn stopped_and_held_carry_nothing() {
        let mut s = night();
        assert_eq!(next_step(Mode::Stopped, true, "STATUS: CONTINUE", &mut s), Step::Idle);
        assert_eq!((s.autopilot_left, s.night), (0, false));
        let mut s = night();
        assert_eq!(next_step(Mode::Held, true, "STATUS: CONTINUE", &mut s), Step::Idle);
        assert_eq!((s.autopilot_left, s.night), (0, true)); // a held message does not end the night
    }

    #[test]
    fn overnight_carries_on_with_the_same_messages() {
        let mut s = night();
        assert_eq!(next_step(Mode::Overnight, true, "SOME TEXT", &mut s), Step::CarryOn(NEXT));
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: CONTINUE", &mut s), Step::CarryOn(NEXT));
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: NEEDS YOU: A KEY", &mut s), Step::CarryOn(NIGHT_AWAY));
        assert_eq!((s.fails, s.done, s.night), (0, 0, true));
        assert_eq!(s.autopilot_left, AUTOPILOT_TURNS); // the night does not spend autopilot's allowance
    }

    #[test]
    fn overnight_done_twice_ends() {
        let mut s = night();
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: DONE", &mut s), Step::CarryOn(NIGHT_NEXT));
        assert_eq!((s.done, s.night), (1, true));
        assert_eq!(
            next_step(Mode::Overnight, true, "STATUS: DONE", &mut s),
            Step::End("Overnight mode finished: twice in a row there was nothing more it could do without you. See the board for what needs you.")
        );
        assert!(!s.night);
    }

    #[test]
    fn overnight_done_count_resets_on_other_lines() {
        let mut s = night();
        next_step(Mode::Overnight, true, "STATUS: DONE", &mut s);
        next_step(Mode::Overnight, true, "STATUS: NEEDS YOU: X", &mut s);
        assert_eq!(s.done, 0);
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: DONE", &mut s), Step::CarryOn(NIGHT_NEXT));
        next_step(Mode::Overnight, true, "STATUS: CONTINUE", &mut s);
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: DONE", &mut s), Step::CarryOn(NIGHT_NEXT));
    }

    #[test]
    fn overnight_three_failures_end() {
        let mut s = night();
        assert_eq!(next_step(Mode::Overnight, false, "", &mut s), Step::CarryOn(FAILED));
        assert_eq!(next_step(Mode::Overnight, false, "", &mut s), Step::CarryOn(FAILED));
        assert_eq!(next_step(Mode::Overnight, false, "", &mut s), Step::End("Overnight mode stopped: three turns in a row failed."));
        assert!(!s.night);
    }

    #[test]
    fn overnight_failures_reset_on_success() {
        let mut s = night();
        next_step(Mode::Overnight, false, "", &mut s);
        next_step(Mode::Overnight, false, "", &mut s);
        next_step(Mode::Overnight, true, "STATUS: CONTINUE", &mut s);
        assert_eq!(s.fails, 0);
        assert_eq!(next_step(Mode::Overnight, false, "", &mut s), Step::CarryOn(FAILED));
        // a failure does not touch the done count
        let mut s = night();
        next_step(Mode::Overnight, true, "STATUS: DONE", &mut s);
        next_step(Mode::Overnight, false, "", &mut s);
        assert_eq!((s.fails, s.done), (1, 1));
    }

    #[test]
    fn autopilot_unchanged() {
        let mut s = LoopState { autopilot_left: 2, ..Default::default() };
        assert_eq!(next_step(Mode::Autopilot, true, "STATUS: CONTINUE", &mut s), Step::CarryOn(NEXT));
        assert_eq!(s.autopilot_left, 1);
        assert_eq!(next_step(Mode::Autopilot, true, "STATUS: CONTINUE", &mut s), Step::CarryOn(NEXT));
        assert_eq!(s.autopilot_left, 0);
        assert_eq!(next_step(Mode::Autopilot, true, "STATUS: CONTINUE", &mut s), Step::Idle);
        let mut s = LoopState { autopilot_left: 5, ..Default::default() };
        assert_eq!(next_step(Mode::Autopilot, true, "STATUS: DONE", &mut s), Step::Idle);
        assert_eq!(s.autopilot_left, 0);
        let mut s = LoopState { autopilot_left: 5, ..Default::default() };
        assert_eq!(next_step(Mode::Autopilot, false, "STATUS: CONTINUE", &mut s), Step::Idle);
        assert_eq!(s.autopilot_left, 0);
    }

    #[test]
    fn studio_waiting_waits_and_resets_counters() {
        let mut s = night();
        next_step(Mode::Studio, true, "STATUS: DONE", &mut s);
        next_step(Mode::Studio, false, "", &mut s);
        assert_eq!((s.fails, s.done), (1, 1));
        assert_eq!(next_step(Mode::Studio, true, "STATUS: WAITING", &mut s), Step::Wait);
        assert_eq!((s.fails, s.done, s.night), (0, 0, false)); // the clock stops
    }

    #[test]
    fn studio_done_twice_ends() {
        let mut s = night();
        assert!(matches!(next_step(Mode::Studio, true, "STATUS: DONE", &mut s), Step::CarryOn(_)));
        assert!(matches!(next_step(Mode::Studio, true, "STATUS: DONE", &mut s), Step::End(_)));
        assert!(!s.night);
    }

    #[test]
    fn studio_three_failures_end_and_failed_turn_never_waits() {
        let mut s = night();
        assert_eq!(next_step(Mode::Studio, false, "STATUS: WAITING", &mut s), Step::CarryOn(FAILED));
        next_step(Mode::Studio, false, "", &mut s);
        assert!(matches!(next_step(Mode::Studio, false, "", &mut s), Step::End(_)));
    }

    #[test]
    fn studio_other_lines_carry_on() {
        let mut s = night();
        assert!(matches!(next_step(Mode::Studio, true, "STATUS: CONTINUE", &mut s), Step::CarryOn(_)));
        assert!(matches!(next_step(Mode::Studio, true, "STATUS: NEEDS YOU: A KEY", &mut s), Step::CarryOn(_)));
        assert_eq!((s.fails, s.done, s.night), (0, 0, true));
    }

    #[test]
    fn waiting_outside_studio_is_an_ordinary_line() {
        let mut s = night();
        assert_eq!(next_step(Mode::Overnight, true, "STATUS: WAITING", &mut s), Step::CarryOn(NEXT));
        assert!(s.night);
        let mut s = LoopState { autopilot_left: 5, ..Default::default() };
        assert_eq!(next_step(Mode::Autopilot, true, "STATUS: WAITING", &mut s), Step::Idle);
        assert_eq!(s.autopilot_left, 0);
    }

    #[test]
    fn studio_late_waits_but_otherwise_is_autopilot() {
        let mut s = LoopState { autopilot_left: 3, fails: 1, done: 1, night: false };
        assert_eq!(next_step(Mode::StudioLate, true, "STATUS: WAITING", &mut s), Step::Wait);
        assert_eq!((s.fails, s.done, s.night), (0, 0, false));
        let mut s = LoopState { autopilot_left: 3, ..Default::default() };
        assert_eq!(next_step(Mode::StudioLate, false, "STATUS: WAITING", &mut s), Step::Idle);
        let mut s = LoopState { autopilot_left: 3, ..Default::default() };
        assert_eq!(next_step(Mode::StudioLate, true, "STATUS: CONTINUE", &mut s), Step::CarryOn(NEXT));
        assert_eq!(s.autopilot_left, 2);
        assert_eq!(next_step(Mode::StudioLate, true, "STATUS: DONE", &mut s), Step::Idle);
        assert_eq!(s.autopilot_left, 0);
    }

    #[test]
    fn studio_needs_a_project() {
        assert_eq!(studio_refusal(true, false), Some(STUDIO_NEEDS_PROJECT));
        assert_eq!(studio_refusal(true, true), None);
        assert_eq!(studio_refusal(false, false), None);
    }

    #[test]
    fn studio_implies_overnight_and_full() {
        let a = effective_ask(&Ask { studio: true, ..Default::default() }, false, &Ask::default());
        assert!(a.studio && a.overnight && a.full);
        let a = effective_ask(&Ask { overnight: true, ..Default::default() }, false, &Ask::default());
        assert!(!a.studio && a.overnight && !a.full); // the server turns full on for overnight, as before
    }

    #[test]
    fn a_waiting_run_resumes_in_studio_mode_whatever_was_sent() {
        let last = Ask { studio: true, overnight: true, full: true, ..Default::default() };
        let plain = Ask { model: Some("opus".into()), ..Default::default() };
        let a = effective_ask(&plain, true, &last);
        assert!(a.studio && a.overnight && a.full);
        assert_eq!(a.model.as_deref(), Some("opus"));
        // not waiting: the message's own mode wins
        assert!(!effective_ask(&plain, false, &last).studio);
    }

    #[test]
    fn begin_send_leaves_a_waiting_run_waiting_until_the_turn_starts() {
        let mut st = State { studio_waiting: true, overnight_left: 500.0, overnight_until: 0.0, ..Default::default() };
        let ask = effective_ask(&Ask::default(), true, &Ask { studio: true, overnight: true, full: true, ..Default::default() });
        assert_eq!(begin_send(&mut st, &ask, false, 1000.0), Some(true));
        assert!(st.studio_waiting);
        assert_eq!((st.overnight_until, st.overnight_left), (0.0, 500.0));
        assert!(st.last_ask.studio);
        // the turn starts: full 24 hours from now
        assert_eq!(resume_from_wait(&mut st, 1000.0), Some(500.0));
        assert!(!st.studio_waiting);
        assert_eq!((st.overnight_until, st.overnight_left), (1000.0 + OVERNIGHT_SECS, 0.0));
        assert_eq!(resume_from_wait(&mut st, 1000.0), None);
        // no AI after all: back to waiting as it was
        back_to_wait(&mut st, 500.0);
        assert!(st.studio_waiting);
        assert_eq!((st.overnight_until, st.overnight_left), (0.0, 500.0));
    }

    #[test]
    fn begin_send_unchanged_for_ordinary_messages() {
        let mut st = State { overnight_fails: 2, overnight_done: 1, ..Default::default() };
        let night = Ask { overnight: true, full: true, ..Default::default() };
        assert_eq!(begin_send(&mut st, &night, false, 1000.0), Some(false));
        assert_eq!((st.autopilot_left, st.overnight_until, st.overnight_fails, st.overnight_done), (AUTOPILOT_TURNS, 1000.0 + OVERNIGHT_SECS, 0, 0));
        // its own carry-on spends, and does not reset the clock
        st.overnight_fails = 1;
        assert_eq!(begin_send(&mut st, &Ask { kickoff: Some("auto".into()), ..night.clone() }, true, 5000.0), Some(false));
        assert_eq!((st.overnight_until, st.overnight_fails), (1000.0 + OVERNIGHT_SECS, 1));
        assert!(st.last_ask.kickoff.is_none());
        // Stop between turns: the carry-on is dropped once
        st.halted = true;
        assert_eq!(begin_send(&mut st, &night, true, 5000.0), None);
        assert!(!st.halted);
        // a plain message turns the night and autopilot off
        assert_eq!(begin_send(&mut st, &Ask::default(), false, 5000.0), Some(false));
        assert_eq!((st.autopilot_left, st.overnight_until), (0, 0.0));
    }

    #[test]
    fn studio_prompt_and_kickoff_name_the_real_tools() {
        for word in ["rbx_stage", "rbx_team", "decisions", "STATUS: WAITING", "`studio`", "replaces", "wait"] {
            assert!(STUDIO_PROMPT.contains(word), "studio.txt should mention {word}");
        }
        for word in ["design", "idea", "scout", "GAME.md", "the plan for this run"] {
            assert!(KICKOFF_STUDIO.contains(word), "kickoff_studio.txt should mention {word}");
        }
    }
}
