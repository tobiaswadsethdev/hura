import HuraKit
import PhotosUI
import SwiftUI

/// A chat session's conversation: the transcript, the reply as it is
/// written, what is waiting on you, and a composer. hura runs Claude Code
/// through the Agent SDK, so a permission request is a request with an id,
/// answered with a button.
struct ChatView: View {
    let chat: Chat
    @State private var draft = ""
    @State private var sending = false
    /// What was just sent, and when: the keyboard commits a pending
    /// prediction into the field after it is emptied, which put the message
    /// back. For a moment after a send, that is not typing.
    @State private var justSent: (text: String, at: Date)?
    @State private var answering: String?
    @State private var atBottom = true
    @State private var items: [ChatItem] = []
    /// Sent and not yet read: drawn after the turn in progress.
    @State private var queued: [ChatItem] = []
    /// Files to send with the next message.
    @State private var attached: [ChatUpload] = []
    @State private var choosingPhotos = false
    @State private var choosingFiles = false
    @State private var photos: [PhotosPickerItem] = []
    /// The subagents, kept with the items they're folded beside.
    @State private var agents: [AgentInfo] = []
    /// The subagent being followed, by task id.
    @State private var following: Following?
    @FocusState private var composing: Bool

    /// hura clones every session's repository here.
    private static let cwd = "/sandbox/repo"
    private var working: Bool { chat.activity == "running" }

