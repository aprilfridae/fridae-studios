//! The knowledge index (RAG): Setup's Off / Light / Heavy, the processes that serve it, its setup steps, and how
//! the tool servers find it.
//!
//! - The mode is the guarded setting `rag_mode` ("off" | "light" | "heavy", default "off"; `paths::rag_mode`), set
//!   only through the page's route (`set_mode`).
//! - One process serves the index for every expert: `fridae-cli rag serve --mode M --data <work>/rag --port 0
//!   --token-file <work>/rag/token`, which prints `{"listening": port}` once bound and answers on 127.0.0.1 with
//!   `Authorization: Bearer <token>`. Heavy also runs Qdrant from `<work>/rag/bin/qdrant` on two free ports (never
//!   6333), with the token as its API key. Both are started in groups of their own, noted in the process ledger
//!   (`procs`, owners "rag" and "rag-qdrant") and ended when the mode changes and when the app quits. The user's own
//!   Ollama, or a Qdrant the user runs, is never started, stopped or touched here.
//! - The token is 32 random bytes as hex, made anew for each start, written to `<work>/rag/token` (mode 0600) and
//!   handed to the processes by that file's path, never on a command line, and never printed or logged.
//! - While the service has said it is ready, the tool servers get `FRIDAE_RAG_URL` and `FRIDAE_RAG_TOKEN`
//!   (`endpoint`, `agents::mcp_servers_with`); otherwise `rbx_vault` uses its keyword search.
//! - A service that exits by itself is started again, at most every 30 s, while its mode is still the one chosen.
//! - Setup's steps run `fridae-cli rag setup <step> --mode M --data DIR --json-progress` in a process group of their
//!   own, one at a time, the way the promo engine's install runs (`promo.rs`), its lines read by the same parser.

use crate::paths::{self, PATHS, Start};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, LazyLock, Mutex, MutexGuard, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{agents::BUS, procs, promo::EngineLine};

/// The modes, as the setting holds them.
pub const MODES: [&str; 3] = ["off", "light", "heavy"];

/// Setup's steps, as `rag setup` takes them.
pub const SETUP_STEPS: [&str; 4] = ["models", "ollama-install", "ollama-pull", "qdrant-install"];

/// The shelves a test query searches (`rbxvault.SHELVES`).
const SHELVES: [&str; 5] = ["vault", "guides", "team", "game", "yours"];

/// A service that exited by itself is started again no more often than this.
pub const RESTART_EVERY: Duration = Duration::from_secs(30);

/// How long the service may take to say it listens (a packaged Python program starts in a few seconds).
const LISTEN_WAIT: Duration = Duration::from_secs(60);

/// How long Qdrant may take to answer that it is ready.
const QDRANT_WAIT: Duration = Duration::from_secs(30);

/// A setting's value as a mode: anything else is "off".
pub fn mode_of(v: &Value) -> &'static str {
    v.as_str().and_then(|m| MODES.iter().find(|x| **x == m)).copied().unwrap_or("off")
}

/// `<work>/rag`: models, indexes, Qdrant's binary and storage, the token. On `paths::app_protected`.
pub fn dir() -> PathBuf {
    PATHS.work.join("rag")
}

fn token_file() -> PathBuf {
    dir().join("token")
}

fn qdrant_exe() -> PathBuf {
    dir().join("bin").join(if cfg!(windows) { "qdrant.exe" } else { "qdrant" })
}

/// Setup's mode switch (`POST /api/rag/mode`): checked, then written as every guarded setting is (`update_config`).
pub fn set_mode(mode: &str) -> Result<(), String> {
    if !MODES.contains(&mode) {
        return Err("Pick Off, Light or Heavy.".into());
    }
    if mode == "heavy" && cfg!(windows) {
        return Err("Heavy is Mac only for now.".into());
    }
    paths::update_config(|cfg| {
        cfg.insert("rag_mode".into(), json!(mode));
    });
    Ok(())
}

// ------------------------------------------------------------------------------------------- the token

/// 32 random bytes from the system's random source, as hex. None when that cannot be read: nothing is started then
/// (no guessable fallback).
fn new_token() -> Option<String> {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).ok()?;
        Some(paths::hex(&bytes))
    }
    #[cfg(not(unix))]
    {
        // Windows runs Light only; RandomState is seeded by the operating system
        Some(paths::random_hex(4))
    }
}

/// Write the token where only the user can read it (0600 from the moment the file exists), replacing any old one.
fn write_token(path: &Path, token: &str) -> Result<(), String> {
    let why = |e: std::io::Error| format!("Fridae could not write the knowledge index's key file: {e}");
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(why)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut open = fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let written = open.open(&tmp).and_then(|mut f| {
        f.write_all(token.as_bytes())?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|_| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(why(e));
    }
    Ok(())
}

/// Text a process printed, as Setup may show it: the token never appears in it.
fn scrubbed(text: &str, token: &str) -> String {
    if token.is_empty() { text.to_string() } else { text.replace(token, "[key]") }
}

// ------------------------------------------------------------------------------------------- commands

/// `rag serve`: the token by its file's path, never itself; Heavy names Qdrant's address and key file.
fn serve_command(mode: &str, data: &Path, token_file: &Path, qdrant_url: Option<&str>) -> Command {
    let (data, token_file) = (data.display().to_string(), token_file.display().to_string());
    let mut args = vec!["rag", "serve", "--mode", mode, "--data", data.as_str(), "--port", "0", "--token-file", token_file.as_str()];
    if let Some(url) = qdrant_url {
        args.extend(["--qdrant-url", url, "--qdrant-key-file", token_file.as_str()]);
    }
    let mut c = paths::cli(&args);
    // a query never reaches the network (the models were downloaded at setup)
    c.env("HF_HUB_OFFLINE", "1");
    c
}

/// Qdrant (Heavy), configured by its environment: 127.0.0.1 only, on two free ports, the token as its API key, its
/// storage in the app's folder, no telemetry.
fn qdrant_command(exe: &Path, storage: &Path, http: u16, grpc: u16, token: &str) -> Command {
    let mut c = Command::new(exe);
    c.env("QDRANT__SERVICE__HOST", "127.0.0.1")
        .env("QDRANT__SERVICE__HTTP_PORT", http.to_string())
        .env("QDRANT__SERVICE__GRPC_PORT", grpc.to_string())
        .env("QDRANT__SERVICE__API_KEY", token)
        .env("QDRANT__STORAGE__STORAGE_PATH", storage)
        .env("QDRANT__TELEMETRY_DISABLED", "true");
    if let Some(d) = storage.parent() {
        c.current_dir(d); // anything it writes by a relative path lands in the app's folder
    }
    paths::quiet(&mut c);
    c
}

/// A port nothing listens on now.
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| format!("Fridae could not find a free port: {e}"))
}

