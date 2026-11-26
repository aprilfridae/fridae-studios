//! The board as a live thread: a member sees new posts for it at its next step, mid-turn, not as a message when its
//! turn ends.
//!
//!   WORK/swarm/<run>/seen/<agent id>   the time ("t") of the newest post the member has been shown: written by
//!                                      whatever showed it (the hook below, `rbxteam.py`'s tool results and board
//!                                      reads, the app when it resumes a parked member); only ever moved forward
//!   WORK/swarm/steer/<agent id>.jsonl  what the user wrote to a busy Lead: {"t", "text"} per line, taken whole (renamed
//!                                      away, then read) by the hook at the Lead's next step, else by the app when the
//!                                      turn ends (`swarm::steer_left`)
//!
//! How each AI is shown them:
//! - Claude Code: a `PostToolUse` hook in the session's `--settings` (`claude_settings`) prints
//!   `{"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "..."}}` (the shape Claude Code
//!   2.1.326 builds for its own PostToolUse context).
//! - Gemini (agy): `<workspace>/.agents/hooks.json` (`antigravity::write_hooks`) runs the same hook as `PreInvocation`,
//!   before each model call, which prints `{"injectSteps": [{"systemMessage": {"systemMessage": "..."}}]}`.
//! - Every AI (Codex, OpenCode, and the two above when a tool of ours ran first): the rbxkit tool server appends the
//!   same lines to its tool results (`rbxteam.py`, `unseen_note`).
//!
//! The hook is this program itself (`fridae --board-hook <ai> <agent id>`, `hook_main`): it starts in milliseconds
//! and, built for Windows' window subsystem, opens no console window. The tool server's Python (`fridae-cli.exe`)
//! measured 360 to 570 ms to start on Windows, too slow for every step.

use super::*;
use std::io::Read;

/// Most posts shown at once; the rest are left to `rbx_team` 'board'.
const SHOWN: usize = 12;
/// Characters of one post shown.
const SHOWN_CHARS: usize = 600;

/// An agent id as the app makes them: letters, digits, '-' and '_' only (it names a file).
fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn seen_file(run: &str, agent: &str) -> Option<PathBuf> {
    (plain(run) && plain(agent)).then(|| root().join(run).join("seen").join(agent))
}

/// The newest post time this member has been shown (0 when never).
pub fn seen(run: &str, agent: &str) -> f64 {
    seen_file(run, agent).and_then(|f| fs::read_to_string(f).ok()).and_then(|s| s.trim().parse::<f64>().ok()).filter(|t| t.is_finite()).unwrap_or(0.0)
}

