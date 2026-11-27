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
    /// The server's offset from UTC, in seconds: a repeating entry's time of
    /// day is on its clock. nil from a server too old to repeat anything,
    /// which is how the phone knows not to offer it.
    public var utcOffset: Int?

    public init(sessions: [QueuedSession] = [], messages: [QueuedMessage] = [], limit: Int = 2,
                utcOffset: Int? = nil) {
        self.sessions = sessions
        self.messages = messages
        self.limit = limit
        self.utcOffset = utcOffset
    }

    enum CodingKeys: String, CodingKey {
        case sessions, messages, limit
        case utcOffset = "utc_offset"
    }

    /// Whether the server can keep a worktree or a message that comes round
    /// again.
    public var canRepeat: Bool { utcOffset != nil }

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
    /// Set on a template that comes round again; its `at` is always the next
    /// time.
    public var `repeat`: Repeat?
    /// How many times a repeating one has started.
    public var runs: Int

    public struct Asked: Decodable, Equatable, Sendable {
        public var project: String?
        public var name: String?
        public var title: String?
        public var repo: String
        public var task: String
    }

    public var label: String { new.title ?? new.name ?? "" }

    enum CodingKeys: String, CodingKey { case id, new, at, started, failed, `repeat`, runs }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        new = try c.decode(Asked.self, forKey: .new)
        at = try c.decodeIfPresent(Int64.self, forKey: .at).map { Date(timeIntervalSince1970: TimeInterval($0)) }
        started = try c.decodeIfPresent(Bool.self, forKey: .started) ?? false
        failed = try c.decodeIfPresent(String.self, forKey: .failed)
        `repeat` = try c.decodeIfPresent(Repeat.self, forKey: .repeat)
        runs = try c.decodeIfPresent(Int.self, forKey: .runs) ?? 0
    }
}

/// Something to say to a session's agent once it is free, and not before
/// `at` when there is one.
public struct QueuedMessage: Decodable, Equatable, Identifiable, Sendable {
    public var id: Int64
    public var session: String
    public var text: String
    public var at: Date?
    /// Said again and again: each time, `at` moves on to the next.
    public var `repeat`: Repeat?

    enum CodingKeys: String, CodingKey { case id, session, text, at, `repeat` }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        session = try c.decode(String.self, forKey: .session)
        text = try c.decode(String.self, forKey: .text)
        at = try c.decodeIfPresent(Int64.self, forKey: .at).map { Date(timeIntervalSince1970: TimeInterval($0)) }
        `repeat` = try c.decodeIfPresent(Repeat.self, forKey: .repeat)
    }
}

extension ServerClient {
    /// What is waiting on this server.
    public func queue() async throws -> QueueView {
        try await call("queue", Nothing(), as: QueueView.self)
    }

    /// Starts a session later: at `at`, or once the queue has room when nil.
    /// With `repeat` it comes round again, a fresh worktree each time; offer
    /// that only where `QueueView.canRepeat`, since an older server starts it
    /// once.
    public func enqueue(_ session: NewSession, at: Date?, repeat rule: Repeat? = nil) async throws -> QueueView {
        struct Body: Encodable {
            var new: NewSession
            var at: Int64?
            var `repeat`: Repeat?
        }
        return try await call("enqueue", Body(new: session, at: at.map { Int64($0.timeIntervalSince1970) },
                                              repeat: rule),
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
    /// With `repeat`, said again each time it comes round.
    public func queueMessage(_ name: String, text: String, at: Date?, repeat rule: Repeat? = nil) async throws -> QueueView {
        struct Body: Encodable {
            var name: String
            var text: String
            var at: Int64?
            var `repeat`: Repeat?
        }
        return try await call("queue-message",
                              Body(name: name, text: text, at: at.map { Int64($0.timeIntervalSince1970) },
                                   repeat: rule),
                              as: QueueView.self)
    }
}

/// How a repeating worktree or message comes round (`hura_core::queue::Repeat`):
/// every so many minutes, or at a time of day on the server's clock, on some
/// days of the week (0 is Sunday) or every day when none are given.
public enum Repeat: Codable, Equatable, Hashable, Sendable {
    case interval(minutes: Int)
    case fixedTime(hour: Int, minute: Int, weekdays: [Int])

    private enum CodingKeys: String, CodingKey { case type, minutes, hour, minute, weekdays }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "interval":
            self = .interval(minutes: try c.decode(Int.self, forKey: .minutes))
        case "fixed_time":
            self = .fixedTime(hour: try c.decode(Int.self, forKey: .hour),
                              minute: try c.decode(Int.self, forKey: .minute),
                              weekdays: try c.decodeIfPresent([Int].self, forKey: .weekdays) ?? [])
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .type, in: c, debugDescription: "no repeat \(other)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .interval(minutes):
            try c.encode("interval", forKey: .type)
            try c.encode(minutes, forKey: .minutes)
        case let .fixedTime(hour, minute, weekdays):
            try c.encode("fixed_time", forKey: .type)
            try c.encode(hour, forKey: .hour)
            try c.encode(minute, forKey: .minute)
            try c.encode(weekdays, forKey: .weekdays)
        }
    }
}

