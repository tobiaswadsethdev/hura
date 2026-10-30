import Foundation
import HuraKit
import Observation
import UIKit
import UserNotifications

/// The paired servers, one `Server` each, and notifications from them.
@MainActor
@Observable
final class AppModel {
    private(set) var servers: [Server] = []
    var loadError: String?
    let push: PushManager
    /// Why notifications can't be set up, when they can't.
    private(set) var pushError: String?
    /// The session on screen, whose notifications aren't shown.
    var visibleSession: (serverID: String, session: String)?
    /// Where tapping a notification asked to go; the server list consumes it.
    var pendingRoute: [Route]?

    @ObservationIgnored private let store: ServerStore
    @ObservationIgnored private var active = false
    /// Servers to link again, because the relay forgot this phone.
    @ObservationIgnored private var needsRelink: Set<String> = []
    @ObservationIgnored private var tokenWaiters: [CheckedContinuation<Void, Never>] = []

    /// From `HURA_RELAY_HOST` in Config/Hura.xcconfig; nil disables notifications.
    static let relayURL: URL? = (Bundle.main.object(forInfoDictionaryKey: "HuraRelayHost") as? String)
        .flatMap { $0.isEmpty ? nil : URL(string: "https://\($0)") }

    static var apnsEnvironment: APNsEnvironment {
        #if DEBUG
        .sandbox
        #else
        .production
        #endif
    }

    init(store: ServerStore = KeychainServerStore(), push: PushManager = PushManager(relayURL: AppModel.relayURL)) {
        self.store = store
        self.push = push
        do {
            servers = try store.loadAll().map { Server(record: $0) }
        } catch {
            loadError = error.localizedDescription
        }
    }

    func server(_ id: String) -> Server? {
        servers.first { $0.id == id }
    }

    /// Starts asking every server; called when the app becomes active.
    func resumeAll() {
        active = true
        for s in servers {
            s.resume()
            Task { await repairPush(s) }
        }
    }

    /// Stops asking; called when the app goes to the background.
    func suspendAll() {
        active = false
        for s in servers { s.suspend() }
    }

    /// Stores a new server. Pairing the same address again replaces it, and
    /// keeps its server ID, so the relay's link and the phone's key still fit.
    @discardableResult
    func add(_ record: ServerRecord) throws -> Server {
        var record = record
        if let existing = servers.first(where: { $0.record.host == record.host && $0.record.port == record.port }) {
            record.serverID = existing.record.serverID
            record.pushLinkID = existing.record.pushLinkID
            existing.suspend()
            servers.removeAll { $0 === existing }
        }
        try store.save(record)
        let server = Server(record: record)
        servers.append(server)
        if active { server.resume() }
        return server
    }

    /// Forgets a server, turning its notifications off first. The token stays
    /// valid on the server until `hurad revoke` there: a client has no way to
    /// revoke its own.
    func remove(_ server: Server) async throws {
        if server.record.pushLinkID != nil {
            await push.disable(server)
        }
        server.suspend()
        try store.delete(serverID: server.id)
        servers.removeAll { $0 === server }
    }

    private func save(_ server: Server, pushLinkID: String?) throws {
        var r = server.record
        r.pushLinkID = pushLinkID
        try store.save(r)
        server.update(record: r)
    }

    // MARK: Notifications

    /// At launch: if notifications are allowed, register with APNs so the
    /// token (and the relay's copy of it) stays current.
    func registerForPushIfAllowed() async {
        guard push.isAvailable else { return }
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        if [.authorized, .provisional, .ephemeral].contains(settings.authorizationStatus) {
            UIApplication.shared.registerForRemoteNotifications()
        }
    }

    func deviceTokenChanged(_ token: Data) async {
        do {
            if try await push.deviceTokenChanged(token, environment: Self.apnsEnvironment) {
                // A new registration has no links; link again what had notifications on.
                needsRelink = Set(servers.filter { $0.record.pushLinkID != nil }.map(\.id))
                for s in servers { await repairPush(s) }
            }
            pushError = nil
        } catch {
            pushError = error.localizedDescription
        }
        resumeTokenWaiters()
    }

    func deviceTokenFailed(_ error: Error) {
        pushError = error.localizedDescription
        resumeTokenWaiters()
    }

    private func resumeTokenWaiters() {
        let waiters = tokenWaiters
        tokenWaiters = []
        for w in waiters { w.resume() }
    }

    /// Turns notifications from `server` on (asking for permission the first
    /// time) or off. `prefs` switches individual kinds off.
    func setNotifications(_ on: Bool, for server: Server, prefs: [String: Bool]? = nil) async throws {
        guard on else {
            await push.disable(server)
            try save(server, pushLinkID: nil)
            return
        }
        if push.device == nil {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard allowed else { throw PushError.notRegistered }
            await withCheckedContinuation { c in
                tokenWaiters.append(c)
                UIApplication.shared.registerForRemoteNotifications()
                // iOS doesn't promise an answer (no network, say); don't wait forever.
                Task {
                    try? await Task.sleep(for: .seconds(20))
                    self.resumeTokenWaiters()
                }
            }
            guard push.device != nil else {
                throw NotificationSetupError(message: pushError ?? "Couldn't register with Apple's push service. Try again later.")
            }
        }
        let linkID = try await push.enable(server, prefs: prefs)
        try save(server, pushLinkID: linkID)
    }

    /// Links a server again if the relay forgot this phone, or if `hurad`
    /// lost the route or had its token rejected.
    private func repairPush(_ server: Server) async {
        guard server.record.pushLinkID != nil, push.device != nil else { return }
        do {
            if needsRelink.remove(server.id) != nil {
                let prefs = try? await server.pushStatus().prefs
                try save(server, pushLinkID: try await push.enable(server, prefs: prefs))
            } else if let linkID = try await push.repairIfNeeded(server) {
                try save(server, pushLinkID: linkID)
            }
        } catch {
            pushError = error.localizedDescription
        }
    }

    /// Tapping a notification opens its session, or its server.
    func open(_ target: NotificationTarget) {
        guard server(target.serverID) != nil else { return }
        var route: [Route] = [.server(target.serverID)]
        if let session = target.session { route.append(.session(serverID: target.serverID, name: session)) }
        pendingRoute = route
    }

    /// Allow or Deny on a permission notification: answer the request it
    /// named, and that request only. Runs in the background with a few
    /// seconds to spare, so on any failure it posts a notification saying so.
    func answer(_ allow: Bool, to target: NotificationTarget) async {
        let failed = allow ? "Not allowed" : "Not declined"
        guard let server = server(target.serverID), let session = target.session, let ask = target.askID else {
            return await notifyNotSent(target, title: failed, why: "That server is no longer paired with this phone.")
        }
        do {
            try await server.answer(session: session, ask: ask, allow: allow)
        } catch let ServerError.failed(_, message) {
            await notifyNotSent(target, title: failed, why: message)
        } catch let e as ServerError {
            await notifyNotSent(target, title: failed, why: e.localizedDescription)
        } catch {
            await notifyNotSent(target, title: failed, why: "Couldn't reach \(server.record.name). Is Tailscale connected?")
        }
    }

    private func notifyNotSent(_ target: NotificationTarget, title: String, why: String) async {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = "\(why) Tap to open the session."
        content.threadIdentifier = target.threadID
        content.userInfo = target.userInfo
        content.sound = .default
        try? await UNUserNotificationCenter.current()
            .add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }
}

struct NotificationSetupError: Error, LocalizedError {
    var message: String
    var errorDescription: String? { message }
}
