//! Fridae: a chat window that runs an AI agent with the rbxkit and
//! Roblox Studio tools attached.
//!
//! The window is a local web page (ui/index.html) shown in a native WebView2
//! window. It talks to this process over 127.0.0.1 with a per-run secret
//! token. The tools themselves (Blender, textures, the MCP server) are the
//! Python programs beside this one: `fridae-cli.exe --mcp` and `--rbx`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acp;
mod agents;
mod antigravity;
mod attach;
mod board;
mod caps;
mod claude_live;
mod codex;
mod exam;
mod cursor;
mod expert_ai;
mod github;
mod fridae_lock;
mod jobs;
mod limits;
mod live;
mod media;
mod notes;
mod accounts;
mod compact;
mod opencode;
mod signin;
mod skills;
mod paths;
mod plugin;
mod procs;
mod project_copy;
mod projects;
mod promo;
mod promo_tab;
mod rag;
mod remote_client;
mod remote_common;
mod remote_host;
#[cfg(all(test, any(windows, target_os = "macos")))]
mod remote_e2e_tests;
mod research;
mod server;
mod share;
mod splash;
mod studio_run;
mod studio_undo;
mod studio_watch;
mod swarm;
mod team;
mod textures;
mod update;
mod vault_git;
mod vault_inbox;
mod vault_sync;
mod usage;

use crate::paths::Start;
use std::{env, fs, thread, time::Duration};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Icon, ResizeDirection, Theme, WindowBuilder},
};
use wry::{WebContext, WebViewBuilder};

use paths::{APP, PATHS, VERSION};

/* What reaches the event loop from elsewhere: the end (a developer's timer), or a press on the window's own bar,
   which the page draws on Windows in place of the system's title bar (`window.ipc.postMessage("win:...")`). */
enum Ui {
    Quit,
    Drag,
    Minimize,
    Maximize,
    Close,
    ResizeTop,
    /// the Mac: put the window's buttons back where the page's top bar has room for them
    PlaceButtons,
}

/* The Mac's close, minimise and zoom buttons, moved into the page's top bar: 18 px from the left and in the middle of
   its 48 px height. AppKit puts them back at the top on its own now and then (a resize, leaving full screen), so this
   runs again then. tao's own with_traffic_light_inset does the same only when its view draws, which the page covering
   it rarely asks for. */
