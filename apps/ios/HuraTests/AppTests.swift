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

/// Host patterns match as `hostMatches` in the desktop's Events.tsx.
@Test func matchesRuleHostsLikeTheDesktop() {
    #expect(hostMatches("github.com:443", "github.com:443"))
    #expect(!hostMatches("github.com:80", "github.com:443"))
    #expect(hostMatches("github.com", "github.com:443"))
    #expect(hostMatches("**", "anything.example:443"))
    #expect(hostMatches("**.github.com", "api.eu.github.com:443"))
    #expect(hostMatches("*.github.com", "api.github.com:443"))
    #expect(!hostMatches("*.github.com", "api.eu.github.com:443"))
    #expect(!hostMatches("*.github.com", "github.com:443"))
}

@Test func checksAPathBeforeAllowingIt() {
    #expect(pathProblem("/feed/v3/**") == nil)
    #expect(pathProblem("") == "")
    #expect(pathProblem("feed") != nil)
    #expect(pathProblem("/a?b") != nil)
    #expect(pathProblem("/a/../b") != nil)
    #expect(pathProblem("/a b") != nil)
    #expect(asPath("https://registry.npmjs.org/-/v1/search", host: "registry.npmjs.org") == "/-/v1/search")
    #expect(asPath("registry.npmjs.org", host: "registry.npmjs.org") == "/")
    #expect(asPath("https://other.org/x", host: "registry.npmjs.org") == "https://other.org/x")
}

@Test func widensAPathOneLevelAtATime() {
    #expect(widen("/a/b/c") == "/a/b/**")
    #expect(widen("/a/b/**") == "/a/**")
    #expect(widen("/a/**") == "/**")
    #expect(widen("/a") == "/**")
    #expect(widen("/**") == "/**")
}

/// What a watched host's row offers, and that a watch opens nothing.
@Test func aWatchedHostIsNotOpen() throws {
    let view = try JSONDecoder().decode(PolicyView.self, from: Data("""
    {"template":null,"rules":[],"watching":[{"id":"w1","host":"example.com:443"}],"lists":null}
    """.utf8))
    let s = Standing(policy: view, endpoint: "example.com:443")
    #expect(s.watch == "w1")
    #expect(!s.open && !s.whole)
    #expect(Standing(policy: view, endpoint: "other.com:443").watch == nil)
}

/// A server's name is this phone's own, kept with its pairing.
@Test @MainActor func renamesAServerAndKeepsTheName() throws {
    let store = InMemoryServerStore()
    let m = model(store)
    let server = try m.add(record())
    try m.rename(server, to: "  Laptop (WSL) ")
    #expect(server.record.name == "Laptop (WSL)")
    #expect(try store.loadAll().first?.name == "Laptop (WSL)")
    // An empty name is no name: the old one stays.
    try m.rename(server, to: "   ")
    #expect(server.record.name == "Laptop (WSL)")
    #expect(server.record.token == "tok", "the pairing is untouched")
}

/// An image on a line of its own is drawn; one inside a sentence or a fence
/// stays text.
struct MarkdownImageTests {
    @Test func anImageLineIsItsOwnSegment() {
        let s = MarkdownText.segments("Here it is:\n\n![the login page](docs/login.png)\n\nLooks right.")
        #expect(s == [.prose("Here it is:"), .image(alt: "the login page", src: "docs/login.png"), .prose("Looks right.")])
        #expect(MarkdownText.image(#"![x](<a b.png> "title")"#)! == ("x", "a b.png"))
        #expect(MarkdownText.image("see ![x](a.png) here") == nil)
        #expect(MarkdownText.segments("```\n![x](a.png)\n```") == [.code("![x](a.png)")])
    }
}

/// Your messages sit where Claude read them, as on the desktop; one not yet
/// read waits at the bottom, and one from before receipts stays where it was.
@Test func messagesArePlacedWhereClaudeReadThem() {
    func at(_ s: UInt64) -> Date { Date(timeIntervalSince1970: 1_790_000_000 + Double(s)) }
    func prompt(_ seq: UInt64, _ uuid: String, _ text: String, receipt: Bool = true) -> TranscriptEntry {
        var e = TranscriptEntry(seq: seq, uuid: uuid, kind: TranscriptEntry.Kind.prompt,
                                blocks: [TranscriptBlock(type: "text", text: text)], at: at(seq))
        e.receipt = receipt
        return e
    }
    func read(_ seq: UInt64, _ uuid: String) -> TranscriptEntry {
        TranscriptEntry(seq: seq, uuid: uuid, kind: TranscriptEntry.Kind.read, at: at(seq))
    }
    func reply(_ seq: UInt64, _ text: String) -> TranscriptEntry {
        TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.assistant, blocks: [TranscriptBlock(type: "text", text: text)],
                        at: at(seq))
    }
    func shown(_ items: [ChatItem]) -> [String] {
        items.compactMap {
            switch $0 {
            case let .prompt(_, p): ">" + p.text
            case let .text(_, t, _): t
            default: nil
            }
        }
    }

    let sentMidTurn = [prompt(1, "a", "one"), read(2, "a"), reply(3, "part 1"), prompt(4, "b", "two"), reply(5, "part 2")]
    var folded = ChatItem.fold(sentMidTurn)
    #expect(shown(folded.items) == [">one", "part 1", "part 2"])
    #expect(shown(folded.queued) == [">two"])

    folded = ChatItem.fold(sentMidTurn + [read(6, "b"), reply(7, "reply two")])
    #expect(shown(folded.items) == [">one", "part 1", "part 2", ">two", "reply two"])
    #expect(folded.queued.isEmpty)

    folded = ChatItem.fold([prompt(1, "x", "old", receipt: false), reply(2, "old reply"), prompt(3, "y", "new")])
    #expect(shown(folded.items) == [">old", "old reply"])
    #expect(shown(folded.queued) == [">new"])
}

