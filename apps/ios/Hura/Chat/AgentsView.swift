import HuraKit
import SwiftUI

/// The subagents a conversation has started, above the composer, running
/// ones first. A background subagent's call returns at once and the turn
/// goes on without it, so its work would otherwise be nowhere to see.
struct AgentsStrip: View {
    let agents: [AgentInfo]
    let open: (AgentInfo) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(sorted) { a in
                    Button { open(a) } label: {
                        HStack(spacing: 5) {
                            AgentStatus(agent: a)
                            Text(a.description).lineLimit(1)
                        }
                        .font(.caption)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .contentShape(Capsule())
                        .glass(interactive: true)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(a.description), \(a.running ? "running" : a.status)")
                }
            }
            .glassGroup(spacing: 6)
            // Room for the glass's edge and shadow, which the scroll view
            // would otherwise clip.
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
        }
    }

    /// Running ones first, then the most recently finished.
    private var sorted: [AgentInfo] {
        agents.sorted {
            if $0.running != $1.running { return $0.running }
            return ($0.ended ?? $0.started ?? .distantPast) > ($1.ended ?? $1.started ?? .distantPast)
        }
    }
}

struct AgentStatus: View {
    let agent: AgentInfo

    var body: some View {
        switch agent.status {
        case "running": WorkingDots().foregroundStyle(Color.working)
        case "completed": Image(glyph: .passing).foregroundStyle(.green)
        default: Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
        }
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
        .environment(\.colorScheme, .dark)
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
                            WorkingDots(size: 22).foregroundStyle(Color.working)
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
        .background(Color.black)
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
