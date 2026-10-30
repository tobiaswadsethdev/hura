import Foundation
import Observation

/// A permission request, question or plan waiting in a hura chat
/// (`ChatAsk` in `hura-core/src/chat.rs`).
public struct Ask: Equatable, Identifiable, Sendable {
    public var id: String
    public var tool: String
    public var input: JSONValue
    /// Claude Code's own sentence for it, when it has one.
    public var title: String?
    public var description: String?
    public var displayName: String?
    /// Why it asks rather than deciding.
    public var reason: String?
    /// A path outside the working copy that the call would touch.
    public var blockedPath: String?
    /// Whether "don't ask again" is on offer.
    public var canRemember: Bool
    /// Approving it mustn't be one stray tap.
    public var defaultToNo: Bool
    /// The subagent asking, when it isn't the main thread.
    public var agent: String?
    /// Questions and plans need a choice a button can't carry.
    public var isQuestion: Bool { tool == "AskUserQuestion" || tool == "ExitPlanMode" }

    public init?(_ v: JSONValue) {
        guard let id = v["id"]?.string, let tool = v["tool"]?.string else { return nil }
        self.id = id
        self.tool = tool
        input = v["input"] ?? .object([:])
        title = v["title"]?.string
        description = v["description"]?.string
        displayName = v["display_name"]?.string
        reason = v["reason"]?.string
        blockedPath = v["blocked_path"]?.string
        canRemember = v["can_remember"]?.bool ?? false
        defaultToNo = v["default_to_no"]?.bool ?? false
        agent = v["agent"]?.string
    }

    /// One line saying what it asks, as the chat host describes it.
    public var summary: String {
        if let title, !title.isEmpty { return title }
        let target = ["command", "file_path", "url", "path", "pattern"].lazy
            .compactMap { self.input[$0]?.string }.first { !$0.isEmpty }
        return target.map { "\(tool): \($0)" } ?? tool
    }
}

/// A model Claude Code offers (`ChatModel`).
public struct ChatModel: Equatable, Identifiable, Sendable {
    public var value: String
    public var displayName: String
    public var description: String

    public var id: String { value }

    init?(_ v: JSONValue) {
        guard let value = v["value"]?.string else { return nil }
        self.value = value
        displayName = v["display_name"]?.string ?? value
        description = v["description"]?.string ?? ""
    }
}

/// The chat protocol's entries, translated into the transcript shapes the
/// chat screen draws.
public enum ChatTranscript {
    /// One hura `ChatEntry` (`{seq, at, ev}`) as a transcript entry, or nil
    /// for what the chat doesn't show. Follows what the desktop window
    /// shows (`apps/desktop/src/chat/transcript.ts`): a subagent's messages
    /// are left out, a user message's text is Claude Code's own note (yours
    /// arrive as `prompt` entries), and a `result` ends the turn.
    public static func entry(_ raw: JSONValue) -> TranscriptEntry? {
        guard case let .number(n)? = raw["seq"], let ev = raw["ev"], let kind = ev["kind"]?.string else { return nil }
        let seq = UInt64(n)
        switch kind {
        case "prompt":
            return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.prompt,
                                   blocks: [TranscriptBlock(type: "text", text: ev["text"]?.string ?? "")])
        case "answer":
            // An allow shows as the tool running; a refusal is worth a line.
            guard ev["allow"]?.bool == false else { return nil }
            var text = "Declined \(ev["tool"]?.string ?? "a tool")"
            if let m = ev["message"]?.string, !m.isEmpty { text += ": \(m)" }
            return notice(seq, text)
        case "error":
            return notice(seq, ev["message"]?.string ?? "The conversation failed.", level: "error")
        case "sdk":
            return ev["msg"].flatMap { sdk($0, seq: seq) }
        default:
            return nil
        }
    }

    static func sdk(_ m: JSONValue, seq: UInt64) -> TranscriptEntry? {
        let mainThread = m["parent_tool_use_id"] == nil || m["parent_tool_use_id"] == .null
        switch m["type"]?.string {
        case "assistant":
            guard mainThread, case let .array(content)? = m["message"]?["content"] else { return nil }
            if let error = m["error"]?.string {
                let said = content.compactMap { $0["type"]?.string == "text" ? $0["text"]?.string : nil }
                    .joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
                return notice(seq, said.isEmpty ? error : said, level: "error")
            }
            let blocks: [TranscriptBlock] = content.compactMap { b in
                switch b["type"]?.string {
                case "text": b["text"]?.string.map { TranscriptBlock(type: "text", text: $0) }
                case "thinking": b["thinking"]?.string.map { TranscriptBlock(type: "thinking", text: $0) }
                case "tool_use":
                    b["id"]?.string.map { TranscriptBlock(type: "tool_use", id: $0, name: b["name"]?.string,
                                                          input: b["input"] ?? .object([:])) }
                default: nil
                }
            }
            return blocks.isEmpty ? nil : TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.assistant, blocks: blocks)
        case "user":
            let content = m["message"]?["content"]
            if case let .string(text)? = content {
                return mainThread ? note(seq, text) : nil
            }
            guard case let .array(blocks)? = content else { return nil }
            let results: [TranscriptBlock] = blocks.compactMap { b in
                guard b["type"]?.string == "tool_result", let id = b["tool_use_id"]?.string else { return nil }
                return TranscriptBlock(type: "tool_result", text: resultText(b["content"]), toolUseID: id,
                                       isError: b["is_error"]?.bool ?? false)
            }
            if !results.isEmpty {
                return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.toolResult, blocks: results)
            }
            let text = blocks.compactMap { $0["type"]?.string == "text" ? $0["text"]?.string : nil }.joined(separator: "\n")
            return mainThread ? note(seq, text) : nil
        case "result":
            guard case let .number(ms)? = m["duration_ms"] else { return nil }
            return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.turnEnd, durationMS: Int64(ms))
        case "system":
            switch m["subtype"]?.string {
            case "compact_boundary":
                return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.divider, reason: "compact")
            case "permission_denied":
                return notice(seq, "\(m["tool_name"]?.string ?? "A tool") was not allowed")
            default:
                return nil
            }
        case "conversation_reset":
            return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.divider, reason: "clear")
        default:
            return nil
        }
    }

    static func resultText(_ content: JSONValue?) -> String {
        switch content {
        case let .string(s)?: s
        case let .array(parts)?: parts.compactMap { $0["text"]?.string }.joined(separator: "\n")
        default: ""
        }
    }

    static func notice(_ seq: UInt64, _ text: String, level: String = "info") -> TranscriptEntry {
        TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.notice, blocks: [TranscriptBlock(type: "text", text: text)],
                        level: level)
    }

    /// Text in a user message that isn't yours: an interrupt, a skill's
    /// content. Kept short, as the desktop keeps it.
    static func note(_ seq: UInt64, _ text: String) -> TranscriptEntry? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return nil }
        return notice(seq, t.count > 400 ? String(t.prefix(400)) + "…" : t)
    }

    /// The reply as it is being written: the text of a `stream_event`'s
    /// delta, on the main thread.
    static func streamedText(_ ev: JSONValue) -> String? {
        guard ev["parent_tool_use_id"] == nil || ev["parent_tool_use_id"] == .null,
              let event = ev["event"], event["type"]?.string == "content_block_delta",
              event["delta"]?["type"]?.string == "text_delta"
        else { return nil }
        return event["delta"]?["text"]?.string
    }
}

