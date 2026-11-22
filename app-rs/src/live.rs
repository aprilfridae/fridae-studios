//! One process of an AI's kept open for a chat between turns: the process and its pipes only, whatever it speaks.
//! Claude Code's line protocol (`claude_live`) and the Agent Client Protocol (`acp`: Gemini through `agy acp`) are thin
//! layers over it; what a turn's lines mean is theirs. Which AI works this way is `caps.live`; the rules for keeping
//! one (the launch key, idle closing, the fallback to a process per turn) are in `agents.rs`, the same for all.
//!
//! A thread reads the output for as long as the process lives and hands the lines over a channel, so nothing the tool
//! writes between two turns blocks it, and a turn starts by dropping what is left from before its message.
//!
//! A kept process can begin work nobody asked for (Claude Code: a turn of its own when a background task ends,
//! measured; an ACP agent: a tool call or a permission request outside a turn). The chat shows nothing of it and Stop
//! would not find it, so it is never left to run: the reading thread ends the process, tree and all, at the first such
//! line that comes while the app has no turn on it (`strayed`; each protocol says which lines those are). The next
//! message starts a process that resumes the conversation, as a process per turn always did.

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

use crate::{paths, procs};

/// How long a process whose stdin was closed gets to exit by itself before its tree is ended.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// A user's chat or a Lead idle this long has its process closed (the next message starts one that resumes).
pub const IDLE: Duration = Duration::from_secs(10 * 60);
/// At most this many idle processes stay open; the longest idle goes first.
pub const KEEP: usize = 3;

/// A line of output (None: one over the line cap, dropped), as `paths::CappedLines` gives them.
pub type Line = Option<Vec<u8>>;

type Stopper = Box<dyn Fn() -> bool + Send + Sync>;
type State = Arc<dyn std::any::Any + Send + Sync>;

pub struct Live {
    pub pid: u32,
    /// What it was started with (see `Agent::send`): a message whose start arguments differ gets a new process.
    pub key: String,
    /// The AI it is ("claude", "gemini"), as the process ledger and the chat name it.
    pub owner: &'static str,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    lines: Mutex<mpsc::Receiver<Line>>,
    err_tail: Arc<Mutex<VecDeque<u8>>>,
    err_reader: Mutex<Option<thread::JoinHandle<()>>>,
    /// Claude Code: it has sent a `command_lifecycle` line, so it says which message each turn belongs to.
    pub marks_turns: AtomicBool,
    /// Turns sent to it so far.
    pub turns: AtomicU32,
    idle_since: Mutex<Instant>,
    requests: AtomicU32,
    /// The app has a turn on it (from just before its message is written until the turn's end is read).
    in_turn: Arc<AtomicBool>,
    /// It began work nobody asked for and was ended for it.
    strayed: Arc<AtomicBool>,
    stopper: Mutex<Option<Stopper>>,
    state: Mutex<Option<State>>,
    tap: Tap,
}

/// FRIDAE_LIVE_RECORD=<folder> (testing only, debug builds only): everything a kept process is sent and writes is
/// also written there, one file per process, a line each: `> ` before what the app sent, `< ` before what it wrote.
/// This is how the recordings in `tests/fixtures/*/` of a kept process are made (DEVELOPING.md says how they are
/// scrubbed).
type Tap = Option<Arc<Mutex<std::fs::File>>>;

#[cfg(debug_assertions)]
fn tap_for(owner: &str, pid: u32) -> Tap {
    let dir = std::path::PathBuf::from(std::env::var_os("FRIDAE_LIVE_RECORD").filter(|d| !d.is_empty())?);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::File::create(dir.join(format!("{owner}-{pid}.wire"))).ok().map(|f| Arc::new(Mutex::new(f)))
}

#[cfg(not(debug_assertions))]
fn tap_for(_owner: &str, _pid: u32) -> Tap {
    None
}

fn tapped(tap: &Tap, mark: &str, line: &[u8]) {
    if let Some(t) = tap {
        let mut f = t.lock().unwrap_or_else(|e| e.into_inner());
        let _ = f.write_all(mark.as_bytes()).and_then(|_| f.write_all(line)).and_then(|_| f.write_all(b"\n"));
    }
}

