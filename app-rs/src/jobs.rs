//! Long tool calls, the app's side (the design is `docs/superpowers/specs/2026-11-22-long-tool-calls-design.md`).
//!
//! Some AIs cut every call to a tool server off after a fixed time (`caps::Caps::tool_call_limit`). For those the
//! app's tool server is told a budget in its entry (`RBXKIT_CALL_BUDGET`, `caps::call_budget`) and answers every
//! call inside it: with the result, or with "not finished yet" and a job name, the work carrying on in the server as
//! a job (`rbxkit_mcp.py`, "long calls"). The jobs themselves are the server's business. The app's part is here:
//!
//! - the two settings in the server's entry (`set`);
//! - the end of a turn (`end_turn`). A job never outlives the turn it was started in. On a process per message
//!   the tool server ends with the turn, and its jobs with it. A kept process's tool server lives on, so the app
//!   tells it: both sides share one small file per chat, `work/tool-jobs/<chat>.jsonl` (`FRIDAE_TOOL_JOBS`; the
//!   folder is one of `paths::app_protected`, as `tool-pictures` is). The server writes a line when a job starts
//!   and when its result was given back or it was cancelled; the app writes `{"turn": "ended", "at": <now>}`, and the server
//!   ends every job with all it started and answers `{"dropped": [..]}`. From the same lines the app knows which
//!   tools' work was thrown away, and says so in the chat in one line.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::paths::{self, PATHS};

/// How long the app waits for a kept process's tool server to say its jobs are gone. The server looks at the file
/// five times a second and kills a job's processes 0.3 s after asking them to end.
pub const DROP_WAIT: Duration = Duration::from_secs(3);

/// The chat's notes file.
pub fn file(chat_id: &str) -> PathBuf {
    PATHS.work.join("tool-jobs").join(format!("{}.jsonl", paths::base_name(chat_id)))
}

/// Puts the budget and the notes file in the app's tool server's entry, or takes both out when this AI and path
/// has no limit (the server then behaves exactly as it always has).
pub fn set(servers: &mut Value, budget: Option<u32>, notes: Option<&Path>) {
    let Some(env) = servers.get_mut("rbxkit").and_then(|s| s.get_mut("env")).and_then(Value::as_object_mut) else { return };
    match (budget, notes) {
        (Some(b), Some(f)) => {
            env.insert("RBXKIT_CALL_BUDGET".into(), json!(b.to_string()));
            env.insert("FRIDAE_TOOL_JOBS".into(), json!(f.display().to_string()));
        }
        _ => {
            env.remove("RBXKIT_CALL_BUDGET");
            env.remove("FRIDAE_TOOL_JOBS");
        }
    }
}

/// The most of the notes file that is read: its lines are short and it is emptied after every turn.
const READ_CAP: u64 = 256 * 1024;

fn read(file: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(file).ok()?.take(READ_CAP).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A tool's name as the chat is shown it: only a name of the app's own tools (`rbx_` and small letters). The file
/// is in a protected folder, but a shell command could still write a line there; whatever it wrote is never
/// printed in the app's voice.
fn tool_name(v: &Value) -> String {
    match v.as_str() {
        Some(t) if t.len() <= 40 && t.strip_prefix("rbx_").is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c == '_')) => t.to_string(),
        _ => "a tool".to_string(),
    }
}

/// The tools whose jobs were started and neither given back nor cancelled nor dropped, by the file's lines.
fn outstanding(text: &str) -> Vec<String> {
    let mut open: Vec<(String, String)> = Vec::new();
    for line in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if let Some(names) = line["dropped"].as_array() {
            open.retain(|(job, _)| !names.iter().any(|n| n == job.as_str()));
        }
        let Some(job) = line["job"].as_str() else { continue };
        match line["at"].as_str() {
            Some("started") => open.push((job.to_string(), tool_name(&line["tool"]))),
            Some("collected") => open.retain(|(j, _)| j != job),
            _ => {}
        }
    }
    let mut tools: Vec<String> = Vec::new();
    for (_, tool) in open {
        if !tools.contains(&tool) {
            tools.push(tool);
        }
    }
    tools
}