/// Opens a websocket to a hura server.
public typealias TransportFactory = @MainActor (_ record: ServerRecord) -> WebSocketTransport

/// One conversation in a hura chat session, live: its transcript, what is
/// waiting on you, and the reply as it is written. One websocket, one chat
/// channel on it; after a drop it opens again from the last entry it has, so
/// nothing arrives twice and nothing is missed.
@MainActor
@Observable
public final class Chat {
    public enum Phase: Equatable, Sendable {
        case connecting
        case live
        case offline(String)
    }

    public let session: String
    public let conversation: String
    public private(set) var phase: Phase = .connecting
    public private(set) var entries: [TranscriptEntry] = []
    /// `idle`, `running` or `waiting`.
    public private(set) var activity = "idle"
    public private(set) var pending: [Ask] = []
    /// Claude Code's permission mode: `auto`, `default`, `acceptEdits`, `plan`.
    public private(set) var mode: String?
    public private(set) var model: String?
    /// The models this account can switch to, once Claude Code has said.
    public private(set) var models: [ChatModel] = []
    /// Slash commands on offer, without the slash.
    public private(set) var commands: [String] = []
    /// What Claude Code is doing that isn't a turn: `compacting`.
    public private(set) var status: String?
    public private(set) var costUSD: Double?
    public private(set) var contextPercentage: Double?
    /// The reply so far, before its message arrives whole.
    public private(set) var streaming = ""
    /// Why there is nothing yet, from hurad: a session still cloning, say.
    public private(set) var notice: String?
    /// The host refused something sent on this channel.
    public var error: String?
    /// The transcript has arrived at least once.
    public private(set) var loaded = false

    @ObservationIgnored private let record: ServerRecord
    @ObservationIgnored private let transport: TransportFactory
    @ObservationIgnored private var socket: WebSocketTransport?
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var lastSeq: UInt64 = 0
    private static let channel = 1

    public init(record: ServerRecord, session: String, conversation: String = "agent",
                transport: @escaping TransportFactory = Chat.network) {
        self.record = record
        self.session = session
        self.conversation = conversation
        self.transport = transport
    }

    public static let network: TransportFactory = { record in
        NetworkTransport(host: record.host, port: record.port, pin: record.fingerprint, token: record.token,
                         path: "/ws", subprotocol: nil, maximumMessageSize: 64 << 20)
    }

