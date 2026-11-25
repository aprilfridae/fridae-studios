//! The vault's GitHub calls: the latest release, raw files, and the git-data and pull-request calls that sharing,
//! the maintainer's inbox and releases are built on. The token is read from `github` inside each call and goes only
//! into the `Authorization` header; nothing here logs, stores or returns it, and every error is a plain sentence.
//!
//! Nothing calls these yet but the tests; the tasks that use them (downloads, sharing, the inbox, releases) come next.
#![allow(dead_code)]

use std::{
    io::Read,
    time::{Duration, Instant},
};

use base64::Engine;
use serde_json::{Value, json};

use crate::{
    github::{self, NOT_CONNECTED, OFFLINE},
    paths,
};

/// The most a raw file may be: 1 MiB.
const RAW_MAX: u64 = 1024 * 1024;
/// How many pages of a pull request's files are read (100 each: up to 300 files).
const PR_FILES_PAGES: u32 = 3;
#[cfg(not(test))]
const FORK_POLL: Duration = Duration::from_secs(2);
#[cfg(test)]
const FORK_POLL: Duration = Duration::from_millis(10);
/// The most the wait for a new fork may take, in all: 60 s.
#[cfg(not(test))]
const FORK_WAIT: Duration = Duration::from_secs(60);
#[cfg(test)]
const FORK_WAIT: Duration = Duration::from_millis(300);

/// The vault's repository ("owner/name"); in tests the stand-in's, empty when none.
pub(crate) fn vault_repo() -> String {
    #[cfg(test)]
    return github::STAND_IN.lock().unwrap().as_ref().and_then(|s| s.vault_repo.clone()).unwrap_or_default();
    #[allow(unreachable_code)]
    paths::VAULT_REPO.to_string()
}

/// A path, percent-encoded segment by segment (the `/` between segments kept).
fn enc(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn sentence(what: &str, code: u16, v: &Value) -> String {
    let said = github::said(v);
    let said = said.trim_end_matches('.');
    match code {
        401 => "GitHub no longer accepts Fridae's sign-in. Disconnect GitHub in Setup, Connections, and connect again.".to_string(),
        403 | 429 if said.to_lowercase().contains("rate limit") => "GitHub says there have been too many requests. Try again in a few minutes.".to_string(),
        // never GitHub's own words: they are not written for this person
        _ => format!("GitHub would not {what} (error {code})."),
    }
}

/// A GET to the API: with the saved token when `auth`, else with none. An authenticated call with no token saved
/// fails with `NOT_CONNECTED` before anything is sent.
pub(crate) fn get(path: &str, auth: bool) -> Result<(u16, Value), String> {
    if auth {
        let token = github::read_token().ok_or_else(|| NOT_CONNECTED.to_string())?;
        return github::api("GET", path, &token, None);
    }
    let url = format!("{}{path}", github::api_base());
    github::answer(github::http().get(&url).header("Accept", "application/vnd.github+json").header("User-Agent", "fridae-studios").header("X-GitHub-Api-Version", "2022-11-28").call())
}

/// An authenticated call that must succeed; its JSON answer.
pub(crate) fn call(method: &str, path: &str, body: Option<Value>, what: &str) -> Result<Value, String> {
    call_as(method, path, body, what, &[])
}

/// `call`, where some answers have a sentence of their own (never GitHub's own words): (status, sentence).
pub(crate) fn call_as(method: &str, path: &str, body: Option<Value>, what: &str, own: &[(u16, &str)]) -> Result<Value, String> {
    let token = github::read_token().ok_or_else(|| NOT_CONNECTED.to_string())?;
    let (code, v) = github::api(method, path, &token, body)?;
    if (200..300).contains(&code) {
        return Ok(v);
    }
    Err(own.iter().find(|(c, _)| *c == code).map(|(_, s)| s.to_string()).unwrap_or_else(|| sentence(what, code, &v)))
}

pub(crate) fn unreadable(what: &str) -> String {
    format!("GitHub gave an answer about {what} that Fridae could not read.")
}

/// The tag of the vault's latest release; `None` when there is no release yet. No token.
pub(crate) fn latest_tag(repo: &str) -> Result<Option<String>, String> {
    let (code, v) = get(&format!("/repos/{repo}/releases/latest"), false)?;
    match code {
        404 => Ok(None),
        200 => v["tag_name"].as_str().filter(|t| !t.is_empty()).map(|t| Some(t.to_string())).ok_or_else(|| unreadable("the latest vault release")),
        _ => Err(sentence("say which vault release is the latest", code, &v)),
    }
}

/// One file of the repository at a tag or branch, no token, at most 1 MiB.
pub(crate) fn raw(repo: &str, git_ref: &str, path: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}/{repo}/{}/{}", github::raw_base(), enc(git_ref), enc(path));
    let mut resp = github::http().get(&url).header("User-Agent", "fridae-studios").call().map_err(|_| OFFLINE.to_string())?;
    let code = resp.status().as_u16();
    if code != 200 {
        return Err(format!("GitHub does not have the file {path} (error {code})."));
    }
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().take(RAW_MAX + 1).read_to_end(&mut bytes).map_err(|_| OFFLINE.to_string())?;
    if bytes.len() as u64 > RAW_MAX {
        return Err(format!("The file {path} is bigger than 1 MB, so Fridae did not take it."));
    }
    Ok(bytes)
}

