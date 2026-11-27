import HuraKit
import SwiftUI

/// What a worktree has open: its agent (a conversation or a terminal), the
/// other conversations a chat session has, and the shells beside it. Which
/// exist is asked of the sandbox rather than remembered here, as the desktop
/// window does, so one opened or closed elsewhere shows up here. The agent is
/// one of them: it can be closed, and another started.
enum WorkTab: Hashable {
    /// A conversation; `agent` is the session's own.
    case chat(String)
    /// A tmux session; nil is the agent's own screen.
    case terminal(String?)

    var label: String {
        switch self {
        case .chat("agent"), .terminal(nil): "Agent"
        case let .chat(conv): conv
        case let .terminal(tmux?): tmux
        }
    }

    var isAgent: Bool { self == .chat("agent") || self == .terminal(nil) }

    var isTerminal: Bool {
        if case .terminal = self { true } else { false }
    }
}

struct WorktreeView: View {
    let server: Server
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(TrackersModel.self) private var trackers
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(TerminalLook.darkKey) private var terminalsDark = false
    @State private var tab: WorkTab?
    /// Nil until asked: the agent is only shown once the sandbox says it is
    /// there, since attaching to a closed one's terminal starts another.
    @State private var conversations: [String]?
    @State private var shells: Shells?
    /// The conversations this screen has opened, out of the server's: kept
    /// there after this screen goes, so coming back doesn't replay them.
    @State private var chats: [String: Chat] = [:]
    @State private var terminals: [String: TerminalChannel] = [:]
    @State private var showingDetails = false
    @State private var showingDock = false
    @State private var confirmingDelete = false
    @State private var error: String?
    /// A search result being opened in one of the conversations.
    @State private var jump: (conv: String, to: ChatJump)?
    /// What to say in a conversation as soon as it is open, by conversation:
    /// an edited message's new words, a side chat's question.
    @State private var firstWords: [String: String] = [:]
    /// The side chat (`/btw`) over the conversation, while it is open: a
    /// conversation forked from it, which closing the sheet ends.
    @State private var side: String?
    @State private var showingSide = false
    /// This project's scripts, kept on the phone.
    @State private var scripts: [ProjectScript] = []
    @State private var editingScripts = false

    private var session: Session? { server.session(named: name) }
    /// The title it is shown by, or its name before the list has it.
    private var label: String { session?.label ?? name }
    /// The open pull request it is on, if the inbox knows it.
    private var pull: PullRequestSummary? { session.flatMap { sessionPull($0, in: trackers.inbox?.pulls ?? []) } }

    /// A terminal can't attach before the sandbox is up, and a session that
    /// failed or died has nothing to attach to. A chat shows hurad's own
    /// notice while it clones.
    private var hasWorkspace: Bool {
        guard let s = session else { return false }
        return s.isLive || (s.isChat && ["creating", "seeding"].contains(s.state))
    }

    /// Whether the sandbox has been asked what is open in it yet.
    private var listed: Bool { shells != nil && (session?.isChat != true || conversations != nil) }

    private var tabs: [WorkTab] {
        guard let session else { return [] }
        // A chat's agent is assumed until the host says otherwise: one being
        // prepared has its notice to show, and attaching to a closed
        // conversation is only refused. A terminal agent's waits, because
        // attaching to a closed one starts another.
        // The side chat is in its sheet, not a tab.
        let agent: [WorkTab] = session.isChat
            ? (conversations ?? ["agent"]).filter { $0 != side }.map(WorkTab.chat)
            : shells?.agent == true ? [.terminal(nil)] : []
        return agent + (shells?.shells ?? []).map { WorkTab.terminal($0) }
    }

    /// Where a worktree opens, and goes back to when the tab in front closes.
    private var home: WorkTab? { tabs.first }

    /// Which project's scripts these are: a session from before projects has
    /// only its repository to go by.
    private var project: String? { session.map { $0.project ?? $0.repo } }

