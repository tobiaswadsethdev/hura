import HuraKit
import SwiftUI

/// The work going on beside the conversation, above the composer: the
/// subagents it has started and the commands it left running in the
/// background, running ones first. Either returns at once and the turn goes
/// on without it, so it would otherwise be nowhere to see.
struct AgentsStrip: View {
    let agents: [AgentInfo]
    var commands: [BackgroundCommand] = []
    let open: (AgentInfo) -> Void
    var openCommand: ((BackgroundCommand) -> Void)?

    /// One chip, a subagent or a command.
    private enum Work: Identifiable {
        case agent(AgentInfo)
        case command(BackgroundCommand)

        var id: String {
            switch self {
            case let .agent(a): "agent.\(a.id)"
            case let .command(c): "command.\(c.id)"
            }
        }
        var running: Bool {
            switch self {
            case let .agent(a): a.running
            case let .command(c): c.running
            }
        }
        var latest: Date {
            switch self {
            case let .agent(a): a.ended ?? a.started ?? .distantPast
            case let .command(c): c.ended ?? c.started ?? .distantPast
            }
        }
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(sorted) { work in
                    switch work {
                    case let .agent(a):
                        Button { open(a) } label: {
                            chip { WorkStatus(status: a.status) } title: { Text(a.description) }
                        }
                        .accessibilityLabel("\(a.description), \(a.running ? "running" : a.status)")
                    case let .command(c):
                        Button { openCommand?(c) } label: {
                            chip {
                                WorkStatus(status: c.status)
                            } title: {
                                Image(glyph: .agentTerminal).foregroundStyle(Color.dim)
                                Text(c.title)
                                if let started = c.started {
                                    Group {
                                        if let ended = c.ended {
                                            Text(ChatFormat.duration(Int64(max(0, ended.timeIntervalSince(started)) * 1000)))
                                        } else {
                                            Elapsed(since: started)
                                        }
                                    }
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(Color.dimmer)
                                }
                            }
                        }
                        .accessibilityLabel("Command \(c.title), \(c.running ? "running" : c.status)")
                    }
                }
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 12)
            .padding(.vertical, 2)
        }
    }

    /// A pill on the canvas, a card's surface with its hairline.
    private func chip(@ViewBuilder status: () -> some View, @ViewBuilder title: () -> some View) -> some View {
        HStack(spacing: 5) {
            status()
            title()
        }
        .lineLimit(1)
        .font(.caption)
        .foregroundStyle(Color.ink)
        .padding(.horizontal, 11)
        .padding(.vertical, 7)
        .background(Color.surface, in: Capsule())
        .overlay(Capsule().strokeBorder(Color.line, lineWidth: 1))
        .contentShape(Capsule())
    }

    /// Running ones first, then the most recently finished.
    private var sorted: [Work] {
        (agents.map(Work.agent) + commands.map(Work.command)).sorted {
            if $0.running != $1.running { return $0.running }
            return $0.latest > $1.latest
        }
    }
}

/// How a subagent or a background command is doing, in the states' hues.
struct WorkStatus: View {
    let status: String

    var body: some View {
        switch status {
        case "running": Spinner().foregroundStyle(Color.working)
        case "completed": Image(glyph: .passing).foregroundStyle(Color.ok)
        case "stopped": Image(glyph: .stop).foregroundStyle(Color.dim)
        default: Image(glyph: .failing).foregroundStyle(Color.bad)
        }
    }
}

/// A command the agent left running in the background, followed: what it
/// runs, what Claude Code said of it when it ended, and the end of what it
/// has printed, read again every two seconds while it runs.
struct CommandScreen: View {
    let chat: Chat
    let id: String
    /// Reads the end of a file in the sandbox.
    let tail: (String) async throws -> TailText
    @Environment(\.dismiss) private var dismiss
    @State private var output: TailText?
    @State private var failure: String?

    private var command: BackgroundCommand? {
        BackgroundCommand.all(chat.entries).first { $0.id == id }
    }

