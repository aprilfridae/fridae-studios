//! The user's notes folders for Setup's "Your notes" row: how many notes and house rules there are, and opening the
//! folder. The notes themselves are written by the agents' tool (rbxnotes.py); these are the same two folders:
//! `<home>/notes` for everything and `<game>/.fridae/notes` for one game.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use crate::{paths::PATHS, projects};

const RULES_FILE: &str = "House rules.md";

pub fn all_dir(home: &Path) -> PathBuf {
    home.join("notes")
}

pub fn game_dir(project: &Path) -> PathBuf {
    project.join(".fridae").join("notes")
}

/// (notes, house rules) in one folder: Markdown files other than House rules.md and hidden ones, and the bullets of
/// House rules.md.
pub fn counts(dir: &Path) -> (usize, usize) {
    let notes = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".md") && !name.starts_with('.') && name != RULES_FILE && e.path().is_file()
        })
        .count();
    let rules = fs::read_to_string(dir.join(RULES_FILE))
        .map(|t| t.lines().filter(|l| l.trim_start().starts_with("- ") || l.trim_start().starts_with("* ")).count())
        .unwrap_or(0);
    (notes, rules)
}

fn one(dir: PathBuf) -> Value {
    let (notes, rules) = counts(&dir);
    json!({"notes": notes, "rules": rules, "folder": dir.display().to_string()})
}

/// GET /api/notes?path=<project>: the counts for everything and, for a known project, for that game.
pub fn info(project: &str) -> Value {
    let game = (!project.is_empty() && projects::known(project)).then(|| one(game_dir(Path::new(project))));
    json!({"all": one(all_dir(&PATHS.home)), "game": game})
}

/// The folder a scope opens: only the home notes folder or a known project's, never a path from the request.
pub fn folder_for(scope: &str, project: &str, home: &Path, known: impl Fn(&str) -> bool) -> Option<PathBuf> {
    match scope {
        "all" => Some(all_dir(home)),
        "game" if !project.is_empty() && known(project) && Path::new(project).is_dir() => Some(game_dir(Path::new(project))),
        _ => None,
    }
}

/// POST /api/notes-open {scope, path}: show the folder in Finder or Explorer, creating it if it is missing.
pub fn open(scope: &str, project: &str) -> bool {
    let Some(dir) = folder_for(scope, project, &PATHS.home, projects::known) else { return false };
    fs::create_dir_all(&dir).is_ok() && crate::paths::open_that(&dir).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_notes_and_rules() {
        let dir = std::env::temp_dir().join(format!("fridae notes é {}", std::process::id()));
        assert_eq!(counts(&dir), (0, 0), "a missing folder is empty");
        fs::create_dir_all(dir.join(".trash")).unwrap();
        fs::write(dir.join("Menu motion.md"), "x").unwrap();
        fs::write(dir.join("Café palette.md"), "x").unwrap();
        fs::write(dir.join(".tmp-1.md"), "x").unwrap();
        fs::write(dir.join(".trash").join("Old.md"), "x").unwrap();
        fs::write(dir.join(RULES_FILE), "# House rules\n\n- one\n- two\n* three\nnot a rule\n").unwrap();
        assert_eq!(counts(&dir), (2, 3));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_known_folders_open() {
        let home = Path::new("/h/Fridae Studios");
        let tmp = std::env::temp_dir();
        let project = tmp.to_str().unwrap();
        assert_eq!(folder_for("all", "", home, |_| false), Some(home.join("notes")));
        assert_eq!(folder_for("game", project, home, |_| true), Some(tmp.join(".fridae").join("notes")));
        assert_eq!(folder_for("game", project, home, |_| false), None, "a project the app does not know");
        assert_eq!(folder_for("game", "", home, |_| true), None);
        assert_eq!(folder_for("../etc", project, home, |_| true), None);
    }
}
