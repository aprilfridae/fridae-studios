//! Agents starting other agents. A tool writes a request file; this picks it up,
//! starts a worker or a reviewer in its own chat, and writes the outcome back.
//!
//!   work/team-requests/<id>.json   {"id", "from", "role", "name", "task"}
//!   work/team-results/<id>.json    {"id", "agent", "name", "role", "status", "reason", "text"}
//!
//! Studio mode adds a named expert: the request also has "expert", "expert_name", "expert_text" and "writes"
//! (the tool read them from the roster; this never does), and the result "expert". Only one expert that changes the
//! game ("writes") runs per project at a time.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::Mutex,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    agents::{self, Agent},
    paths, studio_run,
};

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

/// Take the project's one writer place for `agent_name`, or say who has it. Experts that only read or review never need it.
fn writer_gate(held: &mut HashMap<String, String>, project: &str, agent_name: &str, writes: bool) -> Result<(), String> {
    if !writes {
        return Ok(());
    }
    if let Some(who) = held.get(project) {
        return Err(format!("Another expert is changing the game right now ({who}). Wait for it, then start this one."));
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

/// The run's limit on experts working at once (studio.json "max_experts", 1 to 4, 2 when unset). None when there is
/// room for one more.
fn experts_limit(working: usize, max: u64) -> Option<String> {
    (working as u64 >= max).then(|| format!("The run already has {working} experts working (the limit is {max}). Wait for one, then start this one."))
}

/// For a request from the Lead of a running studio run: refused when as many of its helpers as the run allows are still
/// at work (a helper counts until its result is handed back, or again while the user has it working).
fn over_expert_limit(lead: &Agent, project: &str) -> Option<String> {
    let run = studio_run::read_run(Path::new(project)).filter(|r| studio_run::running(r) && r["lead"] == lead.id.as_str())?;
    let max = run["max_experts"].as_u64().filter(|n| (1..=4).contains(n)).unwrap_or(2);
    let working = agents::agents().iter().filter(|a| a.parent().as_deref() == Some(lead.id.as_str()) && a.team().is_some() && a.holds_slot()).count();
    experts_limit(working, max)
}

pub const SLOT_FREE: &str = "An agent slot is free. Carry on.";

/// A slot came free (a helper finished, or a chat was closed): every waiting Lead whose run has the note "slots" gets
/// the note cleared and is woken with SLOT_FREE.
pub fn slot_freed() {
    for lead in agents::agents() {
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
pub const RUN_ENDED: &str = "This game's studio run was ended.";

fn start(req: &Value) {
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
    let ai = from.as_ref().map(|a| a.ai());
    let parent = from.as_ref().map(|a| a.id.as_str());
    let agent = if let Some(expert_id) = &expert {
        let expert_name = req["expert_name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(expert_id);
        let title = expert_title(expert_name, &name);
        let key = project.clone().unwrap_or_default();
        let writes = req["writes"].as_bool().unwrap_or(false);
        if let Err(why) = with_writers(|w| writer_gate(w, &key, &title, writes)) {
            return refuse(&why); // a refused request never takes the place
        }
        let info = agents::Expert { id: expert_id.clone(), name: expert_name.to_string(), text: req["expert_text"].as_str().unwrap_or("").to_string() };
        let Some(agent) = agents::spawn_expert(&id, role, &title, project.clone(), ai, parent, info) else {
            if writes {
                with_writers(|w| w.remove(&key));
            }
            if let Some(p) = &project {
                set_slots_note(p, true);
            }
            return refuse(NO_SLOT);
        };
        if writes {
            agent.set_writer(&key);
        }
        if let Some(p) = &project {
            set_slots_note(p, false);
        }
        agent
    } else {
        let title = if role == "critic" && !name.to_lowercase().starts_with("review") { format!("Review: {name}") } else { name.clone() };
        let Some(agent) = agents::spawn_helper(Some(&id), role, &title, project, ai, parent) else {
            return refuse(NO_SLOT);
        };
        agent
    };
    write_result(&id, with_expert(json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "running"})));
    thread::spawn(move || agent.send(&task, &agents::Ask::default()));
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

/// Called when an agent's turn ends: hand its reply back if another agent is waiting for it.
/// `stopped`: the turn was ended with Stop (or app quit, or End this run), not finished by the agent.
pub fn finished(agent: &Agent, stopped: bool) {
    release(agent);
    agent.hand_steps_to_lead(); // an expert's Studio steps count toward its Lead's studio run

    if let Some((id, role)) = agent.team() {
        let text = result_text(&agent.last_reply(), stopped);
        let mut result = json!({"id": id, "agent": agent.id, "name": agent.name(), "role": role, "status": "done", "text": text});
        if let Some(e) = agent.expert() {
            result["expert"] = json!(e);
        }
        write_result(&id, result);
        agent.mark_result(); // its slot is free once it stops working (the chat stays open to read)
        wake_lead_of(agent);
        slot_freed();
    }
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

/// Requests left over from a previous run can never be answered: clear them once at start, then watch.
pub fn watch() {
    let dir = paths::work("team-requests");
    let _ = fs::remove_dir_all(&dir);
    loop {
        thread::sleep(Duration::from_millis(700));
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
    fn second_writer_is_refused() {
        let mut held = HashMap::new();
        assert!(writer_gate(&mut held, "/g/one", "Coder: Shop", true).is_ok());
        let why = writer_gate(&mut held, "/g/one", "World builder: Map", true).unwrap_err();
        assert_eq!(why, "Another expert is changing the game right now (Coder: Shop). Wait for it, then start this one.");
        // readers are never held up, another game is separate, and a refusal took nothing
        assert!(writer_gate(&mut held, "/g/one", "Security reviewer: Remotes", false).is_ok());
        assert!(writer_gate(&mut held, "/g/two", "World builder: Map", true).is_ok());
        assert_eq!(held.get("/g/one").map(String::as_str), Some("Coder: Shop"));
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
        assert_eq!(RUN_ENDED, "This game's studio run was ended.");
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
}
