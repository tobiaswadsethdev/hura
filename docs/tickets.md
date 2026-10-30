# Tickets and pull requests

What is yours on your trackers, in a sidebar on the far right of the window:
the tickets assigned to you or created by you, and the pull requests you opened
or that are waiting on you. It is there whatever worktree is selected and
whatever screen is open, because "what is mine right now?" is not a question
about the worktree in front of you.

```
 ┌ mine                          ⟳ ▸ ┐
 │ TICKETS                         4 │
 │ ⌄ assigned to me                2 │
 │   PROJ-7  Bug        In Progress  │
 │   Retry uploads that time out …   │
 │   1h ago · 4 · Sam   ● retry-up…  │
 │   PROJ-12 Story            Ready  │
 │ ⌄ created by me                 2 │
 │   PROJ-28 Task             To Do  │
 │ PULL REQUESTS                   4 │
 │ ⌄ waiting on me                 1 │
 │   Spoon-Knife #61   ◌ review      │
 │ ⌄ created by me                 3 │
 │   Hello-World #58   ✓ approved    │
 │   Shop !26056       changes       │
 └───────────────────────────────────┘
```

Each heading has two groups. A ticket that is both assigned to you and created
by you is in **assigned to me**, because that is the half that asks something
of you. A pull request you opened and assigned to yourself is in **created by
me**, because assigning yourself your own pull request does not make it a
review. Within a group, the newest change is first. A group's name shuts it,
and the window remembers which ones you shut.

**Ctrl+Shift+T** (⌘⇧T on macOS) folds the sidebar to a 40-pixel rail with two
counts and opens it again, from anywhere in the window, a terminal included.
Ctrl+T on its own is left to the terminal, where it transposes two characters.
The sidebar's left edge drags, like the other two sidebars, and its width is
also on the settings screen.

## A row

A **ticket** shows its key, type and status, its title, how long since it last
changed, and how many comments it has and who wrote the newest one.

A **pull request** shows the repository and number, whether its checks pass
(✓), fail (✕) or are still running (◌), and where its reviews stand: approved,
changes asked for, or still in review. A draft has the draft mark and says so;
one that will not merge as it stands says **conflicts**. On the ones waiting on
you it says who opened it, and whether it is a review asked of you or the
assignee field.

