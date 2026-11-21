//! The per-project lock for Studio's files: `<project>/.fridae/.lock`, the same one `rbxstage.locked` takes in Python.
//! Created exclusively holding the pid and time, retried for about 5 seconds, taken over when older than 10 seconds,
//! and removed when the guard is dropped. Not re-entrant.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

pub const WAIT: Duration = Duration::from_secs(5);
pub const STALE: Duration = Duration::from_secs(10);

pub struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn lock_path(project: &Path) -> PathBuf {
    project.join(".fridae").join(".lock")
}

/// An error from creating the lock file that means "someone has it (or is letting go of it)": try again.
fn is_held(kind: std::io::ErrorKind) -> bool {
    matches!(kind, std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied)
}

pub fn acquire(project: &Path) -> Result<Guard, String> {
    acquire_with(project, WAIT, STALE)
}

pub fn acquire_with(project: &Path, wait: Duration, stale: Duration) -> Result<Guard, String> {
    let path = lock_path(project);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not open the game's Studio folder: {e}"))?;
    }
    let deadline = Instant::now() + wait;
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                let _ = write!(f, "{} {:.6}", std::process::id(), now);
                return Ok(Guard(path));
            }
            // PermissionDenied: on Windows a lock file that is being deleted cannot be opened yet; wait like for a held one
            Err(e) if is_held(e.kind()) => {
                match fs::metadata(&path).and_then(|m| m.modified()) {
                    Ok(t) if t.elapsed().map(|age| age > stale).unwrap_or(false) => {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    Ok(_) => {}
                    Err(_) if e.kind() == std::io::ErrorKind::AlreadyExists => continue, // it vanished between the two calls: try again at once
                    Err(_) if Instant::now() > deadline => return Err(format!("Could not lock the game's Studio files: {e}")), // no lock file, still denied
                    Err(_) => {}
                }
                if Instant::now() > deadline {
                    return Err("Another Fridae Studios process is updating this game's Studio files. Try again in a moment.".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("Could not lock the game's Studio files: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fridae-lock-{name}-{}", std::process::id())).join("My Gäme");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lock_is_exclusive_and_removed_on_drop() {
        let p = project("excl");
        let g = acquire(&p).unwrap();
        assert!(lock_path(&p).exists());
        assert!(acquire_with(&p, Duration::from_millis(100), STALE).is_err());
        drop(g);
        assert!(!lock_path(&p).exists());
        assert!(acquire(&p).is_ok());
    }

    #[test]
    fn a_lock_pending_deletion_is_retried() {
        use std::io::ErrorKind;
        assert!(is_held(ErrorKind::AlreadyExists));
        assert!(is_held(ErrorKind::PermissionDenied), "Windows: a lock file pending deletion");
        assert!(!is_held(ErrorKind::NotFound));
        assert!(!is_held(ErrorKind::Other));
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_gives_up_after_the_wait() {
        use std::os::unix::fs::PermissionsExt;
        let p = project("denied");
        let dir = p.join(".fridae");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let r = acquire_with(&p, Duration::from_millis(100), STALE);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        if r.is_ok() {
            return; // running as root: nothing is denied
        }
        assert!(r.err().unwrap().starts_with("Could not lock"), "denied with no lock file: not taken for a held lock forever");
    }

    #[test]
    fn stale_lock_is_taken_over() {
        let p = project("stale");
        let _held = acquire(&p).unwrap();
        thread::sleep(Duration::from_millis(60));
        assert!(acquire_with(&p, Duration::from_millis(500), Duration::from_millis(30)).is_ok());
    }
}
