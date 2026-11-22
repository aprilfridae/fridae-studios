//! The kept Claude Code process (`claude_live`, `Agent::live_turn`, `read_live`).
//!
//! First, real output recorded from one kept process per case (`tests/fixtures/claude/live-*.ndjson`, Claude Code
//! 2.1.319, scrubbed as the other fixtures) replayed through `consume_lines` and `after_turn`: only the pipe is stood
//! in for. Then whole turns against a stand-in process (a shell script that answers as Claude Code does on the wire),
//! for what a recording cannot show: one process across messages, a restart with `--resume`, Stop, a death, closing.

use std::{path::Path, sync::Mutex};

use serde_json::Value;

use super::*;

// ---------------------------------------------------------------- recordings

fn recorded(case: &str) -> std::vec::IntoIter<claude_live::Line> {
    let all = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("claude").join(format!("{case}.ndjson"))).unwrap();
    all.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| Some(l.to_vec())).collect::<Vec<_>>().into_iter()
}

/// The ids the recordings' messages were sent with.
const M1: &str = "00000000-0000-4000-8000-0000000000a1";
const M2: &str = "00000000-0000-4000-8000-0000000000a2";

struct Replayed {
    events: Vec<Value>,
    /// the process's output ended before the turn's result
    ended: bool,
    began: bool,
    /// Claude Code was seen to take the message
    taken: bool,
}

/// One turn read from `lines` as `read_live` reads it; `stop`: Stop was pressed during it.
fn turn(a: &Arc<Agent>, lines: &mut dyn Iterator<Item = claude_live::Line>, message: &str, first: bool, marks: &AtomicBool, stop: bool) -> Replayed {
    let before = {
        let mut st = a.st.lock().unwrap();
        st.ai = Some("claude".into());
        st.running = true;
        st.stopped = stop;
        st.log.len()
    };
    let tp = TurnProc::new(u32::MAX);
    let last = Mutex::new((std::time::Instant::now(), false, false));
    let t = a.consume_lines(lines, "claude", &last, || {}, Some(LiveTurn { tp: &tp, message, first, marks }));
    let (ended, began) = (t.ended, t.began);
    let stopped = a.turn_over();
    a.after_turn("claude", t, "", Some(if ended { (false, -1) } else { (true, 0) }), stopped);
    Replayed { events: a.st.lock().unwrap().log[before..].to_vec(), ended, began, taken: tp.acked.load(Ordering::SeqCst) }
}

fn kinds(ev: &[Value]) -> Vec<&str> {
    ev.iter().map(|e| e["kind"].as_str().unwrap_or("")).collect()
}

fn said(ev: &[Value], kind: &str) -> Vec<String> {
    ev.iter().filter(|e| e["kind"] == kind).map(|e| e["text"].as_str().or(e["label"].as_str()).unwrap_or("").to_string()).collect()
}

fn session(a: &Agent) -> Option<String> {
    a.st.lock().unwrap().session.clone()
}

const S1: &str = "00000000-0000-4000-8000-000000000002";

#[test]
fn recorded_plain_reply_ends_at_its_result_and_keeps_the_limits() {
    let a = Agent::detached_for_test("live-plain", None);
    let marks = AtomicBool::new(false);
    let mut lines = recorded("live-plain");
    let r = turn(&a, &mut lines, M1, true, &marks, false);
    assert_eq!(kinds(&r.events), ["text", "usage", "done"]);
    assert_eq!(said(&r.events, "text"), ["ok"]);
    assert!(!r.ended && r.began && r.taken);
    assert!(marks.load(Ordering::SeqCst), "this Claude Code marks its turns");
    assert_eq!(session(&a).as_deref(), Some(S1));
    assert!(lines.next().is_some(), "what follows the result (the turn's `completed` mark) is left for the next turn");
    let lim = crate::limits::state();
    assert!(lim.to_string().contains("seven_day"), "the rate_limit_event was recorded: {lim}");
}

#[test]
fn recorded_two_messages_in_one_process_are_two_turns_of_one_conversation() {
    let a = Agent::detached_for_test("live-two", None);
    let marks = AtomicBool::new(false);
    let mut lines = recorded("live-two");
    let one = turn(&a, &mut lines, M1, true, &marks, false);
    assert_eq!((kinds(&one.events), said(&one.events, "text")), (vec!["text", "usage", "done"], vec!["one".to_string()]));
    let two = turn(&a, &mut lines, M2, false, &marks, false);
    assert_eq!((kinds(&two.events), said(&two.events, "text")), (vec!["text", "usage", "done"], vec!["two".to_string()]));
    assert!(!two.ended && two.taken);
    assert_eq!(session(&a).as_deref(), Some(S1), "one conversation");
    // usage: a turn's own tokens each time, and the session's running total in each result, which `usage` logs as
    // the turn's share (the other tests replaying this conversation share that total, so it is read here directly)
    let tokens = |r: &Replayed| r.events.iter().find(|e| e["kind"] == "usage").and_then(|e| e["total"].as_u64()).unwrap_or(0);
    assert!(tokens(&one) > 0 && tokens(&two) > 0, "{} {}", tokens(&one), tokens(&two));
    let totals: Vec<f64> = recorded("live-two").flatten().filter_map(|l| serde_json::from_slice::<Value>(&l).ok()).filter(|e| e["type"] == "result").filter_map(|e| e["total_cost_usd"].as_f64()).collect();
    assert!(totals.len() == 2 && totals[1] > totals[0], "{totals:?}");
}

