//! What changed on your trackers since the last read: the comparison behind
//! the desktop's and the phone's tracker notifications, made here once so
//! both say the same things in the same words, and so a service reading the
//! trackers for you can say them too.
//!
//! Tickets, four ways: its status moved, somebody else commented on it, it
//! became yours (assigned to you, where it was not), or it turned up in one of
//! your JQL filters. A ticket that is there only for a filter is watched for
//! the first two like your own: that is what watching a filter is for.
//! Anything else Jira counts as an update (a label, a sprint, a re-estimate)
//! is left alone, because a notification for every edit is one nobody reads
//! by the end of the week.
//!
//! Pull requests, four ways: a review was asked of you or one was assigned to
//! you, and on yours, the checks failed, it was approved, or changes were
//! asked for. Those are the four that are a reason to stop and look.
//!
//! **Compared with what was [`Seen`] at the last read**, which the caller
//! keeps between reads and hands back: nothing here stores anything. The very
//! first read is the baseline and announces nothing, and so is the first read
//! of a tracker added since.
//!
//! **Only trackers that were read take part.** A tracker that is down reads as
//! an empty list, and treating that as everything having left would make all
//! of it "new" when the tracker comes back. [`Inbox::read`] and
//! [`Inbox::pulls_read`] say which trackers actually answered; the rest keep
//! what they had. A filter is the same one level down, by
//! [`Inbox::filters_read`], except that one which fails is forgotten rather
//! than kept: its next answer is a baseline again, because a tracker that
//! answered has already replaced the tickets the filter held.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Checks, Inbox, PullRequest, Relation, Review, Task};

/// What was seen at the last read.
///
/// Opaque to a client, which keeps it as it was given and hands it back, so
/// it has no TypeScript binding.
///
/// **In camelCase**, unlike everything else here, because it is the shape the
/// desktop and the phone already kept before this was moved here: a snapshot
/// written by either still reads, and an update is not a quiet poll.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Seen {
    /// By ticket, keyed `tracker \0 id`.
    tickets: BTreeMap<String, SeenTicket>,
    /// By pull request, keyed `tracker \0 repo \0 id`.
    pulls: BTreeMap<String, SeenPull>,
    /// The trackers read at least once, each half on its own: a tracker's
    /// first answer is a baseline, not a list of arrivals.
    read: BTreeSet<String>,
    pulls_read: BTreeSet<String>,
    /// The JQL filters read at least once, keyed `tracker \0 filter`.
    filters_read: BTreeSet<String>,
}

/// What is remembered about one ticket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SeenTicket {
    tracker: String,
    status: String,
    comments: Option<u32>,
    relation: Relation,
    /// Missing from a snapshot written before there were filters.
    #[serde(default)]
    filters: Vec<String>,
}

/// What is remembered about one pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SeenPull {
    tracker: String,
    checks: Option<Checks>,
    review: Option<Review>,
    relation: Relation,
}

/// One change worth saying, about one ticket or pull request.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "InboxChange"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Which tracker, and its own id for the ticket or pull request: what a
    /// notification that is tapped opens.
    pub tracker: String,
    pub id: String,
    /// `PROJ-7`, or `Spoon-Knife #61`.
    pub key: String,
    pub title: String,
    /// `To Do → In Progress, Alex commented`.
    pub what: String,
}

/// Record a read: what is seen now, and what changed since `before`. With
/// nothing seen before, the read is the baseline and nothing has changed.
pub fn record(before: Option<&Seen>, inbox: &Inbox) -> (Seen, Vec<Change>) {
    let empty = Seen::default();
    let next = before.unwrap_or(&empty).taking(inbox);
    let changes = before.map(|b| b.changes(inbox)).unwrap_or_default();
    (next, changes)
}

/// What a pull request is called in a notification: the repository's own
/// name and its number, since a number alone is ambiguous across
/// repositories.
pub fn pull_name(p: &PullRequest) -> String {
    let repo = p.repo.rsplit('/').next().unwrap_or(&p.repo);
    format!("{repo} {}", p.key)
}