impl Live {
    /// Starts `cmd` (its arguments, folder, environment and process group already set) with all three pipes, and
    /// notes it in the process ledger under `owner` (the AI's id). `stray` says whether a line of its output is work
    /// it began by itself when the line comes with no turn of the app's on the process.
    pub fn start_as(mut cmd: Command, key: String, owner: &'static str, stray: fn(&[u8]) -> bool) -> std::io::Result<Arc<Live>> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let pid = child.id();
        procs::record(owner, pid); // ended at the next start if the app crashes meanwhile
        let (tx, rx) = mpsc::channel();
        let (in_turn, strayed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let tap = tap_for(owner, pid);
        if let Some(out) = child.stdout.take() {
            let (in_turn, strayed, tap) = (in_turn.clone(), strayed.clone(), tap.clone());
            thread::spawn(move || {
                for line in paths::CappedLines::new(BufReader::new(out), paths::LINE_CAP) {
                    if let Some(l) = &line {
                        tapped(&tap, "< ", l);
                    }
                    // work nobody asked for: ended where it begins (the app holds the process unreaped, so the pid is
                    // still its own)
                    if !in_turn.load(Ordering::SeqCst) && !strayed.load(Ordering::SeqCst) && line.as_deref().is_some_and(stray) {
                        strayed.store(true, Ordering::SeqCst);
                        thread::spawn(move || paths::end_trees(&procs::tree_groups(pid)));
                    }
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
            owner,
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            lines: Mutex::new(rx),
            err_tail,
            err_reader: Mutex::new(err_reader),
            marks_turns: AtomicBool::new(false),
            turns: AtomicU32::new(0),
            idle_since: Mutex::new(Instant::now()),
            requests: AtomicU32::new(0),
            in_turn,
            strayed,
            stopper: Mutex::new(None),
            state: Mutex::new(None),
            tap,
        }))
    }

    /// Writes one JSON line to its stdin, whole and under one lock: two threads' lines never mix.
    pub fn write(&self, line: &serde_json::Value) -> std::io::Result<()> {
        let mut stdin = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pipe) = stdin.as_mut() else { return Err(std::io::ErrorKind::BrokenPipe.into()) };
        tapped(&self.tap, "> ", line.to_string().as_bytes());
        pipe.write_all(format!("{line}\n").as_bytes())?;
        pipe.flush()
    }

    /// The next number for a request the app sends it (1, 2, 3...).
    pub fn next_request(&self) -> u32 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// How Stop asks it to end the turn it is running, when its protocol has a way other than Claude Code's.
    pub fn set_stopper(&self, stop: Stopper) {
        *self.stopper.lock().unwrap_or_else(|e| e.into_inner()) = Some(stop);
    }

    /// Asks it that way. None when its protocol set none.
    pub fn ask_stop(&self) -> Option<bool> {
        self.stopper.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|stop| stop())
    }

    /// What its protocol keeps between turns (an ACP session), once it has one.
    pub fn set_state(&self, state: State) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
    }

    pub fn state(&self) -> Option<State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The app's turn on it begins (before its message is written) or has ended (the turn's end was read).
    pub fn set_in_turn(&self, on: bool) {
        self.in_turn.store(on, Ordering::SeqCst);
    }

    /// It began work nobody asked for and was ended for it (not a failure of keeping a process).
    pub fn strayed(&self) -> bool {
        self.strayed.load(Ordering::SeqCst)
    }

    /// Closes its stdin (it then exits by itself: measured, in about a second for Claude Code, under half a second for
    /// agy) and waits up to `wait` for its output to end, dropping what it still writes. True when the output ended.
    pub fn shut(&self, wait: Duration) -> bool {
        drop(self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take());
        let rx = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let until = Instant::now() + wait;
        loop {
            match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return true,
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
            }
        }
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

    /// Whether it has been reaped: it is gone, and so is its hold on whatever it had open.
    pub fn reaped(&self) -> bool {
        self.child.lock().unwrap_or_else(|e| e.into_inner()).is_none()
    }

    /// Closes it between turns: stdin is closed, it gets CLOSE_WAIT to exit by itself, and whatever it or its tools
    /// left running is ended. Blocks: call it off the page's thread.
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
