import Foundation
import Network
import Observation

/// A port inside a sandbox, on this phone's loopback: `previews.rs` in the
/// desktop application, for a phone. Each preview binds a port on
/// 127.0.0.1, and every connection made to it becomes a port channel on one
/// websocket to `hurad` (`Channel::Port` in hura-proto), which runs the other
/// half. So a dev server is reached over the paired, pinned connection and
/// never exposed on a network.
///
/// The same port number is asked for first, so a dev server's idea of its own
/// URL is right; another is taken only when this phone has that one in use.
/// Previews live while the app is in front: iOS closes listeners behind it.
@MainActor
@Observable
public final class PreviewHub {
    public struct Preview: Equatable, Identifiable, Sendable {
        public var session: String
        public var port: Int
        /// The port on this phone.
        public var local: Int
        /// Connections open through it now.
        public var connections: Int
        public var url: URL { URL(string: "http://localhost:\(local)/")! }
        public var id: String { "\(session):\(port)" }
    }

    public private(set) var previews: [Preview] = []

    @ObservationIgnored private let record: ServerRecord
    @ObservationIgnored private let transport: TransportFactory
    @ObservationIgnored private var socket: WebSocketTransport?
    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var listeners: [String: NWListener] = [:]
    /// Each open channel's connection on this side, and where it came in.
    @ObservationIgnored private var tunnels: [Int: (conn: NWConnection, preview: String)] = [:]
    @ObservationIgnored private var nextID = 1

    public init(record: ServerRecord, transport: @escaping TransportFactory = Chat.network) {
        self.record = record
        self.transport = transport
    }

    /// Starts forwarding a port, or answers with the preview already doing it.
    public func open(session: String, port: Int, host: String) async throws -> Preview {
        let key = "\(session):\(port)"
        if let p = previews.first(where: { $0.id == key }) { return p }
        let listener = bind(port) ?? bind(0)
        guard let listener else { throw TransportError.failed("No port free on this phone.") }
        listeners[key] = listener
        listener.newConnectionHandler = { [weak self] conn in
            MainActor.assumeIsolated { self?.accept(conn, session: session, port: port, host: host, key: key) }
        }
        let local: Int = try await withCheckedThrowingContinuation { done in
            var answered = false
            listener.stateUpdateHandler = { state in
                MainActor.assumeIsolated {
                    guard !answered else { return }
                    switch state {
                    case .ready:
                        answered = true
                        done.resume(returning: Int(listener.port?.rawValue ?? 0))
                    case let .failed(e):
                        answered = true
                        done.resume(throwing: TransportError.failed(e.localizedDescription))
                    default:
                        break
                    }
                }
            }
            listener.start(queue: .main)
        }
        let p = Preview(session: session, port: port, local: local, connections: 0)
        previews.append(p)
        return p
    }

    /// Stops a preview here and closes every connection through it. The
    /// forward on the server is stopped with `stop-forward`.
    public func stop(session: String, port: Int) {
        let key = "\(session):\(port)"
        listeners.removeValue(forKey: key)?.cancel()
        for (id, t) in tunnels where t.preview == key { close(id, tellServer: true) }
        previews.removeAll { $0.id == key }
        if previews.isEmpty { disconnect() }
    }

    /// Everything off: the app went to the background.
    public func stopAll() {
        for p in previews { stop(session: p.session, port: p.port) }
    }

