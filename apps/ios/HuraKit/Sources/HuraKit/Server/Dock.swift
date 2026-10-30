import Foundation

// What is true about a worktree beside its tabs: its files, its git, the
// review, and the network policy with the decisions it made. The desktop
// window's dock (`apps/desktop/src/Dock.tsx`); each type mirrors one in
// hura-core.

// MARK: Files

/// One directory of the working copy, read on demand.
public struct Dir: Decodable, Equatable, Sendable {
    public var path: String
    public var entries: [Entry]

    public struct Entry: Decodable, Equatable, Hashable, Sendable {
        public var name: String
        public var dir: Bool
    }
}

/// A file of the working copy, read-only: the agent owns it.
public struct FileText: Decodable, Equatable, Sendable {
    public var path: String
    public var text: String
    public var bytes: Int
    /// Only the first part was read.
    public var truncated: Bool
    public var binary: Bool
}

// MARK: Git

public struct ChangedFile: Decodable, Equatable, Hashable, Sendable {
    public var path: String
    /// `added`, `modified`, `deleted`, `renamed`, `untracked` or `conflicted`.
    public var change: String
}

public struct GitStatus: Decodable, Equatable, Sendable {
    public var branch: String
    public var upstream: String?
    public var ahead: Int
    public var behind: Int
    public var staged: [ChangedFile]
    public var unstaged: [ChangedFile]
}

/// What git said, and the status re-read after it: the agent is editing while
/// this runs, so the status after a stage isn't the one before it plus one.
public struct GitAnswer: Decodable, Equatable, Sendable {
    public var said: String
    public var status: GitStatus
}

/// One thing to do to the working copy or the remote (`GitOp`).
public enum GitOp: Encodable, Equatable, Sendable {
    case stage(String)
    case unstage(String)
    case discard(String)
    case commit(String)
    case push
    case pull
    case fetch

    private enum Key: String, CodingKey { case `do`, path, message }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Key.self)
        switch self {
        case let .stage(p): try c.encode("stage", forKey: .do); try c.encode(p, forKey: .path)
        case let .unstage(p): try c.encode("unstage", forKey: .do); try c.encode(p, forKey: .path)
        case let .discard(p): try c.encode("discard", forKey: .do); try c.encode(p, forKey: .path)
        case let .commit(m): try c.encode("commit", forKey: .do); try c.encode(m, forKey: .message)
        case .push: try c.encode("push", forKey: .do)
        case .pull: try c.encode("pull", forKey: .do)
        case .fetch: try c.encode("fetch", forKey: .do)
        }
    }
}

/// Which two sides of a file a diff compares.
public enum Against: String, Codable, Sendable {
    /// The index against HEAD.
    case staged
    /// The working copy against the index.
    case worktree
    /// The working copy against the branch it started from.
    case base
}

/// Both sides of one file's diff.
public struct FileDiff: Decodable, Equatable, Sendable {
    public var path: String
    public var original: String
    public var modified: String
    public var originalLabel: String
    public var modifiedLabel: String
    public var binary: Bool

    enum CodingKeys: String, CodingKey {
        case path, original, modified, binary
        case originalLabel = "original_label"
        case modifiedLabel = "modified_label"
    }
}

// MARK: Review

/// A remark on one line, kept on the server until it is sent to the agent.
public struct Comment: Decodable, Equatable, Identifiable, Sendable {
    public var id: Int64
    public var file: String
    public var line: Int
    /// The line as it read when the remark was made.
    public var excerpt: String
    public var body: String
    public var at: Int64
}

public struct NewComment: Encodable, Equatable, Sendable {
    public var file: String
    public var line: Int
    public var excerpt: String
    public var body: String

    public init(file: String, line: Int, excerpt: String, body: String) {
        self.file = file
        self.line = line
        self.excerpt = excerpt
        self.body = body
    }
}

// MARK: Policy

/// A method and a path on an endpoint (`Route` in hura-core).
public struct PathRoute: Codable, Equatable, Hashable, Sendable {
    public var method: String
    public var path: String

    public init(method: String, path: String) {
        self.method = method
        self.path = path
    }
}

/// One rule on the sandbox.
public struct PolicyRule: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var allow: Bool
    public var hosts: [String]
    /// Empty for every method, which is the whole host.
    public var methods: [String]
    public var path: String?
    /// From the runtime's own policy, for every sandbox: not changed here.
    public var global: Bool
}

public struct Listed: Decodable, Equatable, Sendable {
    public var endpoint: String
    /// Whether this session has it.
    public var inPolicy: Bool

    enum CodingKeys: String, CodingKey {
        case endpoint
        case inPolicy = "in_policy"
    }
}

public struct ListedRoutes: Decodable, Equatable, Sendable {
    public var endpoint: String
    public var routes: [PathRoute]
    public var inPolicy: Bool

    enum CodingKeys: String, CodingKey {
        case endpoint, routes
        case inPolicy = "in_policy"
    }
}

/// What every new session starts from.
public struct PolicyLists: Decodable, Equatable, Sendable {
    public var allow: [Listed]
    public var routes: [ListedRoutes]
    public var block: [Listed]
}

/// A session's network policy (`View` in hura-core's policy module).
public struct PolicyView: Decodable, Equatable, Sendable {
    public var template: String?
    public var rules: [PolicyRule]
    public var lists: PolicyLists?
}

// MARK: Events