/// Claude's words and the end of a turn say when, as your messages do, from
/// a transcript as the host sends it.
@Test func repliesAndTurnsSayWhen() {
    let raw = [
        #"{"seq":1,"at":1790000001000,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Done."}]}}}}"#,
        #"{"seq":2,"at":1790000002000,"ev":{"kind":"sdk","msg":{"type":"result","subtype":"success","duration_ms":4200}}}"#,
    ].compactMap { ChatTranscript.entry(try! JSONDecoder().decode(JSONValue.self, from: Data($0.utf8))) }
    let items = ChatItem.fold(raw).items
    #expect(items == [.text(id: "1.0", text: "Done.", at: Date(timeIntervalSince1970: 1_790_000_001)),
                      .turnEnd(id: "2.0", ms: 4200, at: Date(timeIntervalSince1970: 1_790_000_002))])
}

/// A `!` command is one card: running until its end arrives, then with what it
/// printed and how it ended. Its bubble's text is not a message's.
@Test func aShellCommandIsOneCardFilledInWhenItEnds() {
    let raw = [
        #"{"seq":1,"at":1790000001000,"ev":{"kind":"shell","uuid":"s1","command":"npm test"}}"#,
        #"{"seq":2,"at":1790000002000,"ev":{"kind":"shell","uuid":"s2","command":"sleep 99"}}"#,
        #"{"seq":3,"at":1790000003000,"ev":{"kind":"exited","uuid":"s1","output":"1 failing\n","code":1,"stopped":false,"truncated":false}}"#,
    ].compactMap { ChatTranscript.entry(try! JSONDecoder().decode(JSONValue.self, from: Data($0.utf8))) }
    let runs: [ShellRun] = ChatItem.fold(raw).items.compactMap { if case let .shell(_, run) = $0 { run } else { nil } }
    #expect(runs.map(\.command) == ["npm test", "sleep 99"])
    #expect(runs[0].output == "1 failing\n")
    #expect(runs[0].code == 1)
    #expect(runs[0].failed)
    #expect(runs[1].running)
    #expect(!runs[1].failed)
    #expect(ChatRows.bubble("/review 119").characters.count == "/review 119".count)
}

/// A subagent is listed from its task events, a resumed one stays one agent,
/// and its messages are its own thread rather than the conversation's.
@Test func subagentsAreListedAndFollowedApart() {
    func task(_ seq: UInt64, _ json: String) -> TranscriptEntry {
        let v = try! JSONDecoder().decode(JSONValue.self, from: Data(json.utf8))
        return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.task, at: Date(timeIntervalSince1970: Double(seq)), task: v)
    }
    func said(_ seq: UInt64, _ text: String, parent: String? = nil) -> TranscriptEntry {
        TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.assistant, blocks: [TranscriptBlock(type: "text", text: text)],
                        parent: parent)
    }
    let entries = [
        said(1, "starting one"),
        task(2, #"{"subtype":"task_started","task_id":"a1","tool_use_id":"toolu_1","description":"Read the docs","subagent_type":"Explore","task_type":"local_agent","is_backgrounded":true,"prompt":"Read docs/"}"#),
        task(3, #"{"subtype":"task_started","task_id":"b1","tool_use_id":"toolu_2","description":"npm test","task_type":"local_bash"}"#),
        said(4, "reading", parent: "toolu_1"),
        said(5, "back to you"),
        task(6, #"{"subtype":"task_notification","task_id":"a1","status":"completed","usage":{"total_tokens":1200,"tool_uses":3,"duration_ms":4000}}"#),
        task(7, #"{"subtype":"task_started","task_id":"a1","tool_use_id":"toolu_3","description":"Read the docs","task_type":"local_agent"}"#),
        said(8, "more reading", parent: "toolu_1"),
    ]
    let agents = AgentInfo.all(entries)
    #expect(agents.map(\.id) == ["a1"], "a shell is not an agent")
    #expect(agents.first?.call == "toolu_1", "resumed, still under the call that started it")
    #expect(agents.first?.running == true)
    #expect(agents.first?.prompt == "Read docs/")
    #expect(agents.first?.tokens == 1200)

    func texts(_ items: [ChatItem]) -> [String] {
        items.compactMap { if case let .text(_, t, _) = $0 { t } else { nil } }
    }
    #expect(texts(ChatItem.fold(entries).items) == ["starting one", "back to you"])
    #expect(texts(ChatItem.fold(entries, thread: "toolu_1").items) == ["reading", "more reading"])
}
