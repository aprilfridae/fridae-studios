//! The local server the window's page talks to. It only listens on 127.0.0.1
//! and only answers requests that carry this run's secret token.

use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{
    agents::{self, BUS, MAX_AGENTS, MODELS},
    antigravity,
    attach, board, codex, media,
    paths::{self, PATHS, VERSION},
    plugin, projects, promo, promo_tab, research, team, textures, update, usage,
};

type Reply = Response<Cursor<Vec<u8>>>;

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

pub fn reply(code: u16, body: Vec<u8>, ctype: &str) -> Reply {
    Response::from_data(body).with_status_code(code).with_header(header("Content-Type", ctype)).with_header(header("Cache-Control", "no-store"))
}

pub fn send_json(v: Value) -> Reply {
    reply(200, v.to_string().into_bytes(), "application/json")
}

fn refuse(code: u16, why: &str) -> Reply {
    reply(code, json!({"error": why}).to_string().into_bytes(), "application/json")
}

fn not_found() -> Reply {
    refuse(404, "not found")
}

pub fn forbidden() -> Reply {
    refuse(403, "forbidden")
}

/// A request header's value, or "" when it was not sent.
pub fn head(req: &Request, name: &'static str) -> String {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string()).unwrap_or_default()
}

/// The request was addressed to this computer by its local name, as our own page and plugin do.
pub fn is_local(req: &Request) -> bool {
    matches!(head(req, "Host").split(':').next().unwrap_or(""), "127.0.0.1" | "localhost")
}

/// The request's body as JSON ({} when it is missing or malformed), reading at most `limit` bytes.
pub fn body_json(req: &mut Request, limit: u64) -> Value {
    let mut raw = Vec::new();
    let _ = req.as_reader().take(limit).read_to_end(&mut raw);
    serde_json::from_slice(&raw).unwrap_or_else(|_| json!({}))
}

