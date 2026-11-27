import HuraKit
import PhotosUI
import SwiftUI

/// A chat session's conversation: the transcript, the reply as it is
/// written, what is waiting on you, and a composer. hura runs Claude Code
/// through the Agent SDK, so a permission request is a request with an id,
/// answered with a button.
/// An entry a search found, for a conversation to open at, and which time:
/// a second tap on the same result goes there again.
struct ChatJump: Equatable {
    var seq: UInt64
    var nonce: Int
}

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
    /// An entry a search found here, to open the conversation at. Taken, and
    /// `onJumped` said, so that coming back later does not go there again.
    var jump: ChatJump?
    var onJumped: (() -> Void)?
    /// Messages waiting on the server to be said to this session's agent.
    var waiting: [QueuedMessage] = []
    /// Keep a message on the server to say at a time, and again by a rule if
    /// one is given. nil hides the clock.
    var onSchedule: ((String, Date, Repeat?) async throws -> Void)?
    /// The server's offset from UTC, from its queue: a repeat's time of day is
    /// on its clock. nil from a server too old to repeat, which is then not
    /// offered.
    var utcOffset: Int?
    /// Take one of `waiting` back.
    var onUnqueue: ((QueuedMessage) -> Void)?
    /// Where this conversation's draft is kept on the phone (`Drafts.key`).
    var draftKey: String?
    /// Paths in the working copy for what is typed after `@`.
    var findFiles: ((String) async throws -> FoundPaths)?
    /// The end of a file in the sandbox: a background command's output.
    var tailFile: ((String) async throws -> TailText)?
    /// Say something else in place of an earlier message: a new conversation
    /// forked at `at`, the uuid of what the agent said last before it (nil
    /// for a plain new one), with `text` said in it. Unset where a
    /// conversation cannot fork, such as a side chat.
    var onBranch: ((_ at: String?, _ text: String) -> Void)?
    /// Ask a question on the side (`/btw`). Unset in a side chat itself.
    var onSideChat: ((String) -> Void)?
    /// To say as soon as the conversation is open, once: a side chat's
    /// question, an edited message's new words. `onSaidFirst` takes it.
    var sayFirst: String?
    var onSaidFirst: (() -> Void)?
    @State private var target: ChatJump?
    /// While a message is being written for later: the time it is for.
    @State private var later: Date?
    /// And how it comes round again; nil is once.
    @State private var cadence: Cadence?
    @State private var found: FoundRow?
    @State private var draft = ""
    @State private var sending = false
    /// What was just sent, and when: the keyboard commits a pending
    /// prediction into the field after it is emptied, which put the message
    /// back. For a moment after a send, that is not typing.
    @State private var justSent: (text: String, at: Date)?
    @State private var answering: String?
    @State private var atBottom = true
    /// How tall the transcript and what floats over it may be: the screen
    /// under the navigation bar, less the keyboard when it is up.
    @State private var room: CGFloat = 0
    @State private var end = ListEnd()
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
    /// The commands the agent left running in the background.
    @State private var commands: [BackgroundCommand] = []
    /// The subagent being followed, by task id.
    @State private var following: Following?
    /// The background command being followed, by its id.
    @State private var followingCommand: Following?
    /// The message being edited, by its entry's seq: sending forks the
    /// conversation from just before it.
    @State private var branchFrom: UInt64?
    /// What the server found for the last `@` asked about.
    @State private var mentioned: (query: String, paths: [String], error: String?)?
    /// The stash is open in the drawer.
    @State private var stashOpen = false
    @FocusState private var composing: Bool
    private let stash = PromptStash.shared

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
    /// The commands on offer: Claude Code's, and the side chat where there
    /// can be one.
    private var offered: [SlashCommand] {
        onSideChat == nil ? chat.commands : [sideCommand] + chat.commands
    }
    /// What is being typed, as Claude Code's terminal reads it.
    private var input: ComposerInput {
        ComposerInput.parse(draft, commands: offered, shell: chat.runsShell)
    }
    private var isShell: Bool {
        if case .shell = input { true } else { false }
    }
    /// A command, which the keyboard mustn't correct: `wc` became "we".
    private var isCode: Bool { isShell || ComposerInput.naming(draft) != nil }
    /// `/btw` and the question.
    private var sideQuestion: String? {
        guard onSideChat != nil, case let .command(name, _, rest) = input,
              name == sideCommand.name || sideCommand.aliases.contains(name) else { return nil }
        return rest.trimmingCharacters(in: .whitespacesAndNewlines)
    }
    /// Sending would steer: while a turn runs, a message waits for it to end,
    /// and a long press on send stops it and says this now instead.
    private var steers: Bool {
        working && branchFrom == nil && later == nil && !isShell && sideQuestion == nil
    }
    /// The `@` being typed, while there is a server to ask.
    private var mention: Mention? {
        guard findFiles != nil, later == nil, !isShell else { return nil }
        return ComposerInput.mention(draft)
    }

    var body: some View {
        let grouped = ChatItem.group(items, working: working)
        let started = working ? ChatTranscript.turnStart(chat.entries) : nil
        // The clock is on the work log's line while it has one.
        let liveFold = grouped.contains { if case .steps(_, _, true) = $0 { true } else { false } }
        VStack(spacing: 0) {
            if !preparing, case let .offline(why) = chat.phase {
                Label(why, glyph: .offline)
                    .font(.footnote)
                    .foregroundStyle(Color.dim)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(Color.surface2)
            }
            ScrollViewReader { proxy in
                ZStack(alignment: .bottomTrailing) {
                    // A list, not a lazy stack in a scroll view. The stack
                    // guessed the height of rows it had not drawn, and when
                    // a turn ended and its steps folded shut under the screen
                    // it lost its place: it drew nothing, wherever it was
                    // scrolled, until the next message came.
                    List {
                        Group {
                            if items.isEmpty && queued.isEmpty && waiting.isEmpty { emptyState }
                            ChatRows(items: grouped, cwd: Self.cwd, working: working, receipts: chat.receipts)
                            // Under whatever is being written, for the whole
                            // turn, as on the desktop; not the dots while a
                            // tool call is running, whose line has its own,
                            // but still how long the turn has taken.
                            Writing(chat: chat,
                                    showsWorking: working && chat.pending.isEmpty && !callRunning && !shellRunning,
                                    started: working && chat.pending.isEmpty && !shellRunning && !liveFold
                                        ? started : nil)
                            ChatRows(items: queued, cwd: Self.cwd, working: working, receipts: chat.receipts)
                            // What is waiting on the server to be said, last,
                            // on your side: it is what you will say next.
                            ForEach(waiting) { m in
                                WaitingBubble(message: m, utcOffset: utcOffset,
                                              edit: onUnqueue.map { unqueue in { unqueue(m); takeBack(m) } },
                                              remove: onUnqueue.map { unqueue in { unqueue(m) } })
                            }
                            Color.clear.frame(height: 1).id("bottom")
                                .onAppear { atBottom = true }
                                // Not the empty one's, going as the history
                                // arrives: that came after the new one's
                                // appearing, and left the arrow up for good.
                                .onDisappear { [empty = items.isEmpty] in
                                    if items.isEmpty == empty { atBottom = false }
                                }
                        }
                        .listRowInsets(EdgeInsets(top: 6, leading: 14, bottom: 6, trailing: 14))
                        .listRowSeparator(.hidden)
                        .listRowBackground(Color.clear)
                        .environment(\.foundRow, found)
                        .environment(\.turnStarted, started)
                        .environment(\.editPrompt, onBranch == nil ? nil : EditPrompt { seq, text in edit(from: seq, text) })
                        .environment(\.backgroundCommands, tailFile == nil ? nil : BackgroundCommands(all: commands) {
                            followingCommand = Following(id: $0.id)
                        })
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .environment(\.defaultMinListRowHeight, 0)
                    .contentMargins(.vertical, 6, for: .scrollContent)
                    // A button in a row with no style of its own is the
                    // whole row's, in a list.
                    .buttonStyle(.borderless)
                    .onChange(of: chat.entries.count, initial: true) { fold(proxy) }
                    .onChange(of: chat.loaded) { fold(proxy) }
                    .onChange(of: jump, initial: true) {
                        guard let jump else { return }
                        target = jump
                        onJumped?()
                    }
                    .onChange(of: target) { fold(proxy) }
                    .onChange(of: waiting.count) { if atBottom { end.scroll() } }
                    .background {
                        FollowsReply(chat: chat) {
                            if atBottom { end.scroll() }
                        }
                    }
                    .scrollDismissesKeyboard(.interactively)
                    .background { ListEnd.Finder(end: end) }
                    if !atBottom {
                        Button {
                            end.scroll(animated: true)
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
                // beneath the composer.
                .safeAreaInset(edge: .bottom, spacing: 0) { bottom }
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { room = $0 }
            }
        }
        // T3 Code's canvas: black in dark, near white in light, with the
        // cards and your bubble a step up from it.
        .background(Color.canvas)
        .sheet(item: $following) { f in AgentScreen(chat: chat, id: f.id) }
        .sheet(item: $followingCommand) { f in
            if let tailFile { CommandScreen(chat: chat, id: f.id, tail: tailFile) }
        }
        .alert("Couldn't do that", isPresented: Binding(get: { chat.error != nil }, set: { if !$0 { chat.error = nil } })) {
            Button("OK") { chat.error = nil }
        } message: {
            Text(chat.error ?? "")
        }
        // A draft left here last time, unless something is already typed.
        .task(id: draftKey) {
            if let draftKey, draft.isEmpty { draft = Drafts().read(draftKey) }
        }
        .onChange(of: draft) {
            if let draftKey { Drafts().write(draftKey, draft) }
        }
        // A side chat's question or an edited message's words, said as soon
        // as the conversation is there to hear them.
        .task(id: chat.loaded && chat.phase == .live) {
            guard chat.loaded, chat.phase == .live, let words = sayFirst else { return }
            onSaidFirst?()
            do { try await chat.send(words) } catch { chat.error = error.userMessage }
        }
        // What the server has for the `@` being typed, a moment after typing
        // stops.
        .task(id: mention?.query) {
            guard let query = mention?.query, let findFiles else { return }
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            do {
                mentioned = (query, try await findFiles(query).paths, nil)
            } catch {
                guard !Task.isCancelled else { return }
                let said = error.userMessage
                mentioned = (query, [], said.firstMatch(of: /(?i)unsupported|unknown variant/) != nil
                    ? "This server can't look for files yet: update hurad." : said)
            }
        }
    }

    /// The transcript as rows, once it is all here. A long one replays in
    /// several frames, oldest first; drawing each as it came scrolled the
    /// whole history past on the way to the newest. Until the chat says it
    /// is loaded there is a spinner, and then the rows arrive at once and
    /// the list opens at the bottom. After that a new entry follows the
    /// bottom only for someone already there.
    private func fold(_ proxy: ScrollViewProxy) {
        guard chat.loaded else { return }
        let had = !items.isEmpty
        (items, queued) = ChatItem.fold(chat.entries)
        agents = AgentInfo.all(chat.entries)
        commands = tailFile == nil ? [] : BackgroundCommand.all(chat.entries)
        if let target, found?.nonce != target.nonce,
           let at = ChatItem.locate(ChatItem.group(items, working: working), seq: target.seq) {
            found = FoundRow(id: at.id, nonce: target.nonce)
            atBottom = false
            ChatView.reveal(at.id, in: proxy)
        } else if !had || atBottom {
            end.scroll()
        }
    }

    /// Scrolls a row to the middle: once the stack has been made again with
    /// the history in it, and again once a turn opening around the row, or
    /// rows above it measuring themselves, has moved it.
    static func reveal(_ id: String, in proxy: ScrollViewProxy) {
        Task { @MainActor in
            for wait in [150, 450] {
                try? await Task.sleep(for: .milliseconds(wait))
                proxy.scrollTo(id, anchor: .center)
            }
        }
    }

    /// The most a question, plan or permission may take of the screen. A
    /// fixed height was more than there is with the keyboard up, and the card
    /// pushed past both ends: its question went under the navigation bar and
    /// the composer under the keyboard. A share of the room there is instead,
    /// within which the card scrolls, from its question down.
    private var askHeight: CGFloat {
        guard room > 0 else { return 420 }
        return min(420, max(120, room * 0.6))
    }

    /// What floats over the end of the transcript: what is waiting on you,
    /// the work going on beside the conversation, the pull request, and the
    /// composer.
    private var bottom: some View {
        VStack(spacing: 8) {
            // The oldest first; the rest wait their turn, as in the terminal.
            if let ask = chat.pending.first {
                ScrollView {
                    AskCard(ask: ask, enabled: answering == nil && chat.phase == .live) { answer(ask, $0) }
                        .id(ask.id)
                    if chat.pending.count > 1 {
                        Text("\(chat.pending.count - 1) more after this").font(.caption).foregroundStyle(Color.dim)
                            .padding(.bottom, 6)
                    }
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(maxHeight: askHeight)
                .fixedSize(horizontal: false, vertical: true)
                .clipShape(RoundedRectangle(cornerRadius: 24))
                .glass(in: RoundedRectangle(cornerRadius: 24))
                .padding(.horizontal, 12)
            }
            if !agents.isEmpty || !commands.isEmpty {
                AgentsStrip(agents: agents, commands: commands) { following = Following(id: $0.id) } openCommand: {
                    followingCommand = Following(id: $0.id)
                }
            }
            if let pull { PullBar(pull: pull) }
            if !stash.all.isEmpty { stashTab }
            composer
        }
        .padding(.top, 8)
        .padding(.bottom, 6)
    }

    /// The tab over the box that opens the stash, while it has something in
    /// it, as T3 Code has it.
    private var stashTab: some View {
        HStack {
            Button {
                withAnimation(.snappy) { stashOpen.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "tray.full")
                    Text("Stash")
                    Text("\(stash.all.count)").foregroundStyle(Color.dimmer)
                    Image(systemName: "chevron.up")
                        .font(.caption2.weight(.semibold))
                        .rotationEffect(.degrees(stashOpen ? 180 : 0))
                }
                .font(.caption)
                .foregroundStyle(stashOpen ? Color.ink : Color.dim)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Color.surface, in: Capsule())
                .overlay(Capsule().strokeBorder(Color.line, lineWidth: 1))
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Prompts put aside, \(stash.all.count)")
            Spacer()
        }
        .padding(.horizontal, 16)
    }

    @ViewBuilder
    private var emptyState: some View {
        VStack(spacing: 8) {
            if let notice = chat.notice {
                Text(notice)
            } else if chat.loaded {
                Text("No messages yet").font(.headline).foregroundStyle(Color.ink)
                Text("Send Claude a message below.")
            } else {
                ProgressView()
            }
        }
        .font(.subheadline)
        .foregroundStyle(Color.dim)
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .padding(.top, 60)
    }

    /// Slash commands that fit the name being typed, best first.
    private var suggestions: [SlashCommand] {
        guard let typed = ComposerInput.naming(draft) else { return [] }
        return SlashCommand.matching(typed, in: offered)
    }

    /// What sending will do, when it isn't to say something to Claude: run a
    /// shell command, or a slash command, which says what it does once the
    /// list of them has gone; and, while a turn runs, that a message waits
    /// for it, and how to say it now instead.
    @ViewBuilder
    private var modeNote: some View {
        switch input {
        case .shell:
            note(Label("Runs in the working copy. Claude sees what it prints with your next message.", glyph: .agentTerminal))
        case let .command(_, known?, _) where suggestions.isEmpty:
            HStack(spacing: 6) {
                Text("/\(known.name)")
                    .font(.caption.monospaced().weight(.semibold))
                    .foregroundStyle(Color.accentColor)
                if !known.argumentHint.isEmpty {
                    Text(known.argumentHint).font(.caption.monospaced()).foregroundStyle(Color.dimmer)
                }
                Text(known.description).font(.caption).foregroundStyle(Color.dim)
            }
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
        default:
            if steers && canSend {
                note(Label("Sends when this turn ends. Hold send to stop it and send now.", systemImage: "text.append"))
            }
        }
    }

    private func note(_ label: some View) -> some View {
        label
            .font(.caption)
            .foregroundStyle(Color.dim)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
    }

    private var canSend: Bool {
        if case let .shell(line) = input { return !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if let sideQuestion { return !sideQuestion.isEmpty }
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

    /// Attach, and put what is written aside: T3 Code's stash is a key on the
    /// desktop, and here one more thing the `+` holds.
    private var attachMenu: some View {
        Menu {
            // Only for a host that keeps them: an older one drops them.
            if chat.takesFiles {
                Button("Photo Library", systemImage: "photo.on.rectangle") { choosingPhotos = true }
                Button("Files", glyph: .folder) { choosingFiles = true }
                Divider()
            }
            Button("Put Aside", systemImage: "tray.and.arrow.down") { putAside() }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } label: {
            Image(systemName: "plus")
                .font(.body.weight(.medium))
                .foregroundStyle(Color.ink)
                .frame(width: 32, height: 32)
                .background(Color.surface2, in: Circle())
                .contentShape(Circle())
        }
        .accessibilityLabel("Attach or put aside")
    }

    /// Stop while Claude works and nothing is typed; send otherwise. While a
    /// turn runs, send says it will wait, and a long press offers to stop the
    /// turn and send now.
    @ViewBuilder
    private var sendButton: some View {
        if later != nil {
            Button { send() } label: {
                Round(symbol: "calendar.badge.clock", fill: .accentColor, dim: !canSend || sending)
            }
            .disabled(!canSend || sending)
            .accessibilityLabel("Schedule")
        } else if working && draft.isEmpty && attached.isEmpty {
            Button {
                Task { try? await chat.interrupt() }
            } label: {
                Image(systemName: "stop.fill")
                    .font(.caption)
                    .foregroundStyle(.white)
                    .frame(width: 32, height: 32)
                    .background(Color.stop, in: Circle())
            }
            .accessibilityLabel("Stop Claude")
        } else if steers {
            Button { send() } label: {
                Round(symbol: "text.append", fill: .accentColor, dim: !canSend || sending)
            }
            .disabled(!canSend || sending)
            .contextMenu {
                Button("Send Now", systemImage: "arrow.turn.up.right") { send(steer: true) }
                Button("Send After This Turn", systemImage: "text.append") { send() }
            }
            .accessibilityLabel("Send after this turn")
            .accessibilityAction(named: "Stop the turn and send now") { send(steer: true) }
        } else {
            Button { send() } label: {
                Round(symbol: branchFrom != nil ? "arrow.triangle.branch" : "arrow.up", fill: .accentColor,
                      dim: !canSend || sending)
            }
            .disabled(!canSend || sending)
            .accessibilityLabel(isShell ? "Run" : branchFrom != nil ? "Send in a new conversation" : "Send")
        }
    }

    /// The drawer on the card's top edge: the commands or files that fit what
    /// is typed, or the stash.
    @ViewBuilder
    private var drawer: some View {
        if !suggestions.isEmpty {
            ComposerDrawer {
                CommandSuggestions(commands: suggestions) { draft = "/\($0.name) " }
            }
        } else if let mention {
            ComposerDrawer {
                MentionSuggestions(paths: mentioned?.paths ?? [], error: mentioned?.error,
                                   searching: mentioned?.query != mention.query) { path in
                    draft = ComposerInput.mentioning(path, in: draft)
                }
            }
        } else if stashOpen, !stash.all.isEmpty {
            ComposerDrawer {
                StashList(stashed: stash.all) { s in
                    takeOut(s)
                } delete: { s in
                    stash.take(s.id)
                }
            }
        }
    }

    /// T3 Code's composer: one card a step up from the canvas, with the
    /// drawer on its top edge, what is attached, the message, and under it a
    /// line of what it goes with -- attach on the left; the model and its
    /// effort, the permission mode, how full the context is, and the round
    /// send or stop on the right.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 0) {
            drawer
            if branchFrom != nil {
                ComposerBar(glyph: .edit, text: "Editing an earlier message: sending starts a new conversation from there.") {
                    branchFrom = nil
                    draft = ""
                }
            }
            if !attached.isEmpty {
                AttachedStrip(files: attached) { f in attached.removeAll { $0.id == f.id } }
            }
            modeNote
            if let at = later { scheduleLine(at) }
            TextField(later == nil ? "Message Claude" : "Message for later", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                // A `!` command is code, and set as code.
                .font(isShell ? Font.body.monospaced() : Font.body)
                .foregroundStyle(Color.ink)
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
            HStack(spacing: 4) {
                attachMenu
                ModeMenu(chat: chat)
                ModelMenu(chat: chat)
                Spacer(minLength: 0)
                // Text only: a file isn't something the server keeps for later.
                if onSchedule != nil, later == nil, attached.isEmpty, !isShell, branchFrom == nil {
                    Button { later = .now.addingTimeInterval(60 * 60) } label: {
                        Image(systemName: "calendar.badge.clock")
                            .font(.body)
                            .foregroundStyle(Color.dim)
                            .frame(width: 30, height: 32)
                    }
                    .accessibilityLabel("Send later")
                }
                if chat.status == "compacting" {
                    Text("compacting").font(.caption).foregroundStyle(Color.dim).lineLimit(1)
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
        .background(Color.surface, in: RoundedRectangle(cornerRadius: Radius.composer, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: Radius.composer, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Radius.composer, style: .continuous)
            .strokeBorder(isShell ? Color.accentColor : Color.line, lineWidth: 1))
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
                chat.error = error.userMessage
            }
        }
    }

    /// When a message for later is to be said, and whether it comes round
    /// again: once, or by one of the queue's rules, where the server can.
    private func scheduleLine(_ at: Date) -> some View {
        HStack(spacing: 6) {
            DatePicker(cadence == .hourly ? "From" : "Say it at", selection: Binding(get: { at }, set: { later = $0 }),
                       in: Date.now..., displayedComponents: cadence == nil ? [.date, .hourAndMinute] : [.hourAndMinute])
                .font(.footnote)
                .foregroundStyle(Color.dim)
            if utcOffset != nil {
                Menu {
                    Picker("Repeat", selection: $cadence) {
                        Text("Once").tag(Cadence?.none)
                        ForEach(Cadence.all) { c in Text(c.label).tag(Cadence?.some(c)) }
                    }
                } label: {
                    HStack(spacing: 3) {
                        if cadence != nil { Image(systemName: "repeat") }
                        Text(cadence?.label ?? "Once")
                    }
                    .font(.footnote)
                    .foregroundStyle(cadence == nil ? Color.dim : Color.ink)
                    .padding(.horizontal, 8)
                    .frame(height: 30)
                    .background(Color.surface2, in: Capsule())
                }
                .menuOrder(.fixed)
                .accessibilityLabel("Repeat, \(cadence?.label ?? "Once")")
            }
            Button {
                later = nil
                cadence = nil
            } label: { Image(systemName: "xmark.circle.fill") }
                .foregroundStyle(Color.dimmer)
                .accessibilityLabel("Send now instead")
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
    }

    // MARK: Sending

    private func send(steer: Bool = false) {
        if let at = later, let onSchedule {
            let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return }
            let parts = Calendar.current.dateComponents([.hour, .minute], from: at)
            let rule = cadence?.rule(hour: parts.hour ?? 0, minute: parts.minute ?? 0,
                                     serverOffset: utcOffset ?? TimeZone.current.secondsFromGMT())
            sending = true
            Task {
                defer { sending = false }
                do {
                    try await onSchedule(text, at, rule)
                    draft = ""
                    later = nil
                    cadence = nil
                } catch {
                    chat.error = error.userMessage
                }
            }
            return
        }
        if case let .shell(line) = input {
            run(line)
            return
        }
        if let question = sideQuestion, let onSideChat {
            guard !question.isEmpty else { return }
            onSideChat(question)
            draft = ""
            return
        }
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if let from = branchFrom, let onBranch {
            guard !text.isEmpty else { return }
            // Forked from what the agent said last before the message; the
            // conversation edited stays as it was.
            onBranch(ChatTranscript.assistantBefore(chat.entries, seq: from), text)
            draft = ""
            branchFrom = nil
            return
        }
        let files = attached
        guard !text.isEmpty || !files.isEmpty else { return }
        // Emptied now rather than once the server has it, so nothing typed
        // during the round trip is wiped with it; put back if it fails and
        // nothing new has been typed.
        draft = ""
        attached = []
        justSent = (text, .now)
        sending = true
        let steering = steer && working
        Task {
            defer { sending = false }
            do {
                // Steering is the turn stopped and this said in its place:
                // the host takes the message as the next turn once the
                // interrupt has ended this one.
                if steering { try await chat.interrupt() }
                try await chat.send(text, files: files)
            } catch {
                justSent = nil
                if draft.isEmpty { draft = text }
                if attached.isEmpty { attached = files }
                chat.error = error.userMessage
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
                chat.error = error.userMessage
            }
        }
    }

    /// "Edit from Here": the message's words back in the box, and sending
    /// them forks the conversation from just before it.
    private func edit(from seq: UInt64, _ text: String) {
        branchFrom = seq
        later = nil
        draft = text
        composing = true
    }

    /// A queued message taken back off the queue to be changed: its words in
    /// the box, after anything already there.
    private func takeBack(_ m: QueuedMessage) {
        draft = draft.isEmpty ? m.text : draft + "\n\n" + m.text
        composing = true
    }

    /// What is written, put aside in the stash, and the box emptied for the
    /// next thing.
    private func putAside() {
        stash.put(draft)
        draft = ""
    }

    /// One taken out of the stash: back in the box, and out of the stash. What
    /// was in the box goes into the stash in its place, so nothing is lost.
    private func takeOut(_ s: Stashed) {
        let had = draft
        stash.take(s.id)
        stash.put(had)
        draft = s.text
        stashOpen = false
        composing = true
    }

    private func answer(_ ask: Ask, _ decision: ChatDecision) {
        answering = ask.id
        Task {
            defer { answering = nil }
            do { try await chat.answer(ask.id, decision: decision) } catch { chat.error = error.userMessage }
        }
    }
}

/// The composer's round button, T3 Code's: a glyph on a filled circle.
private struct Round: View {
    let symbol: String
    let fill: Color
    let dim: Bool

    var body: some View {
        Image(systemName: symbol)
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.white)
            .frame(width: 32, height: 32)
            .background(fill, in: Circle())
            .opacity(dim ? 0.4 : 1)
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


/// The reply as Claude writes it, and that it is still working. The only
/// rows that read the reply: when the transcript's own view did, every word
/// drew the whole lazy stack again, and rows scrolled to while it was busy
/// came up blank until it caught up.
private struct Writing: View {
    let chat: Chat
    let showsWorking: Bool
    /// When the turn in progress started, while there is one to time.
    let started: Date?

    var body: some View {
        if !chat.streaming.isEmpty {
            MarkdownText(text: chat.streaming).opacity(0.8)
        }
        if showsWorking || started != nil {
            HStack(spacing: 6) {
                if showsWorking { WorkingDots(size: 22) }
                // The word only while there is nothing else to look at.
                if chat.streaming.isEmpty { Text("Working").font(.caption) }
                if let started { Elapsed(since: started) }
            }
            .foregroundStyle(Color.working)
        }
    }
}

/// Keeps the transcript at its bottom as the reply grows, from outside it,
/// for the same reason.
private struct FollowsReply: View {
    let chat: Chat
    let follow: () -> Void

    var body: some View {
        Color.clear.onChange(of: chat.streaming) { follow() }
    }
}

/// The model and its effort, in one menu, as Claude Code's app has them:
/// "Opus 5.5 Medium". A view of its own, so that only the model's state draws
/// it again: a menu drawn again while it is open goes back to its top, and the
/// chat around it is drawn again with every word Claude writes.
private struct ModelMenu: View {
    let chat: Chat

    @ViewBuilder
    var body: some View {
        if !chat.models.isEmpty || !chat.efforts.isEmpty {
            Menu {
                if !chat.models.isEmpty {
                    Section("Model") {
                        Picker("Model", selection: Binding(get: { chat.model ?? chat.models[0].value }, set: { m in
                            Task { do { try await chat.setModel(m) } catch { chat.error = error.userMessage } }
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
                            Task { do { try await chat.setEffort(e.isEmpty ? nil : e) } catch { chat.error = error.userMessage } }
                        })) {
                            Text("Model's Default").tag("")
                            ForEach(chat.efforts, id: \.self) { e in
                                Text(ChatView.effortName(e)).tag(e)
                            }
                        }
                        .pickerStyle(.inline)
                    }
                }
            } label: {
                HStack(spacing: 4) {
                    if let model = modelLabel {
                        Text(model).foregroundStyle(Color.ink)
                    }
                    if let effort = chat.effort {
                        Text(ChatView.effortName(effort)).foregroundStyle(Color.dim)
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
}

/// The permission mode, a view of its own for the same reason as the model's.
private struct ModeMenu: View {
    let chat: Chat

    var body: some View {
        let mode = ChatView.modes.first { $0.value == (chat.mode ?? "auto") }
        return Menu {
            Picker("Permission mode", selection: Binding(get: { chat.mode ?? "auto" }, set: { m in
                Task { do { try await chat.setMode(m) } catch { chat.error = error.userMessage } }
            })) {
                ForEach(ChatView.modes, id: \.value) { m in
                    Text("\(m.label) — \(m.hint)").tag(m.value)
                }
            }
        } label: {
            Label(mode?.short ?? (chat.mode ?? "Auto"), glyph: .policy)
                .labelStyle(TightLabel())
                .foregroundStyle(chat.mode == "plan" ? Color.accentColor : Color.dim)
                .modifier(Chip())
                .fixedSize()
        }
        .accessibilityLabel("Permission mode, \(mode?.label ?? chat.mode ?? "Auto")")
    }
}

/// A control in the composer's line, as T3 Code's ghost button: no fill of
/// its own on the card.
private struct Chip: ViewModifier {
    func body(content: Content) -> some View {
        content
            .font(.subheadline)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .frame(height: 32)
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

/// Scrolls the transcript's list to its end by its offset, not to a row. A
/// list scrolled to a row takes the row's index when asked, and a row going
/// in the same update -- the line saying Claude is working, as a turn ends
/// -- left it scrolling past its last row, which it throws for.
@MainActor
final class ListEnd {
    fileprivate weak var view: UIScrollView?
    private var pending = false

    /// After this update, once the list has its rows. Rows it has not drawn
    /// are measured as it reaches them, which moves the end, so it goes
    /// again until the end stays put.
    func scroll(animated: Bool = false) {
        guard !pending else { return }
        pending = true
        DispatchQueue.main.async { [self] in
            pending = false
            guard let view else { return }
            for _ in 0..<4 {
                view.layoutIfNeeded()
                let y = max(-view.adjustedContentInset.top,
                            view.contentSize.height + view.adjustedContentInset.bottom - view.bounds.height)
                if abs(view.contentOffset.y - y) < 1 { break }
                view.setContentOffset(CGPoint(x: view.contentOffset.x, y: y), animated: animated)
                if animated { break }
            }
        }
    }

    /// Beside the list, to find the scroll view under it.
    struct Finder: UIViewRepresentable {
        let end: ListEnd

        func makeUIView(context: Context) -> UIView { Probe(end: end) }
        func updateUIView(_ view: UIView, context: Context) {}

        private final class Probe: UIView {
            let end: ListEnd
            init(end: ListEnd) {
                self.end = end
                super.init(frame: .zero)
                isUserInteractionEnabled = false
            }
            required init?(coder: NSCoder) { nil }

            override func didMoveToWindow() {
                super.didMoveToWindow()
                guard window != nil else { return }
                DispatchQueue.main.async { [self] in
                    var at = superview
                    while let ancestor = at, end.view == nil {
                        end.view = Self.list(in: ancestor)
                        at = ancestor.superview
                    }
                }
            }

            private static func list(in view: UIView) -> UICollectionView? {
                if let list = view as? UICollectionView { return list }
                for sub in view.subviews { if let list = list(in: sub) { return list } }
                return nil
            }
        }
    }
}

