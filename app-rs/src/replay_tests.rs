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
    let last = Mutex::new((std::time::Instant::now(), false, false));
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

/// Recorded output handed over in two parts, with something done in between (a file agy wrote while the turn ran).
struct Staged<F: FnMut()> {
    parts: std::collections::VecDeque<Vec<u8>>,
    between: Option<F>,
}

impl<F: FnMut()> std::io::Read for Staged<F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.parts.front().is_some_and(Vec::is_empty) {
            self.parts.pop_front();
            if let Some(mut f) = self.between.take() {
                f();
            }
        }
        let Some(part) = self.parts.front_mut() else { return Ok(0) };
        let n = part.len().min(buf.len());
        buf[..n].copy_from_slice(&part[..n]);
        part.drain(..n);
        Ok(n)
    }
}

#[test]
fn gemini_command_refused_by_the_apps_rule_carries_on_and_the_picture_it_generated_is_shown() {
    // A real turn (agy 1.5.15, Gemini 3.8 Flash at low effort) with the settings `turn_args` now passes outside Full
    // auto: `"deny": ["command(*)"]`. Asked to run `echo hello-from-shell`, then to use its generate_image tool once,
    // then to report. The command's step ended in ERROR ("Matches user-configured deny rule"), the turn went on, the
    // image helper made a picture in agy's folder for the conversation, and the result was SUCCESS with exit 0 and
    // nothing on stderr. Before, the first refused command ended the turn.
    let raw = fixture("gemini", "command-refused-softly-then-image.ndjson");
    let text = String::from_utf8(raw.clone()).unwrap();
    assert!(text.contains(r#""state":"ERROR","step_type":"tool","tool_name":"run_command""#) && text.contains("Matches user-configured deny rule"));
    assert!(text.contains(r#""type_name":"image-generator""#) && text.contains(r#""status":"SUCCESS""#));
    // the picture is written while the turn runs: after the first line (`init`), as agy's helper did
    let conversation = "00000000-0000-4000-8000-0000000000c1";
    let dir = antigravity::conversation_dir(&antigravity::data_dir(), conversation).unwrap();
    let _ = fs::remove_dir_all(&dir);
    let cut = raw.iter().position(|b| *b == b'\n').unwrap() + 1;
    let picture = dir.join("grey_skipping_stone_1795466515478.jpg");
    let staged = Staged {
        parts: [raw[..cut].to_vec(), raw[cut..].to_vec()].into(),
        between: Some(|| {
            fs::create_dir_all(&dir).unwrap();
            image::RgbImage::from_pixel(8, 8, image::Rgb([120, 120, 120])).save(&picture).unwrap();
        }),
    };
    let a = Agent::detached_for_test("replay-gemini-soft-refusal", None);
    {
        let mut st = a.st.lock().unwrap();
        st.ai = Some("gemini".to_string());
        st.running = true;
    }
    let last = Mutex::new((std::time::Instant::now(), false, false));
    let t = a.consume(std::io::BufReader::with_capacity(64, staged), "gemini", &last, || {});
    let stopped = a.turn_over();
    a.after_turn("gemini", t, "", Some((true, 0)), stopped);
    let st = a.st.lock().unwrap();
    let ev = st.log.clone();
    assert!(!kinds(&ev).contains(&"error"), "no error: {ev:?}");
    assert_eq!(kinds(&ev).last(), Some(&"done"));
    assert_eq!(said(&ev, "tool")[..2], ["Running a command", "Generating an image"], "{:?}", said(&ev, "tool"));
    let texts = said(&ev, "text");
    assert!(texts.last().unwrap().contains("grey_skipping_stone_1795466515478.jpg"), "its last words name the picture: {texts:?}");
    assert!(texts[0].contains("refused"), "and it said the command was refused: {texts:?}");
    let images: Vec<&Value> = ev.iter().filter(|e| e["kind"] == "images").collect();
    assert_eq!(images.len(), 1, "{:?}", kinds(&ev));
    assert_eq!(images[0]["urls"].as_array().map(Vec::len), Some(1), "shown from the chat's own pictures folder: {}", images[0]);
    assert!(images[0]["urls"][0].as_str().unwrap().starts_with("/img/"));
    assert_eq!(st.session.as_deref(), Some(conversation));
}

#[test]
fn gemini_commands_with_a_path_a_pipe_and_two_joined_are_refused_softly_and_so_is_a_write_to_its_own_server_list() {
    // A real turn (agy 1.5.15, Gemini 3.8 Flash at low effort, `--mode accept-edits`) with the deny rules `turn_args`
    // passes outside Full auto: `write_file(<workspace>/.agents/mcp_config.json)` and `command(*)`. Asked to run
    // `/bin/echo one`, `echo two | tr a-z A-Z` and `echo three && echo four`, to write note.txt, then to write its
    // workspace's own list of tool servers. Each command's step ended in ERROR by the rule, note.txt was written, the
    // list was refused by its rule, and the turn ended SUCCESS with exit 0 and nothing on stderr.
    let raw = String::from_utf8(fixture("gemini", "commands-refused-softly-and-own-server-list.ndjson")).unwrap();
    let steps: Vec<Value> = raw.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).map(|e| e["step_update"].clone()).filter(|s| s["step_type"] == "tool" && s["state"] != "ACTIVE").collect();
    let ended: Vec<(String, String, String)> = steps
        .iter()
        .map(|s| {
            let p = &s["tool_info"]["parameters"];
            (s["tool_name"].as_str().unwrap().to_string(), p["CommandLine"].as_str().or(p["Path"].as_str()).unwrap().to_string(), s["state"].as_str().unwrap().to_string())
        })
        .collect();
    let want = [
        ("run_command", "/bin/echo one", "ERROR"),
        ("run_command", "echo two | tr a-z A-Z", "ERROR"),
        ("run_command", "echo three && echo four", "ERROR"),
        ("write_file", "/scratch/ws/note.txt", "DONE"),
        ("write_file", "/scratch/ws/.agents/mcp_config.json", "ERROR"),
    ];
    assert_eq!(ended.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect::<Vec<_>>(), want);
    for s in steps.iter().filter(|s| s["state"] == "ERROR") {
        let why = s["tool_info"]["error"]["message"].as_str().unwrap();
        assert!(why.contains("Permission denied for") && why.contains("Matches user-configured deny rule"), "{why}");
    }
    let (ev, session) = replay("gemini", "commands-refused-softly-and-own-server-list", None, (true, 0));
    assert!(!kinds(&ev).contains(&"error"), "{ev:?}");
    assert_eq!(said(&ev, "tool"), ["Running a command", "Running a command", "Running a command", "Writing a file", "Writing a file"]);
    let text = said(&ev, "text").join("\n");
    assert_eq!(text.matches("Refused").count(), 4, "{text}");
    assert!(text.contains("Step 4: Ran"), "{text}");
    assert_eq!(kinds(&ev).last(), Some(&"done"));
    assert!(session.is_some());
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
fn cursor_reports_the_real_result_of_a_job_longer_than_its_cut_off() {
    // A real turn on a process per message (cursor-agent 2026.11.22, Auto), with the app's tool server given its
    // 45-second budget: the stand-in tool was asked to work for 240 seconds, four times Cursor's 60-second cut-off.
    // The call answered "not finished yet" at 44 s, `rbx_job wait` did so four more times, the fifth returned the
    // tool's own result with its picture, and the model reported the word in it. No call was cut off.
    let raw = String::from_utf8(fixture("cursor", "long-job.ndjson")).unwrap();
    let results: Vec<String> = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["type"] == "tool_call" && e["subtype"] == "completed")
        .filter_map(|e| e["tool_call"]["mcpToolCall"]["result"]["success"]["content"][0]["text"]["text"].as_str().map(str::to_string))
        .collect();
    assert_eq!(results.len(), 6, "the slow call and five waits");
    assert!(results[..5].iter().all(|r| r.starts_with("NOT FINISHED YET. This is not the tool's result")), "{results:?}");
    assert!(results[5].starts_with("The stand-in tool worked for 240 seconds and finished. Its result word is MARBLE."), "{}", results[5]);
    assert!(!raw.contains("Request timed out") && !raw.contains("-32001"), "nothing was cut off");
    let (ev, _) = replay("cursor", "long-job", None, (true, 0));
    assert_eq!(kinds(&ev), ["text", "tool", "text", "tool", "tool", "tool", "tool", "tool", "images", "text", "usage", "done"]);
    assert_eq!(said(&ev, "tool")[0], "rbx test slow");
    assert!(said(&ev, "tool")[1..].iter().all(|t| t == "Waiting for work that takes a while"), "{:?}", said(&ev, "tool"));
    assert_eq!(said(&ev, "text").last().map(String::as_str), Some("Result word: **MARBLE**"));
    assert!(!kinds(&ev).contains(&"error"), "and nothing is said about work thrown away: the job was collected");
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

#[test]
fn full_auto_deny_rules_as_claude_and_antigravity_kept_them() {
    // Claude Code in bypassPermissions: `Edit(//.../a/config.json)` refused config.json and CONFIG.JSON (it ignores
    // case), a `[cC]...` rule refused b/CONFIG.JSON, another file was written; the turn reads as an ordinary reply
    let (ev, _) = replay("claude", "full-auto-deny-rules", None, (true, 0));
    assert_eq!(said(&ev, "tool").len(), 4, "{:?}", kinds(&ev));
    let text = said(&ev, "text").join("\n");
    assert_eq!(text.matches("refused").count(), 3, "{text}");
    assert!(text.contains("other.txt: ok"), "{text}");
    assert!(!kinds(&ev).contains(&"error"));
    // Antigravity with --dangerously-skip-permissions: `write_file(.../a/config.json)` refused config.json but not
    // CONFIG.JSON (it compares case: the app puts the allowed list back after a turn, paths::guard_allowed);
    // `write_file(<folder>)` refused a file inside it
    let (ev, _) = replay("gemini", "full-auto-deny-rules", None, (true, 0));
    let text = said(&ev, "text").join("\n");
    assert!(text.contains("a/config.json refused") && text.contains("CONFIG.JSON ok") && text.contains("c/x.txt refused") && text.contains("other.txt ok"), "{text}");
}