fn ticket_key(tracker: &str, id: &str) -> String {
    format!("{tracker}\0{id}")
}

fn pull_key(p: &PullRequest) -> String {
    format!("{}\0{}\0{}", p.tracker, p.repo, p.id)
}

fn filter_key(tracker: &str, filter: &str) -> String {
    format!("{tracker}\0{filter}")
}

impl Seen {
    /// Whether a ticket was in the last read of its tracker: for hura cloud,
    /// whether a webhook's delivery about it is about something of yours.
    pub fn has_ticket(&self, tracker: &str, id: &str) -> bool {
        self.tickets.contains_key(&ticket_key(tracker, id))
    }

    /// What this becomes once a read is taken into account.
    ///
    /// A tracker that was read is replaced wholesale, so what left it is
    /// forgotten and this does not grow with everything that ever passed
    /// through. A tracker that was not read keeps what it had.
    fn taking(&self, inbox: &Inbox) -> Seen {
        let read: BTreeSet<&str> = inbox.read.iter().map(String::as_str).collect();
        let pulls_read: BTreeSet<&str> = inbox.pulls_read.iter().map(String::as_str).collect();

        let mut tickets: BTreeMap<_, _> = self
            .tickets
            .iter()
            .filter(|(_, t)| !read.contains(t.tracker.as_str()))
            .map(|(k, t)| (k.clone(), t.clone()))
            .collect();
        for t in &inbox.tasks {
            tickets.insert(
                ticket_key(&t.tracker, &t.id),
                SeenTicket {
                    tracker: t.tracker.clone(),
                    status: t.status.clone(),
                    comments: t.comments,
                    relation: t.relation,
                    filters: t.filters.clone(),
                },
            );
        }

        let mut pulls: BTreeMap<_, _> = self
            .pulls
            .iter()
            .filter(|(_, p)| !pulls_read.contains(p.tracker.as_str()))
            .map(|(k, p)| (k.clone(), p.clone()))
            .collect();
        for p in &inbox.pulls {
            pulls.insert(
                pull_key(p),
                SeenPull {
                    tracker: p.tracker.clone(),
                    checks: p.checks,
                    review: p.review,
                    relation: p.relation,
                },
            );
        }

        // A tracker that answered says which of its filters did; one that did
        // not keeps what it had.
        let mut filters_read: BTreeSet<String> = self
            .filters_read
            .iter()
            .filter(|key| !read.contains(key.split('\0').next().unwrap_or_default()))
            .cloned()
            .collect();
        filters_read.extend(
            inbox
                .filters_read
                .iter()
                .map(|f| filter_key(&f.tracker, &f.filter)),
        );

        Seen {
            tickets,
            pulls,
            read: self
                .read
                .iter()
                .cloned()
                .chain(inbox.read.clone())
                .collect(),
            pulls_read: self
                .pulls_read
                .iter()
                .cloned()
                .chain(inbox.pulls_read.clone())
                .collect(),
            filters_read,
        }
    }

    /// The changes between this and a read, one entry per ticket or pull
    /// request.
    fn changes(&self, inbox: &Inbox) -> Vec<Change> {
        let mut out = Vec::new();
        // A tracker answering for the first time is a baseline.
        for t in inbox
            .tasks
            .iter()
            .filter(|t| self.read.contains(&t.tracker))
        {
            let what = self.ticket_changes(self.tickets.get(&ticket_key(&t.tracker, &t.id)), t);
            if !what.is_empty() {
                out.push(Change {
                    tracker: t.tracker.clone(),
                    id: t.id.clone(),
                    key: t.key.clone(),
                    title: t.title.clone(),
                    what: what.join(", "),
                });
            }
        }
        for p in inbox
            .pulls
            .iter()
            .filter(|p| self.pulls_read.contains(&p.tracker))
        {
            let what = pull_changes(self.pulls.get(&pull_key(p)), p);
            if !what.is_empty() {
                out.push(Change {
                    tracker: p.tracker.clone(),
                    id: p.id.clone(),
                    key: pull_name(p),
                    title: p.title.clone(),
                    what: what.join(", "),
                });
            }
        }
        out
    }

