//! project_list, project_read, project_create, project_update and project_delete (ports of T3's project tools, with
//! Fridae's card where T3 refuses: below Full auto, and always for removing a project or adding an existing folder).

use std::path::Path;

use serde_json::{Map, Value, json};

use super::{cursor_of, max_len, opt_bool, opt_choice, opt_int, opt_text, page, project_chip, reads, text, words, writes_environment};
use crate::app_mcp::{Call, Failure, Hints, Labels, Outcome, Summary, ToolDef, access, card::CardSpec, chat_link, events, fail, project_id, project_link};
use crate::{agents, projects};

pub fn defs() -> Vec<ToolDef> {
    let base = ToolDef {
        name: "project_list",
        title: "List projects",
        description: "List projects. Pages can shift.",
        schema: schema_list,
        hints: Hints::READ,
        access: reads,
        labels: labels_list,
        always_load: false,
        handler: list,
    };
    vec![
        base,
        ToolDef { name: "project_read", title: "Read a project", description: "Read one project: its folder, kind and chats.", schema: schema_read, labels: labels_read, handler: read, ..base },
        ToolDef {
            name: "project_create",
            title: "Add a project",
            description: "Register a project folder. `createWorkspaceRootIfMissing` creates it; a registered folder is rejected. No `workspaceRoot`: makes a new folder from `title` in your projects folder, with git when `git` is true. Its result confirms it.",
            schema: schema_create,
            hints: Hints::WRITE,
            access: writes_environment,
            labels: labels_create,
            handler: create,
            ..base
        },
        ToolDef {
            name: "project_update",
            title: "Update a project",
            description: "Update one project's settings. Omitted fields are unchanged. Its result confirms it.",
            schema: schema_update,
            hints: Hints::WRITE,
            access: writes_environment,
            labels: labels_update,
            handler: update,
            ..base
        },
        ToolDef {
            name: "project_delete",
            title: "Remove a project",
            description: "Delete one project. Nonempty needs `force: true`. Folder on disk stays. Its result confirms it.",
            schema: schema_delete,
            hints: Hints::WRITE,
            access: writes_environment,
            labels: labels_delete,
            handler: delete,
            ..base
        },
    ]
}

fn project_id_prop() -> Value {
    json!({"type": "string", "description": "A project's id from project_list."})
}

fn schema_list() -> Value {
    json!({"type": "object", "properties": {
        "cursor": {"type": "string", "description": "nextCursor from the page before."},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Max 100."}
    }})
}

fn schema_read() -> Value {
    json!({"type": "object", "properties": {"projectId": project_id_prop()}, "required": ["projectId"]})
}

fn schema_create() -> Value {
    json!({"type": "object", "properties": {
        "title": {"type": "string", "maxLength": 80, "description": "The project's name."},
        "workspaceRoot": {"type": "string", "description": "The project's folder (a full path)."},
        "createWorkspaceRootIfMissing": {"type": "boolean"},
        "git": {"type": "boolean", "description": "Start a git repository in a new folder."},
        "kind": {"type": "string", "enum": ["roblox", "plain"]}
    }, "required": ["title"]})
}

fn schema_update() -> Value {
    json!({"type": "object", "properties": {
        "projectId": project_id_prop(),
        "title": {"type": "string", "maxLength": 80, "description": "The name Fridae shows. Its folder keeps its own name."},
        "kind": {"type": "string", "enum": ["roblox", "plain", "auto"], "description": "auto: the folder decides."}
    }, "required": ["projectId"]})
}

fn schema_delete() -> Value {
    json!({"type": "object", "properties": {"projectId": project_id_prop(), "force": {"type": "boolean"}}, "required": ["projectId"]})
}

fn labels_list(_: &Map<String, Value>) -> (Labels, Summary) {
    words("List", "Listing", "Listed", "projects", "Listed", "project list", "project lists")
}

fn labels_read(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Read", "Reading", "Read", "a project", "Read", "project", "projects")
}