/// The sha of a branch's newest commit.
pub(crate) fn head_sha(repo: &str, branch: &str) -> Result<String, String> {
    let v = call("GET", &format!("/repos/{repo}/git/ref/heads/{}", enc(branch)), None, "say where the branch is")?;
    v["object"]["sha"].as_str().map(str::to_string).ok_or_else(|| unreadable("the branch"))
}

/// Commit `files` (added or replaced) and `deletes` onto `branch` as one commit, moving the branch only forward
/// (`force: false`). Returns the new commit's sha.
pub(crate) fn commit_files(repo: &str, branch: &str, files: &[(String, Vec<u8>)], deletes: &[String], message: &str) -> Result<String, String> {
    let head = head_sha(repo, branch)?;
    let commit = call("GET", &format!("/repos/{repo}/git/commits/{head}"), None, "read the branch's newest commit")?;
    let base = commit["tree"]["sha"].as_str().ok_or_else(|| unreadable("the commit"))?.to_string();
    let mut tree = Vec::new();
    for (path, bytes) in files {
        let body = json!({"content": base64::engine::general_purpose::STANDARD.encode(bytes), "encoding": "base64"});
        let blob = call("POST", &format!("/repos/{repo}/git/blobs"), Some(body), "take a file")?;
        let sha = blob["sha"].as_str().ok_or_else(|| unreadable("a file"))?;
        tree.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": sha}));
    }
    for path in deletes {
        tree.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": Value::Null}));
    }
    let made = call("POST", &format!("/repos/{repo}/git/trees"), Some(json!({"base_tree": base, "tree": tree})), "build the new files")?;
    let tree_sha = made["sha"].as_str().ok_or_else(|| unreadable("the new files"))?;
    let new = call("POST", &format!("/repos/{repo}/git/commits"), Some(json!({"message": message, "tree": tree_sha, "parents": [head]})), "make the commit")?;
    let sha = new["sha"].as_str().ok_or_else(|| unreadable("the commit"))?.to_string();
    let changed = "The vault changed while Fridae was saving. Try again.";
    call_as("PATCH", &format!("/repos/{repo}/git/refs/heads/{}", enc(branch)), Some(json!({"sha": sha, "force": false})), "move the branch to the new commit", &[(409, changed), (422, changed)])?;
    Ok(sha)
}

