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

/// A hura server: its projects with their worktrees, then sessions that
/// belong to none, then its notifications. Shaped like the desktop window's
/// tree.
struct ServerView: View {
    let server: Server
    @Environment(AppModel.self) private var model
    @State private var addingProject = false
    @State private var creatingIn: Project?
    @State private var destroying: Session?
    @State private var forgetting: Project?
    @State private var screen: ServerScreen?
    @State private var error: String?

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
            ForEach(server.projects) { project in
                Section {
                    let worktrees = server.worktrees(in: project)
                    ForEach(worktrees) { row($0) }
                    Button {
                        creatingIn = project
                    } label: {
                        Label("New Worktree", systemImage: "plus")
                    }
                    .disabled(server.state != .online)
                } header: {
                    ProjectHeader(project: project) { forgetting = project }
                }
            }
            if !server.unfiled.isEmpty {
                Section(server.projects.isEmpty ? "Sessions" : "Not in a project") {
                    ForEach(server.unfiled) { row($0) }
                }
            }
            if server.state == .online, server.projects.isEmpty, server.sessions.isEmpty {
                Section {
                    ContentUnavailableView {
                        Label("No projects", systemImage: "folder")
                    } description: {
                        Text("A project is a repository on this server that you start worktrees from.")
                    } actions: {
                        Button("New Project") { addingProject = true }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
            if model.push.isAvailable {
                NotificationSettings(server: server)
            }
        }
        .navigationTitle(server.record.name)
        .refreshable { await server.refresh() }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button { addingProject = true } label: { Label("New Project", systemImage: "folder.badge.plus") }
                    if !server.projects.isEmpty {
                        Menu {
                            ForEach(server.projects) { p in
                                Button(p.name) { creatingIn = p }
                            }
                        } label: {
                            Label("New Worktree", systemImage: "arrow.triangle.branch")
                        }
                    }
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .disabled(server.state != .online)
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Integrations", systemImage: "puzzlepiece.extension") { screen = .integrations }
                    Button("Settings", systemImage: "gearshape") { screen = .settings }
                    Button("About", systemImage: "info.circle") { screen = .about }
                } label: {
                    Label("Server", systemImage: "ellipsis.circle")
                }
            }
        }
        .navigationDestination(item: $screen) { screen in
            switch screen {
            case .integrations: IntegrationsView(server: server)
            case .settings: ServerSettingsView(server: server)
            case .about: AboutView(server: server)
            }
        }
        .sheet(isPresented: $addingProject) {
            NewProjectView(server: server)
        }
        .sheet(item: $creatingIn) { project in
            NewWorktreeView(server: server, project: project) { name in
                model.pendingRoute = [.server(server.id), .session(serverID: server.id, name: name)]
            }
        }
        .confirmationDialog("Delete \(destroying?.name ?? "")?", isPresented: .constant(destroying != nil),
                            titleVisibility: .visible, presenting: destroying) { s in
            Button("Delete Worktree", role: .destructive) {
                destroying = nil
                act { try await server.destroy(s.name) }
            }
            Button("Cancel", role: .cancel) { destroying = nil }
        } message: { _ in
            Text("Its sandbox is deleted, with everything in it that hasn't been pushed.")
        }
        .confirmationDialog("Forget \(forgetting?.name ?? "")?", isPresented: .constant(forgetting != nil),
                            titleVisibility: .visible, presenting: forgetting) { p in
            Button("Forget Project", role: .destructive) {
                forgetting = nil
                act { try await server.forgetProject(p.name) }
            }
            Button("Cancel", role: .cancel) { forgetting = nil }
        } message: { _ in
            Text("Its worktrees keep running. The checkout on the server isn't touched.")
        }
        .alert("Couldn't do that", isPresented: .constant(error != nil)) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
    }

    private func row(_ s: Session) -> some View {
        NavigationLink(value: Route.session(serverID: server.id, name: s.name)) {
            SessionRow(session: s, report: server.reports[s.name])
        }
        .swipeActions {
            Button("Delete", role: .destructive) { destroying = s }
        }
    }

    private func act(_ work: @escaping () async throws -> Void) {
        Task {
            do { try await work() } catch { self.error = error.localizedDescription }
        }
    }
}

/// A project's name and where it clones from, with its menu.
private struct ProjectHeader: View {
    let project: Project
    let forget: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(project.name).font(.subheadline.weight(.semibold)).textCase(nil)
                Text(project.repo).font(.caption2.monospaced()).textCase(nil).lineLimit(1).truncationMode(.middle)
            }
            Spacer()
            Menu {
                Button("Forget Project", systemImage: "folder.badge.minus", role: .destructive, action: forget)
            } label: {
                Image(systemName: "ellipsis.circle")
            }
        }
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
            if let failure = session.failure, ["failed", "dead"].contains(session.state) {
                Text(failure)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .lineLimit(2)
            } else if let prompt = report?.state == "waiting" ? (report?.prompt ?? report?.detail) : nil {
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
