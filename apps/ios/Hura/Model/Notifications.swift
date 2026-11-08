import BackgroundTasks
import HuraKit
import UIKit
import UserNotifications

/// The `hura` part of a notification's payload (`docs/relay-api.md`).
struct NotificationTarget: Equatable, Sendable {
    var serverID: String
    /// The session's name; absent for a test notification.
    var session: String?
    /// The permission request a `claude_permission` notification answers.
    var askID: String?

    init?(userInfo: [AnyHashable: Any]) {
        guard let h = userInfo["hura"] as? [String: Any], let serverID = h["server_id"] as? String else { return nil }
        self.serverID = serverID
        session = h["session"] as? String
        askID = h["ask_id"] as? String
    }

    var threadID: String { session.map { "\(serverID):\($0)" } ?? serverID }

    var userInfo: [String: Any] {
        var h: [String: Any] = ["server_id": serverID]
        if let session { h["session"] = session }
        if let askID { h["ask_id"] = askID }
        return ["hura": h]
    }
}

enum NotificationCategories {
    static let allowAction = "ALLOW"
    static let denyAction = "DENY"

    /// The relay picks the category from the kind. A chat session's
    /// permission request is answered from the notification: Allow runs
    /// something on the server, so it needs the phone unlocked; Deny only
    /// stops something, so it doesn't.
    static func register() {
        let allow = UNNotificationAction(identifier: allowAction, title: "Allow", options: [.authenticationRequired])
        let deny = UNNotificationAction(identifier: denyAction, title: "Deny", options: [.destructive])
        let categories: Set<UNNotificationCategory> = [
            UNNotificationCategory(identifier: "CLAUDE_PERMISSION", actions: [allow, deny], intentIdentifiers: []),
            UNNotificationCategory(identifier: "CLAUDE_INPUT", actions: [], intentIdentifiers: []),
            UNNotificationCategory(identifier: "CLAUDE_TURN", actions: [], intentIdentifiers: []),
            UNNotificationCategory(identifier: "CLAUDE_FAILED", actions: [], intentIdentifiers: []),
            UNNotificationCategory(identifier: "TEST", actions: [], intentIdentifiers: []),
        ]
        UNUserNotificationCenter.current().setNotificationCategories(categories)
    }
}

/// Owns the models so notification callbacks, which can arrive before any
/// view exists (an answer launches the app in the background), have them at
/// hand, and so does a background read of the trackers.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let model: AppModel
    let trackers: TrackersModel

    override init() {
        let model = AppModel()
        self.model = model
        // Hands hura cloud's link to the relay to the trackers, which turn it
        // on and off with their notifications.
        trackers = TrackersModel(push: model.push)
        super.init()
    }

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        NotificationCategories.register()
        // Registered before launch finishes, as BGTaskScheduler requires.
        let trackers = trackers
        BGTaskScheduler.shared.register(forTaskWithIdentifier: TrackersModel.backgroundTask, using: .main) { task in
            guard let task = task as? BGAppRefreshTask else { return task.setTaskCompleted(success: false) }
            MainActor.assumeIsolated { trackers.readInBackground(task) }
        }
        Task { await model.registerForPushIfAllowed() }
        return true
    }

    /// A background push: hura cloud saying the inbox was looked at on
    /// another device. Anything else is not one this answers.
    func application(_ application: UIApplication,
                     didReceiveRemoteNotification userInfo: [AnyHashable: Any]) async -> UIBackgroundFetchResult {
        guard (userInfo["hura"] as? [String: Any])?["kind"] as? String == "inbox_seen" else { return .noData }
        await trackers.syncSeen()
        return .newData
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { await model.deviceTokenChanged(deviceToken) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        model.deviceTokenFailed(error)
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification)
        async -> UNNotificationPresentationOptions {
        let target = NotificationTarget(userInfo: notification.request.content.userInfo)
        return await MainActor.run {
            if let target, let visible = model.visibleSession,
               visible.serverID == target.serverID, visible.session == target.session {
                return []
            }
            return [.banner, .list, .sound]
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        // A change on a tracker, which the app noticed itself or hura cloud
        // told it (marked by the extension, or not when it couldn't run): to
        // the inbox.
        let info = response.notification.request.content.userInfo
        if info["inbox"] != nil || (info["hura"] as? [String: Any])?["kind"] as? String == "tracker_change" {
            await MainActor.run { trackers.showInbox = true }
            return
        }
        guard let target = NotificationTarget(userInfo: response.notification.request.content.userInfo) else { return }
        let action = response.actionIdentifier
        // iOS keeps the app running until this returns, so the answer is awaited.
        if action == NotificationCategories.allowAction || action == NotificationCategories.denyAction {
            await model.answer(action == NotificationCategories.allowAction, to: target)
        } else if action == UNNotificationDefaultActionIdentifier {
            await model.open(target)
        }
    }
}