/// The user's fork of `upstream`: made when they have none (waiting up to `FORK_WAIT` in all for GitHub to finish,
/// asking the name GitHub gave the fork), then brought up to date with the upstream's main. Returns "login/name".
pub(crate) fn ensure_fork(upstream: &str) -> Result<String, String> {
    let name = upstream.split_once('/').map(|x| x.1).filter(|n| !n.is_empty()).ok_or_else(|| "The vault's address is not valid.".to_string())?;
    let login = match github::user()["login"].as_str() {
        Some(l) => l.to_string(),
        None => call("GET", "/user", None, "say who is signed in")?["login"].as_str().ok_or_else(|| unreadable("who is signed in"))?.to_string(),
    };
    let mut fork = format!("{login}/{name}");
    if get(&format!("/repos/{fork}"), true)?.0 != 200 {
        let made = call("POST", &format!("/repos/{upstream}/forks"), Some(json!({})), "make a copy of the vault for you")?;
        if let Some(given) = made["full_name"].as_str().filter(|n| fork_name_ok(n)) {
            fork = given.to_string();
        }
        let deadline = Instant::now() + FORK_WAIT;
        loop {
            if get(&format!("/repos/{fork}"), true)?.0 == 200 {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("GitHub is still making your copy of the vault. Try again in a minute.".to_string());
            }
            std::thread::sleep(FORK_POLL.min(left));
        }
    }
    call("POST", &format!("/repos/{fork}/merge-upstream"), Some(json!({"branch": "main"})), "bring your copy of the vault up to date")?;
    Ok(fork)
}

/// "owner/name" as GitHub writes one: letters, digits, `-`, `_` and `.`.
fn fork_name_ok(n: &str) -> bool {
    n.split_once('/').is_some_and(|(o, r)| [o, r].iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))))
}

/// A new branch at `sha`.
pub(crate) fn new_branch(repo: &str, branch: &str, sha: &str) -> Result<(), String> {
    let taken = "A branch for this send already exists. Wait a minute and try again.";
    call_as("POST", &format!("/repos/{repo}/git/refs"), Some(json!({"ref": format!("refs/heads/{branch}"), "sha": sha})), "make the branch", &[(422, taken)])?;
    Ok(())
}

/// A pull request into the upstream's main; `head` is "login:branch". Returns its number and web address.
pub(crate) fn open_pr(upstream: &str, head: &str, title: &str, body: &str) -> Result<(u64, String), String> {
    let refused = "GitHub would not open the pull request. Try again later.";
    let v = call_as("POST", &format!("/repos/{upstream}/pulls"), Some(json!({"title": title, "head": head, "base": "main", "body": body})), "open the pull request", &[(422, refused)])?;
    match (v["number"].as_u64(), v["html_url"].as_str()) {
        (Some(n), Some(url)) => Ok((n, url.to_string())),
        _ => Err(unreadable("the pull request")),
    }
}

/// The files of a pull request, up to 300. More than that is an error, never a list cut short: a submission whose
/// files cannot all be seen cannot be reviewed.
pub(crate) fn pr_files(repo: &str, n: u64) -> Result<Vec<Value>, String> {
    let mut all = Vec::new();
    // one page past the last one read, to see whether there is more
    for page in 1..=PR_FILES_PAGES + 1 {
        let v = call("GET", &format!("/repos/{repo}/pulls/{n}/files?per_page=100&page={page}"), None, "list the files of the pull request")?;
        let items = v.as_array().ok_or_else(|| unreadable("the files of the pull request"))?;
        if page > PR_FILES_PAGES {
            if items.is_empty() {
                break;
            }
            return Err("This submission has too many files to review in Fridae.".to_string());
        }
        let full = items.len() >= 100;
        all.extend(items.iter().cloned());
        if !full {
            break;
        }
    }
    Ok(all)
}

/// The paths of every file in a commit's tree. A tree GitHub cut short is an error, never a shorter list.
pub(crate) fn tree_paths(repo: &str, sha: &str) -> Result<Vec<String>, String> {
    let v = call("GET", &format!("/repos/{repo}/git/trees/{sha}?recursive=1"), None, "list the submission's files")?;
    if v["truncated"] == true {
        return Err("This submission has too many files for Fridae to check against the vault.".to_string());
    }
    let tree = v["tree"].as_array().ok_or_else(|| unreadable("the submission's files"))?;
    Ok(tree.iter().filter(|e| e["type"] == "blob").filter_map(|e| e["path"].as_str().map(str::to_string)).collect())
}

