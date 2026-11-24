//! The kept Claude Code process (`claude_live`, `Agent::live_turn`, `read_live`).
//!
//! First, real output recorded from one kept process per case (`tests/fixtures/claude/live-*.ndjson`, Claude Code
//! 2.1.319, scrubbed as the other fixtures) replayed through `consume_lines` and `after_turn`: only the pipe is stood
//! in for. Then whole turns against a stand-in process (a shell script that answers as Claude Code does on the wire),
//! for what a recording cannot show: one process across messages, a restart with `--resume`, Stop, a death, closing.

use std::{path::Path, sync::Mutex};

use serde_json::{Value, json};

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
    /// Claude Code was seen to start on the message
    taken: bool,
    /// the turn left a task of its own running in the background
    background_left: bool,
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
    let (ended, began, background_left) = (t.ended, t.began, t.background_left);
    let stopped = a.turn_over();
    a.after_turn("claude", t, "", Some(if ended { (false, -1) } else { (true, 0) }), stopped);
    Replayed { events: a.st.lock().unwrap().log[before..].to_vec(), ended, began, taken: tp.acked.load(Ordering::SeqCst), background_left }
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
    // (the exit code is this helper's own; the stand-in tests below end a real process through `read_live`)
    assert!(said(&r.events, "error")[0].starts_with("The agent exited unexpectedly"), "{:?}", r.events);
    assert_eq!(session(&a).as_deref(), Some(S1), "the next message resumes it");
}

#[test]
fn recorded_turn_that_leaves_a_background_task_and_the_turn_claude_code_then_starts_by_itself() {
    // recorded: a turn that started a shell task in the background and ended; seven seconds later, with nothing
    // sent, Claude Code noted the task's end and began a turn of its own (`init`, an answer, a `result`)
    let a = Agent::detached_for_test("live-background", None);
    let marks = AtomicBool::new(false);
    let mut lines = recorded("live-background");
    let r = turn(&a, &mut lines, M1, true, &marks, false);
    assert_eq!(said(&r.events, "text"), ["started"]);
    assert!(r.background_left, "so `read_live` closes the process before that turn can begin");
    assert!(!r.ended);
    let rest: Vec<Value> = lines.flatten().filter_map(|l| serde_json::from_slice(&l).ok()).collect();
    assert!(rest.iter().any(|e| e["type"] == "system" && e["subtype"] == "init"), "the turn nobody asked for");
    assert!(rest.iter().any(|e| e["type"] == "result"));
    // read as the next message's turn (as if the process had been kept and its lines were still in the pipe), none
    // of it is taken for that message's answer: the message's id never comes, so its good result is passed over
    let b = Agent::detached_for_test("live-background-next", None);
    let mut all = recorded("live-background");
    let first = all.by_ref().position(|l| l.is_some_and(|l| String::from_utf8_lossy(&l).contains("\"type\":\"result\""))).unwrap();
    assert!(first > 5);
    let next = turn(&b, &mut all, M2, false, &AtomicBool::new(true), false);
    assert!(said(&next.events, "text").is_empty(), "{:?}", next.events);
    assert!(next.ended && !next.began && !next.taken, "nothing of it counted as the turn");
}

#[cfg(unix)]
#[test]
fn a_kept_process_playing_that_recording_is_ended_when_the_turn_nobody_asked_for_begins() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("claude").join("live-background.ndjson");
    let own = fs::read_to_string(&file).unwrap().lines().position(|l| l.contains("\"type\":\"result\"")).unwrap() + 1;
    // the asked-for turn's lines when the message is read; the rest a second later, by itself, as recorded
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", &format!("read -r l; head -n {own} \"$1\"; sleep 1; tail -n +{} \"$1\"; sleep 300", own + 1), "sh"]).arg(&file);
    paths::own_group(&mut cmd);
    let live = Live::start(cmd, "k".into()).unwrap();
    live.set_in_turn(true);
    live.say(M1, "go").unwrap();
    loop {
        let line = live.lines().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        if String::from_utf8_lossy(&line).contains("\"type\":\"result\"") {
            break;
        }
    }
    live.set_in_turn(false);
    assert!(!live.strayed());
    assert!(paths::leader_exited(live.pid, Duration::from_secs(8)), "ended at its `init` line");
    assert!(live.strayed());
    live.close();
}