    public func start() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            var delay: Duration = .seconds(1)
            while !Task.isCancelled {
                guard let self else { return }
                if await self.connectOnce() { delay = .seconds(1) }
                if Task.isCancelled { return }
                try? await Task.sleep(for: delay)
                delay = min(delay * 2, .seconds(30))
            }
        }
    }

    public func stop() {
        loop?.cancel()
        loop = nil
        socket?.close(code: 1000)
        socket = nil
    }

    public func send(_ text: String) async throws {
        struct Body: Encodable { var text: String }
        try await command(Op(op: "send", body: Body(text: text)))
    }

    public func interrupt() async throws {
        try await command(Op(op: "interrupt", body: Nothing()))
    }

    public func answer(_ ask: String, allow: Bool) async throws {
        try await answer(ask, decision: ChatDecision(allow: allow))
    }

    public func answer(_ ask: String, decision: ChatDecision) async throws {
        struct Body: Encodable {
            var id: String
            var decision: ChatDecision
        }
        try await command(Op(op: "answer", body: Body(id: ask, decision: decision)))
    }

    /// Switches the permission mode.
    public func setMode(_ mode: String) async throws {
        struct Body: Encodable { var mode: String }
        try await command(Op(op: "mode", body: Body(mode: mode)))
    }

    /// Switches the model; nil for the account's default.
    public func setModel(_ model: String?) async throws {
        struct Body: Encodable {
            var model: String?
            enum CodingKeys: String, CodingKey { case model }
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(model, forKey: .model)
            }
        }
        try await command(Op(op: "model", body: Body(model: model)))
    }

    private func command<C: Encodable>(_ command: C) async throws {
        guard let socket, phase == .live else { throw ServerError.malformed("not connected") }
        try await socket.send(.text(JSONEncoder().encode(CommandFrame(id: Self.channel, command: command))))
    }

    /// One connection, until it ends. True when it got as far as the channel.
    private func connectOnce() async -> Bool {
        let socket = transport(record)
        self.socket = socket
        if phase != .live { phase = .connecting }
        let open: [String: RequestJSON] = [
            "do": .string("open"), "id": .number(Self.channel),
            "channel": .object(["kind": .string("chat"), "session": .string(session),
                                "conv": .string(conversation), "since": .number(Int(lastSeq))]),
        ]
        var opened = false
        do {
            try await socket.send(.text(JSONEncoder().encode(open)))
            while !Task.isCancelled {
                guard case let .text(data) = try await socket.receive(),
                      let frame = try? JSONDecoder().decode(JSONValue.self, from: data)
                else { continue }
                switch frame["is"]?.string {
                case "opened":
                    opened = true
                    phase = .live
                case "chat":
                    if let f = frame["frame"] { apply(f) }
                case "closed":
                    phase = .offline(frame["reason"]?.string ?? "The conversation closed.")
                    socket.close(code: 1000)
                    return opened
                default:
                    continue
                }
            }
        } catch let e as TransportError {
            phase = .offline(e == .unauthorized ? ServerError.unauthorized.localizedDescription
                                                : "Unreachable. Is Tailscale connected?")
        } catch {
            phase = .offline(error.localizedDescription)
        }
        self.socket = nil
        return opened
    }

    func apply(_ f: JSONValue) {
        switch f["t"]?.string {
        case "replay":
            if case let .array(list)? = f["entries"] { list.forEach(append) }
            loaded = true
        case "entry":
            if let e = f["entry"] { append(e) }
        case "partial":
            if let ev = f["ev"], let text = ChatTranscript.streamedText(ev) { streaming += text }
        case "state":
            guard let s = f["state"] else { return }
            activity = s["activity"]?.string ?? "idle"
            if case let .array(list)? = s["pending"] { pending = list.compactMap(Ask.init) } else { pending = [] }
            mode = s["mode"]?.string
            model = s["model"]?.string
            if case let .array(list)? = s["models"] { models = list.compactMap(ChatModel.init) } else { models = [] }
            if case let .array(list)? = s["commands"] { commands = list.compactMap(\.string) } else { commands = [] }
            status = s["status"]?.string
            if case let .number(n)? = s["cost_usd"] { costUSD = n } else { costUSD = nil }
            if case let .number(n)? = s["context_percentage"] { contextPercentage = n } else { contextPercentage = nil }
            if activity == "idle" { streaming = "" }
        case "error":
            error = f["message"]?.string
        case "notice":
            notice = f["text"]?.string
            loaded = true
        default:
            break
        }
    }

    private func append(_ raw: JSONValue) {
        guard case let .number(n)? = raw["seq"] else { return }
        let seq = UInt64(n)
        // A replay after a reconnect starts after what this already has, but
        // never trust that twice.
        guard seq > lastSeq else { return }
        lastSeq = seq
        notice = nil
        if raw["ev"]?["msg"]?["type"]?.string == "assistant" { streaming = "" }
        if let entry = ChatTranscript.entry(raw) { entries.append(entry) }
    }
}

/// `ClientFrame::Chat`: a command on a chat channel.
private struct CommandFrame<C: Encodable>: Encodable {
    var `do` = "chat"
    var id: Int
    var command: C
}
