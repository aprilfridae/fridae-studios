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
    attach, board, codex, cursor, expert_ai, media, opencode,
    paths::{self, PATHS, VERSION},
    plugin, projects, promo, promo_tab, research, studio_run, studio_watch, team, textures, update, usage,
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

/// GET /media/review/<project key>/<item id>/<file>: only a picture or clip the review item lists, from its folder.
fn review_media(path: &str, range: &str) -> media::Streamed {
    let mut parts = path.trim_start_matches("/media/review/").splitn(3, '/');
    let (key, id, file) = (unescape(parts.next().unwrap_or("")), unescape(parts.next().unwrap_or("")), unescape(parts.next().unwrap_or("")));
    match studio_run::media(&key, &id, &file) {
        Some((file, ctype)) => media::serve(&file, ctype, range),
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
    cursor::refresh_models(); // asks Cursor again while its list is empty (signed out or offline when the app started)
    opencode::refresh_models();
    crate::signin::refresh(); // who is signed in to Claude Code, Cursor and Gemini: asked again every few minutes at most
    json!({
        "agent": paths::find_ai(&paths::default_ai()).is_some(),
        "ai": {"default": paths::default_ai(), "installed": {"claude": paths::find_agent().is_some(), "gemini": paths::find_antigravity().is_some(), "chatgpt": paths::find_codex().is_some(), "cursor": paths::find_cursor().is_some(), "opencode": paths::find_opencode().is_some()},
               "offered": paths::AIS,
               "gaps": {"cursor": cursor::gaps(), "opencode": opencode::gaps()},
               "caps": crate::caps::table(),
               "opencode": opencode::state(),
               "sign_in": crate::signin::state(),
               "accounts": crate::accounts::state(),
               "models": {"gemini": antigravity::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>(),
                          "chatgpt": codex::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>(),
                          "cursor": cursor::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>(),
                          "opencode": opencode::models().iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>()},
               "default_models": {"claude": "opus", "gemini": antigravity::default_model(), "chatgpt": codex::default_model(), "cursor": cursor::default_model(), "opencode": opencode::default_model()}},
        "studio": paths::studio_mcp().is_some(), "blender": blender_found(),
        "busy": agents::agents().iter().any(|a| a.busy()), "max_agents": MAX_AGENTS, "slots_used": agents::slots_used(), "models": MODELS,
        "limits": crate::limits::state(), "plugin": plugin::status(), "git": projects::git_installed(), "gemini": !paths::gemini_key().is_empty(), "gemini_api": paths::gemini_api_on(), "gemini_images": paths::gemini_images_on(), "expert_summaries": paths::expert_summaries_on(), "env_keys": paths::env_keys_on(), "settings_put_back": paths::put_back_at_start(), "version": VERSION, "update": update::state(), "commands": paths::allowed_commands(),
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
    } else if path == "/api/notes" {
        send_json(crate::notes::info(&unescape(&query(url, "path").unwrap_or_default())))
    } else if path == "/api/github/status" {
        send_json(crate::github::status(query(url, "project").map(|p| unescape(&p)).as_deref()))
    } else if path == "/api/review" {
        send_json(studio_run::list(&unescape(&query(url, "path").unwrap_or_default())))
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
                // Studio mode implies full auto and overnight's loop
                let studio = body["studio"].as_bool().unwrap_or(false);
                let overnight = studio || body["overnight"].as_bool().unwrap_or(false);
                let ask = agents::Ask { selftest, gemini, autopilot: body["autopilot"].as_bool().unwrap_or(false), overnight, full: overnight || body["full"].as_bool().unwrap_or(false), model, effort, attachments, kickoff, studio, wake: false, app: body["app"].as_bool().unwrap_or(false), typed: None, resend: None };
                thread::spawn(move || agent.ask(&text, &ask));
            }
            send_json(json!({"ok": target.is_some()}))
        }
        "/api/notes-open" => send_json(json!({"ok": crate::notes::open(&text_of("scope"), &text_of("path"))})),
        "/api/undo-night" => match target {
            Some(a) => send_json(a.undo_night(body["files_only"].as_bool().unwrap_or(false))),
            None => send_json(json!({"ok": false, "text": "That chat is gone."})),
        },
        "/api/usage" => send_json(json!({"ok": true, "records": usage::since(body["since"].as_f64().unwrap_or(0.0))})),
        "/api/stop" => {
            if let Some(a) = target {
                a.stop();
                crate::studio_undo::pause_if_lead(&a); // Stop on a studio run's Lead pauses the run
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
                // Delete in Project details: off the list, and with `trash` the folder to the Trash. Never while one of
                // its chats is working.
                "delete" => {
                    let working = agents::agents().iter().any(|a| a.busy() && a.project().is_some_and(|p| projects::same(&p, &path)));
                    if working {
                        send_json(json!({"ok": false, "error": "A chat is working in this project. Stop it first."}))
                    } else {
                        match projects::delete(&path, body["trash"].as_bool().unwrap_or(false)) {
                            Ok(()) => send_json(json!({"ok": true})),
                            Err(e) => send_json(json!({"ok": false, "error": e})),
                        }
                    }
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
            // with a chat named: that chat's AI (a started chat carries on with the other AI, handed the chat so far);
            // without: the one new chats start on
            let choice = text_of("choice");
            match &target {
                Some(agent) => match agent.switch_ai(&choice) {
                    Ok(switched) => send_json(json!({"ok": true, "switched": switched})),
                    Err(e) => send_json(json!({"ok": false, "error": e})),
                },
                None if !text_of("agent").is_empty() => send_json(json!({"ok": false, "error": "That chat is gone."})),
                None if paths::AIS.contains(&choice.as_str()) => {
                    paths::update_config(|cfg| cfg.insert("chat_ai".into(), json!(choice)));
                    send_json(json!({"ok": true}))
                }
                None => send_json(json!({"ok": false})),
            }
        }
        // a second account for an AI (Setup, Connections): Add makes the app's folder for it and starts the tool's
        // sign-in there; Remove deletes that folder. Only from the page: the setting is guarded.
        "/api/accounts" => {
            let (ai, name) = (text_of("ai"), text_of("name"));
            match text_of("action").as_str() {
                // another account: its folder, last in the AI's list, and the tool's sign-in started for it
                "add" => match crate::accounts::add(&ai) {
                    Ok(name) => {
                        let id = crate::accounts::id_of(&ai, &name);
                        let mut answer = crate::signin::login_start(&id, false);
                        answer["id"] = json!(id);
                        send_json(answer)
                    }
                    Err(why) => send_json(json!({"ok": false, "error": why})),
                },
                "remove" if crate::accounts::folder_of(&ai, &name).is_some() => {
                    // nothing is left running in the folder that is about to go: a waiting sign-in, a turn on that
                    // account, a process kept for one
                    crate::signin::forget(&crate::accounts::id_of(&ai, &name));
                    agents::account_removed(&ai, &name);
                    // (its folder could not be deleted: said, with its place)
                    let left = crate::accounts::remove(&ai, &name);
                    send_json(json!({"ok": true, "left": left.map(|d| d.display().to_string())}))
                }
                _ => send_json(json!({"ok": false})),
            }
        }
        // the user's own sign-in to Claude Code, Cursor or Gemini: start the tool's sign-in, end one that waits, or
        // ask the tool again who is signed in. There is no signing out here.
        "/api/signin" => {
            let ai = text_of("ai");
            match text_of("action").as_str() {
                // `sure`: pressed again on the line that says what a new sign-in does to one that is there
                "login" => send_json(crate::signin::login_start(&ai, body["sure"] == true)),
                "cancel" => {
                    crate::signin::login_cancel(&ai);
                    send_json(crate::signin::login_state(&ai))
                }
                "check" => {
                    crate::signin::check(&ai, true);
                    send_json(crate::signin::login_state(&ai))
                }
                _ => send_json(crate::signin::login_state(&ai)),
            }
        }
        // Setup > Connections: Fridae's own OpenCode sign-in (OpenCode's device sign-in, approved in the browser)
        "/api/opencode" => match text_of("action").as_str() {
            "login" => send_json(opencode::login_start()),
            "open" => send_json(json!({"ok": opencode::login_open()})),
            "cancel" => {
                opencode::login_cancel();
                send_json(opencode::login_state())
            }
            // the model picked in the menu under the message box: remembered for every OpenCode chat and expert
            "model" => match opencode::remember_pick(&text_of("model")) {
                Ok(()) => send_json(json!({"ok": true})),
                Err(why) => send_json(json!({"ok": false, "error": why})),
            },
            _ => send_json(opencode::login_state()),
        },
        // Setup > Connections > GitHub, and Project details' Put on GitHub and Push now (github.rs)
        p if p.starts_with("/api/github/") => send_json(crate::github::post(p, &body)),
        "/api/rename" => send_json(json!({"ok": target.as_ref().is_some_and(|a| a.rename(&text_of("name")))})),
        "/api/unqueue" => send_json(json!({"ok": target.as_ref().is_some_and(|a| a.unqueue(&text_of("qid")))})),
        "/api/board" => {
            // with an action, the user changes the board; without, it is read
            let project = text_of("path");
            if body["action"].is_string() { send_json(board::act(&project, &body)) } else { send_json(board::get(&project)) }
        }
        "/api/experts" => {
            // Setup > Experts: with an expert, save its row (AI "" or "lead" for Same as the Lead); always the listing
            if body["expert"].is_string() {
                if let Err(e) = expert_ai::set_entry(&text_of("expert"), &text_of("ai"), &text_of("model")) {
                    return send_json(json!({"ok": false, "error": e}));
                }
            }
            // Setup's "The Lead may pick an AI for a job"
            if let Some(on) = body["lead_may_pick"].as_bool() {
                paths::update_config(|cfg| cfg.insert(expert_ai::LEAD_PICKS.into(), json!(on)));
            }
            send_json(json!({"ok": true, "list": expert_ai::listing()}))
        }
        "/api/commands" => {
            // programs agents may run without asking: allowed from a "blocked" card in a chat, or removed in Setup
            let names: Vec<String> = body["names"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            send_json(json!({"ok": true, "commands": paths::set_allowed(&names, body["allow"].as_bool().unwrap_or(false))}))
        }
        "/api/settings-start" => {
            // the start-up line about settings put back: take the file's instead, or close it
            send_json(json!({"ok": paths::answer_start_note(body["take_file"].as_bool().unwrap_or(false))}))
        }
        "/api/run-once" => {
            // a command a chat's card showed as not approved, which the user said may run one time in that chat
            send_json(json!({"ok": opencode::approve_once(&text_of("agent"), &text_of("command"))}))
        }
        "/api/git" => send_json(projects::git(&text_of("path"), &text_of("action"), &body)),
        "/api/pick-files" => send_json(json!({"paths": promo::pick_files(&text_of("kind"))})),
        "/api/promo" => send_json(promo::act(&body)),
        "/api/promo-engine" => send_json(promo::engine(&body)),
        "/api/promo-start" => send_json(promo_tab::start(&body)),
        "/api/promo-styles" => send_json(promo_tab::styles()),
        "/api/review" => send_json(studio_run::review(&body)),
        "/api/studio-run" => send_json(studio_run::studio_run(&body)),
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
                    let opened = crate::paths::open_that(page).is_ok();
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
        "/api/env-keys" => {
            let on = body["on"].as_bool().unwrap_or(false);
            paths::update_config(|cfg| cfg.insert("use_env_keys".into(), json!(on)));
            send_json(json!({"ok": true, "env_keys": paths::env_keys_on()}))
        }
        "/api/gemini" => {
            paths::update_config(|cfg| {
                if !body["use_api"].is_null() {
                    cfg.insert("gemini_use_api".into(), json!(body["use_api"].as_bool().unwrap_or(false)));
                }
                // "Agents may generate images with Gemini": on unless switched off
                if let Some(on) = body["images"].as_bool() {
                    cfg.insert("gemini_images".into(), json!(on));
                }
                if let Some(key) = body.get("key") {
                    match key.as_str().map(str::trim).filter(|k| !k.is_empty()) {
                        Some(k) => cfg.insert("gemini_key".into(), json!(k)),
                        None => cfg.remove("gemini_key"),
                    };
                }
            });
            send_json(json!({"ok": true, "gemini": !paths::gemini_key().is_empty()}))
        }
        "/api/expert-summaries" => {
            // "Plain-English summary of each expert's work": on unless switched off
            if let Some(on) = body["on"].as_bool() {
                paths::update_config(|cfg| {
                    cfg.insert("expert_summaries".into(), json!(on));
                });
            }
            send_json(json!({"ok": true, "expert_summaries": paths::expert_summaries_on()}))
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
            let _ = crate::paths::open_that(&PATHS.work);
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
    let media_path = ["/media/promo/", "/media/promo-style/", "/media/review/"].iter().any(|p| path.starts_with(p));
    if allowed && *req.method() == Method::Get && media_path {
        // a <video> or <img> carries the token in the address, like the chat's images
        let range = head(&req, "Range");
        let resp = if path.starts_with("/media/promo/") {
            promo_media(&path, &range)
        } else if path.starts_with("/media/review/") {
            review_media(&path, &range)
        } else {
            style_media(&path, &range)
        };
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
    thread::spawn(studio_watch::watch);
    thread::spawn(crate::studio_undo::watch);
    plugin::start();
    crate::signin::start(); // who is signed in to Claude Code, Cursor and Gemini (Gemini's answer is its model list)
    codex::load_models();
    cursor::load_models();
    opencode::load_models();
    thread::spawn(move || {
        // one thread per request, so a waiting long-poll never holds up anything else
        while let Ok(req) = server.recv() {
            let token = token.clone();
            thread::spawn(move || handle(req, &token));
        }
    });
    url
}
