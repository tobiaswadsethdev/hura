import Foundation

// The messages behind the workspace: projects, the create form, and what runs
// in a session beside its agent. Each mirrors a type in `hura-proto` or
// `hura-core`, whose TypeScript twins are in `apps/desktop/src/gen`.

/// A repository someone is working on, which worktrees are grouped under.
public struct Project: Codable, Equatable, Identifiable, Sendable {
    /// Unique, and what a session records to say where it belongs.
    public var name: String
    /// Clone URL. Every worktree in the project starts from this.
    public var repo: String
    /// The checkout on the server this was named from.
    public var path: String
    public var createdAt: Int64

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, repo, path
        case createdAt = "created_at"
    }

    public init(name: String, repo: String, path: String, createdAt: Int64 = 0) {
        self.name = name
        self.repo = repo
        self.path = path
        self.createdAt = createdAt
    }
}

/// A checkout on the server's disk, from a scan of its `repo_roots`.
public struct LocalRepo: Codable, Equatable, Identifiable, Sendable {
    public var path: String
    /// The path as a person would write it, `~` and all.
    public var display: String
    public var name: String
    /// `origin`'s URL. A checkout without one can't be cloned into a sandbox.
    public var origin: String?
    public var branch: String?

    public var id: String { path }
}

/// What a scan found, and where it looked.
public struct Listing: Codable, Equatable, Sendable {
    public var repos: [LocalRepo]
    public var roots: [String]
}

/// Git facts about a checkout.
public struct Facts: Codable, Equatable, Sendable {
    public var uncommitted: Int
    public var unpushed: Int?
    public var baseOnRemote: Bool
    /// The toolchains the checkout points at.
    public var toolchains: [String]

    enum CodingKeys: String, CodingKey {
        case uncommitted, unpushed, toolchains
        case baseOnRemote = "base_on_remote"
    }
}

/// What is known about the repository a client picked.
public struct Picked: Codable, Equatable, Sendable {
    public var facts: Facts
    /// The branch the checkout is on; nil on a detached HEAD.
    public var branch: String?
    /// Providers a new session here should start with ticked.
    public var providers: [String]
}

public struct PolicyChoice: Codable, Equatable, Sendable {
    public var spec: String
    public var summary: String
}

public struct ToolchainChoice: Codable, Equatable, Sendable {
    public var name: String
    public var summary: String
}

public struct ProviderChoice: Codable, Equatable, Sendable {
    public var name: String
    public var kind: String
}

/// Everything the create form needs that isn't about a repository.
public struct NewOptions: Codable, Equatable, Sendable {
    public var policies: [PolicyChoice]
    public var toolchains: [ToolchainChoice]
    public var providers: [ProviderChoice]
    /// Why `providers` is empty, when listing them failed.
    public var providersError: String?
    public var defaultPolicy: String
    public var defaultProviders: [String]
    public var defaultBase: String?
    /// Named, not chosen: the server's config decides these.
    public var skills: [String]
    public var mcp: [String]
    /// `chat` or `terminal`.
    public var defaultInterface: String

    enum CodingKeys: String, CodingKey {
        case policies, toolchains, providers, skills, mcp
        case providersError = "providers_error"
        case defaultPolicy = "default_policy"
        case defaultProviders = "default_providers"
        case defaultBase = "default_base"
        case defaultInterface = "default_interface"
    }
}

/// A session a client asks for (`NewSession` in hura-proto). Started from the
/// phone, never from a ticket or a pull request, so those stay null.
public struct NewSession: Encodable, Equatable, Sendable {
    public var project: String?
    /// nil to have the server derive one from the task.
    public var name: String?
    public var repo: String
    public var task: String
    /// nil for the remote's default branch.
    public var base: String?
    /// nil for `<branch_prefix>/<name>`.
    public var branch: String?
    public var policy: String
    public var providers: [String]
    public var toolchains: [String]
    public var start: Bool
    /// `chat` or `terminal`; nil for the server's default.
    public var interface: String?

    public init(project: String?, name: String?, repo: String, task: String, base: String?,
                branch: String? = nil, policy: String, providers: [String], toolchains: [String],
                start: Bool = true, interface: String?) {
        self.project = project
        self.name = name
        self.repo = repo
        self.task = task
        self.base = base
        self.branch = branch
        self.policy = policy
        self.providers = providers
        self.toolchains = toolchains
        self.start = start
        self.interface = interface
    }

    enum CodingKeys: String, CodingKey {
        case project, name, repo, task, base, branch, policy, providers, toolchains, start, interface
        case ticket, pull
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        // Every optional is sent as null rather than left out: hurad's serde
        // reads a missing `Option` as None anyway, and this way the body says
        // what it means.
        try c.encode(project, forKey: .project)
        try c.encode(name, forKey: .name)
        try c.encode(repo, forKey: .repo)
        try c.encode(task, forKey: .task)
        try c.encode(base, forKey: .base)
        try c.encode(branch, forKey: .branch)
        try c.encode(policy, forKey: .policy)
        try c.encode(providers, forKey: .providers)
        try c.encode(toolchains, forKey: .toolchains)
        try c.encode(start, forKey: .start)
        try c.encode(interface, forKey: .interface)
        try c.encodeNil(forKey: .ticket)
        try c.encodeNil(forKey: .pull)
    }
}

/// The answer to one permission request, question or plan (`ChatDecision`).
public struct ChatDecision: Codable, Equatable, Sendable {
    public var allow: Bool
    /// Don't ask again for calls like this one, as the SDK suggests the rule.
    public var always: Bool
    /// Said to the agent with a refusal: what to do instead.
    public var message: String?
    /// For a question: the label chosen, keyed by the question it answers.
    public var answers: [String: String]?
    /// For a plan: the permission mode to carry on in once approved.
    public var mode: String?

    public init(allow: Bool, always: Bool = false, message: String? = nil, answers: [String: String]? = nil,
                mode: String? = nil) {
        self.allow = allow
        self.always = always
        self.message = message
        self.answers = answers
        self.mode = mode
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(allow, forKey: .allow)
        try c.encode(always, forKey: .always)
        try c.encode(message, forKey: .message)
        try c.encode(answers, forKey: .answers)
        try c.encode(mode, forKey: .mode)
    }
}

/// A conversation's list, and the one just opened when that is what was asked.
public struct Chats: Decodable, Equatable, Sendable {
    public var chats: [String]
    public var opened: String?
}

/// A request body: `op` beside the fields of `body`, which is how hura-proto's
/// internally tagged `Request` reads.
struct Op<Body: Encodable>: Encodable {
    var op: String
    var body: Body

    private enum Key: String, CodingKey { case op }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Key.self)
        try c.encode(op, forKey: .op)
        try body.encode(to: encoder)
    }
}

/// A body with no fields.
struct Nothing: Encodable {
    func encode(to encoder: Encoder) throws {}
}

struct Named: Encodable {
    var name: String
}

struct ProjectsReply: Decodable {
    var projects: [Project]
}

struct ShellsReply: Decodable {
    var shells: [String]
}

struct CreatedReply: Decodable {
    var name: String
}