/// `rag setup <step>` in a process group of its own (stopping it also stops the downloads it started).
fn setup_command(step: &str, mode: &str, data: &Path) -> Command {
    let data = data.display().to_string();
    let mut c = paths::cli(&["rag", "setup", step, "--mode", mode, "--data", &data, "--json-progress"]);
    paths::own_group(&mut c);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

/// All `r` prints, read on a thread of its own (so a full pipe never stalls the process).
fn read_all<R: Read + Send + 'static>(r: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut r) = r {
            let _ = r.read_to_string(&mut s);
        }
        s
    })
}

/// Run a command that prints one JSON object, waiting at most `limit`.
fn run_json(mut cmd: Command, limit: Duration) -> Result<Value, String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::own_group(&mut cmd);
    let mut child = cmd.start().map_err(|e| format!("Could not run the tools: {e}"))?;
    let (out, err) = (read_all(child.stdout.take()), read_all(child.stderr.take()));
    let until = Instant::now() + limit;
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(50)),
            _ => {
                paths::end_trees(&[child.id()]);
                let _ = child.wait();
                return Err("The tools took too long to answer.".into());
            }
        }
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    match serde_json::from_str::<Value>(out.trim()) {
        Ok(v) if v.is_object() && code == Some(0) => Ok(v),
        _ => Err(match crate::promo::error_tail(&err) {
            why if !why.is_empty() => why,
            _ => format!("The tools' answer could not be read (exit {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "without a code".into())),
        }),
    }
}

// ------------------------------------------------------------------------------------------- setup check

/// Whether a mode's setup is complete, from `rag check`'s answer.
fn setup_complete(mode: &str, check: &Value) -> bool {
    match mode {
        "light" => check["models"] == true,
        "heavy" => check["models"] == true && check["ollama"] == "running" && check["ollama_model"] == true && check["qdrant_binary"] == true,
        _ => false,
    }
}

/// `rag check` for a mode, now.
fn check_now(mode: &str) -> Result<Value, String> {
    let data = dir().display().to_string();
    run_json(paths::cli(&["rag", "check", "--mode", mode, "--data", &data, "--json"]), Duration::from_secs(20))
}

/// The page asks for the status every second while something runs: `rag check` is asked at most every 5 s per mode,
/// and again at once after a setup step or a start.
static CHECKS: LazyLock<Mutex<HashMap<String, (Instant, Value)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn check_cached(mode: &str) -> Value {
    if let Some((at, v)) = lock(&CHECKS).get(mode)
        && at.elapsed() < Duration::from_secs(5)
    {
        return v.clone();
    }
    let v = check_now(mode).unwrap_or_else(|e| json!({"error": e}));
    lock(&CHECKS).insert(mode.to_string(), (Instant::now(), v.clone()));
    v
}

fn forget_checks() {
    lock(&CHECKS).clear();
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------------------------------- the service

/// A process the app started for the index, and the last of what it printed.
struct Proc {
    child: Child,
    tail: Arc<Mutex<String>>,
}

/// What to start for a mode.
struct Launch {
    serve: Command,
    /// Qdrant (Heavy) and the HTTP port it is told to use
    qdrant: Option<(Command, u16)>,
    token: String,
}

/// The service while it runs.
struct Running {
    mode: String,
    rag: Proc,
    qdrant: Option<Proc>,
    url: String,
    token: String,
    /// it has said it is ready since it started (kept: a later rebuild does not take the tools' address away)
    ready: bool,
}

struct State {
    running: Option<Running>,
    /// the mode being started, for the status
    starting: Option<String>,
    /// processes of a start still under way (ended by `shut_down` if the app closes meanwhile)
    starting_pids: Vec<u32>,
    last_start: Option<Instant>,
    error: String,
    /// the service exited by itself: this mode is started again (`RESTART_EVERY`) while it is still the one chosen
    restart: Option<String>,
    /// the app is closing: nothing more is started
    stopping: bool,
}

static STATE: Mutex<State> = Mutex::new(State { running: None, starting: None, starting_pids: Vec::new(), last_start: None, error: String::new(), restart: None, stopping: false });

/// Held for a whole stop-and-start, so two never interleave.
static APPLY: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_ENDPOINT: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

/// For tests: what `endpoint` gives on this thread.
#[cfg(test)]
pub fn set_test_endpoint(e: Option<(String, String)>) {
    TEST_ENDPOINT.with(|t| *t.borrow_mut() = e);
}

/// (url, token) for the tool servers, only while the service runs and has said it is ready.
pub fn endpoint() -> Option<(String, String)> {
    #[cfg(test)]
    if let Some(e) = TEST_ENDPOINT.with(|t| t.borrow().clone()) {
        return Some(e);
    }
    lock(&STATE).running.as_ref().filter(|r| r.ready).map(|r| (r.url.clone(), r.token.clone()))
}

/// The running service's address and token, ready or not (Setup's test query and rebuild).
fn running_endpoint() -> Option<(String, String)> {
    lock(&STATE).running.as_ref().map(|r| (r.url.clone(), r.token.clone()))
}

/// Whether a service that exited by itself may be started again now.
pub fn may_restart(last_start: Option<Instant>, now: Instant) -> bool {
    last_start.is_none_or(|t| now.saturating_duration_since(t) >= RESTART_EVERY)
}

/// The port in the line `rag serve` prints once it listens: `{"listening": 52345}`.
fn listening_line(line: &str) -> Option<u16> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    v["listening"].as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0)
}

