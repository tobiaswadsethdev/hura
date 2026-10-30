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

/// Owns the model so notification callbacks, which can arrive before any view
/// exists (an answer launches the app in the background), have it at hand.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let model = AppModel()

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        NotificationCategories.register()
        Task { await model.registerForPushIfAllowed() }
        return true
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