#[cfg(target_os = "macos")]
fn place_buttons(window: &tao::window::Window) {
    use objc2::{msg_send, runtime::AnyObject};
    use objc2_foundation::NSRect;
    use tao::platform::macos::WindowExtMacOS;
    const LEFT: f64 = 18.0;
    const BAR: f64 = 48.0;
    unsafe {
        let Some(win) = (window.ns_window() as *const AnyObject).as_ref() else { return };
        // NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton
        let buttons: Vec<*const AnyObject> = (0usize..3).map(|i| msg_send![win, standardWindowButton: i]).collect();
        let Some(close) = buttons[0].as_ref() else { return };
        let Some(mini) = buttons[1].as_ref() else { return };
        let parent: *const AnyObject = msg_send![close, superview];
        let Some(parent) = parent.as_ref() else { return };
        let bar: *const AnyObject = msg_send![parent, superview];
        let Some(bar) = bar.as_ref() else { return };
        let close_rect: NSRect = msg_send![close, frame];
        let mini_rect: NSRect = msg_send![mini, frame];
        let space = mini_rect.origin.x - close_rect.origin.x;
        // the bar's container grows down so the buttons, at its bottom, sit in the middle of the page's top bar
        let height = close_rect.size.height + (BAR - close_rect.size.height) / 2.0;
        let win_rect: NSRect = msg_send![win, frame];
        let mut rect: NSRect = msg_send![bar, frame];
        rect.size.height = height;
        rect.origin.y = win_rect.size.height - height;
        let _: () = msg_send![bar, setFrame: rect];
        for (i, b) in buttons.iter().enumerate() {
            let Some(b) = b.as_ref() else { continue };
            let mut r: NSRect = msg_send![b, frame];
            r.origin.x = LEFT + i as f64 * space;
            let _: () = msg_send![b, setFrameOrigin: r.origin];
        }
    }
}

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
    thread::spawn(swarm::watch); // the swarm's watchdog: app code, no tokens
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
    // started by git as its askpass helper for a GitHub push the user pressed (github.rs): answer and end
    if let Some(code) = github::askpass_main() {
        std::process::exit(code);
    }
    // run by an AI before its next step: what is new on its swarm's board (swarm/live.rs). First, so it starts fast
    {
        let args: Vec<String> = env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--board-hook") {
            std::process::exit(swarm::live::hook_main(&args[i + 1..]));
        }
    }
    #[cfg(target_os = "macos")]
    widen_path();
    // installing an update: this copy is the new version, run from the download folder (see splash.rs)
    let args: Vec<String> = env::args().collect();
    // the knowledge exam (tests/scale_bench/exam.py): the real Librarian on the exam's passages, no window (exam.rs)
    if let Some(i) = args.iter().position(|a| a == "--librarian-exam") {
        std::process::exit(exam::main(&args[i + 1..]));
    }
    if let Some(i) = args.iter().position(|a| a == "--apply-update") {
        return splash::run(&args[i + 1..]);
    }
    let _ = fs::create_dir_all(&PATHS.work);
    if env::args().any(|a| a == "--version") {
        let _ = fs::write(paths::work("version.txt"), VERSION);
        return;
    }
    // what a crash of the app left running (an AI's processes, OpenCode's server and its helper) ends now, before
    // any turn can start (see procs)
    procs::reap_orphans();
    paths::start_settings(); // the guarded settings: the copy outside the AIs' folders wins (paths::GUARDED)
    opencode::reap();
    let testing = env::var("RBXKIT_SELFTEST").ok().filter(|v| !v.is_empty());
    agents::new_agent(None);
    agents::keep_awake();
    thread::spawn(usage::tidy);
    if let Some(message) = testing {
        return self_test(&message);
    }
    agents::restore_open();
    // the knowledge index chosen in Setup (Off / Light / Heavy): its service starts in the background, and is ended
    // on quit (`agents::shut_down` -> `rag::shut_down`)
    rag::start_on_launch();

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
    thread::spawn(remote_host::start); // network access for the user's other computers, when they left it on
    thread::spawn(vault_sync::watch); // the shared skill vault: a check once a day, after the first Download

    let event_loop = EventLoopBuilder::<Ui>::with_user_event().build();
    let icon = window_icon();
    let builder = WindowBuilder::new()
        .with_title(APP)
        .with_inner_size(LogicalSize::new(1180.0, 820.0))
        .with_min_inner_size(LogicalSize::new(520.0, 480.0))
        .with_theme(Some(Theme::Dark))
        .with_window_icon(icon);
    // On Windows the page's own top bar is the window's: no system title bar over it (the page draws minimise,
    // maximise and close, and its empty parts move the window). The frame stays, unseen, for resizing and the shadow.
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_decorations(false).with_undecorated_shadow(true)
    };
    // On the Mac the page runs up under a see-through title bar, so the window is one piece: the close, minimise and
    // zoom buttons sit inside the sidebar's head, in the middle of its 48 px height, and the page moves the window.
    #[cfg(target_os = "macos")]
    let builder = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    };
    let window = builder.build(&event_loop).expect("could not create the window");
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        dark_caption(window.hwnd() as isize);
    }
    let data = if paths::packaged() { PATHS.home.join("webview") } else { paths::work("webview") };
    let mut context = WebContext::new(Some(data));
    let builder = WebViewBuilder::new_with_web_context(&mut context).with_background_color((12, 12, 13, 255)).with_url(&url);
    #[cfg(any(windows, target_os = "macos"))]
    let builder = {
        let proxy = event_loop.create_proxy();
        let frame = if cfg!(windows) { "window.__fridaeFrame = true;" } else { "window.__fridaeFrame = 'mac';" };
        builder
            .with_initialization_script(frame)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let ui = match req.body().as_str() {
                    "win:drag" => Ui::Drag,
                    "win:min" => Ui::Minimize,
                    "win:max" => Ui::Maximize,
                    "win:close" => Ui::Close,
                    "win:resize-top" => Ui::ResizeTop,
                    _ => return,
                };
                let _ = proxy.send_event(ui);
            })
    };
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
                        .run_status();
                }
                None => {
                    let _ = crate::paths::open_that(&url);
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
            let _ = proxy.send_event(Ui::Quit);
        });
    }
    #[cfg(target_os = "macos")]
    {
        // once the window is on screen (and once more after AppKit has laid out its bar)
        let proxy = event_loop.create_proxy();
        thread::spawn(move || {
            for ms in [150, 1000] {
                thread::sleep(Duration::from_millis(ms));
                let _ = proxy.send_event(Ui::PlaceButtons);
            }
        });
    }
    let mut was_max = false;
    let mut was_full = false;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(Ui::Quit | Ui::Close) => {
                agents::shut_down();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(Ui::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(Ui::PlaceButtons) => {
                #[cfg(target_os = "macos")]
                place_buttons(&window);
            }
            Event::UserEvent(Ui::Minimize) => window.set_minimized(true),
            Event::UserEvent(Ui::Maximize) => window.set_maximized(!window.is_maximized()),
            Event::UserEvent(Ui::ResizeTop) => {
                if !window.is_maximized() {
                    let _ = window.drag_resize_window(ResizeDirection::North);
                }
            }
            // the page's maximise button shows restore while the window fills the screen
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                #[cfg(target_os = "macos")]
                place_buttons(&window);
                let max = window.is_maximized();
                if max != was_max {
                    was_max = max;
                    let _ = _webview.evaluate_script(&format!("window.fridaeMax && window.fridaeMax({max})"));
                }
                // full screen on the Mac hides the window's buttons, so the sidebar's head stops leaving room for them
                let full = window.fullscreen().is_some();
                if full != was_full {
                    was_full = full;
                    let _ = _webview.evaluate_script(&format!("window.fridaeFull && window.fridaeFull({full})"));
                }
            }
            _ => {}
        }
    });
}
