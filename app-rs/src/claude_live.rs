//! One Claude Code process kept open for a chat: `claude -p --input-format stream-json --output-format stream-json`,
//! each message one JSON line on its stdin, each turn's output ending with a `result` line (checked against Claude
//! Code 2.1.319; DEVELOPING.md has what was measured). This file is the process and its pipes only: what its lines
//! mean is `Agent::consume_lines` in `agents.rs`, the same code that reads a per-turn process, and the per-turn path
//! stays there as the fallback.
//!
//! A thread reads the output for as long as the process lives and hands the lines over a channel, so nothing the tool
//! writes between two turns blocks it, and a turn starts by dropping what is left from before its message.

use std::{
    collections::VecDeque,
    io::{BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

use crate::{paths, procs};

/// How long a process whose stdin was closed gets to exit by itself before its tree is ended.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// A user's chat or a Lead idle this long has its process closed (the next message starts one with `--resume`).
pub const IDLE: Duration = Duration::from_secs(10 * 60);
/// At most this many idle processes stay open; the longest idle goes first.
pub const KEEP: usize = 3;

/// A line of output (None: one over the line cap, dropped), as `paths::CappedLines` gives them.
pub type Line = Option<Vec<u8>>;

pub struct Live {
    pub pid: u32,
    /// What it was started with (see `Agent::send`): a message whose start arguments differ gets a new process.
    pub key: String,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    lines: Mutex<mpsc::Receiver<Line>>,
    err_tail: Arc<Mutex<VecDeque<u8>>>,
    err_reader: Mutex<Option<thread::JoinHandle<()>>>,
    /// It has sent a `command_lifecycle` line: this Claude Code says which message each turn belongs to.
    pub marks_turns: AtomicBool,
    /// Turns sent to it so far.
    pub turns: AtomicU32,
    idle_since: Mutex<Instant>,
    requests: AtomicU32,
}

impl Live {
    /// Starts `cmd` (its arguments, folder, environment and process group already set) with all three pipes, and
    /// notes it in the process ledger.
    pub fn start(mut cmd: Command, key: String) -> std::io::Result<Arc<Live>> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let pid = child.id();
        procs::record("claude", pid); // ended at the next start if the app crashes meanwhile
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            thread::spawn(move || {
                for line in paths::CappedLines::new(BufReader::new(out), paths::LINE_CAP) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        // the error output is always read to its end (a full pipe would block it), and only its tail is kept
        let err_tail = Arc::new(Mutex::new(VecDeque::new()));
        let err_reader = child.stderr.take().map(|e| {
            let kept = err_tail.clone();
            thread::spawn(move || paths::drain_tail_into(e, paths::TAIL, &kept))
        });
        let stdin = child.stdin.take();
        Ok(Arc::new(Live {
            pid,
            key,
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            lines: Mutex::new(rx),
            err_tail,
            err_reader: Mutex::new(err_reader),
            marks_turns: AtomicBool::new(false),
            turns: AtomicU32::new(0),
            idle_since: Mutex::new(Instant::now()),
            requests: AtomicU32::new(0),
        }))
    }

    fn write(&self, line: &serde_json::Value) -> std::io::Result<()> {
        let mut stdin = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pipe) = stdin.as_mut() else { return Err(std::io::ErrorKind::BrokenPipe.into()) };
        pipe.write_all(format!("{line}\n").as_bytes())?;
        pipe.flush()
    }

    /// Sends one message. `uuid` comes back in the lines that mark the turn it starts.
    pub fn say(&self, uuid: &str, text: &str) -> std::io::Result<()> {
        self.write(&json!({"type": "user", "uuid": uuid, "session_id": "", "parent_tool_use_id": null,
                           "message": {"role": "user", "content": [{"type": "text", "text": text}]}}))
    }

    /// Asks it to end the turn it is running. It answers with a `control_response`, then ends the turn with a `result`.
    pub fn interrupt(&self) -> bool {
        let n = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        self.write(&json!({"type": "control_request", "request_id": format!("fridae_{n}"), "request": {"subtype": "interrupt"}})).is_ok()
    }

    /// Drops what it wrote since its last turn (nothing a turn reads may come from before its message). False when
    /// its output has ended: it is gone.
    pub fn drain(&self) -> bool {
        let rx = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match rx.try_recv() {
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Its output, for the turn that reads it (`recv` ends when the process's output does).
    pub fn lines(&self) -> MutexGuard<'_, mpsc::Receiver<Line>> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the process has exited (it is not reaped by asking).
    pub fn exited(&self) -> bool {
        paths::leader_exited(self.pid, Duration::ZERO)
    }

    /// A turn of its has just ended: the idle clock starts.
    pub fn touch(&self) {
        *self.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    pub fn idle_for(&self) -> Duration {
        self.idle_since.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
    }

    /// What it wrote to its error output (the tail), waited for only so long: a process outside its group that still
    /// holds the pipe never keeps a turn busy.
    pub fn err_text(&self, wait: Duration) -> String {
        let reader = self.err_reader.lock().unwrap_or_else(|e| e.into_inner()).take();
        let until = Instant::now() + wait;
        while reader.as_ref().is_some_and(|r| !r.is_finished()) && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        paths::tail_text(&self.err_tail)
    }

    /// Reaps the process once it has exited or been ended, and takes it off the ledger: (exited well, exit code).
    pub fn reap(&self) -> Option<(bool, i32)> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        let status = child.and_then(|mut c| c.wait().ok());
        procs::forget(self.pid);
        status.map(|s| (s.success(), s.code().unwrap_or(-1)))
    }

    /// Closes it between turns: stdin is closed, it gets CLOSE_WAIT to exit by itself (measured: about a second),
    /// and whatever it or its tools left running is ended. Blocks: call it off the page's thread.
    pub fn close(&self) {
        if self.child.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return;
        }
        // the groups of its tree are read while it is alive: its tools' shells have new parents once it is gone
        let groups = procs::tree_groups(self.pid);
        drop(self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take());
        if paths::leader_exited(self.pid, CLOSE_WAIT) {
            // unreaped, so its own group's ID is still reserved: what is left in it is ended, then the other groups
            paths::end_group_soon(self.pid);
            let others: Vec<u32> = groups.into_iter().filter(|&g| g != self.pid).collect();
            paths::end_trees(&others);
        } else {
            paths::end_trees(&groups);
        }
        self.reap();
    }

    /// Ends it now, with its whole tree (the app is quitting).
    pub fn kill(&self) {
        if self.child.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return;
        }
        paths::end_trees(&procs::tree_groups(self.pid));
        self.reap();
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
    fn a_process_that_ignores_its_closed_stdin_is_ended_with_its_tree() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 300 & echo $! ; exec sleep 300"]);
        paths::own_group(&mut cmd);
        let live = Live::start(cmd, "k".into()).unwrap();
        let child: String = String::from_utf8_lossy(&live.lines().recv_timeout(Duration::from_secs(5)).unwrap().unwrap()).trim().to_string();
        live.kill();
        let gone = |pid: &str| Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().is_ok_and(|s| !s.success());
        assert!(gone(&live.pid.to_string()));
        assert!(gone(&child), "what it started went with it");
    }
}
