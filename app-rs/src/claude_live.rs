//! Claude Code kept open for a chat: `claude -p --input-format stream-json --output-format stream-json`, each message
//! one JSON line on its stdin, each turn's output ending with a `result` line (checked against Claude Code 2.1.319;
//! DEVELOPING.md has what was measured). The process and its pipes are `live::Live`, which the other AIs kept open
//! share; this file is what Claude Code's own protocol adds. What its lines mean is `Agent::consume_lines` in
//! `agents.rs`, the same code that reads a per-turn process, and the per-turn path stays there as the fallback.
//!
//! Claude Code can start a turn by itself: measured, seven seconds after a turn that left a shell task in the
//! background, when the task finished (`tests/fixtures/claude/live-background.ndjson`). Such a turn is ended where it
//! begins (`turn_line`, `Live::strayed`), and a turn that leaves a background task has its process closed at once
//! (`Agent::read_live`).

#[cfg(test)]
use crate::paths::Start;
use std::{process::Command, sync::Arc};
#[cfg(test)]
use std::{process::Stdio, time::Duration};

use serde_json::json;

pub use crate::live::{CLOSE_WAIT, IDLE, KEEP, Line, Live};
use crate::paths;

/// Whether a line begins or belongs to a turn's work: the `init` line every turn starts with, or the AI speaking.
fn turn_line(line: &[u8]) -> bool {
    let Ok(e) = serde_json::from_slice::<serde_json::Value>(line) else { return false };
    e["type"] == "assistant" || (e["type"] == "system" && e["subtype"] == "init")
}

impl Live {
    /// Starts Claude Code as `cmd` (its arguments, folder, environment and process group already set), kept open.
    pub fn start(cmd: Command, key: String) -> std::io::Result<Arc<Live>> {
        Live::start_as(cmd, key, "claude", turn_line)
    }

    /// Sends one message. `uuid` comes back in the lines that mark the turn it starts.
    pub fn say(&self, uuid: &str, text: &str) -> std::io::Result<()> {
        self.write(&json!({"type": "user", "uuid": uuid, "session_id": "", "parent_tool_use_id": null,
                           "message": {"role": "user", "content": [{"type": "text", "text": text}]}}))
    }

    /// Asks it to end the turn it is running. It answers with a `control_response`, then ends the turn with a `result`.
    pub fn interrupt(&self) -> bool {
        let n = self.next_request();
        self.write(&json!({"type": "control_request", "request_id": format!("fridae_{n}"), "request": {"subtype": "interrupt"}})).is_ok()
    }
}

/// A new id for a message, in the shape Claude Code's own ids have.
pub fn message_id() -> String {
    let h = paths::random_hex(2);
    format!("{}-{}-4{}-8{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for Claude Code: answers every `user` line with a `result` line, an interrupt with nothing.
    fn echo() -> Arc<Live> {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", r#"while read -r line; do case "$line" in *control_request*) ;; *) echo '{"type":"result","is_error":false}';; esac; done; echo bye >&2"#]);
        paths::own_group(&mut cmd);
        Live::start(cmd, "k".into()).unwrap()
    }

    #[test]
    fn a_message_id_looks_like_a_uuid_and_differs_each_time() {
        let (a, b) = (message_id(), message_id());
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn two_messages_go_to_one_process_and_closing_it_leaves_nothing() {
        let live = echo();
        for id in ["a", "b"] {
            live.say(id, "hi").unwrap();
            let line = live.lines().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            assert!(String::from_utf8_lossy(&line).contains("result"));
        }
        assert!(live.interrupt());
        assert!(live.drain(), "still there between turns");
        assert!(!live.exited());
        live.close();
        assert!(paths::leader_exited(live.pid, Duration::ZERO));
        assert_eq!(live.err_text(Duration::from_secs(2)), "bye", "it ended by itself when its stdin closed");
        assert!(!live.drain(), "its output has ended");
        assert!(live.say("c", "late").is_err());
        live.close(); // a second close is nothing
    }

    #[cfg(unix)]
    #[test]
    fn a_turn_it_starts_by_itself_ends_it_and_one_the_app_asked_for_does_not() {
        // speaks half a second after each message line, whoever is listening
        let talker = || {
            let mut cmd = Command::new("/bin/sh");
            cmd.args(["-c", r#"while read -r line; do sleep 0.5; echo '{"type":"system","subtype":"init"}'; echo '{"type":"assistant"}'; echo '{"type":"result"}'; done"#]);
            paths::own_group(&mut cmd);
            Live::start(cmd, "k".into()).unwrap()
        };
        let asked = talker();
        asked.set_in_turn(true);
        asked.say("a", "hi").unwrap();
        for _ in 0..3 {
            asked.lines().recv_timeout(Duration::from_secs(5)).unwrap();
        }
        asked.set_in_turn(false);
        assert!(!asked.strayed() && !asked.exited(), "the app's own turn is left alone");
        // the same lines with no turn of the app's on it
        asked.say("b", "hi").unwrap();
        assert!(paths::leader_exited(asked.pid, Duration::from_secs(8)), "ended at the first line of the turn");
        assert!(asked.strayed());
        assert!(asked.shut(Duration::from_secs(5)), "its output has ended");
        asked.close();
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_ignores_its_closed_stdin_is_ended_with_its_tree() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 300 & echo $! ; exec sleep 300"]);
        paths::own_group(&mut cmd);
        let live = Live::start(cmd, "k".into()).unwrap();
        let child: String = String::from_utf8_lossy(&live.lines().recv_timeout(Duration::from_secs(5)).unwrap().unwrap()).trim().to_string();
        live.kill();
        let gone = |pid: &str| Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).run_status().is_ok_and(|s| !s.success());
        assert!(gone(&live.pid.to_string()));
        assert!(gone(&child), "what it started went with it");
    }
}