/// Keep the last 2000 characters of what `r` prints.
fn keep_tail(r: impl Read + Send + 'static, tail: Arc<Mutex<String>>) {
    thread::spawn(move || {
        for line in BufReader::new(r).lines().map_while(Result::ok) {
            let mut t = lock(&*tail);
            t.push_str(&line);
            t.push('\n');
            if t.len() > 2000 {
                let cut = t.len() - 2000;
                let cut = (cut..t.len()).find(|&i| t.is_char_boundary(i)).unwrap_or(t.len());
                t.drain(..cut);
            }
        }
    });
}

/// Start a process in a group of its own, noted in the ledger under `owner`. With `listen`, its stdout is read for
/// the listening line (the port comes on the channel); otherwise stdout goes into the tail with stderr.
fn spawn(mut cmd: Command, owner: &str, listen: bool) -> Result<(Proc, Option<mpsc::Receiver<u16>>), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    paths::own_group(&mut cmd);
    let mut child = cmd.start().map_err(|e| format!("Could not start the knowledge index: {e}"))?;
    procs::record(owner, child.id());
    lock(&STATE).starting_pids.push(child.id());
    let tail: Arc<Mutex<String>> = Arc::default();
    if let Some(err) = child.stderr.take() {
        keep_tail(err, tail.clone());
    }
    let mut port = None;
    if let Some(out) = child.stdout.take() {
        if listen {
            let (tx, rx) = mpsc::channel::<u16>();
            port = Some(rx);
            thread::spawn(move || {
                let mut told = false;
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if !told && let Some(p) = listening_line(&line) {
                        told = true;
                        let _ = tx.send(p);
                    }
                }
            });
        } else {
            keep_tail(out, tail.clone());
        }
    }
    Ok((Proc { child, tail }, port))
}

/// End a process and all it started, and forget it.
fn end(mut p: Proc) {
    let pid = p.child.id();
    paths::end_trees(&[pid]);
    let _ = p.child.wait();
    procs::forget(pid);
    lock(&STATE).starting_pids.retain(|x| *x != pid);
}

/// Why a process that should run is gone: its exit and the last of what it printed (never the token).
fn gone_why(what: &str, p: &mut Proc, token: &str) -> Option<String> {
    let code = match p.child.try_wait() {
        Ok(Some(s)) => s.code(),
        Ok(None) => return None,
        Err(_) => None,
    };
    let said = crate::promo::error_tail(&scrubbed(&lock(&*p.tail), token));
    let exit = code.map(|c| format!("exit {c}")).unwrap_or_else(|| "it was ended".into());
    Some(if said.is_empty() { format!("{what} stopped ({exit}).") } else { format!("{what} stopped ({exit}): {said}") })
}

