import HuraKit
import SwiftUI

/// A server in the list: a card on the canvas, its name and how it stands,
/// with what needs you in the colour that says so.
struct ServerRow: View {
    let server: Server

    var body: some View {
        HStack(spacing: 12) {
            Image(glyph: .server)
                .font(.title3)
                .foregroundStyle(.dim)
                .frame(width: 38, height: 38)
                .background(Color.surface2, in: RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(server.record.name).font(.headline).foregroundStyle(.ink)
                    ServerDot(state: server.state)
                }
                summary
                    .font(.subheadline)
                    .foregroundStyle(.dim)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
    }

    private var summary: Text {
        guard server.state == .online else { return Text(server.state.label) }
        if let problem = server.problem { return Text(problem).foregroundStyle(Color.warn) }
        let live = server.sessions.filter(\.isLive)
        var parts = [Text("\(live.count) session\(live.count == 1 ? "" : "s")")]
        let waiting = server.waiting.count
        if waiting > 0 { parts.append(Text("\(waiting) need\(waiting == 1 ? "s" : "") you").foregroundStyle(Color.attention)) }
        let working = live.filter { server.reports[$0.name]?.state == "running" }.count
        if working > 0 { parts.append(Text("\(working) working").foregroundStyle(Color.working)) }
        return parts.dropFirst().reduce(parts[0]) { Text("\($0) · \($1)") }
    }
}

struct ServerDot: View {
    let state: Server.State

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
            .accessibilityLabel(state.label)
    }

    private var color: Color {
        switch state {
        case .online: .ok
        case .connecting: .working
        case .offline: .dimmer
        }
    }
}

/// A hura server: its projects as T3 Code's folders with their worktrees as
/// cards, then sessions that belong to none. Above them the worktrees pinned
/// to the top, and below them the shelves of what is put off and what is
/// done, as the desktop's sidebar has it (`Tree.tsx`).
struct ServerView: View {
    let server: Server
    @Environment(AppModel.self) private var model
    @Environment(TrackersModel.self) private var trackers
    @State private var addingProject = false
    @State private var creatingIn: Project?
    @State private var destroying: Session?
    @State private var renaming: Session?
    @State private var newTitle = ""
    @State private var forgetting: Project?
    /// A project whose new worktrees' defaults are being set.
    @State private var defaulting: Project?
    @State private var showingUsage = false
    @State private var screen: ServerScreen?
    @State private var openPull: PullRequestSummary?
    @State private var error: String?
    /// Whether the list is being put in order: every worktree gets a handle
    /// to drag it by, and the projects are listed at the top to be dragged
    /// the same way, since a folder has nothing to hold.
    @State private var editMode: EditMode = .inactive
    /// Pinned, snoozed and settled worktrees: this phone's, see `Triage`.
    @State private var triage: TriageBook
    /// The projects folded shut, by name. Kept as the folded ones rather than
    /// the open ones so a project that appears -- made here, or on the
    /// desktop -- comes in open: the reason it appeared is usually that
    /// somebody asked for it.
    @State private var folded: Set<String>
    /// Which of the shelves at the foot are open. Shut until opened.
    @State private var openShelves: Set<Shelf> = []
    /// The time the list was placed at, moved on when a snooze runs out.
    @State private var now = Date()
    @State private var query = ""

    init(server: Server) {
        self.server = server
        _triage = State(initialValue: TriageBook.load(server: server.id))
        _folded = State(initialValue: Set(UserDefaults.standard.stringArray(forKey: Self.foldedKey(server.id)) ?? []))
    }

    private static func foldedKey(_ server: String) -> String { "hura.folded.\(server)" }

    enum Shelf: Hashable { case snoozed, settled }

