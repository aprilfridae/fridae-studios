//! The event log the window reads, the agents (one headless Claude Code process
//! per chat), and saved chats.

use std::{
    collections::HashMap,
    fs,
    io::{BufReader, Write},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{
    antigravity::{self, Out},
    attach, board,
    claude_live::{self, Live},
    codex, cursor, opencode,
    paths::{self, PATHS},
    plugin, projects, research, studio_undo, team, textures, usage,
};

pub const MAX_AGENTS: usize = 8;
pub const MAX_ATTACH: usize = 10;
/// The error text a turn ended by the stall guard carries to `after_turn`.
const STALLED: &str = "\u{0}stalled";
/// How long a turn's leader gets to exit by itself once its output has closed, before its group is ended (Cursor).
const LEADER_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
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
    /// A message the app itself sends to a studio run's Lead (Resume, a decision, a free slot, a board card), not one
    /// the user typed: Stop and End this run drop it from the queue (`drop_wakes`).
    pub wake: bool,
    /// A message the page sends for the user without their words (the blocked-command card's "You may now run ...").
    /// Like a wake, it is left out of the chat handed to another AI (`handover_text`).
    pub app: bool,
    /// When typed messages and app messages go out as one turn: the typed part only, for the chat handed to another AI.
    pub typed: Option<String>,
}

/// Messages that go out as one turn: when typed ones and the app's own are mixed, the typed part (see `Ask::typed`).
fn merged_typed(parts: &[(&str, &Ask)]) -> Option<String> {
    let app = |a: &Ask| a.wake || a.app;
    let mixed = parts.iter().any(|(_, a)| app(a)) && parts.iter().any(|(_, a)| !app(a));
    mixed.then(|| parts.iter().filter(|(_, a)| !app(a)).map(|(t, a)| a.typed.clone().unwrap_or_else(|| t.to_string())).collect::<Vec<_>>().join("\n\n"))
}

/// The user's own words inside an app message: the note of "Ask for changes" in Review ("Review: <title> — change: <note>").
fn change_note(line: &str) -> Option<(&str, &str)> {
    let (title, note) = line.trim().strip_prefix("Review: ")?.split_once(" \u{2014} change: ")?;
    Some((title.trim(), note.trim())).filter(|(_, n)| !n.is_empty())
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
        st.pause_told = false;
        st.autopilot_left = if ask.autopilot || ask.full { AUTOPILOT_TURNS } else { 0 };
        if !waiting {
            st.overnight_until = if ask.overnight { now + OVERNIGHT_SECS } else { 0.0 };
        }
        st.overnight_fails = 0;
        st.overnight_done = 0;
    }
    st.last_ask = Ask { attachments: Vec::new(), kickoff: None, ..ask.clone() };
    st.turn_auto = auto;
    st.turn_tools = 0;
    st.turn_progress = false;
    st.sending = true;
    Some(waiting)
}

/// Whether the chat is working: a turn being sent (from `begin_send` until the AI is running, or the send gives up),
/// a turn running, or a texture being made.
fn working(st: &State) -> bool {
    st.sending || st.running || st.making
}

/// Clears `sending` when `send` returns, whichever way: by then the turn is running, or it never started.
struct Sending<'a>(&'a Agent);

impl Drop for Sending<'_> {
    fn drop(&mut self) {
        self.0.st.lock().unwrap().sending = false;
    }
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

/// Whether a failed turn's message says the AI plan's usage limit was reached ("You've hit your 5-hour limit · resets
/// 12:50am"): trying again at once only fails again, so a studio run pauses instead.
fn usage_limit(msg: &str) -> bool {
    let low = msg.to_lowercase();
    low.contains("limit") && (low.contains("hit your") || low.contains("usage limit") || low.contains("rate limit") || low.contains("resets"))
}

const LIMIT_END: &str = "Studio run paused: the AI's usage limit was reached. Press Resume in Review once it is back.";
const FAILED_END: &str = "Studio run stopped: three turns in a row failed.";

/// Whether Claude Code's closing "result" event reports a finished turn. "Not logged in" arrives as a result with
/// is_error set, and must count as a failed turn, not as a reply.
fn result_ok(e: &Value) -> bool {
    !e["is_error"].as_bool().unwrap_or(false)
}

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
                    Step::End(if studio { FAILED_END } else { "Overnight mode stopped: three turns in a row failed." })
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

/// The one message a paused Studio run's Lead gets instead of its next carry-on.
const PAUSE_TEXT: &str = "The studio run is paused. Let the experts already at work finish (wait for their results with rbx_team action wait), start nothing new and hand out no new jobs, then end with STATUS: WAITING.";

/// A Studio run that is paused: its Lead's next carry-on becomes the pause message, once (`told` remembers it was
/// sent); after that the Lead waits, whatever it said, until Resume. Not paused: `step` as it was, and `told` cleared.
fn pause_step(step: Step, paused: bool, told: &mut bool, state: &mut LoopState) -> Step {
    if !paused {
        *told = false;
        return step;
    }
    match step {
        Step::CarryOn(_) if !*told => {
            *told = true;
            Step::CarryOn(PAUSE_TEXT)
        }
        Step::CarryOn(_) => {
            state.night = false;
            Step::Wait
        }
        other => other,
    }
}

/// A carry-on arriving while the Lead's run is paused (decided before the pause, or in the gap before an undo): the
/// pause message goes once instead, and anything after it is dropped.
fn paused_carry_on<'a>(text: &'a str, told: &mut bool) -> Option<&'a str> {
    if text == PAUSE_TEXT {
        return Some(PAUSE_TEXT);
    }
    if *told {
        return None;
    }
    *told = true;
    Some(PAUSE_TEXT)
}

/// Held messages go as one turn with the newest message's settings, but Studio is kept when any of them was a Studio
/// message (a run's Lead never drops out of Studio mode because the last message was typed in another mode).
fn merged_studio(held: &[(String, String, Ask)], newest: bool) -> bool {
    newest || held.iter().any(|(_, _, a)| a.studio)
}

/// Stop or End this run on a Lead: the app's own queued wakes go (a "Resume the studio run." must not start a new
/// turn), and after End what the user typed stays queued but as an ordinary message. True when the queue changed.
fn drop_wakes(queue: &mut Vec<(String, String, Ask)>, ended: bool) -> bool {
    let before = queue.len();
    queue.retain(|(_, _, a)| !a.wake);
    let mut changed = queue.len() < before;
    if ended {
        for (_, _, a) in queue.iter_mut().filter(|(_, _, a)| a.studio) {
            *a = ordinary(a);
            changed = true;
        }
    }
    changed
}

/// A Studio message turned into an ordinary one (Ask me): no Studio, no full auto, no night loop.
fn ordinary(ask: &Ask) -> Ask {
    Ask { studio: false, overnight: false, full: false, autopilot: false, wake: false, ..ask.clone() }
}

/// What a message to a project's chat runs with, given the game's run (`run`) and whether the chat waits at a gate.
/// The Lead of a running run gets Studio mode whatever was sent (`studio_message`); the Lead of an ENDED run is an
/// ordinary chat again: a Studio message goes as Ask me (no Studio mode, so no night checkpoint), and None means it
/// is dropped (an app wake, or a carry-on, for a run that is over).
fn lead_ask(ask: &Ask, run: studio_undo::LeadOf, waiting: bool, last: &Ask) -> Option<Ask> {
    if !run.ended {
        return Some(effective_ask(&studio_message(ask, run.leads), waiting, last));
    }
    if ask.wake || (ask.kickoff.as_deref() == Some("auto") && last.studio) {
        return None;
    }
    Some(if ask.studio { ordinary(ask) } else { effective_ask(ask, false, last) })
}

/// What a Lead is told when its run was ended in Review while it worked or waited.
const RUN_ENDED: &str = "Studio run ended in Review. Its files and review items are kept.";

/// A Lead whose run was ended (Review's "End this run"): nothing carries on and nothing waits any more.
fn ended_step(step: Step, ended: bool, state: &mut LoopState) -> Step {
    match step {
        Step::CarryOn(_) | Step::Wait if ended => {
            state.night = false;
            Step::End(RUN_ENDED)
        }
        other => other,
    }
}

/// How many carry-on turns in a row without a single tool call a Studio Lead may take before it is taken to be waiting.
const IDLE_TURNS: u32 = 3;

/// The no-progress guard: a Studio Lead that answers three carry-ons in a row without calling any tool is going round
/// in circles, so the turn counts as STATUS: WAITING (the user's next answer or message wakes it). `idle` counts such
/// turns; a turn with a tool call, a turn the user sent, or a failed turn starts the count again. True when the guard
/// trips (the count then starts again too).
fn no_progress(studio: bool, carry_on: bool, got_result: bool, tools: u32, idle: &mut u32) -> bool {
    if !(studio && carry_on && got_result && tools == 0) {
        *idle = 0;
        return false;
    }
    *idle += 1;
    if *idle >= IDLE_TURNS {
        *idle = 0;
        return true;
    }
    false
}

/// Whether the no-progress guards count this turn: a Studio Lead's, and only while none of its experts is working
/// (waiting for them is not going round in circles, and STATUS: WAITING would leave them unattended).
fn guarded(studio_lead: bool, experts_working: bool) -> bool {
    studio_lead && !experts_working
}

/// How many Studio carry-ons in a row without a progress call a Lead may take before it is taken to be waiting.
const STUCK_TURNS: u32 = 6;

/// A tool call that moves a studio run on: rbx_stage stage/review/lesson, rbx_team spawn, any tool of Roblox
/// Studio's connector, or a file edit. Looking (status, decisions, wait, reading files) is not progress. `input` is
/// the call's arguments; Null when the AI does not report them (then rbx_stage and rbx_team count as progress).
fn progress_call(name: &str, input: &Value) -> bool {
    let short = name.rsplit("__").next().unwrap_or(name).to_ascii_lowercase();
    let action = input["action"].as_str().map(str::to_ascii_lowercase);
    match short.as_str() {
        "rbx_stage" => action.map_or(input.is_null(), |a| matches!(a.as_str(), "stage" | "review" | "lesson")),
        "rbx_team" => action.map_or(input.is_null(), |a| a == "spawn"),
        "write" | "edit" | "multiedit" | "notebookedit" | "write_file" | "edit_file" | "apply_patch" | "replace" | "delete" => true,
        _ => studio_tool(name),
    }
}

/// The second no-progress guard: a Studio Lead whose carry-ons keep calling tools but none that moves the run on
/// (a `rbx_team wait` or a status check in a loop) is going round in circles too: six such carry-ons in a row count as
/// STATUS: WAITING. A turn with a progress call, a turn the user sent, or a failed turn starts the count again.
fn no_progress_call(studio: bool, carry_on: bool, got_result: bool, progress: bool, stuck: &mut u32) -> bool {
    if !(studio && carry_on && got_result && !progress) {
        *stuck = 0;
        return false;
    }
    *stuck += 1;
    if *stuck >= STUCK_TURNS {
        *stuck = 0;
        return true;
    }
    false
}

/// Any message to the chat that leads a running, not ended studio run is a Studio message, whatever mode it was sent in
/// (a reply typed while the Lead works, or queued in another mode, must not take the run out of Studio mode).
fn studio_message(ask: &Ask, leads_run: bool) -> Ask {
    Ask { studio: ask.studio || leads_run, ..ask.clone() }
}

const STUDIO_FALLBACK: &str = "Studio needs a game folder, so this message ran in Ask me.";

/// Studio runs belong to a game project's folder: in a chat with none, a Studio message goes as Ask me instead (it
/// does what it is asked, then waits), with a line saying so.
fn studio_fallback(ask: &Ask, has_project: bool) -> (Ask, Option<&'static str>) {
    if ask.studio && !has_project {
        (Ask { studio: false, overnight: false, full: false, autopilot: false, ..ask.clone() }, Some(STUDIO_FALLBACK))
    } else {
        (ask.clone(), None)
    }
}

/// Whether a message starts a night checkpoint (a plugin pin and a "Before the overnight run" commit). Not for the
/// loop's own carry-ons, not for a waiting run going on, and not for a message to the Lead of a studio run that already
/// has its "start" checkpoint (a decision, a reply while it works, Resume): those belong to the same run.
fn takes_night_checkpoint(ask: &Ask, auto: bool, resumed: bool, run_started: bool) -> bool {
    ask.overnight && !auto && !resumed && !(ask.studio && run_started)
}

/// A tool from Roblox Studio's own connector: the steps that change the place.
fn studio_tool(name: &str) -> bool {
    name.to_ascii_lowercase().contains("roblox_studio")
}

/// The Studio steps in a chat's log from `since` on: {label, t}. What a helper hands its studio Lead.
fn studio_steps_since(log: &[Value], since: f64) -> Vec<Value> {
    log.iter()
        .filter(|e| e["kind"] == "tool" && e["studio"] == true)
        .filter_map(|e| Some(json!({"label": e["label"], "t": e["t"].as_f64().filter(|t| *t >= since)?})))
        .collect()
}

/// A studio run's undo log: the Lead's own Studio steps and those its experts handed over ("helper_steps" events),
/// as {t, by, label}, oldest first. Steps without a time are left out.
fn merge_run_log(log: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = studio_steps_since(log, f64::MIN).into_iter().map(|s| json!({"t": s["t"], "by": "Lead", "label": s["label"]})).collect();
    for e in log.iter().filter(|e| e["kind"] == "helper_steps") {
        let by = e["name"].as_str().unwrap_or("Expert");
        for step in e["steps"].as_array().into_iter().flatten().filter(|s| s["t"].is_number()) {
            out.push(json!({"t": step["t"], "by": by, "label": step["label"]}));
        }
    }
    out.sort_by(|a, b| a["t"].as_f64().unwrap_or(0.0).total_cmp(&b["t"].as_f64().unwrap_or(0.0)));
    out
}

/// The undo log of a chat that is not open: read from its saved file.
pub fn saved_run_log(chat_id: &str) -> Vec<Value> {
    let d: Value = fs::read_to_string(chat_file(chat_id)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    merge_run_log(d["events"].as_array().map(Vec::as_slice).unwrap_or(&[]))
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
    /// The running turn's process (the command-line AIs; not OpenCode, which runs on the app's server).
    proc_: Option<Arc<TurnProc>>,
    /// Claude Code kept open for this chat between turns (`claude_live`), and how often that failed in this chat:
    /// twice, and the chat stays on a process per turn for the rest of this run of the app.
    live: Option<Arc<Live>>,
    live_fails: u32,
    /// The chat was closed (`remove_agent`): a turn still ending on its kept process closes that process.
    closed: bool,
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
    /// The chat that started this helper (a studio run's Lead, for its experts).
    parent: Option<String>,
    /// A paused studio run's Lead has been told to wind down (see `pause_step`).
    pause_told: bool,
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
    /// A turn is being sent: `begin_send` has run and the AI is not running yet (see `working`).
    sending: bool,
    overnight_fails: u32,
    overnight_done: u32,
    /// Studio mode stopped at a gate (STATUS: WAITING): no carry-on until the user answers, and the clock is stopped
    /// with this many seconds of it left.
    studio_waiting: bool,
    overnight_left: f64,
    /// Which AI this chat runs on ("claude", "gemini", "chatgpt", "cursor" or "opencode"). Open until the first message; a
    /// conversation then stays with the AI it began on, because its memory lives with that AI.
    ai: Option<String>,
    /// The model this chat was started on (an expert placed by Setup > Experts or by its Lead's request): used when a
    /// message names none. None: the AI's default.
    model: Option<String>,
    /// The chat so far, as text, for the AI a started chat was switched to: it goes with that AI's first message
    /// (see `switch_ai` and `handover_text`).
    handover: Option<String>,
    /// A turn of this chat has ended (or it was reopened from a saved log): a helper that is not working then holds no
    /// slot. A helper just started holds one until its first turn is under way, so quick spawns cannot overshoot.
    turned: bool,
    /// Messages the user sent while this chat was working, with their own settings. They go together as the next turn.
    queue: Vec<(String, String, Ask)>,
    /// A helper whose result has been handed back: while it is not working it holds no agent slot (see `holds_slot`).
    result_written: bool,
    /// The current (or latest) turn was the loop's own carry-on, and how many tools it called (the no-progress guard).
    turn_auto: bool,
    turn_tools: u32,
    /// Studio carry-on turns in a row without a tool call (see `no_progress`).
    idle_turns: u32,
    /// Whether this turn made a progress call (see `progress_call`), and how many Studio carry-ons in a row made none.
    turn_progress: bool,
    stuck_turns: u32,
}

/// An expert, a helper or the researcher: a chat the app started for another chat, not one of the user's own.
fn is_helper(st: &State) -> bool {
    st.parent.is_some() || st.team.is_some() || st.role.as_deref() == Some("research")
}

/// Whether a chat counts toward MAX_AGENTS. A helper that is not working (finished, failed or stopped) holds none: it
/// stays open to read, and takes a slot again only while it works. One just started holds a slot until its first turn.
fn holds_slot(st: &State) -> bool {
    !(is_helper(st) && !working(st) && (st.turned || st.result_written))
}

/// Whether a message to a chat must be refused for want of a slot: a chat that holds none now (a finished helper)
/// takes one to work, and there is none free. `taken` is only counted when it matters.
fn no_slot_for(holds: bool, taken: impl FnOnce() -> usize) -> bool {
    !holds && taken() >= MAX_AGENTS
}

/// How many agent slots are taken now.
fn slots_taken(all: &[Arc<Agent>]) -> usize {
    all.iter().filter(|a| holds_slot(&a.st.lock().unwrap())).count()
}

/// The agent slots in use, for the window (it may show more chats than this: finished helpers hold none).
pub fn slots_used() -> usize {
    slots_taken(&AGENTS.lock().unwrap())
}

/// One chat. One turn at a time (Claude Code's on a process kept open for the chat, see `claude_live`); events go
/// to the bus.
pub struct Agent {
    pub id: String,
    st: Mutex<State>,
    save_lock: Mutex<()>,
}

static AGENTS: Mutex<Vec<Arc<Agent>>> = Mutex::new(Vec::new());
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// The app is quitting: nothing new starts (a held message or a wake must not start an agent after shut_down).
/// Per thread in tests, so a test that sets it touches no other test.
#[cfg(not(test))]
static SHUTTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
thread_local! {
    static SHUTTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn set_shutting(on: bool) {
    #[cfg(not(test))]
    SHUTTING.store(on, Ordering::SeqCst);
    #[cfg(test)]
    SHUTTING.with(|s| s.set(on));
}

fn shutting() -> bool {
    #[cfg(not(test))]
    return SHUTTING.load(Ordering::SeqCst);
    #[cfg(test)]
    SHUTTING.with(|s| s.get())
}

pub fn agents() -> Vec<Arc<Agent>> {
    AGENTS.lock().unwrap().clone()
}

/// Whether a chat is one of `lead`'s helpers at work: started by it with rbx_team, and working now.
fn working_helper_of(st: &State, lead: &str) -> bool {
    !lead.is_empty() && st.team.is_some() && st.parent.as_deref() == Some(lead) && working(st)
}

/// Every chat `lead` started, working or not (closing the Lead or ending its run closes them).
pub fn helpers_of(lead: &str) -> Vec<Arc<Agent>> {
    agents().into_iter().filter(|a| !lead.is_empty() && a.st.lock().unwrap().parent.as_deref() == Some(lead)).collect()
}

/// The helpers `lead` started that are working now (End this run stops them).
pub fn working_helpers_of(lead: &str) -> Vec<Arc<Agent>> {
    agents().into_iter().filter(|a| working_helper_of(&a.st.lock().unwrap(), lead)).collect()
}

pub fn get(id: &str) -> Option<Arc<Agent>> {
    AGENTS.lock().unwrap().iter().find(|a| a.id == id).cloned()
}

fn is_user(e: &Value) -> bool {
    e["kind"] == "user"
}

/// A running turn's process, for the command-line AIs (Claude Code, Antigravity, Cursor, Codex): its pid, which is
/// also its process group on Unix (`paths::own_group`), how it ended, and a gate held while it is signalled, so its
/// leader is never reaped meanwhile (an unreaped leader keeps the group's ID reserved: a signal can only reach it).
pub struct TurnProc {
    pid: u32,
    reaped: Mutex<bool>,
    /// its output has ended (the turn is over, whatever is left running)
    done: AtomicBool,
    /// Stop asked it to end and it had not after STOP_WAIT, so it was killed
    killed: AtomicBool,
    /// nothing came for STALL, so it was ended
    stalled: AtomicBool,
    /// The turn runs on the chat's kept Claude Code process: Stop asks it to interrupt and it lives on.
    live: Option<Arc<Live>>,
    /// Claude Code has started on the turn's message (a kept process): an interrupt before that would end nothing.
    acked: AtomicBool,
    /// A kept process has shown that it read the turn's message (any line carrying the message's id, or the turn's
    /// `init` line): one that shows nothing within START_WAIT is not taking messages (`watch_start`).
    heard: AtomicBool,
    /// It showed nothing in that time and was ended.
    silent: AtomicBool,
    /// Claude Code answered an interrupt with the turn's message "still queued": nothing was stopped.
    requeued: AtomicBool,
    /// A tool call of the turn is open right now (a kept process; kept up by `consume_lines`).
    tool_open: AtomicBool,
    /// The kept process is not used again after this turn: Stop landed while a tool call was open (the app's tool
    /// server may still be working on it), or Claude Code said the message was still queued when it was interrupted.
    discard: AtomicBool,
    /// The process groups of a kept process's tree when it took the message (its own, its tool servers'): Stop ends
    /// only what the turn added to them.
    standing: Mutex<Vec<u32>>,
}

/// How long a kept Claude Code process has to show that it read a message before it is taken not to take messages.
const START_WAIT: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 90 });

impl TurnProc {
    fn new(pid: u32) -> Arc<TurnProc> {
        Arc::new(TurnProc { pid, reaped: Mutex::new(false), done: AtomicBool::new(false), killed: AtomicBool::new(false), stalled: AtomicBool::new(false), live: None, acked: AtomicBool::new(false), heard: AtomicBool::new(false), silent: AtomicBool::new(false), requeued: AtomicBool::new(false), tool_open: AtomicBool::new(false), discard: AtomicBool::new(false), standing: Mutex::new(Vec::new()) })
    }

    /// A turn on a chat's kept Claude Code process.
    fn on(live: &Arc<Live>) -> Arc<TurnProc> {
        Arc::new(TurnProc { pid: live.pid, reaped: Mutex::new(false), done: AtomicBool::new(false), killed: AtomicBool::new(false), stalled: AtomicBool::new(false), live: Some(live.clone()), acked: AtomicBool::new(false), heard: AtomicBool::new(false), silent: AtomicBool::new(false), requeued: AtomicBool::new(false), tool_open: AtomicBool::new(false), discard: AtomicBool::new(false), standing: Mutex::new(Vec::new()) })
    }

    /// Claude Code has started on the turn's message (a kept process): noted once, with the groups its tree has now.
    fn taken(&self) {
        self.heard.store(true, Ordering::SeqCst);
        if !self.acked.load(Ordering::SeqCst) {
            *self.standing.lock().unwrap_or_else(|e| e.into_inner()) = crate::procs::tree_groups(self.pid);
            self.acked.store(true, Ordering::SeqCst);
        }
    }

