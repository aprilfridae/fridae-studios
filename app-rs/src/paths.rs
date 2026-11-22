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
/// background thread (members keep the ID reserved too; an empty group is not signalled). Not on Windows, where Cursor
/// is not offered.
pub fn end_group_soon(group: u32) {
    #[cfg(unix)]
    {
        if group <= 1 || group == std::process::id() {
            return;
        }
        let signal = move |sig: &str| signal_group(group, sig);
        signal("-TERM");
        std::thread::spawn(move || {
            std::thread::sleep(STOP_GRACE);
            if signal("-0") {
                signal("-KILL");
            }
        });
    }
    #[cfg(not(unix))]
    let _ = group;
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
/// being made: those wait on purpose).
pub const STALL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// What the chat is told when a turn was ended for that.
pub fn stall_message() -> String {
    format!("Nothing came from the AI for {} minutes, so Fridae ended this turn. Send your message again to carry on.", STALL.as_secs() / 60)
}

/// Tools of the app's own that wait long on purpose (a Lead waiting on experts, a texture being made): no stall then.
pub fn waits_long(tool: &str) -> bool {
    let t = tool.rsplit("__").next().unwrap_or(tool);
    ["rbx_team", "rbx_texture", "rbx_generate_texture"].iter().any(|w| t == *w || t.ends_with(&format!("_{w}")))
}

/// Whether a turn that has sent nothing for `quiet` is stalled: STALL or more, and not waiting on purpose.
pub fn stalled(quiet: std::time::Duration, waiting_long: bool) -> bool {
    quiet >= STALL && !waiting_long
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
pub fn drain_tail(mut r: impl std::io::Read, cap: usize) -> String {
    let mut kept: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                kept.extend(&buf[..n]);
                let over = kept.len().saturating_sub(cap);
                kept.drain(..over);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&kept.into_iter().collect::<Vec<u8>>()).trim().to_string()
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

/// The environment hygiene every AI's process gets (see CLAUDE_SESSION_VARS, plan_keys); Antigravity is also told not
/// to update itself mid-turn.
pub fn child_env(cmd: &mut Command, ai: &str) {
    for k in CLAUDE_SESSION_VARS.iter().chain(plan_keys(ai)) {
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
/// renamed over it), so a reader never sees half a file and two changes never undo each other.
pub fn update_config<R>(change: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
    let _held = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = load_config();
    let out = change(&mut cfg);
    write_config(&config_file(), &cfg);
    out
}

fn write_config(path: &Path, cfg: &Map<String, Value>) {
    let Some(dir) = path.parent() else { return };
    let _ = fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let tmp = dir.join(format!(".config.json.{}-{}.tmp", std::process::id(), stamp()));
        if fs::write(&tmp, text).is_ok() && fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
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

pub fn find_blender() -> Option<PathBuf> {
    if let Some(p) = env::var("RBXKIT_BLENDER").ok().map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    if let Some(p) = load_config().get("blender").and_then(Value::as_str).map(PathBuf::from).filter(|p| p.exists()) {
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
    load_config().get("gemini_key").and_then(Value::as_str).unwrap_or("").to_string()
}

/// The programs the user lets agents run on their own (git, python, ...), chosen in the app.
pub fn allowed_commands() -> Vec<String> {
    load_config().get("allowed_commands").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

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

/// Whether the app itself may approve this shell command because the user allowed its program(s): only when every
/// part of it starts with a program the user allowed, named exactly (no folder part like `./git` or `/tmp/x/python`,
/// no quotes, no variable set in front, same spelling), and it holds nothing the allow list was not written for:
/// command substitution (backticks, `$(`), variables, a redirection other than `2>&1` or `>/dev/null`, a background `&`
/// or a line break. Parts joined by `;`, `&&`, `||` or `|` must each pass. When unsure, no: the blocked card shows.
/// The native allow lists of Claude Code (`Bash(<p>:*)`) and Cursor (`Shell(<p>)`) decide for those AIs; this is for
/// the AIs whose requests the app answers (OpenCode).
pub fn command_allowed(command: &str, allowed: &[String]) -> bool {
    let c = command.trim();
    if c.is_empty() || c.contains(['`', '\n', '\r', '$', '\\']) {
        return false;
    }
    // the two harmless redirections are taken out; any other `<` or `>` is a write or read the list was not written for
    let c = c.replace("2>&1", " ").replace(">/dev/null", " ").replace("> /dev/null", " ");
    if c.contains(['<', '>']) {
        return false;
    }
    // split into parts at ; && || | (a lone & sends a part to the background: refused)
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let chars: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == ';' || ch == '|' => {
                parts.push(std::mem::take(&mut cur));
                if chars.get(i + 1) == Some(&'|') {
                    i += 1;
                }
                i += 1;
                continue;
            }
            None if ch == '&' => {
                if chars.get(i + 1) != Some(&'&') {
                    return false;
                }
                parts.push(std::mem::take(&mut cur));
                i += 2;
                continue;
            }
            None => {}
        }
        cur.push(ch);
        i += 1;
    }
    if quote.is_some() {
        return false;
    }
    parts.push(cur);
    parts.iter().all(|part| {
        let Some(first) = part.split_whitespace().next() else { return false };
        first.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !first.starts_with('.') && allowed.iter().any(|a| a == first)
    })
}

/// The programs a command line starts, by name (each part after `;`, `&&`, `||`, `|`), for the blocked card.
pub fn command_programs(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in command.split([';', '|', '&', '\n']) {
        if let Some(n) = program_name(part.trim()).filter(|n| !out.contains(n)) {
            out.push(n);
        }
    }
    out
}

/// The paid API is only used when a key exists AND the user switched it on.
pub fn gemini_api_on() -> bool {
    !gemini_key().is_empty() && load_config().get("gemini_use_api").and_then(Value::as_bool).unwrap_or(false)
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
        ] {
            assert!(!command_allowed(bad, &allowed), "{bad}");
        }
        assert!(!command_allowed("git status", &["gitk".to_string()]));
    }

    use super::*;

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
        assert!(!stalled(STALL - std::time::Duration::from_secs(1), false));
        assert!(stalled(STALL, false));
        assert!(!stalled(STALL * 4, true), "a Lead waiting on experts or a texture is not stalled");
        assert!(waits_long("mcp__rbxkit__rbx_team") && waits_long("rbx_texture") && !waits_long("rbx_teamwork") && !waits_long("Bash"));
    }

    #[test]
    fn only_the_tail_of_a_long_error_output_is_kept() {
        let big = "x".repeat(200_000) + "THE END";
        let tail = drain_tail(big.as_bytes(), 1000);
        assert_eq!(tail.len(), 1000);
        assert!(tail.ends_with("THE END"));
        assert_eq!(drain_tail("  short \n".as_bytes(), TAIL), "short");
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
