import Foundation

// What changed on your trackers since the last read: the desktop's
// `inboxNotify.ts`, so the phone notifies for the same things in the same
// words.
//
// Tickets: its status moved, somebody else commented on it, it became yours,
// or it turned up in one of your JQL filters. Pull requests: a review was
// asked of you or one was assigned to you, and on yours, the checks failed,
// it was approved, or changes were asked for.
//
// **Compared with a snapshot kept between reads**, so a ticket that moved
// while the app was closed is announced the next time it reads. The first
// read is a baseline, and so is the first read of a tracker, or of a filter,
// added since. A tracker that could not be read keeps what it had; a filter
// that could not be read is forgotten, and its next answer is a baseline.

/// One change worth saying, about one ticket or pull request.
public struct InboxChange: Equatable, Sendable {
    /// `PROJ-7`, or `Spoon-Knife #61`.
    public var key: String
    public var title: String
    /// `To Do → In Progress, Alex commented`.
    public var what: String
    public var tracker: String
    public var id: String
}

/// What was seen at the last read.
public struct InboxSnapshot: Codable, Equatable, Sendable {
    struct Ticket: Codable, Equatable, Sendable {
        var tracker: String
        var status: String
        var comments: Int?
        var relation: String
        var filters: [String]
    }

    struct Pull: Codable, Equatable, Sendable {
        var tracker: String
        var checks: String?
        var review: String?
        var relation: String
    }

    /// By ticket, keyed `tracker \0 id`.
    var tickets: [String: Ticket] = [:]
    /// By pull request, keyed `tracker \0 repo \0 id`.
    var pulls: [String: Pull] = [:]
    /// The trackers read at least once, each half on its own.
    var read: Set<String> = []
    var pullsRead: Set<String> = []
    /// The filters read at least once, keyed `tracker \0 filter`.
    var filtersRead: Set<String> = []

    /// Record a read: what the snapshot becomes, and what changed since the
    /// last one. With no snapshot, the read is the baseline and nothing has
    /// changed.
    public static func record(_ inbox: Inbox, after before: InboxSnapshot?) -> (InboxSnapshot, [InboxChange]) {
        let next = (before ?? InboxSnapshot()).taking(inbox)
        guard let before else { return (next, []) }
        return (next, before.changes(to: inbox))
    }

    private static func ticketKey(_ tracker: String, _ id: String) -> String { "\(tracker)\0\(id)" }
    private static func pullKey(_ p: PullRequestSummary) -> String { "\(p.tracker)\0\(p.repo)\0\(p.id)" }
    private static func filterKey(_ tracker: String, _ filter: String) -> String { "\(tracker)\0\(filter)" }

    /// A tracker that was read is replaced wholesale, so what left it is
    /// forgotten; one that was not keeps what it had.
    private func taking(_ inbox: Inbox) -> InboxSnapshot {
        let read = Set(inbox.read)
        let pullsRead = Set(inbox.pullsRead)
        var next = InboxSnapshot()
        next.tickets = tickets.filter { !read.contains($0.value.tracker) }
        for t in inbox.tasks {
            next.tickets[Self.ticketKey(t.tracker, t.id)] =
                Ticket(tracker: t.tracker, status: t.status, comments: t.comments, relation: t.relation, filters: t.filters)
        }
        next.pulls = pulls.filter { !pullsRead.contains($0.value.tracker) }
        for p in inbox.pulls {
            next.pulls[Self.pullKey(p)] = Pull(tracker: p.tracker, checks: p.checks, review: p.review, relation: p.relation)
        }
        next.read = self.read.union(read)
        next.pullsRead = self.pullsRead.union(pullsRead)
        next.filtersRead = filtersRead.filter { key in
            !read.contains(String(key.prefix(while: { $0 != "\0" })))
        }.union(inbox.filtersRead.map { Self.filterKey($0.tracker, $0.filter) })
        return next
    }

    private func changes(to inbox: Inbox) -> [InboxChange] {
        var out: [InboxChange] = []
        for t in inbox.tasks where read.contains(t.tracker) {
            let what = ticketChanges(tickets[Self.ticketKey(t.tracker, t.id)], t)
            if !what.isEmpty {
                out.append(InboxChange(key: t.key, title: t.title, what: what.joined(separator: ", "), tracker: t.tracker, id: t.id))
            }
        }
        for p in inbox.pulls where pullsRead.contains(p.tracker) {
            let what = pullChanges(pulls[Self.pullKey(p)], p)
            if !what.isEmpty {
                out.append(InboxChange(key: pullName(p), title: p.title, what: what.joined(separator: ", "), tracker: p.tracker, id: p.id))
            }
        }
        return out
    }

    private func ticketChanges(_ was: Ticket?, _ t: TrackerTask) -> [String] {
        var what: [String] = []
        // Yours now, and not before. One you created turning up is not news.
        if t.relation == "assigned", was?.relation != "assigned" { what.append("assigned to you") }
        for f in t.filters where filtersRead.contains(Self.filterKey(t.tracker, f)) && !(was?.filters ?? []).contains(f) {
            what.append("new in \(f)")
        }
        guard let was else { return what }
        if was.status != t.status { what.append("\(was.status) → \(t.status)") }
        // Only when both counts are known, and not for your own comment.
        if let before = was.comments, let now = t.comments, now > before, !t.lastCommentMine {
            what.append(t.lastCommenter.map { "\($0) commented" } ?? "new comment")
        }
        return what
    }

    private func pullChanges(_ was: Pull?, _ p: PullRequestSummary) -> [String] {
        if p.relation == "assigned" {
            // Waiting on you now, and not before.
            if was?.relation == "assigned" { return [] }
            return [p.reviewRequested ? "review requested" : "assigned to you"]
        }
        guard let was else { return [] }
        var what: [String] = []
        if p.checks == "failing", was.checks != "failing" { what.append("checks failed") }
        if p.review != was.review {
            if p.review == "approved" { what.append("approved") }
            if p.review == "changes-requested" { what.append("changes requested") }
        }
        return what
    }
}
