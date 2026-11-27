import HuraKit
import SwiftUI

/// Starting a worktree, as T3 Code starts a thread and the desktop's draft
/// does (`apps/desktop/src/NewWorktree.tsx`): no form, but what to build
/// asked over a composer, and sending it the start. The agent and when it
/// starts are the composer's controls; where it runs, the base branch, the
/// policy and the name are the strip under it; toolchains, credentials,
/// skills and MCP servers are under More. It opens with the project's
/// defaults, each the server's where the project sets none.
///
/// Nothing here decides anything `hura new` decides differently: the name is
/// made by the server when this leaves it blank, and the choices and their
/// defaults arrive from the server.

/// Where a worktree's work comes from, when it isn't typed.
enum WorkOrigin: Hashable {
    case ticket(TrackerTask)
    /// A pull request, to work on (its pushes update it) or to review.
    case pull(PullRequestSummary, review: Bool)
}

struct NewWorktreeView: View {
    let server: Server
    let project: Project
    var origin: WorkOrigin?
    /// Opens the new worktree once it exists.
    var onCreated: (String) -> Void = { _ in }
    @Environment(\.dismiss) private var dismiss

    /// Another of the server's projects, picked in the headline.
    @State private var picked: Project?
    @State private var options: NewOptions?
    @State private var facts: Facts?
    @State private var task = ""
    @State private var name = ""
    @State private var base = ""
    @State private var policy = ""
    @State private var agentInterface = "chat"
    /// A worktree on the server instead of a sandbox; only offered where the
    /// server's config allows it.
    @State private var worktree = false
    @State private var toolchains: Set<String> = []
    @State private var providers: Set<String> = []
    @State private var busy = false
    @State private var error: String?
    /// The work branch a ticket or a pull request names; empty otherwise.
    @State private var branch = ""
    @State private var when: When = .now
    @State private var at = Date.now.addingTimeInterval(60 * 60)
    /// Or again and again: how often, and at what time of day on the phone's
    /// clock. Nine on a weekday, as the desktop starts it.
    @State private var cadence: Cadence = .weekdays
    @State private var time = Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: .now) ?? .now
    @State private var more = false
    @FocusState private var typing: Bool

    /// Now, once the server's queue has room, at a time, or on a schedule.
    enum When: Hashable { case now, queue, at, `repeat` }

    private var current: Project { picked ?? project }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    headline
                    originLine
                    VStack(alignment: .leading, spacing: 8) {
                        composer
                        strip
                        status
                    }
                    if options != nil, worktree { worktreeNote }
                    if more { moreRows }
                }
                .padding(.horizontal, 16)
                .padding(.top, 28)
                .padding(.bottom, 40)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Color.canvas.ignoresSafeArea())
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(busy)
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled(busy || !task.isEmpty)
            // Again for each project picked: the checkout says the base and
            // the toolchains, and the project its own defaults.
            .task(id: current.name) { await load() }
        }
    }

    // MARK: The headline

    /// "What should we build in ‹project›?", the project a menu of the
    /// server's others, as T3 Code's is. A pull request's draft is its
    /// repository's, so it stays put.
    private var headline: some View {
        let choosable = !isPull && server.projects.count > 1
        // The chevron and the mark held to the name by a space that does not
        // break, so a long name never leaves them on a line of their own.
        let words = Text("What should we build in ").foregroundStyle(.dim)
            + Text(current.name).foregroundStyle(.ink)
            + Text(choosable ? "\u{00A0}\(Image(systemName: "chevron.down"))" : "").font(.title3.weight(.semibold))
                .foregroundStyle(.dimmer)
            + Text("?").foregroundStyle(.dim)
        return Menu {
            ForEach(server.projects) { p in
                Button {
                    picked = p
                } label: {
                    if p.name == current.name { Label(p.name, systemImage: "checkmark") } else { Text(p.name) }
                }
            }
        } label: {
            words
                .font(.title.weight(.semibold))
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .disabled(!choosable || busy)
        .accessibilityLabel("What should we build in \(current.name)?")
        .accessibilityHint(choosable ? "Picks another project" : "")
    }

    /// Where the work came from, when it was not typed.
    @ViewBuilder
    private var originLine: some View {
        switch origin {
        case let .ticket(t)?:
            OriginLine(glyph: .tracker, key: t.key, branch: branch, note: nil)
        case let .pull(p, review)?:
            OriginLine(glyph: .pullRequest, key: pullName(p), branch: p.branch,
                       note: review ? "To review it: the agent is asked not to push."
                                    : "To work on it: its pushes update the pull request.")
        case nil:
            EmptyView()
        }
    }

    // MARK: The composer

    private var composer: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Said, because nothing else here does: this is the agent's first
            // prompt, sent the moment the clone finishes.
            TextField("Describe the task. It starts on it once the \(worktree ? "worktree" : "sandbox") is ready; sent empty, the agent waits.",
                      text: $task, axis: .vertical)
                .lineLimit(3...12)
                .focused($typing)
                .foregroundStyle(.ink)
                .padding(.horizontal, 14)
                .padding(.top, 14)
                .padding(.bottom, 10)
            HStack(spacing: 2) {
                ControlMenu(label: agentInterface == "terminal" ? "Terminal" : "Chat",
                            glyph: agentInterface == "terminal" ? .agentTerminal : .chat) {
                    Picker("Agent", selection: $agentInterface) {
                        Label("Chat", glyph: .chat).tag("chat")
                        Label("Terminal", glyph: .agentTerminal).tag("terminal")
                    }
                }
                .disabled(options == nil)
                .accessibilityLabel("Agent: \(agentInterface == "terminal" ? "Terminal" : "Chat")")
                ControlMenu(label: whenLabel, systemImage: whenSymbol) {
                    Picker("Starts", selection: $when) {
                        Label("Now", systemImage: "play").tag(When.now)
                        Label("When one is free", systemImage: "hourglass").tag(When.queue)
                        Label("At a time", systemImage: "calendar.badge.clock").tag(When.at)
                        if server.queue.canRepeat {
                            Label("On a schedule", systemImage: "repeat").tag(When.repeat)
                        }
                    }
                }
                .accessibilityLabel("Starts: \(whenLabel)")
                Spacer(minLength: 8)
                sendButton
            }
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
            if when == .at || when == .repeat {
                Divider().overlay(Color.line)
                schedule
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
            }
        }
        .card(radius: Radius.composer)
        .contentShape(RoundedRectangle(cornerRadius: Radius.composer))
        .onTapGesture { typing = true }
        .onAppear { if origin == nil { typing = true } }
    }

    private var sendButton: some View {
        Button(action: submit) {
            ZStack {
                Circle().fill(ready ? Color.accentColor : Color.surface2)
                if busy {
                    Spinner().foregroundStyle(.white)
                } else {
                    Image(systemName: when == .now ? "arrow.up" : whenSymbol)
                        .font(.system(size: 14, weight: .bold))
                        .foregroundStyle(ready ? Color.white : Color.dimmer)
                }
            }
            .frame(width: 32, height: 32)
        }
        .buttonStyle(.plain)
        .disabled(!ready)
        .accessibilityLabel(sendTitle)
    }

    /// The time it starts at, or the schedule it comes round on, under the
    /// controls that chose them.
    @ViewBuilder
    private var schedule: some View {
        if when == .at {
            DatePicker("Starts at", selection: $at, in: Date.now..., displayedComponents: [.date, .hourAndMinute])
                .font(.subheadline)
                .foregroundStyle(.dim)
        } else {
            HStack {
                Menu {
                    Picker("How often", selection: $cadence) {
                        ForEach(Cadence.all) { Text($0.label).tag($0) }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "repeat")
                        Text(cadence.label)
                        Image(systemName: "chevron.up.chevron.down").font(.caption2)
                    }
                    .font(.subheadline)
                    .foregroundStyle(.ink)
                }
                Spacer()
                if cadence.takesTime {
                    DatePicker("At", selection: $time, displayedComponents: .hourAndMinute)
                        .labelsHidden()
                }
            }
        }
    }

    // MARK: The strip

    /// T3 Code's strip at the composer's foot: where the work happens.
    private var strip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                Menu {
                    Choice(title: "Sandbox", note: "Its own VM, under a policy", on: !worktree) { worktree = false }
                    Choice(title: "Worktree", note: "On the server, as its user", on: worktree) { worktree = true }
                } label: {
                    Chip(glyph: worktree ? .unsandboxed : nil, systemImage: "shippingbox",
                         text: worktree ? "Worktree" : "Sandbox", tint: worktree ? .attention : nil)
                }
                // This server's config allows only sandboxes.
                .disabled(options?.worktrees != true)
                .accessibilityLabel("Runs in: \(worktree ? "Worktree" : "Sandbox")")

                FieldChip(glyph: .branch, placeholder: "Default branch", text: $base, mono: true)
                    .disabled(isPull || options == nil)
                    .accessibilityLabel(isPull ? "Merges into" : "Base branch")

                Menu {
                    ForEach(options?.policies ?? [], id: \.spec) { p in
                        Choice(title: p.spec, note: p.summary, on: p.spec == policy) { policy = p.spec }
                    }
                } label: {
                    Chip(glyph: .policy, text: worktree ? "No policy" : policy.isEmpty ? "Policy" : policy)
                }
                .disabled(options == nil || worktree)
                .accessibilityLabel("Network policy: \(worktree ? "none" : policy)")

                FieldChip(glyph: .edit, placeholder: "Name from the task", text: $name, mono: false)
                    .accessibilityLabel("Name")

                Button {
                    withAnimation(.snappy) { more.toggle() }
                } label: {
                    HStack(spacing: 4) {
                        Text("More")
                        Image(systemName: "chevron.down")
                            .font(.caption2.weight(.semibold))
                            .rotationEffect(.degrees(more ? 180 : 0))
                    }
                    .chip()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(more ? "Less" : "More: toolchains, credentials, skills and MCP servers")
            }
            .padding(.horizontal, 2)
        }
        .scrollClipDisabled()
    }

    /// The error, or what stays behind on the server's checkout, or what a
    /// later start means.
    @ViewBuilder
    private var status: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error {
                Text(error).foregroundStyle(.bad)
            }
            if let facts, let drift = driftWords(facts) {
                Label(drift, glyph: .warning).foregroundStyle(.dim)
            }
            switch when {
            case .now: EmptyView()
            case .queue:
                Text("Starts once fewer worktrees are running than the server allows: \(server.queue.limit) at once.")
            case .at:
                Text("The server keeps it and starts it then, whether or not this phone is around.")
            case .repeat:
                if let rule = scheduleRule {
                    let offset = server.queue.utcOffset ?? TimeZone.current.secondsFromGMT()
                    // The first time is the server's to work out, as the
                    // desktop leaves it; this is that same sum, said ahead.
                    Text("A new worktree each time it comes round."
                        + (rule.next(after: .now, serverOffset: offset).map { " The first starts \(firstWords($0))." } ?? ""))
                    if let other = clockDifference(serverOffset: offset) {
                        Text("Times are this phone's. The server's clock is \(other), and it is told the same moment on its own.")
                    }
                }
            }
        }
        .font(.footnote)
        .foregroundStyle(.dimmer)
        .padding(.horizontal, 4)
    }

    /// Said where the choice is made, because nothing afterwards stops it: the
    /// agent is a process of the server user's, with their files, their
    /// credentials and the whole network.
    private var worktreeNote: some View {
        Label("A git worktree of \(current.path) on the server. The agent runs there as the server's user, "
              + "with its files, credentials and network, and no policy. Its skills and MCP servers are that user's own.",
              glyph: .unsandboxed)
            .font(.footnote)
            .foregroundStyle(.attention)
            .padding(12)
            .card(fill: .surface)
    }

    // MARK: More

    @ViewBuilder
    private var moreRows: some View {
        if let options {
            if worktree {
                // A worktree is given none of these: no image, no proxy to
                // swap a credential in, and the server user's own skills and
                // MCP servers.
                Text("A worktree has no toolchains or credentials of its own: it has the server user's.")
                    .font(.footnote)
                    .foregroundStyle(.dimmer)
            } else {
                VStack(alignment: .leading, spacing: 16) {
                    PickRow(title: "Toolchains", glyph: .tool) {
                        ForEach(options.toolchains, id: \.name) { t in
                            Pill(text: t.name, on: toolchains.contains(t.name)) { toggle(t.name, in: &toolchains) }
                                .accessibilityHint(t.summary)
                        }
                        if options.toolchains.isEmpty { Hint("none") }
                    }
                    PickRow(title: "Credentials", systemImage: "key") {
                        ForEach(options.providers, id: \.name) { p in
                            Pill(text: p.name, on: providers.contains(p.name)) { toggle(p.name, in: &providers) }
                                .accessibilityHint(p.kind)
                        }
                        if let why = options.providersError {
                            Text("Could not list them: \(why)").font(.footnote).foregroundStyle(.bad)
                        } else if options.providers.isEmpty {
                            Hint("None configured")
                        }
                    }
                    // Named, not offered: skills and MCP servers are one
                    // decision about what your agents can reach, made in the
                    // server's config file.
                    PickRow(title: "Skills, set in the server's config", systemImage: "sparkles") {
                        ForEach(options.skills, id: \.self) { Pill(text: $0, on: false, held: true) {} }
                        if options.skills.isEmpty { Hint("none") }
                    }
                    PickRow(title: "MCP servers, which act with the server's credentials", glyph: .integrations) {
                        ForEach(options.mcp, id: \.self) { Pill(text: $0, on: false, held: true) {} }
                        if options.mcp.isEmpty { Hint("none") }
                    }
                }
                // The whole width, as the composer above it is: sized to its
                // pills, it stopped short of the right edge.
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .card()
            }
        } else {
            WaitingRow()
        }
    }

    // MARK: Words

    private var isPull: Bool { if case .pull? = origin { true } else { false } }

    private var ready: Bool { !busy && options != nil }

    private var whenLabel: String {
        switch when {
        case .now: "Now"
        case .queue: "When free"
        case .at: at.formatted(.dateTime.weekday(.abbreviated).hour().minute())
        case .repeat: cadence.label
        }
    }

    private var whenSymbol: String {
        switch when {
        case .now: "play"
        case .queue: "hourglass"
        case .at: "calendar.badge.clock"
        case .repeat: "repeat"
        }
    }

    private var sendTitle: String {
        switch when {
        case .now: "Start It"
        case .queue: "Queue It"
        case .at: "Schedule It"
        case .repeat: "Schedule It to Repeat"
        }
    }

    /// The rule the schedule's choices make, on the server's clock.
    private var scheduleRule: Repeat? {
        guard let offset = server.queue.utcOffset else { return nil }
        let parts = Calendar.current.dateComponents([.hour, .minute], from: time)
        return cadence.rule(hour: parts.hour ?? 9, minute: parts.minute ?? 0, serverOffset: offset)
    }

    private func firstWords(_ date: Date) -> String {
        Calendar.current.isDateInToday(date) ? "today at \(date.formatted(date: .omitted, time: .shortened))"
            : Calendar.current.isDateInTomorrow(date) ? "tomorrow at \(date.formatted(date: .omitted, time: .shortened))"
            : date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated).hour().minute())
    }

    private func toggle(_ value: String, in set: inout Set<String>) {
        if set.contains(value) { set.remove(value) } else { set.insert(value) }
    }

    // MARK: Loading and sending

    private func load() async {
        error = nil
        facts = nil
        do {
            let o: NewOptions
            if let options { o = options } else { o = try await server.api.newOptions() }
            if origin != nil, task.isEmpty { await prefill() }
            // The checkout's own facts: its branch as the base, the
            // toolchains it points at, the providers to tick. A pull
            // request's base is its own.
            var pullBase: String?
            if case let .pull(p, _)? = origin { pullBase = p.base }
            var checkout: Picked?
            do {
                checkout = try await server.api.inspect(path: current.path, branch: pullBase)
            } catch {
                self.error = error.userMessage
            }
            facts = checkout?.facts
            let chosen = DraftDefaults(project: current.defaults, options: o, checkout: checkout, pullBase: pullBase)
            agentInterface = chosen.interface
            worktree = chosen.worktree
            policy = chosen.policy
            base = chosen.base
            toolchains = Set(chosen.toolchains)
            providers = Set(chosen.providers)
            // Last, so nothing above can be changed before the defaults land.
            options = o
        } catch {
            self.error = error.userMessage
        }
    }

    /// The task, name and branch a ticket or a pull request suggests, as the
    /// desktop's draft fills them in.
    private func prefill() async {
        switch origin {
        case let .ticket(t)?:
            task = "\(t.key): \(t.title)\n\n\(t.url)"
            // A name as a person writes one; the server makes the slug.
            name = "\(t.key) \(t.title)"
            // The branch under this server's prefix, which is what a session
            // here will be given.
            let prefix = (try? await server.api.settings()).map { $0.settings.branchPrefix ?? $0.defaultBranchPrefix } ?? "hura"
            branch = ticketBranch(prefix: prefix, key: t.key, title: t.title)
        case let .pull(p, review)?:
            task = pullTask(p, review: review)
            branch = p.branch
        case nil:
            break
        }
    }

    private func submit() {
        guard let options else { return }
        if when == .at, at <= .now {
            error = "Pick a time after now to start it at."
            return
        }
        let rule = scheduleRule
        if when == .repeat, rule == nil {
            error = "This server can't keep a schedule."
            return
        }
        busy = true
        error = nil
        let trimmed = { (s: String) -> String? in
            let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
            return t.isEmpty ? nil : t
        }
        let session = NewSession(project: current.name, name: trimmed(name), repo: current.repo, task: task,
                                 base: trimmed(base), branch: trimmed(branch), policy: policy,
                                 providers: options.providers.map(\.name).filter(providers.contains),
                                 toolchains: options.toolchains.map(\.name).filter(toolchains.contains),
                                 interface: agentInterface, ticket: ticketOrigin, pull: pullOrigin,
                                 worktree: worktree)
        Task {
            defer { busy = false }
            do {
                switch when {
                case .now:
                    let created = try await server.create(session)
                    dismiss()
                    onCreated(created)
                case .queue:
                    try await server.enqueue(session, at: nil)
                    dismiss()
                case .at:
                    try await server.enqueue(session, at: at)
                    dismiss()
                case .repeat:
                    // No first time: the server's is the rule's next, on its
                    // own clock, as the desktop sends it.
                    try await server.enqueue(session, at: nil, repeat: rule)
                    dismiss()
                }
            } catch {
                self.error = error.userMessage
            }
        }
    }

    private var ticketOrigin: TicketOrigin? {
        guard case let .ticket(t)? = origin else { return nil }
        return TicketOrigin(tracker: t.tracker, kind: t.kind, id: t.id, key: t.key, url: t.url, repo: t.repo)
    }

    private var pullOrigin: PullOrigin? {
        guard case let .pull(p, _)? = origin else { return nil }
        return PullOrigin(tracker: p.tracker, kind: p.kind, id: p.id, key: p.key, url: p.url, repo: p.repo)
    }
}

