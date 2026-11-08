import BackgroundTasks
import HuraKit
import Observation
import SwiftUI
import UserNotifications

/// The trackers this phone reads, and what is yours on them.
///
/// **Signed out**, set up here and kept in this phone's Keychain, as the
/// desktop keeps its own on its machine; no server holds a tracker or a
/// token. **Signed in to hura cloud**, the account's trackers, kept there and
/// read there (see `TrackerCore`), so they are the same on every device. A
/// tracker added before signing in stays on this phone until it is moved to
/// the account.
@MainActor
@Observable
final class TrackersModel {
    private(set) var trackers: [StoredTracker] = []
    /// Whether `trackers` are the account's rather than this phone's.
    private(set) var signedIn = false
    /// While signed in, the trackers still in this phone's Keychain: not
    /// read, and offered to be moved to the account.
    private(set) var onPhone: [StoredTracker] = []
    /// Signed in, the trackers hura cloud can connect through their own
    /// sign-in rather than a pasted token: `github`, `jira`.
    private(set) var connectable: Set<String> = []
    private(set) var inbox: Inbox?
    private(set) var reading = false
    private(set) var lastError: String?
    /// What was waiting on you when you last looked at the inbox, by
    /// tracker and id; the badge counts only what has come since.
    private(set) var seen: Set<String> = Set(UserDefaults.standard.stringArray(forKey: "inbox.seen") ?? [])
    /// Notify when your tickets, filters or pull requests change, as the
    /// desktop does. Off until turned on here, since it asks for permission.
    private(set) var notify = UserDefaults.standard.bool(forKey: "inbox.notify")
    /// A notification about the inbox was tapped: the app shows the Inbox tab.
    var showInbox = false

    @ObservationIgnored private let store: TrackerStore
    @ObservationIgnored private let cloud: Cloud
    /// The relay link, which signed in, hura cloud is told about so it can
    /// say what changed: it reads the trackers all day, and the phone only
    /// while iOS lets it.
    @ObservationIgnored private let push: PushManager?
    @ObservationIgnored private var loop: Task<Void, Never>?
    /// How often the inbox is read while the app is in front, as on the
    /// desktop.
    @ObservationIgnored private let every: Duration = .seconds(180)

    init(store: TrackerStore = KeychainTrackerStore(), cloud: Cloud = .shared, push: PushManager? = nil) {
        self.store = store
        self.cloud = cloud
        self.push = push
        trackers = (try? store.load()) ?? []
    }

    /// Which trackers are read: the account's when signed in, which another
    /// device may have changed, and otherwise this phone's.
    func reload() async {
        let local = (try? store.load()) ?? []
        signedIn = await cloud.isSignedIn
        guard signedIn else {
            onPhone = []
            trackers = local
            return
        }
        onPhone = local
        await linkCloud()
        await syncSeen()
        connectable = (try? await cloud.connectable()) ?? connectable
        do {
            trackers = try await cloud.trackers()
        } catch {
            lastError = error.localizedDescription
            // Signed out from another device: back to this phone's own.
            if !(await cloud.isSignedIn) {
                signedIn = false
                onPhone = []
                trackers = local
            }
        }
    }

    /// Where to send the browser to connect `provider` to the account.
    func connectURL(_ provider: String) async throws -> URL {
        try await cloud.connectURL(provider)
    }

    /// Moves this phone's trackers, tokens and all, to the account, and
    /// forgets them here. One whose name the account already has stays on
    /// the phone, and is returned.
    ///
    /// Each is forgotten here as soon as the account has it, so a failure part
    /// way leaves only those not yet moved, to move again, and never a copy of
    /// one the account already holds.
    @discardableResult
    func moveToAccount() async throws -> [String] {
        var kept: [String] = []
        defer { onPhone = (try? store.load()) ?? onPhone }
        for t in onPhone {
            do {
                trackers = try await cloud.addTracker(t.source, token: t.token)
                try store.save(((try? store.load()) ?? []).filter { $0.source.name != t.source.name })
            } catch let e as CloudError where e.status == 409 {
                kept.append(t.source.name)
            }
        }
        await refresh()
        return kept
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
        let token = (t?.isEmpty ?? true) ? nil : t
        if signedIn {
            trackers = try await cloud.addTracker(checked, token: token)
        } else {
            try save(trackers + [StoredTracker(source: checked, token: token)])
        }
        await refresh()
    }

