# herdr-linear

Linear issues and GitHub PR review in a [Herdr](https://herdr.dev) pane. Rust + `ratatui`.
See [GLOSSARY.md](GLOSSARY.md) for the domain terms used below.

## Screenshots

### Account tab

Assigned Linear issues and their Linked PRs. Personal branch and Linear URL fields are hidden
in this screenshot.

![Account tab showing a Linear issue and its Linked PR](docs/images/account-tab.png)

### PR view

Changed files on the left and the selected file's diff on the right.

![PR view showing changed files and a colored diff](docs/images/pr-view.png)

## Features

- Open issues per Account in two **Issue scopes**: **Assigned issues** (the default) and
  **Team issues** (every open issue of your teams, plus your Assigned issues from other teams);
  **In Review** first in both
- Assignee on every row (`you`, display name, or `Unassigned`) and in the detail pane
- Work/personal Account tabs and an **All-accounts queue**, with Account provenance
- Background startup, manual refresh, and 60-second polling of the current Issue scope only;
  selection preserved by issue ID in each scope
- Paginated Linear issue loading (not limited to the first 100)
- PR picker when an issue has several Linked PRs
- Background PR diffs, file navigation, vertical and horizontal scrolling
- Full-file view with syntax highlighting, fetched at the PR's pinned head commit
- Team-specific Linear state picker and background state changes
- PR readiness (checks, review decision, conflicts) and optional, explicitly confirmed merging

The app reads PRs and can merge them when enabled; it does not approve or comment on PRs.
Merging is **off by default**. The app does not write the linked Linear issue's Issue state
when merging; independently configured Linear/GitHub automation may update it.

Polling uses a complete membership snapshot to remove completed, unassigned, or deleted issues;
`updatedAt` identifies unchanged issue objects. One Account failure doesn't disable the others,
and a failed refresh retains the last successful list. The All-accounts queue uses Account + issue ID,
not potentially colliding issue identifiers.

One Issue scope applies to every Account tab and the All-accounts queue, and the app always starts
on Assigned issues. Team issues load the first time you press `t`. After that, each scope keeps its
own list, selection, and errors; the hidden scope starts no requests, but a fetch already running
for it still updates its stored list.

## Requirements

- macOS (Linux and Windows are not validated or advertised)
- Herdr: verified with 0.9.5; the manifest's `min_herdr_version = "0.7.5"` is the original
  build target, not a tested compatibility floor
- Rust 1.100 or newer with Cargo on `PATH` (see `rust-version` in `Cargo.toml`)
- [`gh`](https://cli.github.com) authenticated for the repositories you review (`gh auth status`),
  with repository merge permission if you opt in to merging
- One Linear personal API key per Account, with write access if you want to change issue states
- A real terminal/Herdr pane for the interactive app (Pi's command runner has no TTY)

## Install as a Herdr plugin

Prerequisites: macOS, Herdr, the Rust toolchain above, authenticated `gh`, and one Linear API key
per Account. Herdr builds this plugin from source; it does not install Rust or `gh` for you.
Ensure `cargo` is on the `PATH` inherited by Herdr (restart Herdr after installing Rust if needed).

Install the public repository directly from GitHub. A fresh GitHub-managed install, its Cargo
build, and the CLI launcher action have been verified on macOS with Herdr 0.9.5; both configured
Accounts loaded.

```sh
herdr plugin install JinwooOh/herdr-linear
```

Herdr checks out the source in a managed directory and runs `cargo build --release` before
registering the plugin. Review the displayed build/runtime commands before confirming.
Do not put keys or configuration in that managed checkout: updates replace it.

Next, follow [Configure](#configure) to create your Account config and key files, then use
[Open the pane](#open-the-pane). For local development, use [Local development](#local-development)
instead of installing over a linked checkout.

## Configure

For standalone use, copy `config.example.toml` to `~/.config/herdr-linear/config.toml`.
For plugin use, put it in the directory printed by:

```sh
herdr plugin config-dir herdr-linear
```

Each Account (one Linear login) can use `key_env` or `key_file`. For example:

```toml
# Global options: place before any [[accounts]] table.
enable_merge = false
merge_method = "squash"

[[accounts]]
name = "work"
key_file = "~/.config/herdr-linear/work.key"

[[accounts]]
name = "personal"
key_file = "~/.config/herdr-linear/personal.key"
```

Save only the corresponding API key in each key file, outside this repository, then:

```sh
chmod 600 ~/.config/herdr-linear/{work,personal}.key
```

Never put credentials in `config.toml`. Without a config file, the app falls back to
`LINEAR_API_KEY` (or the plugin config directory's `api_key` file).

Merge options are global, not per Account, and belong **before** the first `[[accounts]]` table:

| Option | Type / default | Meaning |
| --- | --- | --- |
| `enable_merge` | Boolean, `false` | Set to `true` to enable `m` in the PR view; restart the app after changing config |
| `merge_method` | String, `"squash"` | Preferred method: `"merge"`, `"squash"`, or `"rebase"` only |

Only methods allowed by the repository appear in the picker. If the preferred method is unavailable,
`squash` is preselected when allowed, otherwise the first allowed method. A preference is not an
automatic merge. Omitting these options (including the no-config fallback) keeps merging disabled.

## Build and run

```sh
scripts/check                  # quality gate: clippy (warnings are errors), tests, release build
cargo build --release
./target/release/herdr-linear
```

The interactive app must run in a real terminal, not through Pi's non-TTY command runner.
Restart existing panes after rebuilding; they don't hot-reload the binary.

### Issue list

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move |
| `g` / `G` | Top / bottom |
| `Tab`, `Shift+Tab` | Cycle Account tabs and the All-accounts queue |
| `1`–`9` | Select an Account tab |
| `a` | Open the All-accounts queue |
| `Enter` | Open a Linked PR in the PR view; choose one if there are several |
| `s` | Pick a new issue state for its team |
| `t` | Toggle the Issue scope between Assigned issues and Team issues |
| `r` | Refresh the current Issue scope of this Account, or of all Accounts in the All-accounts queue |
| `q`, `Esc` | Quit (`Esc` first cancels a pending PR load) |
| `Ctrl+C` | Quit from any view (waits while a submitted merge is being verified) |

PR and state pickers: `j/k` or arrows to select, `Enter` to open/apply, `Esc` or `q` to cancel.
States are listed in workflow order (backlog → started → done → canceled), with the current state
preselected and marked. Choosing the current state does nothing. Done, Canceled, and Duplicate
require pressing `y` to confirm; any other key returns to the picker. State changes are submitted
only after that. A submitted mutation cannot be canceled; its
result is routed to the original Account even if you switch Account tabs. Errors leave the original
state intact. If the request times out, the outcome is reported as unknown and the Account is
refreshed to show the real state. Completed/canceled issues disappear from the open list after success.
After a state change the Account refreshes its current Issue scope, even if you pressed `t` meanwhile.

### PR view

| Key | Action |
| --- | --- |
| `j` / `k`, up/down | Change file |
| `d` / `u`, PageDown/PageUp | Scroll vertically |
| `h` / `l`, left/right | Scroll horizontally |
| `f` | Toggle Full-file view and diff mode (toggling again retries a failed load) |
| `r` | Refresh PR readiness without replacing the pinned diff |
| `m` | Choose a merge method (only when `enable_merge = true`) |
| `Esc`, `q`, Backspace | Return to issues |

Content and highlighting for Full-file view load off the UI thread and are cached for the current PR view.
Deleted files show their base-commit contents; other files use the head commit, including fork PRs.
Renamed files use their new path. Binary/non-UTF-8, unavailable, submodule, and files over 2 MB
show an explanation instead of crashing. Unknown languages fall back to plain text.
If the PR changes during diff loading, retry to obtain a consistent snapshot. Each `gh` call has a
30-second deadline, and only one full-file load runs at a time. Invisible bidi and zero-width
characters are shown as `<U+XXXX>` (or `�`) rather than rendered silently.

While a PR is loading, a centered box shows the PR, an animated bar, and elapsed time; after a
few seconds it adds a note that large PRs can take a while. The box is modal: `Esc` cancels and
discards the pending result, `q`/`Ctrl+C` quit, and other keys are ignored. Full-file loads and the
state list use the same indicator.
The underlying `gh` process finishes independently; cancellation does not kill it. Issue fetching,
PR fetching, full-file loading, readiness checks, merging, and state changes all happen outside
the terminal event loop.

### Optional merging

With merging enabled, press `m` in the PR view. Use `j/k` or arrows to choose an allowed method,
then `Enter` to inspect the confirmation's repository, PR number, title, and method. Only lowercase
`y` submits; any other key returns to the method picker. From the picker, `Esc`, `q`, or Backspace
cancels without changing the diff. Once submitted, a merge cannot be canceled by navigating away.
You can still read files and return to the issue list, but quitting from the issue list or with
`Ctrl+C` waits until the submitted write and its immediate outcome check finish. This keeps the
worker's deadline and result verification supervised; the pane remains responsive.

The app blocks drafts, closed/already-merged PRs, known conflicts, and unavailable/refreshing PR
status. Checks and review decisions are advisory: GitHub enforces required reviews, checks,
permissions, and branch protections. The app never bypasses protections, requests auto-merge, or
deletes the branch. Each `gh` call has a 30-second deadline.

The merge targets the pinned head commit you read, not a newer push. **PR changed, reload** means
leave and reopen the PR to load and review its new diff; `r` refreshes status only. Repository-method
load failures say to cancel and retry; an empty method list cannot submit. **Merge refused** shows
the first line of GitHub's refusal; resolve the cause and refresh status before considering a retry.

A successful command alone is not proof of a merge: the app re-fetches PR state. Only a verified
`MERGED` state displays **Merged · merging disabled**, retaining the PR view for reading. GitHub
may instead queue the PR. **Merge outcome unknown** or **Merge not confirmed** means press `r`
to refresh status and **do not resubmit**, even after a timeout or connection error. The app never
automatically retries a merge and keeps unresolved/queued PRs blocked from resubmission for the
current app session, including after reopening the PR. Do not restart merely to bypass that guard;
check the actual GitHub state first. Merging does not approve or comment. The merge flow never
submits a Linear Issue-state mutation; any configured GitHub/Linear state automation is separate.

## Open the pane

From a real terminal inside Herdr, invoke the registered action:

```sh
herdr plugin action invoke herdr-linear.open-issues
```

Or open the pane directly:

```sh
herdr plugin pane open --plugin herdr-linear --entrypoint issues --placement split --focus
```

`scripts/open.sh` focuses an existing Linear pane in the current tab instead of opening a duplicate.

To open the pane in its own tab instead, invoke `herdr-linear.open-issues-tab`
(`scripts/open-tab.sh`). It focuses a Linear pane in the current tab, otherwise switches to a tab
in the same Herdr workspace that already has one, otherwise opens a new tab. A Linear pane in
another Herdr workspace is left alone. Neither launcher closes the pane when invoked again.

Optional launcher binding in `~/.config/herdr/config.toml` (choose an unused key first):

```toml
[[keys.command]]
key = "prefix+i"
type = "plugin_action"
command = "herdr-linear.open-issues"
description = "open Linear pane"

[[keys.command]]
key = "prefix+shift+i"
type = "plugin_action"
command = "herdr-linear.open-issues-tab"
description = "open Linear pane in tab"
```

Then run `herdr server reload-config`. This is an example, not an automatically installed binding.
With a `ctrl+a` prefix, the example is `Ctrl+A`, then `i` (or `Shift+I` for the tab).

## Local development

`plugin link` registers your checkout but **does not build it**. Build before linking and after changes:

```sh
git clone https://github.com/JinwooOh/herdr-linear.git
cd herdr-linear
cargo build --release
herdr plugin link "$PWD"
```

The same plugin config directory is used for linked and GitHub-managed installs; keep key files
outside the checkout. To switch from an installed plugin to local development, first unregister
that installation with `herdr plugin uninstall herdr-linear`, then link the checkout.

## Update and restart

For a GitHub-managed install:

```sh
herdr plugin update herdr-linear
```

For a linked development checkout, pull the desired code and run `cargo build --release` instead.
Rebuilding a separate development checkout does **not** update a GitHub-managed installation;
use `herdr plugin update herdr-linear` after changes reach the installed GitHub ref.
Close existing Linear panes with `q` and reopen them: updating/rebuilding does not restart them,
and the launcher deliberately focuses an existing pane rather than replacing it.

## Manual smoke test

1. Launch a fresh pane and confirm work and personal Accounts load independently in their Account tabs.
2. Press `a`: the All-accounts queue shows each issue's Account; `r` refreshes all Accounts without freezing.
3. Press `t`, then check both the work and the personal Account tab: the tab bar shows `Team`, and
   each Account loads its Team issues. In each Account's Team issues, confirm that a teammate's issue and an issue
   assigned to you in another team (one you are not a member of) both appear. Press `t` again: the
   Assigned list and its selection come back at once.
4. Select an issue with multiple Linked PRs, press `Enter`, choose a Linked PR, and test cancellation/retry.
5. In the PR view, test `f` for Full-file view, `h/l`, `d/u`, and changing files; return with `Esc`.
6. On a disposable test issue, press `s`, cancel once, then select/apply a state. Verify in Linear.
7. With the default config, confirm `m` in the PR view does nothing. For an explicitly authorized
   merge smoke test, use only a **disposable PR in a throwaway repository** you control, linked to a
   disposable Linear issue. Set top-level `enable_merge = true` in your private test config and restart.
   Open that PR, inspect readiness (`r`) and the diff, press `m`, and verify only repository-allowed
   methods appear. Cancel once; then choose a method, press `Enter`, verify the confirmation identity,
   and press `y` to merge. Confirm **Merged · merging disabled** and independently verify the PR is
   merged in GitHub and its branch remains. The app must not submit a Linear Issue-state mutation;
   account for any separately configured GitHub/Linear state automation. If the outcome is
   unknown/not confirmed (including queued), refresh and inspect GitHub; do not submit again.
   Restore `enable_merge = false` and restart afterward. This is a manual write test, not an automated
   test or a claim that live merging has been validated.
8. Test a missing/invalid key: the other Account must still work.
9. Invoke **Open Linear issues** from the Herdr UI launcher (or its configured keybinding).
   Repeat it and confirm it focuses the existing Linear pane instead of creating a duplicate.

### Fresh GitHub-install smoke test

This replaces your plugin registration. Keep a local-checkout path available for recovery, and
confirm your config/key files are outside the plugin root. Uninstalling preserves config/state,
but back up local configuration privately before testing; never copy keys into this repository.

```sh
herdr plugin uninstall herdr-linear
herdr plugin install JinwooOh/herdr-linear
herdr plugin config-dir herdr-linear
```

Confirm the managed checkout's build completes, then invoke **Open Linear issues** from Herdr's
UI launcher and verify both configured Accounts load. Repeat the interactive checks above.
If installation fails, restore local development with `herdr plugin link /path/to/herdr-linear`.
Fresh uninstall/install, the managed-checkout build, CLI action launch, and both Account loads
have been verified on macOS / Herdr 0.9.5. The maintainer also confirmed that invoking the action
through Herdr's UI keybinding opens and focuses the Linear pane. No action `contexts` field was
needed on that version. Older Herdr releases, Linux, and Windows are not validated.

Read-only GitHub integration test (optional; requires `gh` authentication):

```sh
HERDR_LINEAR_TEST_PR=https://github.com/OWNER/REPO/pull/NUMBER \
  cargo test --release github_live_full_file_snapshot -- --ignored
```

## Tracking

Development is tracked in Linear; PRs reference their HL issue identifiers. The maintainer's
private project URL is intentionally not part of these public-facing instructions.
Automated checks do not substitute for the manual smoke tests above. Webhooks and push
notifications remain out of scope; polling is sufficient.

## License

[MIT](LICENSE).