// MARK: The pieces

/// What the ticket or pull request is, over the composer.
private struct OriginLine: View {
    let glyph: Glyph
    let key: String
    let branch: String
    let note: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Image(glyph: glyph)
                Text(key).foregroundStyle(.ink)
                if !branch.isEmpty {
                    Text("on").foregroundStyle(.dimmer)
                    Text(branch).font(.footnote.monospaced()).lineLimit(1).truncationMode(.middle)
                }
            }
            if let note { Text(note).foregroundStyle(.dimmer) }
        }
        .font(.footnote)
        .foregroundStyle(.dim)
    }
}

/// A menu's item with a line under it saying what it means, ticked when it
/// is the one chosen.
private struct Choice: View {
    let title: String
    let note: String?
    let on: Bool
    let action: () -> Void

    var body: some View {
        // A toggle, since that is what a menu ticks.
        Toggle(isOn: Binding(get: { on }, set: { _ in action() })) {
            Text(title)
            if let note { Text(note) }
        }
    }
}

/// One of the composer's controls: an icon and what is chosen, opening a menu
/// of the rest. Quiet until pressed, as T3 Code's are.
private struct ControlMenu<Content: View>: View {
    let label: String
    var glyph: Glyph?
    var systemImage: String?
    @ViewBuilder let content: () -> Content

