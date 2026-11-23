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
  cat > "$0.stdin"; conv=S1; prev=; for a in "$@"; do [ "$prev" = --conversation ] && conv=$a; prev=$a; done
  echo '{"event":"init","conversation_id":"'"$conv"'","init":{"model":""}}'
  echo '{"event":"step_update","step_update":{"conversation_id":"'"$conv"'","step_index":1,"step_type":"agent_response","state":"DONE","text_delta":"per message"}}'
  echo '{"event":"result","result":{"conversation_id":"'"$conv"'","status":"SUCCESS","usage":{}}}'
  exit 0;;
esac
f="$0.$n.wire"; [ -f "$f" ] || f="$0.wire"
sig() { printf '%s\n' "$1" | sed -n 's/^{\("id":[^,]*,\)\{0,1\}"jsonrpc":"2.0","method":"\([^"]*\)".*/\2/p'; }
if [ -f "$f" ]; then
  while IFS= read -r l <&3; do
    case "$l" in
      '> '*) IFS= read -r got || exit 0
        # what the app sends now against what it sent when this was recorded: a request by its method, an answer
        # to agy (the gate's yes or no) whole
        if [ ! -f "$0.loose" ]; then
          want="${l#> }"
          case "$want" in *'"method":"'*) w=$(sig "$want"); g=$(sig "$got");; *) w="$want"; g="$got";; esac
          [ "$w" = "$g" ] || printf 'recorded: %s\nsent now: %s\n' "$w" "$g" >> "$0.mismatch"
        fi;;
      '< '*) printf '%s\n' "${l#< }";;
    esac
  done 3<"$f"
  [ -f "$0.dies" ] && kill -9 $$
  cat >/dev/null
  exit 0
