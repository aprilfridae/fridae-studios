//! Watching every known game's Studio files, so the window's Review tab follows the run as it happens: a review
//! item file that is new or changed is a `review` event {project, item}, and a changed studio.json is a `studio_run`
//! event {project, run}. Files are noted by (path, time of last change), as promo.rs does with its cards.
//!
//! What is there when the app starts (or when a project first appears) is not reported. The app's own decisions on
//! items are noted as they are written (studio_run::SEEN), so they are not reported either; studio.json changes always
//! are, whoever made them (the "slots" note, a pause). A file that does not read yet is tried again next time.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

use serde_json::{Value, json};

use crate::{
    agents::BUS,
    projects,
    studio_run::{SEEN, item_files, mtime, read_obj, run_file},
};

/// What changed in `projects` since the last look, as (event kind, data). `looked` holds the projects seen before.
pub(crate) fn changes(projects: &[String], seen: &mut HashMap<PathBuf, SystemTime>, looked: &mut HashSet<String>) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for project in projects {
        let first = looked.insert(project.clone());
        let dir = Path::new(project);
        let mut files: Vec<(PathBuf, bool)> = item_files(dir).into_iter().map(|(_, p)| (p, false)).collect();
        files.push((run_file(dir), true));
        for (file, is_run) in files {
            let Some(m) = mtime(&file) else { continue };
            if seen.get(&file) == Some(&m) {
                continue;
            }
            if first {
                seen.insert(file, m);
                continue;
            }
            let Some(v) = read_obj(&file).filter(|v| is_run || v["id"].is_string()) else { continue };
            seen.insert(file, m);
            out.push(if is_run { ("studio_run", json!({"project": project, "run": v})) } else { ("review", json!({"project": project, "item": v})) });
        }
    }
    out
}

pub fn watch() {
    let mut looked = HashSet::new();
    loop {
        let known: Vec<String> = projects::list().iter().filter_map(|p| p["path"].as_str().map(str::to_string)).collect();
        let events = changes(&known, &mut SEEN.lock().unwrap(), &mut looked);
        for (kind, data) in events {
            BUS.put(kind, data);
        }
        thread::sleep(Duration::from_millis(700));
    }
}