    var body: some View {
        Menu(content: content) {
            HStack(spacing: 5) {
                if let glyph { Image(glyph: glyph) } else if let systemImage { Image(systemName: systemImage) }
                Text(label).lineLimit(1)
            }
            .font(.subheadline.weight(.medium))
            .foregroundStyle(.dim)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .contentShape(Rectangle())
        }
    }
}

/// A choice in the strip: a chip with its icon, as T3 Code's branch and
/// environment pickers are drawn.
private struct Chip: View {
    var glyph: Glyph?
    var systemImage: String?
    let text: String
    var tint: Color?

    var body: some View {
        HStack(spacing: 5) {
            if let glyph { Image(glyph: glyph) } else if let systemImage { Image(systemName: systemImage) }
            Text(text).lineLimit(1)
        }
        .foregroundStyle(tint ?? .dim)
        .chip()
    }
}

/// A chip that is a field: the base branch and the name, as wide as what
/// they say.
private struct FieldChip: View {
    let glyph: Glyph
    let placeholder: String
    @Binding var text: String
    let mono: Bool
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        HStack(spacing: 5) {
            Image(glyph: glyph).foregroundStyle(.dim)
            // As wide as what it says, or its placeholder when blank, as T3
            // Code's branch picker is: the words, hidden, set the width.
            Text(text.isEmpty ? placeholder : text)
                .font(mono ? .footnote.monospaced() : .footnote)
                .lineLimit(1)
                .hidden()
                .overlay {
                    TextField(placeholder, text: $text)
                        .font(mono ? .footnote.monospaced() : .footnote)
                        .foregroundStyle(.ink)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                .padding(.trailing, 2)
        }
        .chip()
        .opacity(enabled ? 1 : 0.6)
    }
}

