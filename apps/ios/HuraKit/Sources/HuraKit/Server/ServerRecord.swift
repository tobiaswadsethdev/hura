import Foundation
import Security

/// Everything needed to reach one paired server. The whole record, token
/// included, lives in the Keychain and nowhere else.
public struct ServerRecord: Codable, Equatable, Identifiable, Sendable {
    /// Chosen by this phone when pairing: an `hurad` has no machine id of its
    /// own, and the relay needs one for the link and every notification from it.
    public var serverID: String
    public var name: String
    public var host: String
    public var port: String
    /// Lowercase hex SHA-256 of the server's certificate.
    public var fingerprint: String
    public var token: String
    public var addedAt: Date
    /// This phone's relay link for the server, while notifications are on.
    public var pushLinkID: String?

    public var id: String { serverID }

    enum CodingKeys: String, CodingKey {
        case name, host, port, fingerprint, token
        case serverID = "server_id"
        case addedAt = "added_at"
        case pushLinkID = "push_link_id"
    }

    public init(serverID: String, name: String, host: String, port: String, fingerprint: String,
                token: String, addedAt: Date = Date()) {
        self.serverID = serverID
        self.name = name
        self.host = host
        self.port = port
        self.fingerprint = fingerprint
        self.token = token
        self.addedAt = addedAt
    }

    /// `h_` and 16 base32 characters, the shape the relay takes.
    public static func newServerID() -> String {
        let alphabet = Array("abcdefghijklmnopqrstuvwxyz234567")
        var rng = SystemRandomNumberGenerator()
        return "h_" + String((0..<16).map { _ in alphabet[Int(rng.next(upperBound: UInt32(32)))] })
    }
}

// Keep the token out of print(), dump(), string interpolation and the debugger summary.
extension ServerRecord: CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable {
    public var description: String { "ServerRecord(\(serverID), \(name), \(host):\(port))" }
    public var debugDescription: String { description }
    public var customMirror: Mirror {
        Mirror(self, children: ["serverID": serverID, "name": name, "host": host, "port": port,
                                "token": "<redacted>", "pushLinkID": pushLinkID as Any])
    }
}

public protocol ServerStore: Sendable {
    func loadAll() throws -> [ServerRecord]
    func save(_ record: ServerRecord) throws
    func delete(serverID: String) throws
}

/// One generic-password item per server. Readable after the first unlock (so
/// an answer from a notification can connect while the phone is locked),
/// never leaves this device, and is not synced to iCloud.
public struct KeychainServerStore: ServerStore {
    private let items: KeychainItems

    public init(service: String = "hura.servers") {
        items = KeychainItems(service: service)
    }

    public func loadAll() throws -> [ServerRecord] {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try items.all().compactMap { try? decoder.decode(ServerRecord.self, from: $0) }
            .sorted { $0.addedAt < $1.addedAt }
    }

    public func save(_ record: ServerRecord) throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        try items.set(encoder.encode(record), account: record.serverID)
    }

    public func delete(serverID: String) throws {
        try items.delete(account: serverID)
    }
}

/// For tests and previews.
public final class InMemoryServerStore: ServerStore, @unchecked Sendable {
    private let lock = NSLock()
    private var records: [String: ServerRecord] = [:]

    public init(_ records: [ServerRecord] = []) {
        for r in records { self.records[r.serverID] = r }
    }

    public func loadAll() throws -> [ServerRecord] {
        lock.withLock { records.values.sorted { $0.addedAt < $1.addedAt } }
    }

    public func save(_ record: ServerRecord) throws {
        lock.withLock { records[record.serverID] = record }
    }

    public func delete(serverID: String) throws {
        _ = lock.withLock { records.removeValue(forKey: serverID) }
    }
}
