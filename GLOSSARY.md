# herdr-linear

A terminal pane inside Herdr for working through Linear issues and reading their linked GitHub PRs without leaving the terminal.

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

**Open issue**:
An issue whose Issue state is not of the completed, canceled, or duplicate kind. Backlog and triage issues are open.
_Avoid_: Active issue, incomplete issue

**Account tab**:
The view of one Account's open issues in the current Issue scope.
_Avoid_: Workspace tab

**Issue scope**:
Which open issues the app shows: Assigned issues or Team issues. One Issue scope applies to every Account tab and the All-accounts queue.
_Avoid_: Filter, mode, view

**Assigned issues**:
An Account's open issues that are assigned to its user. The default Issue scope.
_Avoid_: My issues, mine

**Team issues**:
The open issues of every Linear team the Account's user belongs to, whoever is assigned, plus the Account's Assigned issues from any other team.
_Avoid_: All issues, all tasks

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
The human activity of reviewing a PR. The app supports reading and, when enabled, merging PRs, but does not approve or comment. Merging is off by default.