/// Wait for Qdrant to answer that it is ready (its health routes need no key).
fn wait_qdrant(port: u16, p: &mut Proc, token: &str) -> Result<(), String> {
    let until = Instant::now() + QDRANT_WAIT;
    let url = format!("http://127.0.0.1:{port}");
    loop {
        if let Some(why) = gone_why("Qdrant", p, token) {
            return Err(why);
        }
        if call(&url, "", "GET", "/readyz", None, Duration::from_secs(2)).is_ok_and(|(s, _)| s == 200) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("Qdrant did not start in time.".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Start what `l` names: Qdrant first (Heavy), then the service, waiting for its listening line.
fn start(mode: &str, l: Launch) -> Result<Running, String> {
    let token = l.token;
    let qdrant = match l.qdrant {
        Some((cmd, port)) => {
            let (mut q, _) = spawn(cmd, "rag-qdrant", false)?;
            if let Err(e) = wait_qdrant(port, &mut q, &token) {
                end(q);
                return Err(e);
            }
            Some(q)
        }
        None => None,
    };
    let fail = |q: Option<Proc>, e: String| -> Result<Running, String> {
        if let Some(q) = q {
            end(q);
        }
        Err(e)
    };
    let (mut rag, rx) = match spawn(l.serve, "rag", true) {
        Ok(x) => x,
        Err(e) => return fail(qdrant, e),
    };
    let rx = rx.expect("listening");
    let until = Instant::now() + LISTEN_WAIT;
    let port = loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(p) => break p,
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < until => {
                if let Some(why) = gone_why("The knowledge index", &mut rag, &token) {
                    end(rag);
                    return fail(qdrant, why);
                }
            }
            _ => {
                let why = gone_why("The knowledge index", &mut rag, &token).unwrap_or_else(|| "The knowledge index did not start in time.".into());
                end(rag);
                return fail(qdrant, why);
            }
        }
    };
    {
        let mut s = lock(&STATE);
        s.starting_pids.retain(|p| *p != rag.child.id() && Some(*p) != qdrant.as_ref().map(|q| q.child.id()));
    }
    Ok(Running { mode: mode.to_string(), rag, qdrant, url: format!("http://127.0.0.1:{port}"), token, ready: false })
}

/// End the running service: the index's process first (it is Qdrant's client), then Qdrant.
fn stop_running(r: Running) {
    end(r.rag);
    if let Some(q) = r.qdrant {
        end(q);
    }
}

fn stop_service() {
    let taken = lock(&STATE).running.take();
    if let Some(r) = taken {
        stop_running(r);
    }
}

/// What to start for `mode`, when its setup is complete (None when it is not: nothing starts, Setup shows the steps).
fn launch_for(mode: &str) -> Result<Option<Launch>, String> {
    if mode == "heavy" && cfg!(windows) {
        return Err("Heavy is Mac only for now.".into());
    }
    let check = check_now(mode)?;
    lock(&CHECKS).insert(mode.to_string(), (Instant::now(), check.clone()));
    if !setup_complete(mode, &check) {
        return Ok(None);
    }
    let token = new_token().ok_or("Fridae could not make a key for the knowledge index (the system's random source could not be read), so it did not start it.")?;
    let file = token_file();
    write_token(&file, &token)?;
    let qdrant = if mode == "heavy" {
        let exe = qdrant_exe();
        if !exe.is_file() {
            return Ok(None);
        }
        let (http, grpc) = (free_port()?, free_port()?);
        let storage = dir().join("qdrant-storage");
        let _ = fs::create_dir_all(&storage);
        Some((qdrant_command(&exe, &storage, http, grpc, &token), http))
    } else {
        None
    };
    let url = qdrant.as_ref().map(|(_, http)| format!("http://127.0.0.1:{http}"));
    Ok(Some(Launch { serve: serve_command(mode, &dir(), &file, url.as_deref()), qdrant, token }))
}

/// Stop the running service (and Qdrant) first, then start the one for `mode` with `launch`. Whether one started.
/// `retry`: a start after the service exited by itself, tried again (`RESTART_EVERY`) if it fails.
fn apply_with(mode: &str, retry: bool, launch: impl FnOnce(&str) -> Result<Option<Launch>, String>) -> bool {
    let _one = lock(&APPLY);
    apply_locked(mode, retry, launch)
}

/// `apply_with`, with `APPLY` held by the caller.
fn apply_locked(mode: &str, retry: bool, launch: impl FnOnce(&str) -> Result<Option<Launch>, String>) -> bool {
    stop_service();
    {
        let mut s = lock(&STATE);
        s.restart = None;
        // a start again after the service exited keeps why it exited, for the status; a start the user asked for
        // (a new mode, a setup step, Build) begins afresh
        if !retry {
            s.error.clear();
        }
        if s.stopping || !MODES[1..].contains(&mode) {
            return false;
        }
        s.starting = Some(mode.to_string());
        s.last_start = Some(Instant::now());
    }
    let started = launch(mode).and_then(|l| l.map(|l| start(mode, l)).transpose());
    let mut s = lock(&STATE);
    s.starting = None;
    match started {
        Ok(Some(r)) if s.stopping => {
            drop(s);
            stop_running(r);
            false
        }
        Ok(Some(r)) => {
            s.running = Some(r);
            true
        }
        Ok(None) => false,
        Err(e) => {
            s.error = e;
            if retry {
                s.restart = Some(mode.to_string());
            }
            false
        }
    }
}

/// Stop the current service (and Qdrant) first, then start the one for `mode` if its setup is complete. Called at
/// launch, on a change of the setting and after a setup step, each time with the mode chosen then; should the
/// setting have changed again by the time the lock is held, the newer choice is the one started, so quick changes
/// always end on the last one chosen.
pub fn apply_mode(mode: &str) {
    let _one = lock(&APPLY);
    let chosen = paths::rag_mode();
    apply_locked(if chosen == mode { mode } else { &chosen }, false, launch_for);
    forget_checks();
}

fn apply_chosen() {
    apply_mode(&paths::rag_mode());
}

/// Whether the service runs for `mode` (or is being started for it).
fn serving(mode: &str) -> bool {
    let s = lock(&STATE);
    s.running.as_ref().is_some_and(|r| r.mode == mode) || s.starting.as_deref() == Some(mode)
}

/// At launch: the chosen mode's service, and the watch that starts it again if it exits.
pub fn start_on_launch() {
    thread::spawn(|| {
        apply_chosen();
        loop {
            thread::sleep(Duration::from_secs(2));
            tick(Instant::now());
        }
    });
}

/// The watch, every two seconds: a service that exited is noted (its last words kept for the status) and started
/// again at most every 30 s while its mode is still chosen; one not yet ready is asked whether it is.
fn tick(now: Instant) {
    let died = {
        let mut s = lock(&STATE);
        if s.stopping {
            return;
        }
        let why = s.running.as_mut().and_then(|r| {
            let token = r.token.clone();
            gone_why("The knowledge index", &mut r.rag, &token).or_else(|| r.qdrant.as_mut().and_then(|q| gone_why("Qdrant", q, &token)))
        });
        why.map(|w| (w, s.running.take()))
    };
    if let Some((why, Some(r))) = died {
        let mode = r.mode.clone();
        stop_running(r);
        let mut s = lock(&STATE);
        s.error = why;
        s.restart = Some(mode);
    }
    let again = {
        let s = lock(&STATE);
        s.restart.clone().filter(|_| may_restart(s.last_start, now))
    };
    if let Some(mode) = again {
        if paths::rag_mode() == mode {
            apply_with(&mode, true, launch_for);
        } else {
            lock(&STATE).restart = None;
        }
    }
    let unready = lock(&STATE).running.as_ref().filter(|r| !r.ready).map(|r| (r.url.clone(), r.token.clone()));
    if let Some((url, token)) = unready {
        let _ = service_status(&url, &token);
    }
}

// ------------------------------------------------------------------------------------------- talking to it

/// A request to a local service: (status, JSON body; Null when empty or not JSON). No proxy, ever.
fn call(url: &str, token: &str, method: &str, path: &str, body: Option<&Value>, limit: Duration) -> Result<(u16, Value), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(limit)).http_status_as_error(false).proxy(None).build().into();
    let address = format!("{url}{path}");
    let bearer = format!("Bearer {token}");
    let resp = match method {
        "POST" => {
            let mut r = agent.post(&address);
            if !token.is_empty() {
                r = r.header("Authorization", bearer.as_str());
            }
            r.send_json(body.cloned().unwrap_or_else(|| json!({})))
        }
        _ => {
            let mut r = agent.get(&address);
            if !token.is_empty() {
                r = r.header("Authorization", bearer.as_str());
            }
            r.call()
        }
    };
    let mut resp = resp.map_err(|e| scrubbed(&e.to_string(), token))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// The service's own `/status`, or Null when it does not answer. A ready answer makes it ready for the tools.
fn service_status(url: &str, token: &str) -> Value {
    match call(url, token, "GET", "/status", None, Duration::from_secs(2)) {
        Ok((200, v)) if v.is_object() => {
            if v["ready"] == true {
                let mut s = lock(&STATE);
                if let Some(r) = s.running.as_mut().filter(|r| r.url == url) {
                    r.ready = true;
                }
            }
            v
        }
        _ => Value::Null,
    }
}

