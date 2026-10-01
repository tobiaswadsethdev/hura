# Tickets

What your trackers' filters say, in the window, as a board: one column per
filter, one card per ticket, and a button on each card that turns it into a
session. The **trackers** button in the screen's header switches to where each
tracker is set up — where it points, its token and its filters — and back.

```
 ┌ ready to start      3 ┐ ┌ assigned to me      2 ┐ ┌ needs my review     1 ┐
 │ PROJ-12 Story   Ready │ │ PROJ-7  Bug   In Prog │ │ PROJ-28 Task  In Rev. │
 │ Add a changelog to …  │ │ Retry uploads that …  │ │ Cache the dashboard … │
 │ updated 3h ago        │ │ updated 1h · 4 · Sam  │ │ updated 5h · 3 · Jor… │
 │ [web-app ▾]   ▷ start │ │ [web-app ▾]   ▷ start │ │ [web-app ▾]   ▷ start │
 └───────────────────────┘ └───────────────────────┘ └───────────────────────┘
```

Columns sit side by side and each scrolls on its own, so another filter costs
width rather than a page of scrolling. A card shows the key, its type and
status, the title, how long since it last changed, and how many comments it has
and who wrote the newest one.

## Reading a ticket

**Click a Jira card** (or focus it and press Enter) and the ticket opens beside
the board: its status, assignee, reporter, priority, parent, labels and dates,
the description, and the comments oldest first. The board stays where it was,
so picking the next card does not mean closing this one first, and the start
button is at the bottom of the panel as well as on the card. Escape closes the
panel; a second Escape closes the screen. The ↗ in the panel's header, and
*Open in the browser* in a card's right-click menu, still go to Jira -- for the
things only Jira does: editing, moving, attaching.

It is one `GET /rest/api/3/issue/{key}` with the same token as the board. The
description and comments arrive in Jira's document format and are drawn from
that rather than from Jira's HTML: a comment is something anyone on the project
can write, and markup from it is never put into the window. A link is a link
only when it is `http`, `https` or `mailto`; an image or attachment is shown by
name, since fetching it would need the token in a request the window does not
make; and people are their initials rather than their avatars, for the same
reason.

GitHub and Azure DevOps cards still link out to the browser from their key.

GitHub, Azure DevOps and Jira, read over REST **by the desktop application,
with tokens kept on the computer it runs on**. The server is not involved: it
holds no tracker, no token and no ticket, and needs no setup for any of this.

## REST here, MCP there

The agent may also have a Jira MCP server; this is not that, and the difference
is deliberate. REST is for what the *interface* shows: a list, on a timer,
rendered as rows. MCP is for what the *agent* gets: a tool it calls when it
decides to — which is also how a ticket gets a comment or moves along, if you
want the agent to do it. This screen only reads.

## Setting one up

On the tickets screen (the ticket icon in the header), **trackers** in the
screen's own header lists your trackers and, under them, **add a tracker**. With
no trackers yet, the screen opens there:

| Tracker | Asks for | Token |
| --- | --- | --- |
| Jira | the site (`https://your-org.atlassian.net`) and the email of the account the token belongs to | an API token, from id.atlassian.com → Security → API tokens |
| Azure DevOps | the organisation and the project | a personal access token with *Work Items (read)* |
| GitHub | a repository (`owner/name`), or nothing for every issue assigned to you | a token that can read issues |

A **name** is optional, and only worth giving a second tracker of the same kind.

The token is stored on this computer only — `%LOCALAPPDATA%\hura\trackers.json`
on Windows, `~/.local/state/hura/trackers.json` on Linux, readable by you alone —
and the window is never shown it again. A tracker whose token has expired has a
field on its row to paste a new one; removing a tracker removes its token with
it.

A tracker that could not be read — a token that is missing or refused, a site
that did not answer — says so above the list rather than leaving its rows
quietly missing, which would look exactly like having nothing assigned.

## Filters

A Jira or Azure DevOps tracker can have several **named filters**, each its own
section of the tickets screen: "ready to start", "assigned to me", whatever your
process has a question for. A ticket two filters match is in both sections.

Each tracker's row, under **trackers**, lists the filters it runs, with edit,
remove and **add filter**. A tracker with none runs one called `assigned to me` — assigned to you
and not done — and the list starts from it, so adding a second filter adds a
section rather than replacing the one you had. Remove them all and it goes back
to that one.

The query is JQL for Jira and WIQL for Azure DevOps:

```
ready to start   project = PROJ AND status = "Ready" AND assignee is EMPTY
assigned to me   assignee = currentUser() AND statusCategory != Done
```

GitHub trackers take no filters: they list what is assigned to you.

## Notifications

The window reads your tickets every three minutes and sends an OS notification
when a ticket in one of your filters:

* **changes status** — `PROJ-7 · To Do → In Progress`;
* **gets a comment from somebody else** — your own are never announced, which is
  why Jira is asked who the token belongs to;
* **turns up in a filter** it was not in before — `PROJ-12 · now in ready to
  start`.

Other edits are deliberately left out: a notification for every re-estimate is
one nobody reads by Friday. More than three changed tickets at once — the window
opening after a weekend — is one notification listing them.

What was seen is kept in the window's own storage, so a ticket that moved while
the window was closed is announced when it opens. The first read is the baseline
and says nothing, and so is the first read of a filter you have just added. A
tracker that could not be read is not a list of tickets that left it, and it
coming back is not a list of arrivals.

It is **notify me when a ticket in my filters changes** on the settings screen.
Turned off, the timer stops too, so nothing asks your tracker on the window's
behalf.

## What a ticket becomes

Starting from a row fills in three things:

| | |
| --- | --- |
| the task | `PROJ-123: Add the changelog` and the ticket's URL, so the agent's first instruction says why it exists |
| the session name | `proj-123-add-the-changelog`, cut to what a session name may be |
| the branch | `<branch_prefix>/PROJ-123-add-the-changelog` — the key keeps its case, because a tracker's commit hooks and your reviewers both look for `PROJ-123` |

`branch_prefix` is the server's, from its settings screen, because it names every
session's branch there including the ones started from a terminal. It is the one
thing the tickets screen asks the server for; without an answer the branch uses
the default `hura` prefix and a warning says so.

**A ticket does not know which repository it is about.** A Jira issue names a
project and a work item names an area path; neither is a clone URL, and guessing
from a name would be wrong in exactly the cases where it matters. So the row
carries a project chooser: the tracker says what to do and you say where.

The session records which ticket it came from — tracker, key and link — as a
note about where the work came from. Nothing writes back through it.

## What it costs

This computer holds a token that can read your tickets, in the same private
state directory as the tokens for the servers it is paired with. Scope it to
reading — *Work Items (read)*, a read-only GitHub token — rather than reusing an
administrative one: reading is all this does.

---

[← Documentation](README.md) · [README](../README.md)
