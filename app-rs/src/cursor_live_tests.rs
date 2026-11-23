//! Cursor on a kept `cursor-agent acp` process (`acp` as `acp::Agent::Cursor`, `CursorLaunch`, and the same
//! `Agent::live_turn` and `read_live` that keep Claude Code's and Gemini's).
//!
//! Turns against a stand-in that answers as cursor-agent does (every answer's shape is from a recording), for what
//! keeping the process means: which messages run on it, one process across messages, a chat that stays on the path it
//! started on, the hand-over as text when a kept chat's message has to go per message, every way of failing. Then real
//! traffic recorded from cursor-agent 2026.11.22 through this client (`tests/fixtures/cursor/acp-*.wire`), played
//! back by the same stand-in as Gemini's recordings are (see `acp_tests`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

/// The stand-in for cursor-agent. Kept (its last argument is `acp`): plays the recording beside it when there is one,
/// else answers each request as cursor-agent does, with words in the message choosing what goes wrong. Otherwise the
/// per-message way: one message on stdin, one reply as cursor-agent prints it, exit.
const SCRIPT: &str = r#"
echo "$*" >> "$0.args"
n=$(grep -c '' "$0.args")
for last; do :; done
case "$last" in acp) ;; *)
  cat > "$0.stdin"; conv=P1; prev=; for a in "$@"; do [ "$prev" = --resume ] && conv=$a; prev=$a; done
  echo '{"type":"system","subtype":"init","session_id":"'"$conv"'","model":"Auto"}'
  echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"per message"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"per message","session_id":"'"$conv"'","usage":{"inputTokens":5,"outputTokens":2,"cacheReadTokens":1,"cacheWriteTokens":0}}'
  exit 0;;
esac
f="$0.$n.wire"; [ -f "$f" ] || f="$0.wire"
sig() { printf '%s\n' "$1" | sed -n 's/^{\("id":[^,]*,\)\{0,1\}"jsonrpc":"2.0","method":"\([^"]*\)".*/\2/p'; }
if [ -f "$f" ]; then
  while IFS= read -r l <&3; do
    case "$l" in
      '> '*) IFS= read -r got || exit 0
        want="${l#> }"
        case "$want" in *'"method":"'*) w=$(sig "$want"); g=$(sig "$got");; *) w="$want"; g="$got";; esac
        [ "$w" = "$g" ] || printf 'recorded: %s\nsent now: %s\n' "$w" "$g" >> "$0.mismatch";;
      '< '*) printf '%s\n' "${l#< }";;
    esac
  done 3<"$f"
  [ -f "$0.dies" ] && kill -9 $$
  cat >/dev/null
  sleep 300
fi
ok() { echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$1}"; }
bad() { echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"Invalid params\",\"data\":{\"message\":\"$1\"}}}"; }
sid=S1
say() { echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$sid\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$1\"}}}}"; }
tool() { echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$sid\",\"update\":$1}}"; }
keep() { mkdir -p "$CURSOR_CONFIG_DIR/acp-sessions/$sid"; : > "$CURSOR_CONFIG_DIR/acp-sessions/$sid/store.db"; }
turns=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) [ -f "$0.old" ] && ok '{"protocolVersion":2}' || ok '{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}';;
    *'"method":"session/new"'*)
      if [ -f "$0.signed-out" ]; then echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32000,\"message\":\"Authentication required. Please run 'agent login' first.\"}}"
      else keep; ok '{"sessionId":"S1","modes":{"currentModeId":"agent"}}'; fi;;
    *'"method":"session/load"'*)
      echo load >> "$0.loads"
      case "$line" in
        *'"sessionId":"GONE"'*) bad 'Session \"GONE\" not found';;
        *'"sessionId":"BROKEN"'*) bad 'the store could not be opened';;
        *) sid=$(printf '%s\n' "$line" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
           # the history is played back before the answer, as Cursor does
           tool '{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"an earlier message"}}'
           say "an earlier reply"
           tool '{"sessionUpdate":"tool_call","toolCallId":"replay-0-1","title":"`ls`","kind":"execute","status":"pending","rawInput":{"command":"ls"}}'
           tool '{"sessionUpdate":"tool_call_update","toolCallId":"replay-0-1","status":"completed"}'
           ok '{"modes":{"currentModeId":"agent"}}';;
      esac;;
    *'"method":"session/set_config_option"'*)
      [ -f "$0.no-model" ] && bad 'Invalid model value: default[]' || ok '{"configOptions":[{"id":"mode","currentValue":"agent"},{"id":"model","currentValue":"default[]"}]}';;
    *'"method":"session/cancel"'*)
      if [ -n "$asks" ]; then
        echo '{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":"S1","toolCall":{"toolCallId":"c9","title":"`rm -rf x`","kind":"execute","status":"pending","content":[{"type":"content","content":{"type":"text","text":"Not in allowlist: rm"}}]},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Allow always","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
        IFS= read -r reply; printf '%s\n' "$reply" > "$0.late"
      fi
      echo "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"cancelled\"}}";;
    *'"id":0,'*'"result"'*|*'"id":1,'*'"result"'*)
      printf '%s\n' "$line" >> "$0.asked"
      if [ "$(grep -c '' "$0.asked")" = 2 ] && [ -z "$asks" ]; then say "carried on"; echo "{\"jsonrpc\":\"2.0\",\"id\":$prompt,\"result\":{\"stopReason\":\"end_turn\"}}"; fi;;
    *'"method":"session/prompt"'*)
      prompt=$id; printf '%s\n' "$line" >> "$0.prompts"
      case "$line" in
        *die-early*) exit 3;;
        *die-late*) say "half"; echo "boom" >&2; exit 3;;
        *hang*) say "working";;
        *think*) ;;
        *deaf*) say "working"; trap '' TERM; while IFS= read -r l; do :; done; sleep 300;;
        *limit*) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32603,\"message\":\"Internal error\",\"data\":{\"message\":\"You have hit your usage limit.\"}}}";;
        *refused-twice*|*asks*)
          case "$line" in *asks*) asks=1;; esac
          say "I will run two commands."
          # a command not on the allow list, then a write outside the workspace: each asked about, as recorded
          tool '{"sessionUpdate":"tool_call","toolCallId":"c1","title":"`zzgit status`","kind":"execute","status":"pending","rawInput":{"command":"zzgit status"}}'
          echo '{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{"sessionId":"S1","toolCall":{"toolCallId":"c1","title":"`zzgit status`","kind":"execute","status":"pending","content":[{"type":"content","content":{"type":"text","text":"Not in allowlist: zzgit"}}]},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Allow always","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed"}'
          tool '{"sessionUpdate":"tool_call","toolCallId":"c2","title":"Edit File","kind":"edit","status":"pending","rawInput":{}}'
          echo '{"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"sessionId":"S1","toolCall":{"toolCallId":"c2","title":"Write /outside/x.txt","kind":"edit","status":"pending","content":[{"type":"diff","path":"/outside/x.txt","oldText":null,"newText":"x"}]},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Allow always","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"c2","status":"completed"}';;
        *leaves-running*)
          say "started it"
          sleep 1 # (Cursor thinks before it runs anything: the app has noted the process's groups by then)
          perl -e 'setpgrp(0,0); exec "sleep", "40"' >/dev/null 2>&1 &
          echo $! > "$0.left"
          sleep 1 # (it is in its own group by now, as a command Cursor started long before the turn's end is)
          ok '{"stopReason":"end_turn"}';;
        *tools*)
          # a tool server's call is named only by the update after it; a read; one of another server's
          tool '{"sessionUpdate":"tool_call","toolCallId":"t1","title":"MCP: tool","kind":"other","status":"pending","rawInput":{}}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"t1","title":"rbxkit: rbx_board","rawInput":{"providerIdentifier":"rbxkit","toolName":"rbx_board","args":{"action":"list"}}}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"in_progress"}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","rawOutput":{"success":true}}'
          tool '{"sessionUpdate":"tool_call","toolCallId":"t2","title":"Read /scratch/game/a.txt","kind":"read","status":"pending","rawInput":{"path":"/scratch/game/a.txt"}}'
          tool '{"sessionUpdate":"tool_call_update","toolCallId":"t2","status":"completed"}'
          say "done"; ok '{"stopReason":"end_turn"}';;
        *) turns=$((turns+1)); say "reply $turns"; ok '{"stopReason":"end_turn"}';;
      esac;;
  esac
