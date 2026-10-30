import Foundation
import HuraCoreFFI

// Tickets and pull requests: what is yours on each tracker. Read by this
// phone with this phone's tokens, through hura-core's own tracker code
// (crates/hura-mobile), exactly as the desktop reads them on its machine. No
// server holds a tracker or a token.

/// What hura-core's tracker code answered, or why it couldn't.
public struct TrackerError: Error, LocalizedError, Equatable {
    public var message: String

    public init(message: String) {
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// hura-core's tracker functions, one JSON request at a time.
public enum TrackerCore {
    /// Runs `op` with `args` off the main thread: the call blocks until the
    /// tracker answers.
    public static func call(_ op: String, _ args: [String: JSONValue] = [:]) async throws -> JSONValue {
        var request = args
        request["op"] = .string(op)
        let body = try String(decoding: JSONEncoder().encode(JSONValue.object(request)), as: UTF8.self)
        let answer: String = await Task.detached(priority: .userInitiated) {
            body.withCString { ptr in
                guard let out = hura_tracker(ptr) else { return #"{"error":"no answer"}"# }
                defer { hura_free(out) }
                return String(cString: out)
            }
        }.value
        let v = try JSONDecoder().decode(JSONValue.self, from: Data(answer.utf8))
        if let e = v["error"]?.string { throw TrackerError(message: e) }
        return v["ok"] ?? .null
    }

    static func decode<T: Decodable>(_ v: JSONValue, as: T.Type) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONEncoder().encode(v))
    }

    static func json<T: Encodable>(_ value: T) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
    }
}

// MARK: Trackers

/// A tracker this phone reads (`Source` in hura-core's tracker module).
public struct TrackerSource: Codable, Equatable, Hashable, Sendable {
    /// `git-hub`, `azure-dev-ops` or `jira`.
    public var kind: String
    public var name: String
    /// GitHub: `owner/name`, or nil for every repository the token sees.
    public var repo: String?
    /// Azure DevOps organisation and project.
    public var org: String?
    public var project: String?
    /// Jira site and the account the token belongs to.
    public var site: String?
    public var email: String?

    public init(kind: String, name: String = "", repo: String? = nil, org: String? = nil, project: String? = nil,
                site: String? = nil, email: String? = nil) {
        self.kind = kind
        self.name = name
        self.repo = repo
        self.org = org
        self.project = project
        self.site = site
        self.email = email
    }

    public var kindLabel: String {
        switch kind {
        case "git-hub": "GitHub"
        case "azure-dev-ops": "Azure DevOps"
        case "jira": "Jira"
        default: kind
        }
    }
}

/// A tracker and its token, as the Keychain keeps it. The token goes to
/// hura-core for one call at a time and is never shown.
public struct StoredTracker: Codable, Equatable, Sendable {
    public var source: TrackerSource
    public var token: String?

    public init(source: TrackerSource, token: String?) {
        self.source = source
        self.token = token
    }

    public var hasToken: Bool { !(token ?? "").trimmingCharacters(in: .whitespaces).isEmpty }
}

/// Where the trackers live: the Keychain, this device only.
public protocol TrackerStore: Sendable {
    func load() throws -> [StoredTracker]
    func save(_ trackers: [StoredTracker]) throws
}

public struct KeychainTrackerStore: TrackerStore {
    let items: KeychainItems

    /// In the app's own group, out of the one the notification extension
    /// shares: nothing but the app needs a tracker token.
    public init(service: String = "dev.tobiaswadseth.hura.trackers") {
        items = KeychainItems(service: service, accessGroup: nil)
    }

    public func load() throws -> [StoredTracker] {
        guard let data = try items.get(account: "trackers") else { return [] }
        return try JSONDecoder().decode([StoredTracker].self, from: data)
    }

    public func save(_ trackers: [StoredTracker]) throws {
        try items.set(JSONEncoder().encode(trackers), account: "trackers")
    }
}

