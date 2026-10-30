import HuraKit
import SwiftUI

/// What is yours on every tracker: tickets assigned to you or created by you
/// that aren't done, and open pull requests you opened or that wait on you.
/// The desktop's tickets sidebar, as a tab.
struct InboxView: View {
    @Environment(TrackersModel.self) private var trackers
    @Environment(AppModel.self) private var model
    @State private var settings = false
    @State private var starting: WorkOrigin?

    var body: some View {
        NavigationStack {
            List {
                if let inbox = trackers.inbox {
                    ForEach(inbox.warnings, id: \.self) { w in
                        Label(w, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                    }
                    tickets(inbox, relation: "assigned", title: "Assigned to me")
                    tickets(inbox, relation: "created", title: "Created by me")
                    pulls(inbox.pulls.filter(\.reviewRequested), title: "Waiting on my review")
                    pulls(inbox.pulls.filter { !$0.reviewRequested }, title: "My pull requests")
                }
                if let e = trackers.lastError {
                    Text(e).font(.caption).foregroundStyle(.red)
                }
            }
            .overlay {
                if trackers.trackers.isEmpty {
                    ContentUnavailableView {
                        Label("No trackers", systemImage: "tray")
                    } description: {
                        Text("Add GitHub, Azure DevOps or Jira to see your tickets and pull requests here. The token stays on this phone.")
                    } actions: {
                        Button("Add a Tracker") { settings = true }.buttonStyle(.borderedProminent)
                    }
                } else if trackers.inbox == nil {
                    ProgressView()
                } else if let inbox = trackers.inbox, inbox.tasks.isEmpty, inbox.pulls.isEmpty {
                    ContentUnavailableView("Nothing waiting", systemImage: "checkmark.circle",
                                           description: Text("Read from \(inbox.read.joined(separator: ", "))."))
                }
            }
            .navigationTitle("Inbox")
            .refreshable { await trackers.refresh() }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { settings = true } label: { Label("Trackers", systemImage: "gearshape") }
                }
                if trackers.reading {
                    ToolbarItem(placement: .status) { ProgressView() }
                }
            }
            .navigationDestination(for: TrackerTask.self) { TicketView(task: $0, starting: $starting) }
            .navigationDestination(for: PullRequestSummary.self) { PullView(pull: $0, starting: $starting) }
            .sheet(isPresented: $settings) { TrackersView() }
            .sheet(item: $starting) { origin in StartWorkView(origin: origin) }
        }
    }

    @ViewBuilder
    private func tickets(_ inbox: Inbox, relation: String, title: String) -> some View {
        let rows = newestFirst(inbox.tasks.filter { $0.relation == relation })
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows) { t in
                    NavigationLink(value: t) { TicketRow(task: t, session: ticketSession(t)) }
                        .swipeActions {
                            Button("Start") { starting = .ticket(t) }.tint(.green)
                        }
                }
            }
        }
    }

    @ViewBuilder
    private func pulls(_ rows: [PullRequestSummary], title: String) -> some View {
        if !rows.isEmpty {
            Section(title) {
                ForEach(newestFirst(rows)) { p in
                    NavigationLink(value: p) { PullRow(pull: p, session: pullSession(p)) }
                        .swipeActions {
                            Button("Review") { starting = .pull(p, review: true) }.tint(.blue)
                            Button("Work") { starting = .pull(p, review: false) }.tint(.green)
                        }
                }
            }
        }
    }

    private func newestFirst<T>(_ rows: [T]) -> [T] where T: Identifiable {
        rows.sorted { updated($0) > updated($1) }
    }

    private func updated(_ row: some Identifiable) -> String {
        (row as? TrackerTask)?.updated ?? (row as? PullRequestSummary)?.updated ?? ""
    }

    /// A worktree already on this ticket, on any server.
    private func ticketSession(_ t: TrackerTask) -> String? {
        for server in model.servers {
            let live = server.sessions.filter { $0.state != "dead" }
            if let s = live.first(where: { $0.ticket?.tracker == t.tracker && $0.ticket?.key == t.key }) { return s.name }
            if t.key.wholeMatch(of: /[A-Z][A-Z0-9]+-\d+/) != nil,
               let s = live.first(where: {
                   let last = $0.workBranch.split(separator: "/").last.map(String.init) ?? ""
                   return last == t.key || last.hasPrefix("\(t.key)-")
               }) { return s.name }
        }
        return nil
    }

    /// A worktree already on this pull request: started from it, or on its
    /// branch in its repository.
    private func pullSession(_ p: PullRequestSummary) -> String? {
        for server in model.servers {
            let live = server.sessions.filter { $0.state != "dead" }
            if let s = live.first(where: { $0.pull?.tracker == p.tracker && $0.pull?.repo == p.repo && $0.pull?.id == p.id })
                ?? live.first(where: { $0.workBranch == p.branch && bareRepo($0.repo) == bareRepo(p.repoURL) }) {
                return s.name
            }
        }
        return nil
    }
}

