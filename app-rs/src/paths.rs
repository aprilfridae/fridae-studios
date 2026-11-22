//! Where things live, whether the app runs packaged or from the source folder.
//!
//! KIT   read-only program files (ui/, studio/, vault/ ...)
//! HOME  settings and the update staging folder
//! WORK  everything the tools and chats produce

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::LazyLock,
};

use serde_json::{Map, Value};

pub const APP: &str = "Fridae Studios";
pub const VERSION: &str = env!("FRIDAE_VERSION");
pub const UPDATE_REPO: &str = env!("FRIDAE_REPO");
pub const EXE_NAME: &str = "Fridae Studios.exe";
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

pub struct Paths {
    pub install: PathBuf,
    pub kit: PathBuf,
    pub home: PathBuf,
    pub work: PathBuf,
    /// The packaged Python tools, when this is an installed copy.
    pub cli_exe: Option<PathBuf>,
}

pub static PATHS: LazyLock<Paths> = LazyLock::new(|| {
    let mut install = env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    // the Mac app keeps the packaged tools in Contents/Resources/tools (code signing wants only programs in Contents/MacOS)
    let mac_tools = install.join("..").join("Resources").join("tools");
    if cfg!(target_os = "macos") && mac_tools.join("fridae-cli").exists() {
        install = mac_tools.canonicalize().unwrap_or(mac_tools);
    }
    let cli_exe = Some(install.join(if cfg!(windows) { "fridae-cli.exe" } else { "fridae-cli" })).filter(|p| p.exists());
    let has_ui = |p: &Path| p.join("ui").join("index.html").exists();
    let kit = if has_ui(&install.join("_internal")) {
        install.join("_internal")
    } else if has_ui(&install) {
        install.clone()
    } else {
        env::var("FRIDAE_KIT").map(PathBuf::from).unwrap_or_else(|_| install.clone())
    };
    let home = if cli_exe.is_some() { home_dir().join("Documents").join(APP) } else { kit.clone() };
    // tests never write to a real work folder, whatever RBXKIT_WORK says: one temp folder per test run
    #[cfg(test)]
    let work = test_dir("work");
    #[cfg(not(test))]
    let work = env::var("RBXKIT_WORK").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join("work"));
    Paths { install, kit, home, work, cli_exe }
});

/// Where this test run keeps its files: <temp>/fridae-tests/<pid>/<name>. The first call clears the folders of test
/// runs that are over, and the per-test folders older test runs left straight in <temp> (fridae-<test>-<pid>).
#[cfg(test)]
pub fn test_dir(name: &str) -> PathBuf {
    static ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
        let base = env::temp_dir().join("fridae-tests");
        sweep_test_dirs(&base, &env::temp_dir());
        base.join(std::process::id().to_string())
    });
    ROOT.join(name)
}

/// The pid at the end of a test folder's name ("fridae-team-note-123" -> 123), for the old per-test folders.
#[cfg(test)]
fn legacy_test_pid(name: &str) -> Option<u32> {
    const OLD: [&str; 8] = ["fridae-test-work-", "fridae-lock-", "fridae-promotab-", "fridae-media-", "fridae-studio-run-", "fridae-studio-undo-", "fridae-promo-", "fridae-team-"];
    if !OLD.iter().any(|p| name.starts_with(p)) {
        return None;
    }
    name.rsplit('-').next()?.parse().ok()
}

/// Whether a process is still running (a test run that may still use its folder). Off Unix it is taken to be, and
/// only folders a day old are cleared.
#[cfg(test)]
fn test_run_alive(pid: u32, folder: &Path) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        let _ = folder;
        Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        let age = fs::metadata(folder).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        age.is_none_or(|a| a.as_secs() < 86400)
    }
}