    var body: some View {
        List {
            if server.state != .online {
                HStack(spacing: 8) {
                    ServerDot(state: server.state)
                    Text(server.state.label).font(.subheadline).foregroundStyle(.dim)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .card()
                .canvasRow(.card)
            } else if let problem = server.problem {
                Label(problem, glyph: .warning)
                    .font(.subheadline)
                    .foregroundStyle(Color.warn)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .card()
                    .canvasRow(.card)
            }
            if editMode.isEditing, server.projects.count > 1 {
                Section {
                    ShelfHead(label: "Projects").canvasRow(.head)
                    ForEach(server.projects) { p in
                        HStack(spacing: 10) {
                            ProjectMonogram(name: p.name)
                            Text(p.name).font(.subheadline.weight(.semibold)).foregroundStyle(.ink)
                        }
                        .canvasRow(.folder)
                    }
                    .onMove { from, to in
                        var names = server.projects.map(\.name)
                        names.move(fromOffsets: from, toOffset: to)
                        act { try await server.reorderProjects(names) }
                    }
                }
            }
            if !searching, !pinned.isEmpty {
                Section {
                    ShelfHead(label: "Pinned").canvasRow(.head)
                    ForEach(pinned) { card($0, showingProject: true) }
                }
            }
            ForEach(shownProjects) { project in
                folder(project)
            }
            unfiledFolder
            if !searching {
                shelf(.snoozed, "Snoozed", snoozed)
                shelf(.settled, "Settled", settled)
            }
            if searching, nothingFound {
                ContentUnavailableView.search(text: query).canvasRow(.card)
            }
            if server.state == .online, server.projects.isEmpty, server.sessions.isEmpty {
                ContentUnavailableView {
                    Label("No projects", glyph: .folder)
                } description: {
                    Text("A project is a repository on this server that you start worktrees from.")
                } actions: {
                    Button("New Project") { addingProject = true }
                        .buttonStyle(.borderedProminent)
                }
                .canvasRow(.card)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.canvas)
        .environment(\.editMode, $editMode)
        .navigationTitle(server.record.name)
        // Under the title, out of sight until the list is pulled down, as
        // Mail has it: the list is what the screen is for.
        .searchable(text: $query, placement: .navigationBarDrawer, prompt: "Worktrees")
        .refreshable { await server.refresh() }
        // A snooze runs out while nobody touches the list, so the list is
        // placed again when the soonest one does.
        .task(id: triage.nextWake(after: now)) {
            guard let wake = triage.nextWake(after: now) else { return }
            try? await Task.sleep(for: .seconds(max(1, wake.timeIntervalSinceNow)))
            if !Task.isCancelled { now = Date() }
        }
        .onChange(of: triage) { _, book in book.save(server: server.id) }
        .onChange(of: folded) { _, names in
            UserDefaults.standard.set(names.sorted(), forKey: Self.foldedKey(server.id))
        }
        .toolbar {
            if editMode.isEditing {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { withAnimation { editMode = .inactive } }
                }
            } else {
                toolbarItems
            }
        }
        .navigationDestination(item: $screen) { screen in
            switch screen {
            case .integrations: IntegrationsView(server: server)
            case .settings: ServerSettingsView(server: server)
            case .about: AboutView(server: server)
            }
        }
        .navigationDestination(item: $openPull) { LinkedPullView(pull: $0) }
        .modifier(projectScreens)
        .sheet(isPresented: $addingProject) {
            NewProjectView(server: server)
        }
        .sheet(item: $creatingIn) { project in
            NewWorktreeView(server: server, project: project) { name in
                model.pendingRoute = [.server(server.id), .session(serverID: server.id, name: name)]
            }
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
        .alert("Couldn't delete it", isPresented: .constant(server.deleteFailure != nil)) {
            Button("OK") { server.deleteFailure = nil }
        } message: {
            Text(server.deleteFailure ?? "")
        }
    }

    /// A project's defaults and the server's usage, pushed over the list. A
    /// modifier of their own: on the list's chain they were more than the
    /// compiler could type in time.
    private var projectScreens: ProjectScreens {
        ProjectScreens(server: server, defaulting: $defaulting, showingUsage: $showingUsage) { name in
            model.pendingRoute = [.server(server.id), .session(serverID: server.id, name: name)]
        }
    }

    /// Adding things, and the server's own screens. Away while the list is
    /// being put in order, when the one thing to do is say it is done.
    @ToolbarContentBuilder
    private var toolbarItems: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Menu {
                Button { addingProject = true } label: { Label("New Project", glyph: .newProject) }
                if !server.projects.isEmpty {
                    Menu {
                        ForEach(server.projects) { p in
                            Button(p.name) { creatingIn = p }
                        }
                    } label: {
                        Label("New Worktree", glyph: .branch)
                    }
                }
            } label: {
                Label("Add", systemImage: "plus")
            }
            .disabled(server.state != .online)
        }
        ToolbarItem(placement: .primaryAction) {
            Menu {
                Button("Integrations", glyph: .integrations) { screen = .integrations }
                Button("Settings", glyph: .settings) { screen = .settings }
                Button("Usage", glyph: .events) { showingUsage = true }
                Button("About", glyph: .info) { screen = .about }
                Divider()
                Button("Reorder", systemImage: "arrow.up.arrow.down") {
                    withAnimation { editMode = .active }
                }
                .disabled(server.state != .online)
            } label: {
                Label("Server", systemImage: "ellipsis.circle")
            }
        }
    }

