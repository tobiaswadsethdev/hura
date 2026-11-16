import Foundation

/// What is waiting on a server: sessions to start, at a time or once its
/// queue has room, and messages to say to a session's agent. `hura_core::queue`
/// on the server, which keeps it on disk and acts on it whether or not
/// anything is connected.
public struct QueueView: Decodable, Equatable, Sendable {
    /// Oldest first, which is the order a queued one starts in.
    public var sessions: [QueuedSession]
    /// Oldest first; a session's are said in this order.
    public var messages: [QueuedMessage]
    /// How many sessions the queue lets run at once.
    public var limit: Int

    public init(sessions: [QueuedSession] = [], messages: [QueuedMessage] = [], limit: Int = 2) {
        self.sessions = sessions
        self.messages = messages
        self.limit = limit
    }

    /// Where a session waiting for room stands: 1 is next. nil for one
    /// waiting for a time, or not waiting at all.
    public func place(of waiting: QueuedSession) -> Int? {
        guard waiting.at == nil else { return nil }
        let line = sessions.filter { $0.at == nil && !$0.started && $0.failed == nil }
        return line.firstIndex { $0.id == waiting.id }.map { $0 + 1 }
    }

    /// What is waiting to be said to one session.
    public func messages(for session: String) -> [QueuedMessage] {
        messages.filter { $0.session == session }
    }
}

/// A session waiting to start.
public struct QueuedSession: Decodable, Equatable, Identifiable, Sendable {
    public var id: Int64
    /// The parts of the request the phone shows.
    public var new: Asked
    /// When it starts; nil once there is room.
    public var at: Date?
    /// Being started now.
    public var started: Bool
    /// Why it could not start when its turn came; it waits until started by
    /// hand or removed.
    public var failed: String?

    public struct Asked: Decodable, Equatable, Sendable {
        public var project: String?
        public var name: String?
        public var title: String?
        public var repo: String
        public var task: String
    }

    public var label: String { new.title ?? new.name ?? "" }

    enum CodingKeys: String, CodingKey { case id, new, at, started, failed }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        new = try c.decode(Asked.self, forKey: .new)
        at = try c.decodeIfPresent(Int64.self, forKey: .at).map { Date(timeIntervalSince1970: TimeInterval($0)) }
        started = try c.decodeIfPresent(Bool.self, forKey: .started) ?? false
        failed = try c.decodeIfPresent(String.self, forKey: .failed)
    }
}

/// Something to say to a session's agent once it is free, and not before
/// `at` when there is one.
public struct QueuedMessage: Decodable, Equatable, Identifiable, Sendable {
    public var id: Int64
    public var session: String
    public var text: String
    public var at: Date?

    enum CodingKeys: String, CodingKey { case id, session, text, at }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        session = try c.decode(String.self, forKey: .session)
        text = try c.decode(String.self, forKey: .text)
        at = try c.decodeIfPresent(Int64.self, forKey: .at).map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }
}

extension ServerClient {
    /// What is waiting on this server.
    public func queue() async throws -> QueueView {
        try await call("queue", Nothing(), as: QueueView.self)
    }

    /// Starts a session later: at `at`, or once the queue has room when nil.
    public func enqueue(_ session: NewSession, at: Date?) async throws -> QueueView {
        struct Body: Encodable {
            var new: NewSession
            var at: Int64?
        }
        return try await call("enqueue", Body(new: session, at: at.map { Int64($0.timeIntervalSince1970) }),
                              as: QueueView.self)
    }

    /// Takes a waiting session or message off the queue.
    public func unqueue(_ id: Int64) async throws -> QueueView {
        struct Body: Encodable { var id: Int64 }
        return try await call("unqueue", Body(id: id), as: QueueView.self)
    }

    /// Starts a waiting session now, whatever the limit says.
    public func startQueued(_ id: Int64) async throws -> QueueView {
        struct Body: Encodable { var id: Int64 }
        return try await call("start-queued", Body(id: id), as: QueueView.self)
    }

    /// Says something to a session's agent once its turn has ended, and not
    /// before `at` when there is one.
    public func queueMessage(_ name: String, text: String, at: Date?) async throws -> QueueView {
        struct Body: Encodable {
            var name: String
            var text: String
            var at: Int64?
        }
        return try await call("queue-message",
                              Body(name: name, text: text, at: at.map { Int64($0.timeIntervalSince1970) }),
                              as: QueueView.self)
    }
}
