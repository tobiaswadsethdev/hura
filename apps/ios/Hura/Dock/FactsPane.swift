import HuraKit
import SwiftUI

/// What a session is, and what it has done: its task, its worktree, what it
/// was given, and the agent's own figures once its status line has run. How
/// full the context is, and the account's limits, are in the chat composer.
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
                progress
                Section("Worktree") {
                    LabeledContent("Branch") {
                        VStack(alignment: .trailing) {
                            Text(session.workBranch).font(.footnote)
                            Text("from \(session.baseBranch ?? "the remote's default")").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    LabeledContent("Repository") {
                        Text(repoName(session.repo)).font(.footnote)
                    }
                    if session.isWorktree {
                        LabeledContent("Runs") {
                            Label("On the server, not isolated", glyph: .unsandboxed)
                                .labelStyle(.titleAndIcon)
                                .foregroundStyle(.orange)
                        }
                        if let dir = session.workdir {
                            LabeledContent("Directory") { Text(dir).font(.footnote.monospaced()) }
                        }
                    } else if !session.sandbox.isEmpty {
                        LabeledContent("Sandbox") { Text(session.sandbox).font(.footnote) }
                    }
                }
                Section("Given") {
                    LabeledContent("Agent") {
                        Text([session.agent, usage?.model].compactMap { $0 }.joined(separator: " · ")).font(.footnote)
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
    private var progress: some View {
        Section("Progress") {
            if let usage {
                LabeledContent("Worked", value: usage.durationMS.map(duration) ?? "not yet")
                LabeledContent("Lines") {
                    if let added = usage.linesAdded {
                        Text("+\(added)").foregroundStyle(.green) + Text("  −\(usage.linesRemoved ?? 0)").foregroundStyle(.red)
                    } else {
                        Text("not yet")
                    }
                }
            } else {
                Text("Spending shows here once the agent has reported.").foregroundStyle(.secondary)
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