public final class InMemoryTrackerStore: TrackerStore, @unchecked Sendable {
    private var trackers: [StoredTracker] = []
    private let lock = NSLock()
    public init() {}
    public func load() throws -> [StoredTracker] { lock.withLock { trackers } }
    public func save(_ t: [StoredTracker]) throws { lock.withLock { trackers = t } }
}

// MARK: The inbox

/// A ticket that is yours (`Task`).
public struct TrackerTask: Codable, Equatable, Identifiable, Hashable, Sendable {
    public var tracker: String
    public var kind: String
    public var id: String
    public var key: String
    public var title: String
    public var url: String
    public var status: String
    public var itemType: String
    /// The session name it suggests.
    public var sessionName: String
    /// The branch it suggests, under the prefix the inbox was read with.
    public var branch: String
    public var repo: String?
    /// `assigned` or `created`.
    public var relation: String
    public var updated: String?
    public var comments: Int?
    public var lastCommenter: String?
    public var lastCommentMine: Bool

    enum CodingKeys: String, CodingKey {
        case tracker, kind, id, key, title, url, status, branch, repo, relation, updated, comments
        case itemType = "item_type"
        case sessionName = "session_name"
        case lastCommenter = "last_commenter"
        case lastCommentMine = "last_comment_mine"
    }
}

/// An open pull request that is yours or waits on you.
public struct PullRequestSummary: Codable, Equatable, Identifiable, Hashable, Sendable {
    public var tracker: String
    public var kind: String
    public var id: String
    public var key: String
    public var title: String
    public var url: String
    /// `owner/name` on GitHub; the repository's name on Azure DevOps.
    public var repo: String
    public var repoURL: String
    public var relation: String
    public var reviewRequested: Bool
    public var author: String
    public var branch: String
    public var base: String
    public var draft: Bool
    public var fork: Bool
    public var conflicts: Bool
    /// `passing`, `failing` or `pending`.
    public var checks: String?
    /// `approved`, `changes-requested` or `pending`.
    public var review: String?
    public var comments: Int?
    public var updated: String?

    enum CodingKeys: String, CodingKey {
        case tracker, kind, id, key, title, url, repo, relation, author, branch, base, draft, fork, conflicts, checks,
             review, comments, updated
        case repoURL = "repo_url"
        case reviewRequested = "review_requested"
    }
}

public struct Inbox: Codable, Equatable, Sendable {
    public var tasks: [TrackerTask]
    public var pulls: [PullRequestSummary]
    public var warnings: [String]
    /// The trackers whose tickets were read.
    public var read: [String]
    public var pullsRead: [String]

    enum CodingKeys: String, CodingKey {
        case tasks, pulls, warnings, read
        case pullsRead = "pulls_read"
    }

    public init(tasks: [TrackerTask] = [], pulls: [PullRequestSummary] = [], warnings: [String] = [], read: [String] = [],
                pullsRead: [String] = []) {
        self.tasks = tasks
        self.pulls = pulls
        self.warnings = warnings
        self.read = read
        self.pullsRead = pullsRead
    }
}

// MARK: A ticket

public struct IssuePerson: Codable, Equatable, Hashable, Sendable {
    public var name: String
    public var me: Bool
}

public struct IssueComment: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var author: IssuePerson?
    public var created: String?
    public var edited: String?
    /// Document blocks, as hura-core reads Jira's ADF: drawn by the app.
    public var body: JSONValue
    /// The body as Markdown, for editing, with what that would lose.
    public var markdown: MarkdownText?

    public struct MarkdownText: Codable, Equatable, Sendable {
        public var text: String
        /// What converting to Markdown dropped, for a warning beside an edit.
        public var lost: [String]

        public init(text: String, lost: [String]) {
            self.text = text
            self.lost = lost
        }
    }
}

/// A ticket in full.
public struct TrackerIssue: Decodable, Equatable, Sendable {
    public var tracker: String
    public var kind: String
    public var key: String
    public var title: String
    public var url: String
    public var status: String
    /// `todo`, `doing` or `done`.
    public var stage: String
    public var itemType: String
    public var priority: String?
    public var assignee: IssuePerson?
    public var reporter: IssuePerson?
    public var labels: [String]
    public var parent: Parent?
    public var created: String?
    public var updated: String?
    public var due: String?
    public var description: JSONValue
    public var comments: [IssueComment]
    public var commentsTotal: Int

