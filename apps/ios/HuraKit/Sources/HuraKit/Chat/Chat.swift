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
    /// The model id an alias such as `opus` stands for.
    public var resolved: String?
    /// The reasoning efforts it takes, lowest first; empty for none, or a
    /// host too old to say.
    public var efforts: [String]

    public var id: String { value }

    init?(_ v: JSONValue) {
        guard let value = v["value"]?.string else { return nil }
        self.value = value
        displayName = v["display_name"]?.string ?? value
        description = v["description"]?.string ?? ""
        resolved = v["resolved"]?.string
        if case let .array(list)? = v["efforts"] { efforts = list.compactMap(\.string) } else { efforts = [] }
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
        var entry = entryWithoutTime(seq: seq, ev: ev, kind: kind)
        if case let .number(ms)? = raw["at"] { entry?.at = Date(timeIntervalSince1970: ms / 1000) }
        return entry
    }

    private static func entryWithoutTime(seq: UInt64, ev: JSONValue, kind: String) -> TranscriptEntry? {
        switch kind {
        case "prompt":
            // Each file sent with it: a picture the model saw, or the path the
            // agent was given for anything else.
            var blocks = [TranscriptBlock]()
            if case let .array(files)? = ev["files"] {
                for f in files {
                    if let data = f["data"]?.string, let mime = f["mime"]?.string {
                        blocks.append(TranscriptBlock(type: "image", name: f["name"]?.string,
                                                      pictures: [Picture(mime: mime, data: data)]))
                    } else if let path = f["path"]?.string {
                        blocks.append(TranscriptBlock(type: "file", text: path, name: f["name"]?.string))
                    }
                }
            }
            blocks.append(TranscriptBlock(type: "text", text: ev["text"]?.string ?? ""))
            var prompt = TranscriptEntry(seq: seq, uuid: ev["uuid"]?.string, kind: TranscriptEntry.Kind.prompt, blocks: blocks)
            prompt.receipt = ev["receipt"]?.bool ?? false
            return prompt
        case "read":
            return ev["uuid"]?.string.map { TranscriptEntry(seq: seq, uuid: $0, kind: TranscriptEntry.Kind.read) }
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

    /// A subagent's messages are kept, marked with the call that started it,
    /// for the screen that follows one; the conversation leaves them out.
    static func sdk(_ m: JSONValue, seq: UInt64) -> TranscriptEntry? {
        var entry = sdkEntry(m, seq: seq)
        entry?.parent = m["parent_tool_use_id"]?.string
        return entry
    }

    private static func sdkEntry(_ m: JSONValue, seq: UInt64) -> TranscriptEntry? {
        let mainThread = m["parent_tool_use_id"] == nil || m["parent_tool_use_id"] == .null
        switch m["type"]?.string {
        case "assistant":
            guard case let .array(content)? = m["message"]?["content"] else { return nil }
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
                let pictures = resultPictures(b["content"])
                return TranscriptBlock(type: "tool_result", text: resultText(b["content"]), toolUseID: id,
                                       isError: b["is_error"]?.bool ?? false,
                                       pictures: pictures.isEmpty ? nil : pictures)
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
            case "task_started", "task_updated", "task_notification":
                return TranscriptEntry(seq: seq, kind: TranscriptEntry.Kind.task, task: m)
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

    /// The images in a tool result, sent whole as base64.
    static func resultPictures(_ content: JSONValue?) -> [Picture] {
        guard case let .array(parts)? = content else { return [] }
        return parts.compactMap { p in
            guard p["type"]?.string == "image", let source = p["source"], source["type"]?.string == "base64",
                  let mime = source["media_type"]?.string, let data = source["data"]?.string else { return nil }
            return Picture(mime: mime, data: data)
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

/// A file sent with a message (`ChatUpload`): saved in the sandbox by the
/// chat host, and shown to the model as well when it is a picture.
public struct ChatUpload: Encodable, Equatable, Identifiable, Sendable {
    public var name: String
    public var mime: String
    /// Base64.
    public var data: String
    /// Kept in step with `UPLOAD_BYTES` in `hura-agent.mjs`: the most a
    /// message's files may come to.
    public static let limit = 10 << 20

    public init(name: String, mime: String, bytes: Data) {
        self.name = name
        self.mime = mime
        data = bytes.base64EncodedString()
        size = bytes.count
        id = UUID()
    }

    /// Decoded, for the limit.
    public let size: Int
    public let id: UUID

    enum CodingKeys: String, CodingKey { case name, mime, data }
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
    /// The reasoning effort, `low` to `max`; nil for the model's default.
    public private(set) var effort: String?
    /// The models this account can switch to, once Claude Code has said.
    public private(set) var models: [ChatModel] = []
    /// Slash commands on offer, without the slash.
    public private(set) var commands: [String] = []
    /// What Claude Code is doing that isn't a turn: `compacting`.
    public private(set) var status: String?
    public private(set) var contextPercentage: Double?
    /// The reply so far, before its message arrives whole.
    public private(set) var streaming = ""
    /// What each running subagent last said it was doing (an SDK
    /// `task_progress`), by task id. Live only: never in the transcript.
    public private(set) var tasks: [String: JSONValue] = [:]
    /// Why there is nothing yet, from hurad: a session still cloning, say.
    public private(set) var notice: String?
    /// The host refused something sent on this channel.
    public var error: String?
    /// The transcript has arrived whole at least once. A long one comes as
    /// several replay frames, and the host sends the state after the last
    /// of them, so it is the first state that says the history is all here
    /// -- the view waits for it rather than drawing the oldest first and
    /// scrolling through the rest as it arrives.
    public private(set) var loaded = false
    /// The host takes files with a message. One built before it did drops
    /// them without a word, so they're offered only when this says.
    public private(set) var takesFiles = false
    /// The host says when Claude has read a message. Without it every
    /// message would look unread, so receipts show only when this says.
    public private(set) var receipts = false

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

    public func send(_ text: String, files: [ChatUpload] = []) async throws {
        struct Body: Encodable {
            var text: String
            var files: [ChatUpload]
            enum CodingKeys: String, CodingKey { case text, files }
            // Without the key when there are none: the message an older host
            // has always read.
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(text, forKey: .text)
                if !files.isEmpty { try c.encode(files, forKey: .files) }
            }
        }
        if files.map(\.size).reduce(0, +) > ChatUpload.limit {
            throw ServerError.failed(kind: "too-large", message: "A message takes \(ChatUpload.limit >> 20) MB of files.")
        }
        try await command(Op(op: "send", body: Body(text: text, files: files)))
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

    /// The efforts the model in use takes: found by its alias or by the id
    /// it stands for, and the default's when nothing has been chosen.
    public var efforts: [String] {
        guard let model else { return models.first?.efforts ?? [] }
        return models.first { $0.value == model || $0.resolved == model }?.efforts ?? []
    }

    /// Sets the reasoning effort; nil for the model's default.
    public func setEffort(_ effort: String?) async throws {
        struct Body: Encodable {
            var effort: String?
            enum CodingKeys: String, CodingKey { case effort }
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(effort, forKey: .effort)
            }
        }
        try await command(Op(op: "effort", body: Body(effort: effort)))
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
                                                : "Can't reach the server.")
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
        case "entry":
            if let e = f["entry"] { append(e) }
        case "task":
            if let ev = f["ev"], let id = ev["task_id"]?.string { tasks[id] = ev }
        case "partial":
            if let ev = f["ev"], let text = ChatTranscript.streamedText(ev) { streaming += text }
        case "state":
            guard let s = f["state"] else { return }
            activity = s["activity"]?.string ?? "idle"
            if case let .array(list)? = s["pending"] { pending = list.compactMap(Ask.init) } else { pending = [] }
            mode = s["mode"]?.string
            model = s["model"]?.string
            effort = s["effort"]?.string
            if case let .array(list)? = s["models"] { models = list.compactMap(ChatModel.init) } else { models = [] }
            if case let .array(list)? = s["commands"] { commands = list.compactMap(\.string) } else { commands = [] }
            status = s["status"]?.string
            if case let .number(n)? = s["context_percentage"] { contextPercentage = n } else { contextPercentage = nil }
            takesFiles = s["files"]?.bool ?? false
            receipts = s["receipts"]?.bool ?? false
            if activity == "idle" { streaming = "" }
            loaded = true
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
        if let msg = raw["ev"]?["msg"], msg["type"]?.string == "assistant",
           msg["parent_tool_use_id"]?.string == nil { streaming = "" }
        if let entry = ChatTranscript.entry(raw) { entries.append(entry) }
    }
}

/// `ClientFrame::Chat`: a command on a chat channel.
private struct CommandFrame<C: Encodable>: Encodable {
    var `do` = "chat"
    var id: Int
    var command: C
}