    var body: some View {
        Group {
            if hasWorkspace, let tab {
                // The bar is the colour of what is under it. A terminal kept
                // dark in a light app takes a dark bar with it, or its title
                // is black on black; otherwise both follow the appearance.
                let dark = tab.isTerminal && terminalsDark
                content(tab).id(tab)
                    .safeAreaInset(edge: .top, spacing: 0) {
                        if tabs.count > 1 { tabStrip(dark: dark) }
                    }
                    .toolbarBackground(dark ? Color.black : Color.canvas, for: .navigationBar)
                    .toolbarBackground(.visible, for: .navigationBar)
                    .toolbarColorScheme(dark ? .dark : nil, for: .navigationBar)
            } else if hasWorkspace, !listed, tabs.isEmpty {
                ProgressView()
            } else if hasWorkspace, listed, tabs.isEmpty {
                nothingOpen
            } else {
                details
            }
        }
        .navigationTitle(label)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbar }
        .sheet(isPresented: $showingSide, onDismiss: closeSide) { sideSheet }
        .sheet(isPresented: $editingScripts) {
            ScriptsEditor(project: project ?? "", scripts: scripts) { edited in
                scripts = edited
                if let project { ProjectScripts().write(edited, server: server.id, project: project) }
            }
        }
        .task(id: project) {
            if let project { scripts = ProjectScripts().read(server: server.id, project: project) }
        }
        // A session has the whole screen; back is the way to the tabs.
        .toolbar(.hidden, for: .tabBar)
        .navigationDestination(isPresented: $showingDock) {
            DockView(server: server, name: name)
        }
        .sheet(isPresented: $showingDetails) {
            NavigationStack {
                details
                    .navigationTitle(label)
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { showingDetails = false }
                        }
                    }
            }
            .presentationDetents([.medium, .large])
        }
        .alert("Couldn't do that", isPresented: .constant(error != nil)) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
        // Opened from a notification before the list has arrived: the agent
        // opens once the session is known.
        .task(id: tabs) {
            if tab == nil, let home { switchTo(home) }
        }
        // A search result: its conversation in front, at the message.
        .task(id: model.pendingJump) {
            guard let j = model.pendingJump, j.serverID == server.id, j.session == name else { return }
            model.pendingJump = nil
            switchTo(.chat(j.conv))
            jump = (j.conv, ChatJump(seq: j.seq, nonce: j.nonce))
        }
        .task(id: hasWorkspace) {
            guard hasWorkspace else { return }
            while !Task.isCancelled {
                await reloadTabs()
                try? await Task.sleep(for: .seconds(5))
            }
        }
        .onChange(of: scenePhase) { _, phase in
            // iOS drops sockets in the background anyway; a conversation
            // reopens on return asking only for what it missed. A terminal
            // looks after its own.
            guard case let .chat(conv)? = tab else { return }
            if phase == .background { chats[conv]?.stop() } else if phase == .active { chats[conv]?.start() }
        }
        .onAppear {
            model.visibleSession = (server.id, name)
            // Back from the dock, which is pushed on top of this and so
            // made it disappear: the conversation on screen opens again,
            // asking only for what it missed. A terminal reopens itself.
            if case let .chat(conv)? = tab { chats[conv]?.start() }
        }
        .onDisappear {
            chats.values.forEach { $0.stop() }
            terminals.values.forEach { $0.stop() }
            if model.visibleSession?.session == name { model.visibleSession = nil }
        }
    }

    @ViewBuilder
    private func content(_ tab: WorkTab) -> some View {
        // Made in `switchTo`, never here: state set while drawing is dropped.
        switch tab {
        case let .chat(conv):
            if let c = chats[conv] { chatView(c, conv: conv, nested: false) }
        case let .terminal(tmux):
            if let t = terminals[tmux ?? ""] { TerminalScreen(channel: t) }
        }
    }

    /// A conversation, in a tab or as the side chat in its sheet (`nested`),
    /// which has no side chat or edits of its own.
    private func chatView(_ c: Chat, conv: String, nested: Bool) -> some View {
        ChatView(chat: c, preparing: ["creating", "seeding"].contains(session?.state ?? ""), pull: nested ? nil : pull,
                 usage: { [server, name] in try? await server.api.poll(name).usage },
                 jump: jump?.conv == conv ? jump?.to : nil, onJumped: { jump = nil },
                 waiting: nested ? [] : server.queue.messages(for: name),
                 onSchedule: nested ? nil : { [server, name] text, at, rule in
                     try await server.queueMessage(name, text: text, at: at, repeat: rule)
                 },
                 utcOffset: server.queue.utcOffset,
                 onUnqueue: { [server] m in Task { try? await server.unqueue(m.id) } },
                 draftKey: Drafts.key(server: server.id, session: name, conv: conv),
                 findFiles: { [server, name] query in try await server.api.findFiles(name, query: query) },
                 tailFile: { [server, name] path in try await server.api.tailFile(name, path: path) },
                 onBranch: nested ? nil : { at, text in branch(conv, at: at, text) },
                 onSideChat: nested ? nil : { question in openSide(conv, question) },
                 sayFirst: firstWords[conv], onSaidFirst: { firstWords[conv] = nil })
            .environment(\.readPicture) { [server, name] path in
                try? await server.api.file(name, path: path).image
            }
            .environment(\.heldPicture) { [c] held in await c.picture(held) }
            .downloads(server: server, name: name)
    }

    /// The conversations and shells as T3 Code's tabs under the header, while
    /// there is more than one: the menu still lists them, but a tap here is
    /// quicker than two there.
    private func tabStrip(dark: Bool) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(tabs, id: \.self) { t in
                    let on = t == tab
                    Button { switchTo(t) } label: {
                        Label(t.label, glyph: icon(t))
                            .labelStyle(.titleAndIcon)
                            .font(.caption.weight(on ? .semibold : .regular))
                            .lineLimit(1)
                            .foregroundStyle(on ? Color.ink : Color.dim)
                            .padding(.horizontal, 10)
                            .frame(height: 28)
                            .background(on ? Color.surface2 : .clear, in: RoundedRectangle(cornerRadius: Radius.control))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(on ? .isSelected : [])
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
        .background(dark ? Color.black : Color.canvas)
        .overlay(alignment: .bottom) { Rectangle().fill(Color.line).frame(height: 1) }
        .environment(\.colorScheme, dark ? .dark : colorScheme)
    }

    @Environment(\.colorScheme) private var colorScheme

    /// The side chat: a conversation forked from this one, in a sheet over
    /// it, so the turn in progress goes on. Closing it ends it.
    @ViewBuilder
    private var sideSheet: some View {
        NavigationStack {
            Group {
                if let side, let c = chats[side] {
                    chatView(c, conv: side, nested: true)
                } else {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity).background(Color.canvas)
                }
            }
            .navigationTitle("Side Chat")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(Color.canvas, for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { showingSide = false }
                }
            }
        }
        .presentationDetents([.large, .medium])
        .presentationDragIndicator(.visible)
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        // T3 Code's breadcrumb, stacked for a phone: the title, and under it
        // the project and the branch.
        ToolbarItem(placement: .principal) {
            VStack(spacing: 0) {
                Text(label)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Color.ink)
                if let session {
                    Text([session.project, session.workBranch.isEmpty ? nil : session.workBranch]
                        .compactMap { $0 }.joined(separator: " / "))
                        .font(.caption2)
                        .foregroundStyle(Color.dim)
                }
            }
            .lineLimit(1)
            .truncationMode(.middle)
            .accessibilityElement(children: .combine)
        }
        if session?.isLive == true {
            ToolbarItem(placement: .primaryAction) {
                Button { showingDock = true } label: {
                    Label("Files, Git and Policy", glyph: .dock)
                }
            }
        }
        if hasWorkspace, let session {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    if let current = tab ?? home, tabs.count > 1 {
                        Picker("Open", selection: Binding(get: { current }, set: { switchTo($0) })) {
                            ForEach(tabs, id: \.self) { t in
                                Label(t.label, glyph: icon(t)).tag(t)
                            }
                        }
                    }
                    Section {
                        if session.isChat {
                            Button("New Conversation", glyph: .newConversation) { newConversation() }
                        } else if shells?.agent == false {
                            Button("New Agent", glyph: .agent) { newAgent() }
                        }
                        Button("New Shell", glyph: .shell) { newShell() }
                    }
                    // The project's commands, each in a shell of its own.
                    Section("Scripts") {
                        ForEach(scripts) { s in
                            Button(s.name.isEmpty ? s.command : s.name, glyph: .start) { run(s) }
                        }
                        Button(scripts.isEmpty ? "Add a Script…" : "Edit Scripts…", glyph: .edit) {
                            editingScripts = true
                        }
                    }
                    Section {
                        if let tab {
                            Button(tab == .terminal(nil) ? "Stop Agent" : "Close \(tab.label)",
                                   systemImage: "xmark", role: .destructive) { close(tab) }
                        }
                        Button("Details", glyph: .info) { showingDetails = true }
                    }
                } label: {
                    Label("Worktree", systemImage: "ellipsis.circle")
                }
            }
        }
    }

    private func icon(_ t: WorkTab) -> Glyph {
        switch t {
        case .chat: .chat
        case .terminal(nil): .agentTerminal
        case .terminal: .shell
        }
    }

    /// Everything closed. The sandbox is still there, and so is the way back.
    private var nothingOpen: some View {
        ContentUnavailableView {
            Label("Nothing running here", glyph: session?.isChat == true ? .chat : .agent)
        } actions: {
            if session?.isChat == true {
                Button("New Conversation", glyph: .newConversation) { newConversation() }
                    .buttonStyle(.borderedProminent)
            } else {
                Button("New Agent", glyph: .agent) { newAgent() }
                    .buttonStyle(.borderedProminent)
            }
            Button("New Shell", glyph: .shell) { newShell() }
        }
    }

    // MARK: Details

    private var details: some View {
        List {
            if let session {
                let report = server.reports[session.name]
                Section {
                    LabeledContent("State") { StatusBadge(status: agentStatus(session, report)) }
                    if !session.workBranch.isEmpty {
                        LabeledContent("Branch") { Text(session.workBranch).font(.footnote) }
                    }
                    LabeledContent("Base") {
                        Text(session.baseBranch ?? "default branch").font(.footnote)
                    }
                    if let project = session.project { LabeledContent("Project", value: project) }
                    LabeledContent("Agent", value: session.isChat ? "Chat" : "Terminal")
                    if session.isWorktree {
                        LabeledContent("Runs") {
                            Text("On the server, not isolated").foregroundStyle(.orange)
                        }
                    }
                    if let policy = session.policy { LabeledContent("Network policy", value: policy) }
                    if !session.toolchains.isEmpty {
                        LabeledContent("Toolchains", value: session.toolchains.joined(separator: ", "))
                    }
                    if !session.providers.isEmpty {
                        LabeledContent("Credentials", value: session.providers.joined(separator: ", "))
                    }
                    if session.createdAt > 0 {
                        LabeledContent("Created") {
                            Text(Date(timeIntervalSince1970: TimeInterval(session.createdAt)), style: .relative)
                            + Text(" ago")
                        }
                    }
                }
                if let failure = session.failure {
                    Section("Why it failed") {
                        Text(failure).font(.callout.monospaced()).foregroundStyle(.red).textSelection(.enabled)
                    }
                }
                // A terminal session's prompt is answered in its terminal; a
                // chat's in its conversation.
                if let report, report.state == "waiting", !hasWorkspace {
                    Section("Waiting on you") {
                        Text(report.prompt ?? report.detail ?? "Claude is waiting")
                            .font(.callout.monospaced())
                    }
                }
                if !session.task.isEmpty {
                    Section("Task") { Text(session.task).font(.callout).textSelection(.enabled) }
                }
                Section {
                    LabeledContent("Repository") {
                        Text(session.repo).font(.caption).lineLimit(1).truncationMode(.middle)
                    }
                }
                Section {
                    Button("Delete Worktree", role: .destructive) { confirmingDelete = true }
                        // On the button, so it opens over it, and from the
                        // details sheet as well as the screen.
                        .confirmationDialog("Delete \(label)?", isPresented: $confirmingDelete, titleVisibility: .visible) {
                            Button("Delete Worktree", role: .destructive, action: destroy)
                        } message: {
                            Text(session.isWorktree
                                 ? "Its worktree on the server is deleted with its changes. A branch with commits on it is kept."
                                 : "Its sandbox is deleted, with everything in it that hasn't been pushed.")
                        }
                }
            } else {
                ContentUnavailableView("Session not found", image: Glyph.waiting.name,
                                       description: Text(server.state == .online ? "It may have been removed." : server.state.label))
            }
        }
        .refreshable { await server.refresh() }
    }

    // MARK: Tabs

    private func chat(_ conv: String) -> Chat {
        if let c = chats[conv] { return c }
        let c = server.chat(name, conversation: conv)
        chats[conv] = c
        return c
    }

    private func terminal(_ tmux: String?) -> TerminalChannel {
        let key = tmux ?? ""
        if let t = terminals[key] { return t }
        let t = TerminalChannel(record: server.record, session: name, tmux: tmux)
        terminals[key] = t
        return t
    }

    private func switchTo(_ next: WorkTab?) {
        // Only what is on screen holds a socket.
        if case let .chat(conv)? = tab { chats[conv]?.stop() }
        if case let .terminal(tmux)? = tab { terminals[tmux ?? ""]?.stop() }
        switch next {
        case let .chat(conv)?: chat(conv).start()
        case let .terminal(tmux)?: _ = terminal(tmux)
        case nil: break
        }
        tab = next
    }

    private func reloadTabs() async {
        guard let session else { return }
        async let listed = try? server.api.shells(name)
        if session.isChat, let c = try? await server.api.chats(name) { conversations = c.chats }
        if let s = await listed { shells = s }
        // Closed from elsewhere: back to the first that is left.
        if let tab, !tabs.contains(tab) { forget(tab) }
    }

    private func newConversation() {
        act {
            let c = try await server.api.newChat(name)
            conversations = c.chats
            if let opened = c.opened { switchTo(.chat(opened)) }
        }
    }

    private func newShell() {
        act {
            let before = Set(shells?.shells ?? [])
            let after = try await server.api.newShell(name)
            shells = after
            if let made = after.shells.first(where: { !before.contains($0) }) { switchTo(.terminal(made)) }
        }
    }

    /// A project's script, run in a new shell named for it, in front: the
    /// server types the command in once the shell is up. An older server
    /// opens the shell and leaves it at its prompt.
    private func run(_ script: ProjectScript) {
        act {
            let before = Set(shells?.shells ?? [])
            let after = try await server.api.newShell(name, run: script.command)
            shells = after
            if let made = after.shells.first(where: { !before.contains($0) }) { switchTo(.terminal(made)) }
        }
    }

    /// "Edit from Here": a new conversation forked from just before the
    /// message (or a plain one, before anything was said), in front, with the
    /// new words said in it. The conversation edited stays as it was.
    private func branch(_ conv: String, at: String?, _ text: String) {
        act {
            let c = try await server.api.newChat(name, fork: at == nil ? nil : conv, at: at)
            conversations = c.chats
            guard let opened = c.opened else { return }
            firstWords[opened] = text
            switchTo(.chat(opened))
        }
    }

    /// `/btw`: the question asked in a conversation forked from this one, in
    /// a sheet over it.
    private func openSide(_ conv: String, _ question: String) {
        act {
            let c = try await server.api.newChat(name, fork: conv)
            guard let opened = c.opened else { return }
            firstWords[opened] = question
            side = opened
            conversations = c.chats
            chat(opened).start()
            showingSide = true
        }
    }

    /// The side chat's sheet has gone: its conversation is ended and
    /// forgotten, as on the desktop.
    private func closeSide() {
        guard let conv = side else { return }
        // Out of the list before it is out of `side`, or it is a tab for the
        // moment until the server answers.
        conversations?.removeAll { $0 == conv }
        side = nil
        firstWords[conv] = nil
        chats.removeValue(forKey: conv)?.stop()
        server.forgetChat(name, conversation: conv)
        Task { [server, name] in
            if let c = try? await server.api.closeChat(name, conversation: conv) { conversations = c.chats }
        }
    }

    private func newAgent() {
        act {
            shells = try await server.api.newAgent(name)
            switchTo(.terminal(nil))
        }
    }

    /// Off screen and dropped, its socket with it, and the first that is
    /// left in front instead.
    private func forget(_ t: WorkTab) {
        switch t {
        case let .chat(conv):
            chats.removeValue(forKey: conv)?.stop()
            server.forgetChat(name, conversation: conv)
        case let .terminal(tmux): terminals.removeValue(forKey: tmux ?? "")?.stop()
        }
        guard tab == t else { return }
        tab = nil
        switchTo(tabs.first { $0 != t })
    }

    private func close(_ t: WorkTab) {
        act {
            forget(t)
            switch t {
            case let .chat(conv):
                // The host forgets its transcript, so what this phone has not
                // kept of it is asked for first.
                await server.catchUp(name, conversation: conv, waiting: true)
                conversations = try await server.api.closeChat(name, conversation: conv).chats
                server.history?.ended(ChatHistory.Conversation(serverID: server.id, session: name, conv: conv))
            case let .terminal(tmux?):
                shells = try await server.api.killShell(name, tmux: tmux)
            case .terminal(nil):
                guard let session else { return }
                shells = try await server.api.killShell(name, tmux: session.tmux)
            }
        }
    }

    /// Back to the list at once; the server goes on deleting it, and says
    /// there if it couldn't.
    private func destroy() {
        chats.values.forEach { $0.stop() }
        terminals.values.forEach { $0.stop() }
        server.destroy(name)
        showingDetails = false
        dismiss()
    }

    private func act(_ work: @escaping () async throws -> Void) {
        Task {
            do { try await work() } catch { self.error = error.userMessage }
        }
    }
}