    fn ticket_changes(&self, was: Option<&SeenTicket>, t: &Task) -> Vec<String> {
        let mut what = Vec::new();
        // Yours now, and not before. One you created turning up is not news:
        // you know you made it.
        if t.relation == Relation::Assigned && was.map(|w| w.relation) != Some(Relation::Assigned) {
            what.push("assigned to you".to_string());
        }
        // In a filter now, and not before, once that filter has answered
        // before. Said whatever else changed, because what a filter is waiting
        // for is usually exactly that ticket arriving.
        for f in &t.filters {
            let known = self.filters_read.contains(&filter_key(&t.tracker, f));
            if known && !was.is_some_and(|w| w.filters.contains(f)) {
                what.push(format!("new in {f}"));
            }
        }
        let Some(was) = was else { return what };
        if was.status != t.status {
            what.push(format!("{} → {}", was.status, t.status));
        }
        // Only when both counts are known, and not for your own comment: you
        // know what you wrote.
        if let (Some(before), Some(now)) = (was.comments, t.comments)
            && now > before
            && !t.last_comment_mine
        {
            what.push(match &t.last_commenter {
                Some(who) => format!("{who} commented"),
                None => "new comment".to_string(),
            });
        }
        what
    }
}

fn pull_changes(was: Option<&SeenPull>, p: &PullRequest) -> Vec<String> {
    if p.relation == Relation::Assigned {
        // Waiting on you now, and not before. Once you have reviewed it, it
        // leaves the list, so the next request for a review is news again.
        if was.is_some_and(|w| w.relation == Relation::Assigned) {
            return Vec::new();
        }
        let what = if p.review_requested {
            "review requested"
        } else {
            "assigned to you"
        };
        return vec![what.to_string()];
    }
    // Yours: only what came back from somebody else, or from the checks.
    let Some(was) = was else {
        return Vec::new();
    };
    let mut what = Vec::new();
    if p.checks == Some(Checks::Failing) && was.checks != Some(Checks::Failing) {
        what.push("checks failed".to_string());
    }
    if p.review != was.review {
        match p.review {
            Some(Review::Approved) => what.push("approved".to_string()),
            Some(Review::ChangesRequested) => what.push("changes requested".to_string()),
            _ => {}
        }
    }
    what
}

#[cfg(test)]
mod tests {
    use super::super::{FilterRead, Kind};
    use super::*;

    fn ticket(id: &str) -> Task {
        Task {
            tracker: "work".into(),
            kind: Kind::Jira,
            id: id.into(),
            key: id.into(),
            title: format!("Title {id}"),
            url: String::new(),
            status: "To Do".into(),
            item_type: "Bug".into(),
            session_name: String::new(),
            branch: String::new(),
            repo: None,
            relation: Relation::Assigned,
            filters: Vec::new(),
            updated: None,
            comments: Some(0),
            last_commenter: None,
            last_comment_mine: false,
        }
    }

    fn pull(id: &str) -> PullRequest {
        PullRequest {
            tracker: "gh".into(),
            kind: Kind::GitHub,
            id: id.into(),
            key: format!("#{id}"),
            title: format!("Pull {id}"),
            url: String::new(),
            repo: "octo/Spoon-Knife".into(),
            repo_url: String::new(),
            relation: Relation::Created,
            review_requested: false,
            author: "me".into(),
            branch: "b".into(),
            base: "main".into(),
            draft: false,
            fork: false,
            conflicts: false,
            checks: Some(Checks::Pending),
            review: None,
            comments: None,
            updated: None,
        }
    }