    // MARK: Where each worktree goes

    private var searching: Bool { !query.trimmingCharacters(in: .whitespaces).isEmpty }

    /// Whether a worktree is what is being looked for: by its title, its
    /// name, its branch or its project.
    private func matches(_ s: Session) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces)
        return [s.title, s.name, s.workBranch, s.project].contains { $0?.localizedStandardContains(q) ?? false }
    }

    private func matches(_ w: QueuedSession) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces)
        return [w.new.title, w.new.name, w.new.project].contains { $0?.localizedStandardContains(q) ?? false }
    }

    private func place(_ s: Session) -> Triage.Place {
        triage[s.name].place(needsYou: liveState(s, server.reports[s.name]) == "waiting", now: now)
    }

    /// Every worktree, in the order the folders draw them, for the shelves.
    private var everyone: [Session] {
        server.projects.flatMap { server.worktrees(in: $0) } + server.unfiled
    }

    private var pinned: [Session] { everyone.filter { place($0) == .pinned } }
    private var snoozed: [Session] { everyone.filter { place($0) == .snoozed } }
    private var settled: [Session] { everyone.filter { place($0) == .settled } }

    /// A folder's worktrees: the ones in it that are on no shelf; or, while
    /// searching, every one that matches wherever it is kept, since what is
    /// being looked for may well be settled.
    private func shown(_ rows: [Session]) -> [Session] {
        searching ? rows.filter(matches) : rows.filter { place($0) == .active }
    }

    private func shown(_ rows: [QueuedSession]) -> [QueuedSession] {
        searching ? rows.filter(matches) : rows
    }

    /// The projects to draw: all of them, or while searching the ones with
    /// something that matches.
    private var shownProjects: [Project] {
        guard searching else { return server.projects }
        return server.projects.filter {
            !shown(server.worktrees(in: $0)).isEmpty || !shown(server.waiting(in: $0)).isEmpty
        }
    }

    private var nothingFound: Bool {
        shownProjects.isEmpty && shown(server.unfiled).isEmpty && shown(server.waitingUnfiled).isEmpty
    }

    // MARK: Folders and shelves

    private func folder(_ project: Project) -> some View {
        let worktrees = shown(server.worktrees(in: project))
        let waiting = shown(server.waiting(in: project))
        let open = searching || !folded.contains(project.name)
        return Section {
            ProjectHeader(project: project, folded: !open, count: worktrees.count + waiting.count,
                          canCreate: server.state == .online,
                          toggle: { fold(project.name) },
                          create: { creatingIn = project },
                          defaults: { defaulting = project },
                          forget: { forgetting = project })
                .canvasRow(.folder)
            if open {
                ForEach(worktrees) { card($0) }
                    .onMove { reorder(worktrees, from: $0, to: $1) }
                ForEach(waiting) { waitingRow($0) }
            }
        }
    }

    /// Sessions that belong to no project this server knows: started from a
    /// terminal, or from a project since forgotten.
    @ViewBuilder
    private var unfiledFolder: some View {
        let unfiled = shown(server.unfiled)
        let waiting = shown(server.waitingUnfiled)
        if !unfiled.isEmpty || !waiting.isEmpty {
            Section {
                ShelfHead(label: server.projects.isEmpty ? "Sessions" : "Not in a project").canvasRow(.head)
                ForEach(unfiled) { card($0) }
                    .onMove { reorder(unfiled, from: $0, to: $1) }
                ForEach(waiting) { waitingRow($0) }
            }
        }
    }

    /// A shelf at the foot: its name and how many, folded until opened. A
    /// row on it opens its worktree; a swipe or a long press takes it off.
    @ViewBuilder
    private func shelf(_ which: Shelf, _ label: String, _ rows: [Session]) -> some View {
        if !rows.isEmpty {
            let open = openShelves.contains(which)
            Section {
                ShelfHead(label: label, count: rows.count, open: open) {
                    withAnimation {
                        if open { openShelves.remove(which) } else { openShelves.insert(which) }
                    }
                }
                .canvasRow(.head)
                if open {
                    ForEach(rows) { s in
                        let t = triage[s.name]
                        let undo = which == .snoozed ? "Wake It" : "Unsettle"
                        let undoing: Triage.Change = which == .snoozed ? .wake : .unsettle
                        ShelfRow(session: s, note: which == .snoozed
                                 ? "back \(startsAt(t.snoozedUntil ?? now))"
                                 : age(Int64((t.settled ?? now).timeIntervalSince1970)))
                            .background { NavigationLink(value: Route.session(serverID: server.id, name: s.name)) { EmptyView() }.opacity(0) }
                            .canvasRow(.slim)
                            .swipeActions(edge: .leading) {
                                Button(undo, systemImage: "arrow.uturn.backward") { change(s.name, undoing) }
                                    .tint(.accentColor)
                            }
                            .contextMenu {
                                Button(undo, systemImage: "arrow.uturn.backward") { change(s.name, undoing) }
                            }
                    }
                }
            }
        }
    }

    private func fold(_ project: String) {
        withAnimation {
            if folded.contains(project) { folded.remove(project) } else { folded.insert(project) }
        }
    }

    private func change(_ name: String, _ change: Triage.Change) {
        withAnimation {
            now = Date()
            triage.apply(change, to: name)
        }
    }

    /// One group's worktrees after a drag, sent as the order to keep.
    private func reorder(_ rows: [Session], from: IndexSet, to: Int) {
        var names = rows.map(\.name)
        names.move(fromOffsets: from, toOffset: to)
        act { try await server.reorderSessions(names) }
    }

    // MARK: Rows

    private func card(_ s: Session, showingProject: Bool = false) -> some View {
        let pull = sessionPull(s, in: trackers.inbox?.pulls ?? [])
        let t = triage[s.name]
        return SessionRow(session: s, report: server.reports[s.name], pull: pull,
                          waiting: server.queue.messages(for: s.name),
                          project: showingProject ? s.project : nil)
            // The link behind the card rather than around it, so the list
            // draws no chevron: T3 Code's cards have none, and the whole card
            // is still what is tapped.
            .background { NavigationLink(value: Route.session(serverID: server.id, name: s.name)) { EmptyView() }.opacity(0) }
            .canvasRow(.card)
            .swipeActions(edge: .leading) {
                if t.pinned {
                    Button("Unpin", systemImage: "pin.slash") { change(s.name, .unpin) }
                        .tint(.accentColor)
                } else {
                    Button("Pin", systemImage: "pin") { change(s.name, .pin) }
                        .tint(.accentColor)
                }
                Button("Settle", systemImage: "checkmark.circle") { change(s.name, .settle(Date())) }
                    .tint(Color.ok)
            }
            .swipeActions(edge: .trailing) {
                // Red by tint rather than `role: .destructive`: a destructive
                // swipe action takes the row out of the list at once, and the
                // dialog hung off it goes with it before it can be answered.
                Button("Delete", systemImage: "trash") { destroying = s }
                    .tint(Color.bad)
            }
            // On the row, not the list, so it opens over the row it is about
            // rather than at the top of the screen.
            .contextMenu {
                if let pull {
                    Button("Open \(pullName(pull))", glyph: .pullRequest) { openPull = pull }
                }
                Button("Rename", systemImage: "pencil") {
                    newTitle = s.title ?? ""
                    renaming = s
                }
                Divider()
                if t.pinned {
                    Button("Unpin", systemImage: "pin.slash") { change(s.name, .unpin) }
                } else {
                    Button("Pin to the Top", systemImage: "pin") { change(s.name, .pin) }
                }
                if t.snoozedUntil.map({ $0 > now }) ?? false {
                    Button("Wake It", systemImage: "alarm") { change(s.name, .wake) }
                } else {
                    Menu {
                        // The time is worked out when one is picked, not when
                        // the menu was drawn: an hour is from now.
                        ForEach(Triage.snoozeChoices().map(\.label), id: \.self) { label in
                            Button(label) {
                                guard let until = Triage.snoozeChoices().first(where: { $0.label == label })?.until else { return }
                                change(s.name, .snooze(until: until))
                            }
                        }
                    } label: {
                        Label("Snooze", systemImage: "alarm")
                    }
                }
                if t.settled != nil {
                    Button("Unsettle", systemImage: "arrow.uturn.backward") { change(s.name, .unsettle) }
                } else {
                    Button("Settle", systemImage: "checkmark.circle") { change(s.name, .settle(Date())) }
                }
            }
            .alert("Rename", isPresented: Binding(
                get: { renaming?.name == s.name },
                set: { if !$0 { renaming = nil } }
            )) {
                TextField(s.name, text: $newTitle)
                Button("Cancel", role: .cancel) {}
                Button("Rename") {
                    let title = newTitle
                    act { try await server.retitle(s.name, to: title) }
                }
            } message: {
                Text("Leave it empty to show \(s.name).")
            }
            .confirmationDialog("Delete \(s.label)?", isPresented: Binding(
                get: { destroying?.name == s.name },
                set: { if !$0 { destroying = nil } }
            ), titleVisibility: .visible) {
                Button("Delete Worktree", role: .destructive) {
                    triage.forget(s.name)
                    server.destroy(s.name)
                }
            } message: {
                Text(s.isWorktree
                     ? "Its worktree on the server is deleted with its changes. A branch with commits on it is kept."
                     : "Its sandbox is deleted, with everything in it that hasn't been pushed.")
            }
    }

    /// A session on the server's queue, where it will appear. Nothing to
    /// open yet: starting it now or taking it off is a swipe or a long press.
    private func waitingRow(_ w: QueuedSession) -> some View {
        QueuedRow(waiting: w, place: server.queue.place(of: w), limit: server.queue.limit,
                  serverOffset: server.queue.utcOffset ?? 0)
            .canvasRow(.slim)
            .swipeActions {
                if !w.started {
                    Button("Remove") { act { try await server.unqueue(w.id) } }
                        .tint(Color.bad)
                    Button("Start") { act { try await server.startQueued(w.id) } }
                        .tint(.accentColor)
                }
            }
            .contextMenu {
                if !w.started {
                    Button("Start Now", systemImage: "play") { act { try await server.startQueued(w.id) } }
                    Button("Remove from Queue", systemImage: "trash", role: .destructive) {
                        act { try await server.unqueue(w.id) }
                    }
                }
            }
    }

    private func act(_ work: @escaping () async throws -> Void) {
        Task {
            do { try await work() } catch { self.error = error.userMessage }
        }
    }
}

