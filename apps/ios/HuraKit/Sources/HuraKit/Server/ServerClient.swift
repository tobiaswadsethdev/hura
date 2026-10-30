import CryptoKit
import Foundation

/// What went wrong talking to an `hurad`.
public enum ServerError: Error, LocalizedError, Equatable {
    /// The token was refused: revoked, or from before the server was reset.
    case unauthorized
    /// The server answered with a failure of its own.
    case failed(kind: String, message: String)
    case http(Int)
    case malformed(String)

    public var errorDescription: String? {
        switch self {
        case .unauthorized: "This server no longer accepts this phone. Pair it again with `hurad pair`."
        case let .failed(_, message): message
        case let .http(status): "The server answered with HTTP \(status)."
        case let .malformed(why): "The server's answer couldn't be read: \(why)"
        }
    }
}

/// One request to an `hurad`: `POST /rpc` over TLS pinned to the server's
/// certificate, with the pairing token as a bearer token
/// (`docs/server.md`). Stateless, so it can be used from a notification action
/// with nothing else of the app's running.
public struct ServerClient: Sendable {
    public var record: ServerRecord

    public init(record: ServerRecord) {
        self.record = record
    }

    /// `GET /version`, which needs no token.
    public func hello() async throws -> Hello {
        let r = try await PinnedHTTP.request(host: record.host, port: record.port, pin: record.fingerprint,
                                             method: "GET", path: "/version")
        guard r.status == 200 else { throw ServerError.http(r.status) }
        do {
            return try JSONDecoder().decode(Hello.self, from: r.body)
        } catch {
            throw ServerError.malformed(error.localizedDescription)
        }
    }

    public func sessions() async throws -> [Session] {
        try await call(["op": .string("ls")], as: Sessions.self).sessions
    }

    public func poll(_ name: String) async throws -> Poll {
        try await call(["op": .string("poll"), "name": .string(name)], as: Poll.self)
    }

    /// Answers the permission request `id` in the session `name`. Refused
    /// once the request has been answered anywhere.
    public func answer(session name: String, ask id: String, allow: Bool) async throws {
        try await answer(session: name, ask: id, decision: ChatDecision(allow: allow))
    }

    // MARK: Projects and worktrees

    public func projects() async throws -> [Project] {
        try await call("projects", Nothing(), as: ProjectsReply.self).projects
    }

    /// Makes a project of a checkout on the server; answers with every project.
    public func newProject(path: String, repo: String, name: String?) async throws -> [Project] {
        struct Body: Encodable {
            var path: String
            var repo: String
            var name: String?
        }
        return try await call("new-project", Body(path: path, repo: repo, name: name), as: ProjectsReply.self).projects
    }

    /// Forgets a project. Its worktrees are left alone.
    public func forgetProject(_ name: String) async throws -> [Project] {
        try await call("forget-project", Named(name: name), as: ProjectsReply.self).projects
    }

    /// The checkouts on the server's disk.
    public func repos() async throws -> Listing {
        try await call("repos", Nothing(), as: Listing.self)
    }

    /// Git facts about a checkout, measured against `branch` (nil: its own).
    public func inspect(path: String, branch: String?) async throws -> Picked {
        struct Body: Encodable {
            var path: String
            var branch: String?
        }
        return try await call("inspect", Body(path: path, branch: branch), as: Picked.self)
    }

    public func newOptions() async throws -> NewOptions {
        try await call("new-options", Nothing(), as: NewOptions.self)
    }

    /// Starts a session; answers with the name it got.
    public func create(_ session: NewSession) async throws -> String {
        try await call("create", session, as: CreatedReply.self).name
    }

    /// Ends a session and deletes its sandbox; answers with what is left.
    public func destroy(_ name: String) async throws -> [Session] {
        try await call("destroy", Named(name: name), as: Sessions.self).sessions
    }

    // MARK: Beside the agent

    /// A chat session's conversations.
    public func chats(_ name: String) async throws -> Chats {
        try await call("chats", Named(name: name), as: Chats.self)
    }

    /// Opens another conversation; `opened` names it.
    public func newChat(_ name: String) async throws -> Chats {
        try await call("new-chat", Named(name: name), as: Chats.self)
    }