#[cfg(test)]
fn sweep_test_dirs(base: &Path, temp: &Path) {
    let mut alive: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
    let mut gone = |pid: u32, folder: &Path| !*alive.entry(pid).or_insert_with(|| test_run_alive(pid, folder));
    for e in fs::read_dir(base).into_iter().flatten().flatten() {
        if let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) {
            if gone(pid, &e.path()) {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    for e in fs::read_dir(temp).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(pid) = legacy_test_pid(&name) {
            if e.path().is_dir() && gone(pid, &e.path()) {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
}

/// The kit's Studio team folder inside the vault: roster.json and one <id>.md per expert.
pub fn kit_team(kit: &Path) -> PathBuf {
    kit.join("vault").join("Fridae").join("Team")
}

pub fn home_dir() -> PathBuf {
    PathBuf::from(env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default())
}

pub fn packaged() -> bool {
    PATHS.cli_exe.is_some()
}

pub fn work(name: &str) -> PathBuf {
    PATHS.work.join(name)
}

/// Command line that starts the tool server agents connect to.
pub fn mcp_command() -> Vec<String> {
    match &PATHS.cli_exe {
        Some(exe) => vec![exe.display().to_string(), "--mcp".into()],
        None => vec![python().into(), PATHS.kit.join("rbxkit_mcp.py").display().to_string()],
    }
}

/// A command that runs the rbx tool with these arguments, without flashing a console window.
pub fn cli(args: &[&str]) -> Command {
    let mut c = match &PATHS.cli_exe {
        Some(exe) => {
            let mut c = Command::new(exe);
            c.arg("--rbx");
            c
        }
        None => {
            let mut c = Command::new(python());
            c.arg(PATHS.kit.join("rbx.py"));
            c
        }
    };
    c.args(args);
    quiet(&mut c);
    c
}

/// The Python program: `python` on Windows, `python3` elsewhere.
pub fn python() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

#[allow(unused_variables)]
pub fn quiet(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(NO_WINDOW);
    }
}

/// On Unix, starts the command in a process group of its own, so `end_trees` can end it and everything it starts
/// without touching the app. Nothing changes on Windows (taskkill /T follows the tree there).
pub fn own_group(c: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = c;
}

/// Ends what a finished turn left running in its process group (Cursor's worker-server), without holding up the turn.
/// Called once the turn's output has ended but before its leader is reaped: an unreaped leader keeps the group's ID
/// reserved, so SIGTERM can only reach that group. Any member still there after `STOP_GRACE` gets SIGKILL from a
/// background thread, but only when the group then has a member besides its leader, and only to a group that still
/// has one (members keep the ID reserved; the leader, reaped meanwhile, does not, so a group of the leader alone is
/// never signalled again, and its ID can be given out again without harm). Not on Windows, where Cursor is not offered.
pub fn end_group_soon(group: u32) {
    #[cfg(unix)]
    {
        if group <= 1 || group == std::process::id() {
            return;
        }
        let signal = move |sig: &str| signal_group(group, sig);
        signal("-TERM");
        if !group_has_members(group) {
            return;
        }
        std::thread::spawn(move || {
            std::thread::sleep(STOP_GRACE);
            if group_has_members(group) {
                signal("-KILL");
            }
        });
    }
    #[cfg(not(unix))]
    let _ = group;
}

/// Whether a process group has a member other than its leader (`ps`; no when it cannot be read).
#[cfg(unix)]
fn group_has_members(group: u32) -> bool {
    let out = Command::new("ps").args(["-A", "-o", "pid=,pgid="]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    out.lines().any(|l| {
        let mut w = l.split_whitespace().filter_map(|x| x.parse::<u32>().ok());
        matches!((w.next(), w.next()), (Some(pid), Some(g)) if g == group && pid != group)
    })
}

/// Waits, up to `limit`, until the leader of a turn's group has exited by itself, without reaping it (it shows as a
/// zombie until `wait`), so a leader still finishing its own work after its output closed (saving the chat) is not
/// signalled, and its unreaped pid keeps the group's ID reserved for `end_group_soon`. Returns whether it exited.
pub fn leader_exited(pid: u32, limit: std::time::Duration) -> bool {
    #[cfg(unix)]
    {
        let until = std::time::Instant::now() + limit;
        loop {
            let state = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
            if state.is_empty() || state.starts_with('Z') {
                return true;
            }
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, limit);
        true
    }
}

/// A turn with no output at all for this long is ended (not a Lead waiting on its experts, nor while a texture is
/// being made: those wait on purpose). Longer than a tool server call may take (`MCP_TOOL_TIMEOUT`, 15 minutes), so a
/// tool that hangs returns a time-out the AI can act on instead of ending the turn.
pub const STALL: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// What the chat is told when a turn was ended for that.
pub fn stall_message() -> String {
    format!("Nothing came from the AI for {} minutes, so Fridae ended this turn. Send your message again to carry on.", STALL.as_secs() / 60)
}

/// Tools of the app's own that wait long on purpose (a Lead waiting on experts, a texture being made): no stall then.
pub fn waits_long(tool: &str) -> bool {
    let t = tool.rsplit("__").next().unwrap_or(tool);
    ["rbx_team", "rbx_texture", "rbx_generate_texture"].iter().any(|w| t == *w || t.ends_with(&format!("_{w}")))
}

/// How long a turn may send nothing while one of its tool calls is open (a long build, a test run): a tool that
/// hangs is ended with the turn after this.
pub const STALL_WORKING: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Whether a turn that has sent nothing for `quiet` is stalled: not waiting on purpose, and STALL or more (with a tool
/// call open, STALL_WORKING or more).
pub fn stalled(quiet: std::time::Duration, waiting_long: bool, tool_open: bool) -> bool {
    !waiting_long && quiet >= if tool_open { STALL_WORKING } else { STALL }
}

/// How long a stopped agent gets to end by itself (SIGTERM) before it is killed (Unix).
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(2000);

/// How long Stop waits for a turn to end by itself, once asked, before its processes are killed (every AI).
pub const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(if cfg!(test) { 2 } else { 15 });

/// What the chat is told when a turn did not end within STOP_WAIT of Stop and was killed.
pub fn killed_message(ai: &str) -> String {
    format!("{ai} did not stop within {} seconds of Stop, so Fridae ended it.", STOP_WAIT.as_secs())
}

/// Unix: sends `sig` ("-TERM", "-KILL", "-0") to process group `group`; true when the group had a member. Never the
/// app's own group, nor 0 or 1.
pub fn signal_group(group: u32, sig: &str) -> bool {
    #[cfg(unix)]
    {
        if group <= 1 || group == std::process::id() {
            return false;
        }
        Command::new("kill")
            .args([sig, "--", &format!("-{group}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        let _ = (group, sig);
        false
    }
}

/// How much of a child's error output is kept: it is always read to the end (a full pipe would block the child),
/// and only this much of its tail is kept.
pub const TAIL: usize = 64 * 1024;

/// Reads `r` to its end and returns the last `cap` bytes of it, as text.
#[cfg(test)]
pub fn drain_tail(r: impl std::io::Read, cap: usize) -> String {
    let kept = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    drain_tail_into(r, cap, &kept);
    tail_text(&kept)
}

/// The same, keeping the tail in `kept` as it reads, so what was read so far can be taken while the reading still
/// waits (`tail_text`).
pub fn drain_tail_into(mut r: impl std::io::Read, cap: usize, kept: &std::sync::Mutex<std::collections::VecDeque<u8>>) {
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
                k.extend(&buf[..n]);
                let over = k.len().saturating_sub(cap);
                k.drain(..over);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// What a `drain_tail_into` has kept so far, as text.
pub fn tail_text(kept: &std::sync::Mutex<std::collections::VecDeque<u8>>) -> String {
    let k = kept.lock().unwrap_or_else(|e| e.into_inner());
    String::from_utf8_lossy(&k.iter().copied().collect::<Vec<u8>>()).trim().to_string()
}

/// The longest line of an AI's output that is kept (a tool result carrying a big picture can be one line).
pub const LINE_CAP: usize = 64 * 1024 * 1024;

/// The lines of `r`, each at most `cap` bytes: a longer one is read to its end and dropped, and comes as None (so the
/// chat can say so) instead of being held whole in memory. Ends at the end of `r` or on a read error.
pub struct CappedLines<R: std::io::BufRead> {
    r: R,
    cap: usize,
}

impl<R: std::io::BufRead> CappedLines<R> {
    pub fn new(r: R, cap: usize) -> Self {
        CappedLines { r, cap }
    }
}

impl<R: std::io::BufRead> Iterator for CappedLines<R> {
    type Item = Option<Vec<u8>>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut line = Vec::new();
        let mut over = false;
        let mut any = false;
        loop {
            let buf = match self.r.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            if buf.is_empty() {
                return any.then_some(if over { None } else { Some(line) });
            }
            any = true;
            let (take, done) = match buf.iter().position(|&b| b == b'\n') {
                Some(i) => (i, true),
                None => (buf.len(), false),
            };
            if !over {
                if line.len() + take > self.cap {
                    over = true;
                    line = Vec::new();
                } else {
                    line.extend_from_slice(&buf[..take]);
                }
            }
            self.r.consume(if done { take + 1 } else { take });
            if done {
                return Some(if over { None } else { Some(line) });
            }
        }
    }
}

/// Variables of a Claude Code that runs the app (while developing it): a Claude child given them would think it runs
/// inside that one (nested-session checks, its messaging socket). Removed from every AI's process.
pub const CLAUDE_SESSION_VARS: [&str; 6] = ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_PID", "CLAUDE_AGENT_SDK_VERSION"];

/// Provider API keys that would make an AI bill an API account instead of the plan the user signed in with. Removed
/// from that AI's process. The Gemini keys stay where nothing else would use them: the app's textures read them
/// (`rbxgen.py`), and Antigravity signs in with the user's Google account by its own settings.
pub fn plan_keys(ai: &str) -> &'static [&'static str] {
    match ai {
        "claude" => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"],
        "cursor" => &["CURSOR_API_KEY"],
        "chatgpt" => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        // OpenCode's server would add a provider for each of these, billed to that key; its tool servers get the
        // Gemini keys back (textures)
        "opencode" => &[
            "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "OPENAI_API_KEY", "OPENROUTER_API_KEY", "GROQ_API_KEY", "XAI_API_KEY", "DEEPSEEK_API_KEY",
            "MISTRAL_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY", "AWS_BEARER_TOKEN_BEDROCK", "AZURE_OPENAI_API_KEY",
            "CURSOR_API_KEY",
        ],
        _ => &[],
    }
}

/// The Setup switch "Use API keys from my environment": off unless the user turned it on, since a key in the
/// environment would bill that API account instead of the plan the user signed in with.
pub fn env_keys_on() -> bool {
    setting("use_env_keys").as_bool().unwrap_or(false)
}

/// Whether a variable is kept from an AI's process: a Claude session's always; a provider key (plan_keys) unless the
/// user turned that switch on; OpenCode's always (each key there adds a provider billed to it, so the switch, meant
/// for a Claude gateway token, does not hand them over; the label says so). For OpenCode the switch is not read.
pub fn env_dropped(key: &str, ai: &str) -> bool {
    CLAUDE_SESSION_VARS.contains(&key) || (plan_keys(ai).contains(&key) && (ai == "opencode" || !env_keys_on()))
}

/// Variables `/bin/sh` (bash as sh, dash) acts on at start: functions brought in from the environment (`BASH_FUNC_x%%`
/// replaces the program `x`), start-up files, `cd`'s search folders, options and the trace prompt. Never handed to a
/// shell the app's checks approve commands for (OpenCode's), so an allowed plain name is the program on PATH.
pub fn shell_injected(key: &str) -> bool {
    key.starts_with("BASH_FUNC_") || ["BASH_ENV", "ENV", "CDPATH", "SHELLOPTS", "BASHOPTS", "PS4", "IFS", "GLOBIGNORE"].contains(&key)
}

/// The provider keys of an AI that are in the app's environment now (names only), for the sign-in message.
pub fn env_keys_present(ai: &str) -> Vec<&'static str> {
    plan_keys(ai).iter().copied().filter(|k| env::var_os(k).is_some_and(|v| !v.is_empty())).collect()
}

/// The environment hygiene every AI's process gets (see CLAUDE_SESSION_VARS, plan_keys, env_keys_on); Antigravity is
/// also told not to update itself mid-turn.
pub fn child_env(cmd: &mut Command, ai: &str) {
    let keys_on = env_keys_on() && ai != "opencode";
    for k in CLAUDE_SESSION_VARS.iter().chain(plan_keys(ai).iter().filter(|_| !keys_on)) {
        cmd.env_remove(k);
    }
    if ai == "gemini" {
        cmd.env("AGY_CLI_DISABLE_AUTO_UPDATE", "1");
    }
}

/// Ends each process (started with `own_group`) and everything it started, and returns once they are gone.
/// Unix: SIGTERM to each process group, then SIGKILL to any group still there after `STOP_GRACE`. A pid that is
/// not a group of its own is never signalled as a group of the app's: the app's own pid and pids 0/1 are skipped.
/// Windows: `taskkill /T /F`, as always.
pub fn end_trees(pids: &[u32]) {
    #[cfg(windows)]
    for pid in pids {
        let mut kill = Command::new("taskkill");
        kill.args(["/T", "/F", "/PID", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        quiet(&mut kill);
        let _ = kill.status();
    }
    #[cfg(unix)]
    {
        let me = std::process::id();
        let groups: Vec<u32> = pids.iter().copied().filter(|&p| p > 1 && p != me).collect();
        let signal = |sig: &str, group: u32| signal_group(group, sig);
        for &g in &groups {
            signal("-TERM", g);
        }
        let until = std::time::Instant::now() + STOP_GRACE;
        let mut left = groups;
        loop {
            left.retain(|&g| signal("-0", g)); // still has a member
            if left.is_empty() || std::time::Instant::now() >= until {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        for &g in &left {
            signal("-KILL", g);
        }
    }
}

/// Whether the disk the user's files are on ignores case (a Mac's and Windows' usually do): the home folder is found
/// under its name with every letter's case swapped. Rules that keep the AIs' file tools away from a file must then
/// match it in any case.
pub fn case_blind() -> bool {
    static BLIND: LazyLock<bool> = LazyLock::new(|| {
        let home = home_dir().display().to_string();
        let swapped: String = home.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect();
        swapped != home && Path::new(&swapped).exists()
    });
    *BLIND
}

// ----------------------------------------------------------------- settings
/// The app's settings file. Every AI's rules deny writing it.
pub fn config_file() -> PathBuf {
    PATHS.home.join("config.json")
}

/// What no agent's file tools may write, in any mode, on any AI whose rules the app sets: the app's settings, and the
/// files in the work folder the app reads and acts on (the process ledger, which it uses to end processes; the
/// requests its tools leave for it; every chat's saved state; the tool lists it gives Claude Code; undo copies; usage).
/// As (path, is a folder). The tool servers write some of these themselves; the rules bind only the agents' own tools.
pub fn app_protected() -> Vec<(PathBuf, bool)> {
    let w = &PATHS.work;
    let mut out = vec![(config_file(), false), (w.join("processes.json"), false), (w.join("usage.jsonl"), false), (w.join("limits.json"), false)];
    // the copy of the guarded settings (`guard_copy`): changing a setting then takes a command, not two file writes
    out.push((guard_copy(), false));
    for d in ["processes", "chats", "team-requests", "stage-requests", "texture-requests", "team-results", "team-nudge", "agent-tools", "before-undo"] {
        out.push((w.join(d), true));
    }
    out
}

/// One lock around every change to the settings file, so two changes never interleave (reads need none: the file is
/// replaced whole, never written in place).
static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn load_config() -> Map<String, Value> {
    fs::read_to_string(config_file())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Change the settings: read, change and write back under the one lock, the file replaced whole (a temporary file
/// renamed over it), so a reader never sees half a file and two changes never undo each other. The guarded settings
/// (`GUARDED`) come from memory, after taking in what another running copy of the app changed (the copy's counter is
/// newer than this app's); only the guarded keys the change itself sets are written over them.
pub fn update_config<R>(change: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
    let _held = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let st = store();
    let mut g = guard_locked(&st);
    update_in(g.as_mut().expect("started"), &st, change)
}

// --------------------------------------------------------- guarded settings
/// The settings that widen what an AI may do or can cost money. While the app runs its memory is the authority for
/// them: they change only through the app's own routes (`update_config`), every decision reads them from memory
/// (`setting`), and a different value in the file (an AI's file tool past its deny rule: Antigravity's compares case,
/// measured) is put back (`check_settings`) before a turn starts, before an expert starts and before a turn's end.
/// Across restarts, and between two copies of the app on one home, a copy of them is kept outside every folder an AI
/// is given, one per home folder (`guard_copy`), with a counter raised by every change through a route.
/// - `allowed_commands`: the programs agents run without asking
/// - `use_env_keys`: API keys in the environment reach the AIs (billed to that account)
/// - `gemini_use_api`, `gemini_key`: textures billed to the Google key
/// - `projects`: the games, whose folders the AIs are given
/// - `plugin_token`: the Studio plugin's secret
/// - `expert_ai`, `lead_picks_ai`: which AI and model experts run on (a bigger model costs more of a plan)
/// (`blender`, the program run as Blender, is set from a terminal with `rbx doctor`, so it is not one of them: a path
/// inside a folder an AI is given is never used instead, `blender_ok`.)
pub const GUARDED: &[&str] = &["allowed_commands", "use_env_keys", "gemini_use_api", "gemini_key", "projects", "plugin_token", "expert_ai", "lead_picks_ai"];

/// A guarded setting's name as people read it (Setup's words).
pub fn setting_name(key: &str) -> &'static str {
    match key {
        "allowed_commands" => "programs agents may run",
        "use_env_keys" => "Use API keys from my environment",
        "gemini_use_api" => "paid Gemini textures",
        "gemini_key" => "Gemini API key",
        "projects" => "list of games",
        "plugin_token" => "Studio plugin link",
        "expert_ai" => "Experts table",
        "lead_picks_ai" => "Lead may pick the AI",
        _ => "a setting",
    }
}

/// Where one home folder's settings live and where their copy is kept.
pub struct Store {
    pub home: PathBuf,
    pub config: PathBuf,
    pub copy: PathBuf,
}

fn store() -> Store {
    Store { home: PATHS.home.clone(), config: config_file(), copy: guard_copy() }
}

/// What one running app knows of the guarded settings.
#[derive(Default)]
pub struct Guard {
    known: Map<String, Value>,
    /// the copy's counter this memory is up to
    version: u64,
    /// what was put back at start, for the page, until the user answers it
    at_start: Option<StartNote>,
}

#[derive(Clone)]
struct StartNote {
    /// each setting put back: (key, what changed, in plain words)
    changes: Vec<(String, String)>,
    /// the file's guarded values as they were, for "Use the file's settings instead"
    file: Map<String, Value>,
    /// the settings file could not be read (kept beside it as config.json.bad-<time>)
    unreadable: bool,
}

static GUARD: std::sync::Mutex<Option<Guard>> = std::sync::Mutex::new(None);

/// The real home's guard, started if need be. Call with the settings lock held.
fn guard_locked(st: &Store) -> std::sync::MutexGuard<'static, Option<Guard>> {
    let mut g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(start_in(st));
    }
    g
}

/// The folder copies are kept in: outside the work folder, the games and the kit (macOS `~/Library/Application
/// Support/Fridae Studios`, Windows `%APPDATA%\Fridae Studios`, Linux `$XDG_CONFIG_HOME/fridae-studios` or
/// `~/.config/fridae-studios`).
fn guard_dir() -> PathBuf {
    #[cfg(test)]
    return test_dir("guard-copy");
    #[allow(unreachable_code)]
    {
        let home = home_dir();
        if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support").join(APP)
        } else if cfg!(windows) {
            env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming")).join(APP)
        } else {
            env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("fridae-studios")
        }
    }
}

fn real_home(home: &Path) -> String {
    home.canonicalize().unwrap_or_else(|_| home.to_path_buf()).display().to_string()
}

/// The copy for one home folder: named by a hash of its real path, and holding that path, so a dev build (whose home
/// is the kit) and the installed app (`~/Documents/Fridae Studios`) never read or write each other's.
pub fn guard_copy_for(home: &Path) -> PathBuf {
    use sha2::Digest;
    let h = sha2::Sha256::digest(real_home(home).as_bytes());
    guard_dir().join(format!("settings-guard-{}.json", hex(&h[..8])))
}

pub fn guard_copy() -> PathBuf {
    guard_copy_for(&PATHS.home)
}

fn guarded_of(cfg: &Map<String, Value>) -> Map<String, Value> {
    GUARDED.iter().filter_map(|k| cfg.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
}

fn load_config_at(path: &Path) -> Map<String, Value> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn unreadable(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|t| !serde_json::from_str::<Value>(&t).ok().is_some_and(|v| v.is_object()))
}

/// The copy's counter and settings, or None when there is none, it does not read, or it was made for another home
/// (never applied then).
fn read_copy(st: &Store) -> Option<(u64, Map<String, Value>)> {
    let v: Value = serde_json::from_str(&fs::read_to_string(&st.copy).ok()?).ok()?;
    if v["home"].as_str() != Some(real_home(&st.home).as_str()) {
        return None;
    }
    Some((v["version"].as_u64().unwrap_or(0), guarded_of(v["settings"].as_object()?)))
}

fn write_copy(st: &Store, version: u64, settings: &Map<String, Value>) {
    let mut v = Map::new();
    v.insert("home".into(), Value::from(real_home(&st.home)));
    v.insert("version".into(), Value::from(version));
    v.insert("settings".into(), Value::Object(settings.clone()));
    write_config(&st.copy, &v);
    // it holds the Gemini key and the plugin's secret: the user's alone
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&st.copy, fs::Permissions::from_mode(0o600));
    }
}

/// The file with the guarded keys set to `known` (others as they are).
fn write_with(st: &Store, known: &Map<String, Value>) {
    keep_unreadable(&st.config);
    let mut cfg = load_config_at(&st.config);
    for k in GUARDED {
        match known.get(*k) {
            Some(v) => cfg.insert(k.to_string(), v.clone()),
            None => cfg.remove(*k),
        };
    }
    write_config(&st.config, &cfg);
}

/// Takes in what another running copy of the app changed through its routes (its counter is newer).
fn sync_from_copy(g: &mut Guard, st: &Store) {
    if let Some((v, c)) = read_copy(st).filter(|(v, _)| *v > g.version) {
        g.known = c;
        g.version = v;
    }
}

/// What changed in one setting, in plain words, never showing a secret.
fn describe_change(key: &str, file: Option<&Value>, kept: Option<&Value>) -> String {
    let name = setting_name(key);
    let names = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array).into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string).or_else(|| e["name"].as_str().map(str::to_string)).or_else(|| e["path"].as_str().map(str::to_string))).collect()
    };
    match key {
        "allowed_commands" | "projects" => {
            let (f, k) = (names(file), names(kept));
            let added: Vec<&String> = k.iter().filter(|x| !f.contains(x)).collect();
            let dropped: Vec<&String> = f.iter().filter(|x| !k.contains(x)).collect();
            let mut parts = Vec::new();
            if !dropped.is_empty() {
                parts.push(format!("the file also had {}, which was taken out", dropped.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            if !added.is_empty() {
                parts.push(format!("the file was missing {}, which was put back", added.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            if parts.is_empty() {
                parts.push("the order or details differed".into());
            }
            format!("{name}: {}", parts.join("; "))
        }
        "gemini_key" | "plugin_token" => match (file.is_some(), kept.is_some()) {
            (true, false) => format!("{name}: the file had one Fridae did not know; it was removed"),
            (false, true) => format!("{name}: missing from the file; put back"),
            _ => format!("{name}: different in the file; Fridae's was put back"),
        },
        _ => {
            let show = |v: Option<&Value>| v.map(|v| if let Some(b) = v.as_bool() { if b { "on".to_string() } else { "off".to_string() } } else { "set".to_string() }).unwrap_or_else(|| "not set".into());
            format!("{name}: the file said {}, put back to {}", show(file), show(kept))
        }
    }
}

/// At start: the copy for this home wins over the settings file where they differ (the file is put back and the page
/// says which settings, in plain words, until the user answers, with a way to take the file's instead); with no copy
/// for this home (a first run), the file is taken as it is and the copy written.
fn start_in(st: &Store) -> Guard {
    let broken = unreadable(&st.config);
    if broken {
        keep_unreadable(&st.config);
    }
    let file = guarded_of(&load_config_at(&st.config));
    let Some((version, copy)) = read_copy(st) else {
        write_copy(st, 1, &file);
        return Guard { known: file, version: 1, at_start: None };
    };
    let changes: Vec<(String, String)> = GUARDED.iter().filter(|k| file.get(**k) != copy.get(**k)).map(|k| (k.to_string(), describe_change(k, file.get(*k), copy.get(*k)))).collect();
    let mut at_start = None;
    if !changes.is_empty() {
        write_with(st, &copy);
        at_start = Some(StartNote { changes, file, unreadable: broken });
    }
    Guard { known: copy, version, at_start }
}

fn update_in<R>(g: &mut Guard, st: &Store, change: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
    keep_unreadable(&st.config);
    sync_from_copy(g, st);
    let mut cfg = load_config_at(&st.config);
    for k in GUARDED {
        match g.known.get(*k) {
            Some(v) => cfg.insert(k.to_string(), v.clone()),
            None => cfg.remove(*k),
        };
    }
    let before = guarded_of(&cfg);
    let out = change(&mut cfg);
    // only what the change itself set is the route's: it becomes the known value, with the counter raised
    let mut changed = false;
    for k in GUARDED {
        if before.get(*k) != cfg.get(*k) {
            changed = true;
            match cfg.get(*k) {
                Some(v) => g.known.insert(k.to_string(), v.clone()),
                None => g.known.remove(*k),
            };
        }
    }
    if changed {
        g.version = g.version.max(read_copy(st).map(|(v, _)| v).unwrap_or(0)) + 1;
        write_copy(st, g.version, &g.known);
    }
    write_config(&st.config, &cfg);
    out
}

/// Puts back any guarded key in the file that differs from memory (after taking in another copy's newer changes).
/// The keys put back; "unreadable" first when the file could not be read.
fn check_in(g: &mut Guard, st: &Store) -> Vec<String> {
    sync_from_copy(g, st);
    let broken = unreadable(&st.config);
    let file = guarded_of(&load_config_at(&st.config));
    let mut back: Vec<String> = GUARDED.iter().filter(|k| file.get(**k) != g.known.get(**k)).map(|k| k.to_string()).collect();
    if !back.is_empty() {
        write_with(st, &g.known);
        if broken {
            back.insert(0, "unreadable".into());
        }
    }
    back
}

/// The known guarded settings.
fn known() -> Map<String, Value> {
    if GUARD.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        start_settings();
    }
    GUARD.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|g| g.known.clone()).unwrap_or_default()
}

/// At start (see `start_in`).
pub fn start_settings() {
    let _held = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    drop(guard_locked(&store()));
}

/// A guarded setting's value as the app knows it (Null when unset). Every decision reads them here, never the file.
pub fn setting(key: &str) -> Value {
    debug_assert!(GUARDED.contains(&key));
    known().get(key).cloned().unwrap_or(Value::Null)
}

/// All guarded settings as the app knows them, as a settings map (for code that takes one).
pub fn guarded_settings() -> Map<String, Value> {
    known()
}

/// What was put back at start, for the page: each setting's plain-words change, until the user answers.
pub fn put_back_at_start() -> Value {
    let g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_ref().and_then(|g| g.at_start.clone()) {
        Some(n) => serde_json::json!({"changes": n.changes.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>(), "unreadable": n.unreadable}),
        None => Value::Null,
    }
}

/// The user's answer to the start-up line: "file" takes the settings file's values as they were at start (through
/// the app's own route), anything else just closes the line.
pub fn answer_start_note(take_file: bool) -> bool {
    let note = GUARD.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|g| g.at_start.take());
    let Some(note) = note else { return false };
    if take_file {
        update_config(|cfg| {
            for k in GUARDED {
                match note.file.get(*k) {
                    Some(v) => cfg.insert(k.to_string(), v.clone()),
                    None => cfg.remove(*k),
                };
            }
        });
    }
    true
}

/// Compares the settings file with the known values (after taking in another running copy's newer changes): any
/// guarded setting that differs is put back. Returns the keys put back ("unreadable" first if the file did not read).
pub fn check_settings() -> Vec<String> {
    let _held = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let st = store();
    let mut g = guard_locked(&st);
    check_in(g.as_mut().expect("started"), &st)
}

/// The chat's line when settings were put back.
pub fn put_back_line(keys: &[String]) -> String {
    let unread = keys.first().is_some_and(|k| k == "unreadable");
    let names: Vec<&str> = keys.iter().filter(|k| *k != "unreadable").map(|k| setting_name(k)).collect();
    if unread {
        format!(
            "The settings file could not be read (Fridae kept it beside the new one as config.json.bad-<time>), so Fridae wrote its own settings back: {}. Change settings in Setup.",
            names.join(", ")
        )
    } else {
        format!("A setting was changed by something other than Fridae ({}), so Fridae put it back as you had it. Change settings in Setup.", names.join(", "))
    }
}

/// A settings file that exists but does not read as settings (a hand edit with a stray comma, an empty file after a
/// crash) is kept beside it as `config.json.bad-<time>` before a change writes a fresh one, so nothing in it is lost.
fn keep_unreadable(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else { return };
    if serde_json::from_str::<Value>(&text).ok().is_some_and(|v| v.is_object()) {
        return;
    }
    let _ = fs::copy(path, path.with_file_name(format!("config.json.bad-{}", stamp())));
}

fn write_config(path: &Path, cfg: &Map<String, Value>) {
    let Some(dir) = path.parent() else { return };
    let _ = fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        // a name no other write uses, even in the same second from another thread
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!(".config.json.{}-{}-{n}.tmp", std::process::id(), stamp()));
        // on disk before it replaces the old file (a power cut just after leaves the old or the new, never an empty one)
        let written = fs::File::create(&tmp).and_then(|mut f| {
            use std::io::Write;
            f.write_all(text.as_bytes())?;
            f.sync_all()
        });
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        // Windows refuses a rename over a file another program has open for a moment (a virus scanner): tried again
        for attempt in 0..5 {
            if fs::rename(&tmp, path).is_ok() {
                return;
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        let _ = fs::remove_file(&tmp);
    }
}

// --------------------------------------------------------------- environment
/// Look a program up on PATH, trying the usual Windows extensions.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for ext in ["", ".exe", ".cmd", ".bat"] {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The Claude Code program, if it is installed.
pub fn find_agent() -> Option<PathBuf> {
    which("claude").or_else(|| Some(home_dir().join(".local").join("bin").join(if cfg!(windows) { "claude.exe" } else { "claude" })).filter(|p| p.exists()))
}

/// Antigravity's command-line tool, if it is installed.
pub fn find_antigravity() -> Option<PathBuf> {
    env::var("FRIDAE_AGY").ok().map(PathBuf::from).filter(|p| p.is_file()).or_else(|| which("agy")).or_else(|| {
        Some(PathBuf::from(env::var("LOCALAPPDATA").unwrap_or_default()).join("agy").join("bin").join("agy.exe")).filter(|p| p.exists())
    })
}

/// OpenAI's command-line tool (ChatGPT), if it is installed.
pub fn find_codex() -> Option<PathBuf> {
    // OpenAI's standalone installer puts it under ~/.codex without adding it to PATH
    env::var("FRIDAE_CODEX").ok().map(PathBuf::from).filter(|p| p.is_file()).or_else(|| which("codex")).or_else(|| {
        Some(home_dir().join(".codex").join("packages").join("standalone").join("current").join("bin").join("codex.exe")).filter(|p| p.is_file())
    })
}

/// The Cursor Agent command-line tool, if it is installed. Its installer puts it in ~/.local/bin, which an app started
/// from the Dock does not have on PATH.
pub fn find_cursor() -> Option<PathBuf> {
    // not on Windows yet: see DEVELOPING.md, "Cursor as a chat's AI"
    if cfg!(windows) {
        return None;
    }
    env::var("FRIDAE_CURSOR").ok().map(PathBuf::from).filter(|p| p.is_file()).or_else(|| which("cursor-agent")).or_else(|| {
        let name = if cfg!(windows) { "cursor-agent.cmd" } else { "cursor-agent" };
        [home_dir().join(".local").join("bin").join(name), PathBuf::from(env::var("LOCALAPPDATA").unwrap_or_default()).join("cursor-agent").join(name)]
            .into_iter()
            .find(|p| p.is_file())
    })
}

/// OpenCode's command-line tool, if it is installed. Its installer puts it in ~/.opencode/bin, which is on no PATH an
/// app gets (the installer adds it in the shell's start-up file).
pub fn find_opencode() -> Option<PathBuf> {
    // not on Windows yet: see DEVELOPING.md, "OpenCode as a chat's AI"
    if cfg!(windows) {
        return None;
    }
    env::var("FRIDAE_OPENCODE").ok().map(PathBuf::from).filter(|p| p.is_file()).or_else(|| which("opencode")).or_else(|| {
        Some(home_dir().join(".opencode").join("bin").join("opencode")).filter(|p| p.is_file())
    })
}

/// The AIs a chat can run on. Cursor and OpenCode are not offered on Windows yet (see DEVELOPING.md).
pub const AIS: &[&str] = if cfg!(windows) { &["claude", "gemini", "chatgpt"] } else { &["claude", "gemini", "chatgpt", "cursor", "opencode"] };

/// The program behind one of them, if it is installed.
pub fn find_ai(kind: &str) -> Option<PathBuf> {
    match kind {
        "gemini" => find_antigravity(),
        "chatgpt" => find_codex(),
        "cursor" => find_cursor(),
        "opencode" => find_opencode(),
        _ => find_agent(),
    }
}

/// The AI new chats start on, chosen in Setup. Claude unless the user picked another.
pub fn default_ai() -> String {
    ai_choice(load_config().get("chat_ai").and_then(Value::as_str)).to_string()
}

/// The AI new chats start on, from the saved choice. ChatGPT is no longer offered in the window, so a choice of it made
/// before reads as Claude, as the window shows it.
fn ai_choice(saved: Option<&str>) -> &'static str {
    match saved {
        Some("gemini" | "antigravity") => "gemini",
        Some("cursor") if AIS.contains(&"cursor") => "cursor",
        Some("opencode") if AIS.contains(&"opencode") => "opencode",
        _ => "claude",
    }
}

/// The launch file Roblox Studio creates when its MCP server is turned on.
pub fn studio_mcp() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // on a Mac, Studio ships the server inside its app and writes this folder once it is turned on
        let exe = PathBuf::from("/Applications/RobloxStudio.app/Contents/MacOS/StudioMCP");
        return Some(exe).filter(|p| p.exists() && home_dir().join("Library").join("Roblox").join("StudioMCP").exists());
    }
    #[allow(unreachable_code)]
    Some(PathBuf::from(env::var("LOCALAPPDATA").unwrap_or_default()).join("Roblox").join("mcp.bat")).filter(|p| p.exists())
}

/// Whether a Blender path from the settings file may be run: never one inside a folder an AI's file tools are given
/// (the work folder, the kit, the app's home with its games, a game), where a model could have put a script of its own.
pub fn blender_ok(p: &Path) -> bool {
    let p = real_as_far_as_exists(p);
    let mut roots = vec![PATHS.work.clone(), PATHS.kit.clone(), PATHS.home.clone()];
    roots.extend(setting("projects").as_array().into_iter().flatten().filter_map(|e| e["path"].as_str()).map(PathBuf::from));
    !roots.iter().any(|r| p.starts_with(real_as_far_as_exists(r)))
}

/// `p` with its longest existing start made real (links and `/var` → `/private/var` followed) and the rest kept, so a
/// path that does not exist yet compares the same way as one that does.
fn real_as_far_as_exists(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p.to_path_buf();
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |acc: PathBuf, c| acc.join(c));
        }
        match (at.file_name().map(|c| c.to_os_string()), at.parent()) {
            (Some(c), Some(up)) => {
                rest.push(c);
                at = up.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

pub fn find_blender() -> Option<PathBuf> {
    if let Some(p) = env::var("RBXKIT_BLENDER").ok().map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    if let Some(p) = load_config().get("blender").and_then(Value::as_str).map(PathBuf::from).filter(|p| p.exists() && blender_ok(p)) {
        return Some(p);
    }
    if let Some(p) = which("blender") {
        return Some(p);
    }
    #[cfg(target_os = "macos")]
    for app in [PathBuf::from("/Applications/Blender.app"), home_dir().join("Applications").join("Blender.app")] {
        let exe = app.join("Contents").join("MacOS").join("Blender");
        if exe.exists() {
            return Some(exe);
        }
    }
    let roots = [
        PathBuf::from(r"C:\Program Files\Blender Foundation"),
        PathBuf::from(env::var("LOCALAPPDATA").unwrap_or_default()).join("Programs"),
    ];
    for root in roots {
        let mut hits: Vec<PathBuf> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("Blender"))
            .map(|e| e.path().join("blender.exe"))
            .filter(|p| p.exists())
            .collect();
        hits.sort();
        if let Some(p) = hits.pop() {
            return Some(p);
        }
    }
    None
}

pub fn gemini_key() -> String {
    for k in ["GEMINI_API_KEY", "GOOGLE_API_KEY"] {
        if let Ok(v) = env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    setting("gemini_key").as_str().unwrap_or("").to_string()
}

/// The programs the user lets agents run on their own (git, python, ...), chosen in the app.
pub fn allowed_commands() -> Vec<String> {
    setting("allowed_commands").as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// Held by the tests that change the allowed list, so they never run into each other (they share one settings file).
#[cfg(test)]
pub static ALLOWED_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Adds or removes programs from that list. Names are single words: a program, never a whole command line.
pub fn set_allowed(names: &[String], allow: bool) -> Vec<String> {
    update_config(|cfg| {
        let mut list: Vec<String> = cfg.get("allowed_commands").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        for name in names.iter().filter_map(|n| program_name(n)) {
            list.retain(|n| *n != name);
            if allow {
                list.push(name);
            }
        }
        list.sort();
        cfg.insert("allowed_commands".into(), serde_json::json!(list));
        list
    })
}

/// The program a command line starts with ("git" for `git merge main`), when it is a plain name.
pub fn program_name(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?.trim_matches(['"', '\'', '&']);
    let name = base_name(first).to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name).to_string();
    (!name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))).then_some(name)
}

/// Whether the app itself may approve this shell command because the user allowed its program(s). A whitelist, written
/// for the grammar of `/bin/sh` (the only shell such a command is run with: OpenCode's `fridae-shell`), never for zsh,
/// bash or fish: every part must start with a program the user allowed, named exactly (no folder part like `./git`, no
/// quotes, no variable set in front, same spelling), and outside quotes a word may hold only `A-Z a-z 0-9 _ @ % + = :
/// , . / - !`, and `~` or `^` right after a letter or digit (`HEAD~1`, `HEAD^`: `/bin/sh` and dash expand `~` only at
/// a word's start or after `=` or `:`, `^` never, and `!` only negates a pipeline as a word of its own, which is no
/// program anyone allows; shown in real `/bin/sh` and dash, see DEVELOPING) (so no `( ) { } # * ? [ ]`, no glob, no zsh
/// qualifier or `=( )`, no fish substitution) and may not start with `=`. Inside quotes anything but a line break.
/// Nowhere `$`, a backtick, a backslash or a line break. A redirection only as a whole word: `2>&1`, `>/dev/null`, `2>/dev/null` (or `>`/`2>`
/// then the word `/dev/null`); any other `<` or `>` refuses. Parts joined by `;`, `&&`, `||` or `|` must each pass; a
/// lone `&`, `|&` or an empty part refuses. Nor does anything that reads another process's environment pass
/// (`reads_other_env`), nor a part whose program runs what it is given (`ARG_RUNNERS`). When unsure, no: the card
/// shows, with the reason (`refusal_reason`). The native allow lists of Claude Code (`Bash(<p>:*)`)
/// and Cursor (`Shell(<p>)`) decide for those AIs; this is for the AIs whose requests the app answers (OpenCode).
#[allow(unused_assignments)] // the last word's end leaves its flags unread
pub fn command_allowed(command: &str, allowed: &[String]) -> bool {
    let c = command.trim_matches([' ', '\t']);
    if c.is_empty() || c.contains(['`', '\n', '\r', '$', '\\']) || reads_other_env(c) {
        return false;
    }
    let plain = |ch: char| ch.is_ascii_alphanumeric() || "_@%+=:,./-!".contains(ch);
    let chars: Vec<char> = c.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    // where a word ends: a blank, a separator, or the end
    let ends = |i: usize| at(i).is_none_or(|ch| " \t;|&".contains(ch));
    let starts = |i: usize, s: &str| s.chars().enumerate().all(|(k, ch)| at(i + k) == Some(ch));
    // the parts, each a list of words (a word as written, quotes kept; whether any of it was quoted)
    let mut parts: Vec<Vec<(String, bool)>> = vec![Vec::new()];
    let mut word = String::new();
    let mut quoted = false;
    let mut in_word = false;
    let mut i = 0;
    macro_rules! end_word {
        () => {
            if in_word {
                parts.last_mut().unwrap().push((std::mem::take(&mut word), quoted));
                in_word = false;
                quoted = false;
            }
        };
    }
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            ' ' | '\t' => {
                end_word!();
                i += 1;
            }
            ';' | '|' | '&' => {
                end_word!();
                i += match (ch, at(i + 1)) {
                    ('&', Some('&')) | ('|', Some('|')) => 2,
                    ('&', _) | ('|', Some('&')) => return false,
                    _ => 1,
                };
                parts.push(Vec::new());
            }
            '<' | '>' | '2' if !in_word && (ch != '2' || at(i + 1) == Some('>')) => {
                // a redirection: only the harmless ones, each a whole word
                if let Some(r) = ["2>&1", "2>/dev/null", ">/dev/null"].into_iter().find(|r| starts(i, r) && ends(i + r.chars().count())) {
                    i += r.chars().count();
                    continue;
                }
                let op = if starts(i, "2>") { 2 } else if starts(i, ">") { 1 } else { return false };
                let mut j = i + op;
                if !matches!(at(j), Some(' ' | '\t')) {
                    return false;
                }
                while matches!(at(j), Some(' ' | '\t')) {
                    j += 1;
                }
                if !(starts(j, "/dev/null") && ends(j + 9)) {
                    return false;
                }
                i = j + 9;
            }
            '\'' | '"' => {
                let Some(len) = chars[i + 1..].iter().position(|&x| x == ch) else { return false };
                word.extend(&chars[i..i + len + 2]);
                quoted = true;
                in_word = true;
                i += len + 2;
            }
            _ => {
                let after_letter = i > 0 && in_word && chars[i - 1].is_ascii_alphanumeric();
                if !(plain(ch) || (matches!(ch, '~' | '^') && after_letter)) || (!in_word && ch == '=') {
                    return false;
                }
                word.push(ch);
                in_word = true;
                i += 1;
            }
        }
    }
    end_word!();
    parts.iter().all(|words| {
        let Some((first, quoted)) = words.first() else { return false };
        let plain_words: Vec<String> = words.iter().map(|(w, _)| w.replace(['\'', '"'], "")).collect();
        !quoted
            && first.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && !first.starts_with('.')
            && allowed.iter().any(|a| a == first)
            && runner_in(&plain_words).is_none()
    })
}

