import HuraKit
import UserNotifications

/// Replaces the generic text the relay sends for an encrypted notification with
/// the real title, subtitle and body. If anything goes wrong (no key for the
/// server, a key from an older link, a tampered payload), the generic text
/// is shown as it is.
final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var content: UNMutableNotificationContent?

    override func didReceive(_ request: UNNotificationRequest,
                             withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        self.contentHandler = contentHandler
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
            return contentHandler(request.content)
        }
        self.content = content
        let keys = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)
        if let text = PushCrypto.open(payload: content.userInfo, keys: keys) {
            content.title = text.title
            content.subtitle = PushCrypto.subtitle(text.subtitle, serverName: PushCrypto.serverName(of: content.userInfo,
                                                                                                    keys: keys))
            content.body = text.body
        }
        contentHandler(content)
    }

    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let content { contentHandler(content) }
    }
}