    /// Stop on a kept Claude Code process.
    ///
    /// Before Claude Code has started on the message (its `started` mark or the turn's `init` line), an interrupt
    /// would end nothing and the message would run afterwards (measured: the answer to an interrupt lists what is
    /// "still queued"). So the process is ended at once, tree and all, as a process per turn is: nothing has run, and
    /// the next message starts one that resumes.
    ///
    /// Once it has started, it is asked to interrupt and given STOP_WAIT to end the turn with its `result` (measured:
    /// 30 ms). A turn that does not end in time is killed with its whole tree (noted, so the chat says so). When a
    /// tool call was open as Stop landed, the process is not used again (`discard`; `read_live` closes it, with all
    /// it started): the app's tool server handles one call at a time and may still be in a Blender or Studio
    /// call that Claude Code's interrupt does not reach. Otherwise the process lives on, and only what the turn's
    /// tools left running in groups of their own is ended.
    fn stop_live(&self, live: &Live) {
        let done = || self.done.load(Ordering::SeqCst);
        if done() {
            return;
        }
        if !self.acked.load(Ordering::SeqCst) {
            self.end();
            return;
        }
        if self.tool_open.load(Ordering::SeqCst) {
            self.discard.store(true, Ordering::SeqCst);
        }
        // by its own protocol: Claude Code's interrupt request, an ACP agent's `session/cancel`
        if live.ask_stop().is_none() {
            live.interrupt();
        }
        let until = std::time::Instant::now() + paths::STOP_WAIT;
        // Claude Code may answer that the message was still queued: nothing was stopped, so the process is ended
        let unstopped = || self.requeued.load(Ordering::SeqCst);
        while !done() && !unstopped() && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(25));
        }
        if !done() {
            if !unstopped() {
                self.killed.store(true, Ordering::SeqCst);
            }
            self.end();
            return;
        }
        if self.discard.load(Ordering::SeqCst) {
            return; // the whole process goes (`read_live`)
        }
        let standing = self.standing.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let left: Vec<u32> = self.groups(*self.reaped.lock().unwrap_or_else(|e| e.into_inner())).into_iter().filter(|g| *g != self.pid && !standing.contains(g)).collect();
        if left.iter().any(|&g| paths::signal_group(g, "-0")) {
            paths::end_trees(&left);
        }
    }

    /// The process groups of the turn's tree (`procs::tree_groups`): its own, unless its leader has been reaped, and
    /// those its tools' shells were put in. Call with the gate held.
    fn groups(&self, reaped: bool) -> Vec<u32> {
        // once the leader is reaped its pid may belong to another process: nothing is looked up from it then (its
        // children have new parents anyway)
        if reaped {
            return Vec::new();
        }
        crate::procs::tree_groups(self.pid)
    }

    /// Unix: sends `sig` to each of `groups` (the turn's own only while its leader is unreaped). True when any had
    /// members.
    fn signal(&self, sig: &str, groups: &[u32]) -> bool {
        let reaped = self.reaped.lock().unwrap_or_else(|e| e.into_inner());
        let mut any = false;
        for &g in groups.iter().filter(|&&g| g != self.pid || !*reaped) {
            any |= paths::signal_group(g, sig);
        }
        any
    }

    /// Ends the turn's processes now, its whole tree: SIGTERM, then SIGKILL after STOP_GRACE (Unix); taskkill /T on
    /// Windows.
    fn end(&self) {
        let reaped = self.reaped.lock().unwrap_or_else(|e| e.into_inner());
        if cfg!(windows) {
            if !*reaped {
                paths::end_trees(&[self.pid]);
            }
            return;
        }
        paths::end_trees(&self.groups(*reaped));
    }

    /// Stop, confirmed: the turn is asked to end (SIGTERM to its tree's groups), given STOP_WAIT to end by itself, and
    /// killed if it has not (noted, so the chat says so). What is left after its output ended in time, in its own group
    /// (while the leader is unreaped) or in groups its tools' shells were put in, gets STOP_GRACE more, then SIGKILL.
    /// Blocks: call it off the page's thread. Windows: taskkill /T at once, as before.
    fn stop(&self) {
        if cfg!(windows) {
            self.end();
            return;
        }
        if let Some(live) = &self.live {
            return self.stop_live(live);
        }
        let groups = self.groups(*self.reaped.lock().unwrap_or_else(|e| e.into_inner()));
        self.signal("-TERM", &groups);
        let until = std::time::Instant::now() + paths::STOP_WAIT;
        while !self.done.load(Ordering::SeqCst) && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(100));
        }
        if !self.done.load(Ordering::SeqCst) {
            if self.signal("-0", &groups) {
                self.killed.store(true, Ordering::SeqCst);
                self.signal("-KILL", &groups);
            }
            return;
        }
        let until = std::time::Instant::now() + paths::STOP_GRACE;
        while self.signal("-0", &groups) && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(100));
        }
        self.signal("-KILL", &groups);
    }

    /// The leader is about to be reaped: nothing signals its group from now on.
    fn reaping(&self) {
        *self.reaped.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }
}

/// While a turn runs: every 5 seconds, a turn that sent nothing for STALL (and is not waiting on purpose: on its
/// experts, a texture, or one of the app's tools that waits long) is ended; STALL_WORKING when its last line started a
/// tool. `last` holds when the last line came, whether it started such a long tool, and whether it started any tool.
fn watch_stall(me: Arc<Agent>, tp: Arc<TurnProc>, last: Arc<Mutex<(std::time::Instant, bool, bool)>>) {
    while !tp.done.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(5));
        let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
        let waiting = l.1 || me.st.lock().unwrap().making || !working_helpers_of(&me.id).is_empty();
        if waiting {
            // the quiet clock starts again when a wait on purpose ends, not when it began
            l.0 = std::time::Instant::now();
            continue;
        }
        let (quiet, tool_open) = (l.0.elapsed(), l.2);
        drop(l);
        if paths::stalled(quiet, false, tool_open) {
            // decided against the turn's own end under one lock: a turn that ended meanwhile is not called stalled
            let _gate = tp.reaped.lock().unwrap_or_else(|e| e.into_inner());
            if tp.done.load(Ordering::SeqCst) {
                return;
            }
            tp.stalled.store(true, Ordering::SeqCst);
            drop(_gate);
            tp.end();
            return;
        }
    }
}

/// A kept Claude Code process that shows no sign of having read a turn's message within START_WAIT is not taking
/// messages (a Claude Code whose input format changed, say): it is ended, and `read_live` sends the message to a
/// process of its own and keeps the chat on those.
fn watch_start(tp: Arc<TurnProc>) {
    let until = std::time::Instant::now() + START_WAIT;
    while !tp.done.load(Ordering::SeqCst) && !tp.heard.load(Ordering::SeqCst) {
        if std::time::Instant::now() >= until {
            // decided against the turn's own end under one lock, as the stall guard does
            let gate = tp.reaped.lock().unwrap_or_else(|e| e.into_inner());
            if tp.done.load(Ordering::SeqCst) || tp.heard.load(Ordering::SeqCst) {
                return;
            }
            tp.silent.store(true, Ordering::SeqCst);
            drop(gate);
            tp.end();
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Keeps the tool calls of Claude Code's that started and have no result yet, by id (a long build may send nothing for
/// a while; a line in between, such as another call's result or a usage line, does not close it).
fn claude_open_tools(e: &Value, open: &mut std::collections::HashSet<String>) {
    for b in e["message"]["content"].as_array().into_iter().flatten() {
        match (e["type"].as_str(), b["type"].as_str()) {
            (Some("assistant"), Some("tool_use")) => {
                open.insert(b["id"].as_str().unwrap_or_default().to_string());
            }
            (Some("user"), Some("tool_result")) => {
                open.remove(b["tool_use_id"].as_str().unwrap_or_default());
            }
            _ => {}
        }
    }
}


/// Whether a line of Claude Code's output starts one of the app's tools that wait long on purpose.
fn claude_starts_long_tool(e: &Value) -> bool {
    e["type"] == "assistant" && e["message"]["content"].as_array().into_iter().flatten().any(|b| b["type"] == "tool_use" && b["name"].as_str().is_some_and(paths::waits_long))
}

/// The same for what another AI's parser made of a line.
fn starts_long_tool(outs: &[Out]) -> bool {
    outs.iter().any(|o| matches!(o, Out::Tool(n) | Out::ToolArgs(n, _) if paths::waits_long(n)))
}

/// What a turn's events said about how it went.
#[derive(Default)]
struct TurnEnd {
    got_result: bool,
    /// the AI answered, but its result says the turn failed (signed out, out of turns...)
    result_error: bool,
    /// what that failed result said
    result_text: String,
    /// that failure was already shown in the chat (a result with only an error kind is left to `after_turn`, which
    /// has the AI's error output too)
    result_said: bool,
    /// the AI's own words or a tool call were shown: the turn got somewhere
    began: bool,
    /// a kept process's output ended before the turn's `result` came (it died, or was ended)
    ended: bool,
    /// a kept process's turn ended with a task of its own still running in the background: Claude Code starts a turn
    /// by itself when such a task finishes (measured), so the process is not kept
    background_left: bool,
    /// the kept process cannot serve this message as a process per turn would (an ACP agent that does not offer the
    /// model or effort asked for): it was ended, and the message goes to a process of its own, with no failure counted
    unfit: bool,
}

/// A turn read from a chat's kept Claude Code process (`consume_lines`): its process, the id its message was sent
/// with, and whether what was read before that id came back can be left over from an earlier turn.
struct LiveTurn<'a> {
    tp: &'a TurnProc,
    message: &'a str,
    first: bool,
    /// the process has sent a `command_lifecycle` line (`Live::marks_turns`)
    marks: &'a AtomicBool,
}

/// The line for a failed sign-in when a provider key is set in the app's environment but kept from the AI (the Setup
/// switch "Use API keys from my environment" is off). None when the failure is not about signing in, or no such key.
fn key_note(kind: &str, said: &str, present: &[&str], keys_on: bool) -> Option<String> {
    let s = said.to_lowercase();
    // the AIs' own sign-in wording, not words a tool's error may quote ("401", "login" anywhere)
    let sign_in = ["not signed in", "not logged in", "please run /login", "invalid api key", "api key not found", "missing api key", "unauthorized", "authentication failed", "authentication_error", "invalid x-api-key"].iter().any(|w| s.contains(w));
    // OpenCode's keys stay kept from it whatever the switch says (paths::env_dropped): nothing to turn on
    (!keys_on && kind != "opencode" && sign_in && !present.is_empty()).then(|| {
        format!(
            "{} is set on this computer, but Fridae keeps it from {} so you are never billed to it by surprise. If you sign in with that key or a company gateway, turn on Setup, The AIs, \"Use API keys from my environment\".",
            present.join(", "),
            ai_label(kind)
        )
    })
}

/// What a refused command becomes in the chat: the blocked card for the programs the user has not allowed, and a
/// plain line when every program in it is allowed already (it was refused for its form, such as `./git`: allowing
/// cannot help).
fn blocked_or_note(commands: Vec<Value>, allowed: &[String]) -> (Vec<Value>, Option<&'static str>) {
    let all = commands.len();
    let card: Vec<Value> = commands.into_iter().filter(|c| !c["name"].as_str().is_some_and(|n| allowed.iter().any(|a| a == n))).collect();
    let note = (card.len() < all).then_some(FORM_REFUSED);
    (card, note)
}

/// The programs Claude Code refused in a turn (its result's `permission_denials`), for the card: every program of a
/// refused command (each part of `git status && rm -rf build`) the user has not allowed; a command whose programs are
/// all allowed is listed by its first, so the plain line says it was its form.
fn claude_blocked(denials: &Value, allowed: &[String]) -> Vec<Value> {
    let mut blocked: Vec<Value> = Vec::new();
    for d in denials.as_array().into_iter().flatten() {
        let Some(cmd) = d["tool_input"]["command"].as_str() else { continue };
        let programs = paths::command_programs(cmd);
        let missing: Vec<String> = programs.iter().filter(|p| !allowed.contains(p)).cloned().collect();
        let names = if missing.is_empty() { programs.into_iter().take(1).collect() } else { missing };
        for name in names {
            if !blocked.iter().any(|b| b["name"] == name) {
                blocked.push(json!({"name": name, "command": cmd.chars().take(200).collect::<String>()}));
            }
        }
    }
    blocked
}

/// The line for a command refused although its program is allowed.
const FORM_REFUSED: &str = "A command was not run although its program is allowed: allowed programs run only when called by their plain name (\"git status\", not \"./git status\" or a path).";

/// A Cursor or OpenCode chat whose saved conversation is gone: forget it, and hand over the chat so far (without the
/// message being sent now, the last thing the user said) so the next turn starts a new conversation that knows it.
fn stale_chat(st: &mut State, from: &str, saved: impl Fn(&str) -> bool) {
    let Some(session) = st.session.clone() else { return };
    if saved(&session) {
        return;
    }
    st.session = None;
    if st.handover.is_none() {
        let upto = st.log.iter().rposition(is_user).unwrap_or(st.log.len());
        st.handover = Some(handover_text(&st.log[..upto], from, HANDOVER_CAP));
    }
}

/// How much of a chat goes to the AI it is switched to: the newest part, about 6,000 words.
const HANDOVER_CAP: usize = 24_000;

/// An AI's name as people read it.
fn ai_label(kind: &str) -> &'static str {
    match kind {
        "gemini" => "Gemini",
        "chatgpt" => "ChatGPT",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        _ => "Claude",
    }
}

/// The message as the AI gets it: after a switch of AI, the chat so far goes first. The handover stays in the state
/// until the new AI has a conversation of its own (`got_session`).
fn with_handover(st: &State, text: &str) -> String {
    match &st.handover {
        Some(chat) => chat.clone() + text,
        None => text.to_string(),
    }
}

/// The AI opened (or carried on) a conversation: anything handed over is in it now.
fn got_session(st: &mut State, id: String) {
    st.session = Some(id);
    st.handover = None;
}

/// The chat so far as text, for the AI a started chat is switched to: the user's messages and the agent's replies,
/// oldest first, without tool steps, pictures or the loop's own carry-on messages. When it is longer than `cap`
/// characters the oldest messages are left out (and it says so); a single message longer than that keeps its end.
fn handover_text(log: &[Value], from: &str, cap: usize) -> String {
    // which AI wrote each reply: the one the chat began on, then each switch's new one
    let mut ai = log
        .iter()
        .find(|e| e["kind"] == "ai" && e["from"].is_string())
        .and_then(|e| e["from"].as_str())
        .map(ai_label)
        .unwrap_or(from)
        .to_string();
    let mut lines: Vec<(String, String)> = Vec::new();
    for e in log {
        if e["kind"] == "ai" && e["from"].is_string() {
            ai = ai_label(e["ai"].as_str().unwrap_or("")).to_string();
            continue;
        }
        let Some(text) = e["text"].as_str().map(str::trim).filter(|t| !t.is_empty()) else { continue };
        match e["kind"].as_str() {
            // only what the user typed: the loop's carry-ons and the app's own wakes and messages are left out, except
            // the user's own note when they asked for changes in Review
            Some("user") if e["auto"] != true => {
                let app = e["app"] == true;
                let typed = e["typed"].as_str().map(str::trim).filter(|t| !t.is_empty());
                if !app {
                    lines.push(("User".to_string(), typed.unwrap_or(text).to_string()));
                }
                if app || typed.is_some() {
                    for (title, note) in text.lines().filter_map(change_note) {
                        lines.push(("User".to_string(), format!("(asked for changes in Review, on \"{title}\") {note}")));
                    }
                }
            }
            Some("text") => lines.push((ai.clone(), text.to_string())),
            _ => {}
        }
    }
    let lines: Vec<String> = lines.into_iter().map(|(who, text)| format!("{who}: {text}")).collect();
    let mut kept: Vec<String> = Vec::new();
    let mut size = 0;
    let mut cut = false;
    for line in lines.iter().rev() {
        let n = line.chars().count() + 2;
        if size + n > cap {
            cut = true;
            if kept.is_empty() {
                // one very long message: its end, which is what was said last, still saying who said it
                let who = line.split_once(": ").map(|(w, _)| w).unwrap_or("");
                let room = cap.saturating_sub(who.chars().count() + 4);
                let tail: String = line.chars().rev().take(room).collect::<Vec<_>>().into_iter().rev().collect();
                kept.push(format!("{who}: …{}", tail.trim_start()));
            }
            break;
        }
        size += n;
        kept.push(line.clone());
    }
    kept.reverse();
    let body = if kept.is_empty() { "(nothing was said yet)".to_string() } else { kept.join("\n\n") };
    format!(
        "[This conversation began with {from}; the user switched it to you, so you carry it on. The chat so far, as text \
         (the user's own messages and each AI's replies, without tool steps{}):\n\n{body}\n\nEnd of the chat so far. The \
         user's new message follows.]\n\n",
        if cut { "; the oldest messages are left out" } else { "" }
    )
}

/// How often keeping a Claude Code process may fail in one chat (it could not start, or died) before the chat stays
/// on a process per turn for the rest of this run of the app.
const LIVE_FAILS: u32 = 2;

/// What a Claude Code turn is started with: everything but the conversation to resume and whether the process is kept
/// open. Built afresh for every message, exactly as a process per turn is given it, so a kept process started with
/// anything else (`key`) is replaced and no rule goes stale inside one.
#[derive(Clone)]
struct ClaudeLaunch {
    exe: std::path::PathBuf,
    /// arguments before all others: none for Claude Code (the tests' stand-in is a shell script given to `sh`)
    first: Vec<std::ffi::OsString>,
    args: Vec<std::ffi::OsString>,
    cwd: std::path::PathBuf,
    /// the tool servers written to the `--mcp-config` file (its path is the same for every turn of a chat)
    servers: String,
    /// whether the provider keys in the app's environment reach the process (`paths::child_env`)
    keys_on: bool,
}

/// Everything a Claude Code turn's start depends on, as `send` has it for a message.
struct ClaudeSettings<'a> {
    exe: &'a std::path::Path,
    mcp_config: &'a std::path::Path,
    servers: &'a Value,
    /// the programs the user allowed from a "blocked" card or Setup
    allowed: &'a [String],
    /// the app's own files, never written by an agent (`claude_denied`)
    denied: &'a [String],
    full: bool,
    /// the standing instructions: the mode's texts, the role, the team, the user's house rules
    prompt: &'a str,
    kit: &'a std::path::Path,
    project: Option<&'a str>,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    /// where a chat without a game folder works
    work: &'a std::path::Path,
    keys_on: bool,
}

impl ClaudeLaunch {
    fn new(s: &ClaudeSettings) -> ClaudeLaunch {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        let mut add = |a: &[&str]| args.extend(a.iter().map(std::ffi::OsString::from));
        add(&["--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config"]);
        args.push(s.mcp_config.into());
        let mut add = |a: &[&str]| args.extend(a.iter().map(std::ffi::OsString::from));
        add(&["--allowedTools", "mcp__rbxkit", "mcp__Roblox_Studio", "Read", "Write", "Edit", "Glob", "Grep", "WebSearch", "WebFetch"]);
        // the programs the user allowed, through either shell
        args.extend(s.allowed.iter().flat_map(|p| [format!("Bash({p}:*)"), format!("PowerShell({p}:*)")]).map(Into::into));
        // the app's settings (allowed programs, keys) are never written by an agent, in any mode: deny rules
        // hold in bypass mode too. `//` makes the path absolute in Claude Code's rules.
        args.push("--disallowedTools".into());
        args.extend(s.denied.iter().map(Into::into));
        let mut add = |a: &[&str]| args.extend(a.iter().map(std::ffi::OsString::from));
        // full auto skips every other permission check
        add(&["--permission-mode", if s.full { "bypassPermissions" } else { "acceptEdits" }, "--append-system-prompt", s.prompt, "--add-dir"]);
        args.push(s.kit.into());
        if let Some(p) = s.project {
            args.extend(["--add-dir".into(), p.into()]);
        }
        if let Some(m) = s.model {
            args.extend(["--model".into(), m.into()]);
        }
        if let Some(e) = s.effort {
            args.extend(["--effort".into(), e.into()]);
        }
        // the conversation lives in the project folder when there is one, so the agent works among the game's files
        let cwd = s.project.map(std::path::PathBuf::from).unwrap_or_else(|| s.work.to_path_buf());
        ClaudeLaunch { exe: s.exe.to_path_buf(), first: Vec::new(), args, cwd, servers: s.servers.to_string(), keys_on: s.keys_on }
    }

    /// The command: `live` for a process kept open and sent messages as JSON lines (each played back, so a turn's
    /// lines can be told from an earlier turn's), else one that reads its one message from stdin and exits.
    fn command(&self, live: bool, session: Option<&str>) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.first).arg("-p");
        if live {
            cmd.args(["--input-format", "stream-json", "--replay-user-messages"]);
        }
        cmd.args(&self.args);
        if let Some(s) = session {
            cmd.args(["--resume", s]);
        }
        cmd.current_dir(&self.cwd);
        // a texture request may wait minutes for the user
        cmd.env("MCP_TOOL_TIMEOUT", "900000");
        paths::quiet(&mut cmd);
        paths::own_group(&mut cmd); // Stop ends the agent and all it started (Unix), never the app
        paths::child_env(&mut cmd, "claude");
        if let Some(h) = claude_home() {
            cmd.env("HOME", h);
        }
        cmd
    }

    /// Everything a kept process was started with that a later message could need otherwise.
    fn key(&self) -> String {
        json!([self.exe, self.first.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>(), self.args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>(), self.cwd, self.servers, self.keys_on]).to_string()
    }
}

/// An AI kept open for a chat (`caps.live`): what its process is started with for a message, and how a turn runs on
/// it. Everything else about keeping a process is the same for every AI (`Agent::live_turn`, `read_live`): the launch
/// key, the fallback to a process per turn, idle closing, the ledger.
trait Kept: Clone + Send + 'static {
    /// The AI's id ("claude", "gemini").
    fn ai(&self) -> &'static str;
    /// Everything a kept process is started with, or decides by, that a later message could need otherwise.
    fn key(&self) -> String;
    /// Starts the process to keep, resuming `session` when the chat has one.
    fn start(&self, session: Option<&str>, key: String) -> std::io::Result<Arc<Live>>;
    /// The same message for a process of its own (the fallback): its command and what goes in on its stdin.
    fn per_turn(&self, session: Option<&str>, text: &str) -> (Command, String);
    /// One turn on the kept process: the message written and the output read to the turn's end. `first`: the
    /// process's first turn.
    fn turn(&self, a: &Arc<Agent>, live: &Arc<Live>, tp: &Arc<TurnProc>, text: &str, last: &Mutex<(std::time::Instant, bool, bool)>, first: bool) -> TurnEnd;
}

impl Kept for ClaudeLaunch {
    fn ai(&self) -> &'static str {
        "claude"
    }

    fn key(&self) -> String {
        ClaudeLaunch::key(self)
    }

    fn start(&self, session: Option<&str>, key: String) -> std::io::Result<Arc<Live>> {
        Live::start(self.command(true, session), key)
    }

    fn per_turn(&self, session: Option<&str>, text: &str) -> (Command, String) {
        (self.command(false, session), text.to_string())
    }

    fn turn(&self, a: &Arc<Agent>, live: &Arc<Live>, tp: &Arc<TurnProc>, text: &str, last: &Mutex<(std::time::Instant, bool, bool)>, first: bool) -> TurnEnd {
        let message = claude_live::message_id();
        if live.say(&message, text).is_err() {
            tp.end(); // its output then ends, and the turn with it (`read_live`: nothing shown, so it is sent again)
        }
        let rx = live.lines();
        let mut lines = std::iter::from_fn(|| rx.recv().ok());
        // noted in the ledger again at a new process's first line, as a per-turn process is: the command line the
        // ledger matches on is the one it has by then
        let on_first = || {
            if first {
                crate::procs::record("claude", live.pid);
            }
        };
        a.consume_lines(&mut lines, "claude", last, on_first, Some(LiveTurn { tp, message: &message, first, marks: &live.marks_turns }))
    }
}

