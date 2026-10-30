import HuraKit
import SwiftUI

/// What a session is, and what it has spent: its task, its worktree, what it
/// was given, and the agent's own figures once its status line has run.
struct FactsPane: View {
    let server: Server
    let name: String
    @State private var usage: Usage?

    private var session: Session? { server.session(named: name) }

    var body: some View {
        List {
            if let session {
                Section("Task") {
                    if session.task.isEmpty {
                        Text("Started at a prompt, with no task").foregroundStyle(.secondary)
                    } else {
                        Text(session.task).font(.callout).textSelection(.enabled)
                    }
                }
                spend
                Section("Worktree") {
                    LabeledContent("Branch") {
                        VStack(alignment: .trailing) {
                            Text(session.workBranch).font(.footnote.monospaced())
                            Text("from \(session.baseBranch ?? "the remote's default")").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    LabeledContent("Repository") {
                        Text(repoName(session.repo)).font(.footnote.monospaced())
                    }
                    if !session.sandbox.isEmpty {
                        LabeledContent("Sandbox") { Text(session.sandbox).font(.footnote.monospaced()) }
                    }
                }
                Section("Given") {
                    LabeledContent("Agent") {
                        Text([session.agent, usage?.model].compactMap { $0 }.joined(separator: " · ")).font(.footnote.monospaced())
                    }
                    LabeledContent("Policy", value: session.policy ?? "none recorded")
                    LabeledContent("Toolchains", value: list(session.toolchains))
                    LabeledContent("Providers", value: list(session.providers))
                    LabeledContent("Skills", value: list(session.skills))
                    LabeledContent("MCP", value: list(session.mcp))
                }
            }
        }
        .task {
            while !Task.isCancelled {
                if let poll = try? await server.api.poll(name) { usage = poll.usage }
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    @ViewBuilder
    private var spend: some View {
        Section("Spent") {
            if let usage {
                LabeledContent("Cost", value: usage.costUSD.map { String(format: "$%.2f", $0) } ?? "not yet")
                LabeledContent("Worked", value: usage.durationMS.map(duration) ?? "not yet")
                LabeledContent("Lines") {
                    if let added = usage.linesAdded {
                        Text("+\(added)").foregroundStyle(.green) + Text("  −\(usage.linesRemoved ?? 0)").foregroundStyle(.red)
                    } else {
                        Text("not yet")
                    }
                }
                if let ctx = usage.contextUsedPercentage {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text("Context")
                            Spacer()
                            Text("\(Int(ctx.rounded()))%\(usage.contextSize.map { " of \($0 / 1000)k" } ?? "")")
                                .foregroundStyle(.secondary)
                        }
                        ProgressView(value: min(100, max(0, ctx)), total: 100)
                            .tint(ctx >= 90 ? .red : ctx >= 70 ? .yellow : .accentColor)
                    }
                }
            } else {
                Text("Spending shows here once the agent has reported.").foregroundStyle(.secondary)
            }
        }
        // The account's, not this session's: what the desktop shows in its
        // header beside the worktree.
        if let windows = usage?.windows, !windows.isEmpty {
            Section("Account limits") {
                ForEach(windows, id: \.label) { w in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(w.label)
                            Spacer()
                            Text("\(Int(w.usedPercentage.rounded()))%").foregroundStyle(w.usedPercentage >= 80 ? .orange : .secondary)
                        }
                        ProgressView(value: min(100, max(0, w.usedPercentage)), total: 100)
                            .tint(w.usedPercentage >= 80 ? .orange : .accentColor)
                        if let r = w.resetsAt {
                            Text("Resets \(Date(timeIntervalSince1970: TimeInterval(r)).formatted(.relative(presentation: .named)))")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    private func list(_ values: [String]) -> String { values.isEmpty ? "none" : values.joined(separator: ", ") }

    private func repoName(_ repo: String) -> String {
        let last = repo.split(whereSeparator: { $0 == "/" || $0 == ":" }).last.map(String.init) ?? repo
        return last.hasSuffix(".git") ? String(last.dropLast(4)) : last
    }

    private func duration(_ ms: Int64) -> String {
        let s = ms / 1000
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60)m \(s % 60)s" }
        return "\(s / 3600)h \(s / 60 % 60)m"
    }
}
