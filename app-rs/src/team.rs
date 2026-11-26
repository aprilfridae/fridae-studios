//! Agents starting other agents. A tool writes a request file; this picks it up,
//! starts a worker or a reviewer in its own chat, and writes the outcome back.
//!
//!   work/team-requests/<id>.json   {"id", "from", "role", "name", "task"}
//!   work/team-results/<id>.json    {"id", "agent", "name", "role", "status", "reason", "text"}
//!
//! Studio mode adds a named expert: the request also has "expert", "expert_name", "expert_text" and "writes"
//! (the tool read them from the roster; this never does), and the result "expert". Only one expert that changes the
//! game ("writes") runs per project at a time.
//!
//! A request may also name "ai" and "model"; `expert_ai::resolve` decides where the helper runs (the user's Setup >
//! Experts table first), and the result then carries "ai", "model" (the model it runs on, named even when it is the
//! AI's default) and, whenever that is not what was asked for or there is something the Lead should know about it,
//! "note". A choice that cannot be honoured is a refusal with its reason.
//!
//!   work/team-models.json          what each AI offers, kept up to date here for `rbx_team` action "models"

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    expert_ai, paths, studio_run,
};

/// How long an expert's result waits for its plain-English summary before it goes back to the Lead. A short, hard
/// cap: if the summary is not ready by then, the result goes with the expert's one-liner and the summary attaches
/// late (`Agent::start_plain_summary`), so the run is never held past this. Matches the writer's own `PLAIN_LIMITS`.
#[cfg(not(test))]
const SUMMARY_WAIT: Duration = Duration::from_secs(45);
#[cfg(test)]
const SUMMARY_WAIT: Duration = Duration::from_millis(300);

fn write_result(id: &str, value: Value) {
    let dir = paths::work("team-results");
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join(format!("{id}.tmp"));
    if fs::write(&tmp, value.to_string()).is_ok() {
        let _ = fs::rename(&tmp, dir.join(format!("{id}.json")));
    }
}

/// The writer places in use: project folder -> the title of the helper holding it. In memory only: it starts empty,
/// as it should, because no helper survives a restart.
static WRITERS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn with_writers<T>(f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
    f(WRITERS.lock().unwrap().get_or_insert_with(HashMap::new))
}

/// Take the project's one Studio turn for `agent_name`, or say who has it. Since the swarm, file work runs in parallel
/// (each writer in its own copy, `swarm`): only changes inside the live Studio place take turns. Readers never need it.
fn writer_gate(held: &mut HashMap<String, String>, project: &str, agent_name: &str, writes: bool) -> Result<(), String> {
    if !writes {
        return Ok(());
    }
    if let Some(who) = held.get(project) {
        return Err(format!("Another member has the Studio turn right now ({who}). Do work that needs no Studio, then ask again."));
    }
    held.insert(project.to_string(), agent_name.to_string());
    Ok(())
}

/// A helper that held the writer place lets go of it (only once, however many ways its chat ends).
pub fn release(agent: &Agent) {
    if let Some(project) = agent.take_writer() {
        with_writers(|w| w.remove(&project));
    }
}

/// "Coder: Shop menu", cut to the 34 characters every chat title gets.
fn expert_title(expert_name: &str, name: &str) -> String {
    format!("{expert_name}: {name}").chars().take(34).collect()
}

/// Studio's run file carries a note the page shows ("slots": waiting for a free agent slot). Only touches a run that exists.
fn set_slots_note(project: &str, on: bool) {
    let dir = Path::new(project);
    if !studio_run::run_file(dir).is_file() {
        return;
    }
    let _ = studio_run::change_run(dir, |run| {
        if !run.is_object() {
            return (false, ());
        }
        if on {
            if run["note"] == "slots" {
                return (false, ());
            }
            run["note"] = json!("slots");
        } else if run["note"] == "slots" {
            run.as_object_mut().map(|o| o.remove("note"));
        } else {
            return (false, ());
        }
        (true, ())
    });
}

/// The limit on experts working at once. None when there is room for one more.
fn experts_limit(working: usize, max: u64) -> Option<String> {
    (working as u64 >= max).then(|| format!("The run already has {working} experts working (the limit is {max}). Wait for one, then start this one."))
}

/// How many of a Lead's experts may work at once: the user's ceiling (`swarm::ceiling`, Setup > Experts), or a
/// running studio run's own "max_experts" when that is lower.
fn expert_max(run: Option<&Value>, ceiling: usize) -> u64 {
    let own = run.and_then(|r| r["max_experts"].as_u64()).filter(|n| (1..=agents::MAX_AGENTS as u64).contains(n));
    own.map_or(ceiling as u64, |n| n.min(ceiling as u64))
}

/// For a request from a Lead: refused when as many of its helpers as the ceiling allows are at work (a helper counts
/// until its result is handed back, or again while it works after being resumed; the Librarian is not counted).
fn over_expert_limit(lead: &Agent, project: &str) -> Option<String> {
    let run = studio_run::read_run(Path::new(project)).filter(|r| studio_run::running(r) && r["lead"] == lead.id.as_str());
    let max = expert_max(run.as_ref(), crate::swarm::ceiling());
    let working = agents::agents()
        .iter()
        .filter(|a| a.parent().as_deref() == Some(lead.id.as_str()) && a.team().is_some() && a.holds_slot() && !crate::swarm::is_librarian(&a.id))
        .count();
    experts_limit(working, max)
}

pub const SLOT_FREE: &str = "An agent slot is free. Carry on.";

