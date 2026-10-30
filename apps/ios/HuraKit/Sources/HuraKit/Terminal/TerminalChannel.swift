import Foundation
import Observation

/// A tmux session in a sandbox, live: the agent's own screen when `tmux` is
/// nil, or one of the shells beside it. Bytes out of the sandbox go to
/// `output`, untouched; keystrokes go back with `send`. Nothing here reads the
/// stream, because the escape sequences are the program's.
///
/// After a drop it opens the channel again. tmux redraws the whole screen for
/// a client that attaches, so nothing needs replaying.
@MainActor
@Observable
public final class TerminalChannel {
    public enum Phase: Equatable, Sendable {
        case connecting
        case live
        /// The session ended or the shell was closed; reopening won't help.
        case closed(String)
        case offline(String)
    }

    public let session: String
    public let tmux: String?
    public private(set) var phase: Phase = .connecting

    /// Where output goes. Set by the view before `start`.
    @ObservationIgnored public var output: (Data) -> Void = { _ in }

    @ObservationIgnored private let record: ServerRecord
    @ObservationIgnored private let transport: TransportFactory
    @ObservationIgnored private var socket: WebSocketTransport?
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var size: (cols: Int, rows: Int)?
    private static let channel = 1

    public init(record: ServerRecord, session: String, tmux: String?, transport: @escaping TransportFactory = Chat.network) {
        self.record = record
        self.session = session
        self.tmux = tmux
        self.transport = transport
    }

    public func start() {
        guard loop == nil else { return }
        if case .closed = phase { return }
        loop = Task { [weak self] in
            var delay: Duration = .seconds(1)
            while !Task.isCancelled {
                guard let self else { return }
                if await self.connectOnce() { delay = .seconds(1) }
                if Task.isCancelled { return }
                if case .closed = self.phase { return }
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

    /// Keystrokes, as the terminal produced them.
    public func send(_ bytes: Data) {
        guard let socket, phase == .live else { return }
        let frame = InputFrame(id: Self.channel, data: bytes.base64EncodedString())
        Task { try? await socket.send(.text(JSONEncoder().encode(frame))) }
    }

    /// The view's size in cells. The server has no other way to know it, and a
    /// tmux window keeps whatever size its last client had.
    public func resize(cols: Int, rows: Int) {
        guard cols > 1, rows > 0 else { return }
        if let size, size == (cols, rows) { return }
        size = (cols, rows)
        sendSize()
    }

    private func sendSize() {
        guard let socket, phase == .live, let size else { return }
        let frame = ResizeFrame(id: Self.channel, cols: size.cols, rows: size.rows)
        Task { try? await socket.send(.text(JSONEncoder().encode(frame))) }
    }

    private func connectOnce() async -> Bool {
        let socket = transport(record)
        self.socket = socket
        if phase != .live { phase = .connecting }
        var opened = false
        do {
            try await socket.send(.text(JSONEncoder().encode(OpenFrame(id: Self.channel, session: session, tmux: tmux))))
            while !Task.isCancelled {
                guard case let .text(data) = try await socket.receive(),
                      let frame = try? JSONDecoder().decode(JSONValue.self, from: data)
                else { continue }
                switch frame["is"]?.string {
                case "opened":
                    opened = true
                    phase = .live
                    sendSize()
                case "output":
                    if let b64 = frame["data"]?.string, let bytes = Data(base64Encoded: b64) { output(bytes) }
                case "closed":
                    let reason = frame["reason"]?.string ?? "detached"
                    // Closed before it opened is a refusal (no such session or
                    // shell); after, the program ended.
                    phase = .closed(reason)
                    socket.close(code: 1000)
                    self.socket = nil
                    return opened
                default:
                    continue
                }
            }
        } catch let e as TransportError {
            phase = e.isPermanent ? .closed(e.localizedDescription) : .offline("Unreachable. Is Tailscale connected?")
        } catch {
            phase = .offline(error.localizedDescription)
        }
        self.socket = nil
        return opened
    }
}

private struct OpenFrame: Encodable {
    var `do` = "open"
    var id: Int
    var channel: Channel

    init(id: Int, session: String, tmux: String?) {
        self.id = id
        channel = Channel(session: session, tmux: tmux)
    }

    struct Channel: Encodable {
        var kind = "terminal"
        var session: String
        var tmux: String?

        enum CodingKeys: String, CodingKey { case kind, session, tmux }

        func encode(to encoder: Encoder) throws {
            var c = encoder.container(keyedBy: CodingKeys.self)
            try c.encode(kind, forKey: .kind)
            try c.encode(session, forKey: .session)
            try c.encode(tmux, forKey: .tmux)
        }
    }
}

private struct InputFrame: Encodable {
    var `do` = "input"
    var id: Int
    var data: String
}

private struct ResizeFrame: Encodable {
    var `do` = "resize"
    var id: Int
    var cols: Int
    var rows: Int
}
