import Foundation
import Security

public struct KeychainError: Error, LocalizedError {
    public var status: OSStatus
    public var errorDescription: String? {
        "Keychain error \(status): \(SecCopyErrorMessageString(status, nil) as String? ?? "unknown")"
    }
}

/// Generic-password items under one service, keyed by account.
struct KeychainItems {
    let service: String
    /// A Keychain access group to keep the items in; nil is the app's own.
    var accessGroup: String?

    private var base: [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrSynchronizable as String: false,
        ]
        if let accessGroup { q[kSecAttrAccessGroup as String] = accessGroup }
        #if os(macOS)
        // Tests on the Mac use the same keychain as the app on iOS.
        q[kSecUseDataProtectionKeychain as String] = true
        #endif
        return q
    }

    func all() throws -> [Data] {
        var q = base
        q[kSecMatchLimit as String] = kSecMatchLimitAll
        q[kSecReturnData as String] = true
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let items = out as? [Data] else { throw KeychainError(status: status) }
        return items
    }

    func get(account: String) throws -> Data? {
        var q = base
        q[kSecAttrAccount as String] = account
        q[kSecReturnData as String] = true
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = out as? Data else { throw KeychainError(status: status) }
        return data
    }

    func set(_ data: Data, account: String) throws {
        var q = base
        q[kSecAttrAccount as String] = account
        let update: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        var status = SecItemUpdate(q as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            q.merge(update) { $1 }
            status = SecItemAdd(q as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    func delete(account: String) throws {
        var q = base
        q[kSecAttrAccount as String] = account
        let status = SecItemDelete(q as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError(status: status) }
    }
}