/// The member has been shown every post up to `t` (never moved back).
pub fn mark_seen(run: &str, agent: &str, t: f64) {
    let Some(file) = seen_file(run, agent) else { return };
    if !(t > seen(run, agent)) {
        return;
    }
    let _ = fs::create_dir_all(file.parent().unwrap());
    let tmp = file.with_extension(format!("{}.tmp", paths::random_hex(1)));
    if fs::write(&tmp, format!("{t}")).is_err() || fs::rename(&tmp, &file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Whether a post is for this reader: the Lead hears every post but its own; a member, those naming it, its role or
/// everyone (`mentions`), not its own.
pub fn for_reader(post: &Value, name: &str, role: &str, is_lead: bool) -> bool {
    if post["from"].as_str() == Some(name) {
        return false;
    }
    if is_lead {
        return true;
    }
    let to: Vec<String> = post["to"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_lowercase).collect();
    mentions(&to, name, role)
}

fn steer_file(agent: &str) -> Option<PathBuf> {
    plain(agent).then(|| root().join("steer").join(format!("{agent}.jsonl")))
}

/// Keep a message the user wrote to a busy Lead for its next step.
pub fn put_steer(agent: &str, text: &str) -> bool {
    let Some(file) = steer_file(agent) else { return false };
    let _ = fs::create_dir_all(file.parent().unwrap());
    let line = format!("{}\n", json!({"t": paths::now_secs(), "text": text}));
    fs::OpenOptions::new().create(true).append(true).open(&file).and_then(|mut f| f.write_all(line.as_bytes())).is_ok()
}

/// Take what the user wrote to this Lead (oldest first), so it is shown once: the file is renamed away first, and
/// whoever renames it has it (the hook, or the app at the turn's end).
pub fn take_steer(agent: &str) -> Vec<String> {
    let Some(file) = steer_file(agent) else { return Vec::new() };
    if !file.exists() {
        return Vec::new();
    }
    let mine = file.with_extension(format!("{}.taking", paths::random_hex(1)));
    if fs::rename(&file, &mine).is_err() {
        return Vec::new();
    }
    let text = fs::read_to_string(&mine).unwrap_or_default();
    let _ = fs::remove_file(&mine);
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| v["text"].as_str().map(str::to_string)).filter(|t| !t.trim().is_empty()).collect()
}

/// This chat in a swarm as the app's index has it (the hook runs in a process of its own, where `RUNS` is empty):
/// (run, name, role, is_lead).
fn in_index(agent: &str) -> Option<(String, String, String, bool)> {
    let index: Value = serde_json::from_str(&fs::read_to_string(index_file()).ok()?).ok()?;
    let e = index.get(agent)?;
    let run = e["run"].as_str().filter(|r| plain(r))?.to_string();
    Some((run, e["name"].as_str().unwrap_or("").to_string(), e["role"].as_str().unwrap_or("").to_string(), e["is_lead"] == true))
}

fn post_line(p: &Value) -> String {
    let text: String = p["text"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.chars().count() > SHOWN_CHARS { format!("{} ...", text.chars().take(SHOWN_CHARS).collect::<String>()) } else { text };
    let to: Vec<&str> = p["to"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    format!("@{} -> {} ({}): {}", p["from"].as_str().unwrap_or("?"), if to.is_empty() { "board".to_string() } else { to.join(" ") }, p["kind"].as_str().unwrap_or("msg"), text)
}

/// What this chat has not been shown yet: what the user wrote to it while it worked, then new posts for it. ""
/// when nothing. Marks them shown.
pub fn unseen_text(agent: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let steer = take_steer(agent);
    if !steer.is_empty() {
        out.push("THE USER WROTE TO YOU while you were working (this is the user, not a board post). Answer them now in a line or two and do what they ask; it comes before the task you were on:".into());
        out.extend(steer.iter().map(|t| format!("> {}", t.trim().replace('\n', "\n> "))));
    }
    if let Some((run, name, role, is_lead)) = in_index(agent) {
        let since = seen(&run, agent);
        let me = if is_lead { "lead" } else { name.as_str() };
        let new: Vec<Value> = read_board(&run).into_iter().filter(|p| p["t"].as_f64().unwrap_or(0.0) > since && for_reader(p, me, &role, is_lead)).collect();
        if let Some(last) = new.iter().filter_map(|p| p["t"].as_f64()).reduce(f64::max) {
            if !out.is_empty() {
                out.push(String::new());
            }
            let left = new.len().saturating_sub(SHOWN);
            out.push(format!("New on the swarm's board for you (the live thread; no need to read the board for these{}):", if left > 0 { format!(", and {left} older ones are on the board") } else { String::new() }));
            out.extend(new[left..].iter().map(post_line));
            out.push("Answer with rbx_team action 'post' only when one needs an answer, then carry on.".into());
            mark_seen(&run, agent, last);
        }
    }
    out.join("\n")
}

/// `fridae --board-hook <claude|gemini> <agent id>`: run by the AI before its next step. Prints the AI's hook output:
/// the unseen lines as context, or `{}`. Always ends well: a hook that fails must never stop the agent.
pub fn hook_main(args: &[String]) -> i32 {
    // what the AI sends on stdin is not needed, but is read so it never finds the pipe closed
    let mut sink = Vec::new();
    let _ = std::io::stdin().take(1 << 20).read_to_end(&mut sink);
    let (ai, agent) = (args.first().map(String::as_str).unwrap_or(""), args.get(1).map(String::as_str).unwrap_or(""));
    let text = if plain(agent) { unseen_text(agent) } else { String::new() };
    println!("{}", hook_output(ai, &text));
    0
}

/// The hook's output for each AI (`{}` when there is nothing to show).
pub fn hook_output(ai: &str, text: &str) -> Value {
    if text.is_empty() {
        return json!({});
    }
    let text = format!("[Fridae Studios, live]\n{text}");
    match ai {
        "gemini" => json!({"injectSteps": [{"systemMessage": {"systemMessage": text}}]}),
        _ => json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": text}}),
    }
}

/// The hook's command line for a chat: this program, quoted (its folder may have a space). For Claude Code with '/'
/// (it may run hooks through Git Bash on Windows); for agy as Windows names it (`cmd /c`).
pub fn hook_command(ai: &str, agent: &str) -> Option<String> {
    if !plain(agent) {
        return None;
    }
    let exe = std::env::current_exe().ok()?.display().to_string();
    let exe = if ai == "claude" { exe.replace('\\', "/") } else { exe };
    Some(format!("\"{exe}\" --board-hook {ai} {agent}"))
}

/// The `--settings` Claude Code is given for a chat: the board hook after every tool call.
pub fn claude_settings(agent: &str) -> Option<String> {
    let cmd = hook_command("claude", agent)?;
    Some(json!({"hooks": {"PostToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 10}]}]}}).to_string())
}

/// The entry Fridae keeps in a Gemini workspace's `.agents/hooks.json`: the board hook before each model call.
pub fn gemini_hook(agent: &str) -> Option<Value> {
    let cmd = hook_command("gemini", agent)?;
    Some(json!({"PreInvocation": [{"type": "command", "command": cmd, "timeout": 10}]}))
}
