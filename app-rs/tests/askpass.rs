//! The app as git's askpass helper (github.rs): run as git runs it, with the prompt as its one argument and the
//! token file's path in FRIDAE_GIT_ASKPASS. (This test also makes cargo build the app for github.rs's push tests.)

use std::{fs, path::PathBuf, process::Command};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fridae-askpass-{}-{name}", std::process::id())).join("github");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[allow(clippy::disallowed_methods)]
fn ask(file: &PathBuf, prompt: &str) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_fridae")).arg(prompt).env("FRIDAE_GIT_ASKPASS", file).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string())
}

#[test]
fn the_app_answers_gits_prompts_from_its_private_token_file_only() {
    let dir = scratch("ok");
    let token = dir.join("token");
    fs::write(&token, "gho_abc123\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(ask(&token, "Username for 'https://github.com': "), (true, "x-access-token\n".to_string()));
    assert_eq!(ask(&token, "Password for 'https://x-access-token@github.com': "), (true, "gho_abc123\n".to_string()));
    assert_eq!(ask(&token, "Something else? "), (false, String::new()));
    // a file by any other name, or one others can read, is never printed
    let other = dir.join("notes.txt");
    fs::write(&other, "private").unwrap();
    assert_eq!(ask(&other, "Password: "), (false, String::new()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(ask(&token, "Password: "), (false, String::new()));
    }
    let _ = fs::remove_dir_all(dir.parent().unwrap());
}
