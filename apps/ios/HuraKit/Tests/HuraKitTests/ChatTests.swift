import Foundation
import Testing
@testable import HuraKit

private func json(_ s: String) -> JSONValue {
    try! JSONDecoder().decode(JSONValue.self, from: Data(s.utf8))
}

struct ChatTranscriptTests {
    @Test func translatesWhatTheHostWrites() throws {
        let raw = [
            #"{"seq":1,"at":0,"ev":{"kind":"prompt","text":"fix the tests","uuid":"u"}}"#,
            #"{"seq":2,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Running them."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"npm test"}}]}}}}"#,
            #"{"seq":3,"at":0,"ev":{"kind":"sdk","msg":{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"1 failing"}],"is_error":true}]}}}}"#,
            #"{"seq":4,"at":0,"ev":{"kind":"sdk","msg":{"type":"result","subtype":"success","duration_ms":4200}}}"#,
        ].map(json)
        let entries = raw.compactMap(ChatTranscript.entry)
        #expect(entries.map(\.kind) == ["prompt", "assistant", "tool_result", "turn_end"])
        #expect(entries[1].blocks?.map(\.type) == ["thinking", "text", "tool_use"])
        #expect(entries[1].blocks?[2].input?["command"]?.string == "npm test")
        #expect(entries[2].blocks?.first?.text == "1 failing")
        #expect(entries[2].blocks?.first?.isError == true)
        #expect(entries[3].durationMS == 4200)
    }

    @Test func leavesOutSubagentsAndAllows() {
        let sub = json(#"{"seq":5,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":"toolu_9","message":{"content":[{"type":"text","text":"inner"}]}}}}"#)
        #expect(ChatTranscript.entry(sub) == nil)
        let allowed = json(#"{"seq":6,"at":0,"ev":{"kind":"answer","id":"t","tool":"Bash","allow":true,"always":false,"message":null}}"#)
        #expect(ChatTranscript.entry(allowed) == nil)
        let declined = json(#"{"seq":7,"at":0,"ev":{"kind":"answer","id":"t","tool":"Bash","allow":false,"always":false,"message":"use make"}}"#)
        #expect(ChatTranscript.entry(declined)?.blocks?.first?.text == "Declined Bash: use make")
    }

    @Test func aFailedMessageIsAnError() {
        let e = json(#"{"seq":8,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":null,"error":"authentication_failed","message":{"content":[{"type":"text","text":"Not logged in"}]}}}}"#)
        let entry = ChatTranscript.entry(e)
        #expect(entry?.kind == "notice" && entry?.level == "error")
        #expect(entry?.blocks?.first?.text == "Not logged in")
    }

    @Test func readsTheReplyAsItIsWritten() {
        let delta = json(#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#)
        #expect(ChatTranscript.streamedText(delta) == "Hel")
        let other = json(#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start"}}"#)
        #expect(ChatTranscript.streamedText(other) == nil)
    }

    @Test func describesAnAskLikeTheHost() throws {
        let ask = try #require(Ask(json(#"{"id":"t","tool":"Bash","input":{"command":"npm test"},"title":null}"#)))
        #expect(ask.summary == "Bash: npm test")
        #expect(!ask.isQuestion)
        #expect(Ask(json(#"{"id":"q","tool":"AskUserQuestion","input":{}}"#))?.isQuestion == true)
    }
}

/// A websocket that plays a script: what the server says, and when it hangs up.
@MainActor
final class ScriptedSocket: WebSocketTransport {
    var incoming: [String] = []
    var sent: [JSONValue] = []
    /// Throw once the script runs out, as a dropped connection does.
    var dropWhenDone = true
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func say(_ frames: String...) {
        incoming += frames
        let w = waiters
        waiters = []
        w.forEach { $0.resume() }
    }

    func send(_ message: WSMessage) async throws {
        if case let .text(d) = message { sent.append(try JSONDecoder().decode(JSONValue.self, from: d)) }
    }

    func receive() async throws -> WSMessage {
        while incoming.isEmpty {
            if dropWhenDone { throw TransportError.closed(1006) }
            await withCheckedContinuation { waiters.append($0) }
        }
        return .text(Data(incoming.removeFirst().utf8))
    }

    func sendPing() async throws {}
    func close(code: Int) {}
}

@MainActor
struct ChatTests {
    static let record = ServerRecord(serverID: "h_aaaabbbbccccdddd", name: "studio", host: "127.0.0.1", port: "1",
                                   fingerprint: String(repeating: "ab", count: 32), token: "t")

    static func entry(_ seq: Int, _ text: String) -> String {
        #"{"is":"chat","id":1,"frame":{"t":"entry","entry":{"seq":\#(seq),"at":0,"ev":{"kind":"prompt","text":"\#(text)","uuid":"u"}}}}"#
    }

    @Test func replaysThenFollowsAndResumesWhereItLeftOff() async throws {
        var sockets: [ScriptedSocket] = []
        let first = ScriptedSocket()
        first.say(#"{"is":"opened","id":1}"#,
                  #"{"is":"chat","id":1,"frame":{"t":"replay","seq":2,"entries":[{"seq":1,"at":0,"ev":{"kind":"prompt","text":"one","uuid":"a"}},{"seq":2,"at":0,"ev":{"kind":"prompt","text":"two","uuid":"b"}}]}}"#,
                  #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"waiting","pending":[{"id":"toolu_1","tool":"Bash","input":{"command":"ls"}}]}}}"#,
                  Self.entry(3, "three"))
        let second = ScriptedSocket()
        second.dropWhenDone = false
        // A server replaying a little too much must not duplicate anything.
        second.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":4,"entries":[]}}"#,
                   Self.entry(3, "three again"), Self.entry(4, "four"))
        var queue = [first, second]
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in
            let s = queue.removeFirst()
            sockets.append(s)
            return s
        })
        chat.start()
        defer { chat.stop() }
        try await until { chat.entries.count == 4 }

        #expect(chat.entries.map { $0.blocks?.first?.text } == ["one", "two", "three", "four"])
        #expect(chat.pending.map(\.id) == ["toolu_1"])
        #expect(chat.activity == "waiting")
        // The second connection asked only for what came after entry 3.
        let open = sockets[1].sent.first
        #expect(open?["channel"]?["since"] == .number(3))
        #expect(open?["channel"]?["session"]?.string == "fix-tests")
        #expect(open?["channel"]?["conv"]?.string == "agent")

        try await chat.answer("toolu_1", allow: false)
        let answer = sockets[1].sent.last
        #expect(answer?["do"]?.string == "chat")
        #expect(answer?["command"]?["op"]?.string == "answer")
        #expect(answer?["command"]?["id"]?.string == "toolu_1")
        #expect(answer?["command"]?["decision"]?["allow"] == .bool(false))
    }

    private func until(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        Issue.record("timed out")
    }
}
