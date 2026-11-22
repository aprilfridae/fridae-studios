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
//!   and when its result was given back or it was cancelled; the app writes `{"turn": "ended"}`, and the server
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

/// The tools whose jobs were started and neither given back nor cancelled nor dropped, by the file's lines.
fn outstanding(text: &str) -> Vec<String> {
    let mut open: Vec<(String, String)> = Vec::new();
    for line in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if let Some(names) = line["dropped"].as_array() {
            open.retain(|(job, _)| !names.iter().any(|n| n == job.as_str()));
        }
        let Some(job) = line["job"].as_str() else { continue };
        match line["at"].as_str() {
            Some("started") => open.push((job.to_string(), line["tool"].as_str().unwrap_or("a tool").to_string())),
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

/// A turn has ended: the tools whose work was still going, or finished and never collected, and is now thrown away.
/// `wait`: the turn ran on a kept process, whose tool server is still there: it is told, and waited for that long
/// to say the jobs are gone (so nothing of the turn is running when the chat shows it has finished). Without it the
/// server ended with the turn's process. The file is left empty; when a kept process's server did not answer in
/// time, it is left holding only the app's line, which that server acts on as soon as it reads it.
pub fn end_turn(file: &Path, wait: Option<Duration>) -> Vec<String> {
    let Ok(text) = fs::read_to_string(file) else { return Vec::new() };
    if text.is_empty() {
        return Vec::new();
    }
    let tools = outstanding(&text);
    let ended = "{\"turn\": \"ended\"}\n";
    if text == ended {
        return tools; // the app's own line, still waiting for a server that was slow to read it
    }
    let mut answered = true;
    if let (false, Some(wait)) = (tools.is_empty(), wait) {
        let before = text.lines().count();
        let told = fs::OpenOptions::new().append(true).open(file).and_then(|mut f| std::io::Write::write_all(&mut f, ended.as_bytes()));
        let until = Instant::now() + wait;
        answered = false;
        while told.is_ok() && !answered && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(25));
            let now = fs::read_to_string(file).unwrap_or_default();
            answered = now.lines().skip(before).filter_map(|l| serde_json::from_str::<Value>(l).ok()).any(|l| l["dropped"].is_array());
        }
    }
    let _ = fs::write(file, if answered { "" } else { ended });
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
        // the tool server's side: it sees the app's line and answers
        let server = {
            let f = f.clone();
            std::thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(5);
                while Instant::now() < until {
                    if fs::read_to_string(&f).unwrap_or_default().contains("\"turn\": \"ended\"") {
                        let mut file = fs::OpenOptions::new().append(true).open(&f).unwrap();
                        std::io::Write::write_all(&mut file, b"{\"dropped\": [\"job-a\"]}\n").unwrap();
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
    fn a_server_that_does_not_answer_is_left_the_line_to_act_on() {
        let f = notes("jobs-silent");
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n").unwrap();
        assert_eq!(end_turn(&f, Some(Duration::from_millis(150))), ["rbx_blender"]);
        assert_eq!(fs::read_to_string(&f).unwrap(), "{\"turn\": \"ended\"}\n", "shorter than before, so the server reads it from the top");
        assert!(end_turn(&f, Some(Duration::from_millis(150))).is_empty(), "not said twice");
        assert_eq!(fs::read_to_string(&f).unwrap(), "{\"turn\": \"ended\"}\n", "and the line stays until the server has read it");
        // jobs the server dropped late are not counted again either
        fs::write(&f, "{\"job\": \"job-a\", \"tool\": \"rbx_blender\", \"at\": \"started\"}\n{\"dropped\": [\"job-a\"]}\n").unwrap();
        assert!(end_turn(&f, None).is_empty());
    }
}