    fn inbox(tasks: Vec<Task>) -> Inbox {
        Inbox {
            tasks,
            read: vec!["work".into()],
            ..Inbox::default()
        }
    }

    fn with_filters(mut inbox: Inbox, filters: &[&str]) -> Inbox {
        inbox.filters_read = filters
            .iter()
            .map(|f| FilterRead {
                tracker: "work".into(),
                filter: f.to_string(),
            })
            .collect();
        inbox
    }

    fn pulls(pulls: Vec<PullRequest>) -> Inbox {
        Inbox {
            pulls,
            pulls_read: vec!["gh".into()],
            ..Inbox::default()
        }
    }

    /// Two reads: what the second one announces, as a notification says it.
    fn changes(first: &Inbox, second: &Inbox) -> Vec<String> {
        let (seen, none) = record(None, first);
        assert!(none.is_empty(), "the first read is a baseline");
        said(&record(Some(&seen), second).1)
    }

    fn said(changes: &[Change]) -> Vec<String> {
        changes
            .iter()
            .map(|c| format!("{} · {}", c.key, c.what))
            .collect()
    }

    #[test]
    fn a_move_and_somebody_elses_comment_are_said_and_your_own_is_not() {
        let mut moved = ticket("P-1");
        moved.status = "Done".into();
        moved.comments = Some(1);
        moved.last_commenter = Some("Alex".into());
        let mut mine = ticket("P-2");
        mine.comments = Some(1);
        mine.last_commenter = Some("Me".into());
        mine.last_comment_mine = true;
        let got = changes(
            &inbox(vec![ticket("P-1"), ticket("P-2")]),
            &inbox(vec![moved, mine]),
        );
        assert_eq!(got, ["P-1 · To Do → Done, Alex commented"]);
    }

    #[test]
    fn a_comment_count_nobody_gave_says_nothing() {
        let mut before = ticket("P-1");
        before.comments = None;
        let mut after = ticket("P-1");
        after.comments = Some(4);
        assert!(changes(&inbox(vec![before]), &inbox(vec![after])).is_empty());
    }

    #[test]
    fn becoming_yours_is_news_and_creating_it_is_not() {
        let mut created = ticket("P-1");
        created.relation = Relation::Created;
        let mut also_created = ticket("P-2");
        also_created.relation = Relation::Created;
        let got = changes(
            &inbox(vec![created]),
            &inbox(vec![ticket("P-1"), also_created]),
        );
        assert_eq!(got, ["P-1 · assigned to you"]);
    }

    /// Arriving in a filter is news, once that filter has answered before; a
    /// filter's first answer, and its first after failing, is a baseline.
    #[test]
    fn a_ticket_new_to_a_filter_is_said_once_the_filter_is_known() {
        let matched = |id: &str| Task {
            relation: Relation::Matched,
            filters: vec!["ready".into()],
            ..ticket(id)
        };
        let one = with_filters(inbox(vec![matched("P-1")]), &["ready"]);
        let two = with_filters(inbox(vec![matched("P-1"), matched("P-2")]), &["ready"]);
        assert_eq!(changes(&one, &two), ["P-2 · new in ready"]);

        // Added since: its first answer says nothing.
        assert!(changes(&inbox(vec![]), &two).is_empty());

        // Failed in between: its return says nothing either.
        let (a, _) = record(None, &one);
        let (b, _) = record(Some(&a), &inbox(vec![]));
        assert!(record(Some(&b), &two).1.is_empty());
    }

    /// A tracker that could not be read keeps what it had, so its return is
    /// not a list of arrivals.
    #[test]
    fn a_tracker_that_was_not_read_keeps_its_tickets() {
        let one = inbox(vec![ticket("P-1"), ticket("P-2")]);
        let (a, _) = record(None, &one);
        let down = Inbox::default();
        let (b, none) = record(Some(&a), &down);
        assert!(none.is_empty());
        assert!(record(Some(&b), &one).1.is_empty());
    }

