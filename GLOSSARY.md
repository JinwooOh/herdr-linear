# herdr-linear

A terminal pane inside Herdr for working through assigned Linear issues and reading their linked GitHub PRs without leaving the terminal.

## Language

**Account**:
One Linear login (for example work or personal) with its own API key. Issues, errors, and refreshes are tracked per Account.
_Avoid_: Workspace, tab

**Herdr workspace**:
A workspace of the Herdr terminal multiplexer that hosts the pane. Unrelated to Linear.
_Avoid_: Workspace (unqualified), Account

**Issue state**:
A Linear workflow state of an issue (for example In Review, Done). Each Linear team defines its own set.
_Avoid_: State (unqualified), status

**Account tab**:
The view of one Account's open issues.
_Avoid_: Workspace tab

**All-accounts queue**:
The merged list of open issues from every Account, shown alongside the Account tabs. Each entry keeps its Account of origin.
_Avoid_: All view, merged view, all-workspaces

**In Review**:
The Issue state meaning work is waiting on review. In-review issues sort first in every list.
_Avoid_: Review (unqualified)

**Linked PR**:
A GitHub pull request attached to an issue in Linear, identified by its URL rather than by the attachment's source type. An issue can have several.
_Avoid_: Attached PR

**PR picker**:
The choice list shown when an issue has more than one Linked PR.

**PR view**:
The in-app screen for reading one Linked PR's changed files and their diffs. Leaving it returns to the issue list.
_Avoid_: Review view, review screen

**Full-file view**:
A mode of the PR view that shows one changed file in full, at the PR's head commit, instead of its diff.
_Avoid_: Full view, file view

**Review**:
The human activity of reviewing a PR. The app supports reading only, not approving or commenting.