/// When something comes back or starts, as briefly as is unambiguous, as the
/// desktop's `startsAt` says it: a time alone today, a weekday within the
/// week, and a date beyond that.
func startsAt(_ date: Date, now: Date = Date()) -> String {
    let time = date.formatted(date: .omitted, time: .shortened)
    if Calendar.current.isDate(date, inSameDayAs: now) { return "at \(time)" }
    let days = date.timeIntervalSince(now) / 86400
    if days >= 0, days < 6 { return "\(date.formatted(.dateTime.weekday(.abbreviated))) at \(time)" }
    return "\(date.formatted(.dateTime.day().month(.abbreviated))) at \(time)"
}

/// How a row sits on the canvas: no separators and no grey cell, each kind
/// of row with its own room around it, so the list reads as T3 Code's sidebar
/// rather than as a table.
enum RowFit {
    /// A worktree's card, a little in from the edges.
    case card
    /// A project's folder row, with room above it to set it off from the one
    /// before.
    case folder
    /// A shelf's or a group's label.
    case head
    /// A queued session, or one on a shelf: no card, lined up with the cards'
    /// text.
    case slim

    var insets: EdgeInsets {
        switch self {
        case .card: EdgeInsets(top: 3, leading: 12, bottom: 3, trailing: 12)
        case .folder: EdgeInsets(top: 14, leading: 16, bottom: 4, trailing: 12)
        case .head: EdgeInsets(top: 14, leading: 16, bottom: 2, trailing: 16)
        case .slim: EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)
        }
    }
}