/// The most a blob may be: 1 MiB (no file of a release, and none a submission may hold, is bigger).
pub(crate) const BLOB_MAX: u64 = 1024 * 1024;

/// A blob as read: its bytes, or only its size when that is over the limit it was read with.
#[derive(Debug)]
pub(crate) enum Blob {
    Bytes(Vec<u8>),
    TooBig(u64),
}

/// A blob by its sha, decoded only when it is at most `max` bytes. An answer that is not base64 `content` of exactly
/// the `size` GitHub gives (one cut off, or one with no content) is an error, never empty bytes.
pub(crate) fn blob_within(repo: &str, sha: &str, max: u64) -> Result<Blob, String> {
    let v = call("GET", &format!("/repos/{repo}/git/blobs/{sha}"), None, "send a file")?;
    let size = v["size"].as_u64().ok_or_else(|| unreadable("a file"))?;
    if size > max {
        return Ok(Blob::TooBig(size));
    }
    let (Some("base64"), Some(content)) = (v["encoding"].as_str(), v["content"].as_str()) else {
        return Err(unreadable("a file"));
    };
    let text: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| unreadable("a file"))?;
    if bytes.len() as u64 == size { Ok(Blob::Bytes(bytes)) } else { Err(unreadable("a file")) }
}

/// A file's bytes by its blob sha: at most `BLOB_MAX`, checked as `blob_within` checks it.
pub(crate) fn blob(repo: &str, sha: &str) -> Result<Vec<u8>, String> {
    match blob_within(repo, sha, BLOB_MAX)? {
        Blob::Bytes(b) => Ok(b),
        Blob::TooBig(_) => Err("A file on GitHub is too big for Fridae to read.".to_string()),
    }
}

/// Leave a comment on a pull request and close it.
pub(crate) fn comment_and_close(repo: &str, n: u64, text: &str) -> Result<(), String> {
    call("POST", &format!("/repos/{repo}/issues/{n}/comments"), Some(json!({"body": text})), "add the comment")?;
    call("PATCH", &format!("/repos/{repo}/pulls/{n}"), Some(json!({"state": "closed"})), "close the pull request")?;
    Ok(())
}

