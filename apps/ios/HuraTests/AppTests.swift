import Foundation
import HuraKit
import Testing
@testable import Hura

private func record(_ id: String = "h_aaaabbbbccccdddd", host: String = "box.lan", token: String = "tok") -> ServerRecord {
    ServerRecord(serverID: id, name: "studio", host: host, port: "17671",
                 fingerprint: String(repeating: "ab", count: 32), token: token)
}

@MainActor
private func model(_ store: ServerStore = InMemoryServerStore()) -> AppModel {
    AppModel(store: store, push: PushManager(relayURL: nil, store: InMemoryPushDeviceStore(), keys: InMemoryPushKeyStore()))
}

/// Runs in the app on the simulator, where the Keychain is available (unlike
/// `swift test` on the Mac).
@Test func keychainStoresAndReplacesServers() throws {
    let store = KeychainServerStore(service: "hura.tests.\(UUID().uuidString)")
    defer { try? store.delete(serverID: "h_aaaabbbbccccdddd") }
    try store.save(record())
    try store.save(record(token: "newer"))
    let all = try store.loadAll()
    #expect(all.count == 1)
    #expect(all.first?.token == "newer")
    try store.delete(serverID: "h_aaaabbbbccccdddd")
    #expect(try store.loadAll().isEmpty)
}

@Test func tokensStayOutOfUserDefaults() throws {
    let store = KeychainServerStore(service: "hura.tests.\(UUID().uuidString)")
    defer { try? store.delete(serverID: "h_aaaabbbbccccdddd") }
    try store.save(record(token: "canary_token_value"))
    #expect(!UserDefaults.standard.dictionaryRepresentation().description.contains("canary_token_value"))
}

@Test func keychainStoresThePushDevice() throws {
    let store = KeychainPushDeviceStore(service: "hura.tests.\(UUID().uuidString)")
    defer { try? store.delete() }
    #expect(try store.load() == nil)
    let device = PushDevice(deviceID: "d_x", deviceSecret: "hud_s", apnsToken: "ab", environment: .sandbox)
    try store.save(device)
    #expect(try store.load() == device)
}

@Test func aPermissionNotificationNamesItsSessionAndRequest() throws {
    let target = try #require(NotificationTarget(userInfo: [
        "hura": ["kind": "claude_permission", "server_id": "h_aaaabbbbccccdddd",
                 "session": "fix-tests", "ask_id": "toolu_01abc"],
    ]))
    #expect(target.session == "fix-tests")
    #expect(target.askID == "toolu_01abc")
    #expect(target.threadID == "h_aaaabbbbccccdddd:fix-tests")
    // Carried into a follow-up notification, so tapping that still opens it.
    #expect(NotificationTarget(userInfo: target.userInfo) == target)
    #expect(NotificationTarget(userInfo: ["hedera": ["server_id": "x"]]) == nil)
}

@MainActor
@Test func pairingTheSameServerAgainKeepsItsID() throws {
    let store = InMemoryServerStore()
    let m = model(store)
    var first = record()
    first.pushLinkID = "l_1"
    try m.add(first)
    // A new pairing string comes with a new random server ID; the old one is
    // kept, because the relay's link and the phone's key are under it.
    try m.add(record("h_zzzzzzzzzzzzzzzz", token: "new"))
    #expect(m.servers.count == 1)
    #expect(m.servers.first?.id == "h_aaaabbbbccccdddd")
    #expect(m.servers.first?.record.token == "new")
    #expect(m.servers.first?.record.pushLinkID == "l_1")
    #expect(try store.loadAll().map(\.serverID) == ["h_aaaabbbbccccdddd"])

    try m.add(record("h_yyyyyyyyyyyyyyyy", host: "other.lan"))
    #expect(m.servers.count == 2, "another address is another server")
}

@MainActor
@Test func aNotificationOpensItsSession() throws {
    let m = model()
    try m.add(record())
    m.open(try #require(NotificationTarget(userInfo: ["hura": ["server_id": "h_aaaabbbbccccdddd", "session": "fix-tests"]])))
    #expect(m.pendingRoute == [.server("h_aaaabbbbccccdddd"), .session(serverID: "h_aaaabbbbccccdddd", name: "fix-tests")])
    // A test notification has no session, and opens the server.
    m.open(try #require(NotificationTarget(userInfo: ["hura": ["server_id": "h_aaaabbbbccccdddd"]])))
    #expect(m.pendingRoute == [.server("h_aaaabbbbccccdddd")])
}

/// The model picker says what the desktop window says.
@Test(arguments: [
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5[1m]", "Sonnet 5.5 (1M)"),
    ("claude-haiku-5-5-20260301", "Haiku 5.5"),
    ("claude-fable-5", "Fable 5"),
    ("opus", "opus"),
])
func namesModelsAsAPersonWould(id: String, name: String) {
    #expect(modelName(id) == name)
}

@Test func labelsTabsByWhatTheyAre() {
    #expect(WorkTab.chat("agent").label == "Agent" && WorkTab.chat("agent").isAgent)
    #expect(WorkTab.terminal(nil).label == "Agent" && WorkTab.terminal(nil).isAgent)
    #expect(WorkTab.chat("2").label == "2" && !WorkTab.chat("2").isAgent)
    #expect(WorkTab.terminal("shell-1").label == "shell-1")
}