extension View {
    /// A chip's shape: a hairline round its words, on the canvas.
    fileprivate func chip() -> some View {
        font(.footnote.weight(.medium))
            .foregroundStyle(.dim)
            .padding(.horizontal, 10)
            .frame(height: 30)
            .background(Color.surface, in: RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: Radius.control, style: .continuous).strokeBorder(Color.line))
    }
}

/// One line of choices under More, its kind said above them.
private struct PickRow<Content: View>: View {
    let title: String
    var glyph: Glyph?
    var systemImage: String?
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                if let glyph { Image(glyph: glyph) } else if let systemImage { Image(systemName: systemImage) }
                Text(title)
            }
            .font(.caption.weight(.medium))
            .foregroundStyle(.dimmer)
            WrapRows(spacing: 6) { content() }
        }
    }
}

/// A choice that is on or off. A held one is named, not offered.
private struct Pill: View {
    let text: String
    let on: Bool
    var held = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if on { Image(systemName: "checkmark").font(.caption2.weight(.bold)) }
                Text(text)
            }
            .font(.footnote.weight(.medium))
            .foregroundStyle(on ? Color.accentColor : held ? Color.dim : Color.dim)
            .padding(.horizontal, 10)
            .frame(height: 28)
            .background(on ? Color.accentColor.opacity(0.14) : held ? Color.surface2 : Color.clear, in: Capsule())
            .overlay(Capsule().strokeBorder(on ? Color.accentColor.opacity(0.4) : Color.line))
        }
        .buttonStyle(.plain)
        .disabled(held)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