    func setToken(_ name: String, _ token: String) async throws {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { throw TrackerError(message: "That token is empty.") }
        if signedIn {
            trackers = try await cloud.setToken(name, t)
        } else {
            try save(trackers.map { $0.source.name == name ? StoredTracker(source: $0.source, token: t) : $0 })
        }
        await refresh()
    }

    /// Its filters, or whether it hides what you created: checked by
    /// hura-core as when it was added, and kept with its token. Signed in,
    /// the service checks it instead, which knows what a tracker connected
    /// through its own sign-in needs: a Jira one has no email, and checked
    /// here, would be refused for it.
    func edit(_ name: String, _ source: TrackerSource) async throws {
        if signedIn {
            trackers = try await cloud.editTracker(name, source)
            await refresh()
            return
        }
        let checked = try await TrackerCore.check(source)
        guard checked.name == name else {
            throw TrackerError(message: "A tracker can't be renamed; forget it and add it again.")
        }
        try save(trackers.map { $0.source.name == name ? StoredTracker(source: checked, token: $0.token) : $0 })
        await refresh()
    }

    func forget(_ name: String) async throws {
        if signedIn {
            trackers = try await cloud.forgetTracker(name)
        } else {
            try save(trackers.filter { $0.source.name != name })
        }
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
        await reload()
        guard !trackers.isEmpty else {
            inbox = Inbox()
            return
        }
        reading = true
        defer { reading = false }
        do {
            let read = try await TrackerCore.inbox(trackers, prefix: "hura")
            inbox = read
            lastError = nil
            await announce(read)
        } catch {
            lastError = error.localizedDescription
        }
    }

    // MARK: Notifications

