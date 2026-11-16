import Foundation

/// The push keys again, and what this phone calls each server, in a file the
/// Live Activity widget can read.
///
/// The widget can't use the Keychain: on a phone, drawing an activity on the
/// Lock Screen it gets `errSecNotAvailable` (-25291) for any item at all,
/// where the notification service extension reads the same group without
/// trouble. So the app keeps a copy of each server's key here, in the App
/// Group container the two share, and rewrites it from the Keychain whenever
/// a key or a name changes, and each time the app comes to the front.
///
/// The file is protected until the phone's first unlock after starting, as
/// the Keychain items are (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`),
/// and is out of backups as they are. Only servers' keys are copied: hura
/// cloud's text is shown by notifications alone.
public struct ActivityKeyFile: PushKeyStore {
    public let url: URL

    struct Entry: Codable, Equatable {
        var key: String
        var name: String?
    }

    public init(url: URL) {
        self.url = url
    }

    /// The App Group from the `HuraAppGroup` Info.plist key, which the app and
    /// the widget both carry.
    public static var appGroup: String? {
        (Bundle.main.object(forInfoDictionaryKey: "HuraAppGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    /// The file in the App Group's container; nil when the build has no group.
    public static var shared: ActivityKeyFile? {
        guard let group = appGroup,
              let dir = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
        else { return nil }
        return ActivityKeyFile(url: dir.appending(path: "activity-keys.json"))
    }

    func read() -> [String: Entry] {
        guard let data = try? Data(contentsOf: url) else { return [:] }
        return (try? JSONDecoder().decode([String: Entry].self, from: data)) ?? [:]
    }

    func write(_ entries: [String: Entry]) throws {
        guard entries != read() else { return }
        let data = try JSONEncoder().encode(entries)
        #if os(iOS)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: url, options: .atomic)
        #endif
        var u = url
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? u.setResourceValues(values)
    }

    /// Makes the file hold exactly the keys and names `keys` has for
    /// `serverIDs`: servers forgotten, or with notifications off, drop out.
    public func sync(from keys: PushKeyStore, serverIDs: [String]) throws {
        var entries: [String: Entry] = [:]
        for id in serverIDs {
            guard let key = try keys.key(serverID: id) else { continue }
            entries[id] = Entry(key: key.base64EncodedString(), name: try? keys.name(serverID: id))
        }
        try write(entries)
    }

    public func key(serverID: String) throws -> Data? {
        read()[serverID].flatMap { Data(base64Encoded: $0.key) }
    }

    public func name(serverID: String) throws -> String? {
        read()[serverID]?.name
    }

    public func save(_ key: Data, serverID: String) throws {
        var entries = read()
        entries[serverID, default: Entry(key: "")].key = key.base64EncodedString()
        try write(entries)
    }

    public func saveName(_ name: String, serverID: String) throws {
        var entries = read()
        guard entries[serverID] != nil else { return }
        entries[serverID]?.name = name
        try write(entries)
    }

    public func delete(serverID: String) throws {
        var entries = read()
        entries[serverID] = nil
        try write(entries)
    }
}
