### Task 5: GitHub plumbing for the vault

**Files:**
- Create: `app-rs/src/vault_git.rs`
- Modify: `app-rs/src/github.rs` (make `http`, `api`, `read_token`, `user` and `OFFLINE` crate-visible; `StandIn` gains `raw: String`
  and `vault_repo: Option<String>`; add `raw_base()`), `app-rs/src/paths.rs` (`VAULT_REPO`), `app-rs/src/main.rs`
  (`mod`)
- Test: `app-rs/src/vault_git.rs` tests (a stub GitHub on 127.0.0.1, like `github.rs`'s tests)

**Interfaces:**
- Produces (all `pub(crate)`, every error a plain sentence):
  - `vault_repo() -> String`: `VAULT_REPO`, or the stand-in's in tests.
  - `latest_tag(repo) -> Result<Option<String>, String>`: `GET /repos/{repo}/releases/latest` with no token. A 404
    gives `Ok(None)`, meaning no release yet.
  - `raw(repo, git_ref, path) -> Result<Vec<u8>, String>`: each path segment is percent-encoded. It stops reading
    past 1 MiB plus 1 byte.
  - `get(path, auth: bool) -> Result<(u16, Value), String>`: an authenticated call fails with `NOT_CONNECTED` when no
    token is saved.
  - `head_sha(repo, branch) -> Result<String, String>`.
  - `commit_files(repo, branch, files: &[(String, Vec<u8>)], deletes: &[String], message) -> Result<String, String>`.
    It makes blobs (base64), a tree on the head's tree, and a commit with the head as parent, then sends
    `PATCH .../git/refs/heads/{branch}` with `force: false`. It returns the new commit's sha.
  - `ensure_fork(upstream) -> Result<String, String>`: `POST /repos/{upstream}/forks` when the user has no fork, then
    polls `GET /repos/{login}/{name}` every 2 s for up to 60 s. It then calls
    `POST /repos/{fork}/merge-upstream {"branch": "main"}` and returns `"{login}/{name}"`.
  - `new_branch(repo, branch, sha) -> Result<(), String>`.
  - `open_pr(upstream, head: &str /* "login:branch" */, title, body) -> Result<(u64, String), String>`, returning the
    number and the html URL.
  - `pr_files(repo, n) -> Result<Vec<Value>, String>`: paged at 100, up to 300 files.
  - `blob(repo, sha) -> Result<Vec<u8>, String>`.
  - `comment_and_close(repo, n, text) -> Result<(), String>`.
  - `create_release(repo, tag, sha, name, body) -> Result<String, String>`.

- [ ] **Step 1: Write the failing tests** against a stub that records requests:
  - `latest_tag_none_when_no_release` (404) and `latest_tag_reads_tag_name`.
  - `raw_is_encoded_and_capped`: path `Skills/a b/SKILL.md` requests `/o/r/vault-1/Skills/a%20b/SKILL.md`. A body of
    1 MiB plus 10 bytes gives `Err`.
  - `commit_files_builds_tree_on_head`: the requests go in order (ref, commit, blobs, tree with `base_tree`, commit,
    ref patch with `force:false`). A delete sends a tree entry with `"sha": null`.
  - `fork_waits_until_ready`: the stub answers 404 twice, then 200. `merge-upstream` is called.
  - `no_token_never_calls_out`: with no token saved, `get(_, true)` fails with `NOT_CONNECTED` and the stub sees no
    request.
  - `test_bases_are_empty_without_stand_in`: in `cfg(test)` with `STAND_IN` unset, `raw_base()`, `api_base()` and
    `web_base()` are `""`, so no test can reach GitHub.
- [ ] **Step 2:** Run `cargo test --manifest-path app-rs/Cargo.toml vault_git::`. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces over `github::http()`/`api()`. Nothing logs a header or a body that came
  with a token.
- [ ] **Step 4:** Run `cargo test --manifest-path app-rs/Cargo.toml vault_git:: github::`. Expected: PASS.
- [ ] **Step 5:** Commit: `Vault: GitHub's release, raw-file and git-data calls, token kept in the app`.