/// A slot came free (a helper finished, or a chat was closed): every waiting Lead whose run has the note "slots" gets
/// the note cleared and is woken with SLOT_FREE.
pub fn slot_freed() {
    for lead in agents::agents() {
        // (tests share the list of chats: another test's turn ending must not wake this test's waiting Lead)
        #[cfg(test)]
        if !lead.of_this_test() {
            continue;
        }
        if lead_to_wake(&lead) {
            lead.wake_studio(SLOT_FREE);
        }
    }
}

/// Whether this chat is a waiting Lead of a running, unpaused run with the note "slots"; if so the note is cleared
/// (under the lock), so it is woken once.
fn lead_to_wake(lead: &Agent) -> bool {
    if lead.role().is_some() || !lead.studio_waiting() {
        return false;
    }
    let Some(project) = lead.project() else { return false };
    let dir = Path::new(&project);
    if !studio_run::run_file(dir).is_file() {
        return false;
    }
    studio_run::change_run(dir, |run| {
        let go = run.is_object() && studio_run::running(run) && run["paused"] != true && run["lead"] == lead.id.as_str() && run["note"] == "slots";
        if go {
            run.as_object_mut().map(|o| o.remove("note"));
        }
        (go, go)
    })
    .unwrap_or(false)
}

/// Why a helper cannot start (or a finished one take a message): every agent slot is in use.
pub const NO_SLOT: &str = "All agent slots are in use. Wait for one to finish, or close a chat.";

/// Why an expert is not started for the Lead of a run that was ended in Review.
pub const RUN_ENDED: &str = "This project's studio run was ended.";
/// What a Lead is told when it tries to start an expert that changes the game while Studio cannot be reached.
pub const NO_STUDIO: &str = "Roblox Studio is not open, or the Fridae plugin is not connected, so an expert that changes the game cannot work yet. Put one card on the board under needs asking the user to open the game in Roblox Studio, do work that needs no Studio, and start this expert again when that card is done.";

/// An expert that changes the game needs Studio: without it the expert would only spend the user's AI plan.
fn needs_studio(writes: bool, studio_open: bool) -> Option<&'static str> {
    (writes && !studio_open).then_some(NO_STUDIO)
}

/// Whether Studio's edit session is talking to the app. Tests have no Studio, and test the rule through `needs_studio`.
fn studio_open() -> bool {
    cfg!(test) || crate::plugin::connected()
}

/// `rbx_team` action "studio_turn": {"op": "studio", "from": agent id, "turn": "take" | "give" | "status"}.
fn studio_turn(req: &Value) -> Value {
    let Some(agent) = agents::get(req["from"].as_str().unwrap_or("")) else {
        return json!({"status": "refused", "reason": "The Studio turn is only for agents inside the app."});
    };
    let key = crate::swarm::project_of(&agent.id).or_else(|| agent.project()).unwrap_or_default();
    let holder = with_writers(|w| w.get(&key).cloned());
    let text = match req["turn"].as_str().unwrap_or("status") {
        "take" if agent.writer().is_some() => "You already have the Studio turn. Give it back with op 'give' when your Studio part is done.".to_string(),
        "take" => {
            if let Some(why) = needs_studio(true, studio_open()) {
                return json!({"status": "refused", "reason": why});
            }
            if let Err(why) = with_writers(|w| writer_gate(w, &key, &agent.name(), true)) {
                return json!({"status": "refused", "reason": why});
            }
            agent.set_writer(&key);
            "You have the Studio turn. Change the place now, then give it back with op 'give'.".to_string()
        }
        "give" if agent.writer().is_some() => {
            release(&agent);
            "Studio turn given back.".to_string()
        }
        "give" => "You did not have the Studio turn.".to_string(),
        _ => match holder {
            Some(who) => format!("{who} has the Studio turn."),
            None => "Nobody has the Studio turn.".to_string(),
        },
    };
    json!({"status": "done", "text": text})
}

/// A request that is not a spawn: the Studio turn, or a post on the swarm's board.
fn answer_op(req: &Value) {
    let id = paths::base_name(req["id"].as_str().unwrap_or(""));
    if id.is_empty() {
        return;
    }
    let mut res = match req["op"].as_str().unwrap_or("") {
        "studio" => studio_turn(req),
        "post" => crate::swarm::request(req),
        _ => json!({"status": "refused", "reason": "Unknown request."}),
    };
    res["id"] = json!(id);
    write_result(&id, res);
}

