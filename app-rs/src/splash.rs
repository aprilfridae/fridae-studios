//! Installing a downloaded update, with a small splash window instead of console windows.
//!
//! The old app starts the NEW program from the download folder as
//!   `Fridae Studios.exe --apply-update <old pid> <install folder> <version> [relaunch]`
//! and exits. The new program waits for the old one to close, copies its own folder over the install folder
//! while a progress bar fills, opens the installed app and quits. Nothing here runs a console program, so no
//! console window can appear. It uses nothing from `paths`, which would point at the download folder here.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::windows::WindowExtWindows,
    window::WindowBuilder,
};
use wry::{WebContext, WebViewBuilder};

const EXE_NAME: &str = "Fridae Studios.exe";

enum Msg {
    Progress(f64, String),
    Close,
}

/// Wait (up to `secs`) for a process to exit.
fn wait_for_exit(pid: u32, secs: u32) {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn WaitForSingleObject(handle: isize, ms: u32) -> u32;
        fn CloseHandle(handle: isize) -> i32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;
    unsafe {
        let h = OpenProcess(SYNCHRONIZE, 0, pid);
        if h != 0 {
            WaitForSingleObject(h, secs * 1000);
            CloseHandle(h);
        }
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Copy a file, retrying for a few seconds while the old app's files are still being let go.
fn copy_with_retry(from: &Path, to: &Path) -> bool {
    if let Some(parent) = to.parent() {
        let _ = fs::create_dir_all(parent);
    }
    for _ in 0..40 {
        if fs::copy(from, to).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

fn install(source: &Path, target: &Path, pid: u32, report: impl Fn(f64, &str)) -> bool {
    report(0.0, "Waiting for Fridae Studios to close");
    wait_for_exit(pid, 60);
    let mut files = Vec::new();
    files_under(source, &mut files);
    let total = files.len().max(1);
    let mut ok = true;
    for (i, f) in files.iter().enumerate() {
        let Ok(rel) = f.strip_prefix(source) else { continue };
        ok &= copy_with_retry(f, &target.join(rel));
        if i % 4 == 0 || i + 1 == total {
            report((i + 1) as f64 / total as f64, "Installing");
        }
    }
    ok
}

fn page(version: &str) -> String {
    use base64::Engine;
    let logo = base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../ui/logo.png"));
    let version = version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').collect::<String>();
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>
html,body{{margin:0;height:100%;background:#0c0c0d;color:#e8e8e8;font:13px "Segoe UI Variable Text","Segoe UI",system-ui,sans-serif;
  user-select:none;cursor:default;overflow:hidden}}
body{{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;box-sizing:border-box;
  border:1px solid #2a2a2c}}
img{{width:44px;height:44px;opacity:.95}}
h1{{margin:0;font-size:15px;font-weight:600;letter-spacing:.01em}}
.v{{color:#8a8a8f;font-size:12px;margin-top:-8px}}
.bar{{width:260px;height:4px;border-radius:2px;background:#1f1f22;overflow:hidden}}
.fill{{height:100%;width:0;background:#e8e8e8;border-radius:2px;transition:width .2s ease}}
.s{{color:#8a8a8f;font-size:12px;height:16px}}
</style></head><body>
<img src="data:image/png;base64,{logo}" alt="">
<h1>Updating Fridae Studios</h1><div class="v">Version {version}</div>
<div class="bar"><div class="fill" id="f"></div></div><div class="s" id="s">Starting</div>
<script>function set(p,t){{document.getElementById('f').style.width=(p*100).toFixed(1)+'%';document.getElementById('s').textContent=t;}}</script>
</body></html>"#
    )
}

/// `--apply-update <pid> <install> <version> [relaunch]`. Returns when the update is in place.
pub fn run(args: &[String]) {
    let pid: u32 = args.first().and_then(|v| v.parse().ok()).unwrap_or(0);
    let Some(target) = args.get(1).map(PathBuf::from) else { return };
    let version = args.get(2).cloned().unwrap_or_default();
    let relaunch = args.iter().any(|a| a == "relaunch");
    let Some(source) = env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
    if source == target {
        return;
    }

    let event_loop = EventLoopBuilder::<Msg>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let worker = {
        let (source, target) = (source.clone(), target.clone());
        move || {
            let p = proxy.clone();
            let ok = install(&source, &target, pid, |f, text| {
                let _ = p.send_event(Msg::Progress(f, text.to_string()));
            });
            let _ = proxy.send_event(Msg::Progress(1.0, if ok { "Opening Fridae Studios".into() } else { "Some files could not be replaced. Opening anyway".into() }));
            if relaunch {
                let exe = target.join(EXE_NAME);
                let _ = Command::new(&exe).current_dir(&target).spawn();
            }
            thread::sleep(Duration::from_millis(if ok { 900 } else { 2500 }));
            let _ = proxy.send_event(Msg::Close);
        }
    };

    let window = WindowBuilder::new()
        .with_title("Updating Fridae Studios")
        .with_inner_size(LogicalSize::new(380.0, 220.0))
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top(true)
        .build(&event_loop);
    let Ok(window) = window else {
        worker(); // no window possible: still install
        return;
    };
    if let Some(monitor) = window.current_monitor() {
        let (m, w) = (monitor.size(), window.outer_size());
        let pos = monitor.position();
        window.set_outer_position(tao::dpi::PhysicalPosition::new(pos.x + (m.width as i32 - w.width as i32) / 2, pos.y + (m.height as i32 - w.height as i32) / 2));
    }
    round_corners(window.hwnd() as isize);
    let mut context = WebContext::new(Some(env::temp_dir().join("fridae-update-webview")));
    let view = WebViewBuilder::new_with_web_context(&mut context).with_background_color((12, 12, 13, 255)).with_html(page(&version)).build(&window).ok();
    thread::spawn(worker);
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Msg::Progress(f, text)) => {
                if let Some(v) = &view {
                    let _ = v.evaluate_script(&format!("set({f},{})", serde_json::Value::String(text)));
                }
            }
            // Alt+F4 is ignored: closing halfway through the copy would leave a broken install
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {}
            Event::UserEvent(Msg::Close) => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

/// Windows 11 rounded corners for the borderless splash.
fn round_corners(hwnd: isize) {
    #[link(name = "dwmapi", kind = "raw-dylib")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const core::ffi::c_void, size: u32) -> i32;
    }
    let round: u32 = 2;
    unsafe {
        DwmSetWindowAttribute(hwnd, 33, (&raw const round).cast(), 4);
    }
}