/// `GET /api/rag/status[?mode=light|heavy]`: the mode, `rag check` for that mode (the chosen one by default; Null when
/// Off), the service's own status (Null when it does not run), the setup step, the process, and whether this is
/// Windows (Heavy is Mac only for now).
pub fn status(asked: Option<&str>) -> Value {
    let mode = paths::rag_mode();
    let check_mode = asked.filter(|m| MODES[1..].contains(m)).unwrap_or(mode.as_str());
    let check = if check_mode == "off" { Value::Null } else { check_cached(check_mode) };
    let service = running_endpoint().map(|(url, token)| service_status(&url, &token)).unwrap_or(Value::Null);
    let process = {
        let s = lock(&STATE);
        json!({"running": s.running.is_some(), "mode": s.running.as_ref().map(|r| r.mode.clone()), "starting": s.starting, "ready": s.running.as_ref().is_some_and(|r| r.ready), "error": s.error, "restarting": s.restart.is_some()})
    };
    json!({"mode": mode, "check": check, "service": service, "setup": setup_state(), "windows": cfg!(windows), "process": process})
}

/// `POST /api/rag/mode {"mode"}`: written as the guarded setting, then the service for it started (on a thread of its
/// own; the page follows the status). Choosing the mode that already runs changes nothing.
pub fn choose(body: &Value) -> Value {
    let mode = body["mode"].as_str().unwrap_or("");
    if let Err(e) = set_mode(mode) {
        return json!({"ok": false, "error": e});
    }
    if !serving(mode) || mode == "off" {
        thread::spawn(apply_chosen);
    }
    json!({"ok": true, "mode": paths::rag_mode()})
}

