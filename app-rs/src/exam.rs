//! `fridae --librarian-exam <mode>`: the knowledge exam's way into the real Librarian (tests/scale_bench/exam.py).
//! Hidden: nothing in the app starts it. It runs the app's own code, not a copy: `swarm::librarian::answer` with
//! the real writer and support check (`compact::librarian_answer` / `librarian_check`, the user's OpenCode plan
//! through the app's own OpenCode server and sign-in, `opencode::write_only`). Only the search is the exam's: each
//! job brings the passages its retrieval method found for the question's parts.
//!
//!   split          stdin {"questions": [..]}  ->  stdout {"parts": [[part, ..], ..]}  (`librarian::split_parts`)
//!   answer [--jobs N] [--variant V]
//!                  stdin: one job per line {"id", "question", "parts": [{"question", "hits": [{shelf, title,
//!                  heading, text}]}]}; stdout: one result per line, as each finishes (N at a time, 4 at most)
//!   probe          what the app's OpenCode server says about the writer models and their variants (read only)
//!   reap           end what a killed exam run left (the process ledger in RBXKIT_WORK, as the app does at start)
//!
//! Each result keeps everything the exam's failure analysis needs: the writer's raw text, the claims parsed from it,
//! the ones whose quotes stand in their passages, the support check's prompt and answer, the posted answer, and per
//! model call its model, time, tokens (reasoning apart) and when its first events came.

use std::{
    io::{BufRead, Read, Write},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Instant,
};

use serde_json::{Value, json};

use crate::swarm::librarian::{self, Hit, Part};

const MAX_JOBS: usize = 4;

pub fn main(args: &[String]) -> i32 {
    let mode = args.first().map(String::as_str).unwrap_or("");
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    match mode {
        "split" => {
            let input = read_stdin();
            let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
            let parts: Vec<Value> = v["questions"].as_array().into_iter().flatten().map(|q| json!(librarian::split_parts(q.as_str().unwrap_or("")))).collect();
            println!("{}", json!({"parts": parts}));
            0
        }
        "reap" => {
            let n = crate::procs::reap_orphans();
            println!("{}", json!({"reaped": n}));
            0
        }
        "probe" => {
            crate::procs::reap_orphans();
            let ids = writers_now();
            let out = crate::opencode::exam_probe(&ids);
            println!("{}", json!({"writers": ids, "probe": out}));
            crate::opencode::shut_down();
            0
        }
        "answer" => {
            let jobs: usize = flag("--jobs").and_then(|j| j.parse().ok()).unwrap_or(MAX_JOBS).clamp(1, MAX_JOBS);
            if let Some(v) = flag("--variant").filter(|v| !v.is_empty() && v != "default") {
                *crate::opencode::EXAM_VARIANT.lock().unwrap() = Some(v);
            }
            crate::procs::reap_orphans();
            crate::opencode::EXAM_TAP.store(true, Ordering::SeqCst);
            let writers = writers_now();
            eprintln!("{}", json!({"writers": writers}));
            answer_all(jobs);
            crate::opencode::shut_down();
            0
        }
        _ => {
            eprintln!("usage: fridae --librarian-exam split|answer [--jobs N] [--variant V]|probe|reap");
            2
        }
    }
}

fn read_stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s
}

/// The writers the app would use now: OpenCode's model list asked for as the app does at start (waited for up to a
/// minute), then `opencode::writers`.
fn writers_now() -> Vec<String> {
    crate::opencode::load_models();
    let began = Instant::now();
    while crate::opencode::models().is_empty() && began.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    crate::opencode::writers()
}

