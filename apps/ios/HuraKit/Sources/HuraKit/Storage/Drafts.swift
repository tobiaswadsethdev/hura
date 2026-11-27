// What is being written and not yet sent, kept on the phone rather than the
// server, as the desktop keeps it in the window (`chat/drafts.ts`,
// `chat/stash.ts`): a draft for each conversation, and the prompts put aside.

import Foundation
import Observation

/// What was being written in each conversation and not yet sent, so leaving
/// the worktree, or the app being closed, leaves a half-written message where
/// it was. Only the text: a photo is easy to attach again.
///
/// Under one key in the defaults. A draft not touched for a month is dropped
/// the next time any draft is written, and there are never more than `limit`,
/// so a conversation that ended with its draft unsent does not keep it
/// forever.
public struct Drafts {
    public static let storeKey = "hura.drafts"
    static let limit = 100
    static let month: TimeInterval = 30 * 24 * 60 * 60

    private struct Draft: Codable {
        var text: String
        var at: Date
    }

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// A conversation's key: the server, the session and the conversation,
    /// since a session's name is only unique on its server.
    public static func key(server: String, session: String, conv: String) -> String {
        "\(server)\n\(session)\n\(conv)"
    }

    /// The draft kept for a conversation, or an empty string.
    public func read(_ key: String) -> String {
        all()[key]?.text ?? ""
    }

    /// Keeps the text as the conversation's draft; an empty one forgets it.
    public func write(_ key: String, _ text: String, now: Date = .now) {
        var all = all()
        if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            all.removeValue(forKey: key)
        } else {
            all[key] = Draft(text: text, at: now)
        }
        let kept = all.filter { now.timeIntervalSince($0.value.at) < Self.month }
            .sorted { $0.value.at > $1.value.at }
            .prefix(Self.limit)
        let data = try? JSONEncoder().encode(Dictionary(uniqueKeysWithValues: kept.map { ($0.key, $0.value) }))
        defaults.set(data, forKey: Self.storeKey)
    }

    private func all() -> [String: Draft] {
        guard let data = defaults.data(forKey: Self.storeKey) else { return [:] }
        return (try? JSONDecoder().decode([String: Draft].self, from: data)) ?? [:]
    }
}

/// A prompt put aside to come back to.
public struct Stashed: Codable, Equatable, Identifiable, Sendable {
    public var id: UUID
    public var text: String
    public var at: Date

    public init(id: UUID = UUID(), text: String, at: Date = .now) {
        self.id = id
        self.text = text
        self.at = at
    }
}

/// Prompts put aside, T3 Code's stash: what is written goes here and the box
/// empties for the next thing, and the stash over the box gives it back.
///
/// One for the app rather than one per conversation, as T3 Code's is: a
/// prompt is often put aside to be sent somewhere else. The newest first, at
/// most `limit`.
@MainActor
@Observable
public final class PromptStash {
    public static let shared = PromptStash()
    public static let storeKey = "hura.stash"
    static let limit = 100

    public private(set) var all: [Stashed]
    @ObservationIgnored private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        all = defaults.data(forKey: Self.storeKey).flatMap { try? JSONDecoder().decode([Stashed].self, from: $0) } ?? []
    }

    /// Puts the text aside; nothing for an empty one.
    public func put(_ text: String) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        save([Stashed(text: text)] + all)
    }

    /// Takes one out, to use it or to be rid of it.
    public func take(_ id: UUID) {
        save(all.filter { $0.id != id })
    }

    private func save(_ next: [Stashed]) {
        all = Array(next.prefix(Self.limit))
        defaults.set(try? JSONEncoder().encode(all), forKey: Self.storeKey)
    }
}

/// A project's command, its dev server or its tests, to run in a new shell.
public struct ProjectScript: Codable, Equatable, Hashable, Identifiable, Sendable {
    public var id: UUID
    public var name: String
    public var command: String

    public init(id: UUID = UUID(), name: String, command: String) {
        self.id = id
        self.name = name
        self.command = command
    }
}

/// Each project's scripts, kept on the phone per server and project, as the
/// desktop keeps them in the window (`scripts.ts`): they are how this person
/// works in it, not something the server needs to know.
public struct ProjectScripts {
    static let prefix = "hura.scripts"
    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    static func key(server: String, project: String) -> String {
        "\(prefix)\n\(server)\n\(project)"
    }

    public func read(server: String, project: String) -> [ProjectScript] {
        guard let data = defaults.data(forKey: Self.key(server: server, project: project)) else { return [] }
        return (try? JSONDecoder().decode([ProjectScript].self, from: data)) ?? []
    }

    /// Keeps the scripts, without any that has no command to run.
    public func write(_ scripts: [ProjectScript], server: String, project: String) {
        let kept = scripts.filter { !$0.command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        let key = Self.key(server: server, project: project)
        if kept.isEmpty {
            defaults.removeObject(forKey: key)
        } else {
            defaults.set(try? JSONEncoder().encode(kept), forKey: key)
        }
    }
}
