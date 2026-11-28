//! The Allow/Don't card: a change the user must agree to waits in the calling chat until they answer (Fridae's own;
//! the holding of the call is modelled on T3 Code's request_secret).
//!
//! The card goes to the page as an `appcard` event on the chat; the answer comes back from the page over
//! `POST /api/app-card`, never through the agent. A card's id is 16 random bytes and is bound to its chat.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::json;

use super::{Failure, fail};
use crate::agents;

/// What the card says: a title naming the action ("Rename project"), one plain sentence, and the allow button's words
/// ("Rename it"; the quieter button is always "Don't").
#[derive(Clone, Debug, PartialEq)]
pub struct CardSpec {
    pub title: String,
    pub text: String,
    pub allow: String,
}

/// How a card ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    Allowed,
    Declined,
    /// The run ended or the client gave up on the call (or its session ended).
    Cancelled,
    /// No answer in time: not the same as no.
    TimedOut,
}

impl Answer {
    /// The `appcard_done` outcome.
    pub fn outcome(self) -> &'static str {
        match self {
            Answer::Allowed => "allowed",
            Answer::Declined => "declined",
            Answer::Cancelled => "cancelled",
            Answer::TimedOut => "timed_out",
        }
    }
}

/// How often a waiting card looks for its answer.
pub const POLL: Duration = Duration::from_millis(500);
/// How long a card waits (client tool timeouts are 65 minutes, so this always ends first).
pub const WAIT: Duration = Duration::from_secs(600);
/// Gemini's agy cuts any tool call at 3 minutes, so its cards close a little before.
pub const WAIT_GEMINI: Duration = Duration::from_secs(170);

fn poll() -> Duration {
    if cfg!(test) { Duration::from_millis(20) } else { POLL }
}

/// How long a card for a chat on `ai` waits. Tests: 2 s, and 600 ms for Gemini.
pub fn wait_for(ai: &str) -> Duration {
    match (cfg!(test), ai == "gemini") {
        (false, false) => WAIT,
        (false, true) => WAIT_GEMINI,
        (true, false) => Duration::from_secs(2),
        (true, true) => Duration::from_millis(600),
    }
}

/// Waiting cards: card id → (its chat, the answer once given).
static PENDING: Mutex<Option<HashMap<String, (String, Option<bool>)>>> = Mutex::new(None);