fi
ok() { echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$1}"; }
sid=S1
say() { echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$sid\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$1\"}}}}"; }
done_='{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":2,"thoughtTokens":1,"cachedReadTokens":3,"cachedWriteTokens":0,"totalTokens":13}}'
modes='{"configOptions":[{"id":"mode","currentValue":"default"}]}'
turns=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) [ -f "$0.old" ] && ok '{"protocolVersion":2}' || ok '{"protocolVersion":1}';;
    *'"method":"session/new"'*) [ -f "$0.signed-out" ] && echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"Authentication required\"}}" || ok '{"sessionId":"S1"}';;
    *'"method":"session/resume"'*)
      echo resume >> "$0.resumes"
      case "$line" in
        *'"sessionId":"GONE"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"Failed to read session GONE; retry, or load it with session/load\"}}";;
        *'"sessionId":"BROKEN"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"database is locked\"}}";;
        *'"sessionId":"BUSY"'*)
          if [ "$(grep -c '' "$0.resumes")" = 1 ]; then echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"database is locked\"}}"; else sid=BUSY; ok '{"sessionId":"BUSY"}'; fi;;
        *) ok '{"sessionId":"S1"}';;
      esac;;
    *'"method":"session/set_mode"'*) [ -f "$0.no-mode" ] && echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"Unknown mode: default\"}}" || ok '{}';;
    *'"method":"session/set_config_option"'*)
      case "$line" in
        *'"value":"max"'*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"Thinking level \\\"max\\\" is not offered\"}}";;
        *) [ -f "$0.other-mode" ] && ok '{"configOptions":[{"id":"mode","currentValue":"auto"}]}' || ok "$modes";;
      esac;;
    *'"method":"session/cancel"'*)
      if [ -n "$asks" ]; then
        # a request that comes after Stop: its answer is written down, then the prompt is answered
        echo '{"jsonrpc":"2.0","id":"late","method":"session/request_permission","params":{"sessionId":"S1","toolCall":{"toolCallId":"c9","name":"mcp_rbxkit_rbx_board","kind":"other","rawInput":{}},"options":[{"optionId":"allow","kind":"allow_once"},{"optionId":"deny","kind":"reject_once"}]}}'
        IFS= read -r reply; printf '%s\n' "$reply" > "$0.late"
      fi
      echo "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"cancelled\"}}";;
    *'"id":"asked"'*) printf '%s\n' "$line" > "$0.asked"; say "after the tool";;
    *'"method":"session/prompt"'*)
      prompt=$id
      case "$line" in
        *die-early*) exit 3;;
        *die-late*) say "half"; echo "boom" >&2; exit 3;;
        *hang*) say "working";;
        *asks*) asks=1; say "working"
          echo '{"jsonrpc":"2.0","id":"asked","method":"session/request_permission","params":{"sessionId":"S1","toolCall":{"toolCallId":"c8","name":"mcp_rbxkit_rbx_board","kind":"other","rawInput":{}},"options":[{"optionId":"allow","kind":"allow_once"},{"optionId":"deny","kind":"reject_once"}]}}';;
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
        // Gemini's kept process is asked for (it is not the default yet: `KEPT_ON_REQUEST`); every test here wants
        // it, and asks on its own chat, so the switch itself stays as the tests' environment has it
        let a = Agent::detached_for_test(&format!("acp-{name}"), None);
        a.st.lock().unwrap().ai = Some("gemini".into());
        a.st.lock().unwrap().kept_asked = true;
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
        let text = fs::read_to_string(&from).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        // the recording's scratch folder is this test's
        fs::write(to, text.replace("/scratch", &self.dir.display().to_string())).unwrap();
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
            hidden_in: &[],
            pictures: None,
        jobs: None,
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
        let (servers, guarded) = (json!({"rbxkit": {"command": "python3", "args": [], "env": {}}}), Map::new());
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
    // the one guarded setting no Gemini process is started with: the model picked for OpenCode
    let mut g_opencode = guarded.clone();
    g_opencode.insert("opencode_model".into(), json!("opencode-go/kimi-k3"));
    g_opencode.insert("opencode_model_signed_in".into(), json!(true));
    assert_eq!(key(GeminiSettings { guarded: &g_opencode, ..base() }), same, "picking a model for OpenCode keeps the process");
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
    let home_dirs = vec![PathBuf::from("/home/u")];
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
        ("the home folder whose hidden files are kept from it", GeminiSettings { hidden_in: &home_dirs, ..base() }),
        ("where its tool server notes pictures", GeminiSettings { pictures: Some(Path::new("/work/tool-pictures/c1.jsonl")), ..base() }),
        ("where its tool server notes its jobs", GeminiSettings { jobs: Some(Path::new("/work/tool-jobs/c1.jsonl")), ..base() }),
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
    assert_eq!(l.spec.rules.protected, [protected[0].clone(), c.dir.join(".agents").join("mcp_config.json")], "and the workspace's list of tool servers");
    // the app's tool server is told where to note its pictures, on the kept path only
    let notes = Path::new("/work/tool-pictures/c1.jsonl");
    let with_notes = GeminiLaunch::new(&GeminiSettings { pictures: Some(notes), ..base() });
    assert_eq!(with_notes.spec.servers["rbxkit"]["env"]["FRIDAE_TOOL_PICTURES"], "/work/tool-pictures/c1.jsonl");
    assert_eq!(with_notes.spec.pictures.as_deref(), Some(notes));
    let (per, _) = Kept::per_turn(&with_notes, None, "hello");
    assert!(!fs::read_to_string(c.dir.join(".agents").join("mcp_config.json")).unwrap().contains("FRIDAE_TOOL_PICTURES"), "a process per message hands its results over itself");
    drop(per);
    // agy cuts a tool call off after three minutes over ACP: the app's tool server is given 150 seconds to answer
    // in and the file the turn's end is told through, on the kept path only (a process per message has 15 minutes)
    let with_jobs = GeminiLaunch::new(&GeminiSettings { jobs: Some(Path::new("/work/tool-jobs/c1.jsonl")), ..base() });
    assert_eq!(with_jobs.spec.servers["rbxkit"]["env"]["RBXKIT_CALL_BUDGET"], "150");
    assert_eq!(with_jobs.spec.servers["rbxkit"]["env"]["FRIDAE_TOOL_JOBS"], "/work/tool-jobs/c1.jsonl");
    assert!(l.spec.servers["rbxkit"]["env"].get("RBXKIT_CALL_BUDGET").is_none(), "never a budget without that file");
    let (per, _) = Kept::per_turn(&with_jobs, None, "hello");
    let list = fs::read_to_string(c.dir.join(".agents").join("mcp_config.json")).unwrap();
    assert!(list.contains("rbxkit") && !list.contains("RBXKIT_CALL_BUDGET") && !list.contains("FRIDAE_TOOL_JOBS"), "a process per message: no budget, the tool server as ever");
    drop(per);
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
    // an AI whose kept process is not the default yet is kept only when asked for
    assert!(KEPT_ON_REQUEST.contains(&"gemini") && KEPT_ON_REQUEST.contains(&"cursor") && !KEPT_ON_REQUEST.contains(&"claude"));
    assert!(kept_wanted("claude"));
    unsafe { std::env::remove_var("FRIDAE_TESTAI_KEPT") };
    assert!(kept_wanted("testai"), "not on the list: the default");
    // and the table says which AIs keep a process at all
    let c = Chat::new("caps");
    assert!(c.a.live_allowed(), "asked for by this chat");
    c.a.st.lock().unwrap().ai = Some("opencode".into());
    assert!(!c.a.live_allowed(), "only an AI the table says keeps a process");
}