fn send_file(path: PathBuf) -> Reply {
    let ctype = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "woff2" => "font/woff2",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    match fs::read(&path) {
        Ok(bytes) => reply(200, bytes, ctype),
        Err(_) => not_found(),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A URL path segment with its %XX escapes decoded.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// GET /media/promo/<card id>/<video|cover>: only the file the card names, streamed with byte ranges.
fn promo_media(path: &str, range: &str) -> media::Streamed {
    let mut parts = path.trim_start_matches("/media/promo/").splitn(2, '/');
    let (id, what) = (unescape(parts.next().unwrap_or("")), parts.next().unwrap_or(""));
    match promo::media(&id, what) {
        Some((file, ctype)) => media::serve(&file, ctype, range),
        None => media::missing(),
    }
}

/// GET /media/promo-style/<name>: a style's still from the tool's cache, for a known style name only.
fn style_media(path: &str, range: &str) -> media::Streamed {
    match promo_tab::style_still(&unescape(path.trim_start_matches("/media/promo-style/"))) {
        Some(file) => media::serve(&file, "image/jpeg", range),
        None => media::missing(),
    }
}

fn query(url: &str, key: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

/// Blender is looked for at most every half minute: the page asks for the status often.
fn blender_found() -> bool {
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    match *c {
        Some((at, found)) if at.elapsed().as_secs() < 30 => found,
        _ => {
            let found = paths::find_blender().is_some();
            *c = Some((Instant::now(), found));
            found
        }
    }
}

fn status() -> Value {
    json!({
        "agent": paths::find_ai(&paths::default_ai()).is_some(),
        "ai": {"default": paths::default_ai(), "installed": {"claude": paths::find_agent().is_some(), "gemini": paths::find_antigravity().is_some(), "chatgpt": paths::find_codex().is_some()},
               "models": {"gemini": antigravity::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>(),
                          "chatgpt": codex::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>()},
               "default_models": {"claude": "opus", "gemini": antigravity::default_model(), "chatgpt": codex::default_model()}},
        "studio": paths::studio_mcp().is_some(), "blender": blender_found(),
        "busy": agents::agents().iter().any(|a| a.busy()), "max_agents": MAX_AGENTS, "models": MODELS,
        "plugin": plugin::status(), "git": projects::git_installed(), "gemini": !paths::gemini_key().is_empty(), "gemini_api": paths::gemini_api_on(), "version": VERSION, "update": update::state(), "commands": paths::allowed_commands(),
    })
}

fn get(path: &str, url: &str) -> Reply {
    let ui = PATHS.kit.join("ui");
    let name = paths::base_name(path);
    if path == "/" {
        send_file(ui.join("index.html"))
    } else if path.starts_with("/fonts/") {
        send_file(ui.join("fonts").join(name))
    } else if path == "/logo.png" {
        send_file(ui.join("logo.png"))
    } else if path.starts_with("/img/") {
        send_file(paths::work("chat-images").join(name))
    } else if path.starts_with("/att/") {
        send_file(paths::work("attachments").join(name))
    } else if path == "/api/events" {
        let after = query(url, "after").and_then(|v| v.parse().ok()).unwrap_or(0);
        let wait = query(url, "wait").and_then(|v| v.parse().ok()).unwrap_or(20.0);
        send_json(json!({"events": BUS.after(after, wait)}))
    } else if path == "/api/history" {
        send_json(json!({"chats": agents::history(), "deleted": agents::deleted()}))
    } else if path == "/api/research" {
        send_json(research::state())
    } else if path == "/api/projects" {
        send_json(json!({"projects": projects::list(), "default_parent": projects::default_parent().display().to_string(), "git": projects::git_installed()}))
    } else if path == "/api/status" {
        send_json(status())
    } else if path == "/api/promos" {
        send_json(promo_tab::list())
    } else {
        not_found()
    }
}

/// Close the app and let the updater script swap the new version in.
fn restart_into_update() {
    agents::shut_down();
    if update::apply(true) {
        std::process::exit(0);
    }
}

fn post(path: &str, body: Value) -> Reply {
    let text_of = |key: &str| body[key].as_str().unwrap_or("").to_string();
    let target = agents::get(&text_of("agent"));
    match path {
        "/api/send" => {
            let text = text_of("text").trim().to_string();
            let attachments: Vec<String> = body["attachments"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            if let Some(agent) = target.clone().filter(|_| !text.is_empty() || !attachments.is_empty()) {
                let text = if text.is_empty() { "Look at what I attached.".to_string() } else { text };
                let (selftest, gemini, model) = (body["selftest"].as_bool().unwrap_or(false), body["gemini"].as_bool().unwrap_or(false), body["model"].as_str().map(str::to_string));
                let kickoff = body["kickoff"].as_str().map(str::to_string);
                let effort = body["effort"].as_str().map(str::to_string);
                let overnight = body["overnight"].as_bool().unwrap_or(false);
                let ask = agents::Ask { selftest, gemini, autopilot: body["autopilot"].as_bool().unwrap_or(false), overnight, full: overnight || body["full"].as_bool().unwrap_or(false), model, effort, attachments, kickoff };
                thread::spawn(move || agent.ask(&text, &ask));
            }
            send_json(json!({"ok": target.is_some()}))
        }
        "/api/undo-night" => match target {
            Some(a) => send_json(a.undo_night()),
            None => send_json(json!({"ok": false, "text": "That chat is gone."})),
        },
        "/api/usage" => send_json(json!({"ok": true, "records": usage::since(body["since"].as_f64().unwrap_or(0.0))})),
        "/api/stop" => {
            if let Some(a) = target {
                a.stop();
            }
            send_json(json!({"ok": true}))
        }
        "/api/new" => {
            let agent = agents::new_agent(body["ai"].as_str());
            send_json(json!({"ok": agent.is_some(), "agent": agent.map(|a| a.id.clone())}))
        }
        "/api/project" => {
            let path = text_of("path");
            let done = |r: Result<Value, String>| match r {
                Ok(p) => {
                    // a chat named in the request starts using the project straight away
                    let used = target.as_ref().map(|a| a.set_project(p["path"].as_str().unwrap_or(""))).unwrap_or(false);
                    send_json(json!({"ok": true, "project": p, "used": used}))
                }
                Err(e) => send_json(json!({"ok": false, "error": e})),
            };
            match text_of("action").as_str() {
                "create" => done(projects::create(&text_of("name"), body["parent"].as_str(), body["git"].as_bool().unwrap_or(false))),
                "add" | "use" => done(projects::add(&path)),
                "pick" => send_json(json!({"ok": true, "path": projects::pick_folder()})),
                "open" => send_json(json!({"ok": projects::open(&path, &text_of("what"))})),
                "forget" => {
                    projects::forget(&path);
                    send_json(json!({"ok": true}))
                }
                _ => send_json(json!({"ok": false, "error": "Unknown action."})),
            }
        }
        "/api/plugin" => match text_of("action").as_str() {
            "install" => match plugin::install() {
                Ok(path) => send_json(json!({"ok": true, "path": path, "plugin": plugin::status()})),
                Err(e) => send_json(json!({"ok": false, "error": e})),
            },
            "remove" => send_json(json!({"ok": plugin::remove(), "plugin": plugin::status()})),
            "revert" => match target.as_ref().map(|a| a.turn_started()).filter(|t| *t > 0.0) {
                Some(since) if target.as_ref().is_some_and(|a| !a.busy()) => send_json(plugin::revert(since)),
                Some(_) => send_json(json!({"ok": false, "text": "Stop the agent first, then undo."})),
                None => send_json(json!({"ok": false, "text": "This chat has not done anything in this session."})),
            },
            _ => send_json(json!({"ok": false})),
        },
        "/api/research" => match text_of("action").as_str() {
            "refresh" => {
                thread::spawn(research::refresh);
                send_json(json!({"ok": true}))
            }
            "ideas" => send_json(research::find_ideas(&text_of("focus"), body["ai"].as_str(), body["model"].as_str())),
            _ => send_json(json!({"ok": false})),
        },
        "/api/ai" => {
            // with a chat named: that chat's AI (only before it has started); without: the one new chats start on
            let choice = text_of("choice");
            let ok = paths::AIS.contains(&choice.as_str())
                && match &target {
                    Some(agent) => agent.set_ai(&choice),
                    None => {
                        let mut cfg = paths::load_config();
                        cfg.insert("chat_ai".into(), json!(choice));
                        paths::save_config(&cfg);
                        true
                    }
                };
            send_json(json!({"ok": ok}))
        }
        "/api/rename" => send_json(json!({"ok": target.as_ref().is_some_and(|a| a.rename(&text_of("name")))})),
        "/api/unqueue" => send_json(json!({"ok": target.as_ref().is_some_and(|a| a.unqueue(&text_of("qid")))})),
        "/api/board" => {
            // with an action, the user changes the board; without, it is read
            let project = text_of("path");
            if body["action"].is_string() { send_json(board::act(&project, &body)) } else { send_json(board::get(&project)) }
        }
        "/api/commands" => {
            // programs agents may run without asking: allowed from a "blocked" card in a chat, or removed in Setup
            let names: Vec<String> = body["names"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            send_json(json!({"ok": true, "commands": paths::set_allowed(&names, body["allow"].as_bool().unwrap_or(false))}))
        }
        "/api/git" => send_json(projects::git(&text_of("path"), &text_of("action"), &body)),
        "/api/pick-files" => send_json(json!({"paths": promo::pick_files(&text_of("kind"))})),
        "/api/promo" => send_json(promo::act(&body)),
        "/api/promo-engine" => send_json(promo::engine(&body)),
        "/api/promo-start" => send_json(promo_tab::start(&body)),
        "/api/promo-styles" => send_json(promo_tab::styles()),
        "/api/copy" => send_json(json!({"ok": textures::set_clipboard_text(&text_of("text"))})),
        "/api/texture" => {
            let rid = paths::base_name(&text_of("id"));
            let Some(req) = textures::find(&rid) else { return send_json(json!({"ok": false})) };
            let prompt = req["prompt"].as_str().unwrap_or("");
            match text_of("action").as_str() {
                "open" => {
                    let copied = textures::set_clipboard_text(prompt);
                    // developer aid: FRIDAE_GEMINI_URL points the button at a test page
                    let page = std::env::var("FRIDAE_GEMINI_URL").unwrap_or_else(|_| "https://gemini.google.com/app".to_string());
                    let opened = open::that(page).is_ok();
                    if copied && opened {
                        textures::paste_into_gemini();
                    }
                    send_json(json!({"ok": true, "copied": copied}))
                }
                "copy" => send_json(json!({"ok": textures::set_clipboard_text(prompt)})),
                "skip" => send_json(json!({"ok": textures::skip(&rid)})),
                "image" => {
                    let img = attach::decode(&text_of("data")).and_then(|raw| image::load_from_memory(&raw).ok());
                    send_json(json!({"ok": img.map(|i| textures::fulfil(&rid, &i)).unwrap_or(false)}))
                }
                _ => send_json(json!({"ok": false})),
            }
        }
        "/api/update" => {
            match text_of("action").as_str() {
                "check" => {
                    thread::spawn(|| update::run(Some(true)));
                }
                "auto" => update::set_auto(body["value"].as_bool().unwrap_or(false)),
                "install" if update::ready() => {
                    thread::spawn(restart_into_update);
                }
                _ => {}
            }
            send_json(json!({"ok": true, "update": update::state()}))
        }
        "/api/open-chat" => {
            let agent = agents::open_chat(&text_of("id"));
            send_json(json!({"ok": agent.is_some(), "agent": agent.map(|a| a.id.clone())}))
        }
        "/api/delete-chat" => {
            // deleting has to be meant: the request must carry the word the user typed to unlock it
            let ok = text_of("confirm").trim().eq_ignore_ascii_case("delete") && agents::delete_chat(&text_of("id"));
            send_json(json!({"ok": ok}))
        }
        "/api/restore-chat" => send_json(json!({"ok": agents::restore_chat(&text_of("id"))})),
        "/api/attach" => match attach::save_attachment(&text_of("data"), body["filename"].as_str()) {
            Some(mut got) => {
                got["ok"] = json!(true);
                got["url"] = json!(format!("/att/{}", got["name"].as_str().unwrap_or("")));
                send_json(got)
            }
            None => send_json(json!({"ok": false, "url": null})),
        },
        "/api/gemini" => {
            let mut cfg = paths::load_config();
            if !body["use_api"].is_null() {
                cfg.insert("gemini_use_api".into(), json!(body["use_api"].as_bool().unwrap_or(false)));
            }
            if let Some(key) = body.get("key") {
                match key.as_str().map(str::trim).filter(|k| !k.is_empty()) {
                    Some(k) => cfg.insert("gemini_key".into(), json!(k)),
                    None => cfg.remove("gemini_key"),
                };
            }
            paths::save_config(&cfg);
            send_json(json!({"ok": true, "gemini": !paths::gemini_key().is_empty()}))
        }
        "/api/remove" => {
            if let Some(a) = target {
                agents::remove_agent(&a.id);
            }
            send_json(json!({"ok": true}))
        }
        "/api/setup" => {
            thread::spawn(agents::run_setup);
            send_json(json!({"ok": true}))
        }
        "/api/connect" | "/api/agents" => send_json(json!({"text": agents::connect_other_agents()})),
        "/api/open" => {
            let _ = fs::create_dir_all(&PATHS.work);
            let _ = open::that(&PATHS.work);
            send_json(json!({"ok": true}))
        }
        _ => not_found(),
    }
}

fn handle(mut req: Request, token: &str) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    if let Ok(log) = std::env::var("FRIDAE_LOG") {
        // developer aid: note which requests arrive
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{} {} {}", req.method(), path, head(&req, "User-Agent").contains("Edg"));
        }
    }
    // only this app's own page may talk to the server
    let local = is_local(&req);
    let open_file = path.starts_with("/fonts/") || path == "/logo.png";
    let given = Some(head(&req, "X-Token")).filter(|t| !t.is_empty()).or_else(|| query(&url, "t")).unwrap_or_default();
    let allowed = local && (open_file || same(&given, token));
    if allowed && *req.method() == Method::Get && (path.starts_with("/media/promo/") || path.starts_with("/media/promo-style/")) {
        // a <video> or <img> carries the token in the address, like the chat's images
        let range = head(&req, "Range");
        let resp = if path.starts_with("/media/promo/") { promo_media(&path, &range) } else { style_media(&path, &range) };
        let _ = req.respond(resp);
        return;
    }
    let resp = if !allowed {
        forbidden()
    } else if *req.method() == Method::Get {
        get(&path, &url)
    } else if *req.method() == Method::Post {
        let body = body_json(&mut req, 64_000_000);
        post(&path, body)
    } else {
        not_found()
    };
    let _ = req.respond(resp);
}

/// Start serving; returns the address of the page for the window to open.
pub fn start(token: String) -> String {
    let server = Arc::new(Server::http("127.0.0.1:0").expect("could not open a local port"));
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let url = format!("http://127.0.0.1:{port}/?t={token}");
    thread::spawn(textures::watch);
    thread::spawn(team::watch);
    thread::spawn(promo::watch);
    plugin::start();
    antigravity::load_models();
    codex::load_models();
    thread::spawn(move || {
        // one thread per request, so a waiting long-poll never holds up anything else
        while let Ok(req) = server.recv() {
            let token = token.clone();
            thread::spawn(move || handle(req, &token));
        }
    });
    url
}