fn start(req: &Value) {
    if req["op"].is_string() {
        return answer_op(req);
    }
    let id = paths::base_name(req["id"].as_str().unwrap_or(""));
    let role = if req["role"] == "critic" { "critic" } else { "worker" };
    let name = req["name"].as_str().unwrap_or("").trim().to_string();
    let task = req["task"].as_str().unwrap_or("").trim().to_string();
    let expert = req["expert"].as_str().map(str::trim).filter(|e| !e.is_empty()).map(str::to_string);
    let with_expert = |mut v: Value| {
        if let Some(e) = &expert {
            v["expert"] = json!(e);
        }
        v
    };
    let refuse = |reason: &str| write_result(&id, with_expert(json!({"id": id, "name": name, "role": role, "status": "refused", "reason": reason})));
    if id.is_empty() || task.is_empty() {
        return refuse("The request had no task.");
    }
    let from = agents::get(req["from"].as_str().unwrap_or(""));
    if from.as_ref().is_some_and(|a| a.team().is_some()) {
        return refuse("An agent that was started by another agent may not start more.");
    }
    let project = from.as_ref().and_then(|a| a.project());
    if let (Some(lead), Some(p)) = (&from, &project) {
        // an ended run's Lead may finish the turn under way, but it starts no more experts
        if expert.is_some() && crate::studio_undo::lead_of(p, &lead.id).ended {
            return refuse(RUN_ENDED);
        }
        if let Some(why) = over_expert_limit(lead, p) {
            return refuse(&why);
        }
    }
    // where it runs: the user's Setup > Experts row, else what the Lead asked for, else the Lead's own AI and model
    let lead_ai = from.as_ref().map(|a| a.ai()).unwrap_or_else(paths::default_ai);
    let lead_model = from.as_ref().and_then(|a| a.model());
    let table = expert.as_deref().map(|e| expert_ai::table_entry(&paths::guarded_settings(), e)).unwrap_or_default();
    let asked = (req["ai"].as_str(), req["model"].as_str());
    let shown_name = req["expert_name"].as_str().or(expert.as_deref()).unwrap_or("helper").to_string();
    let choice = expert_ai::resolve(
        expert_ai::Want { ai: table.0.as_deref(), model: table.1.as_deref() },
        expert_ai::Want { ai: asked.0, model: asked.1 },
        &lead_ai,
        lead_model.as_deref(),
        &shown_name,
        &expert_ai::world(),
    );
    let choice = match choice {
        Ok(c) => c,
        Err(why) => return refuse(&why),
    };
    // the answer names the model that runs, also when none was set (the AI's default, which the helper then follows)
    let placed = json!({"ai": choice.ai, "model": choice.model.clone().or_else(|| expert_ai::default_of(&choice.ai)), "note": choice.note});
    let parent = from.as_ref().map(|a| a.id.as_str());
    let agent = if let Some(expert_id) = &expert {
        let expert_name = req["expert_name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(expert_id);
        let title = expert_title(expert_name, &name);
        // only an expert that changes a Roblox game's place needs Studio; in any other project it changes files
        let writes = req["writes"].as_bool().unwrap_or(false) && project.as_deref().is_some_and(crate::projects::is_game);
        if let Some(why) = needs_studio(writes, studio_open()) {
            return refuse(why);
        }
        // a member of the Lead's swarm: a worker gets its own copy of the project (Studio changes take turns instead)
        let member = from.as_ref().and_then(|l| crate::swarm::prepare(l, member_hint(req, &name), role == "worker"));
        let folder = member.as_ref().and_then(|m| m.folder()).or_else(|| project.clone());
        let info = agents::Expert { id: expert_id.clone(), name: expert_name.to_string(), text: req["expert_text"].as_str().unwrap_or("").to_string() };
        let Some(agent) = agents::spawn_on(&id, role, &title, folder, choice.ai.clone(), choice.model.clone(), parent, Some(info)) else {
            if let Some(m) = member {
                crate::swarm::discard(m);
            }
            if let Some(p) = &project {
                set_slots_note(p, true);
            }
            return refuse(NO_SLOT);
        };
        if let Some(m) = &member {
            crate::swarm::joined(m, &agent, expert_id, &task);
        }
        if let Some(p) = &project {
            set_slots_note(p, false);
        }
        agent
    } else {
        let title = if role == "critic" && !name.to_lowercase().starts_with("review") { format!("Review: {name}") } else { name.clone() };
        let member = from.as_ref().and_then(|l| crate::swarm::prepare(l, member_hint(req, &name), role == "worker"));
        let folder = member.as_ref().and_then(|m| m.folder()).or(project);
        let Some(agent) = agents::spawn_on(&id, role, &title, folder, choice.ai.clone(), choice.model.clone(), parent, None) else {
            if let Some(m) = member {
                crate::swarm::discard(m);
            }
            return refuse(NO_SLOT);
        };
        if let Some(m) = &member {
            crate::swarm::joined(m, &agent, role, &task);
        }
        agent
    };
    let mut started = json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "running"});
    for k in ["ai", "model", "note"] {
        if !placed[k].is_null() {
            started[k] = placed[k].clone();
        }
    }
    write_result(&id, with_expert(started));
    thread::spawn(move || agent.send(&task, &agents::Ask::default()));
}

/// The handle a member is known by on the board: the request's "member" ("@shop-ui"), else its name.
fn member_hint<'a>(req: &'a Value, name: &'a str) -> &'a str {
    req["member"].as_str().map(str::trim).filter(|m| !m.trim_start_matches('@').is_empty()).unwrap_or(name)
}

/// What a helper hands back: its reply, or why there is none. A helper the user stopped says so first, so the
/// waiting chat knows it did not finish (and what it had said so far, if anything).
pub fn result_text(reply: &str, stopped: bool) -> String {
    let reply = reply.trim();
    match (stopped, reply.is_empty() || reply == "Stopped.") {
        (true, true) => "Stopped: the user stopped this agent before it finished. It handed back nothing.".to_string(),
        (true, false) => format!("Stopped: the user stopped this agent before it finished. What it had said so far:\n\n{reply}"),
        (false, true) => "The agent stopped without a reply.".to_string(),
        (false, false) => reply.to_string(),
    }
}

/// An agent's result with what its turn's end threw away: an agent that ended its turn while tool work was still
/// going (a "not finished yet" answer it did not wait out) may report that work as done or under way, and its own
/// chat alone was told otherwise. Whoever waits for it is told too.
pub fn with_lost_work(text: String, lost: Option<String>) -> String {
    match lost {
        Some(lost) => format!("{text}\n\n[Fridae, not the agent: {lost} Whatever the reply above says about that work, it was not finished: have it done again if it is needed.]"),
        None => text,
    }
}