/// One decision the sandbox runtime made.
public struct PolicyEvent: Decodable, Equatable, Hashable, Sendable {
    /// Epoch seconds.
    public var at: Int64
    public var count: Int64
    public var last: Int64
    /// `NET:OPEN`, `HTTP:POST`, …
    public var `class`: String
    /// `Info`, `Medium`, `High`, `Critical` or `Other`.
    public var severity: String
    /// `Allowed`, `Denied` or `Neutral`.
    public var verdict: String
    public var subject: String
    public var policy: String?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case at, count, last, `class`, severity, verdict, subject, policy, reason
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        at = try c.decode(Int64.self, forKey: .at)
        count = try c.decodeIfPresent(Int64.self, forKey: .count) ?? 1
        last = try c.decodeIfPresent(Int64.self, forKey: .last) ?? 0
        `class` = try c.decode(String.self, forKey: .class)
        severity = try c.decode(String.self, forKey: .severity)
        verdict = try c.decode(String.self, forKey: .verdict)
        subject = try c.decode(String.self, forKey: .subject)
        policy = try c.decodeIfPresent(String.self, forKey: .policy)
        reason = try c.decodeIfPresent(String.self, forKey: .reason)
    }

    public init(at: Int64, subject: String, verdict: String = "Denied", class: String = "NET:OPEN",
                reason: String? = nil, count: Int64 = 1) {
        self.at = at
        self.count = count
        last = at
        self.class = `class`
        severity = "Info"
        self.verdict = verdict
        self.subject = subject
        policy = nil
        self.reason = reason
    }

    /// The endpoint this was about, when it was about one: what makes the
    /// feed actionable, since an allow and a block both take one. Read the way
    /// `Event::target` in hura-core reads it, and as strictly: prose must
    /// never become a policy change.
    public var endpoint: String? {
        let s = subject.trimmingCharacters(in: .whitespaces)
        // `/usr/bin/curl(79) -> pastebin.com:443`, and with a request after
        // the arrow.
        if let arrow = s.range(of: " -> ") {
            let right = String(s[arrow.upperBound...])
            return endpoint(of: Self.request(right) ?? right)
        }
        // `GET httpbin.org:443/ip`
        if let authority = Self.request(s) { return endpoint(of: authority) }
        return Self.hostPort(s)
    }

    /// `host:port` from an authority, or from the reason when the authority
    /// left the port out and the reason names the same host.
    private func endpoint(of authority: String) -> String? {
        if let e = Self.hostPort(authority) { return e }
        guard let words = reason?.split(whereSeparator: \.isWhitespace),
              let i = words.firstIndex(of: "endpoint"), i + 1 < words.count,
              let e = Self.hostPort(String(words[i + 1])),
              let colon = e.lastIndex(of: ":"), e[..<colon] == authority
        else { return nil }
        return e
    }

    /// The authority of `GET host:443/path`: exactly two words, an uppercase
    /// method first. Anything else is prose.
    static func request(_ s: String) -> String? {
        let words = s.split(whereSeparator: \.isWhitespace)
        guard words.count == 2, words[0].allSatisfy({ $0.isASCII && $0.isUppercase }) else { return nil }
        return String(words[1].split(separator: "/", omittingEmptySubsequences: false).first ?? words[1])
    }

    static func hostPort(_ s: String) -> String? {
        let t = s.trimmingCharacters(in: .whitespaces)
        guard let colon = t.lastIndex(of: ":") else { return nil }
        let host = t[..<colon]
        guard !host.isEmpty, host.contains("."),
              host.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "." || $0 == "-") }),
              let port = UInt16(t[t.index(after: colon)...]), port != 0
        else { return nil }
        return "\(host):\(port)"
    }

    /// The request a denial was about, `POST /o/r.git/git-receive-pack`, for
    /// offering to allow just that path.
    public var request: PathRoute? {
        let tail = subject.range(of: " -> ").map { String(subject[$0.upperBound...]) } ?? subject
        let words = tail.split(whereSeparator: \.isWhitespace)
        guard words.count == 2, words[0].allSatisfy({ $0.isASCII && $0.isUppercase }) else { return nil }
        let rest = words[1]
        let path = rest.firstIndex(of: "/").map { String(rest[$0...]) } ?? "/"
        return PathRoute(method: String(words[0]), path: String(path.split(separator: "?", maxSplits: 1).first ?? "/"))
    }
}

// MARK: Usage

/// What a session has spent, from its agent's status line.
public struct Usage: Decodable, Equatable, Sendable {
    public var model: String?
    public var costUSD: Double?
    public var durationMS: Int64?
    public var linesAdded: Int64?
    public var linesRemoved: Int64?
    public var contextUsedPercentage: Double?
    public var contextSize: Int64?

    enum CodingKeys: String, CodingKey {
        case model
        case costUSD = "cost_usd"
        case durationMS = "duration_ms"
        case linesAdded = "lines_added"
        case linesRemoved = "lines_removed"
        case contextUsedPercentage = "context_used_percentage"
        case contextSize = "context_size"
    }
}

/// How much the working copy differs from its base.
public struct DiffStat: Decodable, Equatable, Sendable {
    public var added: Int
    public var removed: Int
    public var untracked: Int
}

// MARK: Replies

struct CommentsReply: Decodable {
    var comments: [Comment]
}

struct EventsReply: Decodable {
    var events: [PolicyEvent]
}
