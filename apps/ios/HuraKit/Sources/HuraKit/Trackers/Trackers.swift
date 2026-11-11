import Foundation

// Tickets and pull requests: what is yours on each tracker. Read by hura
// cloud, which keeps the account's trackers and their tokens and runs
// hura-core's own tracker code on them, exactly as for the desktop. Signed
// out, there are no trackers.

/// What hura-core's tracker code answered, or why it couldn't.
public struct TrackerError: Error, LocalizedError, Equatable {
    public var message: String

    public init(message: String) {
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// hura-core's tracker functions, one JSON request at a time, run by hura
/// cloud on the account's trackers (`POST /v1/tracker`). A request names its
/// tracker; the service has the rest.
public enum TrackerCore {
    /// Where requests go. A task-local so a test can point it at a service
    /// of its own.
    @TaskLocal public static var cloud: Cloud = .shared

    /// Runs `op` with `args` on hura cloud.
    public static func call(_ op: String, _ args: [String: JSONValue] = [:]) async throws -> JSONValue {
        var request = args
        request["op"] = .string(op)
        guard await cloud.isSignedIn else {
            throw TrackerError(message: "Sign in to hura cloud to use trackers.")
        }
        return try answer(try await cloud.tracker(request))
    }

    /// `{"ok": ...}` or `{"error": ...}`.
    private static func answer(_ v: JSONValue) throws -> JSONValue {
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
    /// `git-hub`, `azure-dev-ops`, `jira` or `fizzy`.
    public var kind: String
    public var name: String
    /// GitHub: `owner/name`, or nil for every repository the token sees.
    public var repo: String?
    /// Azure DevOps organisation and project. On Fizzy, `org` is the one
    /// account to read, or nil for every account the token belongs to.
    public var org: String?
    public var project: String?
    /// Jira site and the account the token belongs to. On Fizzy, a site of
    /// your own, or nil for `app.fizzy.do`.
    public var site: String?
    public var email: String?
    /// Leave out the tickets you created that nobody assigned you.
    public var hideCreated: Bool
    /// Jira only: named JQL queries read beside your own tickets, each a
    /// section of its own.
    public var jqlFilters: [JqlFilter]

    enum CodingKeys: String, CodingKey {
        case kind, name, repo, org, project, site, email
        case hideCreated = "hide_created"
        case jqlFilters = "jql_filters"
    }

    public init(kind: String, name: String = "", repo: String? = nil, org: String? = nil, project: String? = nil,
                site: String? = nil, email: String? = nil, hideCreated: Bool = false, jqlFilters: [JqlFilter] = []) {
        self.kind = kind
        self.name = name
        self.repo = repo
        self.org = org
        self.project = project
        self.site = site
        self.email = email
        self.hideCreated = hideCreated
        self.jqlFilters = jqlFilters
    }

    /// A tracker kept before there were filters has neither field.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        repo = try c.decodeIfPresent(String.self, forKey: .repo)
        org = try c.decodeIfPresent(String.self, forKey: .org)
        project = try c.decodeIfPresent(String.self, forKey: .project)
        site = try c.decodeIfPresent(String.self, forKey: .site)
        email = try c.decodeIfPresent(String.self, forKey: .email)
        hideCreated = try c.decodeIfPresent(Bool.self, forKey: .hideCreated) ?? false
        jqlFilters = try c.decodeIfPresent([JqlFilter].self, forKey: .jqlFilters) ?? []
    }

    public var kindLabel: String {
        switch kind {
        case "git-hub": "GitHub"
        case "azure-dev-ops": "Azure DevOps"
        case "jira": "Jira"
        case "fizzy": "Fizzy"
        default: kind
        }
    }
}

/// One named JQL query on a Jira tracker (`JqlFilter`).
public struct JqlFilter: Codable, Equatable, Hashable, Sendable {
    public var name: String
    public var jql: String

    public init(name: String, jql: String) {
        self.name = name
        self.jql = jql
    }
}

/// One of the account's trackers. hura cloud keeps its token and never
/// sends it back: `hasToken` says whether there is one.
public struct StoredTracker: Equatable, Sendable {
    public var source: TrackerSource
    public var hasToken: Bool
    /// One hura cloud connected through the tracker's own sign-in: who as
    /// (`@octocat`, or a Jira site). Its token is renewed there, and
    /// there is none to paste.
    public var connected: String?
    /// Whether hura cloud hears of a change on it at once, in words
    /// (`webhooks on 3 boards`), or why it reads on its timer. Fizzy's only.
    public var webhooks: String?

    public init(source: TrackerSource, hasToken: Bool = false, connected: String? = nil, webhooks: String? = nil) {
        self.source = source
        self.hasToken = hasToken
        self.connected = connected
        self.webhooks = webhooks
    }
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
    /// `assigned`, `created`, or `matched`: not yours, there for a filter.
    public var relation: String
    /// The JQL filters it is in, by name.
    public var filters: [String]
    public var updated: String?
    public var comments: Int?
    public var lastCommenter: String?
    public var lastCommentMine: Bool

    enum CodingKeys: String, CodingKey {
        case tracker, kind, id, key, title, url, status, branch, repo, relation, filters, updated, comments
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
    /// The JQL filters that answered.
    public var filtersRead: [FilterRead]

    enum CodingKeys: String, CodingKey {
        case tasks, pulls, warnings, read
        case pullsRead = "pulls_read"
        case filtersRead = "filters_read"
    }

    public init(tasks: [TrackerTask] = [], pulls: [PullRequestSummary] = [], warnings: [String] = [], read: [String] = [],
                pullsRead: [String] = [], filtersRead: [FilterRead] = []) {
        self.tasks = tasks
        self.pulls = pulls
        self.warnings = warnings
        self.read = read
        self.pullsRead = pullsRead
        self.filtersRead = filtersRead
    }
}

/// A JQL filter that answered: which tracker, and which of its filters.
public struct FilterRead: Codable, Equatable, Hashable, Sendable {
    public var tracker: String
    public var filter: String

    public init(tracker: String, filter: String) {
        self.tracker = tracker
        self.filter = filter
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
    /// The project's own fields that have a value, by name.
    public var custom: [CustomField]
    public var comments: [IssueComment]
    public var commentsTotal: Int

    public struct Parent: Decodable, Equatable, Sendable {
        public var key: String
        public var title: String
    }

    public struct CustomField: Decodable, Equatable, Identifiable, Sendable {
        public var id: String
        public var name: String
        public var value: String
        /// Rich text, to draw instead of `value`; an empty array otherwise.
        public var doc: JSONValue
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        tracker = try c.decode(String.self, forKey: .tracker)
        kind = try c.decode(String.self, forKey: .kind)
        key = try c.decode(String.self, forKey: .key)
        title = try c.decode(String.self, forKey: .title)
        url = try c.decode(String.self, forKey: .url)
        status = try c.decode(String.self, forKey: .status)
        stage = try c.decode(String.self, forKey: .stage)
        itemType = try c.decode(String.self, forKey: .itemType)
        priority = try c.decodeIfPresent(String.self, forKey: .priority)
        assignee = try c.decodeIfPresent(IssuePerson.self, forKey: .assignee)
        reporter = try c.decodeIfPresent(IssuePerson.self, forKey: .reporter)
        labels = try c.decode([String].self, forKey: .labels)
        parent = try c.decodeIfPresent(Parent.self, forKey: .parent)
        created = try c.decodeIfPresent(String.self, forKey: .created)
        updated = try c.decodeIfPresent(String.self, forKey: .updated)
        due = try c.decodeIfPresent(String.self, forKey: .due)
        description = try c.decode(JSONValue.self, forKey: .description)
        // Absent from a core older than custom fields.
        custom = try c.decodeIfPresent([CustomField].self, forKey: .custom) ?? []
        comments = try c.decode([IssueComment].self, forKey: .comments)
        commentsTotal = try c.decode(Int.self, forKey: .commentsTotal)
    }

    enum CodingKeys: String, CodingKey {
        case tracker, kind, key, title, url, status, stage, priority, assignee, reporter, labels, parent, created, updated,
             due, description, custom, comments
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
    /// How it can be merged from here; nil once it is no longer open.
    public var merging: PullMerging?
    public var warnings: [String]

    enum CodingKeys: String, CodingKey {
        case tracker, kind, id, key, repo, title, url, state, draft, conflicts, fork, author, branch, base, created,
             updated, body, additions, deletions, checks, reviewers, conversation, older, offers, mine, merging, warnings
        case changedFiles = "changed_files"
        case myVote = "my_vote"
    }
}

/// How an open pull request can go into its base (`PullMerging` in
/// hura-core's tracker module).
public struct PullMerging: Decodable, Equatable, Sendable {
    /// `merge`, `squash`, `rebase` or `rebase-merge`, in the forge's order.
    /// Empty when this account cannot merge it.
    public var methods: [String]
    /// Set to merge itself once its checks and reviews allow.
    public var auto: PullAutoMerge?
    /// Whether it can be set to merge itself.
    public var canAuto: Bool
    /// Whether the repository deletes the branch once merged (GitHub); nil
    /// when that is chosen with each merge (Azure DevOps).
    public var deletesBranch: Bool?
    /// The commit that would be merged, which the merge is checked against.
    public var head: String?

    enum CodingKeys: String, CodingKey {
        case methods, auto, head
        case canAuto = "can_auto"
        case deletesBranch = "deletes_branch"
    }
}

public struct PullAutoMerge: Decodable, Equatable, Sendable {
    public var by: IssuePerson?
    public var method: String?
    public var deleteBranch: Bool

    enum CodingKeys: String, CodingKey {
        case by, method
        case deleteBranch = "delete_branch"
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

// MARK: Branches

/// The branch a ticket's worktree gets under `prefix`: `prefix/KEY-the-title`,
/// the key keeping its case (commit hooks and people both look for
/// `PROJ-123`) and the title cut at a dash to forty characters. hura-core's
/// `tracker::branch`, which names the branches in an inbox, in Swift, so
/// the phone can name one for a server's prefix without asking.
public func ticketBranch(prefix: String, key: String, title: String) -> String {
    var key = Substring(key.trimmingCharacters(in: .whitespacesAndNewlines))
    while key.first == "#" { key = key.dropFirst() }
    let bare = key.replacingOccurrences(of: " ", with: "")
    let rest = branchSlug(title)
    let stem = rest.isEmpty ? bare : "\(bare)-\(truncateAtDash(rest, 40))"
    let p = prefix.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    return p.isEmpty ? stem : "\(p)/\(stem)"
}

/// Lowercase ASCII letters and digits, a dash for each run of anything else.
private func branchSlug(_ s: String) -> String {
    var out = ""
    for c in s.unicodeScalars {
        if c.isASCII, CharacterSet.alphanumerics.contains(c) {
            out.unicodeScalars.append(contentsOf: String(c).lowercased().unicodeScalars)
        } else if !out.hasSuffix("-") {
            out.append("-")
        }
    }
    return out.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
}

/// Cut to `max` characters, back to the last dash before it. The slug is
/// ASCII, so characters are bytes, as in hura-core.
private func truncateAtDash(_ s: String, _ max: Int) -> String {
    guard s.count > max else { return s }
    let cut = String(s.prefix(max))
    if let i = cut.lastIndex(of: "-"), i > cut.startIndex {
        return String(cut[..<i])
    }
    return cut.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
}

// MARK: Calls

extension TrackerCore {
    /// Every one of the account's trackers' tickets and pull requests.
    public static func inbox(prefix: String) async throws -> Inbox {
        try decode(try await call("inbox", ["prefix": .string(prefix)]), as: Inbox.self)
    }

    public static func issue(_ t: StoredTracker, key: String) async throws -> TrackerIssue {
        try decode(try await call("issue", ["tracker": .string(t.source.name), "key": .string(key)]), as: TrackerIssue.self)
    }

    /// One of a ticket's images, by the attachment id its document block
    /// carries: fetched with the tracker's token, which Jira wants for it.
    public static func image(_ t: StoredTracker, id: String) async throws -> Data {
        struct Image: Decodable { var mime: String; var data: String }
        let image = try decode(try await call("attachment", ["tracker": .string(t.source.name), "id": .string(id)]), as: Image.self)
        guard let data = Data(base64Encoded: image.data) else {
            throw TrackerError(message: "the image did not come back whole")
        }
        return data
    }

    /// An image in a pull request's text, by the URL it names there: fetched
    /// through the tracker, and only from the forge's own image hosts.
    public static func pullImage(_ t: StoredTracker, url: String) async throws -> Picture {
        try decode(try await call("pull_image", ["tracker": .string(t.source.name), "url": .string(url)]), as: Picture.self)
    }

    public static func editForm(_ t: StoredTracker, key: String) async throws -> EditForm {
        try decode(try await call("edit_form", ["tracker": .string(t.source.name), "key": .string(key)]), as: EditForm.self)
    }

    public static func save(_ t: StoredTracker, key: String, changes: [FieldChange]) async throws {
        _ = try await call("save", ["tracker": .string(t.source.name), "key": .string(key), "changes": json(changes)])
    }

    public static func transitions(_ t: StoredTracker, key: String) async throws -> [IssueTransition] {
        try decode(try await call("transitions", ["tracker": .string(t.source.name), "key": .string(key)]), as: [IssueTransition].self)
    }

    public static func transition(_ t: StoredTracker, key: String, id: String) async throws {
        _ = try await call("transition", ["tracker": .string(t.source.name), "key": .string(key), "id": .string(id)])
    }

    public static func comment(_ t: StoredTracker, key: String, markdown: String) async throws {
        _ = try await call("comment", ["tracker": .string(t.source.name), "key": .string(key), "markdown": .string(markdown)])
    }

    public static func editComment(_ t: StoredTracker, key: String, id: String, markdown: String) async throws {
        _ = try await call("edit_comment", ["tracker": .string(t.source.name), "key": .string(key), "id": .string(id), "markdown": .string(markdown)])
    }

    public static func deleteComment(_ t: StoredTracker, key: String, id: String) async throws {
        _ = try await call("delete_comment", ["tracker": .string(t.source.name), "key": .string(key), "id": .string(id)])
    }

    public static func users(_ t: StoredTracker, key: String, query: String, assignable: Bool) async throws -> [UserChoice] {
        try decode(try await call("users", ["tracker": .string(t.source.name), "key": .string(key), "query": .string(query),
                                            "assignable": .bool(assignable)]), as: [UserChoice].self)
    }

    public static func pull(_ t: StoredTracker, repo: String, id: String) async throws -> PullDetail {
        try decode(try await call("pull", ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id)]), as: PullDetail.self)
    }

    public static func pullFiles(_ t: StoredTracker, repo: String, id: String) async throws -> PullFiles {
        try decode(try await call("pull_files", ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id)]), as: PullFiles.self)
    }

    public static func pullFile(_ t: StoredTracker, files: PullFiles, file: PullFile) async throws -> FileDiff {
        try decode(try await call("pull_file", ["tracker": .string(t.source.name), "repo": .string(files.repo), "base": .string(files.base),
                                                "head": .string(files.head), "file": json(file)]), as: FileDiff.self)
    }

    public static func pullComment(_ t: StoredTracker, repo: String, id: String, body: String, thread: String?) async throws {
        var args: [String: JSONValue] = ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id), "body": .string(body)]
        if let thread { args["thread"] = .string(thread) }
        _ = try await call("pull_comment", args)
    }

    /// `approved`, `suggestions`, `changes-requested`, `waiting`, `rejected`
    /// or `commented`, as `offers` allows.
    public static func pullVote(_ t: StoredTracker, repo: String, id: String, verdict: String, body: String) async throws {
        _ = try await call("pull_vote", ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id), "verdict": .string(verdict),
                                         "body": .string(body)])
    }

    /// Merge it now (Azure DevOps: complete it) with one of
    /// `PullMerging.methods`. `head` is `PullMerging.head`: the forge refuses
    /// when the branch has moved since. `deleteBranch` is Azure DevOps' to
    /// act on; GitHub does as the repository says.
    public static func pullMerge(_ t: StoredTracker, repo: String, id: String, how: String, head: String?,
                                 deleteBranch: Bool) async throws {
        _ = try await call("pull_merge", ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id), "how": .string(how),
                                          "head": .string(head ?? ""), "delete_branch": .bool(deleteBranch)])
    }

    /// Set it to merge itself once it can (GitHub's auto-merge, Azure DevOps'
    /// auto-complete), or with `how` nil, take that back.
    public static func pullAutoMerge(_ t: StoredTracker, repo: String, id: String, how: String?,
                                     deleteBranch: Bool) async throws {
        _ = try await call("pull_auto_merge", ["tracker": .string(t.source.name), "repo": .string(repo), "id": .string(id),
                                               "how": how.map(JSONValue.string) ?? .null,
                                               "delete_branch": .bool(deleteBranch)])
    }
}

/// `repo #12`, as the desktop names a pull request.
public func pullName(_ p: PullRequestSummary) -> String {
    "\(p.repo.split(separator: "/").last.map(String.init) ?? p.repo) \(p.key)"
}

/// The open pull request a session is on, of the ones the inbox knows: the
/// one it was started from, or else one from its branch in its repository
/// (`sessionPull` in the desktop's Inbox.tsx).
public func sessionPull(_ session: Session, in pulls: [PullRequestSummary]) -> PullRequestSummary? {
    pulls.first { session.pull?.tracker == $0.tracker && session.pull?.repo == $0.repo && session.pull?.id == $0.id }
        ?? pulls.first { !session.workBranch.isEmpty && session.workBranch == $0.branch && bareRepo(session.repo) == bareRepo($0.repoURL) }
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