fn settings<'a>(servers: &'a Value, allowed: &'a [String], denied: &'a [String]) -> ClaudeSettings<'a> {
    ClaudeSettings {
        exe: Path::new("/bin/claude"),
        mcp_config: Path::new("/work/agent-tools/c1.json"),
        servers,
        allowed,
        denied,
        full: false,
        prompt: "the rules",
        kit: Path::new("/kit"),
        project: Some("/games/a"),
        model: Some("opus"),
        effort: Some("high"),
        work: Path::new("/work"),
        keys_on: false,
        config_dir: None,
    }
}

#[test]
fn the_launch_key_changes_with_everything_a_turn_is_started_with() {
    let servers = json!({"rbxkit": {"env": {"RBXKIT_PROJECT": "/games/a", "FRIDAE_GEMINI_USE_API": "0"}}});
    let (allowed, denied) = (vec!["git".to_string()], vec!["Edit(//kit/config.json)".to_string()]);
    let base = || settings(&servers, &allowed, &denied);
    let key = |s: ClaudeSettings| ClaudeLaunch::new(&s).key();
    let same = key(base());
    assert_eq!(key(base()), same, "the same settings: the process is kept");
    let other_servers = json!({"rbxkit": {"env": {"RBXKIT_PROJECT": "/games/a", "FRIDAE_GEMINI_USE_API": "1"}}});
    let more_allowed = vec!["git".to_string(), "python3".to_string()];
    let more_denied = vec!["Edit(//kit/config.json)".to_string(), "Edit(//work/usage.jsonl)".to_string()];
    let changed: Vec<(&str, ClaudeSettings)> = vec![
        ("the mode", ClaudeSettings { full: true, ..base() }),
        ("an allowed program", ClaudeSettings { allowed: &more_allowed, ..base() }),
        ("a refused file", ClaudeSettings { denied: &more_denied, ..base() }),
        ("the instructions (a mode's text, a house rule, the team)", ClaudeSettings { prompt: "the rules\n\nNever use red.", ..base() }),
        ("the model", ClaudeSettings { model: Some("haiku"), ..base() }),
        ("no model", ClaudeSettings { model: None, ..base() }),
        ("the effort", ClaudeSettings { effort: Some("low"), ..base() }),
        ("the game folder", ClaudeSettings { project: Some("/games/b"), ..base() }),
        ("no game folder", ClaudeSettings { project: None, ..base() }),
        ("the tool servers (a setting they are handed)", ClaudeSettings { servers: &other_servers, ..base() }),
        ("the tool-server file", ClaudeSettings { mcp_config: Path::new("/work/agent-tools/c2.json"), ..base() }),
        ("the provider-keys switch", ClaudeSettings { keys_on: true, ..base() }),
        ("the account (the second account's folder)", ClaudeSettings { config_dir: Some(Path::new("/app/accounts/claude-2-abcdef")), ..base() }),
        ("the program", ClaudeSettings { exe: Path::new("/other/claude"), ..base() }),
        ("the kit folder", ClaudeSettings { kit: Path::new("/kit2"), ..base() }),
    ];
    let mut seen = vec![same.clone()];
    for (what, s) in changed {
        let k = key(s);
        assert!(!seen.contains(&k), "{what} must start another process");
        seen.push(k);
    }
    // and the two commands a launch gives differ only in how the message goes in and the conversation resumed
    let l = ClaudeLaunch::new(&base());
    let args = |c: Command| c.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>();
    let (kept, per_turn) = (args(l.command(true, Some("S"))), args(l.command(false, Some("S"))));
    assert_eq!(kept[..4], ["-p", "--input-format", "stream-json", "--replay-user-messages"]);
    assert_eq!(kept[4..], per_turn[1..], "the same rules either way");
    assert_eq!(per_turn[per_turn.len() - 2..], ["--resume", "S"]);
    assert!(per_turn.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]) && per_turn.contains(&"Bash(git:*)".to_string()) && per_turn.contains(&"Edit(//kit/config.json)".to_string()));
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
    *mute*) sleep 300;;
    *slow*) sleep 1; echo "$init"; say "late"; echo "$ok";;
    *no-conv*) echo '{"type":"result","is_error":true,"subtype":"error_during_execution","session_id":"S0"}'
       echo "No conversation found with session ID: S0" >&2; exit 1;;
    *signed-out*) echo "$init"; echo '{"type":"result","is_error":true,"result":"Not logged in","session_id":"S1"}';;
    *out-of-usage*) echo "$init"
       echo '{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":4102444800,"rateLimitType":"five_hour"}}'
       echo '{"type":"result","is_error":true,"result":"You have hit your 5-hour limit, resets 12:50am","session_id":"S1"}';;
    *background*) echo "$init"; echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1"}]}'; say "started"; echo "$ok";;
    *in-a-tool*) echo "$init"
       echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__rbxkit__rbx_blender","input":{}}]}}'
       sleep 300 >/dev/null 2>&1 & echo $! > "$0.tool";;
    *die-late*) echo "$init"; say "half"; echo "boom" >&2; exit 3;;
    *hang*) echo "$init";;
    *'"text":"deaf'*) echo "$init"; trap '' TERM; while read -r l; do :; done; sleep 300;;
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
            ClaudeLaunch { exe: "/bin/sh".into(), first: vec![self.script.clone().into()], args: extra.iter().map(Into::into).collect(), cwd: self.script.parent().unwrap().into(), servers: String::new(), keys_on: false, config_dir: None }
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

        /// Waits until the running turn's process says `what` of it (Claude Code started on the message, a tool is open).
        fn until(&self, what: impl Fn(&TurnProc) -> bool) {
            let tp = self.a.st.lock().unwrap().proc_.clone().unwrap();
            let until = std::time::Instant::now() + Duration::from_secs(10);
            while !what(&tp) {
                assert!(std::time::Instant::now() < until);
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn started(&self) {
            self.until(|tp| tp.acked.load(Ordering::SeqCst));
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

    /// Within a few seconds (a process given up is closed beside the next turn, not before it).
    fn goes(pid: u32) -> bool {
        paths::leader_exited(pid, Duration::from_secs(10))
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
        assert!(goes(first), "the old process was closed");
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
        c.started();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed by the interrupt, not by the kill after STOP_WAIT: {:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."], "and no word of a kill");
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
        c.started();
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
    fn stop_before_claude_code_has_started_on_the_message_ends_the_process_at_once() {
        // a slow start (a long conversation resumed, tool servers connecting): an interrupt would end nothing and
        // the message would run afterwards, so nothing is written and the process goes
        let c = Chat::new("stop-early");
        assert!(c.start("slow"));
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "{:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."], "cleanly: no \"did not stop\"");
        assert!(c.said("text").is_empty(), "the message never ran");
        assert_eq!(c.pid(), None);
        assert!(goes(pid));
        assert_eq!(c.fails(), 0);
        assert!(c.send("again"));
        assert_eq!(c.said("text"), ["reply 1"]);
    }

    #[test]
    fn stop_during_a_tool_call_gives_the_process_up_with_what_its_tool_server_was_doing() {
        let c = Chat::new("stop-in-tool");
        assert!(c.start("in-a-tool"));
        let pid = c.pid().unwrap();
        c.until(|tp| tp.tool_open.load(Ordering::SeqCst));
        let tool: u32 = loop {
            if let Some(p) = fs::read_to_string(c.script.with_extension("sh.tool")).ok().and_then(|t| t.trim().parse().ok()) {
                break p;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(paths::signal_group(pid, "-0") && !gone(tool), "the call is at work, in the process's own group");
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed by the interrupt: {:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.pid(), None, "not kept: the next message must not wait behind the call");
        assert!(goes(pid));
        let until = std::time::Instant::now() + Duration::from_secs(10);
        let alive = |p: u32| Command::new("kill").args(["-0", &p.to_string()]).stderr(Stdio::null()).run_status().is_ok_and(|s| s.success());
        while alive(tool) && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(tool), "what the tool call had started is ended");
        assert_eq!(c.fails(), 0);
        assert!(c.send("again"));
        assert!(c.starts()[1].ends_with("--resume S1"), "{:?}", c.starts());
    }

    #[test]
    fn stop_or_closing_the_chat_while_the_message_is_being_sent_cancels_the_turn() {
        // Stop pressed after `send` began and before the turn exists is noted as `halted` (`stop_turn`)
        let c = Chat::new("stop-sending");
        assert!(c.send("one"));
        let pid = c.pid().unwrap();
        c.a.st.lock().unwrap().halted = true;
        assert!(c.send("two"), "handled: it goes nowhere else");
        assert_eq!(c.said("text"), ["reply 1"], "the message was not written");
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.pid(), Some(pid), "the process, which was told nothing, stays");
        c.a.st.lock().unwrap().halted = false;
        // with other arguments (a mode change): the old process is closed beside the cancelled turn
        c.a.st.lock().unwrap().halted = true;
        assert!(c.a.live_turn(&c.launch(&["--permission-mode", "bypassPermissions"]), "three"));
        c.wait();
        assert_eq!(c.said("text"), ["reply 1"]);
        assert!(goes(pid));
        // a chat closed meanwhile keeps nothing
        let closed = Chat::new("closed-sending");
        closed.a.st.lock().unwrap().closed = true;
        assert!(closed.send("one"));
        assert!(closed.said("text").is_empty());
        assert_eq!(closed.pid(), None);
    }

    #[test]
    fn a_process_that_never_answers_has_its_message_sent_per_turn_and_the_chat_stays_there() {
        let c = Chat::new("mute");
        let sent = std::time::Instant::now();
        assert!(c.send("mute"));
        assert!(sent.elapsed() >= START_WAIT, "it was given the wait");
        assert_eq!(c.said("text"), ["per turn"], "without the user typing it again");
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        assert_eq!(c.fails(), LIVE_FAILS);
        assert!(!c.a.live_allowed(), "for the rest of this run of the app");
        assert_eq!(c.pid(), None);
    }

    #[test]
    fn a_kept_process_that_cannot_resume_says_so_as_a_per_turn_one_does() {
        let c = Chat::new("no-conv");
        c.a.st.lock().unwrap().session = Some("S0".into());
        assert!(c.send("no-conv"));
        assert!(c.starts()[0].ends_with("--resume S0"));
        let errors = c.said("error");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("The memory of this earlier chat is no longer on this computer"), "{errors:?}");
        assert_eq!(c.a.st.lock().unwrap().session, None, "forgotten, so the next message starts afresh");
        assert_eq!(c.starts().len(), 1, "not sent a second time");
        assert_eq!((c.fails(), c.pid()), (0, None));
    }

    #[test]
    fn a_failed_turn_and_a_turn_that_left_a_background_task_keep_no_process() {
        let c = Chat::new("signed-out");
        assert!(c.send("signed-out"));
        // Claude Code's own "Not logged in" is said in the app's words, and the line carries the AI for its Sign in
        // button (not on Windows, where the app offers no sign-in and the tool's words stay)
        let line = if cfg!(windows) { "Not logged in" } else { "Claude is not signed in. Sign in, then send your message again." };
        assert_eq!(c.said("error"), [line]);
        let event = c.a.st.lock().unwrap().log.iter().rev().find(|e| e["kind"] == "error").cloned().unwrap();
        assert_eq!(event["sign_in"].as_str(), (!cfg!(windows)).then_some("claude"));
        assert_eq!((c.fails(), c.pid()), (0, None), "a new process gets to find the sign-in");
        assert!(c.send("again"));
        assert_eq!(c.starts().len(), 2);
        let b = Chat::new("background");
        assert!(b.send("background"));
        let first = b.starts().len();
        assert_eq!(b.said("text"), ["started"]);
        assert_eq!((b.fails(), b.pid()), (0, None), "Claude Code would begin a turn by itself when the task ends");
        assert!(b.send("again"));
        assert_eq!(b.starts().len(), first + 1);
        assert!(b.starts()[first].ends_with("--resume S1"));
    }

    #[test]
    fn a_kept_process_that_hits_the_limit_is_given_up_and_the_message_goes_again_for_the_other_account() {
        crate::accounts::stand_in(true, true);
        let c = Chat::new("limit");
        c.a.st.lock().unwrap().accounts_for_test = crate::accounts::stand_in_now();
        assert!(c.send("hello"));
        let first = c.pid().expect("kept for the chat");
        // (as `send` notes every message before it goes)
        c.a.st.lock().unwrap().sent = Some(("out-of-usage".into(), "out-of-usage".into(), Ask::default()));
        assert!(c.send("out-of-usage"));
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !c.a.st.lock().unwrap().log.iter().any(|e| e["kind"] == "rollover") {
            assert!(std::time::Instant::now() < until, "no rollover: {:?}", c.a.st.lock().unwrap().log);
            thread::sleep(Duration::from_millis(20));
        }
        // the reset time in Claude Code's event went through the reader into the turn's end: the account is used up
        // until then
        assert_eq!(crate::accounts::used_until("claude", 1, paths::now_secs()), Some(4_102_444_800.0));
        {
            let st = c.a.st.lock().unwrap();
            assert_eq!((st.account, st.session.clone(), st.resent_for_test.len()), (2, None, 1));
            assert_eq!(st.resent_for_test[0].resend, Some(("out-of-usage".to_string(), "out-of-usage".to_string())));
            assert!(working(&st), "held for the message that goes again");
        }
        // one line about it, the app's; the tool's own words are not said beside it
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        let line = c.a.st.lock().unwrap().log.iter().find(|e| e["kind"] == "rollover").unwrap()["text"].as_str().unwrap().to_string();
        assert!(line.starts_with("Your Claude account ran out of usage (back in about ") && line.ends_with("). Carrying on with account 2."), "{line}");
        assert_eq!(c.a.st.lock().unwrap().log.iter().filter(|e| e["kind"] == "done").count(), 1, "the first message's: this turn is not over");
        // the first account's process is not kept for it: the message goes to one started with the second's folder
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while c.pid().is_some() || !gone(first) {
            assert!(std::time::Instant::now() < until, "the first account's process is still there");
            thread::sleep(Duration::from_millis(20));
        }
        c.a.st.lock().unwrap().sending = false;
        let second = ClaudeLaunch { config_dir: Some("/app/accounts/claude-2-standin".into()), ..c.launch(&[]) };
        assert_ne!(second.key(), c.launch(&[]).key());
        assert!(c.a.live_allowed() && c.a.live_turn(&second, "hello again"));
        c.wait();
        assert_eq!(c.starts().len(), 2, "a process of its own for the other account");
        assert!(c.pid().is_some_and(|p| p != first));
    }

    #[test]
    fn a_kept_process_that_cannot_be_started_sends_the_turn_to_a_process_of_its_own() {
        let c = Chat::new("no-start");
        let mut launch = c.launch(&[]);
        launch.exe = "/nonexistent/claude".into();
        c.a.claude_turn(&launch, "hello");
        c.wait();
        assert_eq!(c.fails(), 1, "the kept way failed");
        // and the per-turn way was tried next with the same message (here it cannot start either, and says so)
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Could not start the agent"), "{errors:?}");
        // where the program is there, the same call ends in a reply: kept first...
        let ok = Chat::new("start");
        ok.a.claude_turn(&ok.launch(&[]), "hello");
        ok.wait();
        assert_eq!(ok.said("text"), ["reply 1"]);
        // ...and per turn once the kept way is off for the chat
        ok.a.st.lock().unwrap().live_fails = LIVE_FAILS;
        ok.a.close_live();
        ok.a.claude_turn(&ok.launch(&[]), "hello");
        ok.wait();
        assert_eq!(ok.said("text"), ["reply 1", "per turn"]);
        assert!(ok.starts()[1].starts_with("-p --resume S1") || ok.starts()[1] == "-p --resume S1", "{:?}", ok.starts());
    }

    #[test]
    fn quitting_leaves_no_kept_process() {
        let (idle, busy) = (Chat::new("quit-idle"), Chat::new("quit-busy"));
        assert!(idle.send("one"));
        assert!(busy.start("hang"));
        busy.started();
        let (p1, p2) = (idle.pid().unwrap(), busy.pid().unwrap());
        // as `shut_down` does: the running turns ended, then the kept processes
        let turns: Vec<Arc<TurnProc>> = [&idle, &busy].iter().filter_map(|c| c.a.stop_turn()).collect();
        assert_eq!(turns.len(), 1);
        for t in &turns {
            t.end();
        }
        end_kept(&[idle.a.clone(), busy.a.clone()], &turns);
        assert!(gone(p1), "the idle one is ended before `end_kept` returns");
        assert!(goes(p2));
        busy.wait();
        assert_eq!((idle.pid(), busy.pid()), (None, None));
        assert_eq!(busy.said("error"), ["Stopped."]);
        assert_eq!(busy.fails(), 0);
    }

    #[test]
    fn forcing_a_process_per_turn_is_one_capability_and_one_switch() {
        let c = Chat::new("caps");
        assert!(c.a.live_allowed());
        c.a.st.lock().unwrap().ai = Some("opencode".into());
        assert!(!c.a.live_allowed(), "only an AI the table says keeps a process");
    }
}
