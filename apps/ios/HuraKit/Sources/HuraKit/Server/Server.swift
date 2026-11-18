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
    /// What hurad last listed, less what is being deleted.
    public var sessions: [Session] { listed.filter { !deleting.contains($0.name) } }
    private var listed: [Session] = []
    /// Sessions asked to be deleted: gone from the list at once, while hurad
    /// takes its time tearing the sandbox down. Each stays hidden until a
    /// listing no longer has it, so a refresh that started before the delete
    /// finished can't bring it back.
    private var deleting: Set<String> = []
    /// Why the last delete failed; the session is back in the list.
    public var deleteFailure: String?
    public private(set) var projects: [Project] = []
    /// What is waiting on the server: sessions to start and messages to say.
    /// Empty from a server too old to have a queue.
    public private(set) var queue = QueueView()
    /// The agent's state per session name, for the sessions that have one.
    public private(set) var reports: [String: AgentReport] = [:]
    /// Why the last refresh failed, when the server answered with a failure
    /// of its own (no sandbox runtime, say) rather than not answering.
    public private(set) var problem: String?

    /// Fixed for the life of the object, so it can be read from anywhere.
    public nonisolated let serverID: String
    public nonisolated var id: String { serverID }

    /// Ports in this server's sandboxes, forwarded to this phone.
    public let previews: PreviewHub
    /// Where its conversations are kept, when they are: see `ChatHistory`.
    /// A session's names are written there on each refresh, its
    /// conversations caught up when a turn in it ends and before it is
    /// deleted, since the sandbox is then the only place they are.
    @ObservationIgnored public var history: ChatHistory?
    @ObservationIgnored fileprivate var client: ServerClient
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private let every: Duration

    public init(record: ServerRecord, every: Duration = .seconds(3)) {
        self.record = record
        serverID = record.serverID
        client = ServerClient(record: record)
        previews = PreviewHub(record: record)
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
        // iOS closes listeners behind an app in the background anyway.
        previews.stopAll()
    }

    /// The list, then each live session's state, all at once.
    public func refresh() async {
        let client = self.client
        do {
            async let named = client.projects()
            async let waiting = client.queue()
            let listed = try await client.sessions()
            self.listed = listed
            deleting.formIntersection(listed.map(\.name))
            // An older hurad without projects still lists its sessions.
            if let named = try? await named { projects = named }
            if let waiting = try? await waiting { queue = waiting }
            problem = nil
            state = .online
            for s in listed {
                history?.name(serverID: serverID, session: s.name, title: s.title, project: s.project)
            }
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
            // A turn has ended in a chat session: what it said is kept, in
            // case no screen had the conversation open. On a turn's end, not
            // a timer, because asking execs into the sandbox, and one that
            // has stopped would be started by being asked.
            let chats = Set(listed.filter(\.isChat).map(\.name))
            for (name, now) in polled where chats.contains(name) {
                if reports[name]?.state == "running", now.state != "running" { catchUpLater(name) }
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
                state = .offline(e.userMessage)
            }
        } catch TransportError.pinMismatch {
            state = .offline("The server's certificate has changed. Pair it again.")
        } catch {
            state = .offline("Can't reach the server.")
        }
    }

    /// Sets what a refresh would have, for tests and previews.
    func load(projects: [Project], sessions: [Session]) {
        self.projects = projects
        listed = sessions
        state = .online
    }

    /// The client, for a screen that asks something of this server once.
    public var api: ServerClient { client }

    /// A project's worktrees, in the order they were put in.
    public func worktrees(in project: Project) -> [Session] {
        sessions.filter { $0.project == project.name }.sorted(by: Self.byPlace)
    }

    /// The order worktrees are drawn in, the same as the desktop's: the ones
    /// nobody has placed first, newest first, since a worktree just started
    /// is the one you are about to look for; then the placed ones, in the
    /// order they were put. The order lives on the server, so it is the same
    /// on every device.
    static func byPlace(_ a: Session, _ b: Session) -> Bool {
        switch (a.rank, b.rank) {
        case let (x?, y?) where x != y: x < y
        case (nil, _?): true
        case (_?, nil): false
        default: a.createdAt != b.createdAt ? a.createdAt > b.createdAt : a.name < b.name
        }
    }

    /// Sessions waiting to start in a project, in the order they will.
    public func waiting(in project: Project) -> [QueuedSession] {
        queue.sessions.filter { $0.new.project == project.name }
    }

    /// Sessions waiting to start in no project this server knows.
    public var waitingUnfiled: [QueuedSession] {
        let known = Set(projects.map(\.name))
        return queue.sessions.filter { $0.new.project.map { !known.contains($0) } ?? true }
    }

    /// Starts a session later: at `at`, or once the queue has room.
    public func enqueue(_ session: NewSession, at: Date?) async throws {
        queue = try await client.enqueue(session, at: at)
    }

    public func unqueue(_ id: Int64) async throws {
        queue = try await client.unqueue(id)
    }

    public func startQueued(_ id: Int64) async throws {
        queue = try await client.startQueued(id)
    }

    /// Says something to a session's agent once its turn is over, and not
    /// before `at`.
    public func queueMessage(_ name: String, text: String, at: Date?) async throws {
        queue = try await client.queueMessage(name, text: text, at: at)
    }

    /// Sessions that belong to no project this server knows.
    public var unfiled: [Session] {
        let known = Set(projects.map(\.name))
        return sessions.filter { $0.project.map { !known.contains($0) } ?? true }
            .sorted(by: Self.byPlace)
    }

    public func newProject(from repo: LocalRepo) async throws {
        guard let origin = repo.origin else { throw ServerError.failed(kind: "bad_request", message: "\(repo.display) has no origin to clone.") }
        projects = try await client.newProject(path: repo.path, repo: origin, name: nil)
    }

    public func forgetProject(_ name: String) async throws {
        projects = try await client.forgetProject(name)
    }

    /// Puts the projects in this order. Shown at once rather than after the
    /// round trip, so a dragged row stays where it was dropped; a refusal
    /// puts back what the server has.
    public func reorderProjects(_ names: [String]) async throws {
        projects.sort { (names.firstIndex(of: $0.name) ?? .max) < (names.firstIndex(of: $1.name) ?? .max) }
        do {
            projects = try await client.reorderProjects(names)
        } catch {
            await refresh()
            throw error
        }
    }

    /// Puts these worktrees, one project's, in this order. Shown at once, as
    /// `reorderProjects` is.
    public func reorderSessions(_ names: [String]) async throws {
        for i in listed.indices {
            if let rank = names.firstIndex(of: listed[i].name) { listed[i].rank = rank }
        }
        do {
            listed = try await client.reorderSessions(names)
        } catch {
            await refresh()
            throw error
        }
    }

    /// Starts a session and answers with its name, once the list has it.
    public func create(_ session: NewSession) async throws -> String {
        let name = try await client.create(session)
        deleting.remove(name)
        await refresh()
        return name
    }

    /// Deletes a session in the background: it leaves the list now, and
    /// comes back with `deleteFailure` set if hurad couldn't delete it.
    public func destroy(_ name: String) {
        let chat = session(named: name)?.isChat == true
        deleting.insert(name)
        reports[name] = nil
        Task {
            do {
                if chat { await catchUp(name, waiting: true) }
                listed = try await client.destroy(name)
                history?.removed(serverID: serverID, session: name)
            } catch {
                deleting.remove(name)
                deleteFailure = "\(name) wasn't deleted: \(error.userMessage)"
            }
        }
    }

    /// Renames a session: its title, which nothing is keyed by.
    public func retitle(_ name: String, to title: String) async throws {
        listed = try await client.retitle(name, title: title)
    }

    /// Keeps what a chat session's hosts have that this phone has not: every
    /// conversation, or one. In the background unless `waiting`.
    public func catchUp(_ name: String, conversation: String? = nil, waiting: Bool = false) async {
        guard let history else { return }
        let client = self.client
        let record = self.record
        let work = Task { @MainActor in
            let convs: [String]
            if let conversation { convs = [conversation] } else {
                guard let listed = try? await client.chats(name) else { return }
                convs = listed.chats
            }
            await history.catchUp(record: record, session: name, conversations: convs)
        }
        if waiting { await work.value }
    }

    private func catchUpLater(_ name: String) {
        Task { await catchUp(name) }
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
        try await answer(session: session, ask: ask, decision: ChatDecision(allow: allow))
    }

    public func answer(session: String, ask: String, decision: ChatDecision) async throws {
        try await client.answer(session: session, ask: ask, decision: decision)
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