private struct Hint: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View { Text(text).font(.footnote).foregroundStyle(.dimmer).frame(height: 28) }
}

/// Its children in rows, as many to a row as fit: the pills under More.
private struct WrapRows: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        return CGSize(width: rows.map(\.width).max() ?? 0,
                      height: rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y + (row.height - size.height) / 2), proposal: .unspecified)
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            if !row.items.isEmpty, row.width + spacing + size.width > width {
                rows.append(row)
                row = Row()
            }
            row.width += (row.items.isEmpty ? 0 : spacing) + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}

// MARK: Defaults, schedules and words

/// What a new worktree in a project opens with: the project's own defaults
/// over what the checkout says, over the server's config, as the desktop's
/// draft settles them.
struct DraftDefaults: Equatable {
    var interface: String
    var worktree: Bool
    var policy: String
    var base: String
    var toolchains: [String]
    var providers: [String]

    /// `checkout` is nil until the server has looked at it, or when it could
    /// not. A pull request's base is the branch it merges into, which is not
    /// the checkout's or the project's to decide.
    init(project own: ProjectDefaults, options: NewOptions, checkout: Picked?, pullBase: String? = nil) {
        interface = own.interface ?? options.defaultInterface
        // Only where the server allows one; a project's wish for a worktree
        // on a server that has since stopped allowing them is a sandbox.
        worktree = options.worktrees == true && own.worktree == true
        policy = own.policy ?? options.defaultPolicy
        // The checkout's branch beats the config's default, which is only for
        // a detached HEAD; a branch never pushed is blanked, since a clone of
        // it would fail.
        if let pullBase {
            base = pullBase
        } else if let b = own.base {
            base = b
        } else if let checkout, !checkout.facts.baseOnRemote {
            base = ""
        } else if let b = checkout?.branch, !b.isEmpty {
            base = b
        } else {
            base = options.defaultBase ?? ""
        }
        toolchains = own.toolchains ?? checkout?.facts.toolchains ?? []
        // The checkout's credentials replace the config's when it names any.
        if let p = own.providers {
            providers = p
        } else if let p = checkout?.providers, !p.isEmpty {
            providers = p
        } else {
            providers = options.defaultProviders
        }
    }
}

