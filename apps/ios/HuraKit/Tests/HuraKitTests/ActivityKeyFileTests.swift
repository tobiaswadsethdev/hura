import Foundation
import Testing
@testable import HuraKit

/// The copy of the push keys the Live Activity widget reads, which has no
/// Keychain on a phone.
struct ActivityKeyFileTests {
    func file() -> ActivityKeyFile {
        ActivityKeyFile(url: FileManager.default.temporaryDirectory
            .appending(path: "activity-keys-\(UUID().uuidString).json"))
    }

    @Test func holdsExactlyTheKeysAndNamesOfTheServersItIsGiven() throws {
        let keys = InMemoryPushKeyStore()
        let a = PushCrypto.newKey(), b = PushCrypto.newKey()
        try keys.save(a, serverID: "h_a")
        try keys.saveName("Mac", serverID: "h_a")
        try keys.save(b, serverID: "h_b")
        try keys.save(PushCrypto.newKey(), serverID: "h_cloud")

        let f = file()
        try f.sync(from: keys, serverIDs: ["h_a", "h_b", "h_none"])
        #expect(try f.key(serverID: "h_a") == a)
        #expect(try f.name(serverID: "h_a") == "Mac")
        #expect(try f.key(serverID: "h_b") == b)
        #expect(try f.name(serverID: "h_b") == nil)
        // Only what it was asked for: not hura cloud's, nor a server with none.
        #expect(try f.key(serverID: "h_cloud") == nil)
        #expect(try f.key(serverID: "h_none") == nil)

        // A server forgotten, or with notifications off, drops out.
        try f.sync(from: keys, serverIDs: ["h_b"])
        #expect(try f.key(serverID: "h_a") == nil)
        #expect(try f.key(serverID: "h_b") == b)
    }

    @Test func opensAnActivitysTextAsTheKeychainWould() throws {
        let keys = InMemoryPushKeyStore()
        let key = PushCrypto.newKey()
        try keys.save(key, serverID: "h_a")
        let f = file()
        try f.sync(from: keys, serverIDs: ["h_a"])

        let enc = try PushCrypto.seal(PushCrypto.Text(title: "Fix the tests", body: "Bash: npm test"), key: key,
                                      serverID: "h_a", kind: SessionActivity.kind, session: "fix-tests", askID: "t1")
        let state = SessionActivityState(phase: .permission, askID: "t1", enc: enc)
        #expect(state.text(serverID: "h_a", session: "fix-tests", keys: f)?.body == "Bash: npm test")
    }

    @Test func aMissingFileIsNoKeys() throws {
        #expect(try file().key(serverID: "h_a") == nil)
    }
}
