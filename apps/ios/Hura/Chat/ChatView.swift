import HuraKit
import PhotosUI
import SwiftUI

/// A chat session's conversation: the transcript, the reply as it is
/// written, what is waiting on you, and a composer. hura runs Claude Code
/// through the Agent SDK, so a permission request is a request with an id,
/// answered with a button.
struct ChatView: View {
    let chat: Chat
    /// The sandbox is still being made. The conversation cannot be reached
    /// until it is, and that is not news: the chat says what is happening, so
    /// a banner saying the server cannot be reached would be a false alarm.
    var preparing = false
    /// The open pull request the session is on: a bar over the composer.
    var pull: PullRequestSummary?
    /// The session's figures, asked for when the context ring is opened: the
    /// account's limits are in a poll, not in the conversation.
    var usage: (() async -> Usage?)?
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
    /// The last thing Claude did is a tool call still waiting on its result.
    private var callRunning: Bool {
        if case let .tool(_, call)? = items.last { call.result == nil } else { false }
    }
    /// The newest thing is a `!` command still running, whose card says so.
    private var shellRunning: Bool {
        if case let .shell(_, run)? = items.last { run.running } else { false }
    }
    /// What is being typed, as Claude Code's terminal reads it.
    private var input: ComposerInput {
        ComposerInput.parse(draft, commands: chat.commands, shell: chat.runsShell)
    }
    private var isShell: Bool {
        if case .shell = input { true } else { false }
    }
    /// A command, which the keyboard mustn't correct: `wc` became "we".
    private var isCode: Bool { isShell || ComposerInput.naming(draft) != nil }

    var body: some View {
        VStack(spacing: 0) {
            if !preparing, case let .offline(why) = chat.phase {
                Label(why, glyph: .offline)
                    .font(.footnote)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(Color.gray.opacity(0.25))
            }
            ScrollViewReader { proxy in
                ZStack(alignment: .bottomTrailing) {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            if items.isEmpty && queued.isEmpty { emptyState }
                            ChatRows(items: ChatItem.group(items, working: working), cwd: Self.cwd, working: working,
                                     receipts: chat.receipts)
                            if !chat.streaming.isEmpty {
                                MarkdownText(text: chat.streaming).opacity(0.8)
                            }
                            // Under whatever is being written, for the whole
                            // turn, as on the desktop; not while a tool call
                            // is running, whose card says so already. The word
                            // only while there is nothing else to look at.
                            if working && chat.pending.isEmpty && !callRunning && !shellRunning {
                                HStack(spacing: 6) {
                                    WorkingDots(size: 22)
                                    if chat.streaming.isEmpty { Text("working").font(.caption) }
                                }
                                .foregroundStyle(Color.working)
                            }
                            ChatRows(items: queued, cwd: Self.cwd, working: working, receipts: chat.receipts)
                            Color.clear.frame(height: 1).id("bottom")
                                .onAppear { atBottom = true }
                                // Not the empty one's, going as the history
                                // arrives: that came after the new one's
                                // appearing, and left the arrow up for good.
                                .onDisappear { [empty = items.isEmpty] in
                                    if items.isEmpty == empty { atBottom = false }
                                }
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
                    .onChange(of: chat.entries.count, initial: true) { fold(proxy) }
                    .onChange(of: chat.loaded) { fold(proxy) }
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
                                .frame(width: 40, height: 40)
                                .contentShape(Circle())
                                .glass(in: Circle(), interactive: true)
                        }
                        .buttonStyle(.plain)
                        .padding(14)
                    }
                }
                // Over the transcript rather than under it, which scrolls on
                // beneath the glass.
                .safeAreaInset(edge: .bottom, spacing: 0) { bottom }
            }
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

    /// The transcript as rows, once it is all here. A long one replays in
    /// several frames, oldest first; drawing each as it came scrolled the
    /// whole history past on the way to the newest. Until the chat says it
    /// is loaded there is a spinner, and then the rows arrive at once and
    /// the stack opens at the bottom. After that a new entry follows the
    /// bottom only for someone already there.
    private func fold(_ proxy: ScrollViewProxy) {
        guard chat.loaded else { return }
        let had = !items.isEmpty
        (items, queued) = ChatItem.fold(chat.entries)
        agents = AgentInfo.all(chat.entries)
        if had, atBottom {
            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
        }
    }

    /// What floats over the end of the transcript: what is waiting on you,
    /// the subagents, the pull request, and the composer, each its own glass.
    private var bottom: some View {
        VStack(spacing: 8) {
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
                .clipShape(RoundedRectangle(cornerRadius: 24))
                .glass(in: RoundedRectangle(cornerRadius: 24))
                .padding(.horizontal, 12)
            }
            if !agents.isEmpty {
                AgentsStrip(agents: agents) { following = Following(id: $0.id) }
            }
            if let pull { PullBar(pull: pull) }
            if !suggestions.isEmpty {
                CommandSuggestions(commands: suggestions) { draft = "/\($0.name) " }
                    .padding(.vertical, 4)
                    .clipShape(RoundedRectangle(cornerRadius: 22))
                    .glass(in: RoundedRectangle(cornerRadius: 22))
                    .padding(.horizontal, 12)
            }
            composer
        }
        .padding(.top, 8)
        .padding(.bottom, 6)
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