    /// Turns tracker notifications on, asking for permission the first time,
    /// or off.
    func setNotify(_ on: Bool) async throws {
        if on {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard allowed else {
                throw TrackerError(message: "Notifications are off for hura in Settings.")
            }
        }
        notify = on
        UserDefaults.standard.set(on, forKey: "inbox.notify")
        if on { scheduleBackgroundRead() } else { BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.backgroundTask) }
        if signedIn {
            if on {
                // The relay needs this phone's device token, which iOS hands
                // out once notifications are allowed.
                UIApplication.shared.registerForRemoteNotifications()
                await linkCloud()
            } else if let push {
                await push.disableCloud(cloud)
            }
        }
    }

    /// Whether hura cloud says what changed, in place of this phone noticing
    /// for itself: signed in, with notifications on and the service linked.
    var cloudNotifies: Bool { signedIn && notify && (push?.cloudLinked ?? false) }

    /// Signed in with notifications on: links hura cloud to this phone, or
    /// repairs the link. Until it can (no device token yet, the relay away),
    /// the phone keeps noticing for itself.
    private func linkCloud() async {
        guard signedIn, notify, let push, push.device != nil else { return }
        do {
            if push.cloudLinked {
                try await push.repairCloudIfNeeded(cloud)
            } else {
                try await push.enableCloud(cloud)
            }
        } catch {
            lastError = error.localizedDescription
        }
    }

    /// Before signing out: stops hura cloud's notifications to this phone.
    func leaveCloud() async {
        await push?.disableCloud(cloud)
    }

    /// Records a read, and says what changed since the last one. The
    /// bookkeeping happens whether or not notifications are on, so turning
    /// them on doesn't announce everything that happened while they were off.
    private func announce(_ read: Inbox) async {
        let defaults = UserDefaults.standard
        // Kept under the key the snapshot had before hura-core took this
        // over: it still reads what was written there.
        let before = defaults.data(forKey: "inbox.snapshot").flatMap { try? JSONDecoder().decode(JSONValue.self, from: $0) }
        guard let (after, changes) = try? await TrackerCore.changes(read, seen: before) else { return }
        defaults.set(try? JSONEncoder().encode(after), forKey: "inbox.snapshot")
        // Signed in and linked, hura cloud tells this phone, and telling it
        // here too would say everything twice.
        guard notify, !changes.isEmpty, !cloudNotifies else { return }
        let center = UNUserNotificationCenter.current()
        // Past three at once, one says how many: usually the phone was away
        // for a weekend, and ten banners would be worse than one.
        let toasts: [(title: String, body: String, change: InboxChange?)] = changes.count > 3
            ? [("\(changes.count) changes in your tickets, filters and pull requests",
                changes.map { "\($0.key) · \($0.what)" }.joined(separator: "\n"), nil)]
            : changes.map { ("\($0.key) · \($0.what)", $0.title, $0) }
        for toast in toasts {
            let content = UNMutableNotificationContent()
            content.title = toast.title
            content.body = toast.body
            content.sound = .default
            content.threadIdentifier = "inbox"
            var info: [String: String] = [:]
            if let c = toast.change {
                info = ["tracker": c.tracker, "id": c.id]
            }
            content.userInfo = ["inbox": info]
            try? await center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
        }
    }

    // MARK: Reading in the background

    /// What the background read is registered under: listed in Info.plist's
    /// `BGTaskSchedulerPermittedIdentifiers`.
    nonisolated static var backgroundTask: String { "\(Bundle.main.bundleIdentifier ?? "hura").inbox" }

    /// Asks iOS to wake the app to read the trackers again. When is iOS's to
    /// decide (not before fifteen minutes, and often later); the app being
    /// opened reads at once regardless.
    func scheduleBackgroundRead() {
        guard notify, !trackers.isEmpty else { return }
        let request = BGAppRefreshTaskRequest(identifier: Self.backgroundTask)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    /// One background read, then the next one asked for.
    func readInBackground(_ task: BGAppRefreshTask) {
        scheduleBackgroundRead()
        let reading = Task { await refresh() }
        task.expirationHandler = { reading.cancel() }
        Task {
            await reading.value
            task.setTaskCompleted(success: !reading.isCancelled && lastError == nil)
        }
    }

    /// Waiting on you: tickets assigned to you and pull requests asking for
    /// your review.
    private var waiting: Set<String> {
        let tasks = inbox?.tasks.filter { $0.relation == "assigned" }.map { "\($0.tracker)/\($0.id)" } ?? []
        let pulls = inbox?.pulls.filter(\.reviewRequested).map { "\($0.tracker)/\($0.id)" } ?? []
        return Set(tasks + pulls)
    }

    /// What is waiting on you that you haven't seen in the inbox yet.
    var unseenCount: Int { waiting.subtracting(seen).count }

    /// The inbox is on screen: what it shows has been seen. What stops
    /// waiting is forgotten, so it counts again if it comes back.
    ///
    /// Signed in, the account's other devices are told, so their badges and
    /// notifications clear too.
    func markSeen() {
        guard inbox != nil, seen != waiting else { return }
        seen = waiting
        UserDefaults.standard.set(Array(seen), forKey: "inbox.seen")
        if signedIn {
            let seen = seen
            Task { try? await cloud.setSeen(seen) }
        }
    }

    /// What was seen on another of the account's devices: the badge follows
    /// it, and the tracker notifications this phone showed before then go.
    /// Asked at every read, and when hura cloud wakes the app to say.
    func syncSeen() async {
        guard signedIn, let (items, at) = try? await cloud.seen(), let at else { return }
        if items != seen {
            seen = items
            UserDefaults.standard.set(Array(seen), forKey: "inbox.seen")
        }
        let center = UNUserNotificationCenter.current()
        let stale = await center.deliveredNotifications()
            .filter { $0.request.content.threadIdentifier == "inbox" && $0.date <= at }
            .map(\.request.identifier)
        if !stale.isEmpty { center.removeDeliveredNotifications(withIdentifiers: stale) }
    }
}
