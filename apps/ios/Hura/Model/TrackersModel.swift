import HuraKit
import Observation
import SwiftUI
import UserNotifications

/// The account's trackers, and what is yours on them.
///
/// Kept and read by hura cloud (see `TrackerCore`), so they are the same on
/// every device. Signed out, there are none, and the screens that show them
/// ask you to sign in.
@MainActor
@Observable
final class TrackersModel {
    private(set) var trackers: [StoredTracker] = []
    /// Whether this phone is signed in to hura cloud, without which there
    /// are no trackers.
    private(set) var signedIn: Bool
    /// The trackers hura cloud can connect through their own
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
    /// Signed out, the inbox's sign-in button was tapped: the app shows the
    /// Account tab.
    var showAccount = false

    @ObservationIgnored private let cloud: Cloud
    /// The relay link, which hura cloud is told about so it can say what
    /// changed: it reads the trackers all day.
    @ObservationIgnored private let push: PushManager?
    @ObservationIgnored private var loop: Task<Void, Never>?
    /// How often the inbox is read while the app is in front, as on the
    /// desktop.
    @ObservationIgnored private let every: Duration = .seconds(180)

    init(cloud: Cloud = .shared, push: PushManager? = nil) {
        self.cloud = cloud
        self.push = push
        // Read as it stands, so a signed-in phone never shows the sign-in
        // prompt while the first read is on its way.
        signedIn = cloud.stored != nil
    }

    /// The account's trackers, which another device may have changed.
    func reload() async {
        signedIn = await cloud.isSignedIn
        guard signedIn else {
            trackers = []
            inbox = nil
            return
        }
        await linkCloud()
        await syncSeen()
        connectable = (try? await cloud.connectable()) ?? connectable
        do {
            trackers = try await cloud.trackers()
        } catch {
            lastError = error.localizedDescription
            // Signed out from another device.
            if !(await cloud.isSignedIn) {
                signedIn = false
                trackers = []
                inbox = nil
            }
        }
    }

    /// Where to send the browser to connect `provider` to the account.
    func connectURL(_ provider: String) async throws -> URL {
        try await cloud.connectURL(provider)
    }

    func tracker(named name: String) -> StoredTracker? {
        trackers.first { $0.source.name == name }
    }

    // MARK: Changing the list

    /// Adds one to the account, which checks it as the desktop does: a name
    /// given when there is none, filters trimmed, and one that could not work
    /// refused.
    func add(_ source: TrackerSource, token: String?) async throws {
        let t = token?.trimmingCharacters(in: .whitespacesAndNewlines)
        let token = (t?.isEmpty ?? true) ? nil : t
        trackers = try await cloud.addTracker(source, token: token)
        await refresh()
    }

    func setToken(_ name: String, _ token: String) async throws {
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { throw TrackerError(message: "That token is empty.") }
        trackers = try await cloud.setToken(name, t)
        await refresh()
    }

    /// Its filters, or whether it hides what you created: checked by the
    /// service as when it was added, keeping its token.
    func edit(_ name: String, _ source: TrackerSource) async throws {
        trackers = try await cloud.editTracker(name, source)
        await refresh()
    }

    func forget(_ name: String) async throws {
        trackers = try await cloud.forgetTracker(name)
        inbox?.tasks.removeAll { $0.tracker == name }
        inbox?.pulls.removeAll { $0.tracker == name }
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
            inbox = try await TrackerCore.inbox(prefix: "hura")
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    // MARK: Notifications

    /// Turns tracker notifications on, asking for permission the first time,
    /// or off. hura cloud sends them, through the relay, once it is linked
    /// to this phone.
    func setNotify(_ on: Bool) async throws {
        if on {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard allowed else {
                throw TrackerError(message: "Notifications are off for hura in Settings.")
            }
        }
        notify = on
        UserDefaults.standard.set(on, forKey: "inbox.notify")
        guard signedIn else { return }
        if on {
            // The relay needs this phone's device token, which iOS hands
            // out once notifications are allowed.
            UIApplication.shared.registerForRemoteNotifications()
            await linkCloud()
        } else if let push {
            await push.disableCloud(cloud)
        }
    }

    /// Signed in with notifications on: links hura cloud to this phone, or
    /// repairs the link. Until it can (no device token yet, the relay away),
    /// nothing is announced.
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
    /// waiting is forgotten, so it counts again if it comes back. The
    /// account's other devices are told, so their badges and notifications
    /// clear too.
    func markSeen() {
        guard inbox != nil, seen != waiting else { return }
        seen = waiting
        UserDefaults.standard.set(Array(seen), forKey: "inbox.seen")
        let seen = seen
        Task { try? await cloud.setSeen(seen) }
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
