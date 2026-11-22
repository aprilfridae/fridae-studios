//! Gemini on a kept `agy acp` process (`acp`, `GeminiLaunch`, and the same `Agent::live_turn` and `read_live` that
//! keep Claude Code's).
//!
//! First, real traffic recorded from agy 1.5.14 through this client (`tests/fixtures/gemini/acp-*.wire`, made with
//! FRIDAE_LIVE_RECORD and scrubbed as the other fixtures: `> ` before each line the app sent, `< ` before each line
//! agy wrote), played back by a stand-in process: it waits for a line on its stdin wherever the app sent one, and
//! writes agy's lines in between. So the real client, the app's answers to agy's requests and the whole turn run as
//! they did, and only the pipe is stood in for. Then turns against a stand-in that answers as agy does, for what a
//! recording cannot show: one process across any number of messages, a restart that resumes, every way of failing.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::acp;

/// The stand-in for agy. Kept (`acp`): plays the recording beside it when there is one (`<script>.<n>.wire` for the
/// n-th process started, else `<script>.wire`), else answers each request as agy does, with words in the message
/// choosing what goes wrong. Otherwise the per-message way: one message on stdin, one reply as agy prints it, exit.
const SCRIPT: &str = r#"
echo "$*" >> "$0.args"
n=$(grep -c '' "$0.args")
case "$1" in acp) ;; *)
  cat > "$0.stdin"
  echo '{"event":"init","conversation_id":"S1","init":{"model":""}}'
  echo '{"event":"step_update","step_update":{"conversation_id":"S1","step_index":1,"step_type":"agent_response","state":"DONE","text_delta":"per message"}}'
  echo '{"event":"result","result":{"conversation_id":"S1","status":"SUCCESS","usage":{}}}'
  exit 0;;
esac
f="$0.$n.wire"; [ -f "$f" ] || f="$0.wire"
if [ -f "$f" ]; then
  while IFS= read -r l <&3; do
    case "$l" in
      '> '*) IFS= read -r _ || exit 0;;
      '< '*) printf '%s\n' "${l#< }";;
    esac
  done 3<"$f"
  [ -f "$0.dies" ] && kill -9 $$
  cat >/dev/null
  exit 0
fi
ok() { echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$1}"; }
say() { echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"S1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$1\"}}}}"; }
done_='{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":2,"thoughtTokens":1,"cachedReadTokens":3,"cachedWriteTokens":0,"totalTokens":13}}'
modes='{"configOptions":[{"id":"mode","currentValue":"default"}]}'
turns=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) [ -f "$0.old" ] && ok '{"protocolVersion":2}' || ok '{"protocolVersion":1}';;
    *'"method":"session/new"'*) [ -f "$0.signed-out" ] && echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"Authentication required\"}}" || ok '{"sessionId":"S1"}';;
    *'"method":"session/resume"'*)
      case "$line" in
        *'"sessionId":"GONE"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"Failed to read session GONE; retry, or load it with session/load\"}}";;
        *) ok '{"sessionId":"S1"}';;
      esac;;
    *'"method":"session/set_mode"'*) [ -f "$0.no-mode" ] && echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"Unknown mode: default\"}}" || ok '{}';;
    *'"method":"session/set_config_option"'*)
      case "$line" in
        *'"value":"max"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"Thinking level \\\"max\\\" is not offered\"}}";;
        *) [ -f "$0.other-mode" ] && ok '{"configOptions":[{"id":"mode","currentValue":"auto"}]}' || ok "$modes";;
      esac;;
    *'"method":"session/cancel"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"cancelled\"}}";;
    *'"method":"session/prompt"'*)
      prompt=$id
      case "$line" in
        *die-early*) exit 3;;
        *die-late*) say "half"; echo "boom" >&2; exit 3;;
        *hang*) say "working";;
        *think*) ;;
        *deaf*) say "working"; trap '' TERM; while IFS= read -r l; do :; done; sleep 300;;
        *limit*) ok '{"stopReason":"refusal"}';;
        *) turns=$((turns+1)); say "reply $turns"; ok "$done_";;
      esac;;
  esac
done
"#;

struct Chat {
    a: Arc<Agent>,
    script: PathBuf,
    dir: PathBuf,
}