extension Repeat {
    /// The first time after `after` it comes round, for a server whose
    /// clock is `serverOffset` seconds from UTC: the server's own sum
    /// (`Repeat::next_after` in `hura_core::queue`), which it does when it is
    /// sent no first time, said here so the draft can say when that is.
    func next(after: Date, serverOffset: Int) -> Date? {
        switch self {
        case let .interval(minutes):
            // The server's least: a worktree takes minutes to make.
            return after.addingTimeInterval(TimeInterval(max(minutes, 15) * 60))
        case let .fixedTime(hour, minute, weekdays):
            var calendar = Calendar(identifier: .gregorian)
            guard let zone = TimeZone(secondsFromGMT: serverOffset) else { return nil }
            calendar.timeZone = zone
            let today = calendar.startOfDay(for: after)
            // Today and the week after it: some day in eight is one of the
            // days asked for, and the time on it is after `after`.
            for ahead in 0...7 {
                guard let day = calendar.date(byAdding: .day, value: ahead, to: today) else { continue }
                let weekday = calendar.component(.weekday, from: day) - 1
                guard weekdays.isEmpty || weekdays.contains(weekday),
                      let at = calendar.date(bySettingHour: hour, minute: minute, second: 0, of: day),
                      at > after else { continue }
                return at
            }
            return nil
        }
    }
}