    public func closeChat(_ name: String, conversation: String) async throws -> Chats {
        struct Body: Encodable {
            var name: String
            var conv: String
        }
        return try await call("close-chat", Body(name: name, conv: conversation), as: Chats.self)
    }

    /// The shells running in a session's sandbox, by tmux session name.
    public func shells(_ name: String) async throws -> [String] {
        try await call("shells", Named(name: name), as: ShellsReply.self).shells
    }

    public func newShell(_ name: String) async throws -> [String] {
        try await call("new-shell", Named(name: name), as: ShellsReply.self).shells
    }

    public func killShell(_ name: String, tmux: String) async throws -> [String] {
        struct Body: Encodable {
            var name: String
            var tmux: String
        }
        return try await call("kill-shell", Body(name: name, tmux: tmux), as: ShellsReply.self).shells
    }

    /// Answers a permission request, question or plan in full.
    public func answer(session name: String, ask id: String, decision: ChatDecision) async throws {
        struct Body: Encodable {
            var name: String
            var id: String
            var decision: ChatDecision
        }
        _ = try await call("answer", Body(name: name, id: id, decision: decision), as: Told.self)
    }

    public func configurePush(relayURL: String, pushToken: String, pushKey: String?,
                              prefs: [String: Bool]?) async throws -> PushStatus {
        var body: [String: RequestJSON] = [
            "op": .string("push-configure"),
            "relay_url": .string(relayURL),
            "push_token": .string(pushToken),
            "server_id": .string(record.serverID),
        ]
        if let pushKey { body["push_key"] = .string(pushKey) }
        if let prefs { body["prefs"] = .object(prefs.mapValues(RequestJSON.bool)) }
        return try await call(body, as: PushView.self).status
    }

    public func pushStatus() async throws -> PushStatus {
        try await call(["op": .string("push-status")], as: PushView.self).status
    }

    public func removePush() async throws {
        _ = try await call(["op": .string("push-remove")], as: PushView.self)
    }

    /// What `hurad` answers: `{"ok": {"reply": …}}` or `{"error": {"kind", "message"}}`.
    private struct Envelope<T: Decodable>: Decodable {
        var ok: T?
        var error: Failure?
        struct Failure: Decodable {
            var kind: String
            var message: String
        }
    }

    func call<T: Decodable>(_ request: [String: RequestJSON], as: T.Type) async throws -> T {
        try await post(JSONEncoder().encode(request), as: T.self)
    }

    /// `op` with the fields of `body` beside it.
    func call<B: Encodable, T: Decodable>(_ op: String, _ body: B, as: T.Type) async throws -> T {
        try await post(JSONEncoder().encode(Op(op: op, body: body)), as: T.self)
    }

    private func post<T: Decodable>(_ body: Data, as: T.Type) async throws -> T {
        let r = try await PinnedHTTP.request(
            host: record.host, port: record.port, pin: record.fingerprint, method: "POST", path: "/rpc",
            headers: [("Authorization", "Bearer \(record.token)"), ("Content-Type", "application/json")],
            body: body, timeout: .seconds(30))
        if r.status == 401 { throw ServerError.unauthorized }
        guard r.status == 200 else { throw ServerError.http(r.status) }
        let envelope: Envelope<T>
        do {
            envelope = try JSONDecoder().decode(Envelope<T>.self, from: r.body)
        } catch {
            throw ServerError.malformed(error.localizedDescription)
        }
        if let f = envelope.error { throw ServerError.failed(kind: f.kind, message: f.message) }
        guard let ok = envelope.ok else { throw ServerError.malformed("neither ok nor error") }
        return ok
    }
}

/// A JSON value, for request bodies.
enum RequestJSON: Encodable, Sendable {
    case string(String)
    case number(Int)
    case bool(Bool)
    case object([String: RequestJSON])

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case let .string(s): try c.encode(s)
        case let .number(n): try c.encode(n)
        case let .bool(b): try c.encode(b)
        case let .object(o): try c.encode(o)
        }
    }
}

public struct Hello: Decodable, Equatable, Sendable {
    public var server: String
    public var `protocol`: Int
    public var version: String
}

