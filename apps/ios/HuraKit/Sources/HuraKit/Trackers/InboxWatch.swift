import Foundation

// What changed on your trackers since the last read. Decided by hura-core's
// `tracker::changes`, the code the desktop runs too, so the phone notifies
// for the same things in the same words: a status that moved, somebody else's
// comment, a ticket that became yours or turned up in a filter, and on pull
// requests a review asked of you, failing checks, an approval or changes
// asked for.
//
// **What was seen is the app's to keep**, between reads and across launches,
// so a ticket that moved while the app was closed is announced the next time
// it reads. It is hura-core's own shape, kept as it came and handed back.

/// One change worth saying, about one ticket or pull request.
public struct InboxChange: Codable, Equatable, Sendable {
    /// `PROJ-7`, or `Spoon-Knife #61`.
    public var key: String
    public var title: String
    /// `To Do → In Progress, Alex commented`.
    public var what: String
    public var tracker: String
    public var id: String
}

extension TrackerCore {
    /// Records a read against what was seen at the last one: what is seen
    /// now, to keep and hand back next time, and what changed. With nothing
    /// seen, or something hura-core did not write, the read is the baseline
    /// and nothing has changed.
    public static func changes(_ inbox: Inbox, seen: JSONValue?) async throws -> (seen: JSONValue, changes: [InboxChange]) {
        let answer = try await call("changes", ["inbox": json(inbox), "seen": seen ?? .null])
        return (answer["seen"] ?? .null, try decode(answer["changes"] ?? .array([]), as: [InboxChange].self))
    }
}