    /// Slash commands that fit the name being typed, best first.
    private var suggestions: [SlashCommand] {
        guard let typed = ComposerInput.naming(draft) else { return [] }
        return SlashCommand.matching(typed, in: chat.commands)
    }

    /// What sending will do, when it isn't to say something to Claude: run a
    /// shell command, or a slash command, which says what it does once the
    /// list of them has gone.
    @ViewBuilder
    private var modeNote: some View {
        switch input {
        case .shell:
            Label("Runs in the working copy. Claude sees what it prints with your next message.", glyph: .agentTerminal)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.top, 12)
        case let .command(_, known?, _) where suggestions.isEmpty:
            HStack(spacing: 6) {
                Text("/\(known.name)")
                    .font(.caption.monospaced().weight(.semibold))
                    .foregroundStyle(Color.accentColor)
                if !known.argumentHint.isEmpty {
                    Text(known.argumentHint).font(.caption.monospaced()).foregroundStyle(.tertiary)
                }
                Text(known.description).font(.caption).foregroundStyle(.secondary)
            }
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
        default:
            EmptyView()
        }
    }

    private var canSend: Bool {
        if case let .shell(line) = input { return !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return !(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attached.isEmpty)
    }

    /// Claude Code's permission modes, in the desktop window's words, and
    /// shorter for the composer, where they share a line with the model.
    static let modes: [(value: String, label: String, short: String, hint: String)] = [
        ("auto", "Auto", "Auto", "Decides what to ask"),
        ("default", "Ask", "Ask", "Before edits and commands"),
        ("acceptEdits", "Accept Edits", "Edits", "Asks for the rest"),
        ("plan", "Plan", "Plan", "Changes nothing"),
    ]

    /// A reasoning effort as Claude Code's `/effort` says it.
    static func effortName(_ e: String) -> String {
        e == "xhigh" ? "Extra High" : e.prefix(1).uppercased() + e.dropFirst()
    }

    /// The model and its effort, in one menu, as Claude Code's app has them:
    /// "Opus 5.5 Medium".
    @ViewBuilder
    private var modelMenu: some View {
        if !chat.models.isEmpty || !chat.efforts.isEmpty {
            Menu {
                if !chat.models.isEmpty {
                    Section("Model") {
                        Picker("Model", selection: Binding(get: { chat.model ?? chat.models[0].value }, set: { m in
                            Task { do { try await chat.setModel(m) } catch { chat.error = error.localizedDescription } }
                        })) {
                            ForEach(chat.models) { m in
                                Text(m.displayName).tag(m.value)
                            }
                        }
                        .pickerStyle(.inline)
                    }
                }
                if !chat.efforts.isEmpty {
                    Section("Effort") {
                        Picker("Effort", selection: Binding(get: { chat.effort ?? "" }, set: { e in
                            Task { do { try await chat.setEffort(e.isEmpty ? nil : e) } catch { chat.error = error.localizedDescription } }
                        })) {
                            Text("Model's Default").tag("")
                            ForEach(chat.efforts, id: \.self) { e in
                                Text(Self.effortName(e)).tag(e)
                            }
                        }
                        .pickerStyle(.inline)
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    if let model = modelLabel {
                        Text(model).foregroundStyle(.primary)
                    }
                    if let effort = chat.effort {
                        Text(Self.effortName(effort)).foregroundStyle(.secondary)
                    }
                }
                .modifier(Chip())
            }
            // The model first, as the pill says it, though the menu opens
            // upwards from the bottom of the screen.
            .menuOrder(.fixed)
            .accessibilityLabel("Model and effort")
        }
    }

    /// The model as its menu names it: the session's model is often an
    /// alias, `sonnet`, which only the list says is Sonnet 5.5.
    private var modelLabel: String? {
        guard let model = chat.model else { return chat.models.first?.displayName }
        return chat.models.first { $0.value == model }?.displayName ?? modelName(model)
    }

    private var modeMenu: some View {
        let mode = Self.modes.first { $0.value == (chat.mode ?? "auto") }
        return Menu {
            Picker("Permission mode", selection: Binding(get: { chat.mode ?? "auto" }, set: { m in
                Task { do { try await chat.setMode(m) } catch { chat.error = error.localizedDescription } }
            })) {
                ForEach(Self.modes, id: \.value) { m in
                    Text("\(m.label) — \(m.hint)").tag(m.value)
                }
            }
        } label: {
            Label(mode?.short ?? (chat.mode ?? "Auto"), glyph: .policy)
                .labelStyle(TightLabel())
                .foregroundStyle(chat.mode == "plan" ? Color.accentColor : .primary)
                .modifier(Chip())
                .fixedSize()
        }
        .accessibilityLabel("Permission mode, \(mode?.label ?? chat.mode ?? "Auto")")
    }

    private var attachMenu: some View {
        Menu {
            Button("Photo Library", systemImage: "photo.on.rectangle") { choosingPhotos = true }
            Button("Files", glyph: .folder) { choosingFiles = true }
        } label: {
            Image(systemName: "plus")
                .font(.body.weight(.medium))
                .foregroundStyle(.primary)
                .frame(width: 34, height: 34)
                .background(Color.white.opacity(0.1), in: Circle())
                .contentShape(Circle())
        }
        .accessibilityLabel("Attach")
    }

    /// Stop while Claude works and nothing is typed; send otherwise.
    @ViewBuilder
    private var sendButton: some View {
        if working && draft.isEmpty && attached.isEmpty {
            Button {
                Task { try? await chat.interrupt() }
            } label: {
                Image(systemName: "stop.fill")
                    .font(.footnote)
                    .foregroundStyle(.white)
                    .frame(width: 34, height: 34)
                    .background(Color.red, in: Circle())
            }
            .accessibilityLabel("Stop Claude")
        } else {
            Button(action: send) {
                Image(systemName: "arrow.up")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(.white)
                    .frame(width: 34, height: 34)
                    .background(Color.accentColor, in: Circle())
                    .opacity(!canSend || sending ? 0.4 : 1)
            }
            .disabled(!canSend || sending)
            .accessibilityLabel(isShell ? "Run" : "Send")
        }
    }

    /// One glass card, as Claude Code's app has it: what is attached, the
    /// message, and under it a line of what it goes with -- files, the model
    /// and its effort, the permission mode, how full the context is -- and
    /// send.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !attached.isEmpty {
                AttachedStrip(files: attached) { f in attached.removeAll { $0.id == f.id } }
            }
            modeNote
            TextField("Message Claude", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                // A `!` command is code, and set as code.
                .font(isShell ? Font.body.monospaced() : Font.body)
                .autocorrectionDisabled(isCode)
                .textInputAutocapitalization(isCode ? TextInputAutocapitalization.never : .sentences)
                .focused($composing)
                .onChange(of: draft) {
                    // Emptying it is the send itself, and mustn't end the
                    // watch for the keyboard putting it back.
                    guard let just = justSent, !draft.isEmpty else { return }
                    if Date.now.timeIntervalSince(just.at) < 1, ComposerInput.isEcho(draft, of: just.text) {
                        draft = ""
                    } else {
                        justSent = nil
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 10)
            HStack(spacing: 6) {
                // Only for a host that keeps them: an older one drops them.
                if chat.takesFiles { attachMenu }
                modelMenu
                modeMenu
                Spacer(minLength: 0)
                if chat.status == "compacting" {
                    Text("compacting").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                if chat.loaded {
                    ContextRing(percentage: chat.contextPercentage, size: chat.contextSize, usage: usage)
                }
                sendButton
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
        }
        .glass(in: RoundedRectangle(cornerRadius: 26))
        .overlay(RoundedRectangle(cornerRadius: 26).strokeBorder(isShell ? Color.accentColor : .clear, lineWidth: 1))
        .padding(.horizontal, 12)
        .disabled(chat.phase != .live)
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
        if case let .shell(line) = input {
            run(line)
            return
        }
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

    /// A `!` command, for the sandbox's shell. Files wait for the next
    /// message: a command doesn't take them.
    private func run(_ line: String) {
        let command = line.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !command.isEmpty else { return }
        draft = ""
        justSent = (text, .now)
        sending = true
        Task {
            defer { sending = false }
            do {
                try await chat.shell(command)
            } catch {
                justSent = nil
                if draft.isEmpty { draft = text }
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

/// The slash commands that fit what is typed: each one's name and what it
/// takes, then what it does. A tap completes the name.
private struct CommandSuggestions: View {
    let commands: [SlashCommand]
    let pick: (SlashCommand) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(commands) { c in
                    Button { pick(c) } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text("/\(c.name)").layoutPriority(1)
                                if !c.argumentHint.isEmpty {
                                    Text(c.argumentHint).foregroundStyle(.tertiary)
                                }
                            }
                            .font(.subheadline.monospaced())
                            .lineLimit(1)
                            if !c.description.isEmpty || !c.aliases.isEmpty {
                                Text(about(c))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    if c.id != commands.last?.id {
                        Divider().padding(.leading, 14)
                    }
                }
            }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxHeight: 240)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func about(_ c: SlashCommand) -> String {
        guard !c.aliases.isEmpty else { return c.description }
        let also = "also " + c.aliases.map { "/\($0)" }.joined(separator: ", ")
        return c.description.isEmpty ? also : "\(c.description) (\(also))"
    }
}

/// A pill in the composer's line, a shade lighter than the glass it is on.
private struct Chip: ViewModifier {
    func body(content: Content) -> some View {
        content
            .font(.subheadline)
            .lineLimit(1)
            .padding(.horizontal, 12)
            .frame(height: 34)
            .background(Color.white.opacity(0.1), in: Capsule())
            .contentShape(Capsule())
    }
}

/// A label's icon close to its title, for a pill.
private struct TightLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon.imageScale(.small)
            configuration.title
        }
    }
}

/// A subagent to follow, as `sheet(item:)` wants one.
private struct Following: Identifiable {
    var id: String
}
