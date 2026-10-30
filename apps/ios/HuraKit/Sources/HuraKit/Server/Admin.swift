import Foundation

// What a server holds on its sessions' behalf, and its editable defaults:
// the desktop window's integrations and settings screens.

/// One MCP server as the integrations screen shows it.
public struct McpStatus: Decodable, Equatable, Identifiable, Sendable {
    public var name: String
    /// The URL the agent is given.
    public var url: String
    /// Whether `hurad` runs it in a container of its own.
    public var managed: Bool
    public var image: String?
    /// `running`, `detached`, `crashing`, `stopped`, `absent` or `unknown`.
    public var state: String
    public var container: String?
    /// The secrets it asks for, and whether each is stored.
    public var secrets: [NamedSecret]
    /// What is wrong, written for a person.
    public var problem: String?
    /// The container's last words, when it isn't running.
    public var log: String?

    public var id: String { name }
}

/// A secret's name, and whether there is a value behind it. A value is never
/// read back.
public struct NamedSecret: Decodable, Equatable, Identifiable, Sendable {
    public var name: String
    public var set: Bool
    /// The MCP servers whose entry names it.
    public var usedBy: [String]

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, set
        case usedBy = "used_by"
    }
}

/// A skill in the server's library.
public struct StoredSkill: Decodable, Equatable, Identifiable, Sendable {
    public var name: String
    /// Where it came from on the machine that uploaded it.
    public var origin: String
    public var uploadedAt: Int64

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, origin
        case uploadedAt = "uploaded_at"
    }
}

public struct Integrations: Decodable, Equatable, Sendable {
    public var mcp: [McpStatus]
    public var secrets: [NamedSecret]
    public var skills: [StoredSkill]
    /// Skills the server's config file names, which aren't in the library.
    public var configuredSkills: [String]

    enum CodingKeys: String, CodingKey {
        case mcp, secrets, skills
        case configuredSkills = "configured_skills"
    }
}

/// The editable defaults in the server's config file. nil is the built-in
/// default, and is written as a missing key.
public struct ServerSettings: Codable, Equatable, Sendable {
    public var branchPrefix: String?
    public var base: String?
    public var policy: String?
    /// nil lets each repository guess; a list replaces the guess.
    public var providers: [String]?
    public var autoUpdate: Bool?
    /// `chat` or `terminal`.
    public var interface: String?

    public init(branchPrefix: String? = nil, base: String? = nil, policy: String? = nil, providers: [String]? = nil,
                autoUpdate: Bool? = nil, interface: String? = nil) {
        self.branchPrefix = branchPrefix
        self.base = base
        self.policy = policy
        self.providers = providers
        self.autoUpdate = autoUpdate
        self.interface = interface
    }

    enum CodingKeys: String, CodingKey {
        case base, policy, providers, interface
        case branchPrefix = "branch_prefix"
        case autoUpdate = "auto_update"
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(branchPrefix, forKey: .branchPrefix)
        try c.encode(base, forKey: .base)
        try c.encode(policy, forKey: .policy)
        try c.encode(providers, forKey: .providers)
        try c.encode(autoUpdate, forKey: .autoUpdate)
        try c.encode(interface, forKey: .interface)
    }
}

public struct SettingsView: Decodable, Equatable, Sendable {
    /// The config file, on the server.
    public var path: String
    /// Whether that file exists yet.
    public var present: Bool
    public var settings: ServerSettings
    public var defaultBranchPrefix: String

    enum CodingKeys: String, CodingKey {
        case path, present, settings
        case defaultBranchPrefix = "default_branch_prefix"
    }
}

extension ServerClient {
    public func integrations() async throws -> Integrations {
        try await call("integrations", Nothing(), as: Integrations.self)
    }

    /// Starts, restarts or stops a managed MCP server: `start`, `restart` or
    /// `stop`.
    public func mcp(_ name: String, action: String) async throws -> Integrations {
        struct Body: Encodable { var name: String; var action: String }
        return try await call("mcp", Body(name: name, action: action), as: Integrations.self)
    }

    /// Stores a secret, or forgets it when `value` is nil.
    public func secret(_ name: String, value: String?) async throws -> Integrations {
        struct Body: Encodable {
            var name: String
            var value: String?
            enum CodingKeys: String, CodingKey { case name, value }
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(name, forKey: .name)
                try c.encode(value, forKey: .value)
            }
        }
        return try await call("secret", Body(name: name, value: value), as: Integrations.self)
    }

    /// Drops a skill from the server's library.
    public func forgetSkill(_ name: String) async throws -> Integrations {
        try await call("forget-skill", Named(name: name), as: Integrations.self)
    }

    public func settings() async throws -> SettingsView {
        try await call("settings", Nothing(), as: SettingsView.self)
    }

    /// Writes the defaults back; answers with the file re-read.
    public func setSettings(_ settings: ServerSettings) async throws -> SettingsView {
        try await call("set-settings", settings, as: SettingsView.self)
    }
}