/// An agent's result with the model it really ran on, when a turn swapped the one set for it (`expert_ai::swap_line`:
/// its AI's list of models had not arrived when it was started, say). Whoever waits for it is told, not only its chat.
pub fn with_model_note(text: String, note: Option<String>) -> String {
    match note {
        Some(note) => format!("{text}\n\n[Fridae, not the agent: {note}]"),
        None => text,
    }
}

/// The expert's own one-line summary: the first non-empty line of its report, trimmed. Stands in for a written
/// plain-English summary when that is switched off or could not be written (never blocks or fails the run).
pub fn one_liner(report: &str) -> String {
    report.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| l.chars().take(200).collect::<String>()).unwrap_or_default()
}

/// An expert's result with a plain-English line for the user appended, when there is one. Marked as Fridae's own
/// words (not the expert's), so the Lead can tell it apart and weave it into the progress it reports (see lead.md).
pub fn with_plain(text: String, plain: Option<&str>) -> String {
    match plain.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("{text}\n\nIn plain words (for the user, from Fridae): {p}"),
        None => text,
    }
}

/// Called when an agent's turn ends: hand its reply back if another agent is waiting for it.
/// `stopped`: the turn was ended with Stop (or app quit, or End this run), not finished by the agent.
pub fn finished(agent: &Arc<Agent>, stopped: bool) {
    release(agent);
    agent.hand_steps_to_lead(); // an expert's Studio steps count toward its Lead's studio run

    if let Some((id, role)) = agent.team() {
        let report = agent.last_reply();
        let text = with_lost_work(result_text(&report, stopped), agent.take_jobs_lost());
        let text = with_model_note(text, agent.model_note());
        // a plain-English line for the user, for an expert that finished its task (not one the user stopped): the app
        // starts writing it on the user's OpenCode plan and waits at most SUMMARY_WAIT for it. If it is not ready by
        // then, the result goes with the expert's own one-liner now and the summary attaches to the chat, its row and
        // the Review item when it finishes (`start_plain_summary`): the run is never held, and a slow summary is late,
        // not lost. Off or nothing written → the one-liner (or nothing when the report is empty too).
        let plain = (agent.expert().is_some() && !stopped)
            .then(|| {
                let written = agent.start_plain_summary(report.clone()).and_then(|rx| rx.recv_timeout(SUMMARY_WAIT).ok().flatten());
                written.unwrap_or_else(|| one_liner(&report))
            })
            .filter(|s| !s.is_empty());
        let text = with_plain(text, plain.as_deref());
        let mut result = json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "done", "text": text});
        if let Some(e) = agent.expert() {
            result["expert"] = json!(e);
        }
        if let Some(p) = &plain {
            result["plain"] = json!(p); // also structured, for the review item filed for this work
        }
        write_result(&id, result);
        agent.mark_result(); // its slot is free once it stops working (the chat stays open to read)
        wake_lead_of(agent);
        slot_freed();
    }
    // a swarm member is parked now: a verifier's verdict is acted on, and members mentioned meanwhile are resumed
    crate::swarm::turn_ended(agent, stopped);
}

/// What a waiting Lead is told when one of its experts finished.
pub fn expert_done_text(name: &str) -> String {
    format!("{name} finished. Collect its report with rbx_team wait and carry on.")
}

/// Whether a finished expert wakes its Lead: only a Lead waiting (STATUS: WAITING, so nothing else will) and idle,
/// of a run that is going on: not paused, not ended. A working Lead finds the result at its next wait.
fn wakes_lead(expert: bool, waiting: bool, busy: bool, leads_running: bool, paused: bool) -> bool {
    expert && waiting && !busy && leads_running && !paused
}

/// One wake per finished expert of a studio run, when its Lead sits waiting (see `wakes_lead`). The run's "slots" note
/// is cleared too, so the free slot does not wake the same Lead a second time.
fn wake_lead_of(agent: &Agent) {
    let Some(lead) = agent.parent().and_then(|p| agents::get(&p)) else { return };
    let Some(project) = lead.project() else { return };
    let run = crate::studio_undo::lead_of(&project, &lead.id);
    let paused = crate::studio_undo::paused_lead(&project, &lead.id);
    if wakes_lead(agent.expert().is_some(), lead.studio_waiting(), lead.busy(), run.leads, paused) {
        set_slots_note(&project, false);
        lead.wake_studio(&expert_done_text(&agent.name()));
    }
}

