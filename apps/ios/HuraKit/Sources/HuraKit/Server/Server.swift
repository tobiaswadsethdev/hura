import Foundation
import Observation

/// One paired hura server, as the app shows it: its sessions and what each
/// one's agent is doing, kept fresh by asking every few seconds while the app
/// is in front. `/rpc` is one request and one answer, so there is no
/// socket to hold open and nothing to reconnect.
@MainActor
@Observable
public final class Server: Identifiable {
    public enum State: Equatable, Sendable {
        case connecting
        case online
        case offline(String)

        public var label: String {
            switch self {
            case .connecting: "Connecting…"
            case .online: "Online"
            case let .offline(why): why
            }
        }
    }

    public private(set) var record: ServerRecord
    public private(set) var state: State = .connecting
    public private(set) var sessions: [Session] = []
    /// The agent's state per session name, for the sessions that have one.
    public private(set) var reports: [String: AgentReport] = [:]
    /// Why the last refresh failed, when the server answered with a failure
    /// of its own (no sandbox runtime, say) rather than not answering.
    public private(set) var problem: String?

    /// Fixed for the life of the object, so it can be read from anywhere.
    public nonisolated let serverID: String
    public nonisolated var id: String { serverID }

    @ObservationIgnored fileprivate var client: ServerClient
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private let every: Duration

    public init(record: ServerRecord, every: Duration = .seconds(3)) {
        self.record = record
        serverID = record.serverID
        client = ServerClient(record: record)
        self.every = every
    }

    public func update(record: ServerRecord) {
        self.record = record
        client = ServerClient(record: record)
    }

    /// Start asking; called when the app becomes active.
    public func resume() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.refresh()
                try? await Task.sleep(for: self.every)
            }
        }
    }

    /// Stop asking; called when the app goes to the background.
    public func suspend() {
        loop?.cancel()
        loop = nil
    }

    /// The list, then each live session's state, all at once.
    public func refresh() async {
        let client = self.client
        do {
            let listed = try await client.sessions()
            sessions = listed
            problem = nil
            state = .online
            let live = listed.filter(\.isLive).map(\.name)
            let polled = await withTaskGroup(of: (String, AgentReport?).self) { group in
                for name in live {
                    group.addTask { (name, try? await client.poll(name).status) }
                }
                var out: [String: AgentReport] = [:]
                for await (name, report) in group {
                    if let report { out[name] = report }
                }
                return out
            }
            // A poll that could not tell keeps what was known.
            reports = Dictionary(uniqueKeysWithValues: live.compactMap { name in
                (polled[name] ?? reports[name]).map { (name, $0) }
            })
        } catch let e as ServerError {
            if case let .failed(_, message) = e {
                state = .online
                problem = message
            } else {
                state = .offline(e.localizedDescription)
            }
        } catch TransportError.pinMismatch {
            state = .offline("The server's certificate has changed. Pair it again.")
        } catch {
            state = .offline("Unreachable. Is Tailscale connected?")
        }
    }

    public func session(named name: String) -> Session? {
        sessions.first { $0.name == name }
    }

    /// Sessions waiting on someone.
    public var waiting: [Session] {
        sessions.filter { reports[$0.name]?.state == "waiting" }
    }

    /// Answer the request `ask` in `session`, then look again if the app is
    /// in front. From a notification it isn't, and the few seconds iOS gives
    /// an action are for the answer alone.
    public func answer(session: String, ask: String, allow: Bool) async throws {
        try await client.answer(session: session, ask: ask, allow: allow)
        if loop != nil { await refresh() }
    }
}

extension Server {
    @discardableResult
    public func configurePush(relayURL: String, pushToken: String, pushKey: String?,
                              prefs: [String: Bool]?) async throws -> PushStatus {
        try await client.configurePush(relayURL: relayURL, pushToken: pushToken, pushKey: pushKey, prefs: prefs)
    }

    public func pushStatus() async throws -> PushStatus {
        try await client.pushStatus()
    }

    public func removePush() async throws {
        try await client.removePush()
    }
}