/// The choices the phone offers for a repeat, as the desktop's
/// (`apps/desktop/src/repeats.ts`), and the rule said back in words. A time
/// picked on the phone is on the phone's clock, and moved onto the server's
/// by the difference between the two; read back, it is moved home again.
public enum Cadence: Hashable, Sendable, Identifiable {
    case daily
    case weekdays
    /// 0 is Sunday.
    case weekly(Int)
    case hourly

    public var id: String { label }

    public static let all: [Cadence] = [.daily, .weekdays] + (0..<7).map(Cadence.weekly) + [.hourly]

    static let days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]

    public var label: String {
        switch self {
        case .daily: "Every Day"
        case .weekdays: "Every Weekday"
        case let .weekly(d): "Every \(Self.days[d])"
        case .hourly: "Every Hour"
        }
    }

    /// Whether a time of day goes with it.
    public var takesTime: Bool { self != .hourly }

    /// The rule for this cadence at `hour`:`minute` on the phone's clock, for a
    /// server `serverOffset` seconds from UTC.
    public func rule(hour: Int, minute: Int, serverOffset: Int,
                     localOffset: Int = TimeZone.current.secondsFromGMT()) -> Repeat {
        if self == .hourly { return .interval(minutes: 60) }
        let days: [Int] = switch self {
        case .daily, .hourly: []
        case .weekdays: [1, 2, 3, 4, 5]
        case let .weekly(d): [d]
        }
        let moved = Cadence.shift(hour: hour, minute: minute, weekdays: days,
                                  by: (serverOffset - localOffset) / 60)
        return .fixedTime(hour: moved.hour, minute: moved.minute, weekdays: moved.weekdays)
    }

    /// A time of day and its days moved by `minutes`: across midnight the
    /// days move with it.
    static func shift(hour: Int, minute: Int, weekdays: [Int], by minutes: Int)
        -> (hour: Int, minute: Int, weekdays: [Int]) {
        let total = hour * 60 + minute + minutes
        let days = Int((Double(total) / 1440).rounded(.down))
        let within = ((total % 1440) + 1440) % 1440
        return (within / 60, within % 60, weekdays.map { (($0 + days) % 7 + 7) % 7 }.sorted())
    }
}

extension Repeat {
    /// The rule in words, on the phone's clock: "Every weekday at 09:00".
    public func describe(serverOffset: Int, localOffset: Int = TimeZone.current.secondsFromGMT()) -> String {
        switch self {
        case let .interval(minutes):
            if minutes == 60 { return "Every hour" }
            return minutes % 60 == 0 ? "Every \(minutes / 60) hours" : "Every \(minutes) minutes"
        case let .fixedTime(hour, minute, weekdays):
            let here = Cadence.shift(hour: hour, minute: minute, weekdays: weekdays,
                                     by: -(serverOffset - localOffset) / 60)
            let time = String(format: "%02d:%02d", here.hour, here.minute)
            let days = here.weekdays
            let which = days.isEmpty || days.count == 7 ? "Every day"
                : days == [1, 2, 3, 4, 5] ? "Every weekday"
                : days == [0, 6] ? "Every weekend"
                : "Every " + days.map { Cadence.days[$0] }.joined(separator: ", ")
            return "\(which) at \(time)"
        }
    }
}
