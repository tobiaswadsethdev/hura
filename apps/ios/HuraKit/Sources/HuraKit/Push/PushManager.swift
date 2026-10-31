import Foundation
import Observation

public enum PushError: Error, Equatable, LocalizedError {
    /// The app was built without a relay host.
    case unavailable
    /// iOS hasn't handed out a device token yet, usually because
    /// notifications aren't allowed.
    case notRegistered

    public var errorDescription: String? {
        switch self {
        case .unavailable: "This build of hura has no notification relay configured."
        case .notRegistered: "Allow notifications for hura in Settings first."
        }
    }
}

/// A server's push route, as `hurad` reports it (`PushView` in hura's
/// `hura-proto`).
public struct PushStatus: Equatable, Sendable {
    public var configured: Bool
    public var label: String?
    public var prefs: [String: Bool]
    /// Why the route has stopped, when the relay rejected its token.
    public var broken: String?
    public var lastError: String?
    /// Notification text is encrypted end to end.
    public var encrypted: Bool
    public var queued: Int
}

/// Registers this phone with the relay and links servers to it
/// (`docs/relay-api.md`). Each server with notifications on has one link; the
/// link's push token goes to `hurad` with `push-configure` and is not kept
/// here. Link IDs live in the server records, which the caller saves.
@MainActor
@Observable
public final class PushManager {
    public private(set) var device: PushDevice?
    public var isAvailable: Bool { relay != nil }

    @ObservationIgnored private let relay: RelayClient?
    @ObservationIgnored private let store: PushDeviceStore
    @ObservationIgnored private let keys: PushKeyStore

    public init(relayURL: URL?, store: PushDeviceStore = KeychainPushDeviceStore(),
                keys: PushKeyStore = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup),
                session: URLSession? = nil) {
        relay = relayURL.map { RelayClient(baseURL: $0, session: session) }
        self.store = store
        self.keys = keys
        device = try? store.load()
    }

    /// Called with each token iOS hands out, at every launch. Updating also
    /// re-enables a device the relay had disabled after APNs refused an old
    /// token. Returns true when the device had to be registered anew, which
    /// drops every link: the caller should enable its servers again.
    @discardableResult
    public func deviceTokenChanged(_ token: Data, environment: APNsEnvironment) async throws -> Bool {
        guard let relay else { throw PushError.unavailable }
        let hex = token.map { String(format: "%02x", $0) }.joined()
        if var d = device {
            d.apnsToken = hex
            d.environment = environment
            do {
                try await relay.updateDevice(d)
                try store.save(d)
                device = d
                return false
            } catch let e as RelayError where e.isGone {
                // Deleted on the relay; register again below.
            }
        }
        let d = try await relay.registerDevice(apnsToken: hex, environment: environment)
        try store.save(d)
        device = d
        return true
    }

    /// Links `server` to this phone with a fresh key and hands `hurad` the
    /// push token, replacing any earlier link. Returns the new link ID.
    public func enable(_ server: Server, prefs: [String: Bool]? = nil) async throws -> String {
        guard let relay else { throw PushError.unavailable }
        guard let device else { throw PushError.notRegistered }
        if let old = server.record.pushLinkID {
            try? await relay.unlink(old, device: device)
        }
        let link = try await relay.link(serverID: server.id, label: server.record.name, device: device)
        let key = PushCrypto.newKey()
        do {
            try await server.configurePush(relayURL: relay.baseURL.absoluteString, pushToken: link.pushToken,
                                           pushKey: key.base64URL, prefs: prefs)
        } catch {
            // The server kept its old route and key; so does the phone.
            try? await relay.unlink(link.linkID, device: device)
            throw error
        }
        try keys.save(key, serverID: server.id)
        try? keys.saveName(server.record.name, serverID: server.id)
        return link.linkID
    }

    /// Tells the notification extension what this phone calls `server`, which
    /// it puts before what the server says: the server knows only its own
    /// host name.
    public func remember(_ server: Server) {
        try? keys.saveName(server.record.name, serverID: server.id)
    }

    /// After `server` is renamed on this phone: the extension's copy, and the
    /// relay's label, which titles a notification that could not be decrypted.
    public func renamed(_ server: Server) async throws {
        remember(server)
        guard let relay, let device, let id = server.record.pushLinkID else { return }
        try await relay.rename(id, label: server.record.name, device: device)
    }

    /// Stops notifications from `server`. Both halves are best effort: a
    /// server without the route has no token, and a deleted link's token is
    /// refused by the relay.
    public func disable(_ server: Server) async {
        try? await server.removePush()
        try? keys.delete(serverID: server.id)
        if let id = server.record.pushLinkID, let relay, let device {
            try? await relay.unlink(id, device: device)
        }
    }

    /// For a server that should notify: links it again if `hurad` lost the
    /// route, the relay rejected the token, or the phone lost the key. Returns
    /// the new link ID, or nil when nothing needed doing.
    public func repairIfNeeded(_ server: Server) async throws -> String? {
        guard server.record.pushLinkID != nil, device != nil else { return nil }
        let status = try await server.pushStatus()
        let keyed = status.encrypted && (try? keys.key(serverID: server.id)) != nil
        if status.configured, status.broken == nil, keyed { return nil }
        return try await enable(server, prefs: status.prefs)
    }

    /// Asks the relay to send this phone a test notification.
    public func sendTest() async throws -> RelayClient.TestResult {
        guard let relay else { throw PushError.unavailable }
        guard let device else { throw PushError.notRegistered }
        return try await relay.sendTest(device: device)
    }

    /// Unregisters this phone, which removes every link.
    public func forgetDevice() async {
        if let device, let relay { try? await relay.deleteDevice(device) }
        try? store.delete()
        device = nil
    }
}
