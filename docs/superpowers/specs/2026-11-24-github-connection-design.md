# Connect GitHub to Fridae

Asked for by the user on 2026-11-24: "make connecting github to the harness one of the first things you do. as i dont
even think thats in as a feature currently." It is not: today a project only has a box to paste a remote address
(Project details), and nothing signs in to GitHub. The community vault (`2026-11-24-unified-knowledge-vault-design.md`)
needs it, so GitHub is built first.

## Behaviour

- **Setup → Connections → GitHub**, laid out like the AI sign-in rows:
  - **Connect** starts GitHub's device sign-in. The row shows the one-time code with a **Copy** button and opens
    `https://github.com/login/device` in the browser. Fridae polls until the user approves, then shows
    "Connected as <login>" with an avatar.
  - **Disconnect** deletes Fridae's copy of the sign-in and links to GitHub's settings page where the user can revoke
    the app entirely.
- **What the sign-in is used for:**
  1. **Projects on GitHub.** Project details gets **Put on GitHub**, which creates a repository under the user's
     account (private by default, named after the project, with a name field to change it), sets it as the project's
     remote and pushes. Afterwards **Push now** sends the latest commits. The existing paste-an-address box stays for
     other remotes. Creating a repo or pushing happens only when the user presses the button (the standing rule:
     Fridae never publishes without the user's yes). Agents cannot push or create repos by themselves.
  2. **Backing up Fridae itself** (the code repo), for the maintainer.
  3. **The community vault**, later: sending submissions as pull requests, and the maintainer's Submissions inbox.
- **The token stays with the app:**
  - It is stored in the app's own Application Support folder with mode 0600, and that path goes on
    `paths::app_protected()`.
  - It is never put in an agent's environment, a tool server's config, argv, a log, or the page.
  - Git operations that need it run in a git process the app starts itself, with the credential supplied through a
    one-shot `GIT_ASKPASS` helper reading from the app.
- **Scopes:** `repo` (create and push private repositories) and `read:user`. The connect row says this in one plain
  line.
- **The user's own git and gh setup is never touched.** That means `~/.gitconfig`, the system git credential helpers,
  and `gh`'s login.

## Needs the user once

GitHub's device sign-in needs an OAuth App registered by the maintainer. Its Client ID is public and gets built into
Fridae; no client secret is made or used. To register it, go to
`https://github.com/settings/applications/new` and fill in:
- Application name: Fridae Studios
- Homepage URL and callback URL: `https://github.com/aprilfridae/fridae-studios`
- Tick **Enable Device Flow**, then **Register application**, and copy the Client ID.

## Anti-goals
- A token anywhere an agent can read it.
- Pushing or creating a repository without the user pressing the button.
- Changing the user's own git, gh or keychain setup.

## Tests
- Device flow against a stub GitHub: the code shown, polling with GitHub's `interval` and `slow_down` respected,
  `expired_token` and `access_denied` surfaced plainly, and the token saved with mode 0600.
- The token never appears in any agent's env or tool config, and its file is protected.
- Put on GitHub against a stub API: the repo is created private by default, the remote is set and the push uses the
  askpass helper. A name clash is surfaced plainly.
- Disconnect removes the file.
