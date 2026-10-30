// End-to-end encrypted notification text (hura's `docs/relay-api.md`,
// "Encrypted text"). The app makes a key for each push link and gives it to
// `hurad`; the notification service extension decrypts with it.

import CryptoKit
import Foundation
import Security

public enum PushCrypto {
    public struct Text: Codable, Equatable, Sendable {
        public var title: String
        public var subtitle: String?
        public var body: String
    }

    public enum Failure: Error, Equatable {
        case malformed
        case authentication
    }

    /// A new random 32-byte key.
    public static func newKey() -> Data {
        SymmetricKey(size: .bits256).withUnsafeBytes { Data($0) }
    }

    /// The additional data that binds a ciphertext to its notification:
    /// its server, kind, session and request, absent ones empty. So the relay
    /// can't move one notification's text onto another's, or point an Allow
    /// at a different request from the one whose text is shown.
    static func additionalData(serverID: String, kind: String, session: String?, askID: String?) -> Data {
        Data("hura-push-v1\n\(serverID)\n\(kind)\n\(session ?? "")\n\(askID ?? "")".utf8)
    }

    /// Decrypts the `enc` field of a notification's `hura` payload.
    public static func open(_ encrypted: String, key: Data, serverID: String, kind: String,
                            session: String?, askID: String?) throws -> Text {
        guard let raw = Data(base64URL: encrypted), raw.count >= 12 + 16, key.count == 32 else { throw Failure.malformed }
        let box: AES.GCM.SealedBox
        do {
            box = try AES.GCM.SealedBox(combined: raw)
        } catch {
            throw Failure.malformed
        }
        let plain: Data
        do {
            plain = try AES.GCM.open(box, using: SymmetricKey(data: key),
                                     authenticating: additionalData(serverID: serverID, kind: kind, session: session, askID: askID))
        } catch {
            throw Failure.authentication
        }
        guard let text = try? JSONDecoder().decode(Text.self, from: plain) else { throw Failure.malformed }
        return text
    }

    /// Opens a notification's `hura` payload with the key stored for its
    /// server. Nil when it isn't encrypted or can't be opened.
    public static func open(payload: [AnyHashable: Any], keys: PushKeyStore) -> Text? {
        guard let h = payload["hura"] as? [String: Any],
              let enc = h["enc"] as? String,
              let serverID = h["server_id"] as? String,
              let kind = h["kind"] as? String,
              let key = try? keys.key(serverID: serverID)
        else { return nil }
        return try? open(enc, key: key, serverID: serverID, kind: kind,
                         session: h["session"] as? String, askID: h["ask_id"] as? String)
    }
}

extension Data {
    init?(base64URL s: String) {
        var b = s.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        b += String(repeating: "=", count: (4 - b.count % 4) % 4)
        self.init(base64Encoded: b)
    }

    var base64URL: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// Push keys, one per server. The notification service extension reads
/// them, so they live in a Keychain group shared with it, apart from the
/// server records: the extension can decrypt notifications but never sees
/// a pairing token.
public protocol PushKeyStore: Sendable {
    func key(serverID: String) throws -> Data?
    func save(_ key: Data, serverID: String) throws
    func delete(serverID: String) throws
}

public struct KeychainPushKeyStore: PushKeyStore {
    private let items: KeychainItems

    /// `accessGroup` is the group shared with the extension; nil uses the
    /// app's own (tests).
    public init(service: String = "hura.pushkeys", accessGroup: String?) {
        items = KeychainItems(service: service, accessGroup: accessGroup)
    }

    /// The shared group from the `HuraKeychainGroup` Info.plist key, which
    /// the app and the extension both carry.
    public static var sharedGroup: String? {
        (Bundle.main.object(forInfoDictionaryKey: "HuraKeychainGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    public func key(serverID: String) throws -> Data? { try items.get(account: serverID) }
    public func save(_ key: Data, serverID: String) throws { try items.set(key, account: serverID) }
    public func delete(serverID: String) throws { try items.delete(account: serverID) }
}

/// For tests and previews.
public final class InMemoryPushKeyStore: PushKeyStore, @unchecked Sendable {
    private let lock = NSLock()
    private var keys: [String: Data] = [:]

    public init() {}

    public func key(serverID: String) throws -> Data? { lock.withLock { keys[serverID] } }
    public func save(_ key: Data, serverID: String) throws { lock.withLock { keys[serverID] = key } }
    public func delete(serverID: String) throws { _ = lock.withLock { keys.removeValue(forKey: serverID) } }
}
