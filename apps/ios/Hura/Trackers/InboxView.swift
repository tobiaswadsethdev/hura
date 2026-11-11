import HuraKit
import SwiftUI

/// What is yours on every tracker: tickets assigned to you or created by you
/// that aren't done, a section for each of a Jira tracker's JQL filters, and
/// open pull requests you opened or that wait on you.
/// The desktop's tickets sidebar, as a tab: tickets or pull requests, one at
/// a time, so neither is scrolled past to reach the other.
struct InboxView: View {
    @Environment(TrackersModel.self) private var trackers
    @Environment(AppModel.self) private var model
    @State private var settings = false
    @State private var starting: WorkOrigin?
    @AppStorage("inboxShowing") private var showing: Showing = .tickets

    enum Showing: String, CaseIterable {
        case tickets, pulls
    }

    var body: some View {
        NavigationStack {
            if trackers.signedIn {
                list
            } else {
                SignInForTrackers()
                    .navigationTitle("Inbox")
            }
        }
    }

    private var list: some View {
        List {
            if let inbox = trackers.inbox {
                ForEach(inbox.warnings, id: \.self) { w in
                    Label(w, glyph: .warning).font(.caption).foregroundStyle(.orange)
                }
                switch showing {
                case .tickets:
                    tickets(inbox, title: "Assigned to me") { $0.relation == "assigned" }
                    ForEach(filterNames, id: \.self) { name in
                        tickets(inbox, title: name) { $0.filters.contains(name) }
                    }
                    // Only with something in it: hidden on one tracker, the
                    // group stayed, empty, while another still had it on.
                    if trackers.trackers.contains(where: { !$0.source.hideCreated }),
                       inbox.tasks.contains(where: { $0.relation == "created" }) {
                        tickets(inbox, title: "Created by me") { $0.relation == "created" }
                    }
                case .pulls:
                    pulls(inbox.pulls.filter(\.reviewRequested), title: "Waiting on my review")
                    pulls(inbox.pulls.filter { !$0.reviewRequested }, title: "My pull requests")
                }
            }
            if let e = trackers.lastError {
                Text(e).font(.caption).foregroundStyle(.red)
            }
        }
        .overlay {
            if trackers.trackers.isEmpty {
                ContentUnavailableView {
                    Label("No trackers", glyph: .tracker)
                } description: {
                    Text("Add GitHub, Azure DevOps, Jira or Fizzy to see your tickets and pull requests here, on every device signed in to your account.")
                } actions: {
                    Button("Add a Tracker") { settings = true }.buttonStyle(.borderedProminent)
                }
            } else if trackers.inbox == nil {
                ProgressView()
            } else if let inbox = trackers.inbox {
                switch showing {
                case .tickets where inbox.tasks.isEmpty:
                    ContentUnavailableView("No tickets", image: Glyph.passing.name,
                                           description: Text("Read from \(inbox.read.joined(separator: ", "))."))
                case .pulls where inbox.pulls.isEmpty:
                    ContentUnavailableView("No pull requests", image: Glyph.passing.name,
                                           description: Text(inbox.pullsRead.isEmpty
                                               ? "Pull requests come from GitHub and Azure DevOps trackers."
                                               : "Read from \(inbox.pullsRead.joined(separator: ", "))."))
                default:
                    EmptyView()
                }
            }
        }
        .navigationTitle("Inbox")
        // The switch is the bar's title, so the rows scroll under the
        // bar's own material rather than showing through a control
        // pinned beneath it; the tab still says Inbox.
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await trackers.refresh() }
        .toolbar {
            if !trackers.trackers.isEmpty {
                ToolbarItem(placement: .principal) {
                    Picker("Showing", selection: $showing) {
                        Text(label("Tickets", trackers.inbox?.tasks.count)).tag(Showing.tickets)
                        Text(label("Pull Requests", trackers.inbox?.pulls.count)).tag(Showing.pulls)
                    }
                    .pickerStyle(.segmented)
                }
            }
            ToolbarItem(placement: .primaryAction) {
                Button { settings = true } label: { Label("Trackers", glyph: .settings) }
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

    /// A segment's name, and how many it holds once that is known.
    private func label(_ name: String, _ count: Int?) -> String {
        count.map { "\(name) (\($0))" } ?? name
    }

    /// Every tracker's filters, in the order they are listed; two of one name
    /// are one section, as on the desktop.
    private var filterNames: [String] {
        var seen = Set<String>()
        return trackers.trackers.flatMap { $0.source.jqlFilters.map(\.name) }.filter { seen.insert($0).inserted }
    }

    @ViewBuilder
    private func tickets(_ inbox: Inbox, title: String, holds: (TrackerTask) -> Bool) -> some View {
        let rows = newestFirst(inbox.tasks.filter(holds))
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows) { t in
                    let on = ticketSession(t)
                    NavigationLink(value: t) { TicketRow(task: t, session: on?.name) }
                        .swipeActions {
                            Button("Start") { starting = .ticket(t) }.tint(.green)
                        }
                        .contextMenu { rowMenu(on, url: t.url, copy: t.key) }
                }
            }
        }
    }

    @ViewBuilder
    private func pulls(_ rows: [PullRequestSummary], title: String) -> some View {
        if !rows.isEmpty {
            Section(title) {
                ForEach(newestFirst(rows)) { p in
                    let on = pullSession(p)
                    NavigationLink(value: p) { PullRow(pull: p, session: on?.name) }
                        .swipeActions {
                            Button("Review") { starting = .pull(p, review: true) }.tint(.blue)
                            Button("Work") { starting = .pull(p, review: false) }.tint(.green)
                        }
                        .contextMenu { rowMenu(on, url: p.url, copy: p.branch) }
                }
            }
        }
    }

    /// Going to the worktree already on it, and the rest of the desktop's
    /// row menu.
    @ViewBuilder
    private func rowMenu(_ on: (server: String, name: String)?, url: String, copy: String) -> some View {
        if let on {
            Button("Go to \(on.name)", glyph: .goTo) {
                model.pendingRoute = [.server(on.server), .session(serverID: on.server, name: on.name)]
            }
        }
        Button("Open in the Browser", systemImage: "safari") { if let u = URL(string: url) { UIApplication.shared.open(u) } }
        Button("Copy \(copy)", systemImage: "doc.on.doc") { UIPasteboard.general.string = copy }
        Button("Copy Link", systemImage: "link") { UIPasteboard.general.string = url }
    }

    private func newestFirst<T>(_ rows: [T]) -> [T] where T: Identifiable {
        rows.sorted { updated($0) > updated($1) }
    }

    private func updated(_ row: some Identifiable) -> String {
        (row as? TrackerTask)?.updated ?? (row as? PullRequestSummary)?.updated ?? ""
    }

    /// A worktree already on this ticket, on any server.
    private func ticketSession(_ t: TrackerTask) -> (server: String, name: String)? {
        for server in model.servers {
            let live = server.sessions.filter { $0.state != "dead" }
            if let s = live.first(where: { $0.ticket?.tracker == t.tracker && $0.ticket?.key == t.key }) { return (server.id, s.name) }
            if t.key.wholeMatch(of: /[A-Z][A-Z0-9]+-\d+/) != nil,
               let s = live.first(where: {
                   let last = $0.workBranch.split(separator: "/").last.map(String.init) ?? ""
                   return last == t.key || last.hasPrefix("\(t.key)-")
               }) { return (server.id, s.name) }
        }
        return nil
    }

    /// A worktree already on this pull request: started from it, or on its
    /// branch in its repository.
    private func pullSession(_ p: PullRequestSummary) -> (server: String, name: String)? {
        for server in model.servers {
            let live = server.sessions.filter { $0.state != "dead" }
            if let s = live.first(where: { $0.pull?.tracker == p.tracker && $0.pull?.repo == p.repo && $0.pull?.id == p.id })
                ?? live.first(where: { $0.workBranch == p.branch && bareRepo($0.repo) == bareRepo(p.repoURL) }) {
                return (server.id, s.name)
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
                Text(task.key).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Text(task.status).font(.caption2).padding(.horizontal, 6).padding(.vertical, 1)
                    .background(Color.secondary.opacity(0.15), in: Capsule())
                Spacer()
                if let session {
                    Label(session, glyph: .branch).font(.caption2).foregroundStyle(.green).lineLimit(1)
                }
            }
            Text(task.title).font(.callout).lineLimit(2)
            HStack(spacing: 8) {
                Text(task.tracker)
                if let n = task.comments, n > 0 {
                    Label("\(n)", glyph: task.lastCommentMine ? .comment : .commentNew)
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
                Text(pullName(pull)).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                if pull.draft { Text("draft").font(.caption2).foregroundStyle(.secondary) }
                if pull.conflicts { Image(glyph: .warning).font(.caption2).foregroundStyle(.orange) }
                ChecksMark(checks: pull.checks)
                Spacer()
                if let session {
                    Label(session, glyph: .branch).font(.caption2).foregroundStyle(.green).lineLimit(1)
                }
            }
            Text(pull.title).font(.callout).lineLimit(2)
            HStack(spacing: 8) {
                Text(pull.author)
                Text("\(pull.branch) → \(pull.base)").font(.caption2).lineLimit(1)
                if let r = pull.review { Text(reviewWord(r)) }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }

}

/// A pull request's checks as one mark, as the desktop's `ChecksMark`; none
/// where the forge didn't say.
struct ChecksMark: View {
    let checks: String?

    var body: some View {
        switch checks {
        case "passing": Image(glyph: .passing).font(.caption2).foregroundStyle(.green).accessibilityLabel("Checks passed")
        case "failing": Image(glyph: .failing).font(.caption2).foregroundStyle(.red).accessibilityLabel("Checks failed")
        case "pending": Image(glyph: .pending).font(.caption2).foregroundStyle(.yellow).accessibilityLabel("Checks running")
        default: EmptyView()
        }
    }
}

func reviewWord(_ r: String) -> String {
    switch r {
    case "approved": "approved"
    case "changes-requested": "changes requested"
    default: "review pending"
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
                                    Text(p.repo).font(.caption).foregroundStyle(.secondary).lineLimit(1)
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

/// Signed out, in place of the inbox and the trackers: they are kept in a
/// hura account, so there are none without one.
struct SignInForTrackers: View {
    @Environment(TrackersModel.self) private var trackers
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ContentUnavailableView {
            Label("Sign in for trackers", glyph: .account)
        } description: {
            Text("Your tickets and pull requests from GitHub, Azure DevOps, Jira and Fizzy come through your hura account, which keeps the trackers and reads them all day, so they are the same on every device and you hear about a change on time.")
        } actions: {
            Button("Sign In") {
                trackers.showAccount = true
                dismiss()
            }
            .buttonStyle(.borderedProminent)
        }
    }
}
