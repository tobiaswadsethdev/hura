import HuraKit
import SwiftUI

/// A server in the list.
struct ServerRow: View {
    let server: Server

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "shippingbox")
                .font(.title2)
                .foregroundStyle(.tint)
                .frame(width: 34)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(server.record.name).font(.headline)
                    ServerDot(state: server.state)
                }
                Text(summary)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 4)
    }

    private var summary: String {
        guard server.state == .online else { return server.state.label }
        if let problem = server.problem { return problem }
        let live = server.sessions.filter(\.isLive)
        var parts = ["\(live.count) session\(live.count == 1 ? "" : "s")"]
        let waiting = server.waiting.count
        if waiting > 0 { parts.append("\(waiting) need\(waiting == 1 ? "s" : "") you") }
        let working = live.filter { server.reports[$0.name]?.state == "running" }.count
        if working > 0 { parts.append("\(working) working") }
        return parts.joined(separator: " · ")
    }
}

struct ServerDot: View {
    let state: Server.State

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 9, height: 9)
            .accessibilityLabel(state.label)
    }

    private var color: Color {
        switch state {
        case .online: .green
        case .connecting: .yellow
        case .offline: .gray
        }
    }
}

/// A hura server's sessions, and its notifications.
struct ServerView: View {
    let server: Server
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            if server.state != .online {
                Section {
                    HStack {
                        ServerDot(state: server.state)
                        Text(server.state.label).font(.subheadline)
                    }
                }
            } else if let problem = server.problem {
                Section {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.subheadline)
                        .foregroundStyle(.orange)
                }
            }
            Section("Sessions") {
                if server.sessions.isEmpty {
                    Text("No sessions").foregroundStyle(.secondary)
                }
                ForEach(server.sessions) { s in
                    NavigationLink(value: Route.session(serverID: server.id, name: s.name)) {
                        SessionRow(session: s, report: server.reports[s.name])
                    }
                }
            }
            if model.push.isAvailable {
                NotificationSettings(server: server)
            }
        }
        .navigationTitle(server.record.name)
        .refreshable { await server.refresh() }
    }
}

struct SessionRow: View {
    let session: Session
    let report: AgentReport?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: session.isChat ? "bubble.left.and.text.bubble.right" : "terminal")
                    .foregroundStyle(.secondary)
                Text(session.name).font(.body.weight(.medium))
                Spacer()
                StatusBadge(status: agentStatus(session, report))
            }
            if let prompt = report?.state == "waiting" ? (report?.prompt ?? report?.detail) : nil {
                Text(prompt)
                    .font(.caption.monospaced())
                    .foregroundStyle(.orange)
                    .lineLimit(2)
            } else if !session.workBranch.isEmpty {
                Text(session.workBranch)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }
}

/// What a session's badge says: the agent's state when there is one,
/// in the words the badge already colours.
func agentStatus(_ session: Session, _ report: AgentReport?) -> String {
    switch report?.state ?? session.state {
    case "waiting": "needs_input"
    case "running": "working"
    case let other: other
    }
}

/// One session. A chat session opens on its conversation, with its details a
/// tap away; a terminal session, whose conversation is a screen this app
/// can't draw, shows its details and what it waits on.
struct SessionView: View {
    let server: Server
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @State private var answering = false
    @State private var error: String?
    @State private var chat: Chat?
    @State private var showingDetails = false

    private var session: Session? { server.session(named: name) }

    var body: some View {
        Group {
            if let chat {
                ChatView(chat: chat)
                    .toolbar {
                        ToolbarItem(placement: .primaryAction) {
                            Button { showingDetails = true } label: { Label("Details", systemImage: "info.circle") }
                        }
                    }
                    .sheet(isPresented: $showingDetails) {
                        NavigationStack {
                            details
                                .navigationTitle(name)
                                .navigationBarTitleDisplayMode(.inline)
                                .toolbar {
                                    ToolbarItem(placement: .confirmationAction) {
                                        Button("Done") { showingDetails = false }
                                    }
                                }
                        }
                        .presentationDetents([.medium, .large])
                    }
            } else {
                details
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        // Opened from a notification before the list has arrived: the chat
        // starts once the session is known.
        .task(id: session?.isChat == true ? session?.name : nil) {
            guard chat == nil, let session, session.isChat else { return }
            let c = Chat(record: server.record, session: session.name)
            chat = c
            c.start()
        }
        .onChange(of: scenePhase) { _, phase in
            // iOS drops the socket in the background anyway; reopen on return,
            // asking only for what was missed.
            if phase == .background { chat?.stop() } else if phase == .active { chat?.start() }
        }
        .onAppear { model.visibleSession = (server.id, name) }
        .onDisappear {
            chat?.stop()
            if model.visibleSession?.session == name { model.visibleSession = nil }
        }
    }

    private var details: some View {
        List {
            if let session {
                let report = server.reports[session.name]
                Section {
                    LabeledContent("State") { StatusBadge(status: agentStatus(session, report)) }
                    if !session.workBranch.isEmpty {
                        LabeledContent("Branch") {
                            Text(session.workBranch).font(.footnote.monospaced())
                        }
                    }
                    LabeledContent("Interface", value: session.isChat ? "Chat" : "Terminal")
                }
                // In a chat the conversation shows what waits, with its buttons.
                if chat == nil, let report, report.state == "waiting" {
                    waiting(session, report)
                }
                if !session.task.isEmpty {
                    Section("Task") {
                        Text(session.task).font(.callout)
                    }
                }
            } else {
                ContentUnavailableView("Session not found", systemImage: "questionmark.circle",
                                       description: Text(server.state == .online ? "It may have been removed." : server.state.label))
            }
        }
        .refreshable { await server.refresh() }
        .alert("Couldn't answer", isPresented: .constant(error != nil)) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
    }

    @ViewBuilder
    private func waiting(_ session: Session, _ report: AgentReport) -> some View {
        Section {
            Text(report.prompt ?? report.detail ?? "Claude is waiting")
                .font(.callout.monospaced())
                .textSelection(.enabled)
            if let ask = report.ask, session.isChat {
                HStack {
                    Button {
                        answer(session, ask, allow: true)
                    } label: {
                        Label("Allow", systemImage: "checkmark").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button(role: .destructive) {
                        answer(session, ask, allow: false)
                    } label: {
                        Label("Deny", systemImage: "xmark")
                            .foregroundStyle(.red)
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .tint(.red)
                }
                .disabled(answering)
            }
        } header: {
            Text("Waiting on you")
        } footer: {
            if !session.isChat {
                Text("A terminal session's prompt is answered in its terminal.")
            } else if report.ask == nil {
                Text("This is a question or a plan, which needs the full window to answer.")
            }
        }
    }

    private func answer(_ session: Session, _ ask: String, allow: Bool) {
        answering = true
        Task {
            defer { answering = false }
            do {
                try await server.answer(session: session.name, ask: ask, allow: allow)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
