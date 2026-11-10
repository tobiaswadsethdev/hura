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

    @Test func marksSubagentsAndLeavesOutAllows() {
        let sub = json(#"{"seq":5,"at":0,"ev":{"kind":"sdk","msg":{"type":"assistant","parent_tool_use_id":"toolu_9","message":{"content":[{"type":"text","text":"inner"}]}}}}"#)
        #expect(ChatTranscript.entry(sub)?.parent == "toolu_9")
        #expect(ChatTranscript.entry(sub)?.blocks?.first?.text == "inner")
        let started = json(#"{"seq":8,"at":0,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_9","description":"Read the docs","task_type":"local_agent"}}}"#)
        #expect(ChatTranscript.entry(started)?.kind == TranscriptEntry.Kind.task)
        #expect(ChatTranscript.entry(started)?.task?["task_id"]?.string == "a1")
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

    @Test func aPromptShowsThePicturesItSentAndNamesTheRest() {
        let e = json(#"{"seq":9,"at":0,"ev":{"kind":"prompt","text":"look","uuid":"u","files":[{"name":"a.png","path":"/sandbox/.hura/uploads/agent/u/a.png","mime":"image/png","size":3,"data":"AAA="},{"name":"b.csv","path":"/sandbox/.hura/uploads/agent/u/b.csv","mime":"text/csv","size":9,"data":null}]}}"#)
        let blocks = ChatTranscript.entry(e)?.blocks ?? []
        #expect(blocks.map(\.type) == ["image", "file", "text"])
        #expect(blocks[0].pictures == [Picture(mime: "image/png", data: "AAA=")])
        #expect(blocks[1].name == "b.csv")
        #expect(blocks[1].text == "/sandbox/.hura/uploads/agent/u/b.csv")
        #expect(blocks[2].text == "look")
    }

    @Test func aPromptKeepsWhenItWasSentAndAReceiptNamesIt() {
        let prompt = ChatTranscript.entry(json(#"{"seq":1,"at":1790000000000,"ev":{"kind":"prompt","text":"hi","uuid":"u1","receipt":true}}"#))
        #expect(prompt?.uuid == "u1")
        #expect(prompt?.receipt == true)
        #expect(prompt?.at == Date(timeIntervalSince1970: 1_790_000_000))
        let read = ChatTranscript.entry(json(#"{"seq":2,"at":1790000002500,"ev":{"kind":"read","uuid":"u1"}}"#))
        #expect(read?.kind == TranscriptEntry.Kind.read)
        #expect(read?.uuid == "u1")
        #expect(read?.at == Date(timeIntervalSince1970: 1_790_000_002.5))
    }

    @Test func readsTheReplyAsItIsWritten() {
        let delta = json(#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#)
        #expect(ChatTranscript.streamedText(delta) == "Hel")
        let other = json(#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start"}}"#)
        #expect(ChatTranscript.streamedText(other) == nil)
    }

    @Test func aShellCommandAndHowItEndedAreTwoEntries() {
        let started = ChatTranscript.entry(json(#"{"seq":1,"at":0,"ev":{"kind":"shell","uuid":"s1","command":"git status"}}"#))
        #expect(started?.kind == TranscriptEntry.Kind.shell)
        #expect(started?.uuid == "s1")
        #expect(started?.blocks?.first?.text == "git status")
        let ended = ChatTranscript.entry(json(#"{"seq":2,"at":0,"ev":{"kind":"exited","uuid":"s1","output":"clean\n","code":1,"stopped":false,"truncated":true}}"#))
        #expect(ended?.kind == TranscriptEntry.Kind.exited)
        #expect(ended?.blocks?.first?.text == "clean\n")
        #expect(ended?.blocks?.first?.truncated == true)
        #expect(ended?.code == 1)
        #expect(ended?.stopped == false)
        let stopped = ChatTranscript.entry(json(#"{"seq":3,"at":0,"ev":{"kind":"exited","uuid":"s2","output":"","code":null,"stopped":true}}"#))
        #expect(stopped?.code == nil)
        #expect(stopped?.stopped == true)
    }

    @Test func aLocalCommandsAnswerIsANotice() {
        let e = json(#"{"seq":4,"at":0,"ev":{"kind":"sdk","msg":{"type":"system","subtype":"local_command_output","content":"Total cost: $0.12"}}}"#)
        #expect(ChatTranscript.entry(e)?.kind == TranscriptEntry.Kind.notice)
        #expect(ChatTranscript.entry(e)?.blocks?.first?.text == "Total cost: $0.12")
    }

    @Test func describesAnAskLikeTheHost() throws {
        let ask = try #require(Ask(json(#"{"id":"t","tool":"Bash","input":{"command":"npm test"},"title":null}"#)))
        #expect(ask.summary == "Bash: npm test")
        #expect(!ask.isQuestion)
        #expect(Ask(json(#"{"id":"q","tool":"AskUserQuestion","input":{}}"#))?.isQuestion == true)
    }
}

struct SlashCommandTests {
    static let commands = [
        SlashCommand(name: "compact", description: "Free up context by summarizing the conversation so far",
                     argumentHint: "<optional custom summarization instructions>"),
        SlashCommand(name: "code-review", description: "Review the current diff"),
        SlashCommand(name: "security-review", description: "Complete a security review"),
        SlashCommand(name: "usage", description: "Show session cost", aliases: ["cost", "stats"]),
        SlashCommand(name: "context", description: "Show current context usage"),
        SlashCommand(name: "clear", description: "Clear the conversation and start over"),
    ]

    @Test func ranksANamesStartThenAnAliasThenAWordThenADescription() {
        #expect(SlashCommand.matching("co", in: Self.commands).map(\.name) == ["code-review", "compact", "context", "usage"])
        #expect(SlashCommand.matching("rev", in: Self.commands).map(\.name) == ["code-review", "security-review"])
        #expect(SlashCommand.matching("cost", in: Self.commands).map(\.name) == ["usage"])
        #expect(SlashCommand.matching("summ", in: Self.commands).map(\.name) == ["compact"])
        #expect(SlashCommand.matching("", in: Self.commands).count == Self.commands.count)
    }

    @Test func readsWhatIsTypedAsTheTerminalDoes() {
        guard case let .command(name, known, rest) = ComposerInput.parse("/compact keep the plan", commands: Self.commands, shell: true) else {
            Issue.record("not a command")
            return
        }
        #expect(name == "compact")
        #expect(known?.name == "compact")
        #expect(rest == " keep the plan")
        guard case let .command(_, alias, _) = ComposerInput.parse("/cost", commands: Self.commands, shell: true) else {
            Issue.record("not a command")
            return
        }
        #expect(alias?.name == "usage", "an alias names its command")
        #expect(ComposerInput.parse("!ls -la", commands: Self.commands, shell: true) == .shell("ls -la"))
        #expect(ComposerInput.parse("!ls", commands: Self.commands, shell: false) == .message, "not for a host that runs none")
        #expect(ComposerInput.parse("/etc/hosts is wrong", commands: Self.commands, shell: true) == .message)
        #expect(ComposerInput.naming("/co") == "co")
        #expect(ComposerInput.naming("/") == "")
        #expect(ComposerInput.naming("/compact now") == nil)
        #expect(ComposerInput.leadingCommand("/plugin:do it")?.command == "/plugin:do")
        #expect(ComposerInput.leadingCommand("/plugin:do it")?.rest == " it")
        #expect(ComposerInput.leadingCommand("/etc/hosts") == nil)
    }

    @Test func knowsTheKeyboardPuttingASentMessageBack() {
        #expect(ComposerInput.isEcho("fix the bug ", of: "fix the bug"))
        #expect(ComposerInput.isEcho("fix the bu", of: "fix the bu"))
        #expect(ComposerInput.isEcho("fix the bug", of: "fix the bu"), "a prediction completed its last word")
        #expect(ComposerInput.isEcho("fix the", of: "fix teh"), "an autocorrection replaced it")
        #expect(ComposerInput.isEcho("hello", of: "hel"))
        #expect(!ComposerInput.isEcho("", of: "fix the bug"))
        #expect(!ComposerInput.isEcho("a", of: "hel"), "the next message being typed")
        #expect(!ComposerInput.isEcho("now run it", of: "fix the bug"))
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

    /// A long transcript comes as several replay frames; the first state
    /// after them is what says it is all here, so the view can open at the
    /// newest rather than draw the oldest and scroll through the rest.
    @Test func isLoadedOnlyOnceTheWholeReplayIsIn() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":1,"entries":[{"seq":1,"at":0,"ev":{"kind":"prompt","text":"one","uuid":"a"}}]}}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":2,"entries":[{"seq":2,"at":0,"ev":{"kind":"prompt","text":"two","uuid":"b"}}]}}"#)
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in socket })
        chat.start()
        defer { chat.stop() }
        try await until { chat.entries.count == 2 }
        #expect(!chat.loaded)

        socket.say(#"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle"}}}"#)
        try await until { chat.loaded }
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

    @Test func offersTheEffortsOfTheModelInUseAndSetsOne() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":0,"entries":[]}}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle","model":"claude-opus-5-5","effort":"high","models":[{"value":"default","display_name":"Default","description":"","resolved":"claude-sonnet-5-5","efforts":["low","medium","high"]},{"value":"opus","display_name":"Opus","description":"","resolved":"claude-opus-5-5","efforts":["low","medium","high","xhigh","max"]}]}}}"#)
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in socket })
        chat.start()
        defer { chat.stop() }
        try await until { chat.loaded && chat.effort != nil }

        #expect(chat.effort == "high")
        #expect(chat.efforts == ["low", "medium", "high", "xhigh", "max"], "Opus, by the id the alias stands for")

        try await chat.setEffort("max")
        #expect(socket.sent.last?["command"]?["op"]?.string == "effort")
        #expect(socket.sent.last?["command"]?["effort"]?.string == "max")
        try await chat.setEffort(nil)
        #expect(socket.sent.last?["command"]?["effort"] == .null, "nil is sent, as the model's default")
    }

    @Test func readsHowFullTheContextIsAndOfWhat() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":0,"entries":[]}}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle","context_percentage":42.5,"context_size":200000}}}"#)
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in socket })
        chat.start()
        defer { chat.stop() }
        try await until { chat.loaded }
        #expect(chat.contextPercentage == 42.5)
        #expect(chat.contextSize == 200_000)
    }

    @Test func sendsFilesOnlyToAHostThatTakesThem() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":0,"entries":[]}}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle","files":true}}}"#)
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in socket })
        chat.start()
        defer { chat.stop() }
        try await until { chat.takesFiles }

        try await chat.send("plain")
        #expect(socket.sent.last?["command"]?["files"] == nil, "a message without files is the one it always was")
        try await chat.send("", files: [ChatUpload(name: "a.txt", mime: "text/plain", bytes: Data("hi".utf8))])
        guard case let .array(files)? = socket.sent.last?["command"]?["files"] else {
            Issue.record("no files sent")
            return
        }
        #expect(files.first?["name"]?.string == "a.txt")
        #expect(files.first?["data"]?.string == "aGk=")

        let big = ChatUpload(name: "big", mime: "", bytes: Data(count: ChatUpload.limit + 1))
        await #expect(throws: ServerError.self) { try await chat.send("", files: [big]) }
    }

    @Test func offersDescribedCommandsAndRunsAShellCommandOnlyWhereItCan() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        socket.say(#"{"is":"opened","id":1}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"replay","seq":0,"entries":[]}}"#,
                   #"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle","commands":["review"],"slash_commands":[{"name":"review","description":"Review a pull request","argument_hint":"<pr>","aliases":[]}],"shell":true}}}"#)
        let chat = Chat(record: Self.record, session: "fix-tests", transport: { _ in socket })
        chat.start()
        defer { chat.stop() }
        try await until { chat.runsShell }

        #expect(chat.commands == [SlashCommand(name: "review", description: "Review a pull request", argumentHint: "<pr>")])
        try await chat.shell("git status")
        #expect(socket.sent.last?["command"]?["op"]?.string == "shell")
        #expect(socket.sent.last?["command"]?["command"]?.string == "git status")

        socket.say(#"{"is":"chat","id":1,"frame":{"t":"state","state":{"activity":"idle","commands":["compact"]}}}"#)
        try await until { !chat.runsShell }
        #expect(chat.commands.map(\.name) == ["compact"], "an older host's names, undescribed")
    }

    private func until(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        Issue.record("timed out")
    }
}