    var body: some View {
        NavigationStack {
            Group {
                if let command {
                    content(command)
                } else {
                    Text("This command is gone from the conversation.").foregroundStyle(Color.dim)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.canvas)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .navigationBarTitleDisplayMode(.inline)
        }
        // Again every two seconds while it runs, and once more when it ends.
        .task(id: command?.running) {
            guard let path = command?.output else { return }
            repeat {
                do {
                    output = try await tail(path)
                    failure = nil
                } catch {
                    failure = error.userMessage
                }
                try? await Task.sleep(for: .seconds(2))
            } while !Task.isCancelled && command?.running == true
        }
    }

    private func content(_ c: BackgroundCommand) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        HStack(spacing: 6) {
                            WorkStatus(status: c.status)
                            Text(facts(c, now: context.date))
                        }
                        .font(.caption.monospaced())
                        .foregroundStyle(Color.dim)
                    }
                    if !c.command.isEmpty { CodeBlock(text: "$ " + c.command) }
                    if let summary = c.summary {
                        Text(summary).font(.subheadline).foregroundStyle(Color.ink)
                    }
                    if let failure {
                        Text(failure).font(.caption).foregroundStyle(Color.bad)
                    }
                    if let output {
                        if output.cut {
                            Text("The last \(ByteCountFormatter.string(fromByteCount: Int64(output.text.utf8.count), countStyle: .file)) of \(ByteCountFormatter.string(fromByteCount: Int64(output.bytes), countStyle: .file))")
                                .font(.caption2)
                                .foregroundStyle(Color.dimmer)
                        }
                        CodeBlock(text: output.text.isEmpty ? "Nothing printed yet." : Self.plain(output.text))
                    } else if c.output == nil {
                        Text("The transcript doesn't say where its output goes.")
                            .font(.caption)
                            .foregroundStyle(Color.dim)
                    } else if failure == nil {
                        Spinner().foregroundStyle(Color.dim).frame(maxWidth: .infinity)
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
            }
            // Followed to its end by scrolling rather than anchored there: an
            // anchor at the bottom left output shorter than the screen
            // hanging under a gap.
            .onChange(of: output?.bytes) { proxy.scrollTo("end", anchor: .bottom) }
        }
        .navigationTitle(c.title)
    }

    private func facts(_ c: BackgroundCommand, now: Date) -> String {
        let ms = c.started.map { Int64(max(0, (c.ended ?? now).timeIntervalSince($0)) * 1000) }
        let state = c.running ? "running" : c.status == "completed" ? "done" : c.status
        return [state, ms.map(ChatFormat.duration)].compactMap { $0 }.joined(separator: " · ")
    }

    /// The output without a terminal's colours: a dev server's log is full
    /// of escapes a text view would print as they are.
    static func plain(_ text: String) -> String {
        text.replacing(/\u{1B}\[[0-9;?]*[ -\/]*[@-~]/, with: "")
    }
}

/// One subagent, followed as it works: what it was asked, what it is doing
/// now, and everything it has said and done.
struct AgentScreen: View {
    let chat: Chat
    let id: String
    @Environment(\.dismiss) private var dismiss
    @State private var instructions = false

    private static let cwd = "/sandbox/repo"

    var body: some View {
        NavigationStack {
            Group {
                if let agent = AgentInfo.all(chat.entries).first(where: { $0.id == id }) {
                    content(agent)
                } else {
                    Text("This agent is gone from the conversation.").foregroundStyle(.secondary)
                }
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func content(_ agent: AgentInfo) -> some View {
        let progress = chat.tasks[agent.id]
        let items = agent.call.map { ChatItem.fold(chat.entries, thread: $0).items } ?? []
        return ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Text(facts(agent, progress: progress, now: context.date))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                    }
                    if agent.running, let summary = progress?["summary"]?.string {
                        Text(summary).font(.subheadline).foregroundStyle(.tint)
                    }
                    if !agent.prompt.isEmpty {
                        DisclosureGroup("Instructions", isExpanded: $instructions) {
                            MarkdownText(text: agent.prompt).padding(.top, 6)
                        }
                        .font(.subheadline)
                    }
                    ChatRows(items: items, cwd: Self.cwd, working: agent.running)
                    if agent.running {
                        HStack(spacing: 8) {
                            Spinner(size: 16).foregroundStyle(Color.working)
                            if let tool = progress?["last_tool_name"]?.string {
                                Text(tool).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    } else if items.isEmpty {
                        Text("Nothing it did came through.").font(.caption).foregroundStyle(.secondary)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: items.count) { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
        }
        .navigationTitle(agent.description)
        .background(Color.canvas)
    }

    private func facts(_ agent: AgentInfo, progress: JSONValue?, now: Date) -> String {
        var tokens = agent.tokens
        var tools = agent.toolUses
        if case let .number(n)? = progress?["usage"]?["total_tokens"] { tokens = Int(n) }
        if case let .number(n)? = progress?["usage"]?["tool_uses"] { tools = Int(n) }
        let ms = agent.started.map { Int64(((agent.ended ?? now).timeIntervalSince($0)) * 1000) }
        return [
            agent.type,
            agent.background ? "background" : nil,
            agent.running ? "running" : agent.status == "completed" ? "done" : agent.status,
            ms.map { ChatFormat.duration(max(0, $0)) },
            tools.map { "\($0) tool use\($0 == 1 ? "" : "s")" },
            tokens.map { $0 < 1000 ? "\($0) tokens" : String(format: "%.1fk tokens", Double($0) / 1000) },
        ].compactMap { $0 }.joined(separator: " · ")
    }
}