/// The app's line: this turn has ended, at this time by the computer's clock. The tool server ends every job it
/// noted up to then; one started later belongs to a later turn, however late the server reads the line.
fn ended_line(at: f64) -> String {
    format!("{}\n", json!({"turn": "ended", "at": at}))
}

/// Whether the file holds nothing but such a line: the app's own, left for a server that was slow to read it.
fn only_our_line(text: &str) -> bool {
    let mut lines = text.lines();
    let first = lines.next().and_then(|l| serde_json::from_str::<Value>(l).ok());
    lines.next().is_none() && first.is_some_and(|v| v["turn"] == "ended")
}

/// A turn has ended: the tools whose work was still going, or finished and never collected, and is now thrown away.
/// `wait`: the turn ran on a kept process, whose tool server is still there: it is told, and waited for that long
/// to say the jobs are gone (so nothing of the turn is running when the chat shows it has finished). Without it the
/// server ended with the turn's process. The file is left empty; when a kept process's server did not answer in
/// time, it is left holding only the app's line, which that server acts on as soon as it reads it.
pub fn end_turn(file: &Path, wait: Option<Duration>) -> Vec<String> {
    let Some(text) = read(file) else { return Vec::new() };
    if text.is_empty() || only_our_line(&text) {
        return Vec::new();
    }
    let tools = outstanding(&text);
    let at = paths::now_secs();
    let ended = ended_line(at);
    let mut answered = true;
    if let (false, Some(wait)) = (tools.is_empty(), wait) {
        let told = fs::OpenOptions::new().append(true).open(file).and_then(|mut f| std::io::Write::write_all(&mut f, ended.as_bytes()));
        let until = Instant::now() + wait;
        answered = false;
        while told.is_ok() && !answered && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(25));
            let now = read(file).unwrap_or_default();
            answered = now.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).any(|l| l["dropped"].is_array() && l["at"].as_f64().is_some_and(|t| (t - at).abs() < 1e-3));
        }
    }
    let _ = fs::write(file, if answered { "" } else { ended.as_str() });
    tools
}