    var body: some View {
        VStack(spacing: 0) {
            if case let .offline(why) = chat.phase {
                Label(why, systemImage: "wifi.slash")
                    .font(.footnote)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(Color.gray.opacity(0.25))
            }
            ScrollViewReader { proxy in
                ZStack(alignment: .bottomTrailing) {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            if chat.entries.isEmpty { emptyState }
                            ChatRows(items: items, cwd: Self.cwd, working: working, receipts: chat.receipts)
                            if !chat.streaming.isEmpty {
                                MarkdownText(text: chat.streaming).opacity(0.8)
                            }
                            if working && chat.streaming.isEmpty && chat.pending.isEmpty {
                                ProgressView().frame(maxWidth: .infinity, alignment: .leading)
                            }
                            ChatRows(items: queued, cwd: Self.cwd, working: working, receipts: chat.receipts)
                            Color.clear.frame(height: 1).id("bottom")
                                .onAppear { atBottom = true }
                                .onDisappear { atBottom = false }
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                    }
                    // Made again once the history is in, so it opens at the
                    // bottom by its default anchor. Scrolling a lazy stack
                    // there instead lands by guessed row heights, short of
                    // or past the rows it has drawn, and it shows nothing
                    // until it is scrolled by hand.
                    .id(items.isEmpty)
                    .onChange(of: chat.entries.count, initial: true) {
                        let had = !items.isEmpty
                        (items, queued) = ChatItem.fold(chat.entries)
                        agents = AgentInfo.all(chat.entries)
                        if had, atBottom {
                            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                        }
                    }
                    .onChange(of: chat.streaming) {
                        if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
                    }
                    .scrollDismissesKeyboard(.interactively)
                    .defaultScrollAnchor(.bottom)
                    if !atBottom {
                        Button {
                            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                        } label: {
                            Image(systemName: "arrow.down")
                                .font(.headline)
                                .padding(10)
                                .background(.ultraThinMaterial, in: Circle())
                        }
                        .padding(14)
                    }
                }
            }
            // The oldest first; the rest wait their turn, as in the terminal.
            if let ask = chat.pending.first {
                ScrollView {
                    AskCard(ask: ask, enabled: answering == nil && chat.phase == .live) { answer(ask, $0) }
                        .id(ask.id)
                    if chat.pending.count > 1 {
                        Text("\(chat.pending.count - 1) more after this").font(.caption).foregroundStyle(.secondary)
                            .padding(.bottom, 6)
                    }
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(maxHeight: 420)
                .fixedSize(horizontal: false, vertical: true)
            }
            if !agents.isEmpty {
                AgentsStrip(agents: agents) { following = Following(id: $0.id) }
            }
            if !suggestions.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack {
                        ForEach(suggestions, id: \.self) { c in
                            Button("/\(c)") { draft = "/\(c) " }
                                .font(.caption.monospaced())
                                .buttonStyle(.bordered)
                        }
                    }
                    .padding(.horizontal, 10)
                }
                .padding(.vertical, 4)
            }
            composer
        }
        .background(Color.black)
        // Dark whatever the system setting: the rows and tool cards are drawn
        // for a dark background.
        .environment(\.colorScheme, .dark)
        .sheet(item: $following) { f in AgentScreen(chat: chat, id: f.id) }
        .alert("Couldn't do that", isPresented: Binding(get: { chat.error != nil }, set: { if !$0 { chat.error = nil } })) {
            Button("OK") { chat.error = nil }
        } message: {
            Text(chat.error ?? "")
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        VStack(spacing: 8) {
            if let notice = chat.notice {
                Text(notice)
            } else if chat.loaded {
                Text("No messages yet").font(.headline)
                Text("Send Claude a message below.")
            } else {
                ProgressView()
            }
        }
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .padding(.top, 60)
    }

    /// Slash commands matching what is being typed.
    private var suggestions: [String] {
        guard draft.hasPrefix("/"), !draft.contains(" ") else { return [] }
        let typed = draft.dropFirst()
        return Array(chat.commands.filter { $0.hasPrefix(typed) }.prefix(8))
    }

    /// Claude Code's permission modes, in the desktop window's words.
    static let modes: [(value: String, label: String, hint: String)] = [
        ("auto", "Auto", "Decides what to ask"),
        ("default", "Ask", "Before edits and commands"),
        ("acceptEdits", "Accept Edits", "Asks for the rest"),
        ("plan", "Plan", "Changes nothing"),
    ]

    /// A reasoning effort as Claude Code's `/effort` says it.
    static func effortName(_ e: String) -> String {
        e == "xhigh" ? "Extra High" : e.prefix(1).uppercased() + e.dropFirst()
    }

    private var settingsBar: some View {
        HStack(spacing: 12) {
            Menu {
                Picker("Permission mode", selection: Binding(get: { chat.mode ?? "auto" }, set: { m in
                    Task { do { try await chat.setMode(m) } catch { chat.error = error.localizedDescription } }
                })) {
                    ForEach(Self.modes, id: \.value) { m in
                        Text("\(m.label) — \(m.hint)").tag(m.value)
                    }
                }
            } label: {
                Label(Self.modes.first { $0.value == (chat.mode ?? "auto") }?.label ?? (chat.mode ?? "Auto"),
                      systemImage: "shield.lefthalf.filled")
            }
            if !chat.models.isEmpty {
                Menu {
                    Picker("Model", selection: Binding(get: { chat.model ?? chat.models[0].value }, set: { m in
                        Task { do { try await chat.setModel(m) } catch { chat.error = error.localizedDescription } }
                    })) {
                        ForEach(chat.models) { m in
                            Text(m.displayName).tag(m.value)
                        }
                    }
                } label: {
                    Label(chat.model.map(modelName) ?? chat.models[0].displayName, systemImage: "cpu")
                }
            }
            if !chat.efforts.isEmpty {
                Menu {
                    Picker("Effort", selection: Binding(get: { chat.effort ?? "" }, set: { e in
                        Task { do { try await chat.setEffort(e.isEmpty ? nil : e) } catch { chat.error = error.localizedDescription } }
                    })) {
                        Text("Model's Default").tag("")
                        ForEach(chat.efforts, id: \.self) { e in
                            Text(Self.effortName(e)).tag(e)
                        }
                    }
                } label: {
                    Label(chat.effort.map(Self.effortName) ?? "Effort", systemImage: "brain")
                }
            }
            Spacer()
            if chat.status == "compacting" { Text("compacting") }
            if let ctx = chat.contextPercentage { Text("ctx \(Int(ctx.rounded()))%") }
            if let cost = chat.costUSD, cost > 0 { Text(String(format: "$%.2f", cost)) }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .lineLimit(1)
        .padding(.horizontal, 14)
        .padding(.top, 6)
    }

    private var composer: some View {
        VStack(spacing: 0) {
        settingsBar
        if !attached.isEmpty {
            AttachedStrip(files: attached) { f in attached.removeAll { $0.id == f.id } }
        }
        HStack(alignment: .bottom, spacing: 8) {
            // Only for a host that keeps them: an older one drops them.
            if chat.takesFiles {
                Menu {
                    Button("Photo Library", systemImage: "photo.on.rectangle") { choosingPhotos = true }
                    Button("Files", systemImage: "folder") { choosingFiles = true }
                } label: {
                    Image(systemName: "plus.circle.fill").font(.system(size: 30))
                }
                .tint(.secondary)
                .accessibilityLabel("Attach")
            }
            TextField("Message Claude", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                .focused($composing)
                .onChange(of: draft) {
                    guard let just = justSent else { return }
                    if Date.now.timeIntervalSince(just.at) < 1,
                       draft.trimmingCharacters(in: .whitespacesAndNewlines) == just.text {
                        draft = ""
                    } else {
                        justSent = nil
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 18))
            if working && draft.isEmpty && attached.isEmpty {
                Button {
                    Task { try? await chat.interrupt() }
                } label: {
                    Image(systemName: "stop.circle.fill").font(.system(size: 30))
                }
                .tint(.red)
                .accessibilityLabel("Stop Claude")
            } else {
                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 30))
                }
                .disabled((draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attached.isEmpty) || sending)
                .accessibilityLabel("Send")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        }
        .disabled(chat.phase != .live)
        .background(.bar)
        .photosPicker(isPresented: $choosingPhotos, selection: $photos, maxSelectionCount: 10, matching: .images)
        .onChange(of: photos) {
            let picked = photos
            photos = []
            Task {
                for (i, item) in picked.enumerated() {
                    guard let data = try? await item.loadTransferable(type: Data.self),
                          let photo = Attachments.photo(data, name: "photo-\(attached.count + i + 1).jpg")
                    else {
                        chat.error = "That photo couldn't be read."
                        continue
                    }
                    attached.append(photo)
                }
            }
        }
        .fileImporter(isPresented: $choosingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            do {
                attached += try result.get().map(Attachments.file)
            } catch {
                chat.error = error.localizedDescription
            }
        }
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let files = attached
        guard !text.isEmpty || !files.isEmpty else { return }
        // Emptied now rather than once the server has it, so nothing typed
        // during the round trip is wiped with it; put back if it fails and
        // nothing new has been typed.
        draft = ""
        attached = []
        justSent = (text, .now)
        sending = true
        Task {
            defer { sending = false }
            do {
                try await chat.send(text, files: files)
            } catch {
                justSent = nil
                if draft.isEmpty { draft = text }
                if attached.isEmpty { attached = files }
                chat.error = error.localizedDescription
            }
        }
    }

    private func answer(_ ask: Ask, _ decision: ChatDecision) {
        answering = ask.id
        Task {
            defer { answering = nil }
            do { try await chat.answer(ask.id, decision: decision) } catch { chat.error = error.localizedDescription }
        }
    }
}

/// A model id as a person would say it: `claude-opus-5-5[1m]` is Opus 5.5
/// (1M). Anything that doesn't look like an id is left as it is.
func modelName(_ id: String) -> String {
    guard let m = id.wholeMatch(of: /claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?(\[1m\])?/) else { return id }
    let family = m.1.prefix(1).uppercased() + m.1.dropFirst()
    let name = "\(family) \(m.2)\(m.3.map { ".\($0)" } ?? "")"
    return m.4 == nil ? name : "\(name) (1M)"
}

/// A subagent to follow, as `sheet(item:)` wants one.
private struct Following: Identifiable {
    var id: String
}