/// What a Gemini turn is started with on the kept path (`agy acp`, see `acp`), and what its permission requests are
/// answered by. Built afresh for every message from the app's memory, as Claude's is; a kept process started with
/// anything else (`key`) is closed and a new one resumes the conversation, so no mode, folder, tool server, model or
/// allowed program can go stale inside one. The same settings give the per-message command (`per_turn`), which is
/// today's `antigravity::command` and resumes the same conversation: agy keeps both kinds in one store.
#[derive(Clone)]
struct GeminiLaunch {
    exe: std::path::PathBuf,
    /// arguments before all others: none for agy (the tests' stand-in is a shell script given to `sh`)
    first: Vec<std::ffi::OsString>,
    /// agy's data folder when it is not `~/.gemini` (FRIDAE_AGY_DIR, testing only): given as `--gemini-dir`
    data_dir: Option<std::path::PathBuf>,
    /// the workspace, the tool servers, the model and effort, and the rules its requests are answered by
    spec: crate::acp::Spec,
    /// whether the provider keys in the app's environment reach the process (`paths::child_env`)
    keys_on: bool,
    /// the programs the user allowed, and a fingerprint of every guarded setting (`paths::GUARDED`): a change to
    /// any of them starts another process
    allowed: Vec<String>,
    guarded: String,
}

/// Everything a Gemini turn's start depends on, as `send` has it for a message.
struct GeminiSettings<'a> {
    exe: &'a std::path::Path,
    agent_id: &'a str,
    /// the chat's folder: its game, or the app's folder for the chat
    workspace: &'a std::path::Path,
    servers: &'a Value,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    /// Full auto or Studio
    full: bool,
    kit: &'a std::path::Path,
    work: &'a std::path::Path,
    /// the app's own files (`paths::app_protected`)
    protected: &'a [std::path::PathBuf],
    /// the programs the user allowed, and all the guarded settings, from the app's memory
    allowed: &'a [String],
    guarded: &'a Map<String, Value>,
    keys_on: bool,
    /// the home folder agy runs with, and its data folder when not `<home>/.gemini`
    home: &'a std::path::Path,
    data_dir: Option<&'a std::path::Path>,
}

impl GeminiLaunch {
    fn new(s: &GeminiSettings) -> GeminiLaunch {
        use sha2::{Digest, Sha256};
        let rules = crate::acp::Rules {
            full: s.full,
            agent_id: s.agent_id.to_string(),
            servers: s.servers.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default(),
            // as a process per message has it: Ask me writes in the workspace only
            writes: vec![s.workspace.to_path_buf()],
            reads: vec![s.kit.to_path_buf(), s.work.join("attachments"), s.work.join("chat-images")],
            protected: s.protected.to_vec(),
            agent_dir: s.data_dir.map(std::path::Path::to_path_buf).unwrap_or_else(|| s.home.join(".gemini")).join("antigravity-cli"),
        };
        let spec = crate::acp::Spec { cwd: s.workspace.to_path_buf(), servers: s.servers.clone(), model: s.model.map(str::to_string), effort: s.effort.map(str::to_string), rules };
        // a fingerprint, not the settings themselves: one of them is the user's Gemini key
        let guarded = paths::hex(&Sha256::digest(Value::Object(s.guarded.clone()).to_string().as_bytes()));
        GeminiLaunch { exe: s.exe.to_path_buf(), first: Vec::new(), data_dir: s.data_dir.map(std::path::Path::to_path_buf), spec, keys_on: s.keys_on, allowed: s.allowed.to_vec(), guarded }
    }

    /// The kept process: `agy acp`, in the chat's workspace.
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.first).arg("acp");
        if let Some(d) = &self.data_dir {
            cmd.arg("--gemini-dir").arg(d);
        }
        cmd.current_dir(&self.spec.cwd);
        // a texture request may wait minutes for the user
        cmd.env("MCP_TOOL_TIMEOUT", "900000");
        paths::quiet(&mut cmd);
        paths::own_group(&mut cmd); // Stop ends the agent and all it started (Unix), never the app
        paths::child_env(&mut cmd, "gemini");
        if let Some(h) = agy_home() {
            cmd.env("HOME", h);
        }
        cmd
    }

    /// Everything a kept process was started with, or answers requests by, that a later message could need otherwise.
    fn key(&self) -> String {
        let r = &self.spec.rules;
        json!([self.exe, self.first.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>(), self.data_dir, self.spec.cwd, self.spec.servers.to_string(), self.spec.model, self.spec.effort,
               r.full, r.agent_id, r.servers, r.writes, r.reads, r.protected, r.agent_dir, self.keys_on, self.allowed, self.guarded])
        .to_string()
    }
}

/// What an ACP turn needs of the chat's turn (`acp::Host`).
struct AcpHost<'a> {
    a: &'a Arc<Agent>,
    tp: &'a TurnProc,
    last: &'a Mutex<(std::time::Instant, bool, bool)>,
    t: TurnEnd,
    /// the ledger's entry is still the one from the start: noted again at the process's first line
    renote: Option<(&'static str, u32)>,
}

impl crate::acp::Host for AcpHost<'_> {
    fn out(&mut self, out: Out) {
        self.a.take(out, &mut self.t);
    }

    fn heard(&mut self) {
        if let Some((owner, pid)) = self.renote.take() {
            crate::procs::record(owner, pid);
        }
        self.tp.heard.store(true, Ordering::SeqCst);
    }

    fn taken(&mut self) {
        self.tp.taken();
    }

    fn activity(&mut self, long_tool: bool, tool_open: bool) {
        self.tp.tool_open.store(tool_open, Ordering::SeqCst);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = (std::time::Instant::now(), long_tool, tool_open);
    }

    fn began(&mut self) {
        self.t.began = true;
    }

    fn end(&mut self) {
        self.tp.end();
    }
}

impl Kept for GeminiLaunch {
    fn ai(&self) -> &'static str {
        "gemini"
    }

    fn key(&self) -> String {
        GeminiLaunch::key(self)
    }

    fn start(&self, _session: Option<&str>, key: String) -> std::io::Result<Arc<Live>> {
        // the conversation is resumed over the protocol, not by an argument (`acp::turn`)
        Live::start_as(self.command(), key, "gemini", crate::acp::stray)
    }

    fn per_turn(&self, session: Option<&str>, text: &str) -> (Command, String) {
        let s = &self.spec;
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.first).args(antigravity::turn_args(&s.cwd, &s.servers, session, s.model.as_deref(), s.effort.as_deref(), s.rules.full));
        cmd.current_dir(&s.cwd);
        if let Some(h) = agy_home() {
            cmd.env("HOME", h);
        }
        (cmd, antigravity::message_line(text))
    }

    fn turn(&self, a: &Arc<Agent>, live: &Arc<Live>, tp: &Arc<TurnProc>, text: &str, last: &Mutex<(std::time::Instant, bool, bool)>, first: bool) -> TurnEnd {
        let resume = a.st.lock().unwrap().session.clone();
        let mut host = AcpHost { a, tp, last, t: TurnEnd::default(), renote: first.then_some(("gemini", live.pid)) };
        let end = crate::acp::turn(live, &self.spec, resume.as_deref(), text, &mut host);
        let mut t = host.t;
        t.ended = end.ended;
        t.unfit = end.unfit;
        t
    }
}

/// The home folder and data folder agy runs with when they are not the app's own (testing only, see
/// `antigravity::home_override`).
fn agy_home() -> Option<std::path::PathBuf> {
    antigravity::home_override()
}

fn agy_dir() -> Option<std::path::PathBuf> {
    antigravity::dir_override()
}

/// The switch that turns keeping a process off for an AI: `FRIDAE_CLAUDE_PER_TURN=1`, `FRIDAE_GEMINI_PER_TURN=1`.
fn per_turn_forced(ai: &str) -> bool {
    std::env::var(format!("FRIDAE_{}_PER_TURN", ai.to_ascii_uppercase())).is_ok_and(|v| v == "1")
}

/// FRIDAE_CLAUDE_HOME (testing only, debug builds only) gives a copy of the app with a scratch HOME the real one for
/// Claude Code alone, which finds its sign-in through the home folder's keychain (as FRIDAE_CURSOR_HOME does for
/// Cursor). The app's tool server gets the app's HOME back in its entry.
#[cfg(debug_assertions)]
fn claude_home() -> Option<std::path::PathBuf> {
    std::env::var_os("FRIDAE_CLAUDE_HOME").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

#[cfg(not(debug_assertions))]
fn claude_home() -> Option<std::path::PathBuf> {
    None
}

/// Starts, once, the thread that closes kept Claude Code processes nobody is using: one whose chat has been idle for
/// `claude_live::IDLE`, one that has died, and the longest idle beyond `claude_live::KEEP`.
fn keep_lives() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if cfg!(test) {
        return; // the tests call `close_idle_lives` themselves
    }
    ONCE.call_once(|| {
        thread::spawn(|| {
            loop {
                thread::sleep(Duration::from_secs(15));
                if shutting() {
                    return;
                }
                close_idle_lives(agents(), claude_live::IDLE, claude_live::KEEP);
            }
        });
    });
}

