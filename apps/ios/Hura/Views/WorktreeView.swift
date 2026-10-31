import HuraKit
import SwiftUI

/// What a worktree has open: its agent (a conversation or a terminal), the
/// other conversations a chat session has, and the shells beside it. Which
/// exist is asked of the sandbox rather than remembered here, as the desktop
/// window does, so one opened or closed elsewhere shows up here.
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
}

struct WorktreeView: View {
    let server: Server
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var tab: WorkTab?
    @State private var conversations: [String] = []
    @State private var shells: [String] = []
    @State private var chats: [String: Chat] = [:]
    @State private var terminals: [String: TerminalChannel] = [:]
    @State private var showingDetails = false
    @State private var showingDock = false
    @State private var confirmingDelete = false
    @State private var error: String?

    private var session: Session? { server.session(named: name) }

    /// A terminal can't attach before the sandbox is up, and a session that
    /// failed or died has nothing to attach to. A chat shows hurad's own
    /// notice while it clones.
    private var hasWorkspace: Bool {
        guard let s = session else { return false }
        return s.isLive || (s.isChat && ["creating", "seeding"].contains(s.state))
    }

    private var home: WorkTab { session?.isChat == true ? .chat("agent") : .terminal(nil) }

    private var tabs: [WorkTab] {
        let agent: [WorkTab] = session?.isChat == true
            ? [.chat("agent")] + conversations.filter { $0 != "agent" }.map(WorkTab.chat)
            : [.terminal(nil)]
        return agent + shells.map { WorkTab.terminal($0) }
    }

    var body: some View {
        Group {
            if hasWorkspace, let tab {
                // Conversations and terminals are drawn dark; so is the bar
                // over them, or its title is black on black.
                content(tab).id(tab)
                    .toolbarBackground(Color.black, for: .navigationBar)
                    .toolbarBackground(.visible, for: .navigationBar)
                    .toolbarColorScheme(.dark, for: .navigationBar)
            } else {
                details
            }
        }
        .navigationTitle(tab.map { $0.isAgent ? name : "\(name) · \($0.label)" } ?? name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbar }
        .navigationDestination(isPresented: $showingDock) {
            DockView(server: server, name: name)
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
        .confirmationDialog("Delete \(name)?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            Button("Delete Worktree", role: .destructive, action: destroy)
        } message: {
            Text("Its sandbox is deleted, with everything in it that hasn't been pushed.")
        }
        .alert("Couldn't do that", isPresented: .constant(error != nil)) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
        // Opened from a notification before the list has arrived: the agent
        // opens once the session is known.
        .task(id: session?.interface) {
            if session != nil, tab == nil { switchTo(home) }
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
        .onAppear { model.visibleSession = (server.id, name) }
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
            if let c = chats[conv] {
                ChatView(chat: c)
                    .environment(\.readPicture) { [server, name] path in
                        try? await server.api.file(name, path: path).image
                    }
            }
        case let .terminal(tmux):
            if let t = terminals[tmux ?? ""] { TerminalScreen(channel: t) }
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if session?.isLive == true {
            ToolbarItem(placement: .primaryAction) {
                Button { showingDock = true } label: {
                    Label("Files, Git and Policy", systemImage: "sidebar.right")
                }
            }
        }
        if hasWorkspace, let session {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Picker("Open", selection: Binding(get: { tab ?? home }, set: { switchTo($0) })) {
                        ForEach(tabs, id: \.self) { t in
                            Label(t.label, systemImage: icon(t)).tag(t)
                        }
                    }
                    Divider()
                    if session.isChat {
                        Button("New Conversation", systemImage: "plus.bubble") { newConversation() }
                    }
                    Button("New Shell", systemImage: "apple.terminal") { newShell() }
                    if let tab, !tab.isAgent {
                        Button("Close \(tab.label)", systemImage: "xmark", role: .destructive) { close(tab) }
                    }
                    Divider()
                    Button("Details", systemImage: "info.circle") { showingDetails = true }
                } label: {
                    Label("Tabs", systemImage: tabs.count > 1 ? "square.on.square" : "ellipsis.circle")
                }
            }
        }
    }

    private func icon(_ t: WorkTab) -> String {
        switch t {
        case .chat: "bubble.left.and.text.bubble.right"
        case .terminal(nil): "terminal"
        case .terminal: "apple.terminal"
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
                        LabeledContent("Branch") { Text(session.workBranch).font(.footnote.monospaced()) }
                    }
                    LabeledContent("Base") {
                        Text(session.baseBranch ?? "default branch").font(.footnote.monospaced())
                    }
                    if let project = session.project { LabeledContent("Project", value: project) }
                    LabeledContent("Agent", value: session.isChat ? "Chat" : "Terminal")
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
                        Text(session.repo).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                    }
                }
                Section {
                    Button("Delete Worktree", role: .destructive) { confirmingDelete = true }
                }
            } else {
                ContentUnavailableView("Session not found", systemImage: "questionmark.circle",
                                       description: Text(server.state == .online ? "It may have been removed." : server.state.label))
            }
        }
        .refreshable { await server.refresh() }
    }

    // MARK: Tabs

    private func chat(_ conv: String) -> Chat {
        if let c = chats[conv] { return c }
        let c = Chat(record: server.record, session: name, conversation: conv)
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

    private func switchTo(_ next: WorkTab) {
        // Only what is on screen holds a socket.
        if case let .chat(conv)? = tab { chats[conv]?.stop() }
        if case let .terminal(tmux)? = tab { terminals[tmux ?? ""]?.stop() }
        switch next {
        case let .chat(conv): chat(conv).start()
        case let .terminal(tmux): _ = terminal(tmux)
        }
        tab = next
    }

    private func reloadTabs() async {
        guard let session else { return }
        async let listed = try? server.api.shells(name)
        if session.isChat, let c = try? await server.api.chats(name) { conversations = c.chats }
        if let s = await listed { shells = s }
        // Closed from elsewhere: back to the agent.
        if let tab, !tabs.contains(tab) { switchTo(home) }
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
            let before = Set(shells)
            shells = try await server.api.newShell(name)
            if let made = shells.first(where: { !before.contains($0) }) { switchTo(.terminal(made)) }
        }
    }

    private func close(_ t: WorkTab) {
        act {
            switchTo(home)
            switch t {
            case let .chat(conv):
                chats.removeValue(forKey: conv)?.stop()
                conversations = try await server.api.closeChat(name, conversation: conv).chats
            case let .terminal(tmux?):
                terminals.removeValue(forKey: tmux)?.stop()
                shells = try await server.api.killShell(name, tmux: tmux)
            case .terminal(nil):
                break
            }
        }
    }

    private func destroy() {
        act {
            chats.values.forEach { $0.stop() }
            terminals.values.forEach { $0.stop() }
            try await server.destroy(name)
            showingDetails = false
            dismiss()
        }
    }

    private func act(_ work: @escaping () async throws -> Void) {
        Task {
            do { try await work() } catch { self.error = error.localizedDescription }
        }
    }
}