**A row knows its worktree.** A ticket a session was started from, or one whose
key a worktree's branch is named after (`tobias/PROJ-7-retry-uploads`), shows
that worktree with its agent's state; so does a pull request a session was
started from, or whose branch a worktree in the same repository is working on.
A click on it selects the worktree. A row with no worktree has a start button
instead (see [What a ticket becomes](#what-a-ticket-becomes) and [What a pull
request becomes](#what-a-pull-request-becomes)). The tree says it the other way
round: a worktree on one of these pull requests' branches has the pull
request's number and checks on its branch line, and a click on them opens it.

**Click a Jira ticket** (or focus it and press Enter) and it opens in full
beside the sidebar, over the dock: its status, assignee, reporter, priority,
parent, labels and dates, the description, and the comments oldest first.
Escape closes it, and so does clicking the row again. The ↗ in its header, and
*Open in the browser* in a row's right-click menu, still go to Jira for what
only Jira does: attaching, linking, logging work. GitHub and Azure DevOps
tickets open in the browser; pull requests open here, as [Reading and
reviewing a pull request](#reading-and-reviewing-a-pull-request) describes.

It is one `GET /rest/api/3/issue/{key}` with the tracker's own token. The
description and comments arrive in Jira's document format and are drawn from
that rather than from Jira's HTML: a comment is something anyone on the project
can write, and markup from it is never put into the window. A link is a link
only when it is `http`, `https` or `mailto`; an image or attachment is shown by
name, since fetching it would need the token in a request the window does not
make; and people are their initials rather than their avatars, for the same
reason.

## What is asked

Every tracker is asked the same question. There are no filters to write: the
sidebar means the same thing whichever tracker it reads.

| Tracker | Tickets | Pull requests |
| --- | --- | --- |
| Jira | `(assignee = currentUser() OR reporter = currentUser()) AND statusCategory != Done`, from `/rest/api/3/search/jql`, after `/myself` to know which of them are assigned to you | none: Jira has none |
| Azure DevOps | WIQL for work items in the project assigned to `@Me` or created by `@Me` and not Closed, Done, Removed or Resolved, then the work items themselves | the project's active pull requests by creator and by reviewer, from `/_apis/git/pullrequests` |
| GitHub | `is:issue is:open` with `assignee:@me`, and with `author:@me` | `is:pr is:open` with `author:@me`, `review-requested:@me` and `assignee:@me` |

**GitHub is one GraphQL request** for all five searches, scoped to the
tracker's repository when it names one. GraphQL rather than REST because it is
the only way to ask in one answer whether each pull request's checks pass and
whether it is approved; over REST that is a request per pull request.
`review-requested` counts a review asked of a team you are on.

**Azure DevOps asks who the token belongs to first**, from `connectionData`:
the work items are sorted into assigned and created by it, and the pull
requests are searched by it. A review **waits on you until you vote**. Once
you have voted, it is the author's move, and it leaves the group, which is what
GitHub means by a review request too. Azure DevOps keeps checks behind a request
per pull request, so its rows have no checks mark, and its list has no time of
last change, so its rows say when they were opened.

GitHub, Azure DevOps and Jira are read over REST **by the desktop application,
with tokens kept on the computer it runs on**. The server is not involved: it
holds no tracker, no token and no ticket, and needs no setup for any of this.

A tracker that could not be read says so at the top of the sidebar rather than
leaving its rows quietly missing, which would look exactly like having nothing
assigned. An Azure DevOps token that can read work items but not code says
that its pull requests need *Code (read)*; its tickets still show.

## REST here, MCP there

The agent may also have a Jira MCP server; this is not that, and the difference
is deliberate. REST is for what the *interface* shows: a list, on a timer,
rendered as rows. MCP is for what the *agent* gets: a tool it calls when it
decides to, which is also how a ticket gets a comment or moves along, if you
want the agent to do it. The window changes a ticket only when you press
something, and never on a timer.

## Changing a ticket

Jira tickets can be changed from the panel, with the tracker's own token:

* **Status.** The status in the panel is a list of the workflow's transitions
  out of it, named by where each lands. A transition whose screen has required
  fields fails with Jira's own message.
* **Fields.** The pencil in the panel's header turns it into the ticket's edit
  screen: the fields Jira's edit screen has for it, each as its control (title
  and text, Markdown for rich text, numbers such as story points, dates,
  selects and multi-selects, labels, priority, and people, found by name). A
  field of any other type is shown read-only with its value, and a field this
  account may not change is not on the form. Only the fields you changed are
  saved, so whatever somebody else changed since the form opened is left
  alone. Ctrl+Enter saves; Escape leaves a form with nothing changed on it.
* **Comments.** A new one goes under the last, in Markdown, and Ctrl+Enter
  sends it. Your own comments have an edit and a delete (the delete asks
  first); other people's do not.

**Rich text is Markdown here and Jira's document format there.** Paragraphs,
headings, lists and checklists, code, quotes, rules, bold, italics,
strike-through, inline code and links go both ways intact. Tables, panels,
collapsible sections, mentions, status lozenges, dates and images do not exist
in Markdown, so they become their text, or nothing for an image. When a
description or comment holds any of those, the box says which, under it,
*before* you save. Saving from here flattens them, and Jira is where to edit it
if they matter.

| What | Request |
| --- | --- |
| the edit screen | `GET /rest/api/3/issue/{key}?expand=editmeta&fields=*all` |
| save | `PUT /rest/api/3/issue/{key}` with the changed fields |
| statuses | `GET` and `POST /rest/api/3/issue/{key}/transitions` |
| comments | `POST`, `PUT` and `DELETE /rest/api/3/issue/{key}/comment[/{id}]` |
| people | `GET /rest/api/3/user/assignable/search` for the assignee, `/user/search` otherwise |

GitHub and Azure DevOps tickets are still changed in the browser.

## Reading and reviewing a pull request

**Click a pull request** and it opens beside the sidebar like a ticket, wider,
because one of its tabs is a diff:

* **overview**: the description, every check by name (failing first, each with
  a link to its log), the reviewers and where each stands, and your own review;
* **conversation**: comments, reviews and threads on lines, oldest first. A
  resolved thread starts shut, showing its file, its line and who started it,
  since a long review is mostly settled threads;
* **files**: the files it changes with the lines added and removed, each opening
  to both sides in the diff editor the dock uses, with unchanged stretches
  folded away. Measured from where the branch left its base, the way the
  forge's own page measures it, so what landed on the base since is not shown
  as this pull request's.

**From here you can comment, reply to a thread, approve and ask for changes.**
On GitHub those are its two verdicts, and a request for changes needs a
comment, as on GitHub. Your own pull request offers neither, because GitHub
refuses a review of your own. Azure DevOps has its five votes (approve, approve
with suggestions, wait for the author, reject, and resetting yours), and a
comment written with one is posted beside it. Merging, resolving a thread and
suggesting a change stay on the forge, one click away in the header.

| What | GitHub | Azure DevOps |
| --- | --- | --- |
| the pull request, its checks, reviewers and conversation | one GraphQL query | `connectionData`, then `GET _apis/git/pullrequests/{id}`, its `threads` and `statuses`, and `_apis/policy/evaluations` for the branch policies |
| the files | `GET /repos/{repo}/pulls/{n}`, then `compare/{base}...{head}` | the last of its `iterations`, and that one's `changes` |
| both sides of a file | one GraphQL query for both blobs | `GET …/items` at each commit |
| a comment, a reply | `POST /repos/{repo}/issues/{n}/comments`, `…/pulls/{n}/comments/{id}/replies` | `POST …/threads`, `…/threads/{id}/comments` |
| a review, a vote | `POST /repos/{repo}/pulls/{n}/reviews` | `PUT …/reviewers/{you}` |

The conversation is the newest hundred comments and the newest hundred threads;
a longer one says how many more are on the forge. An Azure DevOps thread that
only records something happening (a reviewer joining, a branch updated) is left
out, except a vote, which is shown as the review it is. Its checks are two
requests of their own, and a pull request whose checks could not be read still
opens, saying so.

## Setting one up

Trackers are the **trackers** section of the settings screen. With none set up,
the sidebar says so, and its **add a tracker** opens the settings screen there.

| Tracker | Asks for | Token |
| --- | --- | --- |
| Jira | the site (`https://your-org.atlassian.net`) and the email of the account the token belongs to | an API token, from id.atlassian.com → Security → API tokens |
| Azure DevOps | the organisation and the project | a personal access token with *Work Items (read)* for the tickets and *Code (read)* for the pull requests, or *Code (read & write)* to comment and vote on them |
| GitHub | a repository (`owner/name`), or nothing for every repository the token can see | a token that can read issues and pull requests, and write pull requests to comment and review |

A **name** is optional, and only worth giving a second tracker of the same kind.

The token is stored on this computer only (`%LOCALAPPDATA%\hura\trackers.json`
on Windows, `~/.local/state/hura/trackers.json` on Linux, readable by you
alone) and the window is never shown it again. A tracker whose token has
expired has a field on its row to paste a new one; removing a tracker removes
its token with it. A file written when trackers had filters still loads: the
filters are ignored, and dropped the next time it is saved.

## Notifications

The window reads the sidebar every three minutes, and on the refresh button in
its header, and sends an OS notification when:

* a ticket **changes status**: `PROJ-7 · To Do → In Progress`;
* a ticket **gets a comment from somebody else**. Your own are never announced,
  which is why Jira is asked who the token belongs to;
* a ticket **becomes yours**: `PROJ-12 · assigned to you`;
* a pull request **starts waiting on you**: `Spoon-Knife #61 · review requested`;
* one of **your** pull requests' **checks fail**, or it is **approved**, or
  **changes are asked for**.

Other edits are deliberately left out: a notification for every re-estimate is
one nobody reads by Friday. More than three changes at once (the window opening
after a weekend) is one notification listing them.

What was seen is kept in the window's own storage, so a ticket that moved while
the window was closed is announced when it opens. The first read is the
baseline and says nothing, and so is the first read of a tracker you have just
added. A tracker that could not be read is not a list of things that left it,
and it coming back is not a list of arrivals.

It is **notify when my tickets or pull requests change** on the settings
screen. Turned off, the sidebar is still read on its timer, because it is on
screen; only the notifications stop.

## What a ticket becomes

Starting from a row (its start button, or *Start in …* in its right-click
menu, or **start** at the foot of the panel) fills in three things:

| | |
| --- | --- |
| the task | `PROJ-123: Add the changelog` and the ticket's URL, so the agent's first instruction says why it exists |
| the session name | `proj-123-add-the-changelog`, cut to what a session name may be |
| the branch | `<branch_prefix>/PROJ-123-add-the-changelog`. The key keeps its case, because a tracker's commit hooks and your reviewers both look for `PROJ-123` |

`branch_prefix` is the server's, from its settings screen, because it names every
session's branch there including the ones started from a terminal. It is the one
thing the sidebar asks the server for; without an answer the branch uses the
default `hura` prefix and a warning says so.

**A ticket does not know which repository it is about.** A Jira issue names a
project and a work item names an area path; neither is a clone URL, and guessing
from a name would be wrong in exactly the cases where it matters. So the start
button asks which project, the selected worktree's first; with one project it
starts there.

The session records which ticket it came from (tracker, key and link) as a
note about where the work came from, and that is how the row finds it again.
Nothing writes back through it.

## What a pull request becomes

A pull request's start button, *Work on it* and *Review it* in its right-click
menu, and the two buttons at the foot of its panel all open the create form in
a project that is a clone of its repository. A project cloned over SSH counts:
`git@github.com:o/r.git` is the repository its pull requests call
`https://github.com/o/r`. With no such project there is nothing to start in,
and the panel says so.

**Either way the worktree is on the pull request's own branch**, checked out
from the remote and tracking it, rather than a new branch made from its base.
The base is the branch it merges into, so the diff pane measures the whole of
the pull request and not only what the agent adds to it. If the branch has gone
from the remote by the time the sandbox clones (merged, say), the seeding fails
and says so rather than making a new branch of that name.

| | the task, filled in |
| --- | --- |
| **work on it** | its name, title and link, which branch this is and what it merges into, and that pushing updates the pull request; the instruction is yours to write under it |
| **review it** | the same, then the instruction: read the changes with `git diff origin/<base>...HEAD`, run the tests that cover them, and report what is wrong, then what is risky, then anything smaller, with files and lines; and do not commit, push or comment on the pull request |

A review that does not push is asked, not enforced: the session has the same
policy and credentials as any other, because running the tests usually needs
them. What goes on the pull request is still yours to say, from its panel.

**A pull request opened from a fork cannot be started on.** Its branch is in
the fork, which this worktree's clone does not have and could not push to, so
its row says *fork* and has no start button.

The session records which pull request it came from, beside where it records a
ticket, and that is how its row and its tree mark find it. The create goes to
the server as a request of its own, `create-from-pull`, so a server from before
this answers that it does not know it, and the form says to `hurad update` it,
rather than ignoring the pull request and making a new branch with its name.

## What it costs

This computer holds a token that can read your tickets, in the same private
state directory as the tokens for the servers it is paired with.

A Jira API token acts as you, so it can do whatever your account can, and the
window uses that to change tickets when you ask it to. On GitHub and Azure
DevOps it writes only a comment, a reply or a review, and only when you press
the button. Nothing else changes anything: the timer only reads. Scope those
tokens to what you use rather than reusing an administrative one: *Work Items
(read)* and *Code (read)*, or *Code (read & write)* to review from here, and a
GitHub token that can read issues and write pull requests and nothing more.

---

[← Documentation](README.md) · [README](../README.md)