/// One round of `keep_lives`. Returns how many were closed.
fn close_idle_lives(chats: Vec<Arc<Agent>>, idle: Duration, keep: usize) -> usize {
    let mut open: Vec<(Duration, bool, Arc<Agent>)> = chats
        .into_iter()
        .filter_map(|a| {
            let live = {
                let st = a.st.lock().unwrap();
                st.live.clone().filter(|_| !st.running && !st.sending)
            }?;
            Some((live.idle_for(), live.exited(), a))
        })
        .collect();
    open.sort_by(|a, b| b.0.cmp(&a.0)); // longest idle first
    let over = open.len().saturating_sub(keep);
    let mut closed = 0;
    for (n, (quiet, dead, a)) in open.into_iter().enumerate() {
        if dead || quiet >= idle || n < over {
            a.close_live();
            closed += 1;
        }
    }
    closed
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

    /// A helper's result was handed back: from now on it holds an agent slot only while it works.
    pub fn mark_result(&self) {
        self.st.lock().unwrap().result_written = true;
        self.put("handed_back", json!({})); // the window shows the chat as finished, never as needing the user
    }

    /// Whether this chat takes one of the MAX_AGENTS slots now (see `holds_slot`).
    pub fn holds_slot(&self) -> bool {
        holds_slot(&self.st.lock().unwrap())
    }

    /// The run this chat leads was ended in Review: the night's clock and autopilot stop, a wait at a gate is over,
    /// and the next carry-on is dropped. A turn already under way finishes, and then nothing carries on (`ended_step`).
    pub fn end_run(&self) {
        let (was_waiting, requeued) = {
            let mut st = self.st.lock().unwrap();
            // queued wakes go; what the user typed stays queued, as ordinary messages (see drop_wakes)
            let requeued = drop_wakes(&mut st.queue, true);
            st.overnight_until = 0.0;
            st.overnight_left = 0.0;
            st.autopilot_left = 0;
            if !working(&st) {
                st.halted = true;
                // an ended run's Lead keeps no Claude Code process (one still in a turn goes with the idle rule)
                if let Some(l) = st.live.take() {
                    thread::spawn(move || l.close());
                }
            }
            (std::mem::take(&mut st.studio_waiting), requeued)
        };
        if requeued {
            self.show_queue();
        }
        if was_waiting {
            self.put("studio_waiting", json!({"value": false, "left": 0}));
        }
    }

    /// Drop the next carry-on of the loop (the flag Stop sets between turns): an undo is about to put things back.
    pub fn halt(&self) {
        let mut st = self.st.lock().unwrap();
        if !working(&st) {
            st.halted = true;
        }
    }

    #[cfg(test)]
    pub fn set_running_for_test(&self, on: bool) {
        self.st.lock().unwrap().running = on;
    }

    /// A turn "running" as the given process, for the Stop tests.
    #[cfg(test)]
    pub fn set_turn_pid_for_test(&self, pid: u32) -> Arc<TurnProc> {
        let mut st = self.st.lock().unwrap();
        st.running = true;
        st.pid = Some(pid);
        let tp = TurnProc::new(pid);
        st.proc_ = Some(tp.clone());
        tp
    }

    /// The text of the chat's latest "user" event (what was sent), for tests.
    #[cfg(test)]
    pub fn last_user_text_for_test(&self) -> Option<String> {
        self.st.lock().unwrap().log.iter().rev().find(|e| is_user(e)).and_then(|e| e["text"].as_str()).map(str::to_string)
    }

    #[cfg(test)]
    pub fn set_waiting_for_test(&self, on: bool) {
        self.st.lock().unwrap().studio_waiting = on;
    }

    /// A chat for a test that is not among the open ones (so no other test's sweep over the open chats touches it).
    #[cfg(test)]
    pub fn detached_for_test(id: &str, project: Option<&str>) -> Arc<Agent> {
        let a = Agent::new(id.to_string(), "Lead");
        a.st.lock().unwrap().project = project.map(str::to_string);
        a
    }

    #[cfg(test)]
    pub fn notes_for_test(&self) -> Vec<String> {
        self.st.lock().unwrap().notes.clone()
    }

    /// The chat that started this helper, if another chat did.
    pub fn parent(&self) -> Option<String> {
        self.st.lock().unwrap().parent.clone()
    }

    /// A note for the agent, sent with its next message.
    pub fn note(&self, text: &str) {
        self.st.lock().unwrap().notes.push(text.to_string());
    }

    /// This chat's studio run undo log: its own Studio steps and its experts', oldest first (see `merge_run_log`).
    pub fn run_log(&self) -> Vec<Value> {
        merge_run_log(&self.st.lock().unwrap().log)
    }

    /// Whether this chat's last message ran in Studio mode: it leads (or led) a studio run.
    pub fn studio_lead(&self) -> bool {
        let st = self.st.lock().unwrap();
        st.last_ask.studio && st.role.is_none()
    }

    /// A helper's turn ended: when its parent leads a studio run, the Studio steps of this turn join the Lead's undo log.
    pub fn hand_steps_to_lead(&self) {
        let Some(lead) = self.parent().and_then(|p| get(&p)).filter(|l| l.studio_lead()) else { return };
        let steps = {
            let st = self.st.lock().unwrap();
            studio_steps_since(&st.log, st.turn_started)
        };
        if !steps.is_empty() {
            lead.put("helper_steps", json!({"name": self.name(), "agent": self.id, "steps": steps}));
        }
    }

    /// The studio run of this chat's game, when this chat leads it and it is paused.
    fn paused_run(&self) -> bool {
        let Some(p) = self.project() else { return false };
        studio_undo::paused_lead(&p, &self.id)
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
    /// `files_only`: Studio is left as it is (offered when Studio's part cannot be done: "files_only" in the answer).
    pub fn undo_night(&self, files_only: bool) -> Value {
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
        if night["run"] == true {
            return json!({"ok": false, "text": "This chat leads a studio run: use Undo this run, which also puts the run's stages and review back."});
        }
        let start = night["start"].as_f64().unwrap_or(0.0);
        let back = studio_undo::put_back(
            &studio_undo::Checkpoint {
                at: start,
                studio: night["studio"] == true,
                project: night["project"].as_str(),
                commit: night["commit"].as_str(),
                began: "the night began",
                undoing: None,
                skip_studio: files_only.then_some("Studio: left as it is (you chose to undo the files only)."),
                // as before Studio mode: the agents' Studio steps since the night began are undone by time, even when
                // Studio was closed when it began (the scripts are still saved to disk first)
                exact: false,
            },
            &studio_undo::Live,
        );
        if let Some(why) = back.refused {
            // nothing was changed: Studio's part cannot be done, the files alone still can
            let text = if night["commit"].is_string() { format!("{why} \u{2014} undo the files only?") } else { format!("{why}.") };
            return json!({"ok": false, "text": text, "files_only": night["commit"].is_string()});
        }
        let (mut lines, ok) = (back.lines, back.studio_ok && back.project_ok);
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

    /// The model this chat runs on: the one it was started on, else the one its last message asked for (None: the
    /// AI's default).
    pub fn model(&self) -> Option<String> {
        let st = self.st.lock().unwrap();
        st.model.clone().or_else(|| st.last_ask.model.clone())
    }

    /// Choose this chat's AI from the window. Before the conversation has started this only sets it (`Ok(false)`).
    /// A started chat carries on with the other AI (`Ok(true)`): a fresh session, handed the chat so far as text with
    /// its first message (`handover_text`). Refused while the chat works, or for an AI the app does not know.
    pub fn switch_ai(&self, kind: &str) -> Result<bool, String> {
        if !paths::AIS.contains(&kind) {
            return Err("That AI is not one the app knows.".into());
        }
        if self.set_ai(kind) {
            return Ok(false);
        }
        let from = {
            let mut st = self.st.lock().unwrap();
            if working(&st) || st.making {
                return Err("Wait until it finishes (or stop it), then switch.".into());
            }
            let from = st.ai.clone().unwrap_or_else(paths::default_ai);
            if from == kind {
                return Ok(false);
            }
            // an earlier handover not yet sent is part of the log it is rebuilt from, so nothing is lost
            st.handover = Some(handover_text(&st.log, ai_label(&from), HANDOVER_CAP));
            st.session = None;
            st.ai = Some(kind.to_string());
            from
        };
        self.put("ai", json!({"ai": kind, "from": from}));
        self.save();
        // the AI it leaves keeps no process
        if let Some(l) = self.st.lock().unwrap().live.take() {
            thread::spawn(move || l.close());
        }
        Ok(true)
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
        working(&self.st.lock().unwrap())
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
        if matches!(kind, "user" | "text" | "images" | "error" | "done" | "renamed" | "texdone" | "night" | "night_undone" | "studio_waiting" | "helper_steps" | "run_undone") || (kind == "tool" && stale) {
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
            json!({"id": self.id, "name": st.name, "session": st.session, "project": st.project, "role": st.role, "expert": st.expert, "expert_text": st.expert_text, "parent": st.parent, "team": st.team, "result_written": st.result_written, "ai": st.ai, "model": st.model, "handover": st.handover, "updated": paths::now_secs(), "events": st.log})
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
        // a paused studio run's Lead keeps the user's messages until Resume, so the run does not start up again
        let auto = ask.kickoff.as_deref() == Some("auto");
        // (any message to a run's Lead is a Studio message: see studio_message)
        let hold = !auto && !self.busy() && self.paused_run();
        // a finished helper takes an agent slot again to work, so a message to it needs a free one
        if no_slot_for(self.holds_slot(), || slots_taken(&AGENTS.lock().unwrap())) {
            self.put("error", json!({"text": crate::team::NO_SLOT}));
            return;
        }
        let (queued, held) = {
            let mut st = self.st.lock().unwrap();
            if working(&st) || hold {
                let qid = paths::random_hex(1)[..8].to_string();
                st.queue.push((qid, text.to_string(), ask.clone()));
                (true, Vec::new())
            } else {
                // messages kept while the run was paused go first, in the same turn
                (false, std::mem::take(&mut st.queue))
            }
        };
        if queued {
            self.show_queue();
            self.nudge();
            return;
        }
        if held.is_empty() {
            return self.send(text, ask);
        }
        self.show_queue();
        let typed = merged_typed(&held.iter().map(|(_, t, a)| (t.as_str(), a)).chain([(text, ask)]).collect::<Vec<_>>());
        let text = held.iter().map(|(_, t, _)| t.as_str()).chain([text]).collect::<Vec<_>>().join("\n\n");
        let ask = Ask {
            attachments: held.iter().flat_map(|(_, _, a)| a.attachments.clone()).chain(ask.attachments.clone()).collect(),
            studio: merged_studio(&held, ask.studio),
            wake: ask.wake && held.iter().all(|(_, _, a)| a.wake),
            app: (ask.app || ask.wake) && held.iter().all(|(_, _, a)| a.app || a.wake),
            typed,
            ..ask.clone()
        };
        self.send(&text, &ask);
    }

    /// A Studio run goes on: a decision from Review, or Resume. Sent as a message in Studio mode (full auto, overnight's
    /// loop) with the model and effort of the chat's last message; held like any message while the chat is working.
    /// Returns at once: the turn runs on its own thread.
    pub fn wake_studio(self: &Arc<Self>, text: &str) {
        // paused: the decision is in its review item, and rbx_stage hands it over after Resume
        if self.paused_run() {
            return;
        }
        let ask = {
            let st = self.st.lock().unwrap();
            Ask { studio: true, full: true, overnight: true, wake: true, model: st.last_ask.model.clone(), effort: st.last_ask.effort.clone(), ..Default::default() }
        };
        let (me, text) = (self.clone(), text.to_string());
        thread::spawn(move || me.ask(&text, &ask));
    }

    /// A studio run's Lead has something waiting (a typed message or an app wake held behind its turn): a small file
    /// in WORK/team-nudge/<id> tells its `rbx_team wait` to hand it back early (see rbxteam.py). Cleared when the turn
    /// ends (`clear_nudge`). Other chats get none: their wait is unchanged.
    fn nudge(&self) {
        if !self.studio_lead() {
            return;
        }
        let (typed, wakes) = {
            let st = self.st.lock().unwrap();
            if !working(&st) || st.queue.is_empty() {
                return;
            }
            let wakes = st.queue.iter().filter(|(_, _, a)| a.wake).count();
            (st.queue.len() - wakes, wakes)
        };
        let dir = paths::work("team-nudge");
        let _ = fs::create_dir_all(&dir);
        let tmp = dir.join(format!("{}.tmp", self.id));
        if fs::write(&tmp, json!({"typed": typed, "wakes": wakes}).to_string()).is_ok() {
            let _ = fs::rename(&tmp, dir.join(&self.id));
        }
    }

    pub fn nudge_file(&self) -> std::path::PathBuf {
        paths::work("team-nudge").join(&self.id)
    }

    fn clear_nudge(&self) {
        let _ = fs::remove_file(self.nudge_file());
    }

    /// Take back a held message before it is sent.
    pub fn unqueue(&self, qid: &str) -> bool {
        let (removed, empty) = {
            let mut st = self.st.lock().unwrap();
            let before = st.queue.len();
            st.queue.retain(|(id, _, _)| id != qid);
            (st.queue.len() < before, st.queue.is_empty())
        };
        if empty {
            self.clear_nudge();
        }
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
        self.used_in(turn, None);
    }

    /// The same, for a turn of a Claude Code session (whose cost was made per-turn from the session's total).
    fn used_in(&self, turn: &usage::Turn, session: Option<&str>) {
        if turn.is_empty() {
            return;
        }
        usage::record(&self.ai(), &self.id, self.project().as_deref(), turn, session);
        self.put("usage", turn.json());
    }

    fn show_queue(&self) {
        let (items, idle): (Vec<Value>, bool) = {
            let st = self.st.lock().unwrap();
            (st.queue.iter().map(|(id, text, _)| json!({"qid": id, "text": text})).collect(), !(st.running || st.making))
        };
        // messages kept while the chat is idle are held for a paused studio run's Resume, or (the run was ended
        // meanwhile) go with the user's next message
        let kept = idle && !items.is_empty();
        let paused = kept && self.paused_run();
        self.put("queued", json!({"items": items, "paused": paused, "kept": kept && !paused}));
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
        if self.busy() || shutting() {
            return;
        }
        // before anything of the turn reads a setting: one changed behind the app's back is put back first
        self.check_settings();
        let has_project = self.project().is_some_and(|p| std::path::Path::new(&p).is_dir());
        // the Lead of a running studio run: what reaches it is a Studio message, and the run's start checkpoint is kept
        let run = match self.project().filter(|_| has_project && self.role().is_none()) {
            Some(p) => studio_undo::lead_of(&p, &self.id),
            None => studio_undo::LeadOf::default(),
        };
        let lead = {
            let st = self.st.lock().unwrap();
            lead_ask(ask, run, st.studio_waiting, &st.last_ask)
        };
        // a wake or carry-on for a run that was ended in Review goes nowhere
        let Some(ask) = lead else { return };
        let ask = &ask;
        let (mut ask, fallback) = studio_fallback(ask, has_project);
        // a mode this chat's AI cannot keep to by its own rules is not run: the message goes as Ask me (`caps`)
        let (full, studio, mode_note) = crate::caps::mode_for(&self.ai(), ask.full, ask.studio);
        if mode_note.is_some() {
            ask = Ask { full, studio, overnight: false, autopilot: false, ..ask };
        }
        let fallback = fallback.or(mode_note);
        let ask = &ask;
        let (selftest, gemini, autopilot) = (ask.selftest, ask.gemini, ask.autopilot || ask.full);
        // an expert keeps the model it was started on (the user's Setup > Experts row, the Lead's pick or the Lead's own),
        // even when the user writes in its chat (the page sends the menu's model for every chat); another chat with a
        // pinned model uses it when a message names none
        let (pinned, helper) = {
            let st = self.st.lock().unwrap();
            (st.model.clone(), st.parent.is_some())
        };
        let model = if helper { pinned.as_deref().or(ask.model.as_deref()) } else { ask.model.as_deref().or(pinned.as_deref()) };
        let (attachments, kickoff) = (ask.attachments.as_slice(), ask.kickoff.as_deref());
        let auto = kickoff == Some("auto");
        // a carry-on decided before the run was paused (or undone) does not run as it was: see paused_carry_on
        let text: &str = if auto && self.studio_lead() && self.paused_run() {
            match paused_carry_on(text, &mut self.st.lock().unwrap().pause_told) {
                Some(t) => t,
                None => return,
            }
        } else {
            text
        };
        let Some(resumed) = begin_send(&mut self.st.lock().unwrap(), ask, auto, paths::now_secs()) else {
            return;
        };
        let _sending = Sending(self);
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
        // each AI has its own models: Claude's four families, or whatever Antigravity, Codex or Cursor lists
        // each AI starts on a set model rather than whatever its own program defaults to
        let model: Option<String> = match self.ai().as_str() {
            "claude" => Some(model.filter(|m| MODELS.contains(m)).unwrap_or("opus").to_string()),
            "gemini" => model.filter(|m| antigravity::models().iter().any(|(id, _)| id == m)).map(str::to_string).or_else(antigravity::default_model),
            "cursor" => cursor::pick_model(model),
            "opencode" => opencode::pick_model(model),
            _ => model.filter(|m| codex::models().iter().any(|(id, _)| id == m)).map(str::to_string).or_else(codex::default_model),
        };
        let model = model.as_deref();
        // an AI whose model ids carry their own effort is not given the setting (`caps`)
        let effort = ask.effort.as_deref().filter(|e| EFFORTS.contains(e)).filter(|_| crate::caps::of(&self.ai()).effort);
        let infos: Vec<attach::Info> = attachments.iter().take(MAX_ATTACH).map(|n| paths::base_name(n)).filter_map(|n| attach::info(&n)).collect();
        self.put(
            "user",
            json!({
                "text": text, "selftest": selftest, "model": model, "gemini": gemini, "autopilot": autopilot || ask.overnight, "auto": auto, "app": ask.wake || ask.app, "typed": ask.typed,
                "overnight": ask.overnight, "full": ask.full, "until": self.st.lock().unwrap().overnight_until,
                "studio": ask.studio, "studio_waiting": false, "fallback": fallback,
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
        if takes_night_checkpoint(ask, auto, resumed, run.started) {
            let start = self.turn_started();
            plugin::pin(start);
            let project = self.project().filter(|p| std::path::Path::new(p).is_dir());
            if let Some(p) = project.as_deref().filter(|_| ask.studio) {
                studio_undo::ensure_gitignore(std::path::Path::new(p)); // review media stays out of the checkpoint commits
            }
            let commit = project.as_deref().and_then(|p| projects::night_checkpoint(p, ask.studio));
            let studio = plugin::connected();
            // a studio run's night is undone with Undo this run, never with Undo the night ("run")
            self.put("night", json!({"start": start, "commit": commit, "project": project, "studio": studio, "run": ask.studio}));
            // the first checkpoint of a studio run is its start, so "Undo this run" can go all the way back
            if let Some(p) = project.as_deref().filter(|_| ask.studio) {
                let saved = studio_undo::Saved { git_failed: commit.is_none() && projects::is_repo(p), commit: commit.clone(), studio, at: start };
                studio_undo::record_start(p, &self.id, &saved);
            }
        }
        let notes = std::mem::take(&mut self.st.lock().unwrap().notes);
        if !notes.is_empty() {
            text += &format!("\n\n[{}]", notes.join("\n"));
        }
        self.put("busy", json!({"value": true}));
        let kind = self.ai();
        // tests never start an AI, whatever is installed on the computer running them
        let found = if cfg!(test) { None } else { paths::find_ai(&kind) };
        let Some(exe) = found else {
            let missing = match kind.as_str() {
                // a chat saved on Cursor or OpenCode, reopened where they are not offered (Windows, for now)
                k if !crate::caps::available(k) => {
                    "This chat was set to an AI that Fridae Studios does not offer on this computer yet. Pick Claude or Gemini under the message box and send your message again: the chat so far is handed over."
                }
                "gemini" => "This chat is set to Gemini, but Google's Antigravity command-line tool (agy) was not found. Install it from https://antigravity.google, run `agy` once to sign in, then reopen Fridae Studios.",
                "cursor" => "This chat is set to Cursor, but the Cursor Agent command-line tool (cursor-agent) was not found. Install it from https://cursor.com/cli, run `cursor-agent login` once in a terminal, then reopen Fridae Studios.",
                "opencode" => "This chat is set to OpenCode, but OpenCode's command-line tool (opencode) was not found. Install it from https://opencode.ai, then reopen Fridae Studios. Fridae signs in to it on its own (Setup, Connections).",
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
        // the AI was switched in a started chat: the new one gets the chat so far with its first message. It is kept
        // until that AI has opened a conversation (`got_session`), so a first turn that fails or is stopped hands it over again.
        // Cursor asked to resume a chat it no longer has (its saved chats were cleared) starts an empty one under that id
        // without a word: the chat starts afresh with the chat so far instead (up to this message, which follows anyway)
        if kind == "cursor" {
            stale_chat(&mut self.st.lock().unwrap(), "Cursor", cursor::chat_saved);
        }
        // OpenCode: a chat its server no longer has (its folder was cleared) starts afresh the same way
        if kind == "opencode" {
            let session = self.st.lock().unwrap().session.clone();
            let saved = session.as_deref().is_none_or(opencode::chat_saved);
            stale_chat(&mut self.st.lock().unwrap(), "OpenCode", |_| saved);
        }
        text = with_handover(&self.st.lock().unwrap(), &text);
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
        // the user's house rules come last, for every agent (the Lead and experts too), read fresh each turn
        prompt += &house_rules(&PATHS.home, project.as_deref());
        let _ = fs::create_dir_all(&PATHS.work);
        let servers = mcp_servers(&self.id, &self.name(), project.as_deref(), role.as_deref());
        let session = self.st.lock().unwrap().session.clone();
        // every AI but Claude Code takes its standing instructions inside the conversation, and works in a folder
        // of its own (the project, or one made for this chat) because that is where it looks for its tool servers.
        // Cursor always works in the app's folder for the chat, with the game added, so nothing is written in the game.
        let workspace = match (kind.as_str(), &project) {
            ("cursor", _) => cursor::workspace(&self.id),
            (_, Some(p)) => std::path::PathBuf::from(p),
            (_, None) => paths::work(&kind).join(&self.id),
        };
        if kind != "claude" {
            let _ = fs::create_dir_all(&workspace);
            // Cursor has its own text: it may run the programs the user allowed, and a refused one becomes a card
            if kind == "cursor" {
                prompt += &cursor::instructions(project.as_deref(), &paths::allowed_commands());
            } else if kind == "opencode" {
                prompt += &opencode::instructions(project.as_deref(), &paths::allowed_commands(), ask.full);
            } else {
                prompt += OTHER_AI_PROMPT;
            }
            // OpenCode takes the standing instructions on the session itself, afresh every turn (see `opencode::run`)
            text = match &session {
                _ if kind == "opencode" => text,
                None => antigravity::with_instructions(&prompt, &text),
                Some(_) if !switches.is_empty() => antigravity::with_instructions(&switches, &text),
                Some(_) => text,
            };
        }
        // OpenCode runs on the app's own OpenCode server, not as a program per turn
        if kind == "opencode" {
            {
                let mut st = self.st.lock().unwrap();
                st.running = true;
                st.stopped = false;
                st.pid = None;
            }
            let spec = opencode::Spec {
                agent_id: self.id.clone(),
                title: format!("Fridae · {}", self.name()),
                location: workspace.clone(),
                project: project.as_ref().map(std::path::PathBuf::from),
                servers: servers.clone(),
                session: session.clone(),
                model: model.map(str::to_string),
                full: ask.full,
                instructions: prompt.clone(),
                text: text.clone(),
            };
            let me = self.clone();
            thread::spawn(move || me.read_opencode(spec));
            return;
        }
        let cmd = match kind.as_str() {
            "gemini" => {
                let mut servers = servers.clone();
                if agy_home().is_some() {
                    servers["rbxkit"]["env"]["HOME"] = json!(paths::home_dir().display().to_string());
                }
                let protected: Vec<std::path::PathBuf> = paths::app_protected().into_iter().map(|(p, _)| p).collect();
                let launch = GeminiLaunch::new(&GeminiSettings {
                    exe: &exe,
                    agent_id: &self.id,
                    workspace: &workspace,
                    servers: &servers,
                    model,
                    effort,
                    full: ask.full,
                    kit: &PATHS.kit,
                    work: &PATHS.work,
                    protected: &protected,
                    allowed: &paths::allowed_commands(),
                    guarded: &paths::guarded_settings(),
                    keys_on: paths::env_keys_on(),
                    home: &agy_home().unwrap_or_else(paths::home_dir),
                    data_dir: agy_dir().as_deref(),
                });
                return self.kept_turn(&launch, &text);
            }
            "chatgpt" => codex::command(&exe, &workspace, &servers, session.as_deref(), model, effort, ask.full),
            // its models carry their own effort in the id, so the effort setting is not passed
            "cursor" => cursor::command(&exe, &workspace, &servers, session.as_deref(), model, ask.full, project.as_deref(), &PATHS.kit),
            _ => {
                let mut servers = servers.clone();
                if claude_home().is_some() {
                    servers["rbxkit"]["env"]["HOME"] = json!(paths::home_dir().display().to_string());
                }
                let launch = ClaudeLaunch::new(&ClaudeSettings {
                    exe: &exe,
                    mcp_config: &write_mcp_config(&self.id, &servers),
                    servers: &servers,
                    allowed: &paths::allowed_commands(),
                    denied: &claude_denied(),
                    full: ask.full,
                    prompt: &prompt,
                    kit: &PATHS.kit,
                    project: project.as_deref(),
                    model,
                    effort,
                    work: &PATHS.work,
                    keys_on: paths::env_keys_on(),
                });
                return self.claude_turn(&launch, &text);
            }
        };
        self.spawn_turn(cmd, &kind, &text);
    }

    /// A Claude Code turn: on the chat's kept process when that works here, on a process of its own otherwise.
    fn claude_turn(self: &Arc<Self>, launch: &ClaudeLaunch, text: &str) {
        self.kept_turn(launch, text)
    }

    /// A turn of an AI that can be kept open (`caps.live`): on the chat's kept process when that works here, on a
    /// process of its own otherwise.
    fn kept_turn<L: Kept>(self: &Arc<Self>, launch: &L, text: &str) {
        if self.live_allowed() && self.live_turn(launch, text) {
            return;
        }
        let session = self.st.lock().unwrap().session.clone();
        let (cmd, input) = launch.per_turn(session.as_deref(), text);
        self.spawn_turn(cmd, launch.ai(), &input);
    }

    /// Starts a command-line AI's process for one turn, hands it the message and reads it to its end on a thread of
    /// its own. False when it could not be started (said in the chat, and the turn is over).
    fn spawn_turn(self: &Arc<Self>, mut cmd: Command, kind: &str, text: &str) -> bool {
        // a texture request may wait minutes for the user
        cmd.env("MCP_TOOL_TIMEOUT", "900000").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        paths::quiet(&mut cmd);
        paths::own_group(&mut cmd); // Stop ends the agent and all it started (Unix), never the app
        paths::child_env(&mut cmd, kind);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.st.lock().unwrap().turned = true;
                self.put("error", json!({"text": format!("Could not start the agent: {e}")}));
                self.put("done", json!({}));
                team::release(self);
                return false;
            }
        };
        crate::procs::record(kind, child.id()); // ended at the next start if the app crashes meanwhile
        let tp = TurnProc::new(child.id());
        {
            let mut st = self.st.lock().unwrap();
            st.running = true;
            st.stopped = false;
            st.pid = Some(child.id());
            st.proc_ = Some(tp.clone());
        }
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let (me, kind) = (self.clone(), kind.to_string());
        thread::spawn(move || me.read(child, &kind, tp));
        true
    }

    /// Whether this chat's turns go to one kept process (`live`): an AI the table says works that way (`caps.live`),
    /// where its Stop was measured (not Windows), unless its switch says a process per turn
    /// (`FRIDAE_CLAUDE_PER_TURN=1`, `FRIDAE_GEMINI_PER_TURN=1`), and until keeping one has failed twice in this chat.
    fn live_allowed(&self) -> bool {
        let ai = self.ai();
        crate::caps::of(&ai).live && !cfg!(windows) && !per_turn_forced(&ai) && self.st.lock().unwrap().live_fails < LIVE_FAILS
    }

    /// Sends a turn's message to the chat's kept Claude Code process, starting one when the chat has none, when the
    /// one it has was started with other arguments (another mode, model, effort, allowed program, house rule, folder:
    /// it is closed, and the new one resumes the conversation), or when it has died meanwhile. False when that did
    /// not work: the turn then goes to a process of its own.
    fn live_turn<L: Kept>(self: &Arc<Self>, launch: &L, text: &str) -> bool {
        let key = launch.key();
        let held = self.st.lock().unwrap().live.take();
        let live = match held {
            Some(l) if l.key == key && l.drain() && !l.exited() => Some(l),
            Some(l) => {
                if l.key == key && !l.strayed() {
                    self.st.lock().unwrap().live_fails += 1; // it died between two turns
                }
                // closed beside the new one, which resumes the conversation: Stop is never kept waiting for it
                thread::spawn(move || l.close());
                None
            }
            None => None,
        };
        if !self.live_allowed() {
            return false;
        }
        let live = match live {
            Some(l) => l,
            None => {
                let session = self.st.lock().unwrap().session.clone();
                match launch.start(session.as_deref(), key) {
                    Ok(l) => l,
                    Err(_) => {
                        self.st.lock().unwrap().live_fails += 1;
                        return false;
                    }
                }
            }
        };
        let tp = TurnProc::on(&live);
        {
            let mut st = self.st.lock().unwrap();
            // Stop, or the chat closed, since this message began to be sent (`stop_turn` finds no turn then and
            // notes it): the message is not written, and the turn ends here
            if st.halted || st.closed {
                st.turned = true;
                if st.closed {
                    thread::spawn(move || live.close());
                } else {
                    st.live = Some(live); // nothing was written to it: it stays for the next message
                }
                drop(st);
                self.put("error", json!({"text": "Stopped."}));
                self.put("done", json!({}));
                team::release(self);
                return true;
            }
            // from here Stop finds the turn; the message is written by the turn's own thread
            st.running = true;
            st.stopped = false;
            st.pid = Some(live.pid);
            st.proc_ = Some(tp.clone());
            st.live = Some(live.clone());
        }
        keep_lives();
        let (me, launch, text) = (self.clone(), launch.clone(), text.to_string());
        thread::spawn(move || me.read_live(live, tp, launch, &text));
        true
    }

    /// A turn on the chat's kept Claude Code process: its message written, its output read to the `result`.
    ///
    /// The process then stays for the next message, unless something says it must not be used again: the turn failed
    /// (a Claude Code that is signed out, or cannot resume, is not asked twice), Stop landed in a tool call, the turn
    /// left a task running in the background, the chat is an expert's or helper's (it holds nothing between jobs) or
    /// was closed. Then it is closed, and the next message starts one that resumes.
    ///
    /// When its output ends before the `result` it has died or been ended: it is reaped as a per-turn process is. A
    /// turn that had shown nothing yet and was not stopped is sent again to a process of its own, so the user gets
    /// the answer and not the failure; one that showed nothing at all within START_WAIT also keeps the chat on a
    /// process per turn from then on.
    fn read_live<L: Kept>(self: &Arc<Self>, live: Arc<Live>, tp: Arc<TurnProc>, launch: L, text: &str) {
        let kind = launch.ai();
        let last = Arc::new(Mutex::new((std::time::Instant::now(), false, false)));
        {
            let (me, tp, last) = (self.clone(), tp.clone(), last.clone());
            thread::spawn(move || watch_stall(me, tp, last));
        }
        {
            let tp = tp.clone();
            thread::spawn(move || watch_start(tp));
        }
        let first = live.turns.fetch_add(1, Ordering::SeqCst) == 0;
        live.set_in_turn(true);
        let mut t = launch.turn(self, &live, &tp, text, &last, first);
        live.set_in_turn(false);
        {
            // under the gate the stall guard decides under, so a turn that ends now is never called stalled
            let _gate = tp.reaped.lock().unwrap_or_else(|e| e.into_inner());
            tp.done.store(true, Ordering::SeqCst);
        }
        let asked_to_stop = self.st.lock().unwrap().stopped;
        // a turn that failed without Stop: the process is given up. Its stdin is closed and its output read to the
        // end first, so what it says on its error output as it goes ("No conversation found") is the turn's reason
        if !t.ended && t.result_error && !asked_to_stop {
            live.shut(claude_live::CLOSE_WAIT);
            t.ended = true;
        }
        let mine = |st: &State| st.live.as_ref().is_some_and(|l| Arc::ptr_eq(l, &live));
        if !t.ended {
            live.touch();
            self.check_settings();
            let killed = tp.killed.load(Ordering::SeqCst);
            let drop_it = {
                let mut st = self.st.lock().unwrap();
                let drop_it = is_helper(&st) || shutting() || st.closed || killed || t.background_left || tp.discard.load(Ordering::SeqCst);
                if drop_it && mine(&st) {
                    st.live = None;
                }
                drop_it
            };
            let stopped = self.turn_over();
            if killed {
                self.put("error", json!({"text": paths::killed_message(ai_label(kind))}));
            }
            self.after_turn(kind, t, "", Some((true, 0)), stopped);
            if drop_it {
                live.close(); // after the turn is over for the chat: an expert reports without waiting for the exit
            }
            return;
        }
        {
            let mut st = self.st.lock().unwrap();
            if mine(&st) {
                st.live = None;
            }
        }
        // as `finish_turn` ends a per-turn process: what is left in its group, its error output, the leader reaped
        let groups = crate::procs::tree_groups(live.pid);
        if paths::leader_exited(live.pid, LEADER_GRACE) {
            paths::end_group_soon(live.pid);
        } else {
            paths::end_trees(&groups); // it would not go by itself
        }
        let err = live.err_text(paths::STOP_GRACE + Duration::from_secs(2));
        tp.reaping();
        let status = live.reap();
        self.check_settings();
        let (killed, stalled, silent) = (tp.killed.load(Ordering::SeqCst), tp.stalled.load(Ordering::SeqCst), tp.silent.load(Ordering::SeqCst));
        if silent {
            // it takes no messages: no further turn of this chat is tried on a kept process
            self.st.lock().unwrap().live_fails = LIVE_FAILS;
        }
        // lost: it went without a result, and not by Stop, the stall guard, quitting, a turn it began by itself, or
        // to say the conversation is gone
        let stopped_now = asked_to_stop || self.st.lock().unwrap().stopped;
        let lost = !t.got_result && !stopped_now && !killed && !stalled && !shutting() && !live.strayed() && !err.contains("No conversation found");
        if lost {
            // a message the kept process could not serve at all is no failure of keeping one
            if !silent && !t.unfit {
                self.st.lock().unwrap().live_fails += 1;
            }
            if !t.began {
                let session = self.st.lock().unwrap().session.clone();
                let (cmd, input) = launch.per_turn(session.as_deref(), text);
                if !self.spawn_turn(cmd, kind, &input) {
                    self.turn_over();
                }
                return;
            }
        }
        let stopped = self.turn_over();
        if killed {
            self.put("error", json!({"text": paths::killed_message(ai_label(kind))}));
        }
        let err = if stalled { STALLED.to_string() } else { err };
        // a failed result is the turn's end even though the process was then closed by the app (its exit says nothing)
        let exit = if t.got_result { Some((true, 0)) } else { status };
        self.after_turn(kind, t, &err, exit, stopped);
    }

    /// Closes the chat's kept Claude Code process, when it has one and no turn is running on it. Blocks for as long
    /// as the process takes to exit (about a second): call it off the page's thread.
    fn close_live(&self) {
        let live = {
            let mut st = self.st.lock().unwrap();
            if st.running || st.sending { None } else { st.live.take() }
        };
        if let Some(l) = live {
            l.close();
        }
    }

    /// The label shown while a tool runs: a plain phrase for the tools we know, the tool's own name otherwise.
    /// A tool from Roblox Studio's connector is noted with its time, for a studio run's undo log (`run_log`).
    fn show_tool(&self, name: &str, studio: bool) {
        self.st.lock().unwrap().turn_tools += 1; // the no-progress guard counts every tool call, shown or not
        let known = FRIENDLY.iter().filter(|(k, _)| name == k.as_str() || name.ends_with(&format!("_{k}"))).max_by_key(|(k, _)| k.len());
        let label = match known {
            Some((_, v)) => v.as_str().map(str::to_string),
            None => Some(name.replace('_', " ")),
        };
        // a Studio step is always noted, even one whose label the window hides
        let label = label.filter(|l| !l.is_empty()).or_else(|| studio.then(|| name.replace('_', " ")));
        if let Some(label) = label {
            self.put("tool", if studio { json!({"label": label, "studio": true, "t": paths::now_secs()}) } else { json!({"label": label}) });
        }
    }

    fn read(self: &Arc<Self>, mut child: std::process::Child, kind: &str, tp: Arc<TurnProc>) {
        // the error output is always read to its end (a full pipe would block the AI), and only its tail is kept
        let stderr = child.stderr.take();
        let err_tail = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let err_reader = {
            let kept = err_tail.clone();
            thread::spawn(move || {
                if let Some(e) = stderr {
                    paths::drain_tail_into(e, paths::TAIL, &kept);
                }
            })
        };
        let last = Arc::new(Mutex::new((std::time::Instant::now(), false, false)));
        {
            let (me, tp, last) = (self.clone(), tp.clone(), last.clone());
            thread::spawn(move || watch_stall(me, tp, last));
        }
        let pid = child.id();
        // noted again at the first line, as it is now: a launcher script (cursor-agent's) has replaced itself with the
        // real program by then, so the command line the ledger matches on is that one
        let t = match child.stdout.take() {
            Some(out) => self.consume(out, kind, &last, || crate::procs::record(kind, pid)),
            None => TurnEnd::default(),
        };
        self.finish_turn(child, kind, tp, t, err_reader, err_tail);
    }

    /// A command-line AI's output, line by line, through its parser and into the chat (the real transport is the
    /// child's stdout; the replay tests hand it recorded output instead).
    fn consume(self: &Arc<Self>, out: impl std::io::Read, kind: &str, last: &Mutex<(std::time::Instant, bool, bool)>, on_first: impl FnMut()) -> TurnEnd {
        self.consume_lines(&mut paths::CappedLines::new(BufReader::new(out), paths::LINE_CAP), kind, last, on_first, None)
    }

    /// The same from lines already split. With `live` the lines are a kept Claude Code process's (`claude_live`): the
    /// turn ends at its `result` line instead of at the end of the output, and what an earlier turn left behind is
    /// passed over. Claude Code marks a turn with the id its message was sent with (`command_lifecycle` lines, the
    /// `init` line's `user_message_uuids`, the message itself played back): until one of those comes, an answer or a
    /// good result read from a process that has run a turn before belongs to that earlier turn. A failed result ends
    /// the turn either way, so a process that fails before it marks anything never leaves the chat waiting.
    fn consume_lines(self: &Arc<Self>, lines: &mut dyn Iterator<Item = claude_live::Line>, kind: &str, last: &Mutex<(std::time::Instant, bool, bool)>, mut on_first: impl FnMut(), live: Option<LiveTurn>) -> TurnEnd {
        let mut t = TurnEnd::default();
        let mut mine = live.as_ref().is_none_or(|l| l.first || !l.marks.load(Ordering::SeqCst));
        t.ended = live.is_some();
        let (mut gemini, mut chatgpt, mut cursor_turn) = (antigravity::Turn::default(), codex::Turn::default(), cursor::Turn::default());
        let mut first = true;
        // the conversation this turn asked to go on with: an AI that reports another one started afresh
        let asked = self.st.lock().unwrap().session.clone();
        {
            let mut cut_said = false;
            let mut claude_open: std::collections::HashSet<String> = Default::default();
            for raw in lines {
                if std::mem::take(&mut first) {
                    on_first();
                }
                let Some(raw) = raw else {
                    if !cut_said {
                        self.put("error", json!({"text": "One step's output was too large to show (over 64 MB), so it was left out."}));
                        cut_said = true;
                    }
                    continue;
                };
                let Ok(e) = serde_json::from_slice::<Value>(&raw) else { continue };
                if kind != "claude" {
                    let outs = match kind {
                        "gemini" => gemini.feed(&e),
                        "cursor" => cursor_turn.feed(&e),
                        _ => chatgpt.feed(&e),
                    };
                    *last.lock().unwrap_or_else(|e| e.into_inner()) = (std::time::Instant::now(), starts_long_tool(&outs), outs.iter().any(|o| matches!(o, Out::Tool(_) | Out::ToolArgs(..))));
                    for out in outs {
                        self.take(out, &mut t);
                    }
                    continue;
                }
                if let Some(l) = &live {
                    let kind = e["type"].as_str().unwrap_or("");
                    if kind == "command_lifecycle" {
                        l.marks.store(true, Ordering::SeqCst);
                    }
                    // Claude Code has read the message: any line with its id (a `queued` mark is only that: the
                    // message waits behind whatever the process is doing, which is not this turn)
                    let named = e["command_uuid"] == l.message
                        || (e["isReplay"] == true && e["uuid"] == l.message)
                        || e["user_message_uuids"].as_array().is_some_and(|ids| ids.iter().any(|id| id == l.message));
                    if named {
                        l.tp.heard.store(true, Ordering::SeqCst);
                    }
                    // it has started on it: the `started` mark, the turn's `init` line, the message played back
                    if named && !(kind == "command_lifecycle" && e["state"] != "started") {
                        mine = true;
                        l.tp.taken();
                    } else if mine && kind == "system" && e["subtype"] == "init" {
                        l.tp.taken(); // a Claude Code that marks nothing: its `init` line is the turn's start
                    }
                    // the answer to Stop's interrupt: a message "still queued" was not stopped (`stop_live`)
                    if kind == "control_response" && e["response"]["response"]["still_queued"].as_array().is_some_and(|q| !q.is_empty()) {
                        l.tp.requeued.store(true, Ordering::SeqCst);
                    }
                    // the protocol's own lines, and the message played back, are not the conversation
                    if matches!(kind, "command_lifecycle" | "control_response" | "control_request") || e["isReplay"] == true {
                        *last.lock().unwrap_or_else(|e| e.into_inner()) = (std::time::Instant::now(), false, !claude_open.is_empty());
                        continue;
                    }
                    // left over from the turn before: its answer, its tools, its good result
                    if !mine && (matches!(kind, "assistant" | "user") || (kind == "result" && result_ok(&e))) {
                        continue;
                    }
                    // tasks of the turn's own left running in the background
                    if kind == "system" && e["subtype"] == "background_tasks_changed" {
                        t.background_left = e["tasks"].as_array().is_some_and(|tasks| !tasks.is_empty());
                    } else if kind == "system" && e["subtype"] == "task_started" && e["is_backgrounded"] == true {
                        t.background_left = true;
                    }
                }
                claude_open_tools(&e, &mut claude_open);
                if let Some(l) = &live {
                    l.tp.tool_open.store(!claude_open.is_empty(), Ordering::SeqCst);
                }
                *last.lock().unwrap_or_else(|e| e.into_inner()) = (std::time::Instant::now(), claude_starts_long_tool(&e), !claude_open.is_empty());
                if let Some(s) = e["session_id"].as_str().filter(|s| !s.is_empty()) {
                    got_session(&mut self.st.lock().unwrap(), s.to_string());
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
                        t.began = true;
                        for b in e["message"]["content"].as_array().into_iter().flatten() {
                            match b["type"].as_str().unwrap_or("") {
                                "text" => {
                                    if let Some(t) = b["text"].as_str().filter(|t| !t.trim().is_empty()) {
                                        self.put("text", json!({"text": t}));
                                    }
                                }
                                "tool_use" => {
                                    let full = b["name"].as_str().unwrap_or("");
                                    let short = full.rsplit("__").next().unwrap_or("").to_string();
                                    if let Some(id) = b["id"].as_str() {
                                        self.st.lock().unwrap().tools.insert(id.to_string(), short.clone());
                                    }
                                    if progress_call(full, &b["input"]) {
                                        self.st.lock().unwrap().turn_progress = true;
                                    }
                                    self.show_tool(&short, studio_tool(full));
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
                    "rate_limit_event" => crate::limits::record(&e),
                    "result" => {
                        t.got_result = true;
                        self.used_in(&usage::Turn::from_claude_session(&e), e["session_id"].as_str());
                        // commands the agent was not allowed to run: the page offers to allow each program
                        let allowed = paths::allowed_commands();
                        let (blocked, note) = blocked_or_note(claude_blocked(&e["permission_denials"], &allowed), &allowed);
                        if !blocked.is_empty() {
                            self.put("blocked", json!({"commands": blocked}));
                        }
                        if let Some(n) = note {
                            self.put("error", json!({"text": n}));
                        }
                        if !result_ok(&e) {
                            t.result_error = true;
                            // its own words when it has some; a bare error kind ("error_during_execution") waits for
                            // after_turn, which also has what it wrote to its error output
                            match e["result"].as_str().filter(|s| !s.trim().is_empty()) {
                                Some(msg) => {
                                    t.result_text = msg.to_string();
                                    t.result_said = true;
                                    self.put("error", json!({"text": msg}));
                                }
                                None => t.result_text = e["subtype"].as_str().unwrap_or("error").to_string(),
                            }
                        }
                        // a kept process: the turn is over, and the process waits for the next message
                        if live.is_some() {
                            t.ended = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let now = self.st.lock().unwrap().session.clone();
        // only where another id is known to mean a new conversation (Antigravity: measured, it starts one silently
        // for an id it does not have); a Claude Code that gave a fork a new id would otherwise be told off wrongly
        if let (Some(a), Some(n)) = (&asked, &now) {
            if a != n && kind == "gemini" {
                self.put("error", json!({"text": format!("{} could not find this chat's earlier conversation, so it started a new one and does not remember the chat so far. Tell it again what matters.", ai_label(kind))}));
            }
        }
        t
    }

    /// The end of a command-line AI's turn, once its output has ended: its error output, Cursor's helpers, the leader
    /// reaped and forgotten, then `after_turn`.
    fn finish_turn(self: &Arc<Self>, mut child: std::process::Child, kind: &str, tp: Arc<TurnProc>, t: TurnEnd, err_reader: thread::JoinHandle<()>, err_tail: Arc<Mutex<std::collections::VecDeque<u8>>>) {
        {
            // under the gate the stall guard decides under, so a turn that ends now is never called stalled
            let _gate = tp.reaped.lock().unwrap_or_else(|e| e.into_inner());
            tp.done.store(true, Ordering::SeqCst);
        }
        // whatever is left in the turn's own process group once its output has ended (Cursor's "worker-server" with a
        // language server, a build tool, a script that ignores SIGTERM) is ended now, for every AI: SIGTERM, then
        // SIGKILL after STOP_GRACE, before the leader is reaped (so the group's ID cannot have been reused), without
        // waiting for it. The leader may still be saving the chat after its output closed: it gets a few seconds to
        // exit by itself first.
        paths::leader_exited(child.id(), LEADER_GRACE);
        paths::end_group_soon(child.id());
        // the error output, waited for only so long: a process outside the group that still holds it never keeps the
        // turn busy (its reader is left to finish by itself)
        let until = std::time::Instant::now() + paths::STOP_GRACE + Duration::from_secs(2);
        while !err_reader.is_finished() && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        // what was read so far is kept even when the reader still waits (a holder outside the group)
        let err = paths::tail_text(&err_tail);
        tp.reaping();
        let status = child.wait().ok();
        crate::procs::forget(child.id());
        self.check_settings();
        let stopped = self.turn_over();
        if tp.killed.load(Ordering::SeqCst) {
            self.put("error", json!({"text": paths::killed_message(ai_label(kind))}));
        }
        let err = if tp.stalled.load(Ordering::SeqCst) { STALLED.to_string() } else { err };
        self.after_turn(kind, t, &err, status.map(|s| (s.success(), s.code().unwrap_or(-1))), stopped);
    }

    /// Puts back any guarded setting changed behind the app's back (`paths::check_settings`), saying so in this chat.
    fn check_settings(&self) {
        let back = paths::check_settings();
        if !back.is_empty() {
            self.put("error", json!({"text": paths::put_back_line(&back)}));
        }
    }

    /// The chat's turn has ended: it is no longer running. Returns whether Stop ended it.
    fn turn_over(&self) -> bool {
        let mut st = self.st.lock().unwrap();
        st.running = false;
        st.pid = None;
        st.proc_ = None;
        st.turned = true;
        std::mem::take(&mut st.stopped)
    }

    /// One thing an AI other than Claude Code reported during its turn.
    fn take(self: &Arc<Self>, out: Out, t: &mut TurnEnd) {
        match out {
            Out::Conversation(id) => got_session(&mut self.st.lock().unwrap(), id),
            Out::Model(name) => self.put("model", json!({"name": name})),
            Out::Tool(name) => {
                // these AIs do not say what a tool was asked, so any rbxkit, Studio or file tool counts
                if progress_call(&name, &Value::Null) {
                    self.st.lock().unwrap().turn_progress = true;
                }
                // Cursor and OpenCode name a server's tool as mcp__<server>__<tool>, as Claude Code does
                let short = name.rsplit("__").next().unwrap_or(&name).to_string();
                self.show_tool(&short, studio_tool(&name))
            }
            Out::ToolArgs(name, input) => {
                if progress_call(&name, &input) {
                    self.st.lock().unwrap().turn_progress = true;
                }
                let short = name.rsplit("__").next().unwrap_or(&name).to_string();
                self.show_tool(&short, studio_tool(&name))
            }
            Out::Text(text) => self.put("text", json!({"text": text})),
            Out::Pictures(paths) => {
                let urls: Vec<String> = paths.iter().filter_map(|p| fs::read(p).ok()).filter_map(|bytes| attach::save_chat_image(&bytes)).collect();
                if !urls.is_empty() {
                    self.put("images", json!({"urls": urls}));
                }
            }
            Out::Encoded(list) => {
                let urls: Vec<String> = list.iter().filter_map(|d| attach::save_chat_image_b64(d)).collect();
                if !urls.is_empty() {
                    self.put("images", json!({"urls": urls}));
                }
            }
            Out::Blocked(commands) => {
                let (card, note) = blocked_or_note(commands, &paths::allowed_commands());
                if !card.is_empty() {
                    self.put("blocked", json!({"commands": card}));
                }
                if let Some(n) = note {
                    self.put("error", json!({"text": n}));
                }
            }
            Out::Refused(commands) => self.put("refused", json!({"commands": commands})),
            Out::Error(text) => self.put("error", json!({"text": text})),
            Out::Failed(text) => {
                t.result_error = true;
                t.result_said = true;
                t.result_text = text.clone();
                self.put("error", json!({"text": text}));
            }
            Out::Usage(turn) => self.used(&turn),
            Out::Finished => t.got_result = true,
        }
    }

    /// An OpenCode turn, on the app's OpenCode server (see `opencode::run`), then the same ending as any turn.
    fn read_opencode(self: &Arc<Self>, spec: opencode::Spec) {
        let mut t = TurnEnd::default();
        let ended = opencode::run(&spec, |out| self.take(out, &mut t));
        self.check_settings();
        let stopped = self.turn_over();
        if ended.killed {
            // after Stop: it did not stop in time; otherwise the turn already said why OpenCode was ended
            let text = if stopped {
                "OpenCode did not stop when asked within a few seconds, so Fridae ended it (and any other OpenCode chat that was working)."
            } else {
                "Any other OpenCode chat that was working was ended with it."
            };
            self.put("error", json!({"text": text}));
        }
        let exit = (!ended.err.is_empty()).then_some((false, -1));
        self.after_turn("opencode", t, &ended.err, exit, stopped);
    }

    /// What follows every turn: a message for how it ended, the board, and whether the chat carries on by itself.
    /// `exit` is how the AI's program ended (success, code), when there was one.
    fn after_turn(self: &Arc<Self>, kind: &str, t: TurnEnd, err: &str, exit: Option<(bool, i32)>, stopped: bool) {
        let TurnEnd { got_result, result_error, result_text, result_said, .. } = t;
        self.clear_nudge(); // what waited is sent now (or dropped), so the next wait does not return for it
        if err == STALLED && !stopped {
            self.put("error", json!({"text": paths::stall_message()}));
        } else if (!got_result || result_error) && err.contains("No conversation found") {
            self.st.lock().unwrap().session = None;
            self.put("error", json!({"text": "The memory of this earlier chat is no longer on this computer, so the agent has lost its context. Send your message again and it will carry on from what you tell it."}));
        } else if stopped {
            self.put("error", json!({"text": "Stopped."}));
        } else if let Some(why) = match kind {
            "gemini" => antigravity::explain(err, got_result),
            "chatgpt" => codex::explain(err).filter(|_| !got_result).map(str::to_string),
            "cursor" => cursor::explain(err, got_result),
            "opencode" => opencode::explain(err, got_result),
            _ => None,
        } {
            self.put("error", json!({"text": why}));
        } else if (!got_result && !exit.map(|(ok, _)| ok).unwrap_or(true)) || (result_error && !result_said) {
            let tail: String = err.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
            let code = exit.map(|(_, c)| c).unwrap_or(-1);
            let plain = if result_error { format!("The agent stopped with an error ({result_text}).") } else { format!("The agent exited unexpectedly (code {code}).") };
            self.put("error", json!({"text": if tail.is_empty() { plain } else { tail }}));
        }
        // signed out while a key the app keeps from the AI is set on this computer: say which switch lets it through
        if !stopped && (!got_result || result_error) {
            if let Some(note) = key_note(kind, &format!("{err}\n{result_text}"), &paths::env_keys_present(kind), paths::env_keys_on()) {
                self.put("error", json!({"text": note}));
            }
        }
        team::finished(self, stopped);
        research::finished(self);
        self.put("done", json!({}));
        // autopilot: the agent ends each reply by saying whether there is more it can do alone; if so, it is told to carry on
        let reply = self.last_reply();
        let last_line = reply.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        let mut said = last_line.to_uppercase();
        let mut held = std::mem::take(&mut self.st.lock().unwrap().queue);
        // a paused studio run: what the user sent waits for Resume, and the Lead winds down (see pause_step)
        let paused = self.studio_lead() && self.paused_run();
        // a run ended in Review: nothing carries on (see ended_step)
        let ended = self.studio_lead() && self.project().is_some_and(|p| studio_undo::lead_of(&p, &self.id).ended);
        // a Lead whose experts are still working is waiting for them, not going round in circles
        let experts_working = !working_helpers_of(&self.id).is_empty();
        {
            // the no-progress guard: three carry-ons in a row without a tool call count as STATUS: WAITING
            let mut st = self.st.lock().unwrap();
            let lead = guarded(st.last_ask.studio && st.role.is_none(), experts_working);
            let (carry_on, tools) = (st.turn_auto, st.turn_tools);
            let ok = got_result && !result_error;
            let progress = st.turn_progress;
            // both counts go on every turn, so neither guard hides the other
            let idle = no_progress(lead, carry_on, ok, tools, &mut st.idle_turns);
            let stuck = no_progress_call(lead, carry_on, ok, progress, &mut st.stuck_turns);
            if idle || stuck {
                said = "STATUS: WAITING".to_string();
            }
        }
        if paused && !held.is_empty() {
            self.st.lock().unwrap().queue = std::mem::take(&mut held);
            self.show_queue();
        }
        let mut night_over: Option<&str> = None;
        let mut waiting: Option<f64> = None;
        let carry_on: Option<(Ask, &str)> = {
            let mut st = self.st.lock().unwrap();
            let next = Ask { kickoff: Some("auto".to_string()), ..st.last_ask.clone() };
            let now = paths::now_secs();
            let night = st.overnight_until > now;
            let mut ls = LoopState { autopilot_left: st.autopilot_left, fails: st.overnight_fails, done: st.overnight_done, night };
            // a turn whose result is an error counts as failed, so a signed-out AI ends the loop after three turns
            let step = next_step(loop_mode(stopped, !held.is_empty(), night, st.last_ask.studio), got_result && !result_error, &said, &mut ls);
            let step = pause_step(step, paused, &mut st.pause_told, &mut ls);
            let step = ended_step(step, ended, &mut ls);
            // out of usage: the next turn would only fail the same way, so the run stops here, paused, until Resume
            let step = if night && st.last_ask.studio && !stopped && !ended && result_error && usage_limit(&result_text) {
                ls.night = false;
                Step::End(LIMIT_END)
            } else {
                step
            };
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
            // a run that stopped on failures is paused, with the reason, so Review says why and offers Resume
            if why == LIMIT_END {
                studio_undo::pause_with_note(self, &format!("limit: {}", result_text.chars().take(200).collect::<String>()));
            } else if why == FAILED_END {
                studio_undo::pause_with_note(self, "failed");
            }
        }
        // the board follows the chat: still working, waiting on the user, or finished
        if let Some(p) = self.project() {
            let first = board::gist(reply.trim_start(), 200);
            let (column, note) = if !held.is_empty() || carry_on.is_some() {
                ("doing", None)
            } else if stopped {
                ("todo", Some("Stopped before it finished.".to_string()))
            } else if waiting.is_some() && paused {
                ("needs", Some("The studio run is paused. Resume it in Review".to_string()))
            } else if waiting.is_some() {
                ("needs", Some("Waiting for your answer in Review".to_string()))
            } else if said.starts_with("STATUS: NEEDS YOU") {
                ("needs", Some(last_line.splitn(2, ':').nth(1).and_then(|r| r.splitn(2, ':').nth(1)).unwrap_or("").trim().to_string()))
            } else if !got_result || result_error {
                ("needs", Some(format!("It stopped with an error: {first}")))
            } else {
                ("done", Some(first))
            };
            board::track(&p, &self.id, &self.name(), column, None, note.as_deref());
        }
        if !held.is_empty() {
            // what the user sent while it worked goes next, in one message, with the newest settings
            self.show_queue();
            let typed = merged_typed(&held.iter().map(|(_, t, a)| (t.as_str(), a)).collect::<Vec<_>>());
            let text = held.iter().map(|(_, t, _)| t.as_str()).collect::<Vec<_>>().join("\n\n");
            let newest = held.last().unwrap().2.clone();
            let ask = Ask {
                kickoff: None,
                attachments: held.iter().flat_map(|(_, _, a)| a.attachments.clone()).collect(),
                studio: merged_studio(&held, newest.studio),
                wake: held.iter().all(|(_, _, a)| a.wake), // one typed message among them makes it the user's
                app: held.iter().all(|(_, _, a)| a.app || a.wake),
                typed,
                ..newest
            };
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

    /// Stop this chat's turn: the agent's process and everything it started end (see `paths::end_trees`).
    pub fn stop(&self) {
        // OpenCode: the server is asked to end the turn, and ended itself if it does not (see `opencode::stop`)
        if self.ai() == "opencode" {
            let saved = self.st.lock().unwrap().session.clone();
            let _ = self.stop_turn();
            let id = self.id.clone();
            thread::spawn(move || opencode::stop(&id, saved));
            return;
        }
        if let Some(tp) = self.stop_turn() {
            // Windows: taskkill, there and then, as always. Unix: asked to end, killed after STOP_WAIT, off this thread
            #[cfg(windows)]
            tp.stop();
            #[cfg(not(windows))]
            thread::spawn(move || tp.stop());
        }
    }

    /// Everything `stop` does but ending the process, which it returns (None when no turn is running).
    fn stop_turn(&self) -> Option<Arc<TurnProc>> {
        team::release(self); // a stopped expert no longer holds the game
        // the app's own queued wakes go with the turn; what the user typed stays queued
        if drop_wakes(&mut self.st.lock().unwrap().queue, false) {
            self.show_queue();
        }
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
                return None;
            }
            st.stopped = true;
            (st.proc_.clone().filter(|_| st.running), st.making)
        };
        if making {
            textures::skip_for(&self.id); // the waiting texture tool then ends by itself
        }
        pid
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
        if slots_taken(&all) >= MAX_AGENTS {
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
/// request), or the New project page's researcher.
pub fn spawn_helper(team_id: Option<&str>, role: &str, name: &str, project: Option<String>, ai: Option<String>, parent: Option<&str>) -> Option<Arc<Agent>> {
    spawn_named(team_id, Some(role), name, project, ai, None, parent, None)
}

/// A helper or expert started by a chat's `rbx_team` spawn, on the AI and model `expert_ai::resolve` chose.
#[allow(clippy::too_many_arguments)]
pub fn spawn_on(team_id: &str, role: &str, title: &str, project: Option<String>, ai: String, model: Option<String>, parent: Option<&str>, expert: Option<Expert>) -> Option<Arc<Agent>> {
    spawn_named(Some(team_id), Some(role), title, project, Some(ai), model, parent, expert)
}

/// A named Studio expert: a helper whose chat also gets the expert's own instructions (`id`, display `name`, `text`).
pub struct Expert {
    pub id: String,
    pub name: String,
    pub text: String,
}

#[cfg(test)]
pub fn spawn_expert(team_id: &str, role: &str, title: &str, project: Option<String>, ai: Option<String>, parent: Option<&str>, expert: Expert) -> Option<Arc<Agent>> {
    spawn_named(Some(team_id), Some(role), title, project, ai, None, parent, Some(expert))
}

/// Start a chat of the user's own (no role) in the background, already named and in its project: the Promos tab's.
pub fn new_background_chat(name: &str, project: Option<String>) -> Option<Arc<Agent>> {
    spawn_named(None, None, name, project, None, None, None, None)
}

#[allow(clippy::too_many_arguments)]
fn spawn_named(team_id: Option<&str>, role: Option<&str>, name: &str, project: Option<String>, ai: Option<String>, model: Option<String>, parent: Option<&str>, expert: Option<Expert>) -> Option<Arc<Agent>> {
    // before an expert or helper starts: a setting changed behind the app's back is put back, and the chat that
    // started it is told
    let back = paths::check_settings();
    if !back.is_empty() {
        if let Some(p) = parent.and_then(get) {
            p.put("error", json!({"text": paths::put_back_line(&back)}));
        }
    }
    let agent = {
        let mut all = AGENTS.lock().unwrap();
        if slots_taken(&all) >= MAX_AGENTS {
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
            st.model = model;
            st.project = project;
            st.parent = parent.map(str::to_string);
        }
        all.push(agent.clone());
        agent
    };
    // what it runs on, for its line, when that is not what the chat that started it runs on
    let lead = parent.and_then(get);
    let runs_on = lead.as_ref().and_then(|l| crate::expert_ai::runs_on(&agent.ai(), agent.model().as_deref(), &l.ai(), l.model().as_deref()));
    let mut event = json!({"agent": agent.id, "name": agent.name(), "role": role, "background": true, "ai": agent.ai(), "parent": parent, "team": team_id, "runs_on": runs_on});
    if let Some(x) = &expert {
        event["expert"] = json!(x.name);
        event["expert_id"] = json!(x.id);
    }
    BUS.put("agent_new", event);
    // the chat that started it notes where, so the window shows the expert at that point of its conversation
    if let Some(lead) = lead {
        lead.put("helper_started", json!({"helper": agent.id, "name": agent.name(), "expert": expert.as_ref().map(|x| x.name.clone()), "runs_on": runs_on}));
    }
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
    // experts, helpers and the researcher are not reopened as chats: their logs are read when their line is opened
    let ids: Vec<String> = agents().iter().filter(|a| a.has_messages() && !is_helper(&a.st.lock().unwrap())).map(|a| a.id.clone()).collect();
    let _ = fs::write(dir.join("_open.txt"), ids.join("\n"));
}

/// Whether a saved chat is an expert, a helper or the researcher (see `is_helper`).
fn saved_helper(d: &Value) -> bool {
    d["parent"].is_string() || d["team"].is_string() || d["role"] == "research"
}

/// Reopen the chats that were open last time.
pub fn restore_open() -> usize {
    let Ok(text) = fs::read_to_string(paths::work("chats").join("_open.txt")) else { return 0 };
    let listed: Vec<(String, bool, f64)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|id| {
            let d: Value = fs::read_to_string(chat_file(id)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
            // a list written before helpers were left out may still name some: they stay closed
            (!saved_helper(&d)).then(|| (id.to_string(), true, d["updated"].as_f64().unwrap_or(0.0)))
        })
        .collect();
    restore_order(&listed, MAX_AGENTS).iter().filter(|id| open_chat(id).is_some()).count()
}

/// Which of the chats listed as open (id, holds a slot, last updated) are reopened, in order: the slot-holding ones
/// first, then the finished helpers (they take no slot, so all of them). When more slot-holding chats are listed than
/// there are slots, the newest `slots` of them are reopened (in their listed order); the rest stay under Earlier.
fn restore_order(listed: &[(String, bool, f64)], slots: usize) -> Vec<String> {
    let mut holders: Vec<(usize, &(String, bool, f64))> = listed.iter().enumerate().filter(|(_, c)| c.1).collect();
    holders.sort_by(|a, b| b.1 .2.total_cmp(&a.1 .2));
    holders.truncate(slots);
    holders.sort_by_key(|(i, _)| *i);
    holders.iter().map(|(_, c)| c.0.clone()).chain(listed.iter().filter(|c| !c.1).map(|c| c.0.clone())).collect()
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
            // the window lists a game's earlier chats under the game, and never an expert's
            "project": d["project"], "role": d["role"], "parent": d["parent"],
            // an expert's request and name, so a review item that names only the request can still say who made it
            "team": d["team"],
            "expert": if d["expert"].is_string() { json!(d["name"].as_str().unwrap_or("").split(':').next().unwrap_or("").trim()) } else { Value::Null },
        }));
    }
    out.sort_by(|a, b| b["updated"].as_f64().partial_cmp(&a["updated"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(200);
    out
}

/// The `agent_new` event for a reopened chat: its name and AI, and for a helper its role, the chat that started it,
/// its request and its expert, so the window can fold it under that chat as it did before the restart.
fn restored_event(id: &str, name: &str, d: &Value) -> Value {
    let mut e = json!({"agent": id, "name": name, "restored": true, "ai": d["ai"].as_str().unwrap_or("claude")});
    for key in ["role", "parent", "team"] {
        if let Some(v) = d[key].as_str().filter(|v| !v.is_empty()) {
            e[key] = json!(v);
        }
    }
    if let Some(x) = d["expert"].as_str().filter(|v| !v.is_empty()) {
        e["expert_id"] = json!(x);
        // an expert's chat is titled "<expert>: <job>" (team.rs), so its name is the part before the colon
        e["expert"] = json!(name.split(':').next().unwrap_or(name).trim());
    }
    e
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
        // a helper that handed its result back stays finished after a restart, so it takes no agent slot
        let mut st = agent.st.lock().unwrap();
        st.team = d["team"].as_str().map(str::to_string);
        st.result_written = d["result_written"].as_bool().unwrap_or(false);
        st.parent = d["parent"].as_str().map(str::to_string);
        st.role = d["role"].as_str().map(str::to_string);
        st.turned = true; // a saved chat: whatever it was doing is over
    }
    {
        let mut all = AGENTS.lock().unwrap();
        if agent.holds_slot() && slots_taken(&all) >= MAX_AGENTS {
            return None;
        }
        all.push(agent.clone());
    }
    // with who started it, so the window folds an expert's chat under its Lead's
    BUS.put("agent_new", restored_event(&agent.id, &agent.name(), &d));
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
        // a reopened expert keeps its instructions (rbxroster caps them at 16,000 characters)
        st.expert_text = d["expert_text"].as_str().map(str::to_string);
        st.parent = d["parent"].as_str().map(str::to_string);
        // chats saved before there was a choice were all Claude's
        st.ai = Some(d["ai"].as_str().unwrap_or("claude").to_string());
        st.model = d["model"].as_str().map(str::to_string);
        st.handover = d["handover"].as_str().map(str::to_string);
        st.log = events.into_iter().filter(|e| e["kind"].is_string()).collect();
    }
    if !settled {
        BUS.put("done", json!({"agent": agent.id})); // it was closed mid-reply: settle the view
    }
    // an idle fresh chat that was only a placeholder makes way for the reopened one: never a chat the app started with
    // a name (a studio run's Lead or a helper that has not said anything yet: removing it here, bypassing
    // remove_agent, left its experts open; this was the race behind a flaky test)
    let placeholders: Vec<Arc<Agent>> = agents()
        .into_iter()
        .filter(|o| o.id != agent.id && !o.busy() && {
            let st = o.st.lock().unwrap();
            st.log.is_empty() && !st.named && !is_helper(&st)
        })
        .collect();
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
        agent.st.lock().unwrap().closed = true;
        agent.stop();
        // its kept Claude Code process goes with it (one with a turn still ending is closed by that turn, `read_live`)
        {
            let a = agent.clone();
            thread::spawn(move || a.close_live());
        }
        team::release(&agent);
        agent.save();
        BUS.put("agent_removed", json!({"agent": agent.id}));
        // closing a Lead (or any chat) closes the experts it started: none is left holding a slot unseen
        for helper in helpers_of(&agent.id) {
            remove_agent(&helper.id);
        }
        remember_open();
        team::slot_freed(); // a waiting studio run may have been short of a slot
    }
    if AGENTS.lock().unwrap().is_empty() {
        new_agent(None);
    }
}

/// Quitting: ends the Claude Code processes kept open between turns, with all they started. Those with a turn running
/// (`turns`) have just been ended with it.
fn end_kept(chats: &[Arc<Agent>], turns: &[Arc<TurnProc>]) {
    let kept: Vec<Arc<Live>> = chats.iter().filter_map(|a| a.st.lock().unwrap().live.take()).collect();
    let idle: Vec<thread::JoinHandle<()>> = kept.into_iter().filter(|l| !turns.iter().any(|t| t.pid == l.pid)).map(|l| thread::spawn(move || l.kill())).collect();
    for t in idle {
        let _ = t.join();
    }
}

/// Stop every agent and save every chat: called when the window closes.
pub fn shut_down() {
    set_shutting(true); // first: a turn ended below must not send its held messages as a new agent
    crate::promo::stop_install();
    // every running agent is ended, and the app waits for them (at most the stop grace), so none outlives it
    let turns: Vec<Arc<TurnProc>> = agents().iter().filter_map(|a| a.stop_turn()).collect();
    {
        // every gate is held while the groups are ended, so no leader is reaped meanwhile
        let gates: Vec<_> = turns.iter().map(|t| t.reaped.lock().unwrap_or_else(|e| e.into_inner())).collect();
        let groups: Vec<u32> = if cfg!(windows) {
            turns.iter().zip(&gates).filter(|(_, g)| !***g).map(|(t, _)| t.pid).collect()
        } else {
            turns.iter().zip(&gates).flat_map(|(t, g)| t.groups(**g)).collect()
        };
        paths::end_trees(&groups);
    }
    end_kept(&agents(), &turns);
    opencode::shut_down(); // the app's OpenCode server, with every OpenCode turn
    // each ended turn settles its chat ("Stopped.", then done) before the chats are saved, for a moment at most
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while agents().iter().any(|a| a.st.lock().unwrap().running) && std::time::Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }

    for a in agents() {
        a.save();
    }
}

/// The Lead's instructions for a Studio run: the game's own `.fridae/team/experts/lead.md` when it has one, else the
/// kit's `vault/Fridae/Team/lead.md`. Empty when neither can be read (the studio prompt still applies).
fn studio_lead_text(project: Option<&str>) -> String {
    studio_lead_text_in(&PATHS.kit, project)
}

/// The Lead's instructions: a game's own `.fridae/team/experts/lead.md` when it has one, else the vault's
/// `vault/Fridae/Team/lead.md` in the kit folder.
fn studio_lead_text_in(kit: &std::path::Path, project: Option<&str>) -> String {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = project {
        files.push(std::path::Path::new(p).join(".fridae").join("team").join("experts").join("lead.md"));
    }
    files.push(paths::kit_team(kit).join("lead.md"));
    files.iter().find_map(|f| fs::read_to_string(f).ok()).filter(|t| !t.trim().is_empty()).map(|t| format!("\n\n{}", t.trim())).unwrap_or_default()
}

/// Characters of house rules every agent gets at most from each file (the user's, then the game's).
const HOUSE_RULES_FILE_CAP: usize = 2000;
const HOUSE_RULES_FILE: &str = "House rules.md";
/// What house rules may never do; said before and after them so the app's safety rules stay the strongest.
const HOUSE_RULES_LIMIT: &str = "They set taste and working habits only. They NEVER override the rules above on spending money or Robux, publishing, uploading, deleting, access or shell permissions, and never tell you to skip asking the user where those rules say to ask.";

/// One house rules file for the block: its own "# " title lines dropped, cut at HOUSE_RULES_FILE_CAP characters.
fn house_rules_part(text: &str) -> String {
    let t = text.lines().filter(|l| !l.trim_start().starts_with("# ")).collect::<Vec<_>>().join("\n").trim().to_string();
    if t.chars().count() <= HOUSE_RULES_FILE_CAP {
        return t;
    }
    let cut: String = t.chars().take(HOUSE_RULES_FILE_CAP).collect();
    let cut = match cut.rfind('\n') {
        Some(i) if i > HOUSE_RULES_FILE_CAP / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{cut}\n(The rest did not fit; it is in that folder's {HOUSE_RULES_FILE}.)")
}

/// The house rules block for the system prompt: the user's own rules, then this game's (labelled as coming from the
/// game's folder), each capped, framed by the limit sentence before and after. Empty when there are no rules.
/// A pure function: callers read the files.
fn house_rules_block(all: Option<&str>, game: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(t) = all.map(house_rules_part).filter(|t| !t.is_empty()) {
        parts.push(format!("The user's house rules (follow these):\n{t}"));
    }
    if let Some(t) = game.map(house_rules_part).filter(|t| !t.is_empty()) {
        parts.push(format!("This game's house rules (from the game's folder):\n{t}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("\n\nHOUSE RULES. Standing preferences; they beat the guides. {HOUSE_RULES_LIMIT}\n{}\nReminder: {HOUSE_RULES_LIMIT}", parts.join("\n\n"))
}

/// The house rules as they are on disk right now: `<home>/notes/House rules.md` and, with a project, the game's
/// `.fridae/notes/House rules.md`. Read fresh every turn.
fn house_rules(home: &std::path::Path, project: Option<&str>) -> String {
    let all = fs::read_to_string(home.join("notes").join(HOUSE_RULES_FILE)).ok();
    let game = project.and_then(|p| fs::read_to_string(std::path::Path::new(p).join(".fridae").join("notes").join(HOUSE_RULES_FILE)).ok());
    house_rules_block(all.as_deref(), game.as_deref())
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
/// Whether this copy of the app runs with Studio switched off (FRIDAE_NO_STUDIO, for tests and live trials).
fn no_studio() -> bool {
    std::env::var_os("FRIDAE_NO_STUDIO").is_some_and(|v| !v.is_empty())
}

fn mcp_servers(agent_id: &str, agent_name: &str, project: Option<&str>, role: Option<&str>) -> Value {
    mcp_servers_with(agent_id, agent_name, project, role, no_studio())
}

fn mcp_servers_with(agent_id: &str, agent_name: &str, project: Option<&str>, role: Option<&str>, no_studio: bool) -> Value {
    let cmd = paths::mcp_command();
    let mut servers = json!({
        "rbxkit": {"type": "stdio", "command": cmd[0], "args": cmd[1..], "env": {
            "RBXKIT_AGENT_ID": agent_id, "RBXKIT_AGENT_NAME": agent_name,
            "RBXKIT_PROJECT": project.unwrap_or(""), "RBXKIT_AGENT_ROLE": role.unwrap_or(""),
            // the guarded settings the tools act on, from the app's memory (`paths::GUARDED`): the tools never take
            // them from the settings file while the app runs them
            "FRIDAE_SETTINGS_FROM_APP": "1", "FRIDAE_GEMINI_USE_API": if paths::gemini_api_on() { "1" } else { "0" },
            "RBXKIT_BLENDER": paths::find_blender().map(|p| p.display().to_string()).unwrap_or_default(),
        }}
    });
    // FRIDAE_NO_STUDIO (testing only): a copy of the app that cannot reach the user's open Studio place: no Studio
    // server for the agent, and rbxkit's own Studio tools are told so (they answer "Studio is off for this run")
    if no_studio {
        servers["rbxkit"]["env"]["FRIDAE_NO_STUDIO"] = json!("1");
        return servers;
    }
    if let Some(studio) = paths::studio_mcp() {
        servers["Roblox_Studio"] = if cfg!(windows) {
            json!({"type": "stdio", "command": "cmd.exe", "args": ["/c", "cd /d %LOCALAPPDATA%\\Roblox && .\\mcp.bat"]})
        } else {
            json!({"type": "stdio", "command": studio.display().to_string(), "args": ["--stdio"]})
        };
    }
    servers
}

/// The servers an agent gets, with Studio on or off, for other modules' tests.
#[cfg(test)]
pub fn test_servers(no_studio: bool) -> Value {
    mcp_servers_with("c1-test", "Test chat", Some("/games/g"), None, no_studio)
}

/// Claude Code's deny rules for the app's own files (`paths::app_protected`): `Edit(//<path>)`, with `/**` for a
/// folder (`//` is an absolute path in its rules; Edit rules cover every file-writing tool).
fn claude_denied() -> Vec<String> {
    paths::app_protected().into_iter().map(|(p, dir)| claude_edit_rule(&p.display().to_string(), dir, cfg!(windows))).collect()
}

/// Claude Code's deny rule for an absolute path, by its documented syntax (code.claude.com/docs/en/permissions, "Read
/// and Edit"): gitignore patterns where `//` starts an absolute path; on Windows paths are matched in POSIX form, so
/// `C:\Users\a` is `//c/Users/a`. A folder gets `/**`. (Matching ignores case: measured, Claude Code in bypass mode
/// refused `CONFIG.JSON` under a rule for `config.json`, in a folder with a space.)
fn claude_edit_rule(path: &str, dir: bool, windows: bool) -> String {
    let posix = if windows {
        let p = path.replace('\\', "/");
        let b = p.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() { format!("/{}{}", (b[0] as char).to_ascii_lowercase(), &p[2..]) } else { p }
    } else {
        path.to_string()
    };
    format!("Edit(/{posix}{})", if dir { "/**" } else { "" })
}

fn write_mcp_config(agent_id: &str, servers: &Value) -> std::path::PathBuf {
    // in a folder of its own, which no agent's file tools may write (paths::app_protected)
    let dir = PATHS.work.join("agent-tools");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{agent_id}.json"));
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
        "{head}\n\nThose apps can now use the building tools from their own chat. The chats in this window run Claude Code, Gemini \
         (Antigravity) or Cursor themselves and need none of this.\nIn Antigravity, reload the servers after this: type /mcp in the CLI, or open MCP \
         Servers from the agent panel's menu. Also connect Roblox Studio there (Studio's Assistant settings list how).\n\n\
         For any other agent that supports MCP servers, add:\n{}",
        serde_json::to_string_pretty(&json!({"mcpServers": {"rbxkit": entry}})).unwrap_or_default()
    )
}

/// Recorded output of each command-line AI replayed through `consume` and `after_turn` (only the transport is faked).
#[cfg(test)]
#[path = "replay_tests.rs"]
mod replay_tests;

/// The kept Claude Code process: its recorded output replayed, and turns run against a stand-in process.
#[cfg(test)]
#[path = "live_tests.rs"]
mod live_tests;

#[cfg(test)]
#[path = "acp_tests.rs"]
mod acp_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_text_comes_from_the_vault_or_the_games_override() {
        let kit = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let vault = fs::read_to_string(kit.join("vault").join("Fridae").join("Team").join("lead.md")).unwrap();
        assert_eq!(studio_lead_text_in(kit, None), format!("\n\n{}", vault.trim()));
        let tmp = std::env::temp_dir().join(format!("fridae lead é {}", std::process::id()));
        let experts = tmp.join(".fridae").join("team").join("experts");
        fs::create_dir_all(&experts).unwrap();
        let project = tmp.to_str().unwrap();
        assert_eq!(studio_lead_text_in(kit, Some(project)), format!("\n\n{}", vault.trim()), "no override yet");
        fs::write(experts.join("lead.md"), "  This game's Lead: zebra rules.\n").unwrap();
        assert_eq!(studio_lead_text_in(kit, Some(project)), "\n\nThis game's Lead: zebra rules.");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handover_is_the_users_and_agents_words_only() {
        let log = vec![
            json!({"kind": "user", "text": "Make a café sign"}),
            json!({"kind": "tool", "label": "Modelling in Blender"}),
            json!({"kind": "images", "urls": ["/img/a.png"]}),
            json!({"kind": "text", "text": "Done: the sign is by the spawn.\nSTATUS: DONE"}),
            json!({"kind": "user", "text": "Carry on.", "auto": true}),
            json!({"kind": "error", "text": "Stopped."}),
            json!({"kind": "user", "text": "  "}),
            json!({"kind": "user", "text": "Make it bigger"}),
        ];
        let h = handover_text(&log, "Claude", HANDOVER_CAP);
        assert!(h.starts_with("[This conversation began with Claude;"));
        assert!(h.contains("User: Make a café sign\n\nClaude: Done: the sign is by the spawn.\nSTATUS: DONE\n\nUser: Make it bigger"));
        assert!(!h.contains("Blender") && !h.contains("Carry on") && !h.contains("Stopped") && !h.contains("/img/"));
        assert!(!h.contains("left out"), "nothing was cut");
        assert!(h.ends_with("follows.]\n\n"), "the user's new message goes right after it");
        assert!(handover_text(&[], "Gemini", 100).contains("(nothing was said yet)"));
    }

    #[test]
    fn handover_keeps_the_newest_messages_under_the_cap() {
        let log: Vec<Value> = (0..50).map(|n| json!({"kind": if n % 2 == 0 { "user" } else { "text" }, "text": format!("message {n} {}", "x".repeat(80))})).collect();
        let h = handover_text(&log, "Claude", 1_000);
        assert!(h.contains("message 49") && !h.contains("message 0 "), "the newest stay, the oldest go");
        assert!(h.contains("the oldest messages are left out"));
        let body = h.split("\n\n").filter(|l| l.starts_with("User:") || l.starts_with("Claude:")).map(|l| l.chars().count() + 2).sum::<usize>();
        assert!(body <= 1_000, "kept {body} characters");
        // one message longer than the cap keeps its end, where it finished
        let long = vec![json!({"kind": "user", "text": format!("start {} the end", "y".repeat(5_000))})];
        let h = handover_text(&long, "Claude", 300);
        assert!(h.contains("the end") && !h.contains("start "));
        assert!(h.contains("User: …y"), "the kept end still says who said it");
    }

    #[test]
    fn handover_leaves_out_the_apps_messages_and_names_each_ai() {
        let log = vec![
            json!({"kind": "ai", "ai": "claude"}),
            json!({"kind": "user", "text": "Build the shop"}),
            json!({"kind": "text", "text": "The shop is in."}),
            json!({"kind": "user", "text": "Review: Shop — approved", "app": true}),
            json!({"kind": "user", "text": "You may now run git. Run what was refused and carry on.", "app": true}),
            json!({"kind": "ai", "ai": "gemini", "from": "claude"}),
            json!({"kind": "user", "text": "Make it blue"}),
            json!({"kind": "text", "text": "It is blue now."}),
        ];
        let h = handover_text(&log, "Gemini", HANDOVER_CAP);
        assert!(h.contains("User: Build the shop\n\nClaude: The shop is in.\n\nUser: Make it blue\n\nGemini: It is blue now."), "{h}");
        assert!(!h.contains("approved") && !h.contains("may now run"), "the app's own messages are not the user's");
    }

    #[test]
    fn a_mixed_turn_hands_over_only_the_typed_part_and_review_notes() {
        let typed = Ask::default();
        let wake = Ask { wake: true, ..Default::default() };
        assert_eq!(merged_typed(&[("Review: Shop \u{2014} approved", &wake), ("Make it blue", &typed)]).as_deref(), Some("Make it blue"));
        assert_eq!(merged_typed(&[("a", &typed), ("b", &typed)]), None, "all typed: the whole text is the user's");
        assert_eq!(merged_typed(&[("a", &wake)]), None);
        let log = vec![
            json!({"kind": "user", "text": "Review: Shop \u{2014} approved\n\nMake it blue", "typed": "Make it blue"}),
            json!({"kind": "user", "text": "Review: Map \u{2014} change: more trees near spawn", "app": true}),
            json!({"kind": "user", "text": "Resume the studio run.", "app": true}),
        ];
        let h = handover_text(&log, "Claude", HANDOVER_CAP);
        assert!(h.contains("User: Make it blue") && !h.contains("approved") && !h.contains("Resume"), "{h}");
        assert!(h.contains("User: (asked for changes in Review, on \"Map\") more trees near spawn"), "{h}");
    }

    #[test]
    fn a_cursor_chat_it_no_longer_has_starts_again_with_the_chat_so_far() {
        let log = vec![
            json!({"kind": "user", "text": "Make a lava floor"}),
            json!({"kind": "text", "text": "Done: lava floor at the start."}),
            json!({"kind": "user", "text": "Now make it glow"}),
        ];
        let mut st = State { session: Some("abc".into()), log: log.clone(), ..Default::default() };
        stale_chat(&mut st, "Cursor", |_| true);
        assert_eq!(st.session.as_deref(), Some("abc"), "a chat Cursor still has is resumed");
        assert!(st.handover.is_none());
        stale_chat(&mut st, "Cursor", |_| false);
        assert!(st.session.is_none());
        let h = st.handover.clone().unwrap();
        assert!(h.contains("Make a lava floor") && h.contains("lava floor at the start"));
        assert!(!h.contains("Now make it glow"), "the message being sent is not handed over twice");
    }

    #[test]
    fn a_handover_stays_until_the_new_ai_has_a_session() {
        let mut st = State { handover: Some("[chat so far]\n\n".into()), ..Default::default() };
        assert_eq!(with_handover(&st, "Hi"), "[chat so far]\n\nHi");
        // the first turn failed or was stopped before a session: the next message carries it again
        assert_eq!(with_handover(&st, "Again"), "[chat so far]\n\nAgain");
        got_session(&mut st, "s-1".into());
        assert_eq!(st.session.as_deref(), Some("s-1"));
        assert_eq!(with_handover(&st, "Next"), "Next", "once the AI has the chat, it is not sent twice");
    }

    #[test]
    fn a_helper_that_is_not_working_holds_no_slot() {
        let mut st = State { turned: true, ..Default::default() };
        assert!(holds_slot(&st), "the user's own chat holds one, working or not");
        st.parent = Some("lead".into());
        assert!(!holds_slot(&st), "an expert stopped or failed, with no result written");
        st.running = true;
        assert!(holds_slot(&st), "while it works");
        let st = State { role: Some("research".into()), turned: true, ..Default::default() };
        assert!(!holds_slot(&st), "a researcher that is done");
        let fresh = State { parent: Some("lead".into()), ..Default::default() };
        assert!(holds_slot(&fresh), "one just started holds its slot until its first turn is under way");
    }

    #[test]
    fn helpers_are_not_reopened_as_chats() {
        assert!(saved_helper(&json!({"parent": "c1", "team": "w1"})));
        assert!(saved_helper(&json!({"role": "research"})));
        assert!(saved_helper(&json!({"team": "w1"})));
        assert!(!saved_helper(&json!({"role": null, "parent": null, "name": "My chat"})));
    }

    #[test]
    fn closing_a_lead_closes_its_experts() {
        let lead = new_background_chat("Lead: close test", None).unwrap();
        let x = Expert { id: "coder".into(), name: "Coder".into(), text: "Write code.".into() };
        let e = spawn_expert("w-close-test", "worker", "Coder: close test", None, None, Some(&lead.id), x).unwrap();
        assert_eq!(helpers_of(&lead.id).len(), 1);
        remove_agent(&lead.id);
        assert!(get(&e.id).is_none(), "the expert went with its Lead");
    }

    #[test]
    fn reopening_a_chat_never_drops_a_lead_that_has_not_spoken_yet() {
        // the race behind the flaky test above: a chat reopened elsewhere took a silent Lead for a placeholder
        let lead = new_background_chat("Lead: reopen race", None).unwrap();
        let x = Expert { id: "coder".into(), name: "Coder".into(), text: "Write code.".into() };
        let e = spawn_expert("w-reopen-race", "worker", "Coder: reopen race", None, None, Some(&lead.id), x).unwrap();
        let saved = new_background_chat("Saved: reopen race", None).unwrap();
        saved.put("user", json!({"text": "hello"}));
        let id = saved.id.clone();
        remove_agent(&id);
        let back = open_chat(&id).expect("reopened");
        assert!(get(&lead.id).is_some() && get(&e.id).is_some(), "the silent Lead and its expert are still open");
        remove_agent(&lead.id);
        remove_agent(&back.id);
        assert!(get(&e.id).is_none());
    }

    #[test]
    fn switching_a_started_chat_hands_the_chat_over() {
        let a = Agent::detached_for_test("switch-test", None);
        // before anything is said: only the chat's AI is set, nothing is handed over
        assert_eq!(a.switch_ai("gemini"), Ok(false));
        assert_eq!(a.ai(), "gemini");
        assert!(a.st.lock().unwrap().handover.is_none());
        assert!(a.switch_ai("nobody").is_err());
        {
            let mut st = a.st.lock().unwrap();
            st.log.push(json!({"kind": "user", "text": "Build an obby"}));
            st.log.push(json!({"kind": "text", "text": "Three checkpoints are in."}));
            st.session = Some("old-session".into());
        }
        // working: refused, and nothing changes
        a.set_running_for_test(true);
        assert!(a.switch_ai("claude").is_err());
        assert_eq!(a.ai(), "gemini");
        a.set_running_for_test(false);
        assert_eq!(a.switch_ai("claude"), Ok(true));
        {
            let st = a.st.lock().unwrap();
            assert_eq!(st.ai.as_deref(), Some("claude"));
            assert!(st.session.is_none(), "the other AI starts a fresh session");
            let h = st.handover.as_deref().unwrap();
            assert!(h.contains("began with Gemini") && h.contains("User: Build an obby") && h.contains("Gemini: Three checkpoints are in."));
        }
        // the switch is in the chat's log, so the window says so after a restart too
        assert!(a.st.lock().unwrap().log.iter().any(|e| e["kind"] == "ai" && e["from"] == "gemini" && e["ai"] == "claude"));
        // switching to the AI it is already on changes nothing
        assert_eq!(a.switch_ai("claude"), Ok(false));
    }

    #[test]
    fn a_chat_switches_to_cursor_and_back() {
        let a = Agent::detached_for_test("switch-cursor", None);
        assert_eq!(a.switch_ai("claude"), Ok(false));
        {
            let mut st = a.st.lock().unwrap();
            st.log.push(json!({"kind": "user", "text": "Make a lava floor"}));
            st.log.push(json!({"kind": "text", "text": "The floor is in."}));
            st.session = Some("claude-session".into());
        }
        // to Cursor: a fresh session, handed Claude's part of the chat
        assert_eq!(a.switch_ai("cursor"), Ok(true));
        {
            let st = a.st.lock().unwrap();
            assert_eq!((st.ai.as_deref(), st.session.is_none()), (Some("cursor"), true));
            let h = st.handover.as_deref().unwrap();
            assert!(h.contains("User: Make a lava floor") && h.contains("Claude: The floor is in."));
        }
        // Cursor opens its conversation, answers, and the chat goes back to Claude with Cursor's reply under its name
        {
            let mut st = a.st.lock().unwrap();
            got_session(&mut st, "cursor-chat".into());
            assert!(st.handover.is_none(), "handed over once Cursor has a conversation");
            st.log.push(json!({"kind": "user", "text": "Make it glow"}));
            st.log.push(json!({"kind": "text", "text": "It glows now."}));
        }
        assert_eq!(a.switch_ai("claude"), Ok(true));
        let st = a.st.lock().unwrap();
        let h = st.handover.as_deref().unwrap();
        assert!(h.contains("Cursor: It glows now.") && h.contains("User: Make it glow"), "{h}");
        assert!(st.log.iter().any(|e| e["kind"] == "ai" && e["from"] == "cursor" && e["ai"] == "claude"));
    }

    #[test]
    fn a_reopened_expert_says_who_started_it() {
        let d = json!({"ai": "gemini", "role": "worker", "parent": "c1-lead", "team": "w123abc", "expert": "ui", "project": "/g"});
        let e = restored_event("c9-1", "UI builder: Shop menu", &d);
        assert_eq!((e["role"].as_str(), e["parent"].as_str(), e["team"].as_str()), (Some("worker"), Some("c1-lead"), Some("w123abc")));
        assert_eq!((e["expert"].as_str(), e["expert_id"].as_str(), e["ai"].as_str()), (Some("UI builder"), Some("ui"), Some("gemini")));
        assert_eq!(e["restored"], true);
        // the user's own chat: no helper fields at all, and an old chat with no AI is Claude's
        let own = restored_event("c1", "My chat", &json!({"role": null, "parent": null}));
        assert!(own.get("role").is_none() && own.get("parent").is_none() && own.get("expert").is_none());
        assert_eq!(own["ai"], "claude");
    }

    #[test]
    fn house_rules_are_assembled_and_capped() {
        assert_eq!(house_rules_block(None, None), "");
        assert_eq!(house_rules_block(Some("# House rules\n\n"), Some("  ")), "");
        let both = house_rules_block(Some("# House rules\n\n- Never use gradients.\n"), Some("# House rules\n- Lantern glow is amber.\n"));
        assert!(both.starts_with("\n\nHOUSE RULES."));
        assert!(both.contains("The user's house rules (follow these):\n- Never use gradients."));
        assert!(both.contains("This game's house rules (from the game's folder):\n- Lantern glow is amber."));
        assert!(both.find("Never use").unwrap() < both.find("Lantern").unwrap());
        assert!(!both.contains("# House rules"));
        // the limit comes before the rules and again last, so nothing in a rules file has the final word
        assert_eq!(both.matches(HOUSE_RULES_LIMIT).count(), 2);
        assert!(both.find(HOUSE_RULES_LIMIT).unwrap() < both.find("Never use").unwrap());
        assert!(both.trim_end().ends_with(HOUSE_RULES_LIMIT));
        assert!(HOUSE_RULES_LIMIT.contains("NEVER override") && HOUSE_RULES_LIMIT.contains("publishing"));
        let only_game = house_rules_block(None, Some("- Phones first."));
        assert!(only_game.contains("from the game's folder):\n- Phones first.") && !only_game.contains("The user's house rules"));
        // each file is capped on its own: a long file of the user's never pushes out the game's rules
        let long: String = (0..400).map(|n| format!("- Rule number {n} about the café menus.\n")).collect();
        let capped = house_rules_block(Some(&long), Some("- Game rule."));
        assert!(capped.contains("did not fit"));
        assert!(capped.contains("- Rule number 0 about the café menus."));
        assert!(capped.contains("- Game rule."));
        assert!(capped.chars().count() <= HOUSE_RULES_FILE_CAP + 2 * HOUSE_RULES_LIMIT.len() + 400, "{}", capped.chars().count());
        let capped = house_rules_block(Some("- Mine."), Some(&long));
        assert!(capped.contains("- Mine.") && capped.contains("did not fit"));
    }

    #[test]
    fn studio_run_start_commit_says_studio() {
        assert_eq!(projects::night_message(true), "Before the studio run (Fridae Studios checkpoint)");
        assert_eq!(projects::night_message(false), "Before the overnight run (Fridae Studios checkpoint)");
    }

    #[test]
    fn no_studio_keeps_the_studio_server_away() {
        let off = mcp_servers_with("c-1", "Chat", None, None, true);
        assert!(off.get("Roblox_Studio").is_none());
        assert_eq!(off["rbxkit"]["env"]["FRIDAE_NO_STUDIO"], "1");
        let on = mcp_servers_with("c-1", "Chat", None, None, false);
        assert!(on["rbxkit"]["env"].get("FRIDAE_NO_STUDIO").is_none());
        assert_eq!(on.get("Roblox_Studio").is_some(), paths::studio_mcp().is_some());
    }

    #[test]
    fn house_rules_are_read_from_home_and_the_game() {
        let tmp = std::env::temp_dir().join(format!("fridae house rules é {}", std::process::id()));
        let (home, game) = (tmp.join("home"), tmp.join("My Gäme"));
        fs::create_dir_all(home.join("notes")).unwrap();
        fs::create_dir_all(game.join(".fridae").join("notes")).unwrap();
        assert_eq!(house_rules(&home, Some(game.to_str().unwrap())), "");
        fs::write(home.join("notes").join(HOUSE_RULES_FILE), "# House rules\n\n- Restrained palettes.\n").unwrap();
        fs::write(game.join(".fridae").join("notes").join(HOUSE_RULES_FILE), "- Amber glow.\n").unwrap();
        let text = house_rules(&home, Some(game.to_str().unwrap()));
        assert!(text.contains("Restrained palettes.") && text.contains("Amber glow."));
        assert!(!house_rules(&home, None).contains("Amber glow."));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A shell in a group of its own (as `send` starts every agent) that has started a grandchild; returns both.
    #[cfg(unix)]
    fn shell_with_grandchild() -> (std::process::Child, String) {
        use std::io::BufRead;
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 60 & echo $!; wait"]).stdout(Stdio::piped());
        paths::own_group(&mut c);
        let mut child = c.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        (child, line.trim().to_string())
    }

    #[cfg(unix)]
    fn gone(pid: &str) -> bool {
        for _ in 0..60 {
            if !Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().unwrap().success() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn stop_ends_the_agent_and_what_it_started() {
        let (mut child, grandchild) = shell_with_grandchild();
        let a = Agent::detached_for_test("stopgroup", None);
        a.set_turn_pid_for_test(child.id());
        a.stop();
        let _ = child.wait(); // as read() does; the test itself (the app) is still here to do it
        assert!(gone(&grandchild), "the grandchild {grandchild} is still running");
        assert!(a.st.lock().unwrap().stopped);
    }

    #[cfg(unix)]
    #[test]
    fn stop_is_confirmed_a_turn_that_ignores_it_is_killed_after_the_wait_and_said() {
        use std::io::BufRead;
        for ignores in [true, false] {
            let mut c = Command::new("sh");
            c.args(["-c", if ignores { "trap '' TERM; sleep 60 & echo $!; wait" } else { "sleep 60 & echo $!; wait" }]).stdout(Stdio::piped());
            paths::own_group(&mut c);
            let mut child = c.spawn().unwrap();
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
            let a = Agent::detached_for_test(&format!("confirmed{ignores}"), None);
            let tp = a.set_turn_pid_for_test(child.id());
            let t = std::time::Instant::now();
            // the reader's part: the output ends when the group is gone, then the leader is reaped
            let reader = {
                let tp = tp.clone();
                thread::spawn(move || {
                    while signal_alive(tp.pid) {
                        thread::sleep(Duration::from_millis(50));
                    }
                    tp.done.store(true, Ordering::SeqCst);
                    tp.reaping();
                    child.wait()
                })
            };
            a.stop_turn().unwrap().stop();
            let _ = reader.join();
            assert!(gone(line.trim()), "the grandchild is gone");
            assert_eq!(tp.killed.load(Ordering::SeqCst), ignores, "killed only when it did not end when asked");
            if ignores {
                assert!(t.elapsed() >= paths::STOP_WAIT, "it was given the whole wait first");
            } else {
                assert!(t.elapsed() < paths::STOP_WAIT, "a turn that ends when asked is not waited for");
            }
        }
    }

    #[test]
    fn claudes_refused_compound_command_offers_the_program_that_is_missing() {
        let allowed = vec!["git".to_string()];
        let d = json!([{"tool_name": "Bash", "tool_input": {"command": "git status && rm -rf build"}}]);
        let (card, note) = blocked_or_note(claude_blocked(&d, &allowed), &allowed);
        assert_eq!(card.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["rm"]);
        assert!(note.is_none(), "not the form line: rm is simply not allowed");
        let d = json!([{"tool_name": "Bash", "tool_input": {"command": "./git status"}}]);
        let (card, note) = blocked_or_note(claude_blocked(&d, &allowed), &allowed);
        assert!(card.is_empty() && note.is_some(), "all allowed: it was the form");
    }

    #[test]
    fn claudes_deny_rules_are_absolute_on_every_system() {
        assert_eq!(claude_edit_rule("/Users/a/Documents/Fridae Studios/config.json", false, false), "Edit(//Users/a/Documents/Fridae Studios/config.json)");
        assert_eq!(claude_edit_rule("/Users/a/Documents/Fridae Studios/work/chats", true, false), "Edit(//Users/a/Documents/Fridae Studios/work/chats/**)");
        assert_eq!(claude_edit_rule(r"C:\Users\a\Documents\Fridae Studios\config.json", false, true), "Edit(//c/Users/a/Documents/Fridae Studios/config.json)");
        assert_eq!(claude_edit_rule(r"D:\Fridae\work\processes", true, true), "Edit(//d/Fridae/work/processes/**)");
        for r in claude_denied() {
            assert!(r.starts_with("Edit(//"), "{r}");
        }
    }

    #[test]
    fn a_failed_sign_in_with_a_kept_key_names_the_switch() {
        let note = key_note("claude", "Not logged in \u{b7} Please run /login", &["ANTHROPIC_API_KEY"], false).unwrap();
        assert!(note.contains("ANTHROPIC_API_KEY") && note.contains("Use API keys from my environment"));
        assert!(key_note("claude", "Not logged in", &["ANTHROPIC_API_KEY"], true).is_none(), "switch on: the key was passed");
        assert!(key_note("claude", "Not logged in", &[], false).is_none(), "no key set");
        assert!(key_note("claude", "The model is overloaded", &["ANTHROPIC_API_KEY"], false).is_none(), "not about signing in");
        // a tool's error quoting such words is not a sign-in failure; OpenCode gets no note (no switch hands it keys)
        for unrelated in ["curl: (22) The requested URL returned error: 401", "rbx: the login page of the game showed", "Open the place, then log in to Studio"] {
            assert!(key_note("claude", unrelated, &["ANTHROPIC_API_KEY"], false).is_none(), "{unrelated}");
        }
        assert!(key_note("opencode", "Unauthorized", &["OPENAI_API_KEY"], false).is_none());
        // a Claude tool call stays open until its own result, whatever lines come between
        let mut open = Default::default();
        claude_open_tools(&json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "a"}, {"type": "tool_use", "id": "b"}]}}), &mut open);
        claude_open_tools(&json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "b"}]}}), &mut open);
        claude_open_tools(&json!({"type": "rate_limit_event"}), &mut open);
        assert_eq!(open.len(), 1, "a is still running");
        claude_open_tools(&json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "a"}]}}), &mut open);
        assert!(open.is_empty());
        assert!(crate::paths::env_dropped("OPENAI_API_KEY", "opencode") && crate::paths::env_dropped("ANTHROPIC_API_KEY", "opencode"), "whatever the switch says");
        for k in ["BASH_FUNC_echo%%", "BASH_ENV", "ENV", "CDPATH", "PS4"] {
            assert!(crate::paths::shell_injected(k), "{k}");
        }
        assert!(!crate::paths::shell_injected("PATH") && !crate::paths::shell_injected("HOME"));
    }

    #[test]
    fn the_switch_decides_whether_provider_keys_reach_an_ai() {
        // a Claude session's variables always go; a provider key only while the switch is off
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let was = paths::env_keys_on();
        for on in [false, true] {
            paths::update_config(|c| c.insert("use_env_keys".into(), json!(on)));
            assert!(paths::env_dropped("CLAUDECODE", "claude") && paths::env_dropped("CLAUDE_CODE_SESSION_ID", "chatgpt"));
            assert_eq!(paths::env_dropped("ANTHROPIC_API_KEY", "claude"), !on);
            assert_eq!(paths::env_dropped("ANTHROPIC_AUTH_TOKEN", "claude"), !on);
            assert!(!paths::env_dropped("ANTHROPIC_BASE_URL", "claude"));
        }
        paths::update_config(|c| c.insert("use_env_keys".into(), json!(was)));
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_ignores_term_and_holds_the_error_output_neither_survives_nor_keeps_the_turn_busy() {
        // the AI answers and exits; a child of it in its group ignores SIGTERM and keeps the error output open
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // the turn's end checks the allowed list
        let dir = paths::test_dir("stray-child");
        fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let script = format!(
            "(trap '' TERM; exec sleep 60) >/dev/null & echo $! > '{}'; echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"s1\"}}'",
            pidfile.display()
        );
        let mut c = Command::new("sh");
        c.args(["-c", &script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        paths::own_group(&mut c);
        let child = c.spawn().unwrap();
        let a = Agent::detached_for_test("stray", None);
        let tp = a.set_turn_pid_for_test(child.id());
        let t = std::time::Instant::now();
        let reader = {
            let a = a.clone();
            thread::spawn(move || a.read(child, "claude", tp))
        };
        while !reader.is_finished() && t.elapsed() < Duration::from_secs(20) {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(reader.is_finished(), "the turn ended");
        assert!(!a.st.lock().unwrap().running, "and the chat is not busy");
        let pid = fs::read_to_string(&pidfile).unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !gone(pid.trim()) && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(gone(pid.trim()), "the child that ignored SIGTERM was killed");
    }

    #[cfg(unix)]
    fn signal_alive(group: u32) -> bool {
        Command::new("kill").args(["-0", "--", &format!("-{group}")]).stderr(Stdio::null()).status().unwrap().success()
    }

    #[test]
    fn a_long_waiting_tool_is_recognised_in_every_ais_output() {
        let claude = json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "mcp__rbxkit__rbx_team", "input": {}}]}});
        assert!(claude_starts_long_tool(&claude));
        assert!(!claude_starts_long_tool(&json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash"}]}})));
        assert!(starts_long_tool(&[Out::Tool("mcp__rbxkit__rbx_texture".into())]));
        assert!(!starts_long_tool(&[Out::Text("rbx_team".into())]));
    }

    #[cfg(unix)]
    #[test]
    fn end_trees_ends_a_group_that_ignores_sigterm() {
        use std::io::BufRead;
        let mut c = Command::new("sh");
        c.args(["-c", "trap '' TERM; sleep 60 & echo $!; wait"]).stdout(Stdio::piped());
        paths::own_group(&mut c);
        let mut child = c.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let group = child.id();
        let reaper = thread::spawn(move || child.wait());
        let t = std::time::Instant::now();
        paths::end_trees(&[group]);
        assert!(t.elapsed() >= paths::STOP_GRACE / 2, "SIGTERM was ignored, so it took the grace and SIGKILL");
        let _ = reaper.join();
        assert!(gone(line.trim()));
    }

    fn typed(text: &str, studio: bool) -> (String, String, Ask) {
        ("q".into(), text.into(), Ask { studio, ..Default::default() })
    }

    fn woken(text: &str) -> (String, String, Ask) {
        ("w".into(), text.into(), Ask { studio: true, full: true, overnight: true, wake: true, ..Default::default() })
    }

    #[test]
    fn stop_and_end_drop_the_apps_own_wakes() {
        let mut q = vec![woken("Resume the studio run."), typed("Keep it under five minutes", true), woken("Review: Plan — approved")];
        assert!(drop_wakes(&mut q, false), "Stop: the wakes go");
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].1, "Keep it under five minutes");
        assert!(q[0].2.studio, "Stop alone keeps what the user typed as it was");
        assert!(drop_wakes(&mut q, true), "End: the typed message stays, as an ordinary one");
        assert_eq!(q.len(), 1);
        assert!(!q[0].2.studio && !q[0].2.overnight && !q[0].2.full);
        assert!(!drop_wakes(&mut q, true), "nothing more to change");
        let mut plain = vec![typed("hello", false)];
        assert!(!drop_wakes(&mut plain, true), "an ordinary message is left alone");
    }

    #[test]
    fn an_ended_runs_lead_is_an_ordinary_chat_again() {
        let ended = studio_undo::LeadOf { leads: false, started: false, ended: true };
        let running = studio_undo::LeadOf { leads: true, started: true, ended: false };
        let last = Ask { studio: true, full: true, overnight: true, ..Default::default() };
        // the app's wakes and the loop's carry-ons for an ended run go nowhere
        assert!(lead_ask(&woken("Resume the studio run.").2, ended, false, &last).is_none());
        let carry = Ask { kickoff: Some("auto".into()), ..last.clone() };
        assert!(lead_ask(&carry, ended, false, &last).is_none());
        // a message typed in Studio mode goes as Ask me: no Studio mode, so no night checkpoint
        let a = lead_ask(&Ask { studio: true, ..Default::default() }, ended, true, &last).unwrap();
        assert!(!a.studio && !a.overnight && !a.full && !a.autopilot);
        assert!(!takes_night_checkpoint(&a, false, false, false));
        // an ordinary message in an ordinary mode keeps its settings (overnight is an ordinary overnight)
        let night = Ask { overnight: true, ..Default::default() };
        let a = lead_ask(&night, ended, false, &Ask::default()).unwrap();
        assert!(a.overnight && !a.studio);
        // a running run's Lead: as before (any message is a Studio message, the start checkpoint is kept)
        let a = lead_ask(&Ask::default(), running, false, &last).unwrap();
        assert!(a.studio && a.overnight && a.full);
        assert!(!takes_night_checkpoint(&a, false, false, true));
        // no run at all: unchanged
        let a = lead_ask(&Ask::default(), studio_undo::LeadOf::default(), false, &Ask::default()).unwrap();
        assert!(!a.studio && !a.overnight);
    }

    #[test]
    fn end_finds_the_leads_working_helpers() {
        let mut st = State::default();
        assert!(!working_helper_of(&st, "lead"));
        st.team = Some("w1".into());
        st.parent = Some("lead".into());
        assert!(!working_helper_of(&st, "lead"), "not working");
        st.running = true;
        assert!(working_helper_of(&st, "lead"));
        assert!(!working_helper_of(&st, "other"));
        assert!(!working_helper_of(&st, ""));
        st.team = None;
        assert!(!working_helper_of(&st, "lead"), "a chat the user opened is not a helper");
    }

    #[test]
    fn a_busy_leads_held_message_nudges_its_wait() {
        let a = Agent::detached_for_test("nudge-lead", None);
        let _ = fs::remove_file(a.nudge_file());
        {
            let mut st = a.st.lock().unwrap();
            st.last_ask.studio = true;
            st.sending = true; // working
        }
        a.ask("Also keep the course under five minutes", &Ask::default());
        let n: Value = serde_json::from_str(&fs::read_to_string(a.nudge_file()).unwrap()).unwrap();
        assert_eq!(n, json!({"typed": 1, "wakes": 0}));
        a.ask("Review: Plan — approved", &woken("x").2);
        let n: Value = serde_json::from_str(&fs::read_to_string(a.nudge_file()).unwrap()).unwrap();
        assert_eq!(n, json!({"typed": 1, "wakes": 1}));
        a.clear_nudge(); // the turn ended
        assert!(!a.nudge_file().exists());
        // an ordinary chat's held message nudges nothing
        let b = Agent::detached_for_test("nudge-plain", None);
        b.st.lock().unwrap().sending = true;
        b.ask("hello", &Ask::default());
        assert!(!b.nudge_file().exists());
    }

    #[test]
    fn restore_reopens_every_chat_slot_holders_first_and_newest() {
        let c = |id: &str, holds: bool, t: f64| (id.to_string(), holds, t);
        // finished helpers take no slot: all are reopened, after the slot holders
        let listed = vec![c("a", true, 1.0), c("h1", false, 2.0), c("b", true, 3.0), c("h2", false, 4.0)];
        assert_eq!(restore_order(&listed, 8), vec!["a", "b", "h1", "h2"]);
        // more slot holders than slots: the newest ones, in their listed order; the rest stay under Earlier
        let listed = vec![c("old", true, 1.0), c("h", false, 9.0), c("new1", true, 5.0), c("mid", true, 3.0), c("new2", true, 6.0)];
        assert_eq!(restore_order(&listed, 2), vec!["new1", "new2", "h"]);
        assert_eq!(restore_order(&[], 8), Vec::<String>::new());
    }

    #[test]
    fn a_finished_helper_stays_finished_after_a_restart() {
        let a = new_background_chat("Designer: restart", None).unwrap();
        {
            let mut st = a.st.lock().unwrap();
            st.team = Some("w-restart".into());
            st.parent = Some("lead-x".into());
        }
        a.put("user", json!({"text": "Design the plan"}));
        a.mark_result();
        a.save();
        let id = a.id.clone();
        remove_agent(&id);
        let d: Value = serde_json::from_str(&fs::read_to_string(chat_file(&id)).unwrap()).unwrap();
        assert_eq!(d["team"], "w-restart");
        assert_eq!(d["result_written"], true);
        let b = open_chat(&id).unwrap();
        assert!(!b.holds_slot(), "a finished helper holds no slot after a restart");
        assert_eq!(b.team().map(|t| t.0).as_deref(), Some("w-restart"));
        remove_agent(&id);
    }

    #[test]
    fn a_message_to_a_finished_helper_needs_a_free_slot() {
        assert!(!no_slot_for(true, || MAX_AGENTS), "a chat holding a slot is never refused");
        assert!(!no_slot_for(false, || MAX_AGENTS - 1), "a free slot: it takes it");
        assert!(no_slot_for(false, || MAX_AGENTS), "none free: refused");
        let mut counted = false;
        assert!(!no_slot_for(true, || {
            counted = true;
            0
        }));
        assert!(!counted, "the slots are only counted for a chat that holds none");
    }

    #[test]
    fn progress_calls_are_the_ones_that_move_the_run_on() {
        assert!(progress_call("mcp__rbxkit__rbx_stage", &json!({"action": "review", "title": "Plan"})));
        assert!(progress_call("mcp__rbxkit__rbx_stage", &json!({"action": "stage", "id": "build"})));
        assert!(progress_call("mcp__rbxkit__rbx_stage", &json!({"action": "lesson"})));
        assert!(!progress_call("mcp__rbxkit__rbx_stage", &json!({"action": "decisions"})));
        assert!(!progress_call("mcp__rbxkit__rbx_stage", &json!({"action": "status"})));
        assert!(progress_call("mcp__rbxkit__rbx_team", &json!({"action": "spawn"})));
        assert!(!progress_call("mcp__rbxkit__rbx_team", &json!({"action": "wait", "ids": ["w1"]})));
        assert!(!progress_call("mcp__rbxkit__rbx_team", &json!({"action": "status"})));
        assert!(progress_call("mcp__Roblox_Studio__run_code", &json!({})));
        assert!(progress_call("Edit", &json!({"file_path": "GAME.md"})));
        assert!(progress_call("Write", &json!({})));
        assert!(!progress_call("Read", &json!({})));
        assert!(!progress_call("mcp__rbxkit__rbx_board", &json!({"action": "list"})));
        // an AI that does not report the arguments: rbx_stage and rbx_team count
        assert!(progress_call("rbx_team", &Value::Null));
    }

    #[test]
    fn six_carry_ons_without_progress_count_as_waiting() {
        let mut n = 0;
        for _ in 0..5 {
            assert!(!no_progress_call(true, true, true, false, &mut n));
        }
        assert!(no_progress_call(true, true, true, false, &mut n), "the sixth trips it");
        assert_eq!(n, 0, "and the count starts again");
        for _ in 0..5 {
            no_progress_call(true, true, true, false, &mut n);
        }
        assert!(!no_progress_call(true, true, true, true, &mut n), "a progress call resets it");
        assert_eq!(n, 0);
        for (studio, carry_on, ok) in [(false, true, true), (true, false, true), (true, true, false)] {
            let mut n = 5;
            assert!(!no_progress_call(studio, carry_on, ok, false, &mut n));
            assert_eq!(n, 0, "not a Lead, a user's turn, or a failed turn: no count");
        }
        // the three-turns-without-any-tool rule is unchanged
        let mut idle = 0;
        assert!(!no_progress(true, true, true, 0, &mut idle));
        assert!(!no_progress(true, true, true, 0, &mut idle));
        assert!(no_progress(true, true, true, 0, &mut idle));
    }

    #[test]
    fn the_guards_hold_off_while_the_leads_experts_work() {
        assert!(guarded(true, false));
        assert!(!guarded(true, true), "experts at work: the Lead is waiting for them, not stuck");
        assert!(!guarded(false, false));
        // held off, the counts start again, so neither guard fires
        let (mut idle, mut stuck) = (2, 5);
        let lead = guarded(true, true);
        assert!(!no_progress(lead, true, true, 0, &mut idle));
        assert!(!no_progress_call(lead, true, true, false, &mut stuck));
        assert_eq!((idle, stuck), (0, 0));
    }

    #[test]
    fn nothing_starts_once_the_app_is_quitting() {
        let a = Agent::detached_for_test("quitting", None);
        set_shutting(true);
        a.send("Carry on", &Ask::default());
        set_shutting(false);
        assert!(a.last_user_text_for_test().is_none(), "no turn was begun");
        assert!(!a.busy());
        a.send("Hello", &Ask::default());
        assert_eq!(a.last_user_text_for_test().as_deref(), Some("Hello"), "and starts again once it is not");
    }

    #[test]
    fn end_trees_never_signals_the_app_itself() {
        paths::end_trees(&[0, 1, std::process::id()]); // the test run (the app here) goes on
    }

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
    fn an_error_result_is_a_failed_turn() {
        let signed_out = json!({"type": "result", "subtype": "success", "is_error": true, "result": "Not logged in · Please run /login"});
        assert!(usage_limit("You've hit your 5-hour limit \u{b7} resets 12:50am (America/Los_Angeles)"));
        assert!(usage_limit("Claude usage limit reached. Your limit will reset at 3pm."));
        assert!(!usage_limit("Not logged in \u{b7} Please run /login"));
        assert!(!usage_limit("The stone limit is 40 per player"));
        assert!(!result_ok(&signed_out));
        assert!(result_ok(&json!({"type": "result", "subtype": "success", "is_error": false, "result": "Done."})));
        assert!(result_ok(&json!({"type": "result", "result": "Done."})));
        // three such turns in a row end a studio run instead of carrying on for 24 hours
        let mut ls = LoopState { autopilot_left: 0, fails: 0, done: 0, night: true };
        let said = "NOT LOGGED IN · PLEASE RUN /LOGIN";
        assert!(matches!(next_step(Mode::Studio, result_ok(&signed_out), said, &mut ls), Step::CarryOn(_)));
        assert!(matches!(next_step(Mode::Studio, result_ok(&signed_out), said, &mut ls), Step::CarryOn(_)));
        assert!(matches!(next_step(Mode::Studio, result_ok(&signed_out), said, &mut ls), Step::End(_)));
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
    fn studio_without_a_project_falls_back_to_ask_me() {
        // what the server makes of a Studio message (studio implies overnight and full)
        let studio = Ask { studio: true, overnight: true, full: true, autopilot: true, model: Some("opus".into()), ..Default::default() };
        let (a, line) = studio_fallback(&studio, false);
        assert!(!a.studio && !a.overnight && !a.full && !a.autopilot, "Ask me: does what it is asked, then waits");
        assert_eq!(a.model.as_deref(), Some("opus"));
        assert_eq!(line, Some(STUDIO_FALLBACK));
        let (a, line) = studio_fallback(&studio, true);
        assert!(a.studio && a.overnight && line.is_none(), "a game project: Studio as asked");
        let night = Ask { overnight: true, full: true, ..Default::default() };
        let (a, line) = studio_fallback(&night, false);
        assert!(a.overnight && a.full && line.is_none(), "other modes are not touched");
    }

    #[test]
    fn held_studio_message_takes_no_checkpoint() {
        let studio = Ask { studio: true, overnight: true, full: true, ..Default::default() };
        // a message to the Lead of a run that has its start checkpoint (a reply while it works, a decision, Resume)
        assert!(!takes_night_checkpoint(&studio, false, false, true));
        // the run's first message takes the start checkpoint
        assert!(takes_night_checkpoint(&studio, false, false, false));
        // a waiting run going on, and the loop's own carry-ons, never do
        assert!(!takes_night_checkpoint(&studio, false, true, false));
        assert!(!takes_night_checkpoint(&studio, true, false, false));
        // Overnight for an ordinary chat: as before, every message of the user's starts a night
        let night = Ask { overnight: true, full: true, ..Default::default() };
        assert!(takes_night_checkpoint(&night, false, false, false));
        assert!(takes_night_checkpoint(&night, false, false, true), "only a studio message keeps the run's checkpoint");
        assert!(!takes_night_checkpoint(&Ask::default(), false, false, false));
    }

    #[test]
    fn resume_takes_no_checkpoint() {
        // Resume (and a decision) reach the Lead through wake_studio: a non-carry-on Studio message
        let wake = Ask { studio: true, full: true, overnight: true, ..Default::default() };
        let mut st = State { last_ask: wake.clone(), ..Default::default() };
        let ask = effective_ask(&studio_message(&wake, true), st.studio_waiting, &st.last_ask);
        let resumed = begin_send(&mut st, &ask, false, 10.0).unwrap();
        assert!(!takes_night_checkpoint(&ask, false, resumed, true));
    }

    #[test]
    fn a_message_to_the_lead_is_a_studio_message_whatever_its_mode() {
        let last = Ask { studio: true, overnight: true, full: true, ..Default::default() };
        for typed in [Ask::default(), Ask { full: true, ..Default::default() }, Ask { overnight: true, full: true, ..Default::default() }] {
            let a = effective_ask(&studio_message(&typed, true), false, &last);
            assert!(a.studio && a.overnight && a.full, "the Lead of a running run stays in Studio mode");
            let a = effective_ask(&studio_message(&typed, false), false, &last);
            assert!(!a.studio, "not a run's Lead: the message's own mode");
        }
        // messages held while it worked go as one turn: Studio is kept if any of them was a Studio message
        let held = vec![("q1".to_string(), "a".to_string(), Ask { studio: true, ..Default::default() }), ("q2".to_string(), "b".to_string(), Ask::default())];
        assert!(merged_studio(&held, false));
        assert!(!merged_studio(&held[1..], false));
        assert!(merged_studio(&held[1..], true));
    }

    #[test]
    fn no_progress_guard_waits_after_three_idle_carry_ons() {
        let mut idle = 0;
        assert!(!no_progress(true, true, true, 0, &mut idle));
        assert!(!no_progress(true, true, true, 0, &mut idle));
        assert!(no_progress(true, true, true, 0, &mut idle), "the third idle carry-on counts as STATUS: WAITING");
        assert_eq!(idle, 0, "and the count starts again");
        // a tool call, a message from the user, or a failed turn breaks the run of idle turns
        for (carry_on, got, tools) in [(true, true, 1), (false, true, 0), (true, false, 0)] {
            let mut idle = 2;
            assert!(!no_progress(true, carry_on, got, tools, &mut idle));
            assert_eq!(idle, 0);
        }
        // not a Studio Lead: never
        let mut idle = 0;
        for _ in 0..5 {
            assert!(!no_progress(false, true, true, 0, &mut idle));
        }
        // the turn it trips on waits like a Lead that said STATUS: WAITING
        let mut s = night();
        assert_eq!(next_step(Mode::Studio, true, "STATUS: WAITING", &mut s), Step::Wait);
    }

    #[test]
    fn an_ended_run_stops_carrying_on_and_waiting() {
        let mut s = night();
        assert_eq!(ended_step(Step::CarryOn(NEXT), true, &mut s), Step::End(RUN_ENDED));
        assert!(!s.night);
        let mut s = night();
        assert_eq!(ended_step(Step::Wait, true, &mut s), Step::End(RUN_ENDED));
        assert_eq!(ended_step(Step::Idle, true, &mut s), Step::Idle);
        let mut s = night();
        assert_eq!(ended_step(Step::CarryOn(NEXT), false, &mut s), Step::CarryOn(NEXT));
        assert!(s.night);
        // the Lead itself: the clock, autopilot and a wait end, and the next carry-on is dropped
        let a = Agent::new("c-end-test".into(), "Lead");
        {
            let mut st = a.st.lock().unwrap();
            st.studio_waiting = true;
            st.overnight_until = 1e12;
            st.autopilot_left = 5;
        }
        a.end_run();
        {
            let st = a.st.lock().unwrap();
            assert!(!st.studio_waiting && st.overnight_until == 0.0 && st.autopilot_left == 0 && st.halted);
        }
        assert_eq!(begin_send(&mut a.st.lock().unwrap(), &Ask { kickoff: Some("auto".into()), ..Default::default() }, true, 1.0), None);
    }

    #[test]
    fn a_finished_helper_holds_no_slot() {
        let mut st = State::default();
        assert!(holds_slot(&st), "the user's own chat");
        st.team = Some("w1".into());
        assert!(holds_slot(&st), "a helper at work");
        st.result_written = true;
        assert!(!holds_slot(&st), "finished: open to read, no slot");
        st.running = true;
        assert!(holds_slot(&st), "the user messaged it: it counts again while it works");
        st.running = false;
        st.sending = true;
        assert!(holds_slot(&st));
    }

    #[test]
    fn finished_experts_free_their_slots_one_after_another() {
        // more experts than MAX_AGENTS, one after another: each one's slot is free once its result is handed back
        let mut made = Vec::new();
        for n in 0..(MAX_AGENTS + 4) {
            let x = Expert { id: "coder".into(), name: "Coder".into(), text: "Write code.".into() };
            let a = spawn_expert(&format!("w-slots-{n}"), "worker", &format!("Coder: task {n}"), None, None, Some("c-slots-lead"), x).expect("a free slot");
            assert!(a.holds_slot());
            a.mark_result();
            assert!(!a.holds_slot());
            made.push(a);
        }
        assert!(slots_used() < MAX_AGENTS, "finished helpers are not counted");
        for a in made {
            remove_agent(&a.id);
        }
    }

    #[test]
    fn an_expert_keeps_its_instructions_when_reopened() {
        let x = Expert { id: "ui".into(), name: "UI builder".into(), text: "Build the HUD é.".into() };
        let a = spawn_expert("w-reopen", "worker", "UI builder: HUD", None, None, None, x).unwrap();
        a.put("user", json!({"text": "Make the HUD"}));
        let id = a.id.clone();
        remove_agent(&id); // saved to the (temporary) work folder
        let back = open_chat(&id).expect("reopened");
        assert_eq!(back.st.lock().unwrap().expert_text.as_deref(), Some("Build the HUD é."));
        assert_eq!(back.expert().as_deref(), Some("ui"));
        remove_agent(&id);
    }

    #[test]
    fn tests_use_a_temporary_work_folder() {
        assert!(PATHS.work.starts_with(std::env::temp_dir()), "{}", PATHS.work.display());
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
    fn a_turn_being_sent_counts_as_working_so_undo_waits() {
        let a = Agent::new("c-sending-test".into(), "Lead");
        let carry_on = Ask { kickoff: Some("auto".into()), ..Default::default() };
        // a carry-on that got through begin_send is busy from then on, before the AI runs: halt cannot drop it any more,
        // so undo's busy check must refuse
        assert_eq!(begin_send(&mut a.st.lock().unwrap(), &carry_on, true, 1.0), Some(false));
        let sending = Sending(&a);
        assert!(a.busy(), "between begin_send and the turn running");
        a.halt();
        assert!(!a.st.lock().unwrap().halted, "halt does not linger for a turn already under way");
        a.set_running_for_test(true);
        drop(sending);
        assert!(a.busy(), "the turn runs: still busy");
        a.set_running_for_test(false);
        assert!(!a.busy());
        // halted first: the next carry-on never starts sending
        a.halt();
        assert_eq!(begin_send(&mut a.st.lock().unwrap(), &carry_on, true, 2.0), None);
        assert!(!a.busy());
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
    fn pause_replaces_carry_on() {
        // not paused: whatever next_step decided goes ahead, and the "told" mark is cleared
        let (mut told, mut s) = (true, night());
        assert_eq!(pause_step(Step::CarryOn(NEXT), false, &mut told, &mut s), Step::CarryOn(NEXT));
        assert!(!told && s.night);
        // paused: the first carry-on becomes the one pause message
        let mut s = night();
        assert_eq!(pause_step(Step::CarryOn(NEXT), true, &mut told, &mut s), Step::CarryOn(PAUSE_TEXT));
        assert!(told && s.night);
        assert!(PAUSE_TEXT.contains("STATUS: WAITING") && PAUSE_TEXT.contains("start nothing new"));
        // after that it waits, whatever the Lead said, and the clock stops
        assert_eq!(pause_step(Step::CarryOn(NEXT), true, &mut told, &mut s), Step::Wait);
        assert!(told && !s.night);
        // waiting, ending and idling are left as they are
        let mut s = night();
        assert_eq!(pause_step(Step::Wait, true, &mut told, &mut s), Step::Wait);
        assert_eq!(pause_step(Step::Idle, true, &mut told, &mut s), Step::Idle);
        assert!(matches!(pause_step(Step::End("x"), true, &mut told, &mut s), Step::End("x")));
        // a fresh pause after a resume tells it again
        let mut told = false;
        assert_eq!(pause_step(Step::CarryOn(TURN_FAILED), true, &mut told, &mut night()), Step::CarryOn(PAUSE_TEXT));
    }

    #[test]
    fn paused_run_replaces_or_drops_a_late_carry_on() {
        let mut told = false;
        assert_eq!(paused_carry_on(NEXT, &mut told), Some(PAUSE_TEXT));
        assert!(told);
        assert_eq!(paused_carry_on(NEXT, &mut told), None, "told once: the next is dropped");
        assert_eq!(paused_carry_on(PAUSE_TEXT, &mut told), Some(PAUSE_TEXT), "the pause message itself goes");
        let mut told = false;
        assert_eq!(paused_carry_on(PAUSE_TEXT, &mut told), Some(PAUSE_TEXT));
        // halted (an undo, or Stop) drops the carry-on whatever it says
        let mut st = State { halted: true, ..Default::default() };
        assert_eq!(begin_send(&mut st, &Ask { kickoff: Some("auto".into()), ..Default::default() }, true, 1.0), None);
    }

    #[test]
    fn helper_steps_join_run_log() {
        let log = vec![
            json!({"kind": "user", "text": "go"}),
            json!({"kind": "tool", "label": "Reading files"}),
            json!({"kind": "tool", "label": "Moving a part", "studio": true, "t": 30.0}),
            json!({"kind": "helper_steps", "name": "Coder: Shop", "steps": [{"label": "Editing a script", "t": 10.0}, {"label": "Adding a part", "t": 40.0}]}),
            json!({"kind": "tool", "label": "Playtest", "studio": true, "t": 20.0}),
            json!({"kind": "helper_steps", "name": "UI builder: HUD", "steps": [{"label": "Making a frame", "t": 35.0}, {"label": "bad"}]}),
        ];
        let merged = merge_run_log(&log);
        let row = |i: usize| (merged[i]["t"].as_f64().unwrap(), merged[i]["by"].as_str().unwrap().to_string(), merged[i]["label"].as_str().unwrap().to_string());
        assert_eq!(merged.len(), 5, "a step without a time and tools outside Studio are left out");
        assert_eq!(row(0), (10.0, "Coder: Shop".into(), "Editing a script".into()));
        assert_eq!(row(1), (20.0, "Lead".into(), "Playtest".into()));
        assert_eq!(row(2), (30.0, "Lead".into(), "Moving a part".into()));
        assert_eq!(row(3), (35.0, "UI builder: HUD".into(), "Making a frame".into()));
        assert_eq!(row(4), (40.0, "Coder: Shop".into(), "Adding a part".into()));
        // what a helper hands over: its own Studio steps from this turn on
        let helper = vec![json!({"kind": "tool", "label": "Old", "studio": true, "t": 5.0}), json!({"kind": "tool", "label": "New", "studio": true, "t": 50.0}), json!({"kind": "tool", "label": "Read"})];
        let own = studio_steps_since(&helper, 10.0);
        assert_eq!(own, vec![json!({"label": "New", "t": 50.0})]);
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