    private func bind(_ port: Int) -> NWListener? {
        let params = NWParameters.tcp
        params.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(port)) ?? .any)
        params.allowLocalEndpointReuse = true
        return try? NWListener(using: params)
    }

    // MARK: Tunnels

    private func accept(_ conn: NWConnection, session: String, port: Int, host: String, key: String) {
        let id = nextID
        nextID += 1
        tunnels[id] = (conn, key)
        count(key, by: 1)
        conn.start(queue: .main)
        Task {
            do {
                let socket = try await connected()
                try await socket.send(.text(JSONEncoder().encode(OpenPort(id: id, session: session, port: port, host: host))))
            } catch {
                close(id, tellServer: false)
            }
        }
    }

    /// Reads what the browser sends, once the server has opened the channel.
    private func pump(_ id: Int) {
        guard let conn = tunnels[id]?.conn else { return }
        conn.receive(minimumIncompleteLength: 1, maximumLength: 64 << 10) { [weak self] data, _, done, error in
            MainActor.assumeIsolated {
                guard let self, self.tunnels[id] != nil else { return }
                if let data, !data.isEmpty, let socket = self.socket {
                    let frame = Input(id: id, data: data.base64EncodedString())
                    Task { try? await socket.send(.text(JSONEncoder().encode(frame))) }
                }
                if done || error != nil {
                    self.close(id, tellServer: true)
                } else {
                    self.pump(id)
                }
            }
        }
    }

    private func close(_ id: Int, tellServer: Bool) {
        guard let t = tunnels.removeValue(forKey: id) else { return }
        t.conn.cancel()
        count(t.preview, by: -1)
        if tellServer, let socket {
            Task { try? await socket.send(.text(JSONEncoder().encode(CloseFrame(id: id)))) }
        }
    }

    private func count(_ key: String, by n: Int) {
        if let i = previews.firstIndex(where: { $0.id == key }) { previews[i].connections += n }
    }

    // MARK: The websocket

    private func connected() async throws -> WebSocketTransport {
        if let socket { return socket }
        let s = transport(record)
        socket = s
        reader = Task { [weak self] in await self?.read(s) }
        return s
    }

    private func disconnect() {
        reader?.cancel()
        reader = nil
        socket?.close(code: 1000)
        socket = nil
    }

    private func read(_ socket: WebSocketTransport) async {
        do {
            while !Task.isCancelled {
                guard case let .text(data) = try await socket.receive(),
                      let frame = try? JSONDecoder().decode(JSONValue.self, from: data),
                      case let .number(n)? = frame["id"]
                else { continue }
                let id = Int(n)
                switch frame["is"]?.string {
                case "opened":
                    pump(id)
                case "output":
                    if let b64 = frame["data"]?.string, let bytes = Data(base64Encoded: b64) {
                        tunnels[id]?.conn.send(content: bytes, completion: .contentProcessed { _ in })
                    }
                case "closed":
                    close(id, tellServer: false)
                default:
                    continue
                }
            }
        } catch {
            // The connection went: every tunnel on it went with it. The
            // listeners stay, and the next browser connection reconnects.
            if self.socket === socket {
                self.socket = nil
                for id in tunnels.keys { close(id, tellServer: false) }
            }
        }
    }
}

private struct OpenPort: Encodable {
    var `do` = "open"
    var id: Int
    var channel: Channel

    init(id: Int, session: String, port: Int, host: String) {
        self.id = id
        channel = Channel(session: session, port: port, host: host)
    }

    struct Channel: Encodable {
        var kind = "port"
        var session: String
        var port: Int
        var host: String
    }
}

private struct Input: Encodable {
    var `do` = "input"
    var id: Int
    var data: String
}

private struct CloseFrame: Encodable {
    var `do` = "close"
    var id: Int
}

// MARK: Ports

/// A process holding a port, and whether that is known or guessed from its
/// command line.
public struct PortOwner: Decodable, Equatable, Sendable {
    public var pid: Int
    public var command: String
    public var certain: Bool
}

public struct PortListener: Decodable, Equatable, Sendable {
    public var port: Int
    /// `v4` or `v6`.
    public var host: String
    public var owner: PortOwner?
}

public struct PortProcess: Decodable, Equatable, Identifiable, Sendable {
    public var pid: Int
    public var ppid: Int
    public var command: String
    /// The agent or its terminal: ended by deleting the worktree.
    public var protected: Bool
    public var id: Int { pid }
}

/// A forward `hurad` runs for previews.
public struct PortForward: Decodable, Equatable, Sendable {
    public var port: Int
    public var host: String
    public var connections: Int
    public var idleSecs: Int?

    enum CodingKeys: String, CodingKey {
        case port, host, connections
        case idleSecs = "idle_secs"
    }
}

/// The sandbox's ports as the server sees them.
public struct PortsView: Decodable, Equatable, Sendable {
    public var listening: [PortListener]
    public var processes: [PortProcess]
    public var forwards: [PortForward]
}

extension ServerClient {
    public func ports(_ name: String) async throws -> PortsView {
        try await call("ports", Named(name: name), as: PortsView.self)
    }

    /// Stops the server's forward behind a preview.
    public func stopForward(_ name: String, port: Int) async throws -> PortsView {
        struct Body: Encodable { var name: String; var port: Int }
        return try await call("stop-forward", Body(name: name, port: port), as: PortsView.self)
    }

    /// Stops the process listening on a port, inside the sandbox.
    public func killPort(_ name: String, port: Int) async throws -> PortsView {
        struct Body: Encodable { var name: String; var port: Int }
        return try await call("kill-port", Body(name: name, port: port), as: PortsView.self)
    }

    public func killProcess(_ name: String, pid: Int) async throws -> PortsView {
        struct Body: Encodable { var name: String; var pid: Int }
        return try await call("kill-process", Body(name: name, pid: pid), as: PortsView.self)
    }
}