    #[test]
    fn a_tracker_read_for_the_first_time_is_a_baseline() {
        let (seen, _) = record(None, &Inbox::default());
        assert!(
            record(Some(&seen), &inbox(vec![ticket("P-1")]))
                .1
                .is_empty()
        );
    }

    #[test]
    fn a_pull_waiting_on_you_is_said_once() {
        let waiting = PullRequest {
            relation: Relation::Assigned,
            review_requested: true,
            ..pull("61")
        };
        let got = changes(&pulls(vec![]), &pulls(vec![waiting.clone()]));
        assert_eq!(got, ["Spoon-Knife #61 · review requested"]);

        let (a, _) = record(None, &pulls(vec![waiting.clone()]));
        assert!(record(Some(&a), &pulls(vec![waiting])).1.is_empty());
    }

    #[test]
    fn on_your_pull_failing_checks_and_reviews_are_said() {
        let back = PullRequest {
            checks: Some(Checks::Failing),
            review: Some(Review::ChangesRequested),
            ..pull("7")
        };
        let got = changes(&pulls(vec![pull("7")]), &pulls(vec![back]));
        assert_eq!(got, ["Spoon-Knife #7 · checks failed, changes requested"]);

        // A new one of your own is not news: you opened it.
        assert!(changes(&pulls(vec![]), &pulls(vec![pull("8")])).is_empty());
    }

    #[test]
    fn a_change_says_where_it_came_from() {
        let mut moved = ticket("10042");
        moved.key = "P-1".into();
        moved.status = "Done".into();
        let mut before = moved.clone();
        before.status = "To Do".into();
        let (seen, _) = record(None, &inbox(vec![before]));
        let (_, got) = record(Some(&seen), &inbox(vec![moved]));
        assert_eq!(
            (got[0].tracker.as_str(), got[0].id.as_str()),
            ("work", "10042")
        );
    }

    #[test]
    fn what_was_seen_survives_being_stored() {
        let t = Task {
            filters: vec!["ready".into()],
            ..ticket("P-1")
        };
        let (a, _) = record(None, &with_filters(inbox(vec![t]), &["ready"]));
        let back: Seen = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }

    /// What the phone kept before this was moved here: `InboxSnapshot` from
    /// HuraKit, as Swift's encoder writes it.
    #[test]
    fn reads_what_the_phone_kept() {
        let swift = r#"{"tickets":{"work\u0000P-1":{"tracker":"work","status":"To Do","comments":0,"relation":"assigned","filters":[]}},
            "pulls":{"gh\u0000octo/Spoon-Knife\u00007":{"tracker":"gh","checks":"pending","relation":"created"}},
            "read":["work"],"pullsRead":["gh"],"filtersRead":[]}"#;
        let seen: Seen = serde_json::from_str(swift).unwrap();
        let mut moved = ticket("P-1");
        moved.status = "Done".into();
        let failing = PullRequest {
            checks: Some(Checks::Failing),
            ..pull("7")
        };
        let read = Inbox {
            pulls: vec![failing],
            pulls_read: vec!["gh".into()],
            ..inbox(vec![moved])
        };
        assert_eq!(
            said(&record(Some(&seen), &read).1),
            ["P-1 · To Do → Done", "Spoon-Knife #7 · checks failed"]
        );
    }

    /// What the desktop kept: `inboxNotify.ts`'s snapshot, with a key and a
    /// title on each entry that nothing reads, and from before there were
    /// filters, none of those.
    #[test]
    fn reads_what_the_desktop_kept() {
        let ts = r#"{"tickets":{"work\u0000P-1":{"tracker":"work","key":"P-1","title":"T","status":"To Do","comments":null,"relation":"created"}},
            "pulls":{},"read":["work"],"pullsRead":[]}"#;
        let seen: Seen = serde_json::from_str(ts).unwrap();
        assert_eq!(
            said(&record(Some(&seen), &inbox(vec![ticket("P-1")])).1),
            ["P-1 · assigned to you"]
        );
    }
}
