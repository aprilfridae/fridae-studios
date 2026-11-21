//! Where things live, whether the app runs packaged or from the source folder.
//!
//! KIT   read-only program files (ui/, studio/, skills/, guides/ ...)
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
    let work = if cfg!(test) {
        env::temp_dir().join(format!("fridae-test-work-{}", std::process::id()))
    } else {
        env::var("RBXKIT_WORK").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join("work"))
    };
    Paths { install, kit, home, work, cli_exe }
});

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

// ----------------------------------------------------------------- settings
pub fn load_config() -> Map<String, Value> {
    fs::read_to_string(PATHS.home.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn save_config(cfg: &Map<String, Value>) {
    let _ = fs::create_dir_all(&PATHS.home);
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(PATHS.home.join("config.json"), text);
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

/// The AIs a chat can run on.
pub const AIS: [&str; 3] = ["claude", "gemini", "chatgpt"];

/// The program behind one of them, if it is installed.
pub fn find_ai(kind: &str) -> Option<PathBuf> {
    match kind {
        "gemini" => find_antigravity(),
        "chatgpt" => find_codex(),
        _ => find_agent(),
    }
}

/// The AI new chats start on, chosen in Setup. Claude unless the user picked another.
pub fn default_ai() -> String {
    match load_config().get("chat_ai").and_then(Value::as_str) {
        Some("gemini" | "antigravity") => "gemini",
        Some("chatgpt") => "chatgpt",
        _ => "claude",
    }
    .to_string()
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
    let mut list = allowed_commands();
    for name in names.iter().filter_map(|n| program_name(n)) {
        list.retain(|n| *n != name);
        if allow {
            list.push(name);
        }
    }
    list.sort();
    let mut cfg = load_config();
    cfg.insert("allowed_commands".into(), serde_json::json!(list));
    save_config(&cfg);
    list
}

/// The program a command line starts with ("git" for `git merge main`), when it is a plain name.
pub fn program_name(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?.trim_matches(['"', '\'', '&']);
    let name = base_name(first).to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name).to_string();
    (!name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))).then_some(name)
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
