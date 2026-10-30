import HuraKit
import Observation
import SwiftUI

/// The trackers this phone reads, and what is yours on them. Set up here and
/// kept in this phone's Keychain, as the desktop keeps its own on its
/// machine; no server holds a tracker or a token.
@MainActor
@Observable
final class TrackersModel {
    private(set) var trackers: [StoredTracker] = []
    private(set) var inbox: Inbox?
    private(set) var reading = false
    private(set) var lastError: String?

    @ObservationIgnored private let store: TrackerStore
    @ObservationIgnored private var loop: Task<Void, Never>?
    /// How often the inbox is read while the app is in front, as on the
    /// desktop.
    @ObservationIgnored private let every: Duration = .seconds(180)

    init(store: TrackerStore = KeychainTrackerStore()) {
        self.store = store
        trackers = (try? store.load()) ?? []
    }

    func tracker(named name: String) -> StoredTracker? {
        trackers.first { $0.source.name == name }
    }

    // MARK: Changing the list

    /// Checks the tracker with hura-core, as the desktop does before it keeps
    /// one, and stores it with its token.
    func add(_ source: TrackerSource, token: String?) async throws {
        let checked = try await TrackerCore.check(source)
        guard tracker(named: checked.name) == nil else {
            throw TrackerError(message: "There is already a tracker called “\(checked.name)”.")
        }
        let t = token?.trimmingCharacters(in: .whitespacesAndNewlines)
        try save(trackers + [StoredTracker(source: checked, token: (t?.isEmpty ?? true) ? nil : t)])
        await refresh()
    }

    func setToken(_ name: String, _ token: String) async throws {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { throw TrackerError(message: "That token is empty.") }
        try save(trackers.map { $0.source.name == name ? StoredTracker(source: $0.source, token: t) : $0 })
        await refresh()
    }

    func forget(_ name: String) throws {
        try save(trackers.filter { $0.source.name != name })
        inbox?.tasks.removeAll { $0.tracker == name }
        inbox?.pulls.removeAll { $0.tracker == name }
    }

    private func save(_ list: [StoredTracker]) throws {
        try store.save(list)
        trackers = list
    }

    // MARK: Reading

    func resume() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.refresh()
                try? await Task.sleep(for: self.every)
            }
        }
    }

    func suspend() {
        loop?.cancel()
        loop = nil
    }

    /// Every tracker, under the built-in branch prefix: a ticket's branch is
    /// worked out again for the server a worktree starts on.
    func refresh() async {
        guard !trackers.isEmpty else {
            inbox = Inbox()
            return
        }
        reading = true
        defer { reading = false }
        do {
            inbox = try await TrackerCore.inbox(trackers, prefix: "hura")
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    /// Waiting on you: tickets assigned to you and pull requests asking for
    /// your review.
    var waitingCount: Int {
        (inbox?.tasks.filter { $0.relation == "assigned" }.count ?? 0)
            + (inbox?.pulls.filter(\.reviewRequested).count ?? 0)
    }
}