/// A release at `sha`. Returns its web address.
pub(crate) fn create_release(repo: &str, tag: &str, sha: &str, name: &str, body: &str) -> Result<String, String> {
    let v = call("POST", &format!("/repos/{repo}/releases"), Some(json!({"tag_name": tag, "target_commitish": sha, "name": name, "body": body})), "publish the release")?;
    v["html_url"].as_str().map(str::to_string).ok_or_else(|| unreadable("the release"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{STAND_IN, StandIn, TEST_LOCK};
    use std::sync::{Arc, Mutex, MutexGuard};

    /// Each request the stub had: (method, path, body).
    type Log = Arc<Mutex<Vec<(String, String, String)>>>;
    /// (method, path, body, how many times this method and path were asked, this one included) -> (status, body)
    type Answer = Box<dyn Fn(&str, &str, &str, usize) -> (u16, String) + Send + Sync>;

    struct Stub {
        log: Log,
    }

    const TOKEN: &str = "gho_vaulttest0123456789";

    fn stub(answer: Answer) -> (String, Stub) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
        let log: Log = Default::default();
        let l = log.clone();
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let method = req.method().as_str().to_string();
                let path = req.url().to_string();
                let mut body = Vec::new();
                let _ = req.as_reader().read_to_end(&mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let seen = {
                    let mut g = l.lock().unwrap();
                    g.push((method.clone(), path.clone(), body.clone()));
                    g.iter().filter(|(m, p, _)| *m == method && *p == path).count()
                };
                let (code, text) = answer(&method, &path, &body, seen);
                let _ = req.respond(tiny_http::Response::from_string(text).with_status_code(code));
            }
        });
        (url, Stub { log })
    }

    fn setup(answer: Answer, token: bool) -> (MutexGuard<'static, ()>, Stub) {
        let held = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, s) = stub(answer);
        *STAND_IN.lock().unwrap() = Some(StandIn { web: url.clone(), api: url.clone(), raw: url, vault_repo: Some("o/r".into()), ..Default::default() });
        github::disconnect();
        if token {
            github::save_token(TOKEN).unwrap();
        }
        (held, s)
    }

    fn requests(s: &Stub) -> Vec<(String, String, String)> {
        s.log.lock().unwrap().clone()
    }

    #[test]
    fn latest_tag_none_when_no_release() {
        let (_g, s) = setup(Box::new(|_, _, _, _| (404, r#"{"message":"Not Found"}"#.into())), false);
        assert_eq!(latest_tag("o/r"), Ok(None));
        assert_eq!(requests(&s)[0].1, "/repos/o/r/releases/latest");
    }

    #[test]
    fn latest_tag_reads_tag_name() {
        let (_g, _s) = setup(Box::new(|_, _, _, _| (200, r#"{"tag_name":"vault-2026.11.24"}"#.into())), false);
        assert_eq!(latest_tag("o/r"), Ok(Some("vault-2026.11.24".to_string())));
        assert_eq!(vault_repo(), "o/r");
    }

    #[test]
    fn raw_is_encoded_and_capped() {
        let (_g, s) = setup(
            Box::new(|_, p, _, _| {
                if p.ends_with("big.txt") {
                    (200, "x".repeat(1024 * 1024 + 10))
                } else if p.ends_with("exact.txt") {
                    (200, "y".repeat(1024 * 1024))
                } else {
                    (200, "hello".into())
                }
            }),
            false,
        );
        assert_eq!(raw("o/r", "vault-1", "Skills/a b/SKILL.md").unwrap(), b"hello");
        assert_eq!(requests(&s)[0].1, "/o/r/vault-1/Skills/a%20b/SKILL.md");
        assert!(raw("o/r", "vault-1", "big.txt").is_err());
        assert_eq!(raw("o/r", "vault-1", "exact.txt").unwrap().len(), 1024 * 1024);
    }

    #[test]
    fn commit_files_builds_tree_on_head() {
        let (_g, s) = setup(
            Box::new(|m, p, _, _| match (m, p) {
                ("GET", "/repos/o/r/git/ref/heads/main") => (200, r#"{"object":{"sha":"head1"}}"#.into()),
                ("GET", "/repos/o/r/git/commits/head1") => (200, r#"{"tree":{"sha":"tree0"}}"#.into()),
                ("POST", "/repos/o/r/git/blobs") => (201, r#"{"sha":"blob1"}"#.into()),
                ("POST", "/repos/o/r/git/trees") => (201, r#"{"sha":"tree1"}"#.into()),
                ("POST", "/repos/o/r/git/commits") => (201, r#"{"sha":"commit2"}"#.into()),
                ("PATCH", "/repos/o/r/git/refs/heads/main") => (200, "{}".into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        let sha = commit_files("o/r", "main", &[("Skills/a.md".into(), b"hi".to_vec())], &["Skills/old.md".into()], "Add a").unwrap();
        assert_eq!(sha, "commit2");
        let r = requests(&s);
        let order: Vec<(&str, &str)> = r.iter().map(|x| (x.0.as_str(), x.1.as_str())).collect();
        assert_eq!(
            order,
            vec![
                ("GET", "/repos/o/r/git/ref/heads/main"),
                ("GET", "/repos/o/r/git/commits/head1"),
                ("POST", "/repos/o/r/git/blobs"),
                ("POST", "/repos/o/r/git/trees"),
                ("POST", "/repos/o/r/git/commits"),
                ("PATCH", "/repos/o/r/git/refs/heads/main"),
            ]
        );
        let blob: Value = serde_json::from_str(&r[2].2).unwrap();
        assert_eq!(blob["encoding"], "base64");
        assert_eq!(blob["content"], base64::engine::general_purpose::STANDARD.encode(b"hi"));
        let tree: Value = serde_json::from_str(&r[3].2).unwrap();
        assert_eq!(tree["base_tree"], "tree0");
        assert_eq!(tree["tree"][0]["sha"], "blob1");
        assert_eq!(tree["tree"][1]["path"], "Skills/old.md");
        assert!(tree["tree"][1].as_object().unwrap().get("sha").is_some_and(Value::is_null));
        let commit: Value = serde_json::from_str(&r[4].2).unwrap();
        assert_eq!((commit["tree"].as_str(), commit["parents"][0].as_str(), commit["message"].as_str()), (Some("tree1"), Some("head1"), Some("Add a")));
        let patch: Value = serde_json::from_str(&r[5].2).unwrap();
        assert_eq!((patch["sha"].as_str(), patch["force"].as_bool()), (Some("commit2"), Some(false)));
    }

    #[test]
    fn fork_waits_until_ready() {
        let (_g, s) = setup(
            Box::new(|m, p, _, seen| match (m, p) {
                ("GET", "/user") => (200, r#"{"login":"me"}"#.into()),
                ("GET", "/repos/me/r") if seen <= 2 => (404, r#"{"message":"Not Found"}"#.into()),
                ("GET", "/repos/me/r") => (200, r#"{"full_name":"me/r"}"#.into()),
                ("POST", "/repos/o/r/forks") => (202, "{}".into()),
                ("POST", "/repos/me/r/merge-upstream") => (200, "{}".into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        assert_eq!(ensure_fork("o/r"), Ok("me/r".to_string()));
        let r = requests(&s);
        let calls: Vec<String> = r.iter().map(|x| format!("{} {}", x.0, x.1)).collect();
        assert_eq!(calls.iter().filter(|c| *c == "GET /repos/me/r").count(), 3, "{calls:?}");
        assert!(calls.contains(&"POST /repos/o/r/forks".to_string()), "{calls:?}");
        assert_eq!(calls.last().unwrap(), "POST /repos/me/r/merge-upstream");
        assert_eq!(serde_json::from_str::<Value>(&r.last().unwrap().2).unwrap()["branch"], "main");
    }

    #[test]
    fn fork_is_polled_by_the_name_github_gave() {
        let (_g, s) = setup(
            Box::new(|m, p, _, seen| match (m, p) {
                ("GET", "/user") => (200, r#"{"login":"me"}"#.into()),
                ("GET", "/repos/me/r") => (404, r#"{"message":"Not Found"}"#.into()),
                ("POST", "/repos/o/r/forks") => (202, r#"{"full_name":"me/r-1"}"#.into()),
                ("GET", "/repos/me/r-1") if seen <= 1 => (404, "{}".into()),
                ("GET", "/repos/me/r-1") => (200, "{}".into()),
                ("POST", "/repos/me/r-1/merge-upstream") => (200, "{}".into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        assert_eq!(ensure_fork("o/r"), Ok("me/r-1".to_string()));
        let calls: Vec<String> = requests(&s).iter().map(|x| format!("{} {}", x.0, x.1)).collect();
        assert_eq!(calls.iter().filter(|c| *c == "GET /repos/me/r-1").count(), 2, "{calls:?}");
        assert_eq!(calls.iter().filter(|c| *c == "GET /repos/me/r").count(), 1, "the first look only: {calls:?}");
    }

    #[test]
    fn fork_gives_up_at_the_deadline_without_a_last_sleep() {
        let (_g, s) = setup(
            Box::new(|m, p, _, _| match (m, p) {
                ("GET", "/user") => (200, r#"{"login":"me"}"#.into()),
                ("POST", "/repos/o/r/forks") => (202, "{}".into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        let started = std::time::Instant::now();
        let err = ensure_fork("o/r").unwrap_err();
        let took = started.elapsed();
        assert_eq!(err, "GitHub is still making your copy of the vault. Try again in a minute.");
        assert!(took >= FORK_WAIT && took < FORK_WAIT * 3, "{took:?}");
        let polls = requests(&s).iter().filter(|x| x.1 == "/repos/me/r").count();
        assert!(polls >= 3 && polls <= (FORK_WAIT.as_millis() / FORK_POLL.as_millis()) as usize + 3, "{polls}");
        assert!(!requests(&s).iter().any(|x| x.1.ends_with("merge-upstream")));
    }

    #[test]
    fn errors_a_send_can_hit_are_plain_sentences() {
        let raw = r#"{"message":"Reference update failed: raw GitHub text"}"#;
        let (_g, _s) = setup(
            Box::new(move |m, p, _, _| match (m, p) {
                ("GET", "/repos/o/r/git/ref/heads/main") => (200, r#"{"object":{"sha":"head1"}}"#.into()),
                ("GET", "/repos/o/r/git/commits/head1") => (200, r#"{"tree":{"sha":"tree0"}}"#.into()),
                ("POST", "/repos/o/r/git/blobs") => (201, r#"{"sha":"blob1"}"#.into()),
                ("POST", "/repos/o/r/git/trees") => (201, r#"{"sha":"tree1"}"#.into()),
                ("POST", "/repos/o/r/git/commits") => (201, r#"{"sha":"commit2"}"#.into()),
                ("PATCH", "/repos/o/r/git/refs/heads/main") => (409, raw.into()),
                ("PATCH", "/repos/o/r/git/refs/heads/other") => (422, raw.into()),
                ("POST", "/repos/o/r/pulls") => (422, r#"{"message":"Validation Failed: raw GitHub text","errors":[{"message":"x"}]}"#.into()),
                ("POST", "/repos/o/r/git/refs") => (422, raw.into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        let files = [("Skills/a.md".to_string(), b"hi".to_vec())];
        let changed = "The vault changed while Fridae was saving. Try again.";
        assert_eq!(commit_files("o/r", "main", &files, &[], "m"), Err(changed.to_string()));
        assert_eq!(open_pr("o/r", "me:b", "t", "b"), Err("GitHub would not open the pull request. Try again later.".to_string()));
        assert_eq!(new_branch("o/r", "b", "sha"), Err("A branch for this send already exists. Wait a minute and try again.".to_string()));
        for e in [commit_files("o/r", "main", &files, &[], "m").unwrap_err(), open_pr("o/r", "me:b", "t", "b").unwrap_err()] {
            assert!(!e.contains("raw GitHub text"), "{e}");
        }
    }

    #[test]
    fn unmapped_errors_never_quote_github() {
        let (_g, _s) = setup(
            Box::new(|m, p, _, _| match (m, p) {
                ("GET", "/repos/o/r/git/ref/heads/main") => (500, r#"{"message":"Server blew up: raw GitHub text"}"#.into()),
                ("GET", "/repos/o/r/git/ref/heads/ok") => (200, r#"{"object":{"sha":"head1"}}"#.into()),
                ("GET", "/repos/o/r/git/commits/head1") => (200, r#"{"tree":{"sha":"tree0"}}"#.into()),
                ("POST", "/repos/o/r/git/blobs") => (422, r#"{"message":"Invalid blob: raw GitHub text"}"#.into()),
                ("POST", "/repos/o/r/forks") => (403, r#"{"message":"Forbidden: raw GitHub text"}"#.into()),
                _ => (404, "{}".into()),
            }),
            true,
        );
        let files = [("Skills/a.md".to_string(), b"hi".to_vec())];
        let errs = [head_sha("o/r", "main").unwrap_err(), commit_files("o/r", "ok", &files, &[], "m").unwrap_err(), call("POST", "/repos/o/r/forks", None, "make a copy").unwrap_err()];
        for e in &errs {
            assert!(!e.contains("raw GitHub text"), "{e}");
        }
        assert_eq!(errs[0], "GitHub would not say where the branch is (error 500).");
        assert_eq!(errs[1], "GitHub would not take a file (error 422).");
    }

    #[test]
    fn pr_files_never_returns_a_cut_list() {
        let page = |n: usize, from: usize| -> String { serde_json::to_string(&(0..n).map(|i| json!({"filename": format!("Skills/a/f{}.md", from + i)})).collect::<Vec<_>>()).unwrap() };
        // exactly 300 files: all of them
        let (_g, s) = setup(
            Box::new(move |_, p, _, _| {
                let pg: usize = p.rsplit("page=").next().unwrap().parse().unwrap();
                (200, if pg <= 3 { page(100, (pg - 1) * 100) } else { "[]".into() })
            }),
            true,
        );
        assert_eq!(pr_files("o/r", 4).unwrap().len(), 300);
        assert_eq!(requests(&s).len(), 4, "the page after the last full one is looked at too");
        // more than 300: an error, never the first 300
        let (_g2, _s2) = {
            drop(_g);
            setup(Box::new(move |_, _, _, _| (200, page(100, 0))), true)
        };
        assert_eq!(pr_files("o/r", 4), Err("This submission has too many files to review in Fridae.".to_string()));
    }

    #[test]
    fn pr_files_answer_that_is_not_a_list_is_an_error() {
        let (_g, _s) = setup(Box::new(|_, _, _, _| (200, r#"{"message":"surprise"}"#.into())), true);
        let e = pr_files("o/r", 4).unwrap_err();
        assert_eq!(e, "GitHub gave an answer about the files of the pull request that Fridae could not read.");
    }

    #[test]
    fn blob_is_base64_content_of_its_size_or_an_error() {
        let b64 = |t: &[u8]| base64::engine::general_purpose::STANDARD.encode(t);
        let good = json!({"content": b64(b"hello"), "encoding": "base64", "size": 5}).to_string();
        let empty = json!({"content": "", "encoding": "base64", "size": 0}).to_string();
        let short = json!({"content": b64(b"hel"), "encoding": "base64", "size": 5}).to_string();
        let utf8 = json!({"content": "hello", "encoding": "utf-8", "size": 5}).to_string();
        let no_content = json!({"encoding": "base64", "size": 5}).to_string();
        let no_size = json!({"content": b64(b"hello"), "encoding": "base64"}).to_string();
        let huge = json!({"content": b64(b"hello"), "encoding": "base64", "size": 2 * 1024 * 1024}).to_string();
        let (_g, _s) = setup(
            Box::new(move |_, p, _, _| {
                let text = match p.rsplit('/').next().unwrap() {
                    "good" => good.clone(),
                    "empty" => empty.clone(),
                    "short" => short.clone(),
                    "utf8" => utf8.clone(),
                    "nocontent" => no_content.clone(),
                    "nosize" => no_size.clone(),
                    "huge" => huge.clone(),
                    // an answer that could not be read whole (ureq stops at 10 MB) is no answer
                    _ => String::new(),
                };
                (200, text)
            }),
            true,
        );
        assert_eq!(blob("o/r", "good").unwrap(), b"hello");
        assert_eq!(blob("o/r", "empty").unwrap(), b"");
        for bad in ["short", "utf8", "nocontent", "nosize", "huge", "cut"] {
            assert!(blob("o/r", bad).is_err(), "{bad}");
        }
        assert!(matches!(blob_within("o/r", "huge", BLOB_MAX), Ok(Blob::TooBig(n)) if n == 2 * 1024 * 1024));
        assert!(matches!(blob_within("o/r", "good", 4), Ok(Blob::TooBig(5))));
        assert!(matches!(blob_within("o/r", "good", 5), Ok(Blob::Bytes(b)) if b == b"hello"));
    }

    #[test]
    fn no_token_never_calls_out() {
        let (_g, s) = setup(Box::new(|_, _, _, _| (200, "{}".into())), false);
        assert_eq!(get("/repos/o/r", true), Err(NOT_CONNECTED.to_string()));
        assert_eq!(head_sha("o/r", "main"), Err(NOT_CONNECTED.to_string()));
        assert!(requests(&s).is_empty());
    }

    #[test]
    fn test_bases_are_empty_without_stand_in() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STAND_IN.lock().unwrap() = None;
        assert_eq!((github::raw_base(), github::api_base(), github::web_base(), vault_repo()), (String::new(), String::new(), String::new(), String::new()));
    }
}