done
[ -f "$0.stays" ] && sleep 300
"#;

struct Chat {
    a: Arc<Agent>,
    script: PathBuf,
    dir: PathBuf,
}

impl Chat {
    fn new(name: &str) -> Chat {
        let dir = paths::test_dir("cursor-acp").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("ws")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        let script = dir.join("cursor.sh");
        fs::write(&script, SCRIPT).unwrap();
        let a = Agent::detached_for_test(&format!("cur-{name}"), None);
        // Cursor's kept process is asked for (it is on request: `KEPT_ON_REQUEST`), on this chat alone
        a.st.lock().unwrap().ai = Some("cursor".into());
        a.st.lock().unwrap().kept_asked = true;
        Chat { a, script, dir }
    }

    fn playing(name: &str, case: &str) -> Chat {
        let c = Chat::new(name);
        c.plays(case, 0);
        c
    }

    fn plays(&self, case: &str, n: usize) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("cursor").join(format!("{case}.wire"));
        let to = if n == 0 { self.script.with_extension("sh.wire") } else { self.script.with_extension(format!("sh.{n}.wire")) };
        let text = fs::read_to_string(&from).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        fs::write(to, text.replace("/scratch", &self.dir.display().to_string())).unwrap();
    }

    fn mark(&self, what: &str) {
        fs::write(self.script.with_extension(format!("sh.{what}")), "").unwrap();
    }

    fn settings<'a>(&'a self, servers: &'a Value, rules: &'a Value, guarded: &'a Map<String, Value>) -> CursorSettings<'a> {
        CursorSettings {
            exe: Path::new("/bin/sh"),
            agent_id: &self.a.id,
            workspace: &self.dir,
            kit: Path::new("/kit"),
            project: Some("/games/a"),
            servers,
            model: Some("auto"),
            full: false,
            pictures: None,
        jobs: None,
            rules,
            folders: (self.dir.join("config"), self.dir.join("data"), PathBuf::from("/home/u")),
            guarded,
            keys_on: false,
            prompt: "[standing instructions]",
            body: "",
        }
    }

    /// The launch of a message, as `send` builds it: `body` is the message, `text` what the AI would get (the same
    /// here, since these chats' conversations exist: instructions go on a conversation's first message only).
    fn launch_with(&self, body: &str, change: impl FnOnce(&mut CursorSettings)) -> CursorLaunch {
        let (servers, rules, guarded) = (json!({"rbxkit": {"command": "python3", "args": [], "env": {}}}), json!({"allow": ["Mcp(rbxkit:*)"], "deny": []}), Map::new());
        let mut s = self.settings(&servers, &rules, &guarded);
        s.body = body;
        change(&mut s);
        let mut l = CursorLaunch::new(&s);
        l.first = vec![self.script.clone().into()];
        l
    }

    /// A message as the user sends it: noted in the chat first, as `send` does.
    fn say_with(&self, text: &str, change: impl FnOnce(&mut CursorSettings)) {
        self.a.put("user", json!({"text": text}));
        self.a.kept_turn(&self.launch_with(text, change), text);
    }

    fn send(&self, text: &str) {
        self.say_with(text, |_| {});
        self.wait();
    }

    fn start(&self, text: &str) {
        self.say_with(text, |_| {});
    }

    /// Until the turn is over for the chat: no longer running, and its `done` noted (the lines a turn's end adds,
    /// such as "Stopped.", come between the two).
    fn wait(&self) {
        let until = std::time::Instant::now() + Duration::from_secs(25);
        loop {
            {
                let st = self.a.st.lock().unwrap();
                if !st.running && st.log.last().is_some_and(|e| e["kind"] == "done") {
                    return;
                }
                assert!(std::time::Instant::now() < until, "the turn never ended: {:?}", st.log);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn taken(&self) {
        let tp = self.a.st.lock().unwrap().proc_.clone().unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !tp.acked.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < until);
            thread::sleep(Duration::from_millis(10));
        }
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

    fn starts(&self) -> Vec<String> {
        fs::read_to_string(self.script.with_extension("sh.args")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    /// How many of the processes started were kept ones.
    fn kept_starts(&self) -> usize {
        self.starts().iter().filter(|s| s.ends_with(" acp")).count()
    }

    fn per_message_input(&self) -> String {
        fs::read_to_string(self.script.with_extension("sh.stdin")).unwrap_or_default()
    }

    fn loads(&self) -> usize {
        fs::read_to_string(self.script.with_extension("sh.loads")).unwrap_or_default().lines().count()
    }

    fn fails(&self) -> u32 {
        self.a.st.lock().unwrap().live_fails
    }

    fn session(&self) -> Option<String> {
        self.a.st.lock().unwrap().session.clone()
    }

    /// A conversation a kept process made earlier (as Cursor keeps one), which the chat has.
    fn has_kept(&self, id: &str) {
        let d = self.dir.join("config").join("acp-sessions").join(id);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("store.db"), "").unwrap();
        self.a.st.lock().unwrap().session = Some(id.into());
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

// ---------------------------------------------------------------- the launch

#[test]
fn the_launch_key_changes_with_everything_a_cursor_process_is_started_with() {
    let c = Chat::new("key");
    let servers = json!({"rbxkit": {"command": "python3", "args": ["mcp.py"], "env": {"RBXKIT_PROJECT": "/games/a"}}});
    let rules = json!({"allow": ["Mcp(rbxkit:*)", "Shell(git)"], "deny": ["Write(**/.cursor/**)"]});
    let guarded: Map<String, Value> = json!({"allowed_commands": ["git"], "use_env_keys": false, "gemini_key": "k1", "projects": []}).as_object().unwrap().clone();
    let base = || c.settings(&servers, &rules, &guarded);
    let key = |s: CursorSettings| CursorLaunch::new(&s).key();
    let same = key(base());
    assert_eq!(key(base()), same, "the same settings: the process is kept");
    assert_eq!(key(CursorSettings { prompt: "other instructions", body: "another message", ..base() }), same, "the message itself is not what a process is started with");
    let other_servers = json!({"rbxkit": {"command": "python3", "args": ["mcp.py"], "env": {"RBXKIT_PROJECT": "/games/b"}}});
    let two_servers = json!({"rbxkit": servers["rbxkit"], "Roblox_Studio": {"command": "studio", "args": ["--stdio"]}});
    let more_rules = json!({"allow": ["Mcp(rbxkit:*)", "Shell(git)", "Shell(python3)"], "deny": ["Write(**/.cursor/**)"]});
    let more_denied = json!({"allow": ["Mcp(rbxkit:*)", "Shell(git)"], "deny": ["Write(**/.cursor/**)", "Mcp(notion:*)"]});
    let mut g_key = guarded.clone();
    g_key.insert("gemini_key".into(), json!("k2"));
    let mut g_games = guarded.clone();
    g_games.insert("projects".into(), json!([{"name": "a", "path": "/games/a"}]));
    let folders = |i: usize, to: &str| {
        let mut f = base().folders;
        match i {
            0 => f.0 = to.into(),
            1 => f.1 = to.into(),
            _ => f.2 = to.into(),
        }
        f
    };
    let changed: Vec<(&str, CursorSettings)> = vec![
        ("the mode", CursorSettings { full: true, ..base() }),
        ("an allowed program (Cursor's allow list)", CursorSettings { rules: &more_rules, ..base() }),
        ("a deny rule (one of the user's own servers)", CursorSettings { rules: &more_denied, ..base() }),
        ("the guarded settings: the Gemini key", CursorSettings { guarded: &g_key, ..base() }),
        ("the guarded settings: the games", CursorSettings { guarded: &g_games, ..base() }),
        ("the model", CursorSettings { model: Some("gpt-5.3-codex-low"), ..base() }),
        ("no model", CursorSettings { model: None, ..base() }),
        ("the chat's folder", CursorSettings { workspace: Path::new("/work/cursor/chats/other"), ..base() }),
        ("the game folder", CursorSettings { project: Some("/games/b"), ..base() }),
        ("no game", CursorSettings { project: None, ..base() }),
        ("the kit folder", CursorSettings { kit: Path::new("/kit2"), ..base() }),
        ("the tool servers (a setting they are handed)", CursorSettings { servers: &other_servers, ..base() }),
        ("the tool servers (one more)", CursorSettings { servers: &two_servers, ..base() }),
        ("where its tool server notes pictures", CursorSettings { pictures: Some(Path::new("/work/tool-pictures/c1.jsonl")), ..base() }),
        ("where its tool server notes its jobs", CursorSettings { jobs: Some(Path::new("/work/tool-jobs/c1.jsonl")), ..base() }),
        ("the provider-keys switch", CursorSettings { keys_on: true, ..base() }),
        ("the program", CursorSettings { exe: Path::new("/other/cursor-agent"), ..base() }),
        ("the chat", CursorSettings { agent_id: "another", ..base() }),
        ("Cursor's settings folder", CursorSettings { folders: folders(0, "/elsewhere/config"), ..base() }),
        ("Cursor's data folder", CursorSettings { folders: folders(1, "/elsewhere/data"), ..base() }),
        ("the home folder Cursor runs with", CursorSettings { folders: folders(2, "/home/v"), ..base() }),
    ];
    let mut seen = vec![same.clone()];
    for (what, s) in changed {
        let k = key(s);
        assert!(!seen.contains(&k), "{what} must start another process");
        seen.push(k);
    }
    assert!(!same.contains("k1"), "the key holds a fingerprint of the guarded settings, never a key itself");
    // the kept command: today's flags without the message's own, never the sandbox (it is not applied over ACP) and
    // never `--force`; the settings and data folders are the app's
    let l = CursorLaunch::new(&base());
    let cmd = l.command();
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
    assert_eq!(args, ["--trust", "--approve-mcps", "--add-dir", "/kit", "--add-dir", "/games/a", "--sandbox", "disabled", "acp"]);
    let env = |k: &str| cmd.get_envs().find(|(n, _)| n.to_string_lossy() == k).and_then(|(_, v)| v.map(|v| v.to_string_lossy().to_string()));
    assert_eq!(env("CURSOR_CONFIG_DIR"), Some(c.dir.join("config").display().to_string()));
    assert_eq!(env("CURSOR_DATA_DIR"), Some(c.dir.join("data").display().to_string()));
    assert_eq!(cmd.get_current_dir(), Some(c.dir.as_path()));
    // the session is handed the servers, the app's own told where to note its pictures (on the kept path only)
    let noted = CursorLaunch::new(&CursorSettings { pictures: Some(Path::new("/work/tool-pictures/c1.jsonl")), ..base() });
    assert_eq!(noted.spec.servers["rbxkit"]["env"]["FRIDAE_TOOL_PICTURES"], "/work/tool-pictures/c1.jsonl");
    assert!(noted.servers["rbxkit"]["env"].get("FRIDAE_TOOL_PICTURES").is_none(), "a process per message hands its results over itself");
    // Cursor cuts a tool call off after 60 seconds on both paths: the app's tool server is given 45 to answer in,
    // and the file the turn's end is told through, in the session's servers and in the workspace's file alike
    let budgeted = CursorLaunch::new(&CursorSettings { jobs: Some(Path::new("/work/tool-jobs/c1.jsonl")), ..base() });
    for (path, servers) in [("kept", &budgeted.spec.servers), ("per message", &budgeted.servers)] {
        assert_eq!(servers["rbxkit"]["env"]["RBXKIT_CALL_BUDGET"], "45", "{path}");
        assert_eq!(servers["rbxkit"]["env"]["FRIDAE_TOOL_JOBS"], "/work/tool-jobs/c1.jsonl", "{path}");
    }
    let (per, _) = Kept::per_turn(&budgeted, None, "hello");
    drop(per);
    let file: Value = serde_json::from_str(&fs::read_to_string(c.dir.join(".cursor").join("mcp.json")).unwrap()).unwrap();
    assert_eq!(file["mcpServers"]["rbxkit"]["env"]["RBXKIT_CALL_BUDGET"], "45", "as a process per message reads it");
    assert!(l.spec.servers["rbxkit"]["env"].get("RBXKIT_CALL_BUDGET").is_none() && l.servers["rbxkit"]["env"].get("RBXKIT_CALL_BUDGET").is_none(), "never a budget without that file");
    assert_eq!((l.spec.agent, l.spec.model.as_deref(), l.spec.effort.as_deref()), (crate::acp::Agent::Cursor, Some("default[]"), None), "Auto, by its id there");
    assert_eq!(l.spec.rules.servers, ["rbxkit"]);
    // the per-message command with the same settings: today's, unchanged
    let (per, input) = Kept::per_turn(&l, Some("C9"), "hello");
    let per: Vec<String> = per.get_args().map(|a| a.to_string_lossy().to_string()).collect();
    assert_eq!(per[..5], ["-p", "--output-format", "stream-json", "--trust", "--approve-mcps"]);
    assert!(per.windows(2).any(|w| w == ["--resume", "C9"]) && per.windows(2).any(|w| w == ["--model", "auto"]) && per.ends_with(&["--sandbox".to_string(), "disabled".to_string()]), "{per:?}");
    assert_eq!(input, "hello");
    let (full, _) = Kept::per_turn(&CursorLaunch::new(&CursorSettings { full: true, ..base() }), None, "x");
    assert!(full.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>().ends_with(&["--sandbox".to_string(), "enabled".to_string()]), "Full auto, per message: the sandbox");
}

#[test]
fn only_ask_me_on_auto_in_a_chat_that_began_on_a_kept_process_runs_on_one() {
    let c = Chat::new("fits");
    let fits = |change: &dyn Fn(&mut CursorSettings)| c.launch_with("m", |s| change(s)).fits(&c.a);
    assert!(fits(&|_| {}), "Ask me, Auto, a new chat");
    assert!(fits(&|s| s.model = None), "no model named is Auto");
    // Full auto and Studio rely on Cursor's sandbox, which is not applied over ACP (measured)
    assert!(!fits(&|s| s.full = true));
    // only Auto has a known id on both paths (measured: `auto` is `default[]`; no other spelling is taken)
    for m in ["gpt-5.3-codex", "composer-2.5", "gpt-5.3-codex-low"] {
        assert!(!fits(&|s| s.model = Some(m)), "{m}");
    }
    assert_eq!(cursor::kept_model(Some("auto")), Some("default[]"));
    assert_eq!((cursor::kept_model(None), cursor::kept_model(Some("composer-2.5"))), (Some("default[]"), None));
    assert_eq!(cursor::kept_model_name("default[]"), "Cursor (Auto)");
    // a chat keeps the path it started on: a conversation a process per message made is never opened by a kept one
    c.a.st.lock().unwrap().session = Some("P-per-message".into());
    assert!(!fits(&|_| {}));
    c.has_kept("K-kept");
    assert!(fits(&|_| {}));
    // and nothing is kept at all unless asked for, on a Mac or Linux
    c.a.st.lock().unwrap().kept_asked = false;
    assert_eq!(c.a.live_allowed(), std::env::var("FRIDAE_CURSOR_KEPT").is_ok_and(|v| v == "1"), "on request only");
}

// ------------------------------------------------------------ stand-in process

#[cfg(unix)]
mod stand_in {
    use super::*;

    #[test]
    fn two_messages_run_on_one_process_and_its_usage_is_noted_as_not_reported() {
        let c = Chat::new("reuse");
        c.send("one");
        let first = c.pid().unwrap();
        c.send("two");
        assert_eq!(c.pid(), Some(first), "the same process");
        assert_eq!(c.said("text"), ["reply 1", "reply 2"], "and it counted both");
        assert_eq!(c.starts(), ["--trust --approve-mcps --add-dir /kit --add-dir /games/a --sandbox disabled acp"]);
        assert_eq!(c.session().as_deref(), Some("S1"));
        assert_eq!(c.loads(), 0, "a new chat: a new session");
        // Cursor reports no usage over ACP: each turn is logged as one whose usage was not reported, never as zero
        let usage: Vec<Value> = c.events().into_iter().filter(|e| e["kind"] == "usage").collect();
        assert_eq!(usage.len(), 2);
        assert!(usage.iter().all(|u| u["unreported"] == true && u["total"] == 0), "{usage:?}");
        assert_eq!(c.fails(), 0);
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
    }

    #[test]
    fn a_kept_chats_message_in_full_auto_goes_per_message_as_a_new_conversation_handed_the_chat_as_text() {
        let c = Chat::new("handover");
        c.send("the first message");
        let kept = c.pid().unwrap();
        assert_eq!(c.session().as_deref(), Some("S1"));
        // Full auto: Cursor's sandbox is not applied over ACP, so this message runs a process per message, which
        // cannot open the kept process's conversation
        c.say_with("now in full auto", |s| s.full = true);
        c.wait();
        assert_eq!(c.said("text"), ["reply 1", "per message"]);
        assert_eq!(c.said("error"), [CURSOR_HANDED_OVER], "said in one line");
        assert!(goes(kept), "the kept process is not left open beside it");
        assert_eq!(c.pid(), None);
        let starts = c.starts();
        assert_eq!(starts.len(), 2);
        assert!(starts[1].starts_with("-p --output-format stream-json") && starts[1].ends_with("--sandbox enabled"), "{starts:?}");
        assert!(!starts[1].contains("--resume"), "never asked to resume a conversation it does not have: {starts:?}");
        let got = c.per_message_input();
        assert!(got.starts_with(&antigravity::with_instructions("[standing instructions]", "[This chat carries on here in a new conversation, so you do not remember it. The chat so far, as text")), "a new conversation's first message: instructions, then the chat so far: {got}");
        assert!(got.contains("User: the first message") && got.contains("Cursor: reply 1"), "the chat so far, as text: {got}");
        assert!(got.ends_with("now in full auto"), "then the message, once: {got}");
        assert_eq!(got.matches("now in full auto").count(), 1);
        assert_eq!(c.session().as_deref(), Some("P1"), "the new conversation is the chat's");
        assert_eq!(c.fails(), 0, "no failure of keeping a process");
        // from then on the chat is a per-message one, in Ask me too: it keeps the path its conversation is on
        c.send("back in ask me");
        assert_eq!(c.kept_starts(), 1, "{:?}", c.starts());
        assert!(c.starts()[2].contains("--resume P1"), "{:?}", c.starts());
        assert_eq!(c.per_message_input(), "back in ask me", "nothing handed over twice");
        assert_eq!(c.said("error").len(), 1);
    }

    #[test]
    fn a_chat_that_began_per_message_and_one_on_another_model_never_start_a_kept_process() {
        let per = Chat::new("stays-per-message");
        per.a.st.lock().unwrap().session = Some("P7".into());
        per.send("hello");
        assert_eq!(per.kept_starts(), 0);
        assert!(per.starts()[0].contains("--resume P7"), "{:?}", per.starts());
        assert_eq!(per.said("text"), ["per message"]);
        let usage: Vec<Value> = per.events().into_iter().filter(|e| e["kind"] == "usage").collect();
        assert!(usage.len() == 1 && usage[0].get("unreported").is_none() && usage[0]["input"] == 5, "a process per message reports its usage: {usage:?}");
        let model = Chat::new("other-model");
        model.say_with("hello", |s| s.model = Some("gpt-5.3-codex-low"));
        model.wait();
        assert_eq!(model.kept_starts(), 0);
        assert!(model.starts()[0].contains("--model gpt-5.3-codex-low"), "{:?}", model.starts());
        assert!(model.said("error").is_empty());
    }

    #[test]
    fn a_new_process_loads_the_chats_conversation_and_shows_nothing_of_the_history_played_back() {
        let c = Chat::new("load");
        c.has_kept("K1");
        c.send("carry on");
        assert_eq!(c.loads(), 1);
        assert_eq!(c.said("text"), ["reply 1"], "the history Cursor plays back is not this turn's");
        assert!(c.said("tool").is_empty());
        assert_eq!(c.session().as_deref(), Some("K1"));
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        // a process that died between two turns is replaced without a word, and loads again
        let pid = c.pid().unwrap();
        paths::end_trees(&[pid]);
        assert!(paths::leader_exited(pid, Duration::from_secs(5)));
        c.send("again");
        assert_ne!(c.pid(), Some(pid));
        assert_eq!((c.loads(), c.fails()), (2, 1));
        assert!(c.said("error").is_empty());
    }

    #[test]
    fn a_conversation_cursor_no_longer_has_starts_a_new_one_and_the_chat_is_told() {
        let c = Chat::new("lost");
        c.has_kept("GONE");
        c.send("hello");
        assert_eq!(c.said("text"), ["reply 1"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Cursor could not find this chat's earlier conversation"), "{errors:?}");
        assert_eq!(c.session().as_deref(), Some("S1"));
        // (a chat with nothing said before: the new conversation still gets its standing instructions)
        let first = fs::read_to_string(c.script.with_extension("sh.prompts")).unwrap();
        assert!(first.contains("[standing instructions]") && first.contains("hello") && !first.contains("The chat so far"), "{first}");
        // with a chat so far, the new conversation is handed it, and the line says so
        let k = Chat::new("lost-with-history");
        k.a.put("user", json!({"text": "before"}));
        k.a.put("text", json!({"text": "an answer"}));
        k.has_kept("GONE");
        k.send("and now");
        let errors = k.said("error");
        assert!(errors.len() == 1 && errors[0].contains("was handed the chat so far as text"), "{errors:?}");
        let first = fs::read_to_string(k.script.with_extension("sh.prompts")).unwrap();
        assert!(first.contains("[standing instructions]") && first.contains("User: before") && first.contains("Cursor: an answer") && first.matches("and now").count() == 1, "{first}");
        assert_eq!((k.fails(), k.kept_starts()), (0, 1), "on the same kept process: no failure, no second start");
        // any other error from the load: the kept way is given up, and the message goes per message with the chat
        // handed over (the conversation cannot be opened there either)
        let b = Chat::new("load-broken");
        b.a.put("user", json!({"text": "before"}));
        b.a.put("text", json!({"text": "an answer"}));
        b.has_kept("BROKEN");
        b.send("hello");
        assert_eq!(b.said("text"), ["an answer", "per message"]);
        assert_eq!(b.said("error"), [CURSOR_HANDED_OVER]);
        assert!(b.per_message_input().contains("User: before") && b.per_message_input().ends_with("hello"));
        assert_eq!(b.fails(), 1);
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
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed by the cancelled prompt: {:?}", pressed.elapsed());
        assert_eq!(c.said("text"), ["working"]);
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.pid(), Some(pid), "kept");
        c.send("again");
        assert_eq!(c.pid(), Some(pid));
        assert_eq!(c.starts().len(), 1);
    }

    #[test]
    fn stop_before_cursor_has_started_on_the_message_ends_the_process_and_one_that_ignores_the_cancel_is_killed() {
        let c = Chat::new("stop-early");
        c.send("one");
        let pid = c.pid().unwrap();
        c.start("think");
        thread::sleep(Duration::from_millis(300));
        c.a.stop();
        c.wait();
        assert_eq!(c.said("error"), ["Stopped."], "cleanly");
        assert_eq!(c.pid(), None);
        assert!(goes(pid));
        assert_eq!(c.fails(), 0);
        c.send("again");
        assert_eq!(c.loads(), 1, "a new process, which loaded the conversation");
        let d = Chat::new("deaf");
        d.start("deaf");
        d.taken();
        let pid = d.pid().unwrap();
        let pressed = std::time::Instant::now();
        d.a.stop();
        d.wait();
        assert!(pressed.elapsed() >= paths::STOP_WAIT, "it was given the wait");
        assert_eq!(d.said("error"), [paths::killed_message("Cursor").as_str(), "Stopped."]);
        assert!(gone(pid));
        assert_eq!(d.fails(), 0);
    }

    #[test]
    fn a_process_that_dies_before_it_showed_anything_has_its_message_sent_per_message_once() {
        // a new chat's first message: the per-message process gets it as it was (instructions and all), no hand-over
        let c = Chat::new("die-early");
        c.send("die-early please");
        assert_eq!(c.said("text"), ["per message"], "the user gets the answer");
        assert!(c.said("error").is_empty(), "nothing to hand over, nothing said: {:?}", c.said("error"));
        assert_eq!(c.per_message_input(), antigravity::with_instructions("[standing instructions]", "die-early please"), "the same message, once, as a conversation's first");
        assert_eq!((c.fails(), c.pid()), (1, None));
        assert_eq!(c.session().as_deref(), Some("P1"));
        // a chat with a kept conversation: handed over as text, said in one line, the message not sent twice
        let k = Chat::new("die-early-kept");
        k.send("one");
        k.send("die-early now");
        assert_eq!(k.said("text"), ["reply 1", "per message"]);
        assert_eq!(k.said("error"), [CURSOR_HANDED_OVER]);
        let got = k.per_message_input();
        assert!(got.contains("User: one") && got.contains("Cursor: reply 1") && got.ends_with("die-early now") && got.matches("die-early now").count() == 1, "{got}");
        assert_eq!(k.fails(), 1);
    }

    #[test]
    fn a_process_that_dies_mid_turn_is_said_and_its_message_is_not_sent_twice() {
        let c = Chat::new("die-late");
        c.send("die-late");
        assert_eq!(c.said("text"), ["half"], "what it had written is shown");
        assert_eq!(c.said("error"), ["boom"]);
        assert_eq!(c.starts().len(), 1, "no second process: a tool may already have run");
        assert_eq!((c.fails(), c.pid()), (1, None));
        c.send("again");
        assert_eq!(c.said("text"), ["half", "reply 1"]);
        assert_eq!((c.kept_starts(), c.loads()), (2, 1), "the next message loads the same conversation");
    }

    #[test]
    fn a_kept_process_that_cannot_be_started_speaks_another_protocol_or_lacks_the_model_sends_the_message_per_message() {
        let c = Chat::new("no-start");
        let mut launch = c.launch_with("hello", |_| {});
        launch.exe = "/nonexistent/cursor-agent".into();
        c.a.kept_turn(&launch, "hello");
        c.wait();
        assert_eq!(c.fails(), 1);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Could not start the agent"), "{errors:?}");
        let old = Chat::new("old");
        old.mark("old");
        old.send("hello");
        assert_eq!(old.said("text"), ["per message"]);
        assert_eq!((old.fails(), old.pid()), (1, None));
        // a session that does not take the model's id: per message, not counted as a failure of keeping a process
        let m = Chat::new("no-model");
        m.mark("no-model");
        m.send("hello");
        assert_eq!(m.said("text"), ["per message"]);
        assert_eq!((m.fails(), m.pid()), (0, None));
    }

    #[test]
    fn nobody_signed_in_is_said_once_and_no_second_process_is_tried() {
        let c = Chat::new("signed-out");
        c.mark("signed-out");
        c.send("hello");
        assert_eq!(c.said("error"), [cursor::SIGNED_OUT]);
        assert_eq!(c.starts().len(), 1, "not sent again per message, which would only say the same");
        assert_eq!((c.fails(), c.pid()), (0, None));
    }

    #[test]
    fn what_cursors_own_rules_ask_about_is_refused_a_program_gets_its_card_and_the_turn_carries_on() {
        // (`zzgit`: a program no test ever allows, so no other test's allowed list changes the card)
        let c = Chat::new("refused");
        c.send("refused-twice");
        // both requests answered no, by the option's kind, never "always"
        let answers: Vec<Value> = fs::read_to_string(c.script.with_extension("sh.asked")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(answers.len(), 2);
        for a in &answers {
            assert_eq!(a["result"], json!({"outcome": {"outcome": "selected", "optionId": "reject-once"}}));
        }
        // a refusal does not end Cursor's turn: it carried on, and the chat shows it
        assert_eq!(c.said("text"), ["I will run two commands.", "carried on"]);
        assert_eq!(c.said("tool"), ["Running a command", "Editing a file"], "{:?}", c.said("tool"));
        // the program not on the allow list gets the card that offers to allow it; the refused write gets none
        let ev = c.events();
        let cards: Vec<&Value> = ev.iter().filter(|e| e["kind"] == "blocked").collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["commands"], json!([{"name": "zzgit", "command": "zzgit status"}]));
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        assert!(c.pid().is_some(), "a refusal is no failure: the process is kept");
        // in Full auto's rules (not run on a kept process today) a refused program would get no card
        let mut t = crate::acp::Turn::of(crate::acp::Agent::Cursor, true, &[], None);
        let v = t.cursor_request_for_test(&json!({"toolCallId": "x", "title": "`ls`", "kind": "execute"}));
        assert!(!v && !t.finish(&json!({"result": {"stopReason": "end_turn"}})).iter().any(|o| matches!(o, Out::Blocked(_))));
    }

    #[test]
    fn a_request_that_comes_after_stop_is_answered_cancelled() {
        let c = Chat::new("late-request");
        c.start("asks");
        c.taken();
        let asked = c.script.with_extension("sh.asked");
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while fs::read_to_string(&asked).unwrap_or_default().lines().count() < 2 && std::time::Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        c.a.stop();
        c.wait();
        let late: Value = serde_json::from_str(&fs::read_to_string(c.script.with_extension("sh.late")).unwrap()).unwrap();
        assert_eq!((late["id"].as_u64(), &late["result"]), (Some(7), &json!({"outcome": {"outcome": "cancelled"}})));
        assert_eq!(c.said("error"), ["Stopped."]);
        assert!(c.events().iter().any(|e| e["kind"] == "blocked"), "what was refused before Stop still gets its card");
    }

    #[test]
    fn tools_are_named_as_a_process_per_message_names_them() {
        let c = Chat::new("tools");
        c.send("tools");
        assert_eq!(c.said("tool"), ["Updating its task list", "Reading a file"], "{:?}", c.said("tool"));
        assert_eq!(c.said("text"), ["done"]);
    }

    #[test]
    fn a_turn_that_left_a_command_running_keeps_no_process_and_the_command_is_ended() {
        let c = Chat::new("left-running");
        c.send("one");
        let pid = c.pid().unwrap();
        c.send("leaves-running");
        let left: u32 = fs::read_to_string(c.script.with_extension("sh.left")).unwrap().trim().parse().unwrap();
        assert_eq!(c.said("text"), ["reply 1", "started it"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].contains("was still running when the turn ended"), "{errors:?}");
        assert_eq!(c.pid(), None);
        assert!(goes(pid));
        assert!(paths::leader_exited(left, Duration::from_secs(10)), "the command it left is ended with the process");
        assert_eq!(c.fails(), 0);
    }

    #[test]
    fn closing_does_not_wait_for_a_process_that_never_exits_by_itself_and_a_helper_keeps_none() {
        let c = Chat::new("stays");
        c.mark("stays");
        c.send("one");
        let pid = c.pid().unwrap();
        let began = std::time::Instant::now();
        c.a.close_live();
        assert!(began.elapsed() < crate::live::CLOSE_WAIT, "ended by a signal, not waited for: {:?}", began.elapsed());
        assert!(gone(pid));
        // an expert, a helper started with a role, the researcher: never on a kept process, whatever the switch says
        for (name, set) in [("helper-expert", 0), ("helper-team", 1), ("helper-research", 2), ("helper-role", 3)] {
            let helper = Chat::new(name);
            {
                let mut st = helper.a.st.lock().unwrap();
                match set {
                    0 => st.parent = Some("lead".into()),
                    1 => st.team = Some("w123".into()),
                    2 => st.role = Some("research".into()),
                    _ => st.role = Some("worker".into()),
                }
            }
            assert!(helper.a.live_allowed() && !helper.launch_with("job", |_| {}).fits(&helper.a), "{name}");
            helper.send("job");
            assert_eq!(helper.kept_starts(), 0, "{name}: {:?}", helper.starts());
            assert_eq!(helper.said("text"), ["per message"], "{name}");
            assert_eq!((helper.pid(), helper.fails()), (None, 0), "{name}");
            assert!(helper.said("error").is_empty(), "{name}: {:?}", helper.said("error"));
        }
        // quitting ends an idle one before it returns
        let idle = Chat::new("quit-idle");
        idle.send("one");
        let p = idle.pid().unwrap();
        end_kept(&[idle.a.clone()], &[]);
        assert!(gone(p));
    }

    #[test]
    fn a_usage_limit_is_a_failed_turn_in_words_the_run_loop_knows() {
        let c = Chat::new("limit");
        c.send("limit");
        let errors = c.said("error");
        assert!(errors.len() == 1 && usage_limit(&errors[0]), "{errors:?}");
        assert_eq!(c.pid(), None, "a failed turn keeps no process");
    }
}

// ---------------------------------------------------------------- recordings

/// Real traffic of cursor-agent 2026.11.22 (model Auto), recorded through this client with FRIDAE_LIVE_RECORD in a
/// scratch copy of the app and played back by the stand-in, which waits for a line wherever the app sent one and
/// compares it with the line the app sent when recorded (a request by its method, an answer to Cursor whole). The
/// recordings name the scratch folder as `/scratch`. There is no recording of a request the app allowed: on Cursor it
/// allows none (Cursor's own allow list lets through what may run, and asks only about the rest).
#[cfg(unix)]
mod recorded {
    use super::*;

    const S1: &str = "00000000-0000-4000-8000-0000000000c1";
    const S2: &str = "00000000-0000-4000-8000-0000000000c2";

    impl Chat {
        fn ask(&self, text: &str) {
            self.send(text);
        }

        /// Every line the app sent was the line it sent when this was recorded.
        fn clean(&self) {
            let diff = fs::read_to_string(self.script.with_extension("sh.mismatch")).unwrap_or_default();
            assert!(diff.is_empty(), "the app no longer sends what it sent when this was recorded:\n{diff}");
        }

        /// What the app answered Cursor's permission requests in the recording being played, in order.
        fn recorded_answers(&self) -> Vec<String> {
            let all = fs::read_to_string(self.script.with_extension("sh.wire")).unwrap();
            all.lines().filter_map(|l| l.strip_prefix("> ")).filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|m| m["result"]["outcome"]["optionId"].as_str().map(str::to_string)).collect()
        }

        fn unreported(&self) -> usize {
            self.events().iter().filter(|e| e["kind"] == "usage" && e["unreported"] == true).count()
        }
    }

    #[test]
    fn recorded_tool_an_allowed_program_a_refused_one_and_a_refused_write_then_a_plain_reply_on_the_same_process() {
        // recorded: one of the app's tools; `whichshell app`, a program the user had allowed, which Cursor's allow list
        // let run with no request; `touch`, not allowed: asked about and refused; a file-tool write outside the
        // workspace: asked about and refused. Cursor carried on after each refusal and summed up
        let c = Chat::playing("rec-two", "acp-two");
        c.ask("the first message");
        assert_eq!(c.said("tool"), ["Updating its task list", "Running a command", "Running a command", "Editing a file"]);
        let text = c.said("text");
        assert_eq!(text.len(), 2, "what it said before its first tool, and its summary: {text:?}");
        assert!(text[1].contains("`touch /fridae-probe-app.txt`: refused") && text[1].contains("`whichshell app`: ran"), "{text:?}");
        let ev = c.events();
        let cards: Vec<&Value> = ev.iter().filter(|e| e["kind"] == "blocked").collect();
        assert_eq!(cards.len(), 1, "the card that offers to allow the program; none for the write");
        assert_eq!(cards[0]["commands"], json!([{"name": "touch", "command": "touch /fridae-probe-app.txt"}]));
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        assert_eq!(c.unreported(), 1, "Cursor's answer to the prompt has no usage: noted as not reported");
        assert_eq!(c.session().as_deref(), Some(S1));
        let pid = c.pid().expect("a refusal is no failure: the process is kept");
        // (the stand-in does not keep the session on disk as Cursor does)
        c.has_kept(S1);
        c.ask("the second message");
        assert_eq!(c.pid(), Some(pid), "the same process");
        assert_eq!(c.said("text").last().map(String::as_str), Some("second"));
        assert_eq!((c.unreported(), c.starts().len(), c.fails()), (2, 1, 0));
        assert_eq!(c.recorded_answers(), ["reject-once", "reject-once"]);
        c.clean();
    }

    #[test]
    fn recorded_stop_mid_command_is_confirmed_by_the_cancelled_prompt() {
        // recorded: a new process loaded the chat's conversation (Cursor played its history back), ran `sleep 50`,
        // an allowed program, and Stop was pressed while it ran: `session/cancel`, and the prompt answered
        // `cancelled` (live: the turn was over 0.05 s after Stop, and the `sleep` was gone)
        let c = Chat::playing("rec-stop", "acp-stop");
        c.has_kept(S1);
        c.start("the message that is stopped");
        let tp = c.a.st.lock().unwrap().proc_.clone().unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !(tp.tool_open.load(Ordering::SeqCst) && tp.acked.load(Ordering::SeqCst)) {
            assert!(std::time::Instant::now() < until);
            thread::sleep(Duration::from_millis(10));
        }
        let pid = c.pid().unwrap();
        let pressed = std::time::Instant::now();
        c.a.stop();
        c.wait();
        assert!(pressed.elapsed() < paths::STOP_WAIT, "confirmed, not killed after the wait: {:?}", pressed.elapsed());
        assert_eq!(c.said("error"), ["Stopped."]);
        assert_eq!(c.said("tool"), ["Running a command"], "only this turn's: the history played back by the load is not shown");
        assert!(c.said("text").iter().all(|t| !t.contains("second")), "{:?}", c.said("text"));
        assert_eq!(c.pid(), None, "Stop landed in a tool call, so the process is given up, as Claude Code's is");
        assert!(goes(pid));
        assert_eq!((c.fails(), c.session().as_deref()), (0, Some(S1)));
        c.clean(); // the line the app sent for Stop was `session/cancel`, where it was when recorded
    }

    #[test]
    fn recorded_process_killed_mid_turn_is_said_and_the_message_is_not_sent_twice() {
        // recorded: a new process loaded the conversation and answered from it (the word asked for two turns
        // earlier, on another process); on the next message it ran `sleep 50` and was killed (SIGKILL) meanwhile
        let c = Chat::playing("rec-dies", "acp-dies");
        c.mark("dies");
        c.has_kept(S1);
        c.ask("which word");
        assert_eq!(c.said("text"), ["second"], "it remembers: the conversation was loaded");
        c.ask("the message it dies in");
        assert_eq!(c.said("tool"), ["Running a command"]);
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("The agent exited unexpectedly"), "{errors:?}");
        assert_eq!(c.starts().len(), 1, "no second process: the command had already run");
        assert_eq!((c.fails(), c.pid()), (1, None));
        assert_eq!(c.session().as_deref(), Some(S1), "the next message loads it again");
        c.clean();
    }

    #[test]
    fn recorded_session_cursor_does_not_have_starts_a_new_one_and_the_chat_is_told() {
        // recorded: `session/load` of a session that was made and never given a message, which Cursor does not
        // keep: `-32602 Invalid params`, `Session "<id>" not found`; then `session/new` and the turn
        let c = Chat::playing("rec-not-found", "acp-session-not-found");
        c.has_kept(S1);
        c.ask("Reply with exactly the one word: third");
        let errors = c.said("error");
        assert!(errors.len() == 1 && errors[0].starts_with("Cursor could not find this chat's earlier conversation"), "{errors:?}");
        assert_eq!(c.said("text"), ["third"]);
        assert_eq!(c.session().as_deref(), Some(S2), "the new conversation is the chat's from now on");
        assert_eq!((c.fails(), c.starts().len()), (0, 1));
        assert!(c.pid().is_some());
        c.clean();
    }

    #[test]
    fn recorded_two_messages_with_no_tool_call_between_are_shown_as_two() {
        // recorded: Cursor answered, took a step of its own the client is not told of, and answered again; all the
        // client sees between the two answers is its thinking
        let c = Chat::playing("rec-two-messages", "acp-two-messages-no-tool");
        c.has_kept(S1);
        c.ask("Reply with exactly the one word: again");
        assert_eq!(c.said("text"), ["again", "again"], "not run together as one");
        assert!(c.said("error").is_empty());
        c.clean();
    }

    #[test]
    fn recorded_compound_commands_are_each_asked_about_for_the_part_that_is_not_allowed() {
        let _held = paths::ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = paths::set_allowed(&["git".to_string()], false);
        // recorded, with nothing allowed: `git --version && ./git`, `git --version; curl ...`, `git --version | cat`,
        // `git $(echo --version)` and `./git` (a planted script that only writes a marker, and never ran). Cursor
        // asked about each, naming every part its list does not cover; the app answered each no
        let c = Chat::playing("rec-compound", "acp-compound-nothing-allowed");
        c.ask("the five commands");
        assert_eq!(c.said("tool"), ["Running a command"; 5]);
        assert_eq!(c.recorded_answers(), ["reject-once"; 5]);
        let ev = c.events();
        let card = ev.iter().find(|e| e["kind"] == "blocked").expect("the card");
        let names: Vec<&str> = card["commands"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
        assert_eq!(names, ["git", "curl", "cat", "echo"], "each program Cursor said was missing, once");
        assert!(c.said("error").is_empty(), "{:?}", c.said("error"));
        c.clean();
        // recorded, with `git` allowed (a new process, which loaded the conversation): Cursor still asked about all
        // five, now naming only what is not `git`: `./git` (twice), `curl`, `cat`, `echo` (inside `$( )`). An
        // allowed program in a command does not let the rest of the command through, over ACP as per message
        let _ = paths::set_allowed(&["git".to_string()], true);
        let d = Chat::playing("rec-compound-allowed", "acp-compound-one-allowed");
        d.has_kept(S1);
        d.ask("the five commands again");
        let _ = paths::set_allowed(&["git".to_string()], false);
        assert_eq!(d.said("tool"), ["Running a command"; 5]);
        assert_eq!(d.recorded_answers(), ["reject-once"; 5], "every one asked about, none run on its own");
        let ev = d.events();
        let card = ev.iter().find(|e| e["kind"] == "blocked").expect("the card");
        let names: Vec<&str> = card["commands"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
        assert_eq!(names, ["curl", "cat", "echo"], "the programs that could be allowed; `./git` is not one: it is `git` by a path");
        assert!(d.said("error").iter().any(|e| e.contains("plain name")), "{:?}", d.said("error"));
        let said = d.said("text").join("\n");
        assert_eq!(said.matches("Rejected").count(), 5, "{said}");
        d.clean();
    }

    #[test]
    fn recorded_signed_out() {
        // recorded with a home folder that has no Cursor sign-in: `session/new` answered `-32000 Authentication
        // required`
        let c = Chat::playing("rec-signed-out", "acp-signed-out");
        c.ask("hello");
        assert_eq!(c.said("error"), [cursor::SIGNED_OUT]);
        assert_eq!(c.starts().len(), 1, "no second process");
        assert_eq!((c.fails(), c.pid(), c.session()), (0, None, None));
        c.clean();
    }
}