fn labels_create(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Add", "Adding", "Added", "a project", "Added", "project", "projects")
}

fn labels_update(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Update", "Updating", "Updated", "a project", "Updated", "project", "projects")
}

fn labels_delete(_: &Map<String, Value>) -> (Labels, Summary) {
    words("Remove", "Removing", "Removed", "a project", "Removed", "project", "projects")
}

/// A project as agents see it, from `projects::describe`.
fn item(d: &Value) -> Value {
    let (path, name) = (d["path"].as_str().unwrap_or(""), d["name"].as_str().unwrap_or(""));
    let id = project_id(path);
    json!({"projectId": id, "name": name, "path": path, "kind": if d["kind"] == "game" { "roblox" } else { "plain" }, "git": d["git"] == true, "link": project_link(&id, name)})
}

/// The folder of the project an id names (a project whose folder is gone is not listed, as on the page).
pub(crate) fn path_of(id: &str) -> Option<String> {
    projects::list().iter().filter_map(|p| p["path"].as_str()).find(|p| project_id(p) == id.trim()).map(str::to_string)
}

pub(crate) fn not_found() -> Failure {
    fail("invalid_request", "That project was not found. project_list names them.")
}

fn found(args: &Map<String, Value>) -> Result<String, Failure> {
    let id = text(args, "projectId").ok_or_else(|| fail("invalid_request", "Pass projectId."))?;
    path_of(id).ok_or_else(not_found)
}

/// The chats in a project: open ones, then saved ones.
fn chats_in(path: &str) -> Vec<Value> {
    let mut out: Vec<Value> = agents::agents()
        .iter()
        .filter(|a| a.project().is_some_and(|p| projects::same(&p, path)))
        .map(|a| json!({"chatId": a.id, "title": a.name(), "link": chat_link(&a.id, &a.name()), "open": true}))
        .collect();
    for c in agents::history() {
        if c["project"].as_str().is_some_and(|p| projects::same(p, path)) {
            let (id, title) = (c["id"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or("Chat"));
            out.push(json!({"chatId": id, "title": title, "link": chat_link(id, title), "open": false}));
        }
    }
    out
}

pub(super) fn list(_call: &Call, args: &Map<String, Value>) -> Outcome {
    let limit = opt_int(args, "limit", 1, 100)?.unwrap_or(25) as usize;
    let all: Vec<Value> = projects::list().iter().map(item).collect();
    let (projects, next) = page(&all, cursor_of(args)?, limit);
    Ok(json!({"projects": projects, "nextCursor": next}))
}

fn chip_of(out: &Value) -> Value {
    project_chip(out["projectId"].as_str().unwrap_or(""), out["name"].as_str().unwrap_or(""))
}

fn read(call: &Call, args: &Map<String, Value>) -> Outcome {
    let path = found(args)?;
    let d = projects::describe(&path);
    let mut out = item(&d);
    out["bible"] = json!(d["bible"] == true);
    out["chats"] = json!(chats_in(&path));
    call.links(vec![chip_of(&out)]);
    Ok(out)
}

/// A changed project's result, its chip on the row, and the window told.
fn changed(call: &Call, d: &Value) -> Value {
    let out = item(d);
    call.links(vec![chip_of(&out)]);
    events::projects_changed();
    out
}

fn kind_name(kind: &str) -> &'static str {
    if projects::kind_word(Some(kind)) == Some("game") { "Roblox" } else { "plain" }
}

fn quote(s: &str) -> String {
    format!("\"{s}\"")
}

