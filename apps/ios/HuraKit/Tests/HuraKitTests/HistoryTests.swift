import Foundation
import Testing
@testable import HuraKit

private func json(_ s: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: Data(s.utf8))
}

private func prompt(_ seq: Int, _ text: String, at: Int? = nil) -> JSONValue {
    json(#"{"seq":\#(seq),"at":\#(at ?? 1000 + seq),"ev":{"kind":"prompt","text":"\#(text)","uuid":"u\#(seq)","files":[]}}"#)
}

private func scratch() -> ChatHistory {
    ChatHistory(directory: FileManager.default.temporaryDirectory.appending(path: "hura-history-\(UUID().uuidString)"))
}

private let agent = ChatHistory.Conversation(serverID: "h_1", session: "s", conv: "agent")

private func found(_ h: ChatHistory, _ q: String) async -> [UInt64] {
    await h.search(q).hits.map(\.seq)
}

/// The same cases as `history.rs`'s, so the phone and the desktop answer a
/// search alike.
struct HistoryTests {
    @Test func wordsAreAllLookedForWhateverTheirCase() async {
        let h = scratch()
        h.keep(agent, [prompt(1, "Make the Login test pass"), prompt(2, "the login page")])
        #expect(await found(h, "login TEST") == [1])
        #expect(await found(h, "\"login page\"") == [2])
        #expect(await found(h, "login").count == 2)
        #expect(await found(h, "").isEmpty)
    }

    @Test func aToolCallIsFoundByItsToolAndItsFile() async {
        let h = scratch()
        let edit = json(#"{"seq":2,"at":2,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Fixing the token refresh."},{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/sandbox/repo/src/auth.rs"}}]}}}}"#)
        h.keep(agent, [prompt(1, "fix auth"), edit])
        #expect(await found(h, "file:auth.rs") == [2])
        #expect(await found(h, "tool:edit token") == [2])
        #expect(await found(h, "tool:bash").isEmpty)
        #expect(await found(h, "auth.rs") == [2])
        let hit = await h.search("file:auth.rs").hits[0]
        #expect(hit.tools == ["Edit"])
        #expect(hit.role == .claude)
    }

    @Test func aToolsResultIsNotSearchedAndALongOneIsCut() async throws {
        let h = scratch()
        let long = String(repeating: "x", count: ChatHistory.resultChars * 3)
        let result = json(#"{"seq":2,"at":2,"ev":{"kind":"sdk","msg":{"type":"user","tool_use_result":{"a":1},"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"fn secret_function() {} \#(long)"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}}}}"#)
        h.keep(agent, [prompt(1, "read it"), result])
        #expect(await found(h, "secret_function").isEmpty)
        await h.flush()
        let text = try String(contentsOf: h.file(agent), encoding: .utf8)
        #expect(text.count < ChatHistory.resultChars * 2)
        #expect(!text.contains("AAAA"))
        #expect(!text.contains("tool_use_result"))
    }

    @Test func entriesAlreadyKeptAreNotKeptTwice() async {
        let h = scratch()
        h.keep(agent, [prompt(1, "one"), prompt(2, "two")])
        h.replay(agent, since: 1, seq: 3, entries: [prompt(2, "two"), prompt(3, "three")])
        #expect(await h.last(agent) == 3)
        #expect(await h.transcript(serverID: "h_1", sessionKey: "s", convKey: "agent").count == 3)
        #expect(await found(h, "two").count == 1)
    }

    @Test func aHostThatStartedAgainLeavesTheOldTranscriptEnded() async {
        let h = scratch()
        h.keep(agent, [prompt(1, "old one"), prompt(2, "old two")])
        // The same again, from the start: nothing put away.
        h.replay(agent, since: 0, seq: 2, entries: [prompt(1, "old one"), prompt(2, "old two")])
        #expect(await h.search("old").hits.allSatisfy { !$0.ended })
        // Entry 1 again, but not the one kept: another transcript.
        h.replay(agent, since: 0, seq: 1, entries: [prompt(1, "new one", at: 9_999)])
        let hits = await h.search("one").hits
        #expect(hits.count == 2)
        let old = hits.first { $0.snippet == "old one" }
        #expect(old?.ended == true)
        #expect(await h.transcript(serverID: "h_1", sessionKey: "s", convKey: old?.convKey ?? "").count == 2)
        // Asked from 5, and the host's last is 1: a pane's way of telling.
        h.replay(agent, since: 5, seq: 1, entries: [])
        #expect(await h.last(agent) == 0)
    }

    @Test func aDeletedSessionIsStillFoundAndItsNameStartsAgain() async {
        let h = scratch()
        let c = ChatHistory.Conversation(serverID: "h_1", session: "fix-login", conv: "agent")
        h.keep(c, [prompt(1, "make login work")])
        h.name(serverID: "h_1", session: "fix-login", title: "Fix the login", project: "web")
        h.removed(serverID: "h_1", session: "fix-login")
        h.keep(c, [prompt(1, "another login")])
        let hits = await h.search("login").hits
        #expect(hits.count == 2)
        let gone = hits.first(where: \.removed)
        #expect(gone?.session == "fix-login")
        #expect(gone?.title == "Fix the login")
        #expect(gone?.project == "web")
        #expect(hits.first { !$0.removed }?.snippet == "another login")
    }

    @Test func anEndedConversationIsStillFound() async {
        let h = scratch()
        let c = ChatHistory.Conversation(serverID: "h_1", session: "s", conv: "chat-2")
        h.keep(c, [prompt(1, "a side question")])
        h.ended(c)
        let hit = await h.search("side").hits.first
        #expect(hit?.conv == "chat-2")
        #expect(hit?.ended == true)
        #expect(await h.last(c) == 0)
    }

    @Test func aShellCommandAndASubagentSayWhoTheyWere() async {
        let h = scratch()
        let sub = json(#"{"seq":2,"at":2,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":"t9","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo test -p hura-core","description":"Run the core tests"}}]}}}}"#)
        let shell = json(#"{"seq":3,"at":3,"ev":{"kind":"shell","uuid":"x","command":"git log"}}"#)
        h.keep(agent, [sub, shell])
        let cargo = await h.search("cargo").hits
        #expect(cargo.first?.role == .agent)
        #expect(cargo.first?.snippet == "Bash: cargo test -p hura-core · Run the core tests")
        #expect(await h.search("core tests").hits.count == 1)
        #expect(await h.search("tool:shell").hits.first?.role == .shell)
    }

    @Test func newestFirstAndBounded() async {
        let h = scratch()
        h.keep(agent, (1...(ChatHistory.limit + 5)).map { prompt($0, "same words") })
        let f = await h.search("same")
        #expect(f.more)
        #expect(f.hits.count == ChatHistory.limit)
        #expect(f.hits.first?.seq == UInt64(ChatHistory.limit + 5))
    }

    @Test func namesAreMadeSafeAsTheDesktopMakesThem() {
        #expect(ChatHistory.safe("../etc") == "%2e.%2fetc")
        #expect(ChatHistory.unsafe(ChatHistory.safe("my server/x@y")) == "my server/x@y")
        #expect(ChatHistory.safeKey("..%2f..@123") == "%2e.%2f..@123")
        #expect(ChatHistory.safeKey("a@../b") == "a%40..%2fb")
    }

    @Test func aSnippetIsCutAroundTheWord() {
        let long = String(repeating: "a ", count: 100) + "needle " + String(repeating: "b ", count: 100)
        let s = ChatHistory.snippet(long, words: ["needle"])
        #expect(s.hasPrefix("…") && s.hasSuffix("…"))
        #expect(s.contains("needle"))
        #expect(ChatHistory.snippet("short\n\nline", words: []) == "short line")
    }

    @Test func aQueryIsWordsAndQualifiers() {
        let q = ChatHistory.Query(#"  Token file:Auth.rs tool:Edit "two words" file: "#)
        #expect(q.words == ["token", "two words", "file:"])
        #expect(q.files == ["auth.rs"])
        #expect(q.tools == ["edit"])
    }

    /// A catch-up asks each conversation from what was kept, keeps what the
    /// host sends, and is done with each when its state comes.
    @MainActor
    @Test func aCatchUpAsksFromWhatWasKept() async {
        let h = scratch()
        h.keep(ChatHistory.Conversation(serverID: ChatTests.record.serverID, session: "s", conv: "agent"), [prompt(1, "one")])
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(
            #"{"is":"opened","id":1}"#,
            #"{"is":"chat","id":1,"frame":{"t":"replay","seq":2,"entries":[{"seq":2,"at":1002,"ev":{"kind":"prompt","text":"two","uuid":"b"}}]}}"#,
            #"{"is":"chat","id":2,"frame":{"t":"replay","seq":1,"entries":[{"seq":1,"at":5,"ev":{"kind":"prompt","text":"side","uuid":"c"}}]}}"#,
            #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle"}}}"#,
            #"{"is":"chat","id":2,"frame":{"t":"notice","text":"cloning"}}"#)
        await h.catchUp(record: ChatTests.record, session: "s", conversations: ["agent", "chat-1"],
                        transport: { _ in socket })
        #expect(socket.sent.map { $0["channel"]?["since"] } == [.number(1), .number(0)])
        #expect(await found(h, "two") == [2])
        #expect(await found(h, "side") == [1])
    }

    /// The chat screen's own channel keeps what it is sent.
    @MainActor
    @Test func aChatKeepsWhatItIsSent() async throws {
        let h = scratch()
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":1,"entries":[{"seq":1,"at":0,"ev":{"kind":"prompt","text":"kept here","uuid":"a"}}]}}"#,
                   ChatTests.entry(2, "and this"))
        let chat = Chat(record: ChatTests.record, session: "fix-tests", transport: { _ in socket }, history: h)
        chat.start()
        defer { chat.stop() }
        for _ in 0..<200 where chat.entries.count < 2 { try await Task.sleep(for: .milliseconds(20)) }
        #expect(chat.entries.count == 2)
        await h.flush()
        let hits = await h.search("kept").hits
        #expect(hits.first?.serverID == ChatTests.record.serverID)
        #expect(hits.first?.session == "fix-tests")
        #expect(await found(h, "this") == [2])
    }
}