extension View {
    func canvasRow(_ fit: RowFit) -> some View {
        listRowInsets(fit.insets)
            .listRowSeparator(.hidden)
            .listRowBackground(Color.canvas)
    }
}

/// A project as T3 Code draws it: a folder row with its monogram and name,
/// how many it holds when folded, a `+` for a new worktree (the reason to be
/// looking at a project), and its menu. A tap anywhere else folds it.
private struct ProjectHeader: View {
    let project: Project
    let folded: Bool
    let count: Int
    let canCreate: Bool
    let toggle: () -> Void
    let create: () -> Void
    let defaults: () -> Void
    let forget: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            HStack(spacing: 10) {
                ProjectMonogram(name: project.name)
                Text(project.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.ink)
                    .lineLimit(1)
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.dimmer)
                    .rotationEffect(.degrees(folded ? 0 : 90))
                if folded, count > 0 {
                    Text("\(count)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.dimmer)
                }
                Spacer(minLength: 8)
            }
            .contentShape(Rectangle())
            .onTapGesture(perform: toggle)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(folded ? "\(project.name), folded, \(count) inside" : project.name)
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(folded ? "Unfolds the project" : "Folds the project")
            Button(action: create) {
                Image(systemName: "plus")
                    .frame(width: 30, height: 30)
                    .contentShape(Rectangle())
            }
            .disabled(!canCreate)
            .accessibilityLabel("New Worktree in \(project.name)")
            Menu {
                // Where it clones from, which the folder row has no room for.
                Section(project.repo) {
                    Button("New Worktree", systemImage: "plus", action: create)
                        .disabled(!canCreate)
                    Button("Defaults for New Worktrees", glyph: .settings, action: defaults)
                        .disabled(!canCreate)
                    Button("Forget Project", glyph: .forgetProject, role: .destructive, action: forget)
                }
            } label: {
                Image(systemName: "ellipsis")
                    .frame(width: 30, height: 30)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("\(project.name) menu")
        }
        .font(.subheadline.weight(.medium))
        .foregroundStyle(.dim)
        .buttonStyle(.borderless)
    }
}

