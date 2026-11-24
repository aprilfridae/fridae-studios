//! The app as git's askpass helper (github.rs): run as git runs it, with the prompt as its one argument and
//! "<port>:<one-time code>" in FRIDAE_GIT_ASKPASS. It trades the code once with whoever listens on that port (here a
//! stand-in for the app) and never reads a file. (This test also makes cargo build the app for github.rs's push tests.)

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::Command,
};

#[allow(clippy::disallowed_methods)]
fn ask(spec: &str, prompt: &str) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_fridae")).arg(prompt).env("FRIDAE_GIT_ASKPASS", spec).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string())
}

#[test]
fn the_app_answers_gits_prompts_by_trading_its_one_time_code() {
    let code = "0123456789abcdef0123456789abcdef";
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = std::thread::spawn(move || {
        // the app's side: one connection, the code checked, the token given
        let (mut s, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        if line.trim() == code {
            s.write_all(b"gho_abc123\n").unwrap();
        }
    });
    let spec = format!("{port}:{code}");
    assert_eq!(ask(&spec, "Username for 'https://github.com': "), (true, "x-access-token\n".to_string()));
    assert_eq!(ask(&spec, "Password for 'https://x-access-token@github.com': "), (true, "gho_abc123\n".to_string()));
    app.join().unwrap();
    assert_eq!(ask(&spec, "Password: "), (false, String::new()), "nobody listens any more: nothing");
    assert_eq!(ask(&spec, "Something else? "), (false, String::new()));
    // a path is no way to anything: the helper never reads a file
    let file = std::env::temp_dir().join(format!("fridae-askpass-{}", std::process::id())).join("github").join("token");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "gho_secret").unwrap();
    assert_eq!(ask(&file.display().to_string(), "Password: "), (false, String::new()));
    let _ = std::fs::remove_dir_all(file.parent().unwrap().parent().unwrap());
}
