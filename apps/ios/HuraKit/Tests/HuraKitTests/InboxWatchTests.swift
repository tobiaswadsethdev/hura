import Foundation
import Testing
@testable import HuraKit

/// The same changes the desktop's inboxNotify.ts announces, in the same words.
struct InboxWatchTests {
    func ticket(_ id: String, status: String = "To Do", relation: String = "assigned", filters: [String] = [],
                comments: Int? = 0, lastCommenter: String? = nil, mine: Bool = false) -> TrackerTask {
        TrackerTask(tracker: "work", kind: "jira", id: id, key: id, title: "Title \(id)", url: "", status: status,
                    itemType: "Bug", sessionName: "", branch: "", repo: nil, relation: relation, filters: filters,
                    updated: nil, comments: comments, lastCommenter: lastCommenter, lastCommentMine: mine)
    }

    func inbox(_ tasks: [TrackerTask], read: [String] = ["work"], filters: [String] = []) -> Inbox {
        Inbox(tasks: tasks, read: read, filtersRead: filters.map { FilterRead(tracker: "work", filter: $0) })
    }

    /// Two reads: what the second one announces.
    func changes(_ first: Inbox, _ second: Inbox) -> [String] {
        let (snapshot, none) = InboxSnapshot.record(first, after: nil)
        #expect(none.isEmpty, "the first read is a baseline")
        return InboxSnapshot.record(second, after: snapshot).1.map { "\($0.key) · \($0.what)" }
    }

    @Test func aMoveAndSomebodyElsesCommentAreSaidAndYourOwnIsNot() {
        let got = changes(
            inbox([ticket("P-1"), ticket("P-2")]),
            inbox([ticket("P-1", status: "Done", comments: 1, lastCommenter: "Alex"),
                   ticket("P-2", comments: 1, lastCommenter: "Me", mine: true)]))
        #expect(got == ["P-1 · To Do → Done, Alex commented"])
    }

    @Test func becomingYoursIsNewsAndCreatingItIsNot() {
        let got = changes(inbox([ticket("P-1", relation: "created")]),
                          inbox([ticket("P-1"), ticket("P-2", relation: "created")]))
        #expect(got == ["P-1 · assigned to you"])
    }

    /// Arriving in a filter is news, once that filter has answered before;
    /// a filter's first answer, and its first after failing, is a baseline.
    @Test func aTicketNewToAFilterIsSaidOnceTheFilterIsKnown() {
        let one = inbox([ticket("P-1", relation: "matched", filters: ["ready"])], filters: ["ready"])
        let two = inbox([ticket("P-1", relation: "matched", filters: ["ready"]),
                         ticket("P-2", relation: "matched", filters: ["ready"])], filters: ["ready"])
        #expect(changes(one, two) == ["P-2 · new in ready"])

        // Added since: its first answer says nothing.
        #expect(changes(inbox([]), two).isEmpty)

        // Failed in between: its return says nothing either.
        let (a, _) = InboxSnapshot.record(one, after: nil)
        let (b, _) = InboxSnapshot.record(inbox([]), after: a)
        #expect(InboxSnapshot.record(two, after: b).1.isEmpty)
    }

    /// A tracker that could not be read keeps what it had, so its return is
    /// not a list of arrivals.
    @Test func aTrackerThatWasNotReadKeepsItsTickets() {
        let one = inbox([ticket("P-1"), ticket("P-2")])
        let (a, _) = InboxSnapshot.record(one, after: nil)
        let (b, down) = InboxSnapshot.record(inbox([], read: []), after: a)
        #expect(down.isEmpty)
        #expect(InboxSnapshot.record(one, after: b).1.isEmpty)
    }

    @Test func aSnapshotSurvivesBeingStored() throws {
        let (a, _) = InboxSnapshot.record(inbox([ticket("P-1", filters: ["ready"])], filters: ["ready"]), after: nil)
        let back = try JSONDecoder().decode(InboxSnapshot.self, from: JSONEncoder().encode(a))
        #expect(back == a)
    }

    @Test func aTrackerKeptBeforeFiltersStillReads() throws {
        let old = #"{"kind":"jira","name":"work","site":"https://x.atlassian.net","email":"a@b.c"}"#
        let s = try JSONDecoder().decode(TrackerSource.self, from: Data(old.utf8))
        #expect(s.jqlFilters.isEmpty && !s.hideCreated)
    }
}
