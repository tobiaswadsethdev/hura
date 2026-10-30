import Foundation
import Network
import os

public enum WSMessage: Equatable, Sendable {
    case text(Data)
    case binary(Data)
}

public enum TransportError: Error, Equatable, LocalizedError {
    /// The upgrade was refused with 401: the pairing token is no longer valid.
    case unauthorized
    /// The server closed the connection with this WebSocket close code.
    case closed(Int)
    /// The TLS handshake was aborted because the server key isn't the pinned one.
    case pinMismatch
    case timeout
    case failed(String)

    public var errorDescription: String? {
        switch self {
        case .unauthorized: "The server no longer accepts this phone. Pair it again with `hurad pair`."
        case let .closed(code): "Connection closed (\(code))."
        case .pinMismatch: "The server's certificate changed. Pair it again to trust the new one."
        case .timeout: "The server didn't answer in time."
        case let .failed(why): why
        }
    }

    /// Errors after which reconnecting cannot help.
    public var isPermanent: Bool {
        switch self {
        case .unauthorized, .pinMismatch: true
        default: false
        }
    }
}

/// One WebSocket connection to a server.
@MainActor
public protocol WebSocketTransport: AnyObject {
    func send(_ message: WSMessage) async throws
    /// The next message; throws a `TransportError` once the connection is gone.
    func receive() async throws -> WSMessage
    func sendPing() async throws
    func close(code: Int)
}

/// A WebSocket on Network.framework with the bearer token and pinning an
/// `hurad` requires. Its callbacks run on the main queue, so its state belongs
/// to the main actor.
@MainActor
public final class NetworkTransport: WebSocketTransport {
    private enum Phase {
        case connecting
        case ready
        /// The upgrade failed and we are asking the server why.
        case explaining
        case failed(TransportError)
    }

    private let host: String
    private let port: String
    private let pin: String
    private let token: String
    private let path: String
    private let connection: NWConnection
    private let check = PinCheck()
    private var phase = Phase.connecting
    private var readyWaiters: [CheckedContinuation<Void, Error>] = []
    private var closedLocally: Int?
    /// The close code the server sent, once it has.
    private var closedByPeer: Int?

    /// `maximumMessageSize` has to fit a conversation's replay, which is the
    /// whole transcript in one message.
    public init(host: String, port: String, pin: String, token: String, path: String,
                subprotocol: String?, maximumMessageSize: Int) {
        self.host = host
        self.port = port
        self.pin = pin
        self.token = token
        self.path = path
        let ws = NWProtocolWebSocket.Options()
        ws.autoReplyPing = true
        ws.maximumMessageSize = maximumMessageSize
        if let subprotocol { ws.setSubprotocols([subprotocol]) }
        // Sent in the upgrade request, after the verify block accepted the key.
        ws.setAdditionalHeaders([(name: "Authorization", value: "Bearer \(token)")])
        let parameters = PinnedTLS.parameters(pin: pin, check: check)
        parameters.defaultProtocolStack.applicationProtocols.insert(ws, at: 0)
        connection = NWConnection(to: .url(URL(string: "wss://\(host.urlHost):\(port)\(path)")!), using: parameters)
        connection.stateUpdateHandler = { [weak self] state in
            MainActor.assumeIsolated { self?.update(state) }
        }
        connection.start(queue: .main)
    }

    public func send(_ message: WSMessage) async throws {
        try await ready()
        switch message {
        case let .text(d): try await send(d, opcode: .text)
        case let .binary(d): try await send(d, opcode: .binary)
        }
    }

    public func receive() async throws -> WSMessage {
        try await ready()
        while true {
            let (data, opcode, closeCode, error) = await withCheckedContinuation { c in
                connection.receiveMessage { data, context, _, error in
                    let meta = context?.protocolMetadata(definition: NWProtocolWebSocket.definition) as? NWProtocolWebSocket.Metadata
                    c.resume(returning: (data, meta?.opcode, meta.map { Self.rawCode($0.closeCode) }, error))
                }
            }
            switch opcode {
            case .text: return .text(data ?? Data())
            case .binary: return .binary(data ?? Data())
            case .close:
                closedByPeer = closeCode
                throw ended(nil)
            case nil: throw ended(error)
            default: continue // ping, pong, continuation
            }
        }
    }