#[test]
fn what_an_earlier_turn_left_behind_is_not_taken_for_this_turns_answer() {
    // the whole recording read as the second message's turn, as if the first turn's lines were still in the pipe: its
    // answer and its good result are passed over until Claude Code marks the second message
    let a = Agent::detached_for_test("live-two-stale", None);
    let marks = AtomicBool::new(true);
    let r = turn(&a, &mut recorded("live-two"), M2, false, &marks, false);
    assert_eq!(said(&r.events, "text"), ["two"]);
    assert_eq!(kinds(&r.events), ["text", "usage", "done"]);
}

#[test]
fn recorded_tool_call() {
    let a = Agent::detached_for_test("live-tool", None);
    let r = turn(&a, &mut recorded("live-tool"), M1, true, &AtomicBool::new(false), false);
    assert_eq!(kinds(&r.events), ["tool", "text", "usage", "done"]);
    assert_eq!(said(&r.events, "tool"), ["Running a command"]);
    assert_eq!(said(&r.events, "text"), ["done"]);
}

#[test]
fn recorded_stop_mid_turn_ends_the_turn_and_the_next_message_works() {
    // recorded: a shell command running, the interrupt request, Claude Code's `control_response`, the stopped tool,
    // the turn's failed result 30 ms later, then a second message on the same process
    let a = Agent::detached_for_test("live-stop", None);
    let marks = AtomicBool::new(false);
    let mut lines = recorded("live-stop");
    let stopped = turn(&a, &mut lines, M1, true, &marks, true);
    assert!(!stopped.ended, "the turn ended with a result: the process lives on");
    assert_eq!(said(&stopped.events, "tool"), ["Running a command"]);
    assert_eq!(said(&stopped.events, "error"), ["Stopped."], "{:?}", stopped.events);
    assert_eq!(kinds(&stopped.events).last(), Some(&"done"));
    let after = turn(&a, &mut lines, M2, false, &marks, false);
    assert_eq!(said(&after.events, "text"), ["after"]);
    assert_eq!(kinds(&after.events), ["text", "usage", "done"]);
    assert_eq!(session(&a).as_deref(), Some(S1));
}

#[test]
fn recorded_process_that_dies_mid_turn() {
    // recorded: SIGKILL while a shell command ran: the output ends with no result
    let a = Agent::detached_for_test("live-dies", None);
    let r = turn(&a, &mut recorded("live-dies"), M1, true, &AtomicBool::new(false), false);
    assert!(r.ended && r.began, "it had started, so the message is not sent a second time");
    assert_eq!(said(&r.events, "tool"), ["Running a command"]);
    assert_eq!(said(&r.events, "error"), ["The agent exited unexpectedly (code -1)."]);
    assert_eq!(session(&a).as_deref(), Some(S1), "the next message resumes it");
}

// ------------------------------------------------------------ stand-in process

#[cfg(unix)]
mod stand_in {
    use super::*;

    /// Answers as Claude Code does on the wire. Kept open (`--input-format`): a reply per message line, with words in
    /// the message choosing what goes wrong. Otherwise the per-turn way: one message on stdin, one reply, exit.
    const SCRIPT: &str = r#"
echo "$*" >> "$0.args"
init='{"type":"system","subtype":"init","session_id":"S1","model":"claude-haiku-5-5","mcp_servers":[]}'
say() { echo "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"$1\"}]}}"; }
ok='{"type":"result","is_error":false,"session_id":"S1","usage":{}}'
case "$*" in *--input-format*) ;; *) cat >/dev/null; echo "$init"; say "per turn"; echo "$ok"; exit 0;; esac
n=0
while read -r line; do
  case "$line" in
    *control_request*) echo '{"type":"control_response","response":{"subtype":"success"}}'
       echo '{"type":"result","is_error":true,"subtype":"error_during_execution","session_id":"S1"}';;
    *die-early*) exit 3;;
    *die-late*) echo "$init"; say "half"; echo "boom" >&2; exit 3;;
    *hang*) echo "$init";;
    *deaf*) echo "$init"; trap '' TERM; while read -r l; do :; done; sleep 300;;
    *) n=$((n+1)); echo "$init"; say "reply $n"; echo "$ok";;
  esac