fn pending<R>(f: impl FnOnce(&mut HashMap<String, (String, Option<bool>)>) -> R) -> R {
    f(PENDING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Shows the card in the chat and holds the call until the user answers, the run ends, the call is cancelled, or the
/// wait runs out. Always closes the card (`appcard_done`).
pub fn ask(agent: &Arc<agents::Agent>, ai: &str, spec: CardSpec, cancelled: &AtomicBool) -> Answer {
    let card = crate::remote_common::random_hex(16);
    pending(|p| p.insert(card.clone(), (agent.id.clone(), None)));
    agent.put("appcard", json!({"card": card, "title": spec.title, "text": spec.text, "allow": spec.allow}));
    let (started, wait) = (Instant::now(), wait_for(ai));
    let answer = loop {
        match pending(|p| p.get(&card).and_then(|(_, a)| *a)) {
            Some(true) => break Answer::Allowed,
            Some(false) => break Answer::Declined,
            None => {}
        }
        if cancelled.load(Ordering::SeqCst) || !agent.is_working() || agent.is_closed() {
            break Answer::Cancelled;
        }
        if started.elapsed() >= wait {
            break Answer::TimedOut;
        }
        std::thread::sleep(poll());
    };
    pending(|p| p.remove(&card));
    agent.put("appcard_done", json!({"card": card, "title": spec.title, "outcome": answer.outcome()}));
    answer
}

/// The user pressed a card's button (`POST /api/app-card`). False when no such card waits in that chat.
pub fn answer(agent_id: &str, card_id: &str, allow: bool) -> bool {
    pending(|p| match p.get_mut(card_id) {
        Some((chat, slot @ None)) if chat == agent_id => {
            *slot = Some(allow);
            true
        }
        _ => false,
    })
}

/// What the agent is told when a card did not end in a yes.
pub fn failure_of(a: &Answer) -> Option<Failure> {
    match a {
        Answer::Allowed => None,
        Answer::Declined => Some(fail("declined", "The user declined. Don't ask again in chat; say what's blocked and go on without it.")),
        Answer::Cancelled => Some(fail("cancelled", "The card closed with the run. Ask again with a new clientRequestId only if it is still needed.")),
        Answer::TimedOut => Some(fail("timed_out", "No answer in time. Don't ask in chat; ask again with a new clientRequestId only if it is still needed.")),
    }
}

#[cfg(test)]
pub(crate) fn pending_for_test(card_id: &str) -> bool {
    pending(|p| p.contains_key(card_id))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app_mcp::{drop_chat, test_chat};
    use serde_json::Value;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Instant;

    /// The chat's newest event of `kind` that names `card` (any card when None), waiting up to 5 s for it.
    pub(crate) fn wait_event(agent: &agents::Agent, kind: &str, card: Option<&str>) -> Value {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let found = agent.log_snapshot().into_iter().rev().find(|e| e["kind"] == kind && card.is_none_or(|c| e["card"] == c));
            if let Some(e) = found {
                return e;
            }
            assert!(Instant::now() < until, "no {kind} event");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn spec() -> CardSpec {
        CardSpec { title: "Rename project".into(), text: "Rename \"A\" to \"B\". Its folder keeps its own name.".into(), allow: "Rename it".into() }
    }

    /// `ask` on another thread, as a tool call would; returns its handle and the cancel flag.
    fn asking(agent: &Arc<agents::Agent>, ai: &'static str) -> (thread::JoinHandle<Answer>, Arc<AtomicBool>) {
        let (a, flag) = (agent.clone(), Arc::new(AtomicBool::new(false)));
        let f = flag.clone();
        (thread::spawn(move || ask(&a, ai, spec(), &f)), flag)
    }

    #[test]
    fn allow_returns_allowed_and_logs_both_events() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let (h, _) = asking(&a, "claude");
        let card = wait_event(&a, "appcard", None);
        assert_eq!((card["title"].as_str(), card["allow"].as_str()), (Some("Rename project"), Some("Rename it")));
        assert_eq!(card["text"], spec().text);
        let id = card["card"].as_str().unwrap().to_string();
        assert!(id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(answer(&a.id, &id, true));
        assert_eq!(h.join().unwrap(), Answer::Allowed);
        let done = wait_event(&a, "appcard_done", Some(&id));
        assert_eq!((done["outcome"].as_str(), done["title"].as_str()), (Some("allowed"), Some("Rename project")));
        assert!(!answer(&a.id, &id, true), "an answered card is gone");
        drop_chat(&a);
    }

    #[test]
    fn a_card_is_bound_to_its_chat() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let (h, _) = asking(&a, "claude");
        let id = wait_event(&a, "appcard", None)["card"].as_str().unwrap().to_string();
        assert!(!answer("other-chat", &id, true));
        assert!(!answer(&a.id, "0123456789abcdef0123456789abcdef", true));
        assert!(pending_for_test(&id), "still waiting");
        assert!(answer(&a.id, &id, false));
        assert_eq!(h.join().unwrap(), Answer::Declined);
        drop_chat(&a);
    }

    #[test]
    fn decline() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let (h, _) = asking(&a, "claude");
        let id = wait_event(&a, "appcard", None)["card"].as_str().unwrap().to_string();
        assert!(answer(&a.id, &id, false));
        assert_eq!(h.join().unwrap(), Answer::Declined);
        assert_eq!(wait_event(&a, "appcard_done", Some(&id))["outcome"], "declined");
        drop_chat(&a);
    }

    #[test]
    fn the_run_ending_cancels() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let (h, _) = asking(&a, "claude");
        let id = wait_event(&a, "appcard", None)["card"].as_str().unwrap().to_string();
        a.set_running_for_test(false);
        assert_eq!(h.join().unwrap(), Answer::Cancelled);
        assert_eq!(wait_event(&a, "appcard_done", Some(&id))["outcome"], "cancelled");
        assert!(!pending_for_test(&id));
        drop_chat(&a);
    }

    #[test]
    fn mcp_cancel_cancels() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let (h, flag) = asking(&a, "claude");
        let id = wait_event(&a, "appcard", None)["card"].as_str().unwrap().to_string();
        flag.store(true, Ordering::SeqCst);
        assert_eq!(h.join().unwrap(), Answer::Cancelled);
        assert_eq!(wait_event(&a, "appcard_done", Some(&id))["outcome"], "cancelled");
        drop_chat(&a);
    }

    #[test]
    fn timing_out_is_not_declining() {
        let a = test_chat("claude");
        a.set_running_for_test(true);
        let started = Instant::now();
        let (h, _) = asking(&a, "claude");
        assert_eq!(h.join().unwrap(), Answer::TimedOut);
        assert!(started.elapsed() >= wait_for("claude"));
        assert_eq!(wait_event(&a, "appcard_done", None)["outcome"], "timed_out");
        drop_chat(&a);
    }

    #[test]
    fn gemini_waits_less() {
        assert!(WAIT_GEMINI < WAIT);
        assert_eq!((WAIT, WAIT_GEMINI, POLL), (Duration::from_secs(600), Duration::from_secs(170), Duration::from_millis(500)));
        assert!(wait_for("gemini") < wait_for("claude"));
        let a = test_chat("gemini");
        a.set_running_for_test(true);
        let started = Instant::now();
        let (h, _) = asking(&a, "gemini");
        assert_eq!(h.join().unwrap(), Answer::TimedOut);
        assert!(started.elapsed() < wait_for("claude"), "closed at Gemini's shorter wait");
        drop_chat(&a);
    }

    #[test]
    fn failure_notes() {
        let d = failure_of(&Answer::Declined).unwrap();
        assert_eq!((d.code, d.message.as_str()), ("declined", "The user declined. Don't ask again in chat; say what's blocked and go on without it."));
        let c = failure_of(&Answer::Cancelled).unwrap();
        assert_eq!((c.code, c.message.as_str()), ("cancelled", "The card closed with the run. Ask again with a new clientRequestId only if it is still needed."));
        let t = failure_of(&Answer::TimedOut).unwrap();
        assert_eq!((t.code, t.message.as_str()), ("timed_out", "No answer in time. Don't ask in chat; ask again with a new clientRequestId only if it is still needed."));
        assert_eq!(failure_of(&Answer::Allowed), None);
    }
}
