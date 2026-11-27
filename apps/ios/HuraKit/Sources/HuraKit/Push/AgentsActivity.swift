// One Live Activity for every session, T3 Code's (github.com/pingdotgg/t3code,
// MIT; `apps/mobile/src/widgets/AgentActivity.tsx`): how many agents are
// working and how many need you, and a row for each, the ones waiting on you
// first. The relay keeps the rows and sends them all each time one changes
// (`docs/relay-api.md`, "One activity for every session"); the widget draws
// them, and the app asks the relay for it.

import Foundation
#if canImport(ActivityKit) && os(iOS)
import ActivityKit
#endif

/// One session in the activity: its event, as a session's own activity
/// carries it, and the server it came from, whose key opens its text.
public struct AgentRow: Codable, Hashable, Sendable, Identifiable {
    public var serverID: String
    public var session: String
    public var state: SessionActivityState

    public var id: String { "\(serverID):\(session)" }
    public var phase: SessionPhase { state.phase }

    public init(serverID: String, session: String, state: SessionActivityState) {
        self.serverID = serverID
        self.session = session
        self.state = state
    }

    enum CodingKeys: String, CodingKey {
        case serverID = "server_id"
        case session
    }

    // The row's own fields beside the state's, in one object, as the relay
    // writes it.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        serverID = try c.decode(String.self, forKey: .serverID)
        session = try c.decode(String.self, forKey: .session)
        state = try SessionActivityState(from: decoder)
    }

    public func encode(to encoder: Encoder) throws {
        try state.encode(to: encoder)
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(serverID, forKey: .serverID)
        try c.encode(session, forKey: .session)
    }

    /// The row's text, opened with its server's key.
    public func text(keys: PushKeyStore) -> PushCrypto.Text? {
        state.text(serverID: serverID, session: session, keys: keys)
    }
}

/// What the activity shows: every row that fits, and the counts of them all,
/// which may be more than the rows.
public struct AgentsActivityState: Codable, Hashable, Sendable {
    /// Working or waiting.
    public var active: Int
    /// Waiting on a permission or input.
    public var waiting: Int
    public var failed: Int
    public var rows: [AgentRow]

    public init(active: Int, waiting: Int, failed: Int, rows: [AgentRow]) {
        self.active = active
        self.waiting = waiting
        self.failed = failed
        self.rows = rows
    }

    /// The counts worked out from `rows`, for a state the app makes itself.
    public init(rows: [AgentRow]) {
        self.init(active: rows.filter { !$0.phase.isOver }.count,
                  waiting: rows.filter { $0.phase.isWaiting }.count,
                  failed: rows.filter { $0.phase == .failed }.count,
                  rows: AgentsActivityState.ordered(rows))
    }

    /// T3 Code's order: waiting on you, then failed, then working, then done;
    /// the newest first within each. The relay sends them so; a state the app
    /// changes itself is put back in it.
    public static func ordered(_ rows: [AgentRow]) -> [AgentRow] {
        func rank(_ p: SessionPhase) -> Int {
            switch p {
            case .permission, .input: 0
            case .failed: 1
            case .working: 2
            case .done, .ended: 3
            }
        }
        return rows.enumerated().sorted { a, b in
            let (ra, rb) = (rank(a.element.phase), rank(b.element.phase))
            if ra != rb { return ra < rb }
            let (ta, tb) = (a.element.state.at ?? 0, b.element.state.at ?? 0)
            return ta != tb ? ta > tb : a.offset < b.offset
        }.map(\.element)
    }

    /// The row the activity leads with, and opens when tapped: the first
    /// waiting on you, or failing that the first failed, or the first.
    public var hero: AgentRow? {
        rows.first { $0.phase.isWaiting } ?? rows.first { $0.phase == .failed } ?? rows.first
    }

    /// Nothing is working or waiting any more.
    public var allDone: Bool { active == 0 }

    /// The heading: "3 active agents", or how it all ended.
    public var headline: String {
        if allDone { return failed > 0 ? "Agent work failed" : "Agent work completed" }
        return "\(active) active \(active == 1 ? "agent" : "agents")"
    }

    /// Beside the heading while any are waiting: "1 needs attention".
    public var attention: String? {
        waiting == 0 ? nil : "\(waiting) need\(waiting == 1 ? "s" : "") attention"
    }

    /// The fewest words for it, where there is room for little: "2 active",
    /// or "Done", or "Failed".
    public var short: String {
        if allDone { return failed > 0 ? "Failed" : "Done" }
        return "\(active) active"
    }

    /// The compact island's trailing word, when there is one: what the first
    /// waiting asks for, or that something failed. While agents only work it
    /// says nothing, and a glyph marks it, as T3 Code's does.
    public var compactWord: String? {
        if let waiting = rows.first(where: { $0.phase.isWaiting }) {
            return waiting.phase == .permission ? "Approval" : "Input"
        }
        if failed > 0 { return "Failed" }
        if allDone { return "Done" }
        return nil
    }

    /// The same state with one row answered: working again, with what was
    /// said, until the server says what came next. Only this phone sees it,
    /// so the text goes in the clear.
    public func answered(serverID: String, session: String, askID: String, allow: Bool,
                         keys: PushKeyStore, now: Date = Date()) -> AgentsActivityState? {
        guard let i = rows.firstIndex(where: {
            $0.serverID == serverID && $0.session == session && $0.state.askID == askID
        }) else { return nil }
        var rows = rows
        let text = rows[i].text(keys: keys)
        rows[i].state = rows[i].state.answered(text, body: allow ? "Allowed" : "Declined", now: now)
        // The counts the relay sent cover rows that may not be here; only
        // the one that moved is moved.
        var next = self
        next.rows = AgentsActivityState.ordered(rows)
        next.waiting = max(0, waiting - 1)
        return next
    }
}

extension SessionPhase {
    /// A row's status, in T3 Code's words.
    public var status: String {
        switch self {
        case .working: "Working"
        case .permission: "Needs approval"
        case .input: "Needs input"
        case .done: "Done"
        case .failed: "Failed"
        case .ended: "Ended"
        }
    }
}

#if canImport(ActivityKit) && os(iOS)
/// The one activity. Its name matches the relay's `attributes-type`, and its
/// field the `attributes` it sends, so a push can start it.
public struct AgentsActivityAttributes: ActivityAttributes {
    public typealias ContentState = AgentsActivityState

    /// Always `agents`: an activity has to carry something.
    public var kind: String

    public init(kind: String = "agents") {
        self.kind = kind
    }
}
#endif