/// A session as `hurad` lists it; only the fields the app shows.
public struct Session: Decodable, Equatable, Identifiable, Sendable {
    public var name: String
    /// `creating`, `seeding`, `ready`, `running`, `waiting`, `idle`, `failed`
    /// or `dead`.
    public var state: String
    public var workBranch: String
    public var task: String
    /// `terminal` or `chat`. Only a chat session's requests can be answered
    /// from here.
    public var interface: String
    /// The project it belongs to; nil for one made outside a project.
    public var project: String?
    /// Clone URL.
    public var repo: String
    public var baseBranch: String?
    /// Epoch seconds.
    public var createdAt: Int64
    /// Why it failed, in the words of whatever refused it.
    public var failure: String?
    public var toolchains: [String]
    public var providers: [String]
    public var policy: String?

    public var id: String { name }
    public var isChat: Bool { interface == "chat" }
    public var isLive: Bool { !["creating", "seeding", "failed", "dead"].contains(state) }

    enum CodingKeys: String, CodingKey {
        case name, state, task, interface, project, repo, failure, toolchains, providers, policy
        case workBranch = "work_branch"
        case baseBranch = "base_branch"
        case createdAt = "created_at"
    }

    public init(name: String, state: String, workBranch: String = "", task: String = "", interface: String = "terminal",
                project: String? = nil, repo: String = "", baseBranch: String? = nil, createdAt: Int64 = 0,
                failure: String? = nil, toolchains: [String] = [], providers: [String] = [], policy: String? = nil) {
        self.name = name
        self.state = state
        self.workBranch = workBranch
        self.task = task
        self.interface = interface
        self.project = project
        self.repo = repo
        self.baseBranch = baseBranch
        self.createdAt = createdAt
        self.failure = failure
        self.toolchains = toolchains
        self.providers = providers
        self.policy = policy
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        state = try c.decode(String.self, forKey: .state)
        workBranch = try c.decodeIfPresent(String.self, forKey: .workBranch) ?? ""
        task = try c.decodeIfPresent(String.self, forKey: .task) ?? ""
        interface = try c.decodeIfPresent(String.self, forKey: .interface) ?? "terminal"
        project = try c.decodeIfPresent(String.self, forKey: .project)
        repo = try c.decodeIfPresent(String.self, forKey: .repo) ?? ""
        baseBranch = try c.decodeIfPresent(String.self, forKey: .baseBranch)
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        failure = try c.decodeIfPresent(String.self, forKey: .failure)
        toolchains = try c.decodeIfPresent([String].self, forKey: .toolchains) ?? []
        providers = try c.decodeIfPresent([String].self, forKey: .providers) ?? []
        policy = try c.decodeIfPresent(String.self, forKey: .policy)
    }
}

struct Sessions: Decodable {
    var sessions: [Session]
}

/// What the agent is doing, from a session's poll.
public struct AgentReport: Decodable, Equatable, Sendable {
    /// `running`, `waiting` or `idle`.
    public var state: String
    /// The tool, or Claude Code's own sentence, while it waits.
    public var detail: String?
    /// The id of the request it waits on: chat sessions only.
    public var ask: String?
    /// What that request asks, in one line: `Bash: npm test`.
    public var prompt: String?

    public init(state: String, detail: String? = nil, ask: String? = nil, prompt: String? = nil) {
        self.state = state
        self.detail = detail
        self.ask = ask
        self.prompt = prompt
    }
}

public struct Poll: Decodable, Equatable, Sendable {
    public var status: AgentReport?
}

struct Told: Decodable {
    var message: String
}

/// `PushView`, as `hurad` answers every push request.
struct PushView: Decodable {
    var configured: Bool
    var label: String
    var prefs: [String: Bool]
    var encrypted: Bool
    var broken: String?
    var lastError: String?
    var queued: Int

    enum CodingKeys: String, CodingKey {
        case configured, label, prefs, encrypted, broken, queued
        case lastError = "last_error"
    }

    var status: PushStatus {
        PushStatus(configured: configured, label: label.isEmpty ? nil : label, prefs: prefs, broken: broken,
                   lastError: lastError, encrypted: encrypted, queued: queued)
    }
}