/// Programs that run what they are given as a command: the app never approves a part they lead by itself, however
/// the user's list reads, because what they would run is not what the list was asked about (`time ps eww`). `Always`:
/// whatever follows; `Find`: with `-exec`, `-execdir`, `-ok`, `-okdir`; `Git`: only in git's command-running forms
/// (`git_runs`). The one place they are listed: the check, the reason on the card (`refusal_reason`) and the
/// environment-reader check of what they run (`reads_other_env`) read it.
#[derive(Clone, Copy, PartialEq)]
pub enum Runs {
    Always,
    Find,
    Git,
}

pub const ARG_RUNNERS: &[(&str, Runs)] = {
    use Runs::*;
    &[
        ("time", Always), ("nice", Always), ("nohup", Always), ("xargs", Always), ("env", Always), ("command", Always), ("exec", Always),
        ("builtin", Always), ("eval", Always), ("timeout", Always), ("gtimeout", Always), ("stdbuf", Always), ("sudo", Always), ("doas", Always),
        ("su", Always), ("caffeinate", Always), ("watch", Always), ("script", Always), ("unbuffer", Always), ("setsid", Always), ("flock", Always),
        ("ionice", Always), ("taskset", Always), ("chrt", Always), ("chroot", Always), ("nsenter", Always), ("unshare", Always), ("strace", Always),
        ("ltrace", Always), ("parallel", Always), ("entr", Always), ("arch", Always), ("systemd-run", Always), ("busybox", Always), ("wsl", Always),
        ("runas", Always), ("start", Always), ("sh", Always), ("bash", Always), ("zsh", Always), ("dash", Always), ("ksh", Always), ("fish", Always),
        ("xcrun", Always), ("sandbox-exec", Always), ("open", Always), ("osascript", Always), ("tmux", Always), ("screen", Always), ("ssh", Always),
        ("expect", Always),
        ("find", Find),
        ("git", Git),
    ]
};