done
"#;

    struct Chat {
        a: Arc<Agent>,
        script: std::path::PathBuf,
    }

    impl Chat {
        fn new(name: &str) -> Chat {
            let dir = paths::test_dir("live").join(name);
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let script = dir.join("claude.sh");
            fs::write(&script, SCRIPT).unwrap();
            let a = Agent::detached_for_test(&format!("live-{name}"), None);
            a.st.lock().unwrap().ai = Some("claude".into());
            Chat { a, script }
        }

        fn launch(&self, extra: &[&str]) -> ClaudeLaunch {
            ClaudeLaunch { exe: "/bin/sh".into(), first: vec![self.script.clone().into()], args: extra.iter().map(Into::into).collect(), cwd: self.script.parent().unwrap().into(), servers: String::new() }
        }

        /// Sends a message the kept way and waits for the turn to end. False: it went to no kept process.
        fn send(&self, text: &str) -> bool {
            let sent = self.start(text);
            self.wait();
            sent
        }

        fn start(&self, text: &str) -> bool {
            self.a.live_allowed() && self.a.live_turn(&self.launch(&[]), text)
        }

        fn wait(&self) {
            let until = std::time::Instant::now() + Duration::from_secs(20);
            while self.a.st.lock().unwrap().running {
                assert!(std::time::Instant::now() < until, "the turn never ended: {:?}", self.a.st.lock().unwrap().log);
                thread::sleep(Duration::from_millis(20));
            }
        }

        fn pid(&self) -> Option<u32> {
            self.a.st.lock().unwrap().live.as_ref().map(|l| l.pid)
        }

        fn said(&self, kind: &str) -> Vec<String> {
            said(&self.a.st.lock().unwrap().log, kind)
        }

        /// The arguments of every process started for the chat.
        fn starts(&self) -> Vec<String> {
            fs::read_to_string(self.script.with_extension("sh.args")).unwrap_or_default().lines().map(str::to_string).collect()
        }

        fn fails(&self) -> u32 {
            self.a.st.lock().unwrap().live_fails
        }
    }

    impl Drop for Chat {
        fn drop(&mut self) {
            self.a.close_live();
        }
    }

    fn gone(pid: u32) -> bool {
        paths::leader_exited(pid, Duration::ZERO)
    }

    #[test]
    fn two_messages_run_on_one_process_and_other_arguments_start_another_that_resumes() {
        let c = Chat::new("reuse");
        assert!(c.send("one"));
        let first = c.pid().unwrap();
        assert!(c.send("two"));
        assert_eq!(c.pid(), Some(first), "the same process");
        assert_eq!(c.said("text"), ["reply 1", "reply 2"], "and it counted both");
        assert_eq!(c.starts().len(), 1);
        assert!(c.starts()[0].starts_with("-p --input-format stream-json --replay-user-messages"), "{:?}", c.starts());
        assert!(!c.starts()[0].contains("--resume"), "a new conversation");
        // another model (any other start argument): the process is closed and a new one resumes the conversation
        assert!(c.a.live_turn(&c.launch(&["--model", "other"]), "three"));
        c.wait();
        let second = c.pid().unwrap();
        assert_ne!(second, first);
        assert!(gone(first), "the old process was closed");
        assert_eq!(c.said("text").last().map(String::as_str), Some("reply 1"), "a new process");
        assert!(c.starts()[1].ends_with("--model other --resume S1"), "{:?}", c.starts());
        assert_eq!(c.fails(), 0);
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
    }

    #[test]
    fn stop_interrupts_the_turn_at_once_and_the_process_stays() {
        let c = Chat::new("stop");
        assert!(c.start("hang"));
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop(); // at once: Stop itself waits until Claude Code has taken the message
        c.wait();
        assert!(pressed.elapsed() < Duration::from_millis(1500), "confirmed by the interrupt, not by the kill after STOP_WAIT: {:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.pid(), Some(pid), "kept");
        assert!(!gone(pid));
        assert!(c.send("again"));
        assert_eq!(c.pid(), Some(pid));
        assert_eq!(c.said("text"), ["reply 1"]);
        assert_eq!(c.starts().len(), 1);
    }

    #[test]
    fn a_turn_that_ignores_the_interrupt_is_killed_after_the_wait_and_the_next_message_resumes() {
        let c = Chat::new("deaf");
        assert!(c.start("deaf"));
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() >= paths::STOP_WAIT, "it was given the wait");
        assert_eq!(c.said("error"), [paths::killed_message("Claude").as_str(), "Stopped."]);
        assert_eq!(c.pid(), None);
        assert!(gone(pid));
        assert_eq!(c.fails(), 0, "ended by Stop: not a failure of the kept process");
        assert!(c.send("again"));
        assert!(c.starts()[1].ends_with("--resume S1"), "{:?}", c.starts());
    }

    #[test]
    fn a_process_that_dies_before_it_showed_anything_has_its_message_sent_again_per_turn() {
        let c = Chat::new("die-early");
        assert!(c.send("die-early"));
        assert_eq!(c.said("text"), ["per turn"], "the user gets the answer");
        assert!(c.said("error").is_empty(), "and no failure: {:?}", c.said("error"));
        assert_eq!(c.fails(), 1);
        assert_eq!(c.pid(), None);
        assert!(!c.starts()[1].contains("--input-format"), "{:?}", c.starts());
        // a second failure: the chat stays on a process per turn
        assert!(c.send("die-early"));
        assert_eq!(c.fails(), 2);
        assert!(!c.a.live_allowed());
        assert!(!c.send("hello"));
    }

    #[test]
    fn a_process_that_dies_mid_turn_is_said_and_its_message_is_not_sent_twice() {
        let c = Chat::new("die-late");
        assert!(c.send("die-late"));
        assert_eq!(c.said("text"), ["half"]);
        assert_eq!(c.said("error"), ["boom"], "what it wrote to its error output");
        assert_eq!(c.starts().len(), 1, "no second process: a tool may already have run");
        assert_eq!((c.fails(), c.pid()), (1, None));
        // the next message starts a new process, which resumes
        assert!(c.send("again"));
        assert_eq!(c.said("text"), ["half", "reply 1"]);
        assert!(c.starts()[1].ends_with("--resume S1"), "{:?}", c.starts());
    }

    #[test]
    fn a_process_that_died_between_turns_is_replaced_without_a_word() {
        let c = Chat::new("idle-death");
        assert!(c.send("one"));
        let pid = c.pid().unwrap();
        paths::end_trees(&[pid]);
        assert!(paths::leader_exited(pid, Duration::from_secs(5)));
        assert!(c.send("two"));
        assert_ne!(c.pid(), Some(pid));
        assert_eq!(c.said("text"), ["reply 1", "reply 1"]);
        assert!(c.said("error").is_empty());
        assert!(c.starts()[1].ends_with("--resume S1"));
        assert_eq!(c.fails(), 1);
    }

    #[test]
    fn a_helper_and_a_closed_chat_keep_no_process_after_the_turn() {
        let helper = Chat::new("helper");
        helper.a.st.lock().unwrap().parent = Some("lead".into());
        assert!(helper.send("job"));
        assert_eq!(helper.pid(), None, "an expert holds nothing between jobs");
        assert_eq!(helper.said("text"), ["reply 1"]);
        let closed = Chat::new("closed");
        closed.a.st.lock().unwrap().closed = true;
        assert!(closed.send("last"));
        assert_eq!(closed.pid(), None);
    }

    #[test]
    fn idle_processes_are_closed_the_longest_idle_first() {
        let chats: Vec<Chat> = ["idle-a", "idle-b", "idle-c"].iter().map(|n| Chat::new(n)).collect();
        for c in &chats {
            assert!(c.send("hello"));
            thread::sleep(Duration::from_millis(30));
        }
        let all = || chats.iter().map(|c| c.a.clone()).collect::<Vec<_>>();
        let pids: Vec<u32> = chats.iter().map(|c| c.pid().unwrap()).collect();
        assert_eq!(close_idle_lives(all(), claude_live::IDLE, 3), 0, "three may stay");
        // one too many: the one idle longest goes
        assert_eq!(close_idle_lives(all(), claude_live::IDLE, 2), 1);
        assert_eq!(chats.iter().map(|c| c.pid()).collect::<Vec<_>>(), [None, Some(pids[1]), Some(pids[2])]);
        assert!(gone(pids[0]));
        // a chat with a turn running is left alone, whatever the clock says
        chats[1].a.st.lock().unwrap().running = true;
        assert_eq!(close_idle_lives(all(), Duration::ZERO, 3), 1, "idle past the limit");
        assert_eq!(chats.iter().map(|c| c.pid()).collect::<Vec<_>>(), [None, Some(pids[1]), None]);
        chats[1].a.st.lock().unwrap().running = false;
        // the closed chat's next message starts a process that resumes
        assert!(chats[0].send("back"));
        assert!(chats[0].starts()[1].ends_with("--resume S1"));
    }

    #[test]
    fn forcing_a_process_per_turn_is_one_capability_and_one_switch() {
        let c = Chat::new("caps");
        assert!(c.a.live_allowed());
        c.a.st.lock().unwrap().ai = Some("gemini".into());
        assert!(!c.a.live_allowed(), "only an AI the table says keeps a process");
    }
}