/// A shelf's head, T3 Code's section header: its name, a hairline, how many,
/// and a chevron when it folds.
private struct ShelfHead: View {
    let label: String
    var count: Int?
    var open: Bool?
    var toggle: (() -> Void)?

    var body: some View {
        HStack(spacing: 8) {
            Text(label)
            Rectangle().fill(Color.line).frame(height: 1)
            if let count { Text("\(count)").monospacedDigit() }
            if let open {
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.bold))
                    .rotationEffect(.degrees(open ? 90 : 0))
            }
        }
        .font(.caption.weight(.medium))
        .foregroundStyle(.dim)
        .contentShape(Rectangle())
        .onTapGesture { toggle?() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(count.map { "\(label), \($0)" } ?? label)
        .accessibilityAddTraits(toggle == nil ? .isHeader : .isButton)
    }
}

/// A worktree as T3 Code's card: its title at full weight, with its state as
/// a glyph and a word on the right, or its age when there is nothing to say;
/// under it the branch and what else is true about it -- its pull request,
/// that it is not sandboxed, messages waiting to be said to it -- and, when
/// there is something to say, what the agent is waiting on or why it failed.
struct SessionRow: View {
    let session: Session
    let report: AgentReport?
    /// The open pull request it is on, if the inbox knows it.
    var pull: PullRequestSummary?
    /// Messages waiting on the server to be said to it, oldest first.
    var waiting: [QueuedMessage] = []
    /// Its project, for a card outside its project's folder: on the Pinned
    /// shelf, where the folder no longer says it.
    var project: String?

    var body: some View {
        let state = liveState(session, report)
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 8) {
                Text(session.label)
                    .font(.body.weight(.medium))
                    .foregroundStyle(.ink)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if let word = stateWord(state) {
                    HStack(spacing: 4) {
                        StateMark(state: state).font(.footnote)
                        Text(word.label)
                    }
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(word.color)
                    .fixedSize()
                } else {
                    Text(age(session.createdAt))
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(.dimmer)
                        .accessibilityLabel("Started \(age(session.createdAt)) ago")
                }
            }
            HStack(spacing: 6) {
                if let project {
                    ProjectMonogram(name: project, size: 15)
                    Text(project).lineLimit(1).layoutPriority(1)
                }
                if !session.workBranch.isEmpty {
                    if project == nil { Image(glyph: .branch).font(.caption2) }
                    Text(session.workBranch).lineLimit(1).truncationMode(.middle)
                }
                Spacer(minLength: 6)
                if let pull { PullChip(pull: pull) }
                if !waiting.isEmpty { WaitingMark(messages: waiting) }
                if session.isWorktree {
                    Image(glyph: .unsandboxed)
                        .font(.caption)
                        .foregroundStyle(Color.warn)
                        .accessibilityLabel("Worktree on the server, not isolated")
                }
            }
            .font(.footnote)
            .foregroundStyle(.dim)
            if let failure = session.failure, ["failed", "dead"].contains(session.state) {
                Text(failure)
                    .font(.footnote)
                    .foregroundStyle(.bad)
                    .lineLimit(2)
            } else if let prompt = report?.state == "waiting" ? (report?.prompt ?? report?.detail) : nil {
                Text(prompt)
                    .font(.footnote)
                    .foregroundStyle(.attention)
                    .lineLimit(2)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }
}