fn answer_all(jobs: usize) {
    let lines: Vec<String> = std::io::stdin().lock().lines().map_while(Result::ok).filter(|l| !l.trim().is_empty()).collect();
    let queue = Arc::new(Mutex::new(lines.into_iter().collect::<std::collections::VecDeque<String>>()));
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let workers: Vec<_> = (0..jobs)
        .map(|_| {
            let (queue, out) = (queue.clone(), out.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(line) = queue.lock().unwrap().pop_front() else { break };
                    let result = match serde_json::from_str::<Value>(&line) {
                        Ok(job) => one(&job),
                        Err(e) => json!({"error": format!("bad job: {e}")}),
                    };
                    let mut o = out.lock().unwrap();
                    let _ = writeln!(o, "{result}");
                    let _ = o.flush();
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
}

fn parts_of(job: &Value) -> Vec<Part> {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    job["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Part {
            question: s(p, "question"),
            hits: p["hits"].as_array().into_iter().flatten().map(|h| Hit { shelf: s(h, "shelf"), title: s(h, "title"), heading: s(h, "heading"), text: s(h, "text") }).collect(),
        })
        .collect()
}

fn claims_json(claims: &[librarian::Claim]) -> Value {
    json!(claims.iter().map(|c| json!({"part": c.part + 1, "claim": c.claim, "quote": c.quote, "note": c.note})).collect::<Vec<_>>())
}

/// One question through `librarian::answer`, with the writer and the check recorded on their way.
fn one(job: &Value) -> Value {
    let id = job["id"].clone();
    let question = job["question"].as_str().unwrap_or("").to_string();
    let given = parts_of(job);
    let asked_seen: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let calls: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    let raw: Mutex<Option<String>> = Mutex::new(None);
    let checked: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);
    let search = |asked: &[String]| -> Result<Vec<Part>, String> {
        *asked_seen.lock().unwrap() = asked.to_vec();
        // the exam searched the same parts (it asked `split` first); a part it did not bring has no passages
        Ok(asked.iter().enumerate().map(|(i, q)| Part { question: q.clone(), hits: given.get(i).map(|p| p.hits.clone()).unwrap_or_default() }).collect())
    };
    let record = |what: &str, prompt: &str, began: Instant, o: &crate::compact::Outcome| {
        let s = o.summary.as_ref();
        calls.lock().unwrap().push(json!({
            "what": what,
            "secs": began.elapsed().as_secs_f64(),
            "prompt_chars": prompt.chars().count(),
            "model": s.map(|s| s.model.clone()),
            "usage": s.map(|s| json!({"input": s.usage.input, "output": s.usage.output, "cache_read": s.usage.cache_read})),
            "failed": o.failed.iter().map(|(m, why)| json!({"model": m, "why": why})).collect::<Vec<_>>(),
            "tap": crate::opencode::exam_stats(prompt),
        }));
    };
    let write = |prompt: &str| -> Option<String> {
        let began = Instant::now();
        let o = crate::compact::librarian_answer(prompt, &|| false);
        record("write", prompt, began, &o);
        let text = o.summary.map(|s| s.text);
        *raw.lock().unwrap() = text.clone();
        text
    };
    let check = |prompt: &str| -> Option<String> {
        let began = Instant::now();
        let o = crate::compact::librarian_check(prompt, &|| false);
        record("check", prompt, began, &o);
        let text = o.summary.map(|s| s.text);
        *checked.lock().unwrap() = Some((prompt.to_string(), text.clone()));
        text
    };
    let began = Instant::now();
    let posted = librarian::answer(&question, &search, &write, &check);
    let secs = began.elapsed().as_secs_f64();
    let asked = asked_seen.lock().unwrap().clone();
    let parts: Vec<Part> = asked.iter().enumerate().map(|(i, q)| Part { question: q.clone(), hits: given.get(i).map(|p| p.hits.clone()).unwrap_or_default() }).collect();
    let raw = raw.lock().unwrap().clone();
    let (claims, kept, dropped) = match raw.as_deref().and_then(|r| librarian::parse_claims(r, parts.len())) {
        Some(c) => {
            let (k, d) = librarian::supported(&parts, c.clone());
            (Some(c), Some(k), d)
        }
        None => (None, None, 0),
    };
    let declined = raw.as_deref().map(|r| librarian::declined(r, parts.len())).unwrap_or_else(|| vec![false; parts.len()]);
    let check = checked.lock().unwrap().clone();
    let given_parts: Vec<String> = given.iter().map(|p| p.question.clone()).collect();
    json!({
        "id": id,
        "question": question,
        "asked": asked,
        "parts_match": given_parts == asked,
        "answer": posted,
        "secs": secs,
        "writer_raw": raw,
        "format_ok": claims.is_some() || declined.iter().any(|d| *d),
        "declined": declined,
        "claims_raw": claims.as_deref().map(claims_json),
        "claims_quoted": kept.as_deref().map(claims_json),
        "dropped_by_quote": dropped,
        "check_prompt": check.as_ref().map(|c| c.0.clone()),
        "check_raw": check.as_ref().and_then(|c| c.1.clone()),
        "check_noes": check.as_ref().and_then(|c| c.1.as_deref()).map(librarian::check_noes),
        "calls": calls.into_inner().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_reads_into_parts() {
        let job = json!({"id": "P01", "question": "q", "parts": [{"question": "a", "hits": [{"shelf": "vault", "title": "T", "heading": "H", "text": "x y z"}]}, {"question": "b", "hits": []}]});
        let p = parts_of(&job);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].hits[0], Hit { shelf: "vault".into(), title: "T".into(), heading: "H".into(), text: "x y z".into() });
        assert!(p[1].hits.is_empty());
    }

    #[test]
    fn a_job_runs_the_real_answer_path_and_records_the_writer() {
        crate::compact::STAND_IN.with(|s| *s.borrow_mut() = Some((crate::compact::writes("{\"part\": 1, \"claim\": \"Prompts go off.\", \"quote\": \"Disable ProximityPrompts on the client\", \"note\": \"Drags\"}"), crate::compact::LIMITS)));
        let job = json!({"id": "X", "question": "how do touch drags and prompts get along?", "parts": [{"question": "how do touch drags and prompts get along", "hits": [{"shelf": "vault", "title": "Drags", "heading": "", "text": "Disable ProximityPrompts on the client while a drag is held."}]}]});
        let r = one(&job);
        assert_eq!(r["parts_match"], true, "{r}");
        assert_eq!(r["format_ok"], true, "{r}");
        assert!(r["answer"].as_str().unwrap().contains("Prompts go off."), "{r}");
        assert_eq!(r["claims_quoted"].as_array().unwrap().len(), 1);
        assert_eq!(r["calls"][0]["what"], "write");
    }

    #[test]
    fn a_writer_that_declines_is_no_format_failure() {
        crate::compact::STAND_IN.with(|s| *s.borrow_mut() = Some((crate::compact::writes("{\"part\": 1, \"not_found\": true}"), crate::compact::LIMITS)));
        let job = json!({"id": "N01", "question": "how do I turn on Nanite in Unreal Engine 5?", "parts": [{"question": "how do I turn on Nanite in Unreal Engine 5", "hits": [{"shelf": "vault", "title": "Foliage", "heading": "", "text": "Kits over one-offs."}]}]});
        let r = one(&job);
        assert_eq!(r["format_ok"], true, "{r}");
        assert_eq!(r["declined"], json!([true]), "{r}");
        assert!(r["answer"].as_str().unwrap().contains(librarian::NOT_FOUND), "{r}");
    }
}