/// Gemini's kept process is on request (`KEPT_ON_REQUEST`): with its switch unset, or set to anything but "1", a
/// Gemini chat runs a process per message, exactly as before the kept path existed.
#[test]
fn gemini_is_not_kept_unless_its_switch_asks_for_it() {
    for ai in ["gemini", "cursor"] {
        assert!(!kept_wanted_with(ai, None), "{ai}: unset");
        assert!(!kept_wanted_with(ai, Some("0")) && !kept_wanted_with(ai, Some("")) && !kept_wanted_with(ai, Some("true")), "{ai}: only \"1\" asks");
        assert!(kept_wanted_with(ai, Some("1")), "{ai}");
    }
    assert!(kept_wanted_with("claude", None), "Claude's is the default");
    // a whole turn, where the tests' own environment leaves the switch alone (no test sets it: each asks on its own
    // chat): no `agy acp` is started, the message goes to a process per message and is answered
    if std::env::var_os("FRIDAE_GEMINI_KEPT").is_some() || cfg!(windows) {
        return;
    }
    let c = Chat::new("not-asked");
    c.a.st.lock().unwrap().kept_asked = false;
    assert!(!kept_wanted("gemini") && !c.a.live_allowed());
    c.send("hello");
    assert_eq!(c.said("text"), ["per message"]);
    assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
    let starts = c.starts();
    assert!(starts.len() == 1 && starts[0].starts_with("--input-format stream-json"), "one process, the per-message one: {starts:?}");
    assert_eq!((c.pid(), c.fails()), (None, 0), "nothing kept, and nothing counted as a failure of keeping");
    c.send("again");
    assert!(c.starts().iter().all(|s| !s.starts_with("acp")), "{:?}", c.starts());
    assert!(c.starts()[1].contains("--conversation S1"), "the same conversation, resumed per message: {:?}", c.starts());
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
    fn a_resume_that_fails_once_is_tried_again_and_only_a_conversation_that_is_gone_starts_a_new_one() {
        // the store let go of a moment later (another process of the chat's on its way out): the second try works,
        // and the chat keeps its conversation without a word
        let busy = Chat::new("resume-busy");
        busy.a.st.lock().unwrap().session = Some("BUSY".into());
        busy.send("hello");
        assert_eq!(busy.said("text"), ["reply 1"]);
        assert!(busy.said("error").is_empty(), "{:?}", busy.said("error"));
        assert_eq!(busy.session().as_deref(), Some("BUSY"));
        assert_eq!(fs::read_to_string(busy.script.with_extension("sh.resumes")).unwrap().lines().count(), 2);
        // an error that is not "no such conversation", twice: the kept way is given up, the message goes per message,
        // and the conversation is still the chat's
        let broken = Chat::new("resume-broken");
        broken.a.st.lock().unwrap().session = Some("BROKEN".into());
        broken.send("hello");
        assert_eq!(broken.said("text"), ["per message"]);
        assert!(broken.said("error").is_empty(), "no word of a lost conversation: {:?}", broken.said("error"));
        assert!(broken.starts()[1].contains("--conversation BROKEN"), "{:?}", broken.starts());
        assert_eq!(broken.fails(), 1);
        // gone (agy's own words for it), twice: a new conversation, said
        let gone = Chat::new("resume-gone");
        gone.a.st.lock().unwrap().session = Some("GONE".into());
        gone.send("hello");
        assert_eq!(fs::read_to_string(gone.script.with_extension("sh.resumes")).unwrap().lines().count(), 2, "asked twice before it is given up");
        assert_eq!(gone.session().as_deref(), Some("S1"));
    }

    #[test]
    fn a_request_is_answered_by_the_gate_and_one_that_comes_after_stop_is_answered_cancelled() {
        let c = Chat::new("late-request");
        c.start("asks");
        c.taken();
        // in the turn: one of the app's tools, let through
        let asked = c.script.with_extension("sh.asked");
        let until = std::time::Instant::now() + Duration::from_secs(10);
        // (until the stand-in has written the whole line, not only made the file)
        while !fs::read_to_string(&asked).is_ok_and(|t| t.ends_with('\n')) && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        let first: Value = serde_json::from_str(&fs::read_to_string(&asked).unwrap()).unwrap();
        assert_eq!(first["result"], json!({"outcome": {"outcome": "selected", "optionId": "allow"}}));
        // after Stop: nothing more is let through, whatever it is
        c.a.stop();
        c.wait();
        let late: Value = serde_json::from_str(&fs::read_to_string(c.script.with_extension("sh.late")).unwrap()).unwrap();
        assert_eq!(late["id"], "late");
        assert_eq!(late["result"], json!({"outcome": {"outcome": "cancelled"}}));
        assert_eq!(c.said("error"), ["Stopped."]);
    }

    #[test]
    fn a_new_process_does_not_touch_the_conversation_while_the_chats_old_one_is_still_there() {
        let c = Chat::new("one-at-a-time");
        c.send("one");
        let old = c.a.st.lock().unwrap().live.clone().unwrap();
        // the old process is on its way out (as when the mode changes) and takes its time
        retire(&mut c.a.st.lock().unwrap(), &old);
        let sent = std::time::Instant::now();
        let closer = {
            let old = old.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(700));
                old.close();
            })
        };
        c.a.st.lock().unwrap().live = None;
        c.a.kept_turn(&c.launch_with(|s| s.full = true), "two");
        c.wait();
        assert!(sent.elapsed() >= Duration::from_millis(700), "the new turn waited for the old process: {:?}", sent.elapsed());
        assert!(old.reaped());
        assert_eq!(c.said("text"), ["reply 1", "reply 1"]);
        assert!(c.a.st.lock().unwrap().retiring.iter().all(|l| l.reaped()));
        closer.join().unwrap();
        // Claude Code's turns do not wait (its new process was measured beside its old one)
        assert!(Kept::alone(&c.launch()));
    }

    #[test]
    fn the_workspaces_list_of_tool_servers_is_changed_whole_and_put_back_after_the_turn() {
        let c = Chat::new("server-list");
        let list = c.dir.join(".agents").join("mcp_config.json");
        fs::create_dir_all(list.parent().unwrap()).unwrap();
        // as a process per message left it: the app's server, and one of the user's own
        let before = r#"{"mcpServers": {"rbxkit": {"command": "python3"}, "mine": {"command": "my-server"}}}"#;
        fs::write(&list, before).unwrap();
        let servers = json!({"rbxkit": {"command": "python3", "args": [], "env": {}}});
        // for the turn: without the app's entry (agy would start it a second time), the user's kept
        let was = antigravity::unwrite_servers(&c.dir, &servers).expect("changed");
        let during: Value = serde_json::from_str(&fs::read_to_string(&list).unwrap()).unwrap();
        assert_eq!(during["mcpServers"], json!({"mine": {"command": "my-server"}}));
        antigravity::rewrite_list(&c.dir, &was);
        assert_eq!(fs::read_to_string(&list).unwrap(), before, "put back as it was, byte for byte");
        // nothing of the app's in it, a file that does not read, no file: left alone
        assert!(antigravity::unwrite_servers(&c.dir, &json!({"other": {}})).is_none());
        fs::write(&list, "{ not json").unwrap();
        assert!(antigravity::unwrite_servers(&c.dir, &servers).is_none());
        assert_eq!(fs::read_to_string(&list).unwrap(), "{ not json");
        fs::remove_file(&list).unwrap();
        assert!(antigravity::unwrite_servers(&c.dir, &servers).is_none());
        // no half-written file is ever left beside it
        assert_eq!(fs::read_dir(list.parent().unwrap()).unwrap().count(), 0);
        // and a whole turn, also one that ends badly, leaves the list as it found it
        fs::write(&list, before).unwrap();
        let with_server = |c: &Chat, text: &str| {
            let guarded = Map::new();
            let mut l = GeminiLaunch::new(&GeminiSettings { servers: &servers, ..c.settings(&json!({}), &guarded, &[]) });
            l.first = vec![c.script.clone().into()];
            c.a.kept_turn(&l, text);
            c.wait();
        };
        with_server(&c, "hello");
        assert_eq!(c.said("text"), ["reply 1"]);
        assert_eq!(fs::read_to_string(&list).unwrap(), before);
        with_server(&c, "die-late");
        assert_eq!(fs::read_to_string(&list).unwrap(), before);
    }

    #[test]
    fn a_list_of_tool_servers_that_is_a_symbolic_link_is_neither_written_through_nor_replaced() {
        // a game folder can come with `.agents/mcp_config.json` as a link to any file (a shell start-up file, the
        // app's settings): the app made no such link, so it writes nothing through it and does not replace it
        let c = Chat::new("linked-list");
        let (target, list) = (c.dir.join("elsewhere").join("zshrc"), c.dir.join(".agents").join("mcp_config.json"));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(list.parent().unwrap()).unwrap();
        fs::write(&target, "export PATH=/usr/bin\n").unwrap();
        std::os::unix::fs::symlink(&target, &list).unwrap();
        let is_link = || list.symlink_metadata().unwrap().file_type().is_symlink();
        let untouched = || fs::read_to_string(&target).unwrap() == "export PATH=/usr/bin\n";
        let servers = json!({"rbxkit": {"command": "python3", "args": [], "env": {}}});
        assert!(antigravity::list_is_link(&c.dir));
        antigravity::write_servers(&c.dir, &servers);
        assert!(is_link() && untouched(), "a process per message's list");
        // a link to a list that does read as one (the user's own, shared between games) is left alone as well
        fs::write(&target, r#"{"mcpServers": {"rbxkit": {"command": "other"}, "mine": {"command": "my-server"}}}"#).unwrap();
        let shared = fs::read_to_string(&target).unwrap();
        antigravity::write_servers(&c.dir, &servers);
        assert!(antigravity::unwrite_servers(&c.dir, &servers).is_none(), "a kept turn takes nothing out of it");
        antigravity::rewrite_list(&c.dir, b"{}");
        assert!(is_link());
        assert_eq!(fs::read_to_string(&target).unwrap(), shared);
        // a link to a file that is not there, and the folder itself as a link: nothing is made
        fs::remove_file(&list).unwrap();
        std::os::unix::fs::symlink("../elsewhere/later.json", &list).unwrap();
        antigravity::write_servers(&c.dir, &servers);
        assert!(is_link() && !c.dir.join("elsewhere").join("later.json").exists());
        fs::remove_file(&list).unwrap();
        fs::remove_dir(c.dir.join(".agents")).unwrap();
        std::os::unix::fs::symlink(c.dir.join("elsewhere"), c.dir.join(".agents")).unwrap();
        antigravity::write_servers(&c.dir, &servers);
        assert!(antigravity::list_is_link(&c.dir) && !c.dir.join("elsewhere").join("mcp_config.json").exists());
        for dir in [c.dir.join("elsewhere")] {
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no half-written file left in {}", dir.display());
        }
        // the chat is told, once per message that goes to a process per message
        c.a.st.lock().unwrap().kept_asked = false;
        c.send("hello");
        assert_eq!(c.said("error"), [antigravity::LIST_IS_A_LINK]);
        assert_eq!(c.said("text"), ["per message"]);
        // a plain file is written as before
        fs::remove_file(c.dir.join(".agents")).unwrap();
        antigravity::write_servers(&c.dir, &servers);
        assert!(fs::read_to_string(&list).unwrap().contains("rbxkit") && !antigravity::list_is_link(&c.dir));
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

/// Real traffic, played back (see the top of this file). The recordings name the scratch folder they were made in as
/// `/scratch`; it becomes the test's own folder as the recording is handed to the stand-in, so the app's answers to
/// agy's requests are decided on folders that exist, as they were when it was recorded. All but one were made
/// through the app with Gemini 3.8 Flash at low effort. The stand-in compares each line the app sends with the line
/// it sent when recorded (a request by its method, an answer to agy whole): `clean` says none differed.
#[cfg(unix)]
mod recorded {
    use super::*;

    const S1: &str = "00000000-0000-4000-8000-0000000000a1";
    const S2: &str = "00000000-0000-4000-8000-0000000000a2";

    /// How a recording's messages were sent.
    #[derive(Clone, Copy)]
    struct As {
        effort: Option<&'static str>,
        full: bool,
    }

    const ASK_ME: As = As { effort: Some("low"), full: false };

    impl Chat {
        fn recorded(name: &str, case: &str) -> Chat {
            let c = Chat::playing(name, case);
            fs::create_dir_all(c.dir.join("game")).unwrap();
            fs::create_dir_all(c.dir.join("kit")).unwrap();
            fs::write(c.dir.join("kit").join("config.json"), "{}").unwrap();
            c
        }

        /// A message sent as the recordings' were: the game as the workspace, the app's two tool servers, the app's
        /// settings file among what no file tool may write.
        fn say(&self, text: &str, how: As) {
            let servers = json!({"rbxkit": {"command": "python3", "args": ["/scratch/kit/rbxkit_mcp.py"], "env": {"RBXKIT_PROJECT": "/scratch/game"}},
                                 "Roblox_Studio": {"command": "/usr/bin/python3", "args": ["/scratch/studio_standin.py"]}});
            let (guarded, game, protected) = (Map::new(), self.dir.join("game"), vec![self.dir.join("kit").join("config.json")]);
            let mut l = GeminiLaunch::new(&GeminiSettings { workspace: &game, servers: &servers, effort: how.effort, full: how.full, ..self.settings(&json!({}), &guarded, &protected) });
            l.first = vec![self.script.clone().into()];
            self.a.kept_turn(&l, text);
        }

        fn ask(&self, text: &str) {
            self.say(text, ASK_ME);
            self.wait();
        }

        /// Each turn's usage as the chat was told it: input, output, read from the cache.
        fn tokens(&self) -> Vec<(u64, u64, u64)> {
            self.events().iter().filter(|e| e["kind"] == "usage").map(|e| (e["input"].as_u64().unwrap(), e["output"].as_u64().unwrap(), e["cache_read"].as_u64().unwrap())).collect()
        }

        /// The user pressed "Run it once" on the card for this command, as when the recording was made (or, for the
        /// recordings made while the app still approved allowed programs by itself, in place of that).
        fn once(&self, command: &str) {
            opencode::note_refused(&self.a.id, &[json!({"command": command, "once": true})]);
            assert!(opencode::approve_once(&self.a.id, command));
        }

        /// Every line the app sent was the line it sent when this was recorded.
        fn clean(&self) {
            let diff = fs::read_to_string(self.script.with_extension("sh.mismatch")).unwrap_or_default();
            assert!(diff.is_empty(), "the app no longer sends what it sent when this was recorded:\n{diff}");
        }

        /// What the app answered agy's permission requests in the recording being played, in order.
        fn recorded_answers(&self) -> Vec<String> {
            let all = fs::read_to_string(self.script.with_extension("sh.wire")).unwrap();
            all.lines().filter_map(|l| l.strip_prefix("> ")).filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|m| m["result"]["outcome"]["optionId"].as_str().map(str::to_string)).collect()
        }
    }

    #[test]
    fn recorded_tool_calls_allowed_one_refused_with_its_card_then_a_plain_reply_on_the_same_process() {
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = paths::set_allowed(&["ls".to_string()], false);
        // recorded: one of the app's tools, a file written in the game, a file read there (agy does not ask about a
        // read inside its workspace), then `ls -la`, a program the user had not allowed: refused, and agy ended the
        // turn there without a word
        let c = Chat::recorded("rec-two", "acp-two");
        c.ask("the first message");
        assert_eq!(c.said("tool"), ["Updating its task list", "Writing a file", "Reading a file", "Running a command"]);
        let ev = c.events();
        let card = ev.iter().find(|e| e["kind"] == "blocked").expect("the card that offers to allow the program");
        assert_eq!(card["commands"], json!([{"name": "ls", "command": "ls -la"}]));
        assert!(c.said("text").is_empty() && c.said("error").is_empty(), "{:?}", c.kinds());
        assert_eq!(c.kinds().last().map(String::as_str), Some("done"));
        assert_eq!(c.tokens(), [(26959, 251, 55103)], "usage from the prompt's answer");
        assert_eq!(c.session().as_deref(), Some(S1));
        assert!(c.a.notes_for_test().iter().any(|n| n.contains("ls -la")), "the agent is told with the next message what was not run");
        let pid = c.pid().expect("a refusal is no failure: the process is kept");
        // the second message, on the same process: a plain reply
        c.ask("the second message");
        assert_eq!(c.pid(), Some(pid));
        assert_eq!(c.said("text"), ["I need to run the shell command `sleep 61` to wait for 61 seconds as requested."]);
        assert_eq!(c.tokens().len(), 2);
        assert_eq!(c.starts(), ["acp"], "one process for both");
        assert_eq!(c.fails(), 0);
        assert_eq!(c.recorded_answers(), ["allow", "allow", "deny"]);
        c.clean();
    }

    #[test]
    fn recorded_turn_that_left_a_command_in_the_background_keeps_no_process() {
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = paths::set_allowed(&["ls".to_string()], false);
        // recorded: `sleep 61` was let run, went to the background after five seconds, and agy ended the turn with
        // the tool call still open. A minute later, with nothing sent, it asked the model again and wrote "done".
        let c = Chat::recorded("rec-background", "acp-background");
        c.ask("the first message");
        c.ask("the second message");
        let pid = c.pid().unwrap();
        c.once("sleep 61");
        c.ask("the third message");
        assert_eq!(c.said("text").last().map(String::as_str), Some("Launched the command `sleep 61` in the background and waiting for it to finish."));
        assert_eq!(c.pid(), None, "not kept: what it would do by itself later is never left to run");
        assert!(goes(pid));
        assert_eq!(c.fails(), 0);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].contains("left a command running"), "said in one line: {errors:?}");
        assert!(!c.said("text").iter().any(|t| t.contains("done")), "the answer nobody asked for is not the chat's");
        // and that answer, coming with no turn on the process, is what ends a process (`Live::strayed`)
        let all = fs::read_to_string(c.script.with_extension("sh.wire")).unwrap();
        let after: Vec<&str> = all.lines().skip_while(|l| !l.contains("\"id\":8}")).skip(1).filter_map(|l| l.strip_prefix("< ")).collect();
        assert_eq!(after.len(), 3);
        assert_eq!(after.iter().map(|l| acp::stray(l.as_bytes())).collect::<Vec<_>>(), [false, true, false], "the tool's last update, the answer, usage");
        c.clean();
    }

    #[test]
    fn recorded_stop_mid_turn_is_confirmed_by_the_cancelled_prompt() {
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = paths::set_allowed(&["ls".to_string()], false);
        // recorded: Stop pressed in the page while `sleep 61` ran: `session/cancel`, the command's tool call failed,
        // and the prompt answered `cancelled`. (The recording is of a process's first, second and fourth turns: the
        // third, the background one above, is left out, so its request number is passed over here.)
        let c = Chat::recorded("rec-stop", "acp-stop");
        c.ask("the first message");
        c.ask("the second message");
        let pid = c.pid().unwrap();
        c.a.st.lock().unwrap().live.as_ref().unwrap().next_request();
        c.once("sleep 61");
        c.say("the message that is stopped", ASK_ME);
        c.until(|tp| tp.tool_open.load(Ordering::SeqCst) && tp.acked.load(Ordering::SeqCst));
        thread::sleep(Duration::from_millis(200)); // the request for `sleep 61` is answered by now
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed, not killed after the wait: {:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.said("tool").last().map(String::as_str), Some("Running a command"));
        assert_eq!(c.tokens().last(), Some(&(2127, 41, 20012)), "a cancelled prompt still says what it used");
        assert_eq!(c.pid(), None, "Stop landed in a tool call, so the process is given up, as Claude Code's is");
        assert!(goes(pid));
        assert_eq!(c.fails(), 0);
        c.clean(); // the line the app sent for Stop was `session/cancel`, where it was when recorded
    }

    #[test]
    fn recorded_process_killed_mid_turn_is_said_and_the_message_is_not_sent_twice() {
        // recorded: a new process resumed the chat's conversation, `sleep 61` was let run, and the process was
        // killed (SIGKILL) while it ran: the output ends with the prompt unanswered
        let c = Chat::recorded("rec-dies", "acp-dies");
        c.mark("dies");
        c.a.st.lock().unwrap().session = Some(S1.into());
        c.once("sleep 61");
        c.ask("the message");
        assert_eq!(c.said("tool"), ["Running a command"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("The agent exited unexpectedly"), "{errors:?}");
        assert_eq!(c.starts(), ["acp"], "no second process: the command had already run");
        assert_eq!((c.fails(), c.pid()), (1, None));
        assert_eq!(c.session().as_deref(), Some(S1), "the next message resumes it");
        c.clean();
    }

    #[test]
    fn recorded_process_that_dies_at_its_start_has_the_message_sent_per_message() {
        // recorded: the kept process was killed as it started, before it answered anything (all there is of it is
        // the app's first line); live, the message then went to a process per message and was answered
        let c = Chat::recorded("rec-dies-at-start", "acp-dies-at-start");
        c.mark("dies");
        c.ask("Reply with exactly: ok");
        assert_eq!(c.said("text"), ["per message"]);
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        assert_eq!(c.per_message_input(), antigravity::message_line("Reply with exactly: ok"), "the same message, once");
        assert_eq!((c.fails(), c.pid()), (1, None));
        assert_eq!(c.starts().len(), 2);
        c.clean();
    }

    #[test]
    fn recorded_signed_out() {
        // recorded: `session/new` answered `Authentication required`
        let c = Chat::recorded("rec-signed-out", "acp-signed-out");
        c.say("hello", As { effort: Some("high"), full: false });
        c.wait();
        assert_eq!(c.said("error"), [antigravity::SIGNED_OUT]);
        assert_eq!(c.kinds(), ["error", "done"]);
        assert_eq!(c.starts(), ["acp"]);
        assert_eq!((c.fails(), c.pid(), c.session()), (0, None, None));
        c.clean();
    }

    #[test]
    fn recorded_conversation_made_by_a_process_per_message_carries_on_and_a_write_outside_the_chat_is_refused() {
        // recorded by the research probe (its own client; no effort set, so its requests come in the app's number and
        // order, but they are its own, so what the app sends is not compared line by line here):
        // a conversation made by `agy` per message was resumed over ACP and carried on. agy kept the folder the
        // conversation was saved with, and asked to write there: not this chat's folder, so the app says no, as the
        // probe did, and agy ended the turn.
        let c = Chat::recorded("rec-resumed", "acp-resumed-per-message");
        c.mark("loose");
        c.a.st.lock().unwrap().session = Some(S2.into());
        c.say("the message", As { effort: None, full: false });
        c.wait();
        assert_eq!(c.said("tool"), ["Writing a file"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].contains("h3a/home/gemini/chat1/note.txt") && errors[0].contains("outside this chat's folder"), "{errors:?}");
        assert!(!c.dir.join("h3a").exists());
        assert_eq!(c.tokens(), [(9907, 97, 0)]);
        assert_eq!(c.session().as_deref(), Some(S2), "the same conversation: no word of a new one");
        assert!(c.pid().is_some());
        assert_eq!(c.starts(), ["acp"]);
        assert_eq!(c.recorded_answers(), ["deny"]);
    }

    #[test]
    fn recorded_read_outside_the_chats_folders_is_asked_about_and_refused() {
        // recorded: agy asked before reading a file outside its workspace (it does not for one inside, above), and the
        // app said no
        let c = Chat::recorded("rec-read-outside", "acp-read-outside");
        c.ask("the message");
        assert_eq!(c.said("tool"), ["Reading a file"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].contains("outside/secret.txt") && errors[0].contains("outside this chat's folders"), "{errors:?}");
        assert!(c.said("text").is_empty());
        assert_eq!(c.recorded_answers(), ["deny"]);
        assert!(c.pid().is_some());
        c.clean();
    }

    #[test]
    fn recorded_command_gets_its_card_and_runs_once_the_user_says_so_on_the_same_process() {
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = paths::set_allowed(&["whichshell".to_string()], true);
        // recorded on the last build: a command whose program the user had allowed is still asked about (agy runs
        // commands with its own shell and the user's start-up file), with the offer to run it once
        let c = Chat::recorded("rec-once", "acp-command-once");
        c.ask("the first message");
        let ev = c.events();
        let card = ev.iter().find(|e| e["kind"] == "refused").expect("the card");
        assert_eq!((card["commands"][0]["command"].clone(), card["commands"][0]["once"].clone()), (json!("whichshell two"), json!(true)));
        assert!(!ev.iter().any(|e| e["kind"] == "blocked"), "its program is allowed: no offer to allow it");
        let pid = c.pid().unwrap();
        // "Run it once", then the page's message: the same command runs, and one of the app's tools after it
        assert!(opencode::approve_once(&c.a.id, "whichshell two"));
        c.ask("the second message");
        let _ = paths::set_allowed(&["whichshell".to_string()], false);
        assert_eq!(c.said("tool"), ["Running a command", "Running a command", "Checking images"]);
        assert_eq!(c.said("text"), ["done"]);
        assert_eq!(c.pid(), Some(pid), "the same process for both messages");
        assert_eq!(c.recorded_answers(), ["deny", "allow", "allow"]);
        assert_eq!(c.starts(), ["acp"]);
        c.clean();
    }

    #[test]
    fn recorded_tools_of_the_server_named_as_roblox_studios_pass_and_a_call_of_three_minutes_is_cut_off_by_agy() {
        // recorded with a stand-in server under the name Roblox Studio's has: agy names its tools
        // `mcp_Roblox_Studio_<tool>`, the gate lets them through, and they count as Studio steps. The second call
        // waited 190 seconds: agy gave it up after three minutes, whatever time limit the server's entry asked for
        let c = Chat::recorded("rec-studio-tool", "acp-studio-tool");
        c.ask("the message");
        let tools: Vec<Value> = c.events().into_iter().filter(|e| e["kind"] == "tool").collect();
        assert_eq!(c.said("tool"), ["Checking Studio", "wait job finished"]);
        assert!(tools.iter().all(|t| t["studio"] == true), "{tools:?}");
        assert_eq!(c.said("text"), ["Encountered error in tool execution: MCP tool call to server \"Roblox_Studio\" timed out after 3m0s: context deadline exceeded"]);
        assert_eq!(c.recorded_answers(), ["allow", "allow"]);
        c.clean();
    }

    #[test]
    fn recorded_web_search_and_a_read_of_a_web_address_are_not_asked_about() {
        // recorded while the app still approved allowed programs by itself: here the user's "Run it once" stands in
        // for that. agy then read a web address with its file-reading tool (which failed) and searched the web, and
        // asked about neither: the only request in the recording is the command's
        let c = Chat::recorded("rec-web", "acp-web-search");
        c.once("whichshell one");
        c.ask("the message");
        assert_eq!(c.said("tool"), ["Running a command", "Reading a file", "Searching the web"]);
        assert_eq!(c.said("text"), ["done"]);
        assert_eq!(c.recorded_answers(), ["allow"]);
        let all = fs::read_to_string(c.script.with_extension("sh.wire")).unwrap();
        assert_eq!(all.lines().filter(|l| l.starts_with("< ") && l.contains("\"method\":\"session/request_permission\"")).count(), 1);
        assert!(all.contains("\"name\":\"search_web\"") && all.contains("\"kind\":\"search\""));
        c.clean();
    }

    #[test]
    fn recorded_studio_mode_lead_turn() {
        // recorded in Studio mode (Full auto's rules): the Lead called the app's team tool twice and ended the turn
        // waiting for the user
        let c = Chat::recorded("rec-studio-lead", "acp-studio-lead");
        c.say("the message", As { effort: Some("low"), full: true });
        c.wait();
        assert_eq!(c.said("tool"), ["Working with other agents", "Working with other agents"]);
        assert!(c.said("text").last().is_some_and(|t| t.ends_with("STATUS: WAITING")), "{:?}", c.said("text"));
        assert!(c.said("error").is_empty());
        assert_eq!(c.recorded_answers(), ["allow", "allow"]);
        assert!(c.pid().is_some());
        c.clean();
    }

    #[test]
    fn recorded_expert_on_gemini_is_refused_the_apps_settings_in_another_case_and_keeps_no_process() {
        // recorded: a helper started for a Lead (no effort set) asked to write `CONFIG.JSON` in the kit folder with
        // its file tool; the app said no, and its report went back to the Lead
        let c = Chat::recorded("rec-expert", "acp-expert");
        c.a.st.lock().unwrap().parent = Some("lead".into());
        c.say("the task", As { effort: None, full: false });
        c.wait();
        assert_eq!(c.said("tool"), ["Writing a file"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].to_lowercase().contains("kit/config.json") && errors[0].contains("no AI's file tools may change"), "{errors:?}");
        assert_eq!(fs::read_to_string(c.dir.join("kit").join("config.json")).unwrap(), "{}");
        assert_eq!(c.recorded_answers(), ["deny"]);
        assert_eq!(c.pid(), None, "an expert holds nothing between jobs");
        c.clean();
    }

    #[test]
    fn recorded_job_longer_than_agys_three_minutes_is_waited_for_and_its_real_result_reported() {
        // recorded through the app with the tool server given its budget (`session/new` hands it
        // RBXKIT_CALL_BUDGET=150 and the notes file): the stand-in tool was asked to work for 240 seconds, a minute
        // past agy's cut-off. The call answered "not finished yet" at 150 s (153 s on the chat's clock), the model
        // called `rbx_job wait`, which returned the tool's own result at 240 s, and it reported the word in it. Where
        // the call was cut off before ("timed out after 3m0s", the recording above), both calls completed.
        let raw = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gemini/acp-long-job.wire")).unwrap();
        let new = raw.lines().find(|l| l.contains("\"session/new\"")).unwrap();
        assert!(new.contains(r#"{"name":"RBXKIT_CALL_BUDGET","value":"150"}"#) && new.contains(r#"{"name":"FRIDAE_TOOL_JOBS","value":"/scratch/work/tool-jobs/"#), "the entry as the app handed it over");
        assert!(!raw.contains("timed out"));
        let completed = |tool: &str| raw.lines().any(|l| l.contains("\"tool_call_update\"") && l.contains(&format!("\"name\":\"{tool}\"")) && l.contains("\"status\":\"completed\""));
        assert!(completed("mcp_rbxkit_rbx_test_slow") && completed("mcp_rbxkit_rbx_job"));
        let c = Chat::recorded("rec-long-job", "acp-long-job");
        c.ask("the message");
        assert_eq!(c.said("tool"), ["rbx test slow", "Waiting for work that takes a while"]);
        assert_eq!(c.said("text"), ["The tool returned the result word **MARBLE**."]);
        assert!(c.said("error").is_empty(), "the job was collected: nothing was thrown away");
        assert_eq!(c.recorded_answers(), ["allow", "allow"]);
        assert!(c.pid().is_some());
        c.clean();
    }

    #[test]
    fn recorded_full_auto_write_to_the_apps_settings_by_a_file_tool_is_refused() {
        // recorded in Full auto, on a kept process that resumed the chat's conversation: asked to overwrite the app's
        // settings file with its file tool, agy asked (`write_file`), the app said no, the call failed and agy ended
        // the turn. The file was unchanged afterwards (checked when recorded).
        let c = Chat::recorded("rec-full-protected", "acp-full-auto-protected-write");
        c.a.st.lock().unwrap().session = Some("00000000-0000-4000-8000-0000000000c1".into());
        c.say("the message", As { effort: Some("low"), full: true });
        c.wait();
        assert_eq!(c.said("tool"), ["Writing a file"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].contains("kit/config.json") && errors[0].contains("which no AI's file tools may change"), "{errors:?}");
        assert_eq!(fs::read_to_string(c.dir.join("kit").join("config.json")).unwrap(), "{}");
        assert_eq!(c.recorded_answers(), ["deny"]);
        c.clean();
    }
}