/// A worktree on a shelf: T3 Code's slim row, its project's mark gone grey,
/// its title, and the shelf's note: when it comes back, or how long since it
/// was settled.
private struct ShelfRow: View {
    let session: Session
    let note: String

    var body: some View {
        HStack(spacing: 10) {
            if let project = session.project {
                ProjectMonogram(name: project, size: 16).grayscale(1).opacity(0.6)
            } else {
                Image(glyph: .branch).font(.footnote).frame(width: 16)
            }
            Text(session.label).font(.subheadline).lineLimit(1)
            Spacer(minLength: 8)
            Text(note).font(.caption.monospacedDigit()).foregroundStyle(.dimmer)
        }
        .foregroundStyle(.dim)
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
    }
}

/// That something is waiting to be said to a session, and when the first of
/// it will be: after the turn when any is waiting only for that, or else at
/// the soonest time. A glyph and a count, as the desktop's sidebar has it;
/// the time is in the chat, and read out to VoiceOver.
private struct WaitingMark: View {
    let messages: [QueuedMessage]

    var body: some View {
        let times = messages.map(\.at)
        let soonest = times.contains(nil) ? nil : times.compactMap { $0 }.min()
        HStack(spacing: 3) {
            Image(systemName: soonest == nil ? "hourglass" : "calendar.badge.clock")
            if messages.count > 1 { Text("\(messages.count)") }
        }
        .font(.caption.monospacedDigit())
        .foregroundStyle(.dim)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label(soonest))
    }

    private func label(_ soonest: Date?) -> String {
        let what = messages.count == 1 ? "A message waiting" : "\(messages.count) messages waiting"
        let when = soonest.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? "after this turn"
        return messages.count == 1 ? "\(what), \(when)" : "\(what), the first \(when)"
    }
}

/// A session waiting on the server's queue: T3 Code's slim row, dimmed, with
/// when it starts under its name. A repeating one says its rule, on this
/// phone's clock, and how many times it has come round.
struct QueuedRow: View {
    let waiting: QueuedSession
    let place: Int?
    let limit: Int
    /// The server's offset from UTC, which a repeat's time of day is on.
    var serverOffset: Int = 0

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Image(systemName: waiting.repeat != nil ? "repeat" : waiting.at == nil ? "hourglass" : "calendar.badge.clock")
                .font(.footnote)
                .foregroundStyle(.dimmer)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 2) {
                Text(waiting.label)
                    .font(.subheadline)
                    .foregroundStyle(.dim)
                    .lineLimit(1)
                Text(note)
                    .font(.caption)
                    .foregroundStyle(waiting.failed == nil ? Color.dimmer : Color.bad)
                    .lineLimit(2)
            }
            Spacer(minLength: 8)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    private var note: String {
        if waiting.started { return "Starting…" }
        if let failed = waiting.failed { return "Couldn't start: \(failed)" }
        if let rule = waiting.repeat {
            var parts = [rule.describe(serverOffset: serverOffset)]
            if let at = waiting.at { parts.append("next \(startsAt(at))") }
            if waiting.runs > 0 { parts.append("run \(waiting.runs) time\(waiting.runs == 1 ? "" : "s")") }
            return parts.joined(separator: " · ")
        }
        if let at = waiting.at {
            return "Starts \(at.formatted(.relative(presentation: .named))), \(startsAt(at))"
        }
        if place == 1 { return "Next, when fewer than \(limit) are running" }
        return "Queued, \(place.map { "\($0)" } ?? "?") in line"
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

private struct ProjectScreens: ViewModifier {
    let server: Server
    @Binding var defaulting: Project?
    @Binding var showingUsage: Bool
    let open: (String) -> Void

    func body(content: Content) -> some View {
        content
            .navigationDestination(item: $defaulting) { ProjectDefaultsView(server: server, project: $0) }
            .navigationDestination(isPresented: $showingUsage) { UsageView(server: server, onOpen: open) }
    }
}
