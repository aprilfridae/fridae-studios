//! Fridae Studios: a chat window that runs an AI agent with the rbxkit and
//! Roblox Studio tools attached.
//!
//! The window is a local web page (ui/index.html) shown in a native WebView2
//! window. It talks to this process over 127.0.0.1 with a per-run secret
//! token. The tools themselves (Blender, textures, the MCP server) are the
//! Python programs beside this one: `fridae-cli.exe --mcp` and `--rbx`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod antigravity;
mod attach;
mod board;
mod codex;
mod cursor;
mod expert_ai;
mod fridae_lock;
mod limits;
mod media;
mod notes;
mod paths;
mod plugin;
mod projects;
mod promo;
mod promo_tab;
mod research;
mod server;
mod splash;
mod studio_run;
mod studio_undo;
mod studio_watch;
mod team;
mod textures;
mod update;
mod usage;

use std::{env, fs, thread, time::Duration};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Icon, Theme, WindowBuilder},
};
use wry::{WebContext, WebViewBuilder};

use paths::{APP, PATHS, VERSION};

fn window_icon() -> Option<Icon> {
    let img = image::open(PATHS.kit.join("ui").join("fridae.ico")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn find_browser() -> Option<std::path::PathBuf> {
    let local = env::var("LOCALAPPDATA").unwrap_or_default();
    [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".to_string(),
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".to_string(),
        r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
        format!(r"{local}\Google\Chrome\Application\chrome.exe"),
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|p| p.exists())
    .or_else(|| paths::which("msedge"))
    .or_else(|| paths::which("chrome"))
}

/// Match the title bar to the page (Windows 11).
#[cfg(windows)]
fn dark_caption(hwnd: isize) {
    #[link(name = "dwmapi", kind = "raw-dylib")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const core::ffi::c_void, size: u32) -> i32;
    }
    let on: i32 = 1;
    let color: u32 = 0x000D_0C0C; // BGR of #0c0c0d
    unsafe {
        for attr in [20, 19] {
            DwmSetWindowAttribute(hwnd, attr, (&raw const on).cast(), 4);
        }
        DwmSetWindowAttribute(hwnd, 35, (&raw const color).cast(), 4);
    }
}

/// Developer aid: send one message with no window, write the events to a file, and quit.
fn self_test(message: &str) {
    let var = |k: &str| env::var(k).ok().filter(|v| !v.is_empty());
    let who = var("RBXKIT_OPEN").and_then(|id| agents::open_chat(&id)).or_else(|| agents::agents().first().cloned());
    let Some(who) = who else { return };
    if let Some(p) = var("RBXKIT_PROJECT") {
        who.set_project(&p);
    }
    thread::spawn(team::watch);
    let mut atts = Vec::new();
    if let Some(path) = var("RBXKIT_ATTACH") {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(fs::read(&path).unwrap_or_default());
        if let Some(got) = attach::save_attachment(&data, Some(&path)) {
            atts.push(got["name"].as_str().unwrap_or("").to_string());
        }
    }
    let ask = agents::Ask { selftest: var("RBXKIT_SELFTEST_MODE").is_some(), model: var("RBXKIT_MODEL"), attachments: atts, kickoff: var("RBXKIT_KICKOFF"), ..Default::default() };
    who.send(message, &ask);
    thread::sleep(Duration::from_secs(2));
    while who.busy() {
        thread::sleep(Duration::from_millis(500));
    }
    thread::sleep(Duration::from_secs(1));
    let out = paths::work("selftest");
    let _ = fs::create_dir_all(&out);
    let _ = fs::write(out.join("events.json"), serde_json::to_string_pretty(&agents::BUS.after(0, 0.0)).unwrap_or_default());
    agents::shut_down();
}

/// Apps opened from Finder get a bare PATH; add the places Claude Code, Homebrew and Blender's wrapper live.
#[cfg(target_os = "macos")]
fn widen_path() {
    let home = paths::home_dir();
    let mut dirs = vec![home.join(".local").join("bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    dirs.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    if let Ok(p) = env::join_paths(dirs) {
        // SAFETY: called first thing in main, before any other thread starts
        unsafe { env::set_var("PATH", p) };
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    widen_path();
    // installing an update: this copy is the new version, run from the download folder (see splash.rs)
    let args: Vec<String> = env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--apply-update") {
        return splash::run(&args[i + 1..]);
    }
    let _ = fs::create_dir_all(&PATHS.work);
    if env::args().any(|a| a == "--version") {
        let _ = fs::write(paths::work("version.txt"), VERSION);
        return;
    }
    let testing = env::var("RBXKIT_SELFTEST").ok().filter(|v| !v.is_empty());
    agents::new_agent(None);
    agents::keep_awake();
    thread::spawn(usage::tidy);
    if let Some(message) = testing {
        return self_test(&message);
    }
    agents::restore_open();

    // an update that finished downloading last time goes in before the window opens
    if paths::packaged() {
        match update::staged_version() {
            Some(staged) if update::newer(&staged, VERSION) && update::state()["auto"] == true => {
                if update::apply(true) {
                    return;
                }
            }
            Some(_) => update::clear_stage_later(),
            None => {}
        }
        thread::spawn(update::run_forever);
    }
    let url = server::start(paths::random_hex(3));

    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let icon = window_icon();
    let window = WindowBuilder::new()
        .with_title(APP)
        .with_inner_size(LogicalSize::new(1180.0, 820.0))
        .with_min_inner_size(LogicalSize::new(520.0, 480.0))
        .with_theme(Some(Theme::Dark))
        .with_window_icon(icon)
        .build(&event_loop)
        .expect("could not create the window");
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        dark_caption(window.hwnd() as isize);
    }
    let data = if paths::packaged() { PATHS.home.join("webview") } else { paths::work("webview") };
    let mut context = WebContext::new(Some(data));
    let builder = WebViewBuilder::new_with_web_context(&mut context).with_background_color((12, 12, 13, 255)).with_url(&url);
    // videos and music dropped on the window are used by path, never uploaded; other drops go to the page as before.
    // Not on Windows: there wry's handler replaces WebView2's own, which would stop cards dragging on the board, so a
    // dropped video reaches the page, which points to Add clips instead.
    #[cfg(not(windows))]
    let builder = builder.with_drag_drop_handler(|e| match e {
        wry::DragDropEvent::Drop { paths, .. } => promo::dropped(paths),
        _ => false,
    });
    let built = builder.build(&window);
    let _webview = match built {
        Ok(view) => view,
        Err(_) => {
            // no WebView2 on this computer: show the page in Edge or Chrome instead, or in the default browser
            window.set_visible(false);
            match find_browser() {
                Some(browser) => {
                    let profile = paths::work("ui-profile");
                    let _ = std::process::Command::new(browser)
                        .arg(format!("--app={url}"))
                        .arg("--window-size=1180,820")
                        .arg(format!("--user-data-dir={}", profile.display()))
                        .status();
                }
                None => {
                    let _ = open::that(&url);
                    loop {
                        thread::sleep(Duration::from_secs(3600));
                    }
                }
            }
            agents::shut_down();
            return;
        }
    };

    // developer aids: say where the page is, and close after a while through the normal closing path
    if let Ok(path) = env::var("FRIDAE_URL_FILE") {
        let _ = fs::write(path, &url);
    }
    if let Some(secs) = env::var("FRIDAE_QUIT_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
        let proxy = event_loop.create_proxy();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(secs));
            let _ = proxy.send_event(());
        });
    }
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(()) => {
                agents::shut_down();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