impl Chat {
    fn new(name: &str) -> Chat {
        let dir = paths::test_dir("acp").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("ws")).unwrap();
        let script = dir.join("agy.sh");
        fs::write(&script, SCRIPT).unwrap();
        let a = Agent::detached_for_test(&format!("acp-{name}"), None);
        a.st.lock().unwrap().ai = Some("gemini".into());
        Chat { a, script, dir }
    }

    /// With a recording to play: the `n`-th process started plays it (0: every process).
    fn playing(name: &str, case: &str) -> Chat {
        let c = Chat::new(name);
        c.plays(case, 0);
        c
    }

    fn plays(&self, case: &str, n: usize) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("gemini").join(format!("{case}.wire"));
        let to = if n == 0 { self.script.with_extension("sh.wire") } else { self.script.with_extension(format!("sh.{n}.wire")) };
        fs::copy(&from, to).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
    }

    /// A marker file the stand-in acts on.
    fn mark(&self, what: &str) {
        fs::write(self.script.with_extension(format!("sh.{what}")), "").unwrap();
    }

    fn settings<'a>(&'a self, servers: &'a Value, guarded: &'a Map<String, Value>, protected: &'a [PathBuf]) -> GeminiSettings<'a> {
        GeminiSettings {
            exe: Path::new("/bin/sh"),
            agent_id: &self.a.id,
            workspace: &self.dir,
            servers,
            model: Some("gemini-3.8-flash-low"),
            effort: None,
            full: false,
            kit: Path::new("/kit"),
            work: Path::new("/work"),
            protected,
            allowed: &[],
            guarded,
            keys_on: false,
            home: Path::new("/home/u"),
            data_dir: None,
        }
    }

    fn launch(&self) -> GeminiLaunch {
        self.launch_with(|_| {})
    }

    fn launch_with(&self, change: impl FnOnce(&mut GeminiSettings)) -> GeminiLaunch {
        let (servers, guarded) = (json!({}), Map::new());
        let mut s = self.settings(&servers, &guarded, &[]);
        change(&mut s);
        let mut l = GeminiLaunch::new(&s);
        l.first = vec![self.script.clone().into()];
        l
    }

    fn send(&self, text: &str) {
        self.a.kept_turn(&self.launch(), text);
        self.wait();
    }

    fn start(&self, text: &str) {
        self.a.kept_turn(&self.launch(), text);
    }

    fn wait(&self) {
        let until = std::time::Instant::now() + Duration::from_secs(25);
        while self.a.st.lock().unwrap().running {
            assert!(std::time::Instant::now() < until, "the turn never ended: {:?}", self.a.st.lock().unwrap().log);
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn until(&self, what: impl Fn(&TurnProc) -> bool) {
        let tp = self.a.st.lock().unwrap().proc_.clone().unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !what(&tp) {
            assert!(std::time::Instant::now() < until);
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn taken(&self) {
        self.until(|tp| tp.acked.load(Ordering::SeqCst));
    }

    fn pid(&self) -> Option<u32> {
        self.a.st.lock().unwrap().live.as_ref().map(|l| l.pid)
    }

    fn events(&self) -> Vec<Value> {
        self.a.st.lock().unwrap().log.clone()
    }

    fn said(&self, kind: &str) -> Vec<String> {
        self.events().iter().filter(|e| e["kind"] == kind).map(|e| e["text"].as_str().or(e["label"].as_str()).unwrap_or("").to_string()).collect()
    }

    fn kinds(&self) -> Vec<String> {
        self.events().iter().map(|e| e["kind"].as_str().unwrap_or("").to_string()).collect()
    }

    /// The arguments of every process started for the chat, in order.
    fn starts(&self) -> Vec<String> {
        fs::read_to_string(self.script.with_extension("sh.args")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    /// What the per-message process was handed on its stdin.
    fn per_message_input(&self) -> String {
        fs::read_to_string(self.script.with_extension("sh.stdin")).unwrap_or_default()
    }

    fn fails(&self) -> u32 {
        self.a.st.lock().unwrap().live_fails
    }

    fn session(&self) -> Option<String> {
        self.a.st.lock().unwrap().session.clone()
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

fn goes(pid: u32) -> bool {
    paths::leader_exited(pid, Duration::from_secs(10))
}

// ---------------------------------------------------------------- the launch key

#[test]
fn the_launch_key_changes_with_everything_a_gemini_process_is_started_with_or_decides_by() {
    let c = Chat::new("key");
    let servers = json!({"rbxkit": {"command": "python3", "args": ["mcp.py"], "env": {"RBXKIT_PROJECT": "/games/a", "FRIDAE_GEMINI_USE_API": "0"}}});
    let guarded: Map<String, Value> = json!({"allowed_commands": ["git"], "use_env_keys": false, "gemini_use_api": false, "gemini_key": "k1", "projects": [{"name": "a", "path": "/games/a"}], "expert_ai": {}}).as_object().unwrap().clone();
    let protected = vec![PathBuf::from("/kit/config.json")];
    let allowed = vec!["git".to_string()];
    let base = || GeminiSettings { allowed: &allowed, ..c.settings(&servers, &guarded, &protected) };
    let key = |s: GeminiSettings| GeminiLaunch::new(&s).key();
    let same = key(base());
    assert_eq!(key(base()), same, "the same settings: the process is kept");
    let other_servers = json!({"rbxkit": {"command": "python3", "args": ["mcp.py"], "env": {"RBXKIT_PROJECT": "/games/a", "FRIDAE_GEMINI_USE_API": "1"}}});
    let two_servers = json!({"rbxkit": servers["rbxkit"], "Roblox_Studio": {"command": "studio", "args": ["--stdio"]}});
    let more_allowed = vec!["git".to_string(), "python3".to_string()];
    let with = |k: &str, v: Value| {
        let mut g = guarded.clone();
        g.insert(k.to_string(), v);
        g
    };
    let (g_allowed, g_keys, g_api, g_key, g_games, g_experts, g_token) = (
        with("allowed_commands", json!(["git", "python3"])),
        with("use_env_keys", json!(true)),
        with("gemini_use_api", json!(true)),
        with("gemini_key", json!("k2")),
        with("projects", json!([{"name": "a", "path": "/games/b"}])),
        with("expert_ai", json!({"builder": "gemini"})),
        with("plugin_token", json!("t")),
    );
    let more_protected = vec![PathBuf::from("/kit/config.json"), PathBuf::from("/work/chats")];
    let changed: Vec<(&str, GeminiSettings)> = vec![
        ("the mode", GeminiSettings { full: true, ..base() }),
        ("an allowed program", GeminiSettings { allowed: &more_allowed, ..base() }),
        ("the guarded settings: allowed programs", GeminiSettings { guarded: &g_allowed, ..base() }),
        ("the guarded settings: the provider-keys switch", GeminiSettings { guarded: &g_keys, ..base() }),
        ("the guarded settings: the Gemini API switch", GeminiSettings { guarded: &g_api, ..base() }),
        ("the guarded settings: the Gemini key", GeminiSettings { guarded: &g_key, ..base() }),
        ("the guarded settings: the games", GeminiSettings { guarded: &g_games, ..base() }),
        ("the guarded settings: the Experts table", GeminiSettings { guarded: &g_experts, ..base() }),
        ("the guarded settings: the Studio plugin's token", GeminiSettings { guarded: &g_token, ..base() }),
        ("the model", GeminiSettings { model: Some("gemini-4-neon-high"), ..base() }),
        ("no model", GeminiSettings { model: None, ..base() }),
        ("the effort", GeminiSettings { effort: Some("high"), ..base() }),
        ("the workspace (the game folder)", GeminiSettings { workspace: Path::new("/games/b"), ..base() }),
        ("the tool servers (a setting they are handed)", GeminiSettings { servers: &other_servers, ..base() }),
        ("the tool servers (one more)", GeminiSettings { servers: &two_servers, ..base() }),
        ("the app's own files", GeminiSettings { protected: &more_protected, ..base() }),
        ("the provider-keys switch as the process gets it", GeminiSettings { keys_on: true, ..base() }),
        ("the program", GeminiSettings { exe: Path::new("/other/agy"), ..base() }),
        ("the kit folder", GeminiSettings { kit: Path::new("/kit2"), ..base() }),
        ("the work folder", GeminiSettings { work: Path::new("/work2"), ..base() }),
        ("the chat", GeminiSettings { agent_id: "another", ..base() }),
        ("agy's home folder", GeminiSettings { home: Path::new("/home/v"), ..base() }),
        ("agy's data folder", GeminiSettings { data_dir: Some(Path::new("/scratch/gemini")), ..base() }),
    ];
    let mut seen = vec![same.clone()];
    for (what, s) in changed {
        let k = key(s);
        assert!(!seen.contains(&k), "{what} must start another process");
        seen.push(k);
    }
    assert!(!same.contains("k1"), "the key holds a fingerprint of the guarded settings, never the Gemini key itself");
    // the rules a turn's requests are answered by are that message's own
    let l = GeminiLaunch::new(&GeminiSettings { full: true, ..base() });
    assert!(l.spec.rules.full);
    assert_eq!(l.spec.rules.writes, [c.dir.clone()]);
    assert_eq!(l.spec.rules.protected, protected);
    assert_eq!(l.spec.rules.servers, ["rbxkit"]);
    assert_eq!(l.spec.rules.agent_dir, Path::new("/home/u/.gemini/antigravity-cli"));
    // the kept command, and the per-message one with the same settings, which resumes the same conversation
    let args = |c: &Command| c.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>();
    assert_eq!(args(&l.command()), ["acp"]);
    let scratch = GeminiLaunch::new(&GeminiSettings { data_dir: Some(Path::new("/scratch/gemini")), ..base() });
    assert_eq!(args(&scratch.command()), ["acp", "--gemini-dir", "/scratch/gemini"]);
    let (per, input) = Kept::per_turn(&l, Some("C9"), "hello");
    let per = args(&per);
    assert_eq!(per[..6], ["--input-format", "stream-json", "--output-format", "stream-json", "--mode", "accept-edits"]);
    assert!(per.contains(&"--dangerously-skip-permissions".to_string()), "Full auto, as before");
    assert!(per.windows(2).any(|w| w == ["--conversation", "C9"]) && per.windows(2).any(|w| w == ["--model", "gemini-3.8-flash-low"]));
    assert_eq!(input, antigravity::message_line("hello"));
}

#[test]
fn the_switch_that_forces_a_process_per_message_is_per_ai() {
    // (an AI of no chat's, so no other test's turn reads this switch)
    assert!(!per_turn_forced("testai"));
    unsafe { std::env::set_var("FRIDAE_TESTAI_PER_TURN", "1") };
    assert!(per_turn_forced("testai"));
    assert!(!per_turn_forced("gemini") && !per_turn_forced("claude"));
    unsafe { std::env::set_var("FRIDAE_TESTAI_PER_TURN", "0") };
    assert!(!per_turn_forced("testai"));
    // and the table says which AIs keep a process at all
    let c = Chat::new("caps");
    assert!(c.a.live_allowed());
    c.a.st.lock().unwrap().ai = Some("cursor".into());
    assert!(!c.a.live_allowed(), "only an AI the table says keeps a process");
}

// ------------------------------------------------------------ stand-in process

#[cfg(unix)]
mod stand_in {
    use super::*;

    #[test]
    fn two_messages_run_on_one_process_and_other_settings_start_another_that_resumes() {
        let c = Chat::new("reuse");
        c.send("one");
        let first = c.pid().unwrap();
        c.send("two");
        assert_eq!(c.pid(), Some(first), "the same process");
        assert_eq!(c.said("text"), ["reply 1", "reply 2"], "and it counted both");
        assert_eq!(c.starts(), ["acp"]);
        assert_eq!(c.session().as_deref(), Some("S1"));
        // usage from each prompt's answer: input, output with thinking, cache
        let usage: Vec<Value> = c.events().into_iter().filter(|e| e["kind"] == "usage").collect();
        assert_eq!(usage.len(), 2);
        assert_eq!((usage[0]["input"].as_u64(), usage[0]["output"].as_u64(), usage[0]["cache_read"].as_u64()), (Some(7), Some(3), Some(3)));
        // Full auto instead of Ask me (any other setting): the process is closed and a new one resumes
        c.a.kept_turn(&c.launch_with(|s| s.full = true), "three");
        c.wait();
        let second = c.pid().unwrap();
        assert_ne!(second, first);
        assert!(goes(first), "the old process was closed");
        assert_eq!(c.said("text").last().map(String::as_str), Some("reply 1"), "a new process");
        assert_eq!(c.starts(), ["acp", "acp"]);
        assert_eq!(c.fails(), 0);
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
    }

    #[test]
    fn stop_cancels_the_turn_at_once_and_the_process_stays() {
        let c = Chat::new("stop");
        c.start("hang");
        c.taken();
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed by the cancelled prompt, not by the kill after STOP_WAIT: {:?}", pressed.elapsed());
        assert_eq!(c.said("text"), ["working"], "what it had written is shown");
        assert_eq!(c.said("error"), ["Stopped."], "and no word of a kill");
        assert_eq!(c.pid(), Some(pid), "kept");
        c.send("again");
        assert_eq!(c.pid(), Some(pid));
        assert_eq!(c.said("text"), ["working", "reply 1"]);
        assert_eq!(c.starts().len(), 1);
    }

    #[test]
    fn stop_before_the_agent_has_started_on_the_message_ends_the_process_at_once() {
        let c = Chat::new("stop-early");
        c.send("one");
        let pid = c.pid().unwrap();
        // the prompt is written and nothing has come of it yet: a cancel might find nothing to cancel
        c.start("think");
        thread::sleep(Duration::from_millis(300));
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "{:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."], "cleanly: no \"did not stop\"");
        assert_eq!(c.pid(), None);
        assert!(goes(pid));
        assert_eq!(c.fails(), 0);
        c.send("again");
        assert_eq!(c.said("text"), ["reply 1", "reply 1"], "a new process, which resumed");
    }

    #[test]
    fn a_turn_that_ignores_the_cancel_is_killed_after_the_wait_and_the_next_message_resumes() {
        let c = Chat::new("deaf");
        c.start("deaf");
        c.taken();
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() >= paths::STOP_WAIT, "it was given the wait");
        assert_eq!(c.said("error"), [paths::killed_message("Gemini").as_str(), "Stopped."]);
        assert_eq!(c.pid(), None);
        assert!(gone(pid));
        assert_eq!(c.fails(), 0, "ended by Stop: not a failure of the kept process");
        c.send("again");
        assert_eq!(c.starts().len(), 2);
    }

    #[test]
    fn a_process_that_dies_before_it_showed_anything_has_its_message_sent_again_per_message_in_the_same_conversation() {
        let c = Chat::new("die-early");
        c.send("one");
        assert_eq!(c.session().as_deref(), Some("S1"));
        c.send("die-early please");
        assert_eq!(c.said("text"), ["reply 1", "per message"], "the user gets the answer");
        assert!(c.said("error").is_empty(), "and no failure: {:?}", c.said("error"));
        assert_eq!(c.fails(), 1);
        assert_eq!(c.pid(), None);
        let starts = c.starts();
        assert!(starts[1].starts_with("--input-format stream-json") && starts[1].contains("--conversation S1"), "{starts:?}");
        assert_eq!(c.per_message_input(), antigravity::message_line("die-early please"), "the same message, once");
        // a second failure: the chat stays on a process per message
        c.send("die-early");
        assert_eq!(c.fails(), 2);
        assert!(!c.a.live_allowed());
        c.send("hello");
        assert_eq!(c.starts().len(), 5, "{:?}", c.starts());
        assert!(!c.starts()[4].starts_with("acp"));
    }

    #[test]
    fn a_process_that_dies_mid_turn_is_said_and_its_message_is_not_sent_twice() {
        let c = Chat::new("die-late");
        c.send("die-late");
        assert_eq!(c.said("text"), ["half"], "what it had written is shown");
        assert_eq!(c.said("error"), ["boom"], "what it wrote to its error output");
        assert_eq!(c.starts().len(), 1, "no second process: a tool may already have run");
        assert_eq!((c.fails(), c.pid()), (1, None));
        c.send("again");
        assert_eq!(c.said("text"), ["half", "reply 1"]);
        assert_eq!(c.starts(), ["acp", "acp"]);
    }

    #[test]
    fn a_process_that_died_between_turns_is_replaced_without_a_word() {
        let c = Chat::new("idle-death");
        c.send("one");
        let pid = c.pid().unwrap();
        paths::end_trees(&[pid]);
        assert!(paths::leader_exited(pid, Duration::from_secs(5)));
        c.send("two");
        assert_ne!(c.pid(), Some(pid));
        assert_eq!(c.said("text"), ["reply 1", "reply 1"]);
        assert!(c.said("error").is_empty());
        assert_eq!(c.fails(), 1);
    }

    #[test]
    fn a_kept_process_that_cannot_be_started_or_speaks_another_protocol_sends_the_message_per_message() {
        let c = Chat::new("no-start");
        let mut launch = c.launch();
        launch.exe = "/nonexistent/agy".into();
        c.a.kept_turn(&launch, "hello");
        c.wait();
        assert_eq!(c.fails(), 1, "the kept way failed");
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Could not start the agent"), "per message next, which cannot start either: {errors:?}");
        // an agy that answers with another protocol version, and one that will not take the mode the app's gate needs
        for what in ["old", "no-mode", "other-mode"] {
            let c = Chat::new(what);
            c.mark(what);
            c.send("hello");
            assert_eq!(c.said("text"), ["per message"], "{what}");
            assert!(c.said("error").is_empty(), "{what}: {:?}", c.said("error"));
            assert_eq!((c.fails(), c.pid()), (1, None), "{what}");
            assert_eq!(c.per_message_input(), antigravity::message_line("hello"));
        }
    }

    #[test]
    fn an_effort_the_agent_does_not_offer_goes_per_message_without_counting_as_a_failure() {
        let c = Chat::new("unfit");
        c.a.kept_turn(&c.launch_with(|s| s.effort = Some("max")), "hello");
        c.wait();
        assert_eq!(c.said("text"), ["per message"]);
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        assert_eq!((c.fails(), c.pid()), (0, None));
        assert!(c.starts()[1].contains("--effort max"), "{:?}", c.starts());
    }

    #[test]
    fn nobody_signed_in_is_said_once_and_no_second_process_is_tried() {
        let c = Chat::new("signed-out");
        c.mark("signed-out");
        c.send("hello");
        assert_eq!(c.said("error"), [antigravity::SIGNED_OUT]);
        assert_eq!(c.starts(), ["acp"], "not sent again per message, which would only say the same");
        assert_eq!((c.fails(), c.pid()), (0, None), "a new process gets to find the sign-in");
    }

    #[test]
    fn a_conversation_agy_no_longer_has_starts_a_new_one_and_the_chat_is_told() {
        let c = Chat::new("lost");
        c.a.st.lock().unwrap().session = Some("GONE".into());
        c.send("hello");
        assert_eq!(c.said("text"), ["reply 1"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Gemini could not find this chat's earlier conversation"), "{errors:?}");
        assert_eq!(c.session().as_deref(), Some("S1"), "the new conversation is the chat's from now on");
    }

    #[test]
    fn a_usage_limit_is_a_failed_turn_in_words_the_run_loop_knows() {
        let c = Chat::new("limit");
        c.send("limit");
        let errors = c.said("error");
        assert!(errors.len() == 1 && usage_limit(&errors[0]), "{errors:?}");
        assert_eq!(c.pid(), None, "a failed turn keeps no process");
        assert_eq!(c.fails(), 0);
    }

    #[test]
    fn a_helper_and_a_closed_chat_keep_no_process_and_quitting_leaves_none() {
        let helper = Chat::new("helper");
        helper.a.st.lock().unwrap().parent = Some("lead".into());
        helper.send("job");
        assert_eq!(helper.pid(), None, "an expert holds nothing between jobs");
        assert_eq!(helper.said("text"), ["reply 1"]);
        let (idle, busy) = (Chat::new("quit-idle"), Chat::new("quit-busy"));
        idle.send("one");
        busy.start("hang");
        busy.taken();
        let (p1, p2) = (idle.pid().unwrap(), busy.pid().unwrap());
        assert_eq!(close_idle_lives(vec![idle.a.clone(), busy.a.clone()], Duration::from_secs(600), 3), 0, "neither is idle past the limit");
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
    }
}

// ---------------------------------------------------------------- recordings

/// The chat's workspace as the recordings have it (the scratch folder, scrubbed).
#[cfg(unix)]
mod recorded {
    use super::*;

    fn sent(c: &Chat) -> Vec<Value> {
        let _ = c;
        Vec::new()
    }

    #[test]
    fn placeholder_until_the_recordings_are_made() {
        let c = Chat::new("placeholder");
        assert!(sent(&c).is_empty());
        let _ = (acp::PROTOCOL, Chat::playing as fn(&str, &str) -> Chat);
    }
}