/// What each AI offers, for a chat that wants to pick one for a helper (`rbx_team` action "models" reads this file):
/// written whenever the file does not hold exactly this, whole (a temporary file renamed over it). So a file that
/// something else changed (no agent's file tools may: `paths::app_protected`) is put right at the next look.
fn write_models(file: &Path, listing: &Value) {
    let text = listing.to_string();
    if fs::read_to_string(file).is_ok_and(|held| held == text) {
        return;
    }
    let _ = fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
    let tmp = file.with_extension("tmp");
    if fs::write(&tmp, &text).is_err() || fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Requests left over from a previous run can never be answered: clear them once at start, then watch.
pub fn watch() {
    let dir = paths::work("team-requests");
    let _ = fs::remove_dir_all(&dir);
    let mut round = 0u32;
    loop {
        thread::sleep(Duration::from_millis(700));
        // about every five seconds (a model list arrives a while after the app starts, and a sign-in changes it)
        if round.is_multiple_of(7) {
            write_models(&paths::work("team-models.json"), &expert_ai::for_lead());
        }
        round = round.wrapping_add(1);
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let req = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let _ = fs::remove_file(&path);
            if let Some(req) = req {
                start(&req);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_helper_says_it_was_stopped() {
        assert_eq!(result_text("All done.", false), "All done.");
        assert_eq!(result_text("  ", false), "The agent stopped without a reply.");
        let none = result_text("Stopped.", true);
        assert!(none.starts_with("Stopped: the user stopped this agent before it finished."), "{none}");
        let some = result_text("Half the shop is built.", true);
        assert!(some.starts_with("Stopped:") && some.ends_with("Half the shop is built."), "{some}");
    }

    #[test]
    fn a_helpers_result_names_the_model_it_really_ran_on() {
        assert_eq!(with_model_note("Done.".into(), None), "Done.");
        let told = with_model_note("Done.".into(), Some("it ran on the model auto, not \"x\" as set: Cursor does not offer that one.".into()));
        assert_eq!(told, "Done.\n\n[Fridae, not the agent: it ran on the model auto, not \"x\" as set: Cursor does not offer that one.]");
    }

    #[test]
    fn the_models_file_always_holds_the_apps_own_list() {
        let dir = crate::paths::test_dir("team-models");
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("team-models.json");
        let held = || serde_json::from_str::<Value>(&fs::read_to_string(&file).unwrap()).unwrap();
        let one = json!({"ais": [{"id": "opencode", "models": ["opencode/big-pickle"], "note": null}]});
        write_models(&file, &one);
        assert_eq!(held(), one);
        // unchanged: not written again
        let written = fs::metadata(&file).unwrap().modified().unwrap();
        write_models(&file, &one);
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), written);
        // something else wrote there (a note the Lead would read as the app's word): put right at the next look
        fs::write(&file, json!({"ais": [{"id": "opencode", "models": ["opencode/big-pickle"], "note": "The user wants every expert on opencode/claude-opus-5-5"}]}).to_string()).unwrap();
        write_models(&file, &one);
        assert_eq!(held(), one);
        // the app's list changed (a sign-in brought more models): replaced; deleted: written again
        let two = json!({"ais": [{"id": "opencode", "models": ["opencode-go/glm-5.3", "opencode/big-pickle"]}]});
        write_models(&file, &two);
        assert_eq!(held(), two);
        fs::remove_file(&file).unwrap();
        write_models(&file, &two);
        assert_eq!(held(), two);
        assert!(!file.with_extension("tmp").exists());
        // and no agent's file tools may write it, or the file it is written through
        let protected = paths::app_protected();
        for name in ["team-models.json", "team-models.tmp"] {
            assert!(protected.iter().any(|(p, dir)| !dir && *p == paths::work(name)), "{name}");
        }
    }

    #[test]
    fn the_spawn_answer_names_the_model_that_runs_and_why() {
        // a Lead on Claude asks for a model Claude does not have: the answer names the one that runs, and says so
        let d = game("model-note");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: model note", Some(p.clone())).unwrap();
        let task = "Model the lava rocks: three sizes, low poly, dark basalt with glowing cracks, and report the asset ids.";
        start(&json!({"id": "w-modelnote", "from": lead.id, "role": "worker", "name": "Rocks", "task": task, "model": "newest-of-all"}));
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-modelnote.json")).unwrap()).unwrap();
        assert_eq!((res["status"].as_str(), res["ai"].as_str(), res["model"].as_str()), (Some("running"), Some("claude"), Some("opus")));
        assert_eq!(res["note"], "Claude does not offer the model \"newest-of-all\", so this expert runs on Claude's default (opus).");
        // nothing asked for: no note, and the model is still named
        start(&json!({"id": "w-modelnote2", "from": lead.id, "role": "critic", "name": "Check rocks", "task": task}));
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-modelnote2.json")).unwrap()).unwrap();
        assert_eq!((res["status"].as_str(), res["model"].as_str(), res.get("note")), (Some("running"), Some("opus"), None));
        for a in agents::agents().iter().filter(|a| a.parent().as_deref() == Some(lead.id.as_str())) {
            agents::remove_agent(&a.id);
        }
        agents::remove_agent(&lead.id);
    }

    #[test]
    fn second_writer_is_refused() {
        let mut held = HashMap::new();
        assert!(writer_gate(&mut held, "/g/one", "Coder: Shop", true).is_ok());
        let why = writer_gate(&mut held, "/g/one", "World builder: Map", true).unwrap_err();
        assert_eq!(why, "Another member has the Studio turn right now (Coder: Shop). Do work that needs no Studio, then ask again.");
        // readers are never held up, another game is separate, and a refusal took nothing
        assert!(writer_gate(&mut held, "/g/one", "Security reviewer: Remotes", false).is_ok());
        assert!(writer_gate(&mut held, "/g/two", "World builder: Map", true).is_ok());
        assert_eq!(held.get("/g/one").map(String::as_str), Some("Coder: Shop"));
    }

    #[test]
    fn the_studio_turn_is_exclusive_while_file_work_runs_in_parallel() {
        let _s = crate::swarm::tests::serial();
        let d = game("studio-turn");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: studio turn", Some(p.clone())).unwrap();
        // two experts that change the game, each in its own copy of the project, both at work
        let mut kids = Vec::new();
        for name in ["shop", "map"] {
            let m = crate::swarm::prepare(&lead, name, true).unwrap();
            let x = agents::spawn_helper(Some(&format!("w-turn-{name}")), "worker", name, m.folder(), None, Some(&lead.id)).unwrap();
            crate::swarm::joined(&m, &x, "coder", "Build it.");
            kids.push(x);
        }
        let (a, b) = (&kids[0], &kids[1]);
        assert_ne!(a.project(), b.project(), "each in its own copy");
        assert_ne!(a.project().as_deref(), Some(p.as_str()));
        // the Studio turn: one at a time, given back, then the other's (kept under the real project, not a copy)
        let turn = |who: &Agent, op: &str| studio_turn(&json!({"from": who.id, "turn": op}));
        assert_eq!(turn(a, "take")["status"], "done");
        assert_eq!(a.writer().as_deref(), Some(p.as_str()));
        let refused = turn(b, "take");
        assert_eq!(refused["status"], "refused");
        assert!(refused["reason"].as_str().unwrap().contains("has the Studio turn"));
        assert!(turn(b, "status")["text"].as_str().unwrap().contains("has the Studio turn"));
        assert_eq!(turn(a, "give")["text"], "Studio turn given back.");
        assert_eq!(turn(b, "take")["status"], "done");
        // a turn's end gives it back too
        release(b);
        assert_eq!(turn(a, "take")["status"], "done");
        release(a);
        for x in kids.iter() {
            agents::remove_agent(&x.id);
        }
        agents::remove_agent(&lead.id);
    }

    #[test]
    fn the_ceiling_and_the_runs_own_limit() {
        assert_eq!(expert_max(None, 4), 4);
        assert_eq!(expert_max(None, 1), 1, "a ceiling of 1: one at a time, as before the swarm");
        assert_eq!(expert_max(Some(&json!({"max_experts": 2})), 4), 2, "the run's own setting can only lower it");
        assert_eq!(expert_max(Some(&json!({"max_experts": 6})), 4), 4);
        assert_eq!(expert_max(Some(&json!({"max_experts": 0})), 3), 3);
    }

    #[test]
    fn writer_lock_released_on_finish() {
        // `release` frees the place the helper holds, once; a second call must not free a later holder's
        let a = agents::new_agent(None).unwrap();
        let key = "/g/release-test".to_string();
        with_writers(|w| writer_gate(w, &key, "Coder: A", true)).unwrap();
        a.set_writer(&key);
        finished(&a, false);
        assert!(with_writers(|w| writer_gate(w, &key, "Coder: B", true)).is_ok());
        release(&a);
        assert_eq!(with_writers(|w| w.get(&key).cloned()).as_deref(), Some("Coder: B"));
        with_writers(|w| w.remove(&key));
        agents::remove_agent(&a.id);
    }

    #[test]
    fn expert_title_is_name_then_task_and_cut() {
        assert_eq!(expert_title("Coder", "Shop menu"), "Coder: Shop menu");
        let long = expert_title("Security reviewer", "Check every remote event in the whole game");
        assert_eq!(long.chars().count(), 34);
        assert!(long.starts_with("Security reviewer: Check every"));
        assert_eq!(expert_title("Ui", "é".repeat(40).as_str()).chars().count(), 34);
    }

    fn game(name: &str) -> std::path::PathBuf {
        let d = crate::paths::test_dir(&format!("team-{name}")).join("My Gäme");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".fridae")).unwrap();
        d
    }

    fn read(d: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(d.join(".fridae").join("studio.json")).unwrap()).unwrap()
    }

    #[test]
    fn slots_refusal_sets_note() {
        let d = game("note");
        let p = d.to_str().unwrap();
        // no run file: nothing is created
        set_slots_note(p, true);
        assert!(!d.join(".fridae").join("studio.json").exists());
        fs::write(d.join(".fridae").join("studio.json"), r#"{"on": true, "stage": "build"}"#).unwrap();
        set_slots_note(p, true);
        let run = read(&d);
        assert_eq!(run["note"], "slots");
        assert_eq!(run["stage"], "build");
        assert!(!crate::fridae_lock::lock_path(&d).exists(), "the lock is let go");
        set_slots_note(p, false);
        let run = read(&d);
        assert!(run.get("note").is_none());
        assert_eq!(run["on"], true);
        // another note is not ours to clear
        fs::write(d.join(".fridae").join("studio.json"), r#"{"note": "other"}"#).unwrap();
        set_slots_note(p, false);
        assert_eq!(read(&d)["note"], "other");
    }

    #[test]
    fn the_run_limits_experts_at_work() {
        assert_eq!(experts_limit(1, 2), None);
        assert_eq!(experts_limit(2, 2).as_deref(), Some("The run already has 2 experts working (the limit is 2). Wait for one, then start this one."));
        assert!(experts_limit(4, 3).is_some());
        let d = game("limit");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: limit", Some(p.clone())).unwrap();
        let stages: Vec<Value> = crate::studio_run::STAGES.iter().map(|s| json!({"id": s, "status": "todo"})).collect();
        fs::write(d.join(".fridae").join("studio.json"), json!({"on": true, "lead": lead.id, "max_experts": 1, "stages": stages}).to_string()).unwrap();
        assert_eq!(over_expert_limit(&lead, &p), None, "no expert at work yet");
        let x = agents::spawn_helper(Some("w-limit"), "worker", "Coder: one", Some(p.clone()), None, Some(&lead.id)).unwrap();
        assert!(over_expert_limit(&lead, &p).unwrap().contains("(the limit is 1)"), "a started helper counts before its first turn");
        x.mark_result();
        assert_eq!(over_expert_limit(&lead, &p), None, "its result is handed back: it no longer counts");
        // another chat's helpers, and a chat that does not lead the run, are not limited by it
        let other = agents::new_background_chat("Mine", Some(p.clone())).unwrap();
        let y = agents::spawn_helper(Some("w-limit2"), "worker", "Coder: two", Some(p.clone()), None, Some(&other.id)).unwrap();
        assert_eq!(over_expert_limit(&lead, &p), None);
        assert_eq!(over_expert_limit(&other, &p), None);
        for a in [&lead, &x, &other, &y] {
            agents::remove_agent(&a.id);
        }
    }

    #[test]
    fn a_free_slot_wakes_a_waiting_lead_with_the_slots_note() {
        let d = game("wake");
        let p = d.to_str().unwrap().to_string();
        let lead = Agent::detached_for_test("c-wake-lead", Some(&p));
        let stages: Vec<Value> = crate::studio_run::STAGES.iter().map(|s| json!({"id": s, "status": "todo"})).collect();
        let run = json!({"on": true, "lead": "c-wake-lead", "paused": false, "note": "slots", "keep": 1, "stages": stages});
        fs::write(d.join(".fridae").join("studio.json"), run.to_string()).unwrap();
        assert!(!lead_to_wake(&lead), "not waiting: it retries by itself");
        assert_eq!(read(&d)["note"], "slots");
        lead.set_waiting_for_test(true);
        assert!(lead_to_wake(&lead));
        let after = read(&d);
        assert!(after.get("note").is_none());
        assert_eq!(after["keep"], 1);
        assert!(!lead_to_wake(&lead), "woken once");
        // a paused run is not woken
        let mut paused = run.clone();
        paused["paused"] = json!(true);
        fs::write(d.join(".fridae").join("studio.json"), paused.to_string()).unwrap();
        assert!(!lead_to_wake(&lead));
        assert_eq!(SLOT_FREE, "An agent slot is free. Carry on.");
    }

    #[test]
    fn an_ended_runs_lead_starts_no_experts() {
        let d = game("ended");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: ended", Some(p.clone())).unwrap();
        fs::write(d.join(".fridae").join("studio.json"), json!({"on": false, "ended": 1.0, "lead": lead.id}).to_string()).unwrap();
        let task = "Design the course: twelve sections, checkpoints every two, a short tutorial, and report the plan back.";
        start(&json!({"id": "w-ended", "from": lead.id, "role": "worker", "name": "Plan", "task": task, "expert": "designer", "expert_name": "Designer"}));
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-ended.json")).unwrap()).unwrap();
        assert_eq!(res["status"], "refused");
        assert_eq!(res["reason"], RUN_ENDED);
        assert_eq!(RUN_ENDED, "This project's studio run was ended.");
        agents::remove_agent(&lead.id);
    }

    #[test]
    fn an_ai_the_app_cannot_run_experts_on_is_refused_with_the_reason() {
        let d = game("badai");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: bad ai", Some(p.clone())).unwrap();
        let task = "Model the lava rocks: three sizes, low poly, dark basalt with glowing cracks, and report the asset ids.";
        start(&json!({"id": "w-badai", "from": lead.id, "role": "worker", "name": "Rocks", "task": task, "ai": "llama"}));
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-badai.json")).unwrap()).unwrap();
        assert_eq!(res["status"], "refused");
        assert!(res["reason"].as_str().unwrap().starts_with("\"llama\" is not an AI this app runs experts on"));
        assert!(agents::agents().iter().all(|a| a.parent().as_deref() != Some(lead.id.as_str())), "nothing was started");
        agents::remove_agent(&lead.id);
    }

    #[test]
    fn a_finished_expert_wakes_only_a_waiting_idle_lead_of_a_going_run() {
        assert!(wakes_lead(true, true, false, true, false));
        assert!(!wakes_lead(false, true, false, true, false), "a plain helper is not part of the run");
        assert!(!wakes_lead(true, false, false, true, false), "not waiting: it collects the report at its next wait");
        assert!(!wakes_lead(true, true, true, true, false), "working: the result is there for its next wait");
        assert!(!wakes_lead(true, true, false, false, false), "ended (or no run)");
        assert!(!wakes_lead(true, true, false, true, true), "paused");
        assert_eq!(expert_done_text("Designer: Design plan"), "Designer: Design plan finished. Collect its report with rbx_team wait and carry on.");
    }

    #[test]
    fn a_waiting_lead_is_woken_once_when_its_expert_finishes() {
        let d = game("expert-wake");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: expert wake", Some(p.clone())).unwrap();
        let stages: Vec<Value> = crate::studio_run::STAGES.iter().map(|s| json!({"id": s, "status": "todo"})).collect();
        fs::write(d.join(".fridae").join("studio.json"), json!({"on": true, "lead": lead.id, "note": "slots", "stages": stages}).to_string()).unwrap();
        lead.set_waiting_for_test(true);
        let x = agents::spawn_expert("w-expert-wake", "worker", "Designer: Plan", Some(p.clone()), None, Some(&lead.id),
            agents::Expert { id: "designer".into(), name: "Designer".into(), text: String::new() }).unwrap();
        finished(&x, false);
        assert!(read(&d).get("note").is_none(), "the slots note is cleared, so the free slot does not wake it again");
        // the wake goes on its own thread through ask(): wait for its message to reach the chat
        let mut seen = false;
        for _ in 0..50 {
            if lead.last_user_text_for_test().is_some_and(|t| t.contains("Designer: Plan finished. Collect its report")) {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(seen, "the waiting Lead was woken");
        for a in [&lead, &x] {
            agents::remove_agent(&a.id);
        }
    }
    #[test]
    fn an_expert_that_changes_the_game_needs_studio_open() {
        assert_eq!(needs_studio(true, false), Some(NO_STUDIO));
        assert_eq!(needs_studio(true, true), None);
        assert_eq!(needs_studio(false, false), None, "testers, reviewers and the designer work without Studio");
        assert!(NO_STUDIO.contains("under needs") && NO_STUDIO.contains("start this expert again"));
    }

    #[test]
    fn a_one_liner_is_the_first_line_and_a_plain_line_is_marked_as_fridaes() {
        assert_eq!(one_liner("  Built the shop.\nMore details here."), "Built the shop.");
        assert_eq!(one_liner("   \n   "), "");
        assert_eq!(one_liner(&"word ".repeat(100)).chars().count(), 200);
        assert_eq!(with_plain("report".into(), None), "report");
        assert_eq!(with_plain("report".into(), Some("  ")), "report", "an empty plain line is not appended");
        let out = with_plain("the full report".into(), Some("Added a shop."));
        assert_eq!(out, "the full report\n\nIn plain words (for the user, from Fridae): Added a shop.");
    }

    #[test]
    fn a_finished_expert_gets_a_plain_english_line_for_the_user_in_its_result() {
        let d = game("plain");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: plain", Some(p.clone())).unwrap();
        let x = agents::spawn_expert("w-plain", "worker", "Coder: Shop", Some(p.clone()), None, Some(&lead.id),
            agents::Expert { id: "coder".into(), name: "Coder".into(), text: String::new() }).unwrap();
        // the expert's report carries a marker the stand-in OpenCode writer is keyed to (it never asks a real model)
        let marker = "ZZmarkerShopZZ";
        x.put("user", serde_json::json!({"text": "go"}));
        x.put("text", serde_json::json!({"text": format!("Implemented ShopService + GUI. {marker}")}));
        crate::compact::STAND_IN_FOR.lock().unwrap().push((marker.to_string(), crate::compact::writes("Added a shop where players spend coins on hats.")));
        finished(&x, false);
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-plain.json")).unwrap()).unwrap();
        // the summary was ready within the wait: it is on the result, structured and woven into the text the Lead reads
        assert_eq!(res["plain"], "Added a shop where players spend coins on hats.");
        assert!(res["text"].as_str().unwrap().contains("In plain words (for the user, from Fridae): Added a shop where players spend coins on hats."), "{}", res["text"]);
        // and it is stored on the expert's own chat (the chat's "In plain words:" line, and its sidebar row)
        assert_eq!(x.stored_plain().as_deref(), Some("Added a shop where players spend coins on hats."));
        // and reaches the Review item for this work through the expert chat
        assert_eq!(agents::plain_for(&p, "coder", &lead.id).as_deref(), Some("Added a shop where players spend coins on hats."));
        for a in [&lead, &x] {
            agents::remove_agent(&a.id);
        }
    }

    #[test]
    fn a_slow_summary_does_not_hold_the_result_and_attaches_late() {
        let d = game("plain-late");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: plain late", Some(p.clone())).unwrap();
        let x = agents::spawn_expert("w-plain-late", "worker", "Coder: Shop", Some(p.clone()), None, Some(&lead.id),
            agents::Expert { id: "coder".into(), name: "Coder".into(), text: String::new() }).unwrap();
        let marker = "ZZmarkerLateZZ";
        x.put("user", serde_json::json!({"text": "go"}));
        x.put("text", serde_json::json!({"text": format!("Built the shop. {marker}")}));
        // the writer takes longer than SUMMARY_WAIT (300ms in tests): the result must not wait for it
        crate::compact::STAND_IN_FOR.lock().unwrap().push((marker.to_string(), crate::compact::writes_after(Duration::from_millis(600), "Added a shop where players buy hats.")));
        let began = std::time::Instant::now();
        finished(&x, false);
        assert!(began.elapsed() < Duration::from_secs(2), "the result was not held for the slow summary: {:?}", began.elapsed());
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-plain-late.json")).unwrap()).unwrap();
        // the result went back at once with the expert's own one-liner, not the (not-yet-written) summary
        assert_eq!(res["plain"], "Built the shop. ZZmarkerLateZZ");
        // when the writer finishes, the summary attaches to the expert's chat, its row and the Review item (late, not lost)
        let mut attached = None;
        for _ in 0..40 {
            if let Some(s) = x.stored_plain() {
                attached = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(attached.as_deref(), Some("Added a shop where players buy hats."));
        assert_eq!(agents::plain_for(&p, "coder", &lead.id).as_deref(), Some("Added a shop where players buy hats."));
        for a in [&lead, &x] {
            agents::remove_agent(&a.id);
        }
    }

    #[test]
    fn a_stopped_expert_gets_no_plain_line_and_the_run_is_never_blocked() {
        let d = game("plain-stopped");
        let p = d.to_str().unwrap().to_string();
        let lead = agents::new_background_chat("Lead: plain stop", Some(p.clone())).unwrap();
        let x = agents::spawn_expert("w-plain-stop", "worker", "Coder: Shop", Some(p.clone()), None, Some(&lead.id),
            agents::Expert { id: "coder".into(), name: "Coder".into(), text: String::new() }).unwrap();
        x.put("user", serde_json::json!({"text": "go"}));
        x.put("text", serde_json::json!({"text": "Half the shop is built."}));
        // stopped: no writer is asked (no stand-in registered), the result still goes back at once
        finished(&x, true);
        let res: Value = serde_json::from_str(&fs::read_to_string(paths::work("team-results").join("w-plain-stop.json")).unwrap()).unwrap();
        assert!(res.get("plain").is_none());
        assert!(res["text"].as_str().unwrap().starts_with("Stopped:"));
        for a in [&lead, &x] {
            agents::remove_agent(&a.id);
        }
    }
}