    public struct Parent: Decodable, Equatable, Sendable {
        public var key: String
        public var title: String
    }

    enum CodingKeys: String, CodingKey {
        case tracker, kind, key, title, url, status, stage, priority, assignee, reporter, labels, parent, created, updated,
             due, description, comments
        case itemType = "item_type"
        case commentsTotal = "comments_total"
    }
}

public struct IssueTransition: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var toID: String
    public var to: String
    public var stage: String

    enum CodingKeys: String, CodingKey {
        case id, name, to, stage
        case toID = "to_id"
    }
}

public struct Choice: Codable, Equatable, Hashable, Identifiable, Sendable {
    public var id: String
    public var name: String
}

public struct UserChoice: Codable, Equatable, Hashable, Sendable {
    public var accountID: String
    public var name: String

    enum CodingKeys: String, CodingKey {
        case name
        case accountID = "account_id"
    }
}

/// A field's value and the control it takes (`Editable`).
public enum Editable: Codable, Equatable, Sendable {
    case text(String)
    case doc(IssueComment.MarkdownText)
    case number(Double?)
    case date(String?)
    case select(String?, options: [Choice])
    case multi([String], options: [Choice])
    case labels([String])
    case user(UserChoice?, assignable: Bool)
    case readOnly(String)

    private enum Key: String, CodingKey { case kind, value, options, assignable }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Key.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "text": self = .text(try c.decode(String.self, forKey: .value))
        case "doc": self = .doc(try c.decode(IssueComment.MarkdownText.self, forKey: .value))
        case "number": self = .number(try c.decodeIfPresent(Double.self, forKey: .value))
        case "date": self = .date(try c.decodeIfPresent(String.self, forKey: .value))
        case "select": self = .select(try c.decodeIfPresent(String.self, forKey: .value), options: try c.decode([Choice].self, forKey: .options))
        case "multi": self = .multi(try c.decode([String].self, forKey: .value), options: try c.decode([Choice].self, forKey: .options))
        case "labels": self = .labels(try c.decode([String].self, forKey: .value))
        case "user": self = .user(try c.decodeIfPresent(UserChoice.self, forKey: .value), assignable: try c.decode(Bool.self, forKey: .assignable))
        default: self = .readOnly((try? c.decode(String.self, forKey: .value)) ?? "")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Key.self)
        switch self {
        case let .text(v): try c.encode("text", forKey: .kind); try c.encode(v, forKey: .value)
        case let .doc(v): try c.encode("doc", forKey: .kind); try c.encode(v, forKey: .value)
        case let .number(v): try c.encode("number", forKey: .kind); try c.encode(v, forKey: .value)
        case let .date(v): try c.encode("date", forKey: .kind); try c.encode(v, forKey: .value)
        case let .select(v, o): try c.encode("select", forKey: .kind); try c.encode(v, forKey: .value); try c.encode(o, forKey: .options)
        case let .multi(v, o): try c.encode("multi", forKey: .kind); try c.encode(v, forKey: .value); try c.encode(o, forKey: .options)
        case let .labels(v): try c.encode("labels", forKey: .kind); try c.encode(v, forKey: .value)
        case let .user(v, a): try c.encode("user", forKey: .kind); try c.encode(v, forKey: .value); try c.encode(a, forKey: .assignable)
        case let .readOnly(v): try c.encode("read-only", forKey: .kind); try c.encode(v, forKey: .value)
        }
    }
}

public struct EditField: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var required: Bool
    public var edit: Editable
}

public struct EditForm: Decodable, Equatable, Sendable {
    public var tracker: String
    public var key: String
    public var fields: [EditField]
}

public struct FieldChange: Encodable, Equatable, Sendable {
    public var id: String
    public var edit: Editable