fn create(call: &Call, args: &Map<String, Value>) -> Outcome {
    let title = text(args, "title").ok_or_else(|| fail("invalid_request", "Pass a title."))?.to_string();
    max_len("title", &title, 80)?;
    let kind = opt_choice(args, "kind", &["roblox", "plain"])?;
    let git = opt_bool(args, "git")?.unwrap_or(false);
    let make_missing = opt_bool(args, "createWorkspaceRootIfMissing")?.unwrap_or(false);
    let root = opt_text(args, "workspaceRoot")?.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
    let caller = access::writing_caller(call.scope)?;
    let made = match root {
        None => {
            let parent = if projects::kind_word(kind.as_deref()) == Some("game") { projects::default_parent() } else { projects::default_parent_plain() };
            let card = CardSpec { title: "Make a project".into(), text: format!("Make a new project folder {} in {}.", quote(&title), parent.display()), allow: "Make it".into() };
            access::environment_gate(call, &caller, card, false)?;
            projects::create(&title, None, git, kind.as_deref())
        }
        Some(root) => {
            let dir = Path::new(&root);
            if !dir.is_absolute() {
                return Err(fail("invalid_request", "workspaceRoot must be a full path."));
            }
            if projects::known(&root) {
                return Err(fail("invalid_request", "That folder is already a project."));
            }
            if dir.is_dir() {
                // an existing folder is always asked about: its files become the AIs' to work in
                let card = CardSpec { title: "Add a project".into(), text: format!("Add the folder {} to your projects.", dir.display()), allow: "Add it".into() };
                access::environment_gate(call, &caller, card, true)?;
                projects::add(&root, kind.as_deref())
            } else if make_missing {
                let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
                    return Err(fail("invalid_request", "workspaceRoot must name a folder inside another."));
                };
                let name = name.to_string_lossy().to_string();
                let card = CardSpec { title: "Make a project".into(), text: format!("Make a new project folder {} in {}.", quote(&name), parent.display()), allow: "Make it".into() };
                access::environment_gate(call, &caller, card, false)?;
                projects::create(&name, Some(&parent.display().to_string()), git, kind.as_deref())
            } else {
                return Err(fail("invalid_request", "That folder does not exist. Pass createWorkspaceRootIfMissing: true to make it."));
            }
        }
    };
    let d = made.map_err(|e| fail("invalid_request", e))?;
    let path = d["path"].as_str().unwrap_or("").to_string();
    if projects::describe(&path)["name"].as_str() != Some(title.as_str()) {
        let _ = projects::rename(&path, &title);
    }
    let mut out = changed(call, &projects::describe(&path));
    if let Some(note) = d["note"].as_str().filter(|n| !n.is_empty()) {
        out["note"] = json!(note);
    }
    Ok(out)
}

fn update(call: &Call, args: &Map<String, Value>) -> Outcome {
    let path = found(args)?;
    let title = opt_text(args, "title")?.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
    if title.as_deref() == Some("") {
        return Err(fail("invalid_request", "Give the project a name."));
    }
    if let Some(t) = &title {
        max_len("title", t, 80)?;
    }
    let kind = opt_choice(args, "kind", &["roblox", "plain", "auto"])?;
    if title.is_none() && kind.is_none() {
        return Err(fail("invalid_request", "Pass title or kind."));
    }
    let caller = access::writing_caller(call.scope)?;
    let old = projects::describe(&path)["name"].as_str().unwrap_or("").to_string();
    let shown = title.clone().unwrap_or_else(|| old.clone());
    let rename_line = title.as_ref().map(|t| format!("Rename {} to {}. Its folder keeps its own name.", quote(&old), quote(t)));
    let kind_line = kind.as_ref().map(|k| match k.as_str() {
        "auto" => format!("Let its folder decide whether {} is a Roblox project.", quote(&shown)),
        k => format!("Make {} a {} project.", quote(&shown), kind_name(k)),
    });
    let card = match (rename_line, kind_line) {
        (Some(r), None) => CardSpec { title: "Rename project".into(), text: r, allow: "Rename it".into() },
        (None, Some(k)) => CardSpec { title: "Change project type".into(), text: k, allow: "Change it".into() },
        (Some(r), Some(k)) => CardSpec { title: "Update project".into(), text: format!("{r} {k}"), allow: "Change it".into() },
        (None, None) => return Err(fail("invalid_request", "Pass title or kind.")),
    };
    access::environment_gate(call, &caller, card, false)?;
    if let Some(t) = &title {
        projects::rename(&path, t).map_err(|e| fail("invalid_request", e))?;
    }
    if let Some(k) = &kind {
        projects::set_kind(&path, k).map_err(|e| fail("invalid_request", e))?;
    }
    Ok(changed(call, &projects::describe(&path)))
}