/// Options of a runner that take the next word as their value (so it is not the program run).
fn value_options(runner: &str) -> &'static [&'static str] {
    match runner {
        "nice" => &["-n"],
        "xargs" => &["-I", "-L", "-n", "-P", "-s", "-E", "-d", "-a"],
        "timeout" | "gtimeout" => &["-s", "-k"],
        "env" => &["-u", "-C"],
        "sudo" | "doas" => &["-u", "-g", "-C", "-D", "-h", "-p", "-U", "-r", "-t"],
        "watch" => &["-n", "-d"],
        "caffeinate" => &["-t", "-w"],
        "ionice" => &["-c", "-n"],
        "xcrun" => &["--sdk", "--toolchain"],
        "sandbox-exec" => &["-f", "-n", "-p", "-D"],
        _ => &[],
    }
}

/// When a git part is in one of git's command-running forms: the words it would run (or set as something to run).
/// Precisely: `-c`, `--config-env`, `--exec-path=` before the subcommand; `rebase -x/--exec`; `bisect run`;
/// `submodule foreach`; `filter-branch`; `difftool -x/--extcmd`; `grep -O`; `clone -c/--config/-u/--upload-pack/
/// --template`; `fetch/pull/push/ls-remote/archive --upload-pack/--receive-pack/--exec`; `config` setting a value
/// (an alias that starts with `!` runs a command). Everyday forms (`switch -c`, `cherry-pick -x`, `clean -x`,
/// `log -c`, the word `run` as an argument) are not.
pub fn git_runs(words: &[String]) -> Option<Vec<String>> {
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    let mut payload: Vec<String> = Vec::new();
    let mut runs = false;
    let mut i = 1;
    while i < words.len() && w(i).starts_with('-') {
        let o = w(i);
        if o == "-c" || o == "--config-env" {
            runs = true;
            payload.push(w(i + 1).to_string());
            i += 2;
            continue;
        }
        if (o.starts_with("-c") && o.len() > 2) || o.starts_with("--config-env=") || o.starts_with("--exec-path=") {
            runs = true;
            payload.push(o.splitn(2, ['=', 'c']).nth(1).unwrap_or("").to_string());
        }
        i += if ["-C", "--git-dir", "--work-tree", "--namespace", "--super-prefix"].contains(&o) { 2 } else { 1 };
    }
    let sub = w(i);
    let rest: Vec<&str> = words.iter().skip(i + 1).map(String::as_str).collect();
    // the word after an option, or its value after `=`
    let mut takes = |names: &[&str], short_joined: bool| {
        for (k, a) in rest.iter().enumerate() {
            for n in names {
                if a == n {
                    runs = true;
                    payload.push(rest.get(k + 1).copied().unwrap_or("").to_string());
                } else if n.starts_with("--") && a.starts_with(&format!("{n}=")) {
                    runs = true;
                    payload.push(a[n.len() + 1..].to_string());
                } else if short_joined && n.len() == 2 && !n.starts_with("--") && a.starts_with(n) && a.len() > 2 && !a.starts_with("--") {
                    runs = true;
                    payload.push(a[2..].to_string());
                }
            }
        }
    };
    match sub {
        "rebase" => takes(&["-x", "--exec"], true),
        "difftool" => takes(&["-x", "--extcmd"], true),
        "grep" => {
            takes(&["--open-files-in-pager"], false);
            if rest.iter().any(|a| a.starts_with("-O")) {
                runs = true;
                payload.extend(rest.iter().filter(|a| a.starts_with("-O")).map(|a| a[2..].to_string()));
            }
        }
        "clone" => takes(&["-c", "--config", "-u", "--upload-pack", "--template"], true),
        "fetch" | "pull" | "push" | "ls-remote" | "archive" | "remote" => takes(&["--upload-pack", "--receive-pack", "--exec"], false),
        "bisect" if rest.first() == Some(&"run") => {
            runs = true;
            payload.extend(rest[1..].iter().map(|a| a.to_string()));
        }
        "submodule" => {
            if let Some(k) = rest.iter().position(|a| *a == "foreach") {
                runs = true;
                payload.extend(rest[k + 1..].iter().map(|a| a.to_string()));
            }
        }
        "filter-branch" => {
            runs = true;
            payload.extend(rest.iter().map(|a| a.to_string()));
        }
        "config" => {
            let positional = rest.iter().filter(|a| !a.starts_with('-')).count();
            let reads = rest.iter().any(|a| matches!(*a, "--get" | "--get-all" | "--get-regexp" | "-l" | "--list" | "--unset" | "--unset-all" | "--show-origin" | "--name-only"));
            let edits = rest.iter().any(|a| matches!(*a, "--add" | "--replace-all" | "-e" | "--edit"));
            if edits || (positional >= 2 && !reads) {
                runs = true;
                payload.extend(rest.iter().map(|a| a.to_string()));
            }
        }
        _ => {}
    }
    runs.then_some(payload)
}

