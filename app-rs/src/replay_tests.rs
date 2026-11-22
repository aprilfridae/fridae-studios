//! Real recorded output (`app-rs/tests/fixtures/<ai>/`, scrubbed) replayed through the app's own parsers and turn
//! state: `Agent::consume` reads it as it would read the AI's stdout, and `after_turn` ends the turn with the recorded
//! error output and exit code. Nothing but the pipe is stood in for.

use std::{path::Path, sync::Mutex};

use serde_json::Value;

use super::*;

fn fixture(ai: &str, name: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(ai).join(name)).unwrap_or_default()
}

/// The chat's events after the replay, and the conversation it would resume next. `resume` is the conversation the
/// turn was sent to go on with, as the app would have it.
fn replay(ai: &str, case: &str, resume: Option<&str>, exit: (bool, i32)) -> (Vec<Value>, Option<String>) {
    let out = fixture(ai, &format!("{case}.ndjson"));
    let err = String::from_utf8_lossy(&fixture(ai, &format!("{case}.err"))).trim().to_string();
    let a = Agent::detached_for_test(&format!("replay-{ai}-{case}"), None);
    {
        let mut st = a.st.lock().unwrap();
        st.ai = Some(ai.to_string());
        st.session = resume.map(str::to_string);
        st.running = true;
    }
    let last = Mutex::new((std::time::Instant::now(), false));
    let t = a.consume(&out[..], ai, &last, || {});
    let stopped = a.turn_over();
    a.after_turn(ai, t, &err, Some(exit), stopped);
    let st = a.st.lock().unwrap();
    (st.log.clone(), st.session.clone())
}

fn kinds(ev: &[Value]) -> Vec<&str> {
    ev.iter().map(|e| e["kind"].as_str().unwrap_or("")).collect()
}

fn said(ev: &[Value], kind: &str) -> Vec<String> {
    ev.iter().filter(|e| e["kind"] == kind).map(|e| e["text"].as_str().or(e["label"].as_str()).unwrap_or("").to_string()).collect()
}

const C1: &str = "00000000-0000-4000-8000-000000000001";

#[test]
fn claude_plain_reply_with_its_rate_limit_event() {
    let (ev, session) = replay("claude", "plain-with-rate-limit", None, (true, 0));
    assert_eq!(kinds(&ev), ["text", "usage", "done"]);
    assert_eq!(said(&ev, "text"), ["ok"]);
    assert_eq!(session.as_deref(), Some(C1));
    let lim = crate::limits::state();
    assert!(lim.to_string().contains("seven_day"), "the rate_limit_event was recorded: {lim}");
}

#[test]
fn claude_resumed_session_with_two_commands() {
    let (ev, session) = replay("claude", "resumed-with-tools", Some(C1), (true, 0));
    assert_eq!(kinds(&ev), ["tool", "tool", "text", "usage", "done"], "same conversation: no note");
    assert_eq!(said(&ev, "tool"), ["Running a command", "Running a command"]);
    assert_eq!(session.as_deref(), Some(C1));
}

#[test]
fn claude_resume_of_a_conversation_that_is_gone() {
    // Claude Code answers with a bare `error_during_execution` result and says why on stderr only
    let (ev, session) = replay("claude", "failed-no-conversation", Some("00000000-0000-4000-8000-000000000018"), (false, 1));
    assert_eq!(kinds(&ev), ["error", "done"]);
    assert!(said(&ev, "error")[0].starts_with("The memory of this earlier chat is no longer on this computer"));
    assert_eq!(session, None, "forgotten, so the next message starts afresh");
}

#[test]
fn gemini_plain_reply() {
    let (ev, session) = replay("gemini", "plain", None, (true, 0));
    assert_eq!(kinds(&ev), ["text", "usage", "done"]);
    assert_eq!(said(&ev, "text"), ["ok"]);
    assert!(session.is_some());
}

#[test]
fn gemini_resumed_with_a_file_written_and_a_command_refused() {
    let (first, s) = replay("gemini", "plain", None, (true, 0));
    assert!(!first.is_empty());
    let (ev, session) = replay("gemini", "resumed-tool-and-refused", s.as_deref(), (true, 0));
    assert_eq!(said(&ev, "tool"), ["Writing a file", "Running a command"]);
    assert!(said(&ev, "error").iter().any(|e| e.contains("does not allow")), "{ev:?}");
    assert_eq!(session, s, "the same conversation");
    assert_eq!(kinds(&ev).last(), Some(&"done"));
}

#[test]
fn gemini_started_afresh_when_its_conversation_was_gone_and_the_chat_is_told() {
    // agy only warns on stderr and opens a new conversation (measured)
    let (ev, session) = replay("gemini", "missing-conversation", Some("00000000-0000-4000-8000-000000000099"), (true, 0));
    assert_eq!(said(&ev, "text"), ["Hello! How can I help you today?"]);
    assert!(said(&ev, "error").iter().any(|e| e.starts_with("Gemini could not find this chat's earlier conversation")), "{ev:?}");
    assert_ne!(session.as_deref(), Some("00000000-0000-4000-8000-000000000099"));
}

#[test]
fn cursor_plain_tool_resumed_and_signed_out() {
    let (ev, _) = replay("cursor", "plain", None, (true, 0));
    assert_eq!(kinds(&ev), ["text", "usage", "done"]);
    let (ev, s) = replay("cursor", "tool", None, (true, 0));
    assert_eq!(kinds(&ev), ["tool", "text", "usage", "done"]);
    let (ev, again) = replay("cursor", "resumed", s.as_deref(), (true, 0));
    assert_eq!(kinds(&ev), ["text", "usage", "done"], "the same chat: no note");
    assert_eq!(again, s);
    let (ev, _) = replay("cursor", "failed-signed-out", None, (false, 1));
    assert_eq!(kinds(&ev), ["error", "done"]);
    assert!(said(&ev, "error")[0].starts_with("Cursor is not signed in"));
}

#[test]
fn cursor_refused_command_shows_the_card_or_a_plain_line() {
    let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // `./git status` refused: git not allowed yet, the card offers it
    let _ = paths::set_allowed(&["git".to_string()], false);
    let (ev, _) = replay("cursor", "refused", None, (true, 0));
    assert!(ev.iter().any(|e| e["kind"] == "blocked" && e["commands"][0]["name"] == "git"));
    // git allowed already: allowing cannot help (it was the `./`), so a plain line instead of the card
    let _ = paths::set_allowed(&["git".to_string()], true);
    let (ev, _) = replay("cursor", "refused", None, (true, 0));
    let _ = paths::set_allowed(&["git".to_string()], false);
    assert!(!ev.iter().any(|e| e["kind"] == "blocked"));
    assert!(said(&ev, "error").iter().any(|e| e.contains("plain name")));
}

#[test]
fn every_fixture_is_scrubbed() {
    let home = paths::home_dir();
    let user = home.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");
    let mut n = 0;
    for ai in fs::read_dir(&dir).unwrap().flatten() {
        for f in fs::read_dir(ai.path()).unwrap().flatten() {
            let text = fs::read_to_string(f.path()).unwrap();
            n += 1;
            assert!(user.len() < 3 || !text.contains(&format!("/{user}")), "{:?} names the user's folder", f.path());
            assert!(!text.contains("/private/tmp/claude-"), "{:?}", f.path());
            let email = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}").unwrap();
            assert!(email.find_iter(&text).all(|m| m.as_str() == "user@example.com"), "{:?}", f.path());
        }
    }
    assert!(n >= 20);
}