    public func sendPing() async throws {
        try await ready()
        let meta = NWProtocolWebSocket.Metadata(opcode: .ping)
        let once = Once()
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            meta.setPongHandler(.main) { error in
                once.run { if let error { c.resume(throwing: error) } else { c.resume() } }
            }
            let context = NWConnection.ContentContext(identifier: "ping", metadata: [meta])
            connection.send(content: Data(), contentContext: context, isComplete: true, completion: .contentProcessed { error in
                if let error { once.run { c.resume(throwing: error) } }
            })
        }
    }

    public func close(code: Int) {
        guard closedLocally == nil else { return }
        closedLocally = code
        guard case .ready = phase else {
            connection.cancel()
            return
        }
        let meta = NWProtocolWebSocket.Metadata(opcode: .close)
        meta.closeCode = (try? NWProtocolWebSocket.CloseCode(rawValue: UInt16(clamping: code))) ?? .protocolCode(.normalClosure)
        let context = NWConnection.ContentContext(identifier: "close", metadata: [meta])
        let connection = self.connection
        connection.send(content: nil, contentContext: context, isComplete: true, completion: .contentProcessed { _ in
            connection.cancel()
        })
    }

    private func send(_ data: Data, opcode: NWProtocolWebSocket.Opcode) async throws {
        let context = NWConnection.ContentContext(identifier: "message", metadata: [NWProtocolWebSocket.Metadata(opcode: opcode)])
        let error: NWError? = await withCheckedContinuation { c in
            connection.send(content: data, contentContext: context, isComplete: true, completion: .contentProcessed { c.resume(returning: $0) })
        }
        if let error { throw ended(error) }
    }

    private func ready() async throws {
        switch phase {
        case .ready: return
        case let .failed(e): throw e
        case .connecting, .explaining: try await withCheckedThrowingContinuation { readyWaiters.append($0) }
        }
    }

    private func update(_ state: NWConnection.State) {
        switch state {
        case .ready:
            guard case .connecting = phase else { return }
            phase = .ready
            resumeWaiters(nil)
        case let .waiting(error), let .failed(error):
            // .waiting means no route; the chat retries on its own schedule.
            fail(error)
        case .cancelled:
            fail(nil)
        default:
            break
        }
    }

    private func fail(_ error: NWError?) {
        switch phase {
        case .failed, .explaining: return
        default: break
        }
        let wasReady = if case .ready = phase { true } else { false }
        connection.cancel()
        if !wasReady, closedLocally == nil, check.outcome == .matched {
            // TLS succeeded but the upgrade didn't. Network.framework doesn't
            // say why; ask the server the same question over plain HTTP to tell
            // a revoked token (401) from anything else.
            phase = .explaining
            Task { await explainFailedUpgrade(error) }
            return
        }
        settle(ended(error))
    }

    private func explainFailedUpgrade(_ error: NWError?) async {
        let status = try? await PinnedHTTP.request(
            host: host, port: port, pin: pin, method: "GET", path: path,
            headers: [("Authorization", "Bearer \(token)")]).status
        settle(status == 401 ? .unauthorized : .failed(error?.localizedDescription ?? "WebSocket upgrade failed"))
    }

    private func settle(_ error: TransportError) {
        phase = .failed(error)
        resumeWaiters(error)
    }

    private func resumeWaiters(_ error: TransportError?) {
        let waiters = readyWaiters
        readyWaiters = []
        for w in waiters {
            if let error { w.resume(throwing: error) } else { w.resume() }
        }
    }

    /// The error for a connection that has ended, preferring what we know
    /// about why over what Network.framework reports.
    private func ended(_ error: NWError?) -> TransportError {
        if case let .failed(e) = phase, closedLocally == nil, closedByPeer == nil { return e }
        if let code = closedLocally { return .closed(code) }
        if let code = closedByPeer { return .closed(code) }
        if check.outcome == .mismatched { return .pinMismatch }
        if case .posix(.ETIMEDOUT) = error { return .timeout }
        return .failed(error?.localizedDescription ?? "connection closed")
    }

    private nonisolated static func rawCode(_ code: NWProtocolWebSocket.CloseCode) -> Int {
        switch code {
        case let .protocolCode(d): Int(d.rawValue)
        case let .applicationCode(c), let .privateCode(c): Int(c)
        @unknown default: 1000
        }
    }
}

/// Runs a closure at most once, from any thread.
private final class Once: Sendable {
    private let done = OSAllocatedUnfairLock(initialState: false)

    func run(_ body: () -> Void) {
        let first = done.withLock { done in
            defer { done = true }
            return !done
        }
        if first { body() }
    }
}