struct TicketRow: View {
    let task: TrackerTask
    let session: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(task.key).font(.caption.monospaced().weight(.semibold)).foregroundStyle(.secondary)
                Text(task.status).font(.caption2).padding(.horizontal, 6).padding(.vertical, 1)
                    .background(Color.secondary.opacity(0.15), in: Capsule())
                Spacer()
                if let session {
                    Label(session, systemImage: "arrow.triangle.branch").font(.caption2).foregroundStyle(.green).lineLimit(1)
                }
            }
            Text(task.title).font(.callout).lineLimit(2)
            HStack(spacing: 8) {
                Text(task.tracker)
                if let n = task.comments, n > 0 {
                    Label("\(n)", systemImage: task.lastCommentMine ? "bubble.left" : "bubble.left.fill")
                }
                if let who = task.lastCommenter, !task.lastCommentMine { Text("last: \(who)") }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }
}

struct PullRow: View {
    let pull: PullRequestSummary
    let session: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(pullName(pull)).font(.caption.monospaced().weight(.semibold)).foregroundStyle(.secondary)
                if pull.draft { Text("draft").font(.caption2).foregroundStyle(.secondary) }
                if pull.conflicts { Image(systemName: "exclamationmark.triangle.fill").font(.caption2).foregroundStyle(.orange) }
                checks
                Spacer()
                if let session {
                    Label(session, systemImage: "arrow.triangle.branch").font(.caption2).foregroundStyle(.green).lineLimit(1)
                }
            }
            Text(pull.title).font(.callout).lineLimit(2)
            HStack(spacing: 8) {
                Text(pull.author)
                Text("\(pull.branch) → \(pull.base)").font(.caption2.monospaced()).lineLimit(1)
                if let r = pull.review { Text(reviewWord(r)) }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var checks: some View {
        switch pull.checks {
        case "passing": Image(systemName: "checkmark.circle.fill").font(.caption2).foregroundStyle(.green)
        case "failing": Image(systemName: "xmark.circle.fill").font(.caption2).foregroundStyle(.red)
        case "pending": Image(systemName: "clock.fill").font(.caption2).foregroundStyle(.yellow)
        default: EmptyView()
        }
    }

    private func reviewWord(_ r: String) -> String {
        switch r {
        case "approved": "approved"
        case "changes-requested": "changes requested"
        default: "review pending"
        }
    }
}

extension WorkOrigin: Identifiable {
    var id: String {
        switch self {
        case let .ticket(t): "t:\(t.tracker):\(t.key)"
        case let .pull(p, review): "p:\(p.tracker):\(p.repo):\(p.id):\(review)"
        }
    }
}

/// Where a ticket's or a pull request's worktree starts: a server, and one of
/// its projects. A pull request offers only the clones of its repository.
struct StartWorkView: View {
    let origin: WorkOrigin
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var chosen: Chosen?

    struct Chosen: Identifiable {
        var server: Server
        var project: Project
        var id: String { "\(server.id):\(project.name)" }
    }

    private var repoURL: String? {
        if case let .pull(p, _) = origin { p.repoURL } else { nil }
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.servers) { server in
                    let projects = server.projects.filter { p in repoURL.map { bareRepo(p.repo) == bareRepo($0) } ?? true }
                    Section(server.record.name) {
                        if server.state != .online {
                            Text(server.state.label).foregroundStyle(.secondary)
                        } else if projects.isEmpty {
                            Text(repoURL == nil ? "No projects" : "No project clones this repository").foregroundStyle(.secondary)
                        }
                        ForEach(projects) { p in
                            Button {
                                chosen = Chosen(server: server, project: p)
                            } label: {
                                VStack(alignment: .leading) {
                                    Text(p.name)
                                    Text(p.repo).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            .tint(.primary)
                            .disabled(server.state != .online)
                        }
                    }
                }
            }
            .sheet(item: $chosen) { c in
                NewWorktreeView(server: c.server, project: c.project, origin: origin) { name in
                    dismiss()
                    model.pendingRoute = [.server(c.server.id), .session(serverID: c.server.id, name: name)]
                }
            }
            .navigationTitle("Start Where?")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
    }
}