/// `POST /api/rag/test {"q"}`: one query through the service, its top 3 titles.
pub fn test_query(body: &Value) -> Value {
    let q = body["q"].as_str().unwrap_or("").trim();
    if q.is_empty() {
        return json!({"ok": false, "error": "Type something to look for."});
    }
    let Some((url, token)) = running_endpoint() else { return json!({"ok": false, "error": "The knowledge index is not running."}) };
    match call(&url, &token, "POST", "/query", Some(&json!({"q": q, "shelves": SHELVES, "k": 3, "project": ""})), Duration::from_secs(15)) {
        Ok((200, v)) => {
            let hits: Vec<Value> = v["hits"].as_array().into_iter().flatten().take(3).map(|h| json!({"title": h["title"], "heading": h["heading"], "shelf": h["shelf"]})).collect();
            let titles: Vec<String> = hits.iter().map(|h| h["title"].as_str().unwrap_or("").to_string()).collect();
            json!({"ok": true, "titles": titles, "hits": hits})
        }
        Ok((503, _)) => json!({"ok": false, "error": "The index is still being built."}),
        Ok((code, v)) => json!({"ok": false, "error": v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("The knowledge index answered {code}."))}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// `POST /api/rag/rebuild`: the running service builds its index again; one that does not run is started (Setup's
/// Build step).
pub fn rebuild() -> Value {
    let Some((url, token)) = running_endpoint() else {
        if paths::rag_mode() == "off" {
            return json!({"ok": false, "error": "The knowledge index is off."});
        }
        thread::spawn(apply_chosen);
        return json!({"ok": true, "started": true});
    };
    match call(&url, &token, "POST", "/rebuild", None, Duration::from_secs(10)) {
        Ok((200, v)) => json!({"ok": true, "started": v["started"] == true}),
        Ok((code, _)) => json!({"ok": false, "error": format!("The knowledge index answered {code}.")}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

// ------------------------------------------------------------------------------------------- setup steps

/// The setup step running in the background, or how the last one ended.
struct Setup {
    running: bool,
    /// the setup step (`SETUP_STEPS`) running or run last
    step: String,
    /// what it is doing now, from its progress lines
    detail: String,
    fraction: f64,
    error: String,
    child: Option<Child>,
    stopping: bool,
}

static SETUP: Mutex<Setup> = Mutex::new(Setup { running: false, step: String::new(), detail: String::new(), fraction: 0.0, error: String::new(), child: None, stopping: false });

fn setup_state() -> Value {
    let s = lock(&SETUP);
    json!({"running": s.running, "step": s.step, "detail": s.detail, "fraction": s.fraction, "error": s.error})
}

/// One line of `rag setup --json-progress`, read by the promo engine's parser (`promo::engine_line`), with the last
/// line's `error` kept.
pub fn setup_line(line: &str) -> Option<(EngineLine, String)> {
    let parsed = crate::promo::engine_line(line)?;
    let error = match &parsed {
        EngineLine::Done(..) => serde_json::from_str::<Value>(line.trim()).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_default(),
        EngineLine::Progress(..) => String::new(),
    };
    Some((parsed, error))
}

/// How a setup step ended.
#[derive(Debug, PartialEq)]
enum SetupEnd {
    Ready,
    /// stopped by the app (it is closing), not a failure
    Stopped,
    Failed(String),
}

/// `done` is the last line's (ready, missing, error), when it came; `code` the exit code (None when killed).
fn setup_end(stopping: bool, done: Option<(bool, Vec<String>, String)>, code: Option<i32>, stderr: &str) -> SetupEnd {
    match done {
        Some((true, _, _)) if code == Some(0) => SetupEnd::Ready,
        _ if stopping => SetupEnd::Stopped,
        Some((_, _, error)) if !error.trim().is_empty() => SetupEnd::Failed(error.trim().to_string()),
        Some((_, missing, _)) if code == Some(0) && !missing.is_empty() => SetupEnd::Failed(format!("The step finished, but these parts are still missing: {}.", missing.join(", "))),
        _ => match crate::promo::error_tail(stderr) {
            why if !why.is_empty() => SetupEnd::Failed(why),
            _ => SetupEnd::Failed(format!("The step stopped (exit {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "without a code".into()))),
        },
    }
}

/// Run one setup step's command and pass its progress on; `ready` runs when it ends ready.
fn run_setup(mut cmd: Command, step: String, ready: fn()) {
    let mut child = match cmd.start() {
        Ok(c) => c,
        Err(e) => return finish_setup(&step, SetupEnd::Failed(format!("Could not run the tools: {e}")), ready),
    };
    procs::record("rag-setup", child.id());
    let pid = child.id();
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    {
        let mut s = lock(&SETUP);
        if s.stopping {
            // the app began closing while this was starting
            crate::promo::kill_tree(&mut child);
        }
        s.child = Some(child);
    }
    let errs = thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut text);
        }
        text
    });
    let (mut detail, mut shown, mut done) = (String::new(), -1.0, None);
    for line in stdout.map(BufReader::new).into_iter().flat_map(|r| r.lines()).map_while(Result::ok) {
        match setup_line(&line) {
            Some((EngineLine::Progress(d, f), _)) => {
                {
                    let mut s = lock(&SETUP);
                    s.detail = d.clone();
                    s.fraction = f;
                }
                if d != detail || f >= 1.0 || f - shown >= 0.01 {
                    BUS.put("rag_setup", json!({"step": step, "detail": d, "fraction": f}));
                    shown = f;
                }
                detail = d;
            }
            Some((EngineLine::Done(r, missing), error)) => {
                done = Some((r, missing, error));
                break;
            }
            None => {}
        }
    }
    let err_text = errs.join().unwrap_or_default();
    let child = lock(&SETUP).child.take();
    let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
    procs::forget(pid);
    let stopping = lock(&SETUP).stopping;
    finish_setup(&step, setup_end(stopping, done, code, &err_text), ready);
}

/// The step has ended: the state is settled before the page hears of it.
fn finish_setup(step: &str, end: SetupEnd, ready: fn()) {
    {
        let mut s = lock(&SETUP);
        s.running = false;
        s.child = None;
        s.error = match &end {
            SetupEnd::Failed(e) => e.clone(),
            _ => String::new(),
        };
        if end == SetupEnd::Ready {
            s.fraction = 1.0;
        }
    }
    forget_checks();
    match end {
        SetupEnd::Ready => {
            BUS.put("rag_setup", json!({"done": true, "step": step}));
            ready();
        }
        SetupEnd::Stopped => BUS.put("rag_setup", json!({"stopped": true, "step": step})),
        SetupEnd::Failed(error) => BUS.put("rag_setup", json!({"failed": true, "step": step, "error": error})),
    }
}

/// After a step ends ready: the chosen mode's service starts if it does not run (it starts only once its setup is
/// complete; Setup's last step, Build, is that start).
fn after_setup() {
    let mode = paths::rag_mode();
    if mode != "off" && !serving(&mode) {
        thread::spawn(apply_chosen);
    }
}

/// Mark a step as running, unless one already runs or the app is closing. Whether it was marked.
fn begin_setup(step: &str) -> bool {
    let mut s = lock(&SETUP);
    let start = !s.running && !s.stopping;
    if start {
        *s = Setup { running: true, step: step.to_string(), detail: String::new(), fraction: 0.0, error: String::new(), child: None, stopping: false };
    }
    start
}

/// `POST /api/rag/setup {"step", "mode"?}`: one step at a time. `models` is per mode (the body's, else the chosen
/// one); the Ollama and Qdrant steps are Heavy's.
pub fn setup(body: &Value) -> Value {
    let step = body["step"].as_str().unwrap_or("");
    if !SETUP_STEPS.contains(&step) {
        return json!({"ok": false, "error": "Unknown step."});
    }
    let chosen = paths::rag_mode();
    let mode = match step {
        "models" => body["mode"].as_str().filter(|m| MODES[1..].contains(m)).unwrap_or(chosen.as_str()).to_string(),
        _ => "heavy".to_string(),
    };
    if mode == "off" {
        return json!({"ok": false, "error": "Pick Light or Heavy first."});
    }
    if mode == "heavy" && cfg!(windows) {
        return json!({"ok": false, "error": "Heavy is Mac only for now."});
    }
    if begin_setup(step) {
        let (cmd, step) = (setup_command(step, &mode, &dir()), step.to_string());
        thread::spawn(move || run_setup(cmd, step, after_setup));
    }
    json!({"ok": true, "setup": setup_state()})
}

// ------------------------------------------------------------------------------------------- quitting

/// When the app quits: a setup step is stopped with all it started, and the service and Qdrant end. Nothing more
/// starts. The user's own Ollama is not the app's: it is never stopped.
pub fn shut_down() {
    {
        let mut s = lock(&SETUP);
        s.stopping = true;
        if let Some(c) = s.child.as_mut() {
            crate::promo::kill_tree(c);
        }
    }
    let starting = {
        let mut s = lock(&STATE);
        s.stopping = true;
        s.restart = None;
        std::mem::take(&mut s.starting_pids)
    };
    stop_service();
    if !starting.is_empty() {
        paths::end_trees(&starting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn mode_is_guarded_and_validated() {
        assert!(crate::paths::GUARDED.contains(&"rag_mode"));
        assert_eq!(MODES, ["off", "light", "heavy"]);
        assert!(set_mode("bogus").is_err(), "an unknown mode is refused before anything is written");
        assert_eq!(mode_of(&json!("bogus")), "off");
        assert_eq!(mode_of(&json!(null)), "off");
        assert_eq!(mode_of(&json!("heavy")), "heavy");
    }

    #[test]
    fn rag_folder_is_protected() {
        let rag = crate::paths::PATHS.work.join("rag");
        assert!(crate::paths::app_protected().contains(&(rag, true)));
    }

    #[test]
    fn env_has_rag_only_when_ready() {
        let guarded = crate::paths::guarded_settings();
        let print = crate::agents::guarded_print(&guarded);
        let env = crate::agents::test_servers(true)["rbxkit"]["env"].clone();
        assert!(env.get("FRIDAE_RAG_URL").is_none() && env.get("FRIDAE_RAG_TOKEN").is_none(), "{env}");
        set_test_endpoint(Some(("http://127.0.0.1:5999".into(), "t0k".into())));
        let env = crate::agents::test_servers(true)["rbxkit"]["env"].clone();
        set_test_endpoint(None);
        assert_eq!(env["FRIDAE_RAG_URL"], "http://127.0.0.1:5999");
        assert_eq!(env["FRIDAE_RAG_TOKEN"], "t0k");
        assert_eq!(crate::agents::guarded_print(&crate::paths::guarded_settings()), print, "keyed on the mode only, never the token");
    }

    #[test]
    fn restart_backoff_30s() {
        let t0 = Instant::now();
        assert!(!may_restart(Some(t0), t0 + Duration::from_secs(29)));
        assert!(may_restart(Some(t0), t0 + Duration::from_secs(30)));
        assert!(may_restart(None, t0), "never started: at once");
    }

    #[test]
    fn setup_lines_parse() {
        use crate::promo::{EngineLine, engine_line};
        for line in [r#"{"step": "models", "fraction": 0.42}"#, r#"{"step": "pull", "fraction": 3}"#, r#"{"ready": true, "missing": []}"#, r#"{"ready": false, "missing": ["qdrant"], "error": "no disk"}"#, "Downloading", "[1]"] {
            assert_eq!(setup_line(line).map(|s| s.0), engine_line(line), "{line}");
        }
        assert_eq!(setup_line(r#"{"ready": false, "missing": ["qdrant"], "error": "no disk"}"#), Some((EngineLine::Done(false, vec!["qdrant".into()]), "no disk".into())));
    }

    #[test]
    fn setup_is_complete_per_mode() {
        let all = json!({"models": true, "ollama": "running", "ollama_model": true, "qdrant_binary": true});
        assert!(setup_complete("light", &json!({"models": true, "ollama": "missing"})));
        assert!(!setup_complete("light", &json!({"models": false})));
        assert!(setup_complete("heavy", &all));
        for (k, v) in [("models", json!(false)), ("ollama", json!("stopped")), ("ollama_model", json!(false)), ("qdrant_binary", json!(false))] {
            let mut c = all.clone();
            c[k] = v;
            assert!(!setup_complete("heavy", &c), "{k}");
        }
        assert!(!setup_complete("off", &all));
        assert!(!setup_complete("heavy", &json!({"error": "could not run"})));
    }

    #[test]
    fn the_token_goes_by_file_never_on_a_command_line() {
        let args = |c: &Command| c.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>();
        let light = serve_command("light", Path::new("/w/rag"), Path::new("/w/rag/token"), None);
        let a = args(&light);
        let tail: Vec<&str> = a.iter().map(String::as_str).skip_while(|x| *x != "rag").collect();
        assert_eq!(tail, ["rag", "serve", "--mode", "light", "--data", "/w/rag", "--port", "0", "--token-file", "/w/rag/token"]);
        assert!(light.get_envs().any(|(k, v)| k == "HF_HUB_OFFLINE" && v.is_some_and(|v| v == "1")));
        let heavy = serve_command("heavy", Path::new("/w/rag"), Path::new("/w/rag/token"), Some("http://127.0.0.1:7001"));
        let a = args(&heavy);
        assert!(a.windows(2).any(|w| w == ["--qdrant-url", "http://127.0.0.1:7001"]) && a.windows(2).any(|w| w == ["--qdrant-key-file", "/w/rag/token"]), "{a:?}");
        // Qdrant: its key in its environment (as Qdrant reads it), never in its arguments; never the default port
        let q = qdrant_command(Path::new("/w/rag/bin/qdrant"), Path::new("/w/rag/qdrant-storage"), 7001, 7002, "s3cret");
        assert!(args(&q).is_empty());
        let env: HashMap<String, String> = q.get_envs().filter_map(|(k, v)| Some((k.to_string_lossy().to_string(), v?.to_string_lossy().to_string()))).collect();
        assert_eq!(env["QDRANT__SERVICE__HOST"], "127.0.0.1");
        assert_eq!((env["QDRANT__SERVICE__HTTP_PORT"].as_str(), env["QDRANT__SERVICE__GRPC_PORT"].as_str()), ("7001", "7002"));
        assert_eq!(env["QDRANT__SERVICE__API_KEY"], "s3cret");
        assert_eq!(env["QDRANT__STORAGE__STORAGE_PATH"], "/w/rag/qdrant-storage");
        assert_eq!(env["QDRANT__TELEMETRY_DISABLED"], "true");
        let setup = setup_command("ollama-pull", "heavy", Path::new("/w/rag"));
        let a = args(&setup);
        let tail: Vec<&str> = a.iter().map(String::as_str).skip_while(|x| *x != "rag").collect();
        assert_eq!(tail, ["rag", "setup", "ollama-pull", "--mode", "heavy", "--data", "/w/rag", "--json-progress"]);
    }

    #[test]
    fn the_token_file_is_the_users_alone() {
        let t = new_token().unwrap();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, new_token().unwrap());
        let d = crate::paths::test_dir("rag-token");
        let _ = fs::remove_dir_all(&d);
        let file = d.join("token");
        write_token(&file, &t).unwrap();
        write_token(&file, "second").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "no temporary file is left");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn what_a_process_printed_never_shows_the_token() {
        assert_eq!(scrubbed("bad key abc123 for abc123", "abc123"), "bad key [key] for [key]");
        assert_eq!(scrubbed("plain", ""), "plain");
        assert_eq!(listening_line(r#"{"listening": 52345}"#), Some(52345));
        assert_eq!(listening_line(r#"{"listening": 0}"#), None);
        assert_eq!(listening_line("Loading models"), None);
    }

    /// Each request a stub was sent: (path, Authorization header).
    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// A stub HTTP server: answers every request with `body`, and notes each one's path and Authorization header.
    fn stub(body: Value) -> (String, Seen) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let seen: Seen = Arc::default();
        let noted = seen.clone();
        thread::spawn(move || {
            while let Ok(req) = server.recv() {
                let auth = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
                lock(&noted).push((req.url().to_string(), auth));
                let _ = req.respond(tiny_http::Response::from_string(body.to_string()));
            }
        });
        (url, seen)
    }

    #[test]
    fn requests_carry_the_bearer_token() {
        let (url, seen) = stub(json!({"ready": true, "hits": [{"title": "DataStores"}]}));
        let (code, v) = call(&url, "t0k", "GET", "/status", None, Duration::from_secs(3)).unwrap();
        assert_eq!((code, v["ready"].as_bool()), (200, Some(true)));
        let (code, v) = call(&url, "t0k", "POST", "/query", Some(&json!({"q": "save"})), Duration::from_secs(3)).unwrap();
        assert_eq!((code, v["hits"][0]["title"].as_str()), (200, Some("DataStores")));
        assert_eq!(*lock(&seen), [("/status".to_string(), "Bearer t0k".to_string()), ("/query".to_string(), "Bearer t0k".to_string())]);
        // a service that does not answer is an error, not a hang
        let closed = format!("http://127.0.0.1:{}", free_port().unwrap());
        assert!(call(&closed, "t0k", "GET", "/status", None, Duration::from_secs(3)).is_err());
    }

    fn alive(pid: u32) -> bool {
        Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).run_status().is_ok_and(|s| s.success())
    }

    /// A stand-in for `rag serve`: says it listens, then waits.
    fn stand_in_serve() -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", "echo '{\"listening\": 4321}'; exec sleep 30"]);
        c
    }

    fn pids() -> (Option<u32>, Option<u32>) {
        let s = lock(&STATE);
        (s.running.as_ref().map(|r| r.rag.child.id()), s.running.as_ref().and_then(|r| r.qdrant.as_ref().map(|q| q.child.id())))
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_switch_stops_the_old_processes_first() {
        // Heavy's stand-ins: a Qdrant (whose readiness a stub answers) and the service
        let (qdrant_url, _) = stub(json!({}));
        let qdrant_port: u16 = qdrant_url.rsplit(':').next().unwrap().parse().unwrap();
        let heavy = || {
            let mut q = Command::new("sleep");
            q.arg("30");
            Ok(Some(Launch { serve: stand_in_serve(), qdrant: Some((q, qdrant_port)), token: "t".into() }))
        };
        assert!(apply_with("heavy", false, |_| heavy()));
        let (Some(rag1), Some(q1)) = pids() else { panic!("Heavy runs both") };
        assert!(alive(rag1) && alive(q1) && crate::procs::recorded(rag1) && crate::procs::recorded(q1));
        assert_eq!(running_endpoint(), Some(("http://127.0.0.1:4321".to_string(), "t".to_string())));
        assert_eq!(endpoint(), None, "not ready yet: the tools are not told of it");
        // to Light: both of Heavy's are gone before Light's is asked for
        assert!(apply_with("light", false, |_| {
            assert!(!alive(rag1) && !alive(q1), "the old service and its Qdrant end first");
            Ok(Some(Launch { serve: stand_in_serve(), qdrant: None, token: "t2".into() }))
        }));
        assert!(!crate::procs::recorded(rag1) && !crate::procs::recorded(q1));
        let (Some(rag2), None) = pids() else { panic!("Light runs the service alone") };
        // a mode whose setup is not complete starts nothing, after stopping what ran
        assert!(!apply_with("light", false, |_| Ok(None)));
        assert!(!alive(rag2) && pids() == (None, None));
        // a service that does not start: its last words are kept, and nothing is left running
        assert!(!apply_with("light", true, |_| {
            let mut c = Command::new("sh");
            c.args(["-c", "echo 'no models found' >&2; exit 3"]);
            Ok(Some(Launch { serve: c, qdrant: None, token: "t3".into() }))
        }));
        {
            let s = lock(&STATE);
            assert!(s.error.contains("no models found") && s.error.contains("exit 3"), "{}", s.error);
            assert_eq!(s.restart.as_deref(), Some("light"), "a failed restart is tried again later");
            assert!(s.running.is_none() && s.starting_pids.is_empty());
        }
        assert!(!apply_with("off", false, |_| panic!("Off starts nothing")));
        assert!(lock(&STATE).error.is_empty() && lock(&STATE).restart.is_none());
    }

    #[test]
    fn a_setup_step_ends_ready_stopped_or_failed() {
        assert_eq!(setup_end(false, Some((true, vec![], String::new())), Some(0), ""), SetupEnd::Ready);
        assert_eq!(setup_end(true, None, None, "BrokenPipeError"), SetupEnd::Stopped);
        assert_eq!(setup_end(true, Some((true, vec![], String::new())), Some(0), ""), SetupEnd::Ready);
        assert_eq!(setup_end(false, Some((false, vec!["ollama".into()], "Ollama did not answer.".into())), Some(0), ""), SetupEnd::Failed("Ollama did not answer.".into()));
        assert_eq!(setup_end(false, Some((false, vec!["qdrant".into()], String::new())), Some(0), ""), SetupEnd::Failed("The step finished, but these parts are still missing: qdrant.".into()));
        assert_eq!(setup_end(false, None, Some(1), "pip failed\n"), SetupEnd::Failed("pip failed".into()));
        assert_eq!(setup_end(false, None, None, ""), SetupEnd::Failed("The step stopped (exit without a code).".into()));
    }

    #[cfg(unix)]
    #[test]
    fn a_setup_step_runs_one_at_a_time_and_reports_its_progress() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static READY: AtomicBool = AtomicBool::new(false);
        let stand_in = |script: &str| {
            let mut c = Command::new("sh");
            c.args(["-c", script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            c
        };
        assert!(begin_setup("models"));
        assert!(!begin_setup("ollama-pull"), "one step at a time");
        run_setup(stand_in(r#"echo '{"step": "bge-small", "fraction": 0.5}'; echo '{"ready": true, "missing": [], "error": ""}'"#), "models".into(), || READY.store(true, Ordering::SeqCst));
        let s = setup_state();
        assert_eq!((s["running"].as_bool(), s["step"].as_str(), s["detail"].as_str(), s["error"].as_str()), (Some(false), Some("models"), Some("bge-small"), Some("")));
        assert!(READY.load(Ordering::SeqCst), "a step that ends ready starts the service if its setup is now complete");
        assert!(begin_setup("qdrant-install"));
        run_setup(stand_in(r#"echo '{"step": "download", "fraction": 0.1}'; echo '{"ready": false, "missing": ["qdrant"], "error": "The download failed."}'"#), "qdrant-install".into(), || panic!("not ready"));
        let s = setup_state();
        assert_eq!((s["step"].as_str(), s["error"].as_str()), (Some("qdrant-install"), Some("The download failed.")));
    }
}