fn delete(call: &Call, args: &Map<String, Value>) -> Outcome {
    let path = found(args)?;
    let force = opt_bool(args, "force")?.unwrap_or(false);
    let caller = access::writing_caller(call.scope)?;
    if agents::agents().iter().any(|a| a.id != caller.agent.id && a.project().is_some_and(|p| projects::same(&p, &path)) && a.is_working()) {
        return Err(fail("thread_not_sendable", "A chat is working in this project. Stop it first."));
    }
    if !force && !chats_in(&path).is_empty() {
        return Err(fail("invalid_request", "This project has chats. Pass force: true to remove it anyway; its folder and chats stay."));
    }
    let name = projects::describe(&path)["name"].as_str().unwrap_or("").to_string();
    let card = CardSpec { title: "Remove project".into(), text: format!("Take {} off your projects. The folder and its chats stay.", quote(&name)), allow: "Remove it".into() };
    access::environment_gate(call, &caller, card, true)?;
    // only off the list: the folder and its chats stay (never `projects::delete`)
    projects::forget(&path);
    let id = project_id(&path);
    call.links(vec![project_chip(&id, &name)]);
    events::projects_changed();
    Ok(json!({"projectId": id, "removed": true}))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use crate::app_mcp::project_id;
    use crate::projects;
    use serde_json::{Value, json};

    fn registered(name: &str) -> (String, String) {
        let dir = project_dir(name);
        let p = projects::add(&dir, Some("plain")).unwrap();
        let path = p["path"].as_str().unwrap().to_string();
        (project_id(&path), path)
    }

    fn name_of(path: &str) -> String {
        projects::describe(path)["name"].as_str().unwrap().to_string()
    }

    #[test]
    fn project_list_and_read() {
        let _serial = serial();
        let (id, path) = registered("list");
        let a = caller(false);
        let mut all: Vec<Value> = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let got = ok(&call(&a, "project_list", json!({"cursor": cursor, "limit": 1})));
            all.extend(got["projects"].as_array().unwrap().iter().cloned());
            cursor = got["nextCursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        let mine = all.iter().find(|p| p["projectId"] == id.as_str()).expect("listed");
        assert_eq!(mine["path"], path.as_str());
        assert_eq!(mine["kind"], "plain");
        assert_eq!(mine["link"], crate::app_mcp::project_link(&id, &name_of(&path)));
        let read = ok(&call(&a, "project_read", json!({"projectId": id})));
        assert_eq!(read["projectId"], id.as_str());
        assert_eq!(read["chats"], json!([]));
        assert_eq!(err(&call(&a, "project_read", json!({"projectId": "pnope"}))).0, "invalid_request");
        crate::app_mcp::drop_chat(&a);
        projects::forget(&path);
    }

    #[test]
    fn project_tools_full_auto_no_card() {
        let _serial = serial();
        let (id, path) = registered("fullauto");
        let a = caller(true);
        let n = bus_len();
        let got = ok(&call(&a, "project_update", json!({"projectId": id, "title": "Renamed Full"})));
        assert_eq!(got["name"], "Renamed Full");
        assert_eq!(name_of(&path), "Renamed Full");
        assert!(bus_since(n).iter().any(|e| e["kind"] == "projects_changed"));
        assert!(cards(&a).is_empty());
        let row = a.log_snapshot().into_iter().rev().find(|e| e["kind"] == "apptool").unwrap();
        assert_eq!(row["links"][0]["kind"], "project");
        crate::app_mcp::drop_chat(&a);
        projects::forget(&path);
    }

    #[test]
    fn project_tools_below_full_auto_ask() {
        let _serial = serial();
        let (id, path) = registered("ask");
        let before = name_of(&path);
        let a = caller(false);
        let card = answer_card(&a, true);
        ok(&call(&a, "project_update", json!({"projectId": id, "title": "Renamed Ask"})));
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["allow"].as_str()), (Some("Rename project"), Some("Rename it")));
        assert_eq!(card["text"], format!("Rename \"{before}\" to \"Renamed Ask\". Its folder keeps its own name."));
        assert_eq!(name_of(&path), "Renamed Ask");
        let card = answer_card(&a, false);
        let (code, _) = err(&call(&a, "project_update", json!({"projectId": id, "kind": "roblox"})));
        assert_eq!(code, "declined");
        assert_eq!(card.join().unwrap()["text"], "Make \"Renamed Ask\" a Roblox project.");
        assert_eq!(projects::describe(&path)["kind"], "plain", "unchanged");
        crate::app_mcp::drop_chat(&a);
        projects::forget(&path);
    }

    #[test]
    fn project_delete_always_asks_and_keeps_the_folder() {
        let _serial = serial();
        let (id, path) = registered("delete");
        let a = caller(true);
        // a chat in it makes it nonempty
        let other = crate::app_mcp::test_chat("claude");
        assert!(other.set_project(&path));
        let (code, msg) = err(&call(&a, "project_delete", json!({"projectId": id})));
        assert_eq!((code.as_str(), msg.as_str()), ("invalid_request", "This project has chats. Pass force: true to remove it anyway; its folder and chats stay."));
        other.set_running_for_test(true);
        assert_eq!(err(&call(&a, "project_delete", json!({"projectId": id, "force": true}))).0, "thread_not_sendable");
        other.set_running_for_test(false);
        let card = answer_card(&a, true);
        let got = ok(&call(&a, "project_delete", json!({"projectId": id, "force": true})));
        assert_eq!(got, json!({"projectId": id, "removed": true}));
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["allow"].as_str()), (Some("Remove project"), Some("Remove it")));
        assert!(!projects::known(&path));
        assert!(std::path::Path::new(&path).is_dir(), "the folder stays");
        crate::app_mcp::drop_chat(&other);
        crate::app_mcp::drop_chat(&a);
    }

    #[test]
    fn project_create_existing_folder_always_asks() {
        let _serial = serial();
        let dir = project_dir("existing");
        let a = caller(true);
        let card = answer_card(&a, true);
        let got = ok(&call(&a, "project_create", json!({"title": "Existing", "workspaceRoot": dir, "kind": "plain"})));
        let card = card.join().unwrap();
        assert_eq!((card["title"].as_str(), card["allow"].as_str()), (Some("Add a project"), Some("Add it")));
        assert!(projects::known(&dir));
        assert_eq!(got["projectId"], project_id(&dir));
        let (code, msg) = err(&call(&a, "project_create", json!({"title": "Existing", "workspaceRoot": dir})));
        assert_eq!((code.as_str(), msg.as_str()), ("invalid_request", "That folder is already a project."));
        projects::forget(&dir);
        // a folder that is not there yet, made on request (Full auto: no card)
        let parent = project_dir("made");
        let made = std::path::Path::new(&parent).join("Fresh One").display().to_string();
        assert_eq!(err(&call(&a, "project_create", json!({"title": "Fresh One", "workspaceRoot": made}))).0, "invalid_request");
        let n = cards(&a).len();
        let got = ok(&call(&a, "project_create", json!({"title": "Fresh One", "workspaceRoot": made, "createWorkspaceRootIfMissing": true})));
        assert_eq!(cards(&a).len(), n, "Full auto: no card for a new folder");
        assert!(std::path::Path::new(&made).is_dir());
        projects::forget(got["path"].as_str().unwrap());
        crate::app_mcp::drop_chat(&a);
    }
}