/// How the server's clock stands against the phone's, in words ("2 hours
/// ahead"); nil when they agree, which is when there is nothing to say.
func clockDifference(serverOffset: Int, localOffset: Int = TimeZone.current.secondsFromGMT()) -> String? {
    let minutes = (serverOffset - localOffset) / 60
    guard minutes != 0 else { return nil }
    let hours = abs(minutes) / 60, rest = abs(minutes) % 60
    var words: [String] = []
    if hours > 0 { words.append("\(hours) hour\(hours == 1 ? "" : "s")") }
    if rest > 0 { words.append("\(rest) minute\(rest == 1 ? "" : "s")") }
    return words.joined(separator: " ") + (minutes > 0 ? " ahead" : " behind")
}

/// What stays behind on the server's checkout: the sandbox clones the
/// remote, so uncommitted work and unpushed commits do not come with it.
/// Worth saying before the session starts rather than after the agent has
/// failed to find them. nil when nothing does.
func driftWords(_ facts: Facts) -> String? {
    var bits: [String] = []
    if facts.uncommitted > 0 { bits.append("\(facts.uncommitted) uncommitted") }
    if let n = facts.unpushed, n > 0 { bits.append("\(n) unpushed") }
    if !facts.baseOnRemote { bits.append("branch not on the remote") }
    guard !bits.isEmpty else { return nil }
    return bits.joined(separator: " · ") + " on the server's checkout, which this won't have."
}

/// The task a pull request starts with: `pullTask` in the desktop's
/// NewWorktree.tsx.
func pullTask(_ p: PullRequestSummary, review: Bool) -> String {
    let where_ = "This worktree is on the pull request's own branch, `\(p.branch)`, which merges into `\(p.base)`."
    if review {
        return "Review \(pullName(p)): \(p.title)\n\(p.url)\n\n\(where_) "
            + "Read what it changes with `git diff origin/\(p.base)...HEAD` and its commits with "
            + "`git log origin/\(p.base)..HEAD`, run the tests that cover it, and report what you "
            + "find: what is wrong first, then what is risky, then anything smaller, each with its file "
            + "and line. Do not commit, push, or comment on the pull request."
    }
    return "\(pullName(p)): \(p.title)\n\(p.url)\n\n\(where_) Pushing updates the pull request.\n\n"
}