    public init(id: String, edit: Editable) {
        self.id = id
        self.edit = edit
    }
}

// MARK: A pull request

public struct PullComment: Decodable, Equatable, Sendable {
    public var author: IssuePerson?
    public var body: String
    public var created: String?
}

public struct PullThread: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var path: String?
    public var line: Int?
    public var resolved: Bool
    public var outdated: Bool
    public var verdict: String?
    public var author: IssuePerson?
    public var at: String?
    public var comments: [PullComment]
    public var canReply: Bool

    enum CodingKeys: String, CodingKey {
        case id, path, line, resolved, outdated, verdict, author, at, comments
        case canReply = "can_reply"
    }
}

public struct PullCheck: Decodable, Equatable, Sendable {
    public var name: String
    /// `passing`, `failing`, `pending`, `neutral` or `skipped`.
    public var state: String
    public var url: String?
    public var detail: String?
    public var required: Bool
}

public struct PullReviewer: Decodable, Equatable, Sendable {
    public var who: IssuePerson
    public var verdict: String
    public var required: Bool
    public var group: Bool
}

public struct PullDetail: Decodable, Equatable, Sendable {
    public var tracker: String
    public var kind: String
    public var id: String
    public var key: String
    public var repo: String
    public var title: String
    public var url: String
    /// `open`, `merged` or `closed`.
    public var state: String
    public var draft: Bool
    public var conflicts: Bool
    public var fork: Bool
    public var author: IssuePerson?
    public var branch: String
    public var base: String
    public var created: String?
    public var updated: String?
    /// Markdown.
    public var body: String
    public var additions: Int?
    public var deletions: Int?
    public var changedFiles: Int?
    public var checks: [PullCheck]
    public var reviewers: [PullReviewer]
    public var conversation: [PullThread]
    /// Threads older than the ones shown.
    public var older: Int
    /// The verdicts this account may give.
    public var offers: [String]
    public var myVote: String?
    /// The pull request is this account's own.
    public var mine: Bool
    public var warnings: [String]

    enum CodingKeys: String, CodingKey {
        case tracker, kind, id, key, repo, title, url, state, draft, conflicts, fork, author, branch, base, created,
             updated, body, additions, deletions, checks, reviewers, conversation, older, offers, mine, warnings
        case changedFiles = "changed_files"
        case myVote = "my_vote"
    }
}

public struct PullFile: Codable, Equatable, Hashable, Identifiable, Sendable {
    public var path: String
    public var oldPath: String?
    public var change: String
    public var additions: Int?
    public var deletions: Int?
    public var id: String { path }

    enum CodingKeys: String, CodingKey {
        case path, change, additions, deletions
        case oldPath = "old_path"
    }
}

public struct PullFiles: Codable, Equatable, Sendable {
    public var repo: String
    public var base: String
    public var head: String
    public var files: [PullFile]
    public var truncated: Bool
}

// MARK: Calls

extension TrackerCore {
    /// The source trimmed and named, or why it wouldn't work.
    public static func check(_ source: TrackerSource) async throws -> TrackerSource {
        try decode(try await call("check", ["source": json(source)]), as: TrackerSource.self)
    }

    /// Every tracker's tickets and pull requests, one after another.
    public static func inbox(_ trackers: [StoredTracker], prefix: String) async throws -> Inbox {
        try decode(try await call("inbox", ["trackers": json(trackers), "prefix": .string(prefix)]), as: Inbox.self)
    }

    /// The branch a ticket's worktree gets under `prefix`.
    public static func branch(prefix: String, key: String, title: String) async throws -> String {
        try await call("branch", ["prefix": .string(prefix), "key": .string(key), "title": .string(title)]).string ?? ""
    }

    public static func issue(_ t: StoredTracker, key: String) async throws -> TrackerIssue {
        try decode(try await call("issue", ["tracker": json(t), "key": .string(key)]), as: TrackerIssue.self)
    }

    public static func editForm(_ t: StoredTracker, key: String) async throws -> EditForm {
        try decode(try await call("edit_form", ["tracker": json(t), "key": .string(key)]), as: EditForm.self)
    }