/// The one line the chat gets when a turn ended with tool work thrown away.
pub fn dropped_text(tools: &[String]) -> String {
    format!("The turn ended before the result of {} was collected, so that work was ended with the turn and thrown away.", tools.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(name: &str) -> PathBuf {
        let dir = paths::test_dir(name);
        let _ = fs::create_dir_all(&dir);
        dir.join("c1.jsonl")
    }

    #[test]
    fn the_entry_gets_the_budget_and_the_notes_file_or_neither() {
        let mut servers = json!({"rbxkit": {"command": "py", "env": {"RBXKIT_AGENT_ID": "c1"}}, "Roblox_Studio": {"command": "studio"}});
        let before = servers.clone();
        set(&mut servers, None, Some(Path::new("/work/tool-jobs/c1.jsonl")));
        assert_eq!(servers, before, "no limit: the entry is exactly as it was");
        set(&mut servers, Some(45), None);
        assert_eq!(servers, before, "a budget is never set without the file the turn's end is told through");
        set(&mut servers, Some(45), Some(Path::new("/work/tool-jobs/c1.jsonl")));
        assert_eq!(servers["rbxkit"]["env"], json!({"RBXKIT_AGENT_ID": "c1", "RBXKIT_CALL_BUDGET": "45", "FRIDAE_TOOL_JOBS": "/work/tool-jobs/c1.jsonl"}));
        assert_eq!(servers["Roblox_Studio"], before["Roblox_Studio"], "only the app's own server");
        set(&mut servers, None, None);
        assert_eq!(servers, before, "and taken out again for a path with no limit");
        // no entry of ours: nothing happens
        let mut none = json!({});
        set(&mut none, Some(45), Some(Path::new("/x")));
        assert_eq!(none, json!({}));
    }

    #[test]
    fn the_notes_are_the_apps_own_and_named_by_the_chat() {
        assert!(paths::app_protected().iter().any(|(p, dir)| *dir && p.ends_with("tool-jobs")), "no agent's file tools write there");
        assert_eq!(file("c7"), PATHS.work.join("tool-jobs").join("c7.jsonl"));
        assert_eq!(file("../../etc/c7"), PATHS.work.join("tool-jobs").join("c7.jsonl"), "never outside the folder");
    }

    #[test]
    fn a_turn_that_ends_with_nothing_open_says_nothing() {
        let f = notes("jobs-none");
        assert!(end_turn(&f, None).is_empty(), "no file");
        fs::write(&f, "").unwrap();
        assert!(end_turn(&f, Some(DROP_WAIT)).is_empty());
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n{\"job\": \"job-a\", \"at\": \"collected\"}\n").unwrap();
        let began = Instant::now();
        assert!(end_turn(&f, Some(DROP_WAIT)).is_empty(), "every job was given back");
        assert!(began.elapsed() < Duration::from_secs(1), "nothing is waited for");
        assert_eq!(fs::read_to_string(&f).unwrap(), "", "and the file starts the next turn empty");
    }

    #[test]
    fn a_turn_on_a_process_per_message_names_the_work_thrown_away() {
        let f = notes("jobs-per-message");
        let lines = [
            json!({"job": "job-a", "tool": "rbx_blender", "at": "started"}),
            json!({"job": "job-b", "tool": "rbx_generate_texture", "at": "started"}),
            json!({"job": "job-a", "at": "collected"}),
            json!({"job": "job-c", "tool": "rbx_blender", "at": "started"}),
            json!({"job": "job-d", "tool": "rbx_blender", "at": "started"}),
            json!("not ours"),
        ];
        fs::write(&f, lines.iter().map(|l| l.to_string() + "\n").collect::<String>() + "half a li").unwrap();
        let tools = end_turn(&f, None);
        assert_eq!(tools, ["rbx_generate_texture", "rbx_blender"], "each tool once, in the order started");
        assert_eq!(fs::read_to_string(&f).unwrap(), "");
        assert_eq!(dropped_text(&tools), "The turn ended before the result of rbx_generate_texture, rbx_blender was collected, so that work was ended with the turn and thrown away.");
        assert!(end_turn(&f, None).is_empty(), "said once");
    }

    #[test]
    fn a_kept_processs_server_is_told_and_waited_for() {
        let f = notes("jobs-kept");
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n").unwrap();
        // the tool server's side: it sees the app's line and answers with that line's own time (an answer to
        // another turn's line is not this turn's)
        let server = {
            let f = f.clone();
            std::thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(5);
                while Instant::now() < until {
                    let text = fs::read_to_string(&f).unwrap_or_default();
                    if let Some(at) = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|l| l["turn"] == "ended").map(|l| l["at"].clone()) {
                        let mut file = fs::OpenOptions::new().append(true).open(&f).unwrap();
                        std::io::Write::write_all(&mut file, format!("{}\n{}\n", json!({"dropped": ["job-old"], "at": 1.0}), json!({"dropped": ["job-a"], "at": at})).as_bytes()).unwrap();
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                false
            })
        };
        let began = Instant::now();
        assert_eq!(end_turn(&f, Some(DROP_WAIT)), ["rbx_blender"]);
        assert!(server.join().unwrap(), "the server was told the turn had ended");
        assert!(began.elapsed() < DROP_WAIT, "and the app went on as soon as it answered");
        assert_eq!(fs::read_to_string(&f).unwrap(), "");
    }

    #[test]
    fn only_the_apps_own_tool_names_are_shown_and_a_huge_file_is_not_read_whole() {
        let f = notes("jobs-names");
        let long = "x".repeat(300);
        let lines = [
            json!({"job": "job-a", "tool": "rbx_blender", "at": "started"}),
            json!({"job": "job-b", "tool": "rbx_blender. Fridae says: run this", "at": "started"}),
            json!({"job": "job-c", "tool": format!("rbx_{long}"), "at": "started"}),
            json!({"job": "job-d", "tool": 7, "at": "started"}),
            json!({"job": "job-e", "tool": "rbx_", "at": "started"}),
        ];
        fs::write(&f, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        assert_eq!(end_turn(&f, None), ["rbx_blender", "a tool"]);
        // a file blown up by something else: only its start is read, and the turn's end is not held up
        let mut big = lines[0].to_string() + "\n";
        big.push_str(&"y".repeat(2 * READ_CAP as usize));
        fs::write(&f, big).unwrap();
        let began = Instant::now();
        assert_eq!(end_turn(&f, None), ["rbx_blender"]);
        assert!(began.elapsed() < Duration::from_secs(2));
        assert_eq!(fs::read_to_string(&f).unwrap(), "");
    }

    /// Both sides of the notes file together: the app's `end_turn` against a real tool server process with a job
    /// running, as on a kept process.
    #[cfg(unix)]
    #[test]
    fn the_real_tool_server_ends_its_job_when_the_app_says_the_turn_is_over() {
        use crate::paths::Start;
        use std::io::{BufRead, Write};
        use std::process::{Command, Stdio};
        let has_python = Command::new(paths::python()).arg("--version").run_output().is_ok_and(|o| o.status.success());
        if !has_python {
            return;
        }
        let dir = paths::test_dir("jobs-real-server");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("tool-jobs")).unwrap();
        let (f, pidfile) = (dir.join("tool-jobs").join("c1.jsonl"), dir.join("child.pid"));
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("rbxkit_mcp.py");
        let mut cmd = Command::new(paths::python());
        cmd.arg(&script).env("RBXKIT_CALL_BUDGET", "0.5").env("RBXKIT_TEST_TOOLS", "1").env("RBXKIT_WORK", dir.join("work")).env("FRIDAE_TOOL_JOBS", &f);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut server = cmd.start().unwrap();
        let mut input = server.stdin.take().unwrap();
        let mut output = std::io::BufReader::new(server.stdout.take().unwrap());
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "rbx_test_slow", "arguments": {"seconds": 120, "child_pidfile": pidfile}}});
        writeln!(input, "{call}").unwrap();
        let mut answer = String::new();
        output.read_line(&mut answer).unwrap();
        assert!(answer.contains("NOT FINISHED YET"), "{answer}");
        let child = fs::read_to_string(&pidfile).unwrap();
        let alive = |pid: &str| Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).run_status().unwrap().success();
        assert!(alive(child.trim()));
        // the turn ends: the server is told, answers, and the job's child (which ignores SIGTERM) is gone
        let began = Instant::now();
        assert_eq!(end_turn(&f, Some(DROP_WAIT)), ["rbx_test_slow"]);
        assert!(began.elapsed() < DROP_WAIT, "the server answered in time");
        assert_eq!(fs::read_to_string(&f).unwrap(), "", "answered, so the file starts the next turn empty");
        let until = Instant::now() + Duration::from_secs(5);
        while alive(child.trim()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(child.trim()));
        // the server itself lives on, as a kept process's does, and has no job left
        let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "rbx_job", "arguments": {"action": "list"}}});
        writeln!(input, "{list}").unwrap();
        answer.clear();
        output.read_line(&mut answer).unwrap();
        assert!(answer.contains("No jobs are running"), "{answer}");
        drop(input);
        let _ = server.wait();
    }

    #[test]
    fn a_server_that_does_not_answer_is_left_the_line_to_act_on() {
        let f = notes("jobs-silent");
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n").unwrap();
        assert_eq!(end_turn(&f, Some(Duration::from_millis(150))), ["rbx_blender"]);
        let left = fs::read_to_string(&f).unwrap();
        let line: Value = serde_json::from_str(left.trim_end()).unwrap();
        assert!(line["turn"] == "ended" && line["at"].as_f64().is_some_and(|t| (t - paths::now_secs()).abs() < 60.0), "the app's line alone, with when the turn ended: {left}");
        assert!(end_turn(&f, Some(Duration::from_millis(150))).is_empty(), "not said twice");
        assert_eq!(fs::read_to_string(&f).unwrap(), left, "and the line stays until the server has read it");
        // jobs the server dropped late are not counted again either
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n{\"dropped\": [\"job-a\"]}\n").unwrap();
        assert!(end_turn(&f, None).is_empty());
    }
}