/// The runner leading a part (its words, quotes removed), when it runs something given to it here, and the words it
/// would run.
fn runner_in(words: &[String]) -> Option<(&'static str, Vec<String>)> {
    let first = words.first().map(|w| program_name(w).unwrap_or_default())?;
    let (name, runs) = ARG_RUNNERS.iter().find(|(n, _)| *n == first)?;
    match runs {
        Runs::Git => git_runs(words).map(|p| (*name, p)),
        Runs::Find => {
            let at: Vec<usize> = words.iter().enumerate().filter(|(_, w)| matches!(w.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")).map(|(k, _)| k).collect();
            (!at.is_empty()).then(|| (*name, at.iter().flat_map(|k| words[k + 1..].iter().take_while(|w| !matches!(w.as_str(), ";" | "+" | r"\;")).cloned()).collect()))
        }
        Runs::Always => {
            // the program it runs: past its options (and their values), numbers and durations, and `NAME=value`; a
            // shell's or su's `-c` script
            let shell = matches!(*name, "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "su");
            let mut k = 1;
            while k < words.len() {
                let a = words[k].as_str();
                if shell && a.starts_with('-') && !a.starts_with("--") && a.ends_with('c') {
                    return Some((*name, words.get(k + 1).map(|s| s.split_whitespace().map(str::to_string).collect()).unwrap_or_default()));
                }
                let number = !a.is_empty() && a.trim_end_matches(['s', 'm', 'h', 'd']).chars().all(|c| c.is_ascii_digit() || c == '.');
                if a.starts_with('-') {
                    k += if value_options(name).contains(&a) { 2 } else { 1 };
                } else if number || (a.contains('=') && !a.starts_with('=')) {
                    k += 1;
                } else {
                    break;
                }
            }
            let mut inner: Vec<String> = words.get(k).map(|s| s.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
            inner.extend(words.iter().skip(k + 1).cloned());
            Some((*name, inner))
        }
    }
}

/// Whether what a runner runs reads other programs' settings: an Always runner's program (and a runner it runs, in
/// turn); for git and find, any name in what they would run.
fn runs_env_reader(words: &[String], depth: usize) -> bool {
    let Some((name, payload)) = runner_in(words) else { return false };
    let reader = |t: &str| ENV_READERS.contains(&base_name(t).to_lowercase().as_str());
    if matches!(ARG_RUNNERS.iter().find(|(n, _)| *n == name).map(|r| r.1), Some(Runs::Always)) {
        let Some(first) = payload.first() else { return false };
        reader(first) || (depth < 8 && runs_env_reader(&payload, depth + 1))
    } else {
        payload.iter().flat_map(|w| w.split(|c: char| !(c.is_ascii_alphanumeric() || "-_./".contains(c))).map(str::to_string).collect::<Vec<_>>()).any(|t| reader(&t))
            || (depth < 8 && runs_env_reader(&payload.iter().flat_map(|w| w.split_whitespace().map(str::to_string).collect::<Vec<_>>()).collect::<Vec<_>>(), depth + 1))
    }
}

/// The words of each part of a command as `/bin/sh` would mostly split them (quotes removed; for the checks that
/// look for something, not for approving).
fn rough_parts(command: &str) -> Vec<Vec<String>> {
    command
        .replace("2>&1", " ")
        .split([';', '|', '&', '\n'])
        .map(|p| p.split_whitespace().map(|w| w.replace(['\'', '"'], "")).collect::<Vec<String>>())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Why the app did not approve a command by itself, in plain words for the card.
pub fn refusal_reason(command: &str, allowed: &[String]) -> String {
    if reads_other_env(command) {
        return "it would run a program that can read other programs' settings (such as ps), where the password of OpenCode's server is kept. That never runs in a Fridae chat.".into();
    }
    if command.contains(['$', '`']) {
        return "it uses $ or backquotes (a variable, or a command inside the command), so what it runs cannot be known before it runs.".into();
    }
    if command.contains('\\') {
        return "it has a backslash, which can change what the shell runs.".into();
    }
    if command.contains(['\n', '\r']) {
        return "it is more than one line.".into();
    }
    for words in rough_parts(command) {
        if let Some((r, _)) = runner_in(&words) {
            return format!("{r} runs another program given to it{}, so it is asked about every time.", if words[0] == "git" || words[0] == "find" { " in this form" } else { "" });
        }
        let first = &words[0];
        if first.contains(['/', '\\']) || first.starts_with('.') {
            return format!("it calls a program by a path ({first}) instead of its plain name, so it may not be the program that was allowed.");
        }
        if first.contains('=') {
            return "it sets a setting in front of the program, which can change what the program does.".into();
        }
    }
    if command.contains(['<', '>']) {
        return "it reads or writes a file with < or >.".into();
    }
    let odd: String = command.chars().filter(|c| "*?[](){}#~^".contains(*c)).fold(String::new(), |mut a, c| {
        if !a.contains(c) {
            a.push(c);
        }
        a
    });
    if !odd.is_empty() {
        return format!("it uses characters the shell gives a meaning of its own ({odd}).");
    }
    if command_programs(command).iter().all(|p| allowed.contains(p)) {
        return "Fridae could not check how it is written.".into();
    }
    "it runs a program you have not allowed.".into()
}

/// Programs that can read another process's environment or memory (the OpenCode server's password travels in its
/// own), on macOS, Linux or Windows: the app never approves a command that runs one of these, however the user's list
/// reads, also behind a runner (`time ps eww`, `git -c core.pager=ps`), nor one that names Linux's `/proc` folder
/// (where any file reader can read `/proc/<pid>/environ`). The one place they are listed.
pub const ENV_READERS: &[&str] = &[
    "ps", "procs", "htop", "top", "pargs", "gdb", "lldb", "dtrace", "dtruss", "launchctl", "procdump", "procexp", "procexp64", "handle", "wmic", "gcore",
    "strace", "ltrace", "stringdups", "vmmap", "heap", "leaks", "sample", "spindump",
];

/// Whether a command runs one of the ENV_READERS (as its program, or as what a runner runs) or names the
/// `/proc` folder: `/proc` at the start of a path, after only `.` and `..` parts (`../../proc`), or a leading `proc/`
/// then a number or `self` (`cd /` first); never the bare word `proc`, `proc/gen.lua` or `src/procgen.lua`.
pub fn reads_other_env(command: &str) -> bool {
    let names_proc = command
        .split(|c: char| c.is_whitespace() || "'\"=:,;|&<>(){}".contains(c))
        .any(|word| {
            let comps: Vec<&str> = word.split('/').collect();
            comps.iter().enumerate().any(|(i, c)| {
                *c == "proc"
                    && if i == 0 {
                        // `proc/self`, `proc/1` after `cd /`; the bare word, or `proc/gen.lua`, is not the folder
                        comps.get(1).is_some_and(|n| n == &"self" || n == &"thread-self" || (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
                    } else {
                        comps[..i].iter().all(|p| matches!(*p, "" | "." | ".."))
                    }
            })
        });
    names_proc || command_programs(command).iter().any(|p| ENV_READERS.contains(&p.as_str())) || rough_parts(command).iter().any(|words| runs_env_reader(words, 0))
}

/// The programs a command line starts, by name (each part after `;`, `&&`, `||`, `|`), for the blocked card.
pub fn command_programs(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // `2>&1` is a redirection, not a separator: its `&` must not start a part (`git log 2>&1/x` names no program `x`)
    let command = command.replace("2>&1", " ");
    for part in command.split([';', '|', '&', '\n']) {
        if let Some(n) = program_name(part.trim()).filter(|n| !out.contains(n)) {
            out.push(n);
        }
    }
    out
}

/// The paid API is only used when a key exists AND the user switched it on.
pub fn gemini_api_on() -> bool {
    !gemini_key().is_empty() && setting("gemini_use_api").as_bool().unwrap_or(false)
}

/// A file name with any folder part removed, so requests cannot reach outside a folder.
pub fn base_name(s: &str) -> String {
    s.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Random hex, 16 characters per round. RandomState is seeded by the operating system.
pub fn random_hex(rounds: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..rounds).map(|_| format!("{:016x}", std::collections::hash_map::RandomState::new().build_hasher().finish())).collect()
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Local time as YYYYMMDD-HHMMSS, for file names.
pub fn stamp() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            y: u16,
            mo: u16,
            dow: u16,
            d: u16,
            h: u16,
            mi: u16,
            s: u16,
            ms: u16,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocalTime(t: *mut SystemTime);
        }
        let mut t = SystemTime::default();
        unsafe { GetLocalTime(&mut t) };
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.y, t.mo, t.d, t.h, t.mi, t.s)
    }
    #[cfg(not(windows))]
    {
        // struct tm: nine ints, then (macOS, Linux) the offset and the zone name; room to spare
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
            rest: [u64; 4],
        }
        unsafe extern "C" {
            fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
        }
        let t = now_secs() as i64;
        let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, rest: [0; 4] };
        if unsafe { localtime_r(&t, &mut tm) }.is_null() {
            return format!("{t}");
        }
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.year + 1900, tm.mon + 1, tm.mday, tm.hour, tm.min, tm.sec)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_saved_chatgpt_default_reads_as_claude() {
        assert_eq!(super::ai_choice(Some("chatgpt")), "claude");
        assert_eq!(super::ai_choice(Some("gemini")), "gemini");
        assert_eq!(super::ai_choice(Some("antigravity")), "gemini");
        assert_eq!(super::ai_choice(None), "claude");
        // Cursor is not offered on Windows yet: a saved choice of it reads as Claude there
        assert_eq!(super::ai_choice(Some("cursor")), if cfg!(windows) { "claude" } else { "cursor" });
        assert_eq!(super::AIS.contains(&"cursor"), !cfg!(windows));
        assert_eq!(super::find_cursor().is_none() || !cfg!(windows), true);
        // OpenCode likewise
        assert_eq!(super::ai_choice(Some("opencode")), if cfg!(windows) { "claude" } else { "opencode" });
        assert_eq!(super::AIS.contains(&"opencode"), !cfg!(windows));
        assert!(super::find_opencode().is_none() || !cfg!(windows));
    }

    #[test]
    fn only_an_exactly_named_allowed_program_is_approved() {
        let allowed = vec!["git".to_string(), "ls".to_string(), "python3".to_string()];
        for ok in ["git status", "git", "ls -la", "git log --oneline | ls", "git add . && git commit -m 'a; b'", "ls 2>&1", "ls >/dev/null", "git status; ls"] {
            assert!(command_allowed(ok, &allowed), "{ok}");
        }
        for bad in [
            "./git status", "bin/git", "/tmp/x/python3 -c 1", "/usr/bin/git", "GIT status", "gitk", "git; curl x", "git && ./x",
            "FOO=1 git", "\"git\" status", "'git'", "g\\it", "git $(curl x)", "git `id`", "git > /etc/x", "ls < /etc/passwd",
            "git status &", "git\nrm -rf x", "git status | sh", "ls ${HOME}", "", "git 'unclosed", ".git",
            // zsh glob qualifiers and =( ), fish's ( ): the shell is /bin/sh, and none of these pass the whitelist
            "git status *(e:'./git':)", "git status *(e:./git:)", "git status >/dev/null(e:'./git':)", "git diff =(./git)", "git log (./git)",
            "git log {a,b}", "git log ~/x", "git log =x", "git log x=~", "git log a:~/x", "git log ~", "git log /~", "git log ^x", "ls *.lua", "ls x?", "ls [ab]", "git log # x",
            // redirections only as whole words
            "git log 2>&1/../../outside.txt", "python3 2>&1/x", "git log 2>&1x", "git x>/dev/null", "ls >/dev/nullx", "ls >>/dev/null",
            "ls 1>/dev/null", "ls > /etc/x", "ls 2> x", "ls >", "git status |& ls", "git status;", "; git", "git status || ",
            // never a program that reads another process's environment, nor /proc
            "ps eww -ax", "git log | ps -E", "ls /proc/1/environ", "launchctl procinfo 1", "ls /proc", "ls proc/self/environ",
            "ls ../../proc/1/environ", "ls ./proc/1", "ls x=/proc/1", "ls 'a b' /proc/",
            // runners: what they run is not what the list was asked about, even when they are on it
            "time ps eww", "time git status", "nice git status", "nohup git status", "xargs git", "env git status", "command git", "exec git",
            "git -c alias.x='!ps eww' x", "git -c core.pager=ps log", "find . -exec rm {} ;", "git status && time ls",
            // look-alikes
            "git\u{a0}status", "\u{ff47}it status",
        ] {
            assert!(!command_allowed(bad, &allowed), "{bad}");
        }
        // python2 is not python
        assert!(!command_allowed("python2>&1 x.py", &["python".to_string()]));
        assert!(!command_allowed("git status", &["gitk".to_string()]));
        // ps is refused even when the user allowed it, and so is a runner the user allowed, and an env reader behind one
        assert!(!command_allowed("ps -ax", &["ps".to_string()]));
        let runners: Vec<String> = ["time", "nice", "nohup", "xargs", "env", "command", "exec", "ps", "git", "find"].map(String::from).to_vec();
        for c in ["time ps eww", "nice ps", "nohup ps", "xargs ps", "env ps", "command ps", "exec ps", "time /bin/ps eww", "git -c core.pager=ps log", "git -c alias.x='!ps eww' x", "find . -exec ps ;"] {
            assert!(reads_other_env(c), "{c}");
            assert!(!command_allowed(c, &runners), "{c}");
        }
        for c in ["time git status", "env FOO=1 git", "xargs git add", "find . -execdir rm {} ;", "git -c x=y log"] {
            assert!(!command_allowed(c, &runners), "{c}");
            assert!(refusal_reason(c, &runners).contains("runs another program"), "{c}: {}", refusal_reason(c, &runners));
        }
        // a name holding the letters is not /proc, and git's own use of words is not an env reader
        for c in ["git add src/procgen.lua", "git add proceed.txt procs.md/x", "git commit -m \"fix ps output\"", "find . -name x", "git log --grep=top"] {
            assert!(!reads_other_env(c), "{c}");
            assert!(command_allowed(c, &runners), "{c}");
        }
        // the bare word proc is no environment reader; only the folder's path is
        for c in ["git commit -m \"tune proc rate\"", "git checkout proc", "grep proc notes.md", "git add proc/gen.lua", "ls src/proc/x"] {
            assert!(!reads_other_env(c), "{c}");
        }
        for c in ["cat /proc/1/environ", "cat //proc/self/environ", "cat proc/self/environ", "cat proc/42/environ", "cat ../../proc/1/environ", "cat ./proc/1"] {
            assert!(reads_other_env(c), "{c}");
        }
        // git's command-running forms, precisely, both ways
        let git = vec!["git".to_string()];
        for c in ["git switch -c feature", "git checkout -b x", "git branch -c a b", "git cherry-pick -x abc", "git clean -xdf", "git grep -c x", "git log -c",
                  "git checkout run", "git commit -m run", "git add foreach", "git config --get user.name", "git config user.name", "git --exec-path", "git -C sub status"] {
            assert!(command_allowed(c, &git), "{c}");
        }
        for c in ["git -c core.pager=less log", "git -c alias.x=y x", "git --config-env=a=B log", "git --exec-path=/tmp/x log", "git rebase -x make", "git rebase --exec=make",
                  "git rebase -i --exec make main", "git bisect run make", "git submodule foreach ls", "git submodule --quiet foreach ls", "git filter-branch --tree-filter x",
                  "git difftool -x meld", "git grep -Ofoo x", "git clone --config core.sshCommand=x url", "git clone -u x url", "git fetch --upload-pack=x origin",
                  "git push --receive-pack=x origin", "git config alias.x '!ls'", "git config --add core.pager less", "git -C sub -c a=b log"] {
            assert!(!command_allowed(c, &git), "{c}");
            assert!(refusal_reason(c, &git).contains("runs another program"), "{c}");
        }
        // a reader's name as an argument of what a runner runs is not running the reader
        for c in ["xargs grep sample", "timeout 5 git checkout top", "time git log --grep=ps", "nice -n 5 make heap", "env A=1 make leaks"] {
            assert!(!reads_other_env(c), "{c}");
        }
        for c in ["timeout 5s ps", "nice -n 5 ps", "env A=1 B=2 ps", "time nice ps", "sh -c 'ps eww'", "bash -lc 'top -l 1'", "xargs -I{} ps", "sudo -u root leaks 1", "git rebase -x 'ps eww'", "find . -exec ps ;"] {
            assert!(reads_other_env(c), "{c}");
        }
        // every refusal has a reason in plain words
        for (c, has) in [("./git status", "path"), ("git $(id)", "$"), ("ls > x", "file"), ("ls *.lua", "*"), ("ps", "password"), ("FOO=1 git", "setting")] {
            assert!(refusal_reason(c, &runners).contains(has), "{c}: {}", refusal_reason(c, &runners));
        }
        assert_eq!(command_programs("git log 2>&1/../../x.txt"), ["git"]);

        assert_eq!(command_programs("git status && ./rm x; ls | wc"), ["git", "rm", "ls", "wc"]);
        for ok in ["ls 2>/dev/null", "ls > /dev/null", "ls 2> /dev/null && git status", "git commit -m \"a (b) {c} ~d *\"", "git log --format=%H:%an,x@y+z", "git log -- 'a ~ * ( ) ! \"'", "ls 2",
            // ~ and ^ after a letter or digit, and !, are taken literally by /bin/sh and dash
            "git diff HEAD~1", "git show HEAD^", "git commit -m \"Fixed it!\"", "git log HEAD~2^..HEAD", "git log !x", "git log \"!x\"", "git commit -m Done!"] {
            assert!(command_allowed(ok, &allowed), "{ok}");
        }
    }

    use super::*;

    /// A home of its own for a settings test: its folder, settings file and copy (in the tests' guard folder).
    fn home_store(name: &str) -> Store {
        let home = test_dir(&format!("home-{name}"));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let copy = guard_copy_for(&home);
        let _ = fs::remove_file(&copy);
        Store { config: home.join("config.json"), copy, home }
    }

    fn file_of(st: &Store) -> Map<String, Value> {
        load_config_at(&st.config)
    }

    fn put(st: &Store, v: Value) {
        write_config(&st.config, v.as_object().unwrap());
    }

    #[test]
    fn a_restart_puts_a_changed_file_back_and_says_what_changed_and_a_route_change_survives() {
        let st = home_store("restart");
        put(&st, serde_json::json!({"allowed_commands": ["git"], "projects": [{"name": "Obby", "path": "/g/obby"}], "chat_ai": "claude"}));
        // a first run with no copy takes the file as it is
        let mut g = start_in(&st);
        assert!(g.at_start.is_none() && g.known.get("allowed_commands") == Some(&serde_json::json!(["git"])));
        // a route change survives a restart
        update_in(&mut g, &st, |c| c.insert("use_env_keys".into(), serde_json::json!(true)));
        let g2 = start_in(&st);
        assert!(g2.at_start.is_none() && g2.known.get("use_env_keys") == Some(&serde_json::json!(true)));
        // changed while the app was closed: put back, and each setting named with what changed (a secret never shown)
        let mut f = file_of(&st);
        f.insert("allowed_commands".into(), serde_json::json!(["git", "sh"]));
        f.insert("gemini_key".into(), serde_json::json!("SECRETVALUE"));
        f.remove("projects");
        write_config(&st.config, &f);
        let g3 = start_in(&st);
        let note = g3.at_start.clone().expect("a note");
        let said = note.changes.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>().join("\n");
        assert!(said.contains("programs agents may run") && said.contains("sh, which was taken out"), "{said}");
        assert!(said.contains("list of games") && said.contains("Obby"), "{said}");
        assert!(said.contains("Gemini API key") && !said.contains("SECRETVALUE"), "{said}");
        let back = file_of(&st);
        assert_eq!(back.get("allowed_commands"), Some(&serde_json::json!(["git"])));
        assert!(back.get("gemini_key").is_none() && back.get("chat_ai") == Some(&serde_json::json!("claude")), "unguarded keys stay");
        // an unreadable file is kept, said so, and the guarded settings written back
        fs::write(&st.config, "{ \"chat_ai\": \"claude\",, }").unwrap();
        let g4 = start_in(&st);
        assert!(g4.at_start.as_ref().is_some_and(|n| n.unreadable));
        assert!(fs::read_dir(&st.home).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("config.json.bad-")));
        assert_eq!(file_of(&st).get("projects"), g4.known.get("projects"));
        let mut g5 = start_in(&st);
        fs::write(&st.config, "not json").unwrap();
        assert_eq!(check_in(&mut g5, &st).first().map(String::as_str), Some("unreadable"));
        assert!(put_back_line(&["unreadable".into(), "projects".into()]).contains("could not be read"));
    }

    #[test]
    fn two_homes_never_see_each_others_copy() {
        let real = home_store("real");
        let dev = home_store("dev");
        assert_ne!(real.copy, dev.copy, "one copy per home");
        put(&real, serde_json::json!({"projects": [{"name": "Obby", "path": "/g/obby"}], "gemini_key": "REALKEY", "plugin_token": "REALTOKEN", "allowed_commands": ["git"]}));
        put(&dev, serde_json::json!({"blender": "/Applications/Blender.app"}));
        let _ = start_in(&real);
        // a dev run on its own home: takes its own file, never the real home's values
        let _ = start_in(&dev);
        let d = fs::read_to_string(&dev.config).unwrap();
        assert!(!d.contains("REALKEY") && !d.contains("REALTOKEN") && !d.contains("Obby"), "{d}");
        // a copy written as {} for another home, lying where the real home's copy goes, is never applied
        let mut v = Map::new();
        v.insert("home".into(), Value::from(real_home(&dev.home)));
        v.insert("version".into(), Value::from(99));
        v.insert("settings".into(), Value::Object(Map::new()));
        write_config(&real.copy, &v);
        let g = start_in(&real);
        assert!(g.at_start.is_none(), "taken as no copy");
        let r = file_of(&real);
        assert_eq!(r.get("gemini_key"), Some(&serde_json::json!("REALKEY")));
        assert!(r.get("projects").is_some() && r.get("plugin_token").is_some() && r.get("allowed_commands").is_some());
        // and its own copy is written again for its own home
        assert_eq!(read_copy(&real).map(|(_, c)| c.get("gemini_key").cloned()), Some(Some(serde_json::json!("REALKEY"))));
    }

    #[test]
    fn two_running_copies_on_one_home_keep_each_others_changes() {
        let st = home_store("two-copies");
        put(&st, serde_json::json!({"projects": [{"name": "A", "path": "/g/a"}], "chat_ai": "claude"}));
        let mut a = start_in(&st);
        let mut b = start_in(&st);
        // A adds a game; B then changes an unrelated guarded setting and an unguarded one
        update_in(&mut a, &st, |c| c.insert("projects".into(), serde_json::json!([{"name": "A", "path": "/g/a"}, {"name": "P", "path": "/g/p"}])));
        update_in(&mut b, &st, |c| {
            c.insert("use_env_keys".into(), serde_json::json!(true));
            c.insert("chat_ai".into(), serde_json::json!("opencode"));
        });
        assert!(check_in(&mut a, &st).is_empty(), "no false alarm in A");
        assert!(check_in(&mut b, &st).is_empty(), "nor in B");
        for (who, g) in [("A", &a), ("B", &b)] {
            assert_eq!(g.known.get("projects").and_then(Value::as_array).map(Vec::len), Some(2), "{who} has both games");
            assert_eq!(g.known.get("use_env_keys"), Some(&serde_json::json!(true)), "{who} has the switch");
        }
        let f = file_of(&st);
        assert_eq!(f.get("projects").and_then(Value::as_array).map(Vec::len), Some(2));
        assert_eq!((f.get("use_env_keys"), f.get("chat_ai")), (Some(&serde_json::json!(true)), Some(&serde_json::json!("opencode"))));
        // a tampered file is still put back in both
        let mut t = file_of(&st);
        t.insert("allowed_commands".into(), serde_json::json!(["sh"]));
        write_config(&st.config, &t);
        assert_eq!(check_in(&mut a, &st), ["allowed_commands"]);
        assert!(check_in(&mut b, &st).is_empty() && b.known.get("allowed_commands").is_none());
    }

    /// The settings file changed as an AI's file tool would change it: written straight, not through the app.
    fn tamper(key: &str, v: Value) {
        let mut cfg = load_config();
        cfg.insert(key.into(), v);
        write_config(&config_file(), &cfg);
    }

    #[test]
    fn guarded_settings_come_from_memory_and_a_tampered_file_is_put_back() {
        let _held = ALLOWED_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = set_allowed(&["zzprobe".to_string()], false);
        let keys_before = env_keys_on();
        // mid-turn: the file says sh and the key switch on; nothing that decides reads it
        tamper("allowed_commands", serde_json::json!(["sh"]));
        tamper("use_env_keys", serde_json::json!(true));
        assert_eq!(allowed_commands(), before, "a second turn's spawn and an OpenCode ask read the known list");
        assert!(!command_allowed("sh -c x", &allowed_commands()));
        assert_eq!(env_keys_on(), keys_before);
        // the check puts the file back (another test's turn may have done so first: either way it is back)
        let _ = check_settings();
        assert_eq!(load_config().get("allowed_commands"), Some(&serde_json::json!(before)));
        // a change through the app's own route is written over a tampered file, never merged with it, and is what
        // the copy outside the AIs' folders holds
        tamper("allowed_commands", serde_json::json!(["sh"]));
        let now = set_allowed(&["zzprobe".to_string()], true);
        assert!(!now.contains(&"sh".to_string()) && now.contains(&"zzprobe".to_string()));
        let copy: Value = serde_json::from_str(&fs::read_to_string(guard_copy()).unwrap()).unwrap();
        assert_eq!(copy["settings"].get("allowed_commands"), Some(&serde_json::json!(now)));
        assert_eq!(copy["home"].as_str(), Some(real_home(&PATHS.home).as_str()), "the copy names its home");
        let _ = set_allowed(&["zzprobe".to_string()], false);
    }

    #[test]
    fn a_blender_inside_a_folder_ais_are_given_is_never_run() {
        assert!(!blender_ok(&PATHS.work.join("games").join("g").join("blender")));
        assert!(!blender_ok(&PATHS.kit.join("blender")));
        assert!(blender_ok(Path::new("/Applications/Blender.app/Contents/MacOS/Blender")));
    }

    #[test]
    fn an_over_long_output_line_is_dropped_not_held() {
        let text = b"short\n0123456789abcdef\nlast";
        let got: Vec<Option<Vec<u8>>> = CappedLines::new(std::io::BufReader::with_capacity(4, &text[..]), 8).collect();
        assert_eq!(got, vec![Some(b"short".to_vec()), None, Some(b"last".to_vec())]);
        let got: Vec<Option<Vec<u8>>> = CappedLines::new(&b"a\n\nb\n"[..], 8).collect();
        assert_eq!(got, vec![Some(b"a".to_vec()), Some(vec![]), Some(b"b".to_vec())]);
    }

    #[test]
    fn an_unreadable_settings_file_is_kept_before_it_is_replaced() {
        let dir = test_dir("bad-config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.json");
        fs::write(&f, "{\"allowed_commands\": [\"git\"],}").unwrap();
        keep_unreadable(&f);
        let kept: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("config.json.bad-")).collect();
        assert_eq!(kept.len(), 1);
        assert!(fs::read_to_string(kept[0].path()).unwrap().contains("git"));
        write_config(&f, &serde_json::from_str("{\"a\": 1}").unwrap());
        keep_unreadable(&f); // readable now: nothing more kept
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn old_test_folders_are_known_by_name_only() {
        assert_eq!(legacy_test_pid("fridae-team-note-123"), Some(123));
        assert_eq!(legacy_test_pid("fridae-test-work-77"), Some(77));
        assert_eq!(legacy_test_pid("fridae-update-webview"), None, "the app's own folder is never touched");
        assert_eq!(legacy_test_pid("fridae-team-note"), None);
        assert_eq!(legacy_test_pid("other-team-12"), None);
    }

    #[test]
    fn a_finished_test_runs_folder_is_cleared_and_a_live_one_kept() {
        let base = test_dir("sweep").join("tests");
        let temp = test_dir("sweep").join("temp");
        let me = std::process::id().to_string();
        // a pid that is surely not running: past the largest pid macOS and Linux hand out by default
        let dead = "4194304";
        for d in [base.join(dead), base.join(&me), temp.join(format!("fridae-team-x-{dead}")), temp.join("fridae-update-webview")] {
            fs::create_dir_all(&d).unwrap();
        }
        sweep_test_dirs(&base, &temp);
        if cfg!(unix) {
            assert!(!base.join(dead).exists());
            assert!(!temp.join(format!("fridae-team-x-{dead}")).exists());
        }
        assert!(base.join(&me).exists(), "this run's own folder stays");
        assert!(temp.join("fridae-update-webview").exists());
        assert!(PATHS.work.starts_with(env::temp_dir().join("fridae-tests").join(&me)));
    }

    #[test]
    fn a_turn_is_stalled_only_after_the_whole_wait_and_never_while_waiting_on_purpose() {
        assert!(!stalled(STALL - std::time::Duration::from_secs(1), false, false));
        assert!(stalled(STALL, false, false));
        assert!(!stalled(STALL * 4, true, false), "a Lead waiting on experts or a texture is not stalled");
        assert!(!stalled(STALL * 2, false, true), "a tool call that is open gets STALL_WORKING");
        assert!(stalled(STALL_WORKING, false, true));
        assert!(waits_long("mcp__rbxkit__rbx_team") && waits_long("rbx_texture") && !waits_long("rbx_teamwork") && !waits_long("Bash"));
    }

    #[test]
    fn only_the_tail_of_a_long_error_output_is_kept() {
        let big = "x".repeat(200_000) + "THE END";
        let tail = drain_tail(big.as_bytes(), 1000);
        assert_eq!(tail.len(), 1000);
        assert!(tail.ends_with("THE END"));
        assert_eq!(drain_tail("  short \n".as_bytes(), TAIL), "short");
        // what was read before the wait ran out is kept, while the reader still waits for more (a holder outside the
        // group keeps the pipe open)
        struct Then(Vec<u8>, std::sync::mpsc::Receiver<()>);
        impl std::io::Read for Then {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    let _ = self.1.recv();
                    return Ok(0);
                }
                let n = self.0.len().min(buf.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0.drain(..n);
                Ok(n)
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let kept = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let k = kept.clone();
        let reader = std::thread::spawn(move || drain_tail_into(Then(b"Error: rate limited\n".to_vec(), rx), TAIL, &k));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while tail_text(&kept).is_empty() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!reader.is_finished());
        assert_eq!(tail_text(&kept), "Error: rate limited");
        tx.send(()).unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn ai_processes_lose_the_parent_claude_session_and_ambient_api_keys() {
        let removed = |ai: &str| {
            let mut c = Command::new("true");
            child_env(&mut c, ai);
            let mut gone: Vec<String> = c.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
            gone.sort();
            let set: Vec<(String, String)> = c.get_envs().filter_map(|(k, v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))).collect();
            (gone, set)
        };
        let (claude, set) = removed("claude");
        for k in CLAUDE_SESSION_VARS.iter().chain(&["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]) {
            assert!(claude.contains(&k.to_string()), "{k}");
        }
        assert!(!claude.contains(&"CLAUDE_CODE_OAUTH_TOKEN".to_string()), "the plan's own token stays");
        assert!(set.is_empty());
        let (gemini, set) = removed("gemini");
        assert!(!gemini.iter().any(|k| k.contains("GEMINI") || k.contains("GOOGLE")), "textures read the Gemini key");
        assert_eq!(set, [("AGY_CLI_DISABLE_AUTO_UPDATE".to_string(), "1".to_string())]);
        assert!(removed("cursor").0.contains(&"CURSOR_API_KEY".to_string()));
        assert!(removed("opencode").0.contains(&"OPENAI_API_KEY".to_string()) && removed("opencode").0.contains(&"GEMINI_API_KEY".to_string()));
    }
}
