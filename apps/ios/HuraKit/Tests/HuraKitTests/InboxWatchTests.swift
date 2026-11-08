import Foundation
import Testing
@testable import HuraKit

/// What changed between reads, worked out by hura-core (where the rules are
/// tested): here, that a read and what was seen get there and back.
struct InboxWatchTests {
    func ticket(_ id: String, status: String = "To Do", comments: Int? = 0, lastCommenter: String? = nil) -> TrackerTask {
        TrackerTask(tracker: "work", kind: "jira", id: id, key: id, title: "Title \(id)", url: "", status: status,
                    itemType: "Bug", sessionName: "", branch: "", repo: nil, relation: "assigned", filters: [],
                    updated: nil, comments: comments, lastCommenter: lastCommenter, lastCommentMine: false)
    }

    func inbox(_ tasks: [TrackerTask]) -> Inbox { Inbox(tasks: tasks, read: ["work"]) }

    @Test func aChangeIsSaidAfterTheBaseline() async throws {
        let (seen, none) = try await TrackerCore.changes(inbox([ticket("P-1")]), seen: nil)
        #expect(none.isEmpty, "the first read is a baseline")
        // Kept the way TrackersModel keeps it.
        let kept = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(seen))
        let (_, got) = try await TrackerCore.changes(
            inbox([ticket("P-1", status: "Done", comments: 1, lastCommenter: "Alex")]), seen: kept)
        #expect(got.map { "\($0.key) · \($0.what)" } == ["P-1 · To Do → Done, Alex commented"])
        #expect(got.first?.tracker == "work" && got.first?.id == "P-1")
    }

    @Test func aTrackerKeptBeforeFiltersStillReads() throws {
        let old = #"{"kind":"jira","name":"work","site":"https://x.atlassian.net","email":"a@b.c"}"#
        let s = try JSONDecoder().decode(TrackerSource.self, from: Data(old.utf8))
        #expect(s.jqlFilters.isEmpty && !s.hideCreated)
    }
}