    public static func save(_ t: StoredTracker, key: String, changes: [FieldChange]) async throws {
        _ = try await call("save", ["tracker": json(t), "key": .string(key), "changes": json(changes)])
    }

    public static func transitions(_ t: StoredTracker, key: String) async throws -> [IssueTransition] {
        try decode(try await call("transitions", ["tracker": json(t), "key": .string(key)]), as: [IssueTransition].self)
    }

    public static func transition(_ t: StoredTracker, key: String, id: String) async throws {
        _ = try await call("transition", ["tracker": json(t), "key": .string(key), "id": .string(id)])
    }

    public static func comment(_ t: StoredTracker, key: String, markdown: String) async throws {
        _ = try await call("comment", ["tracker": json(t), "key": .string(key), "markdown": .string(markdown)])
    }

    public static func editComment(_ t: StoredTracker, key: String, id: String, markdown: String) async throws {
        _ = try await call("edit_comment", ["tracker": json(t), "key": .string(key), "id": .string(id), "markdown": .string(markdown)])
    }

    public static func deleteComment(_ t: StoredTracker, key: String, id: String) async throws {
        _ = try await call("delete_comment", ["tracker": json(t), "key": .string(key), "id": .string(id)])
    }

    public static func users(_ t: StoredTracker, key: String, query: String, assignable: Bool) async throws -> [UserChoice] {
        try decode(try await call("users", ["tracker": json(t), "key": .string(key), "query": .string(query),
                                            "assignable": .bool(assignable)]), as: [UserChoice].self)
    }

    public static func pull(_ t: StoredTracker, repo: String, id: String) async throws -> PullDetail {
        try decode(try await call("pull", ["tracker": json(t), "repo": .string(repo), "id": .string(id)]), as: PullDetail.self)
    }

    public static func pullFiles(_ t: StoredTracker, repo: String, id: String) async throws -> PullFiles {
        try decode(try await call("pull_files", ["tracker": json(t), "repo": .string(repo), "id": .string(id)]), as: PullFiles.self)
    }

    public static func pullFile(_ t: StoredTracker, files: PullFiles, file: PullFile) async throws -> FileDiff {
        try decode(try await call("pull_file", ["tracker": json(t), "repo": .string(files.repo), "base": .string(files.base),
                                                "head": .string(files.head), "file": json(file)]), as: FileDiff.self)
    }

    public static func pullComment(_ t: StoredTracker, repo: String, id: String, body: String, thread: String?) async throws {
        var args: [String: JSONValue] = ["tracker": try json(t), "repo": .string(repo), "id": .string(id), "body": .string(body)]
        if let thread { args["thread"] = .string(thread) }
        _ = try await call("pull_comment", args)
    }

    /// `approved`, `suggestions`, `changes-requested`, `waiting`, `rejected`
    /// or `commented`, as `offers` allows.
    public static func pullVote(_ t: StoredTracker, repo: String, id: String, verdict: String, body: String) async throws {
        _ = try await call("pull_vote", ["tracker": json(t), "repo": .string(repo), "id": .string(id), "verdict": .string(verdict),
                                         "body": .string(body)])
    }
}

/// A repository's address with what varies between two spellings taken off,
/// so a project cloned over SSH is still the repository its pull requests
/// name over HTTPS (`bare` in the desktop's Inbox.tsx).
public func bareRepo(_ url: String) -> String {
    var s = url.trimmingCharacters(in: .whitespaces).lowercased()
    s = s.replacing(/^[a-z+]+:\/\//, with: "")
    s = s.replacing(/^[^@\/]+@/, with: "")
    while s.hasSuffix("/") { s.removeLast() }
    if s.hasSuffix(".git") { s.removeLast(4) }
    if s.firstMatch(of: /^[a-z0-9.-]+:[^\/\d]/) != nil, let colon = s.firstIndex(of: ":") {
        s.replaceSubrange(colon...colon, with: "/")
    }
    return s
}
