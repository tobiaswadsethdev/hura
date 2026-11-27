import HuraKit
import SwiftUI

/// What the worktrees have spent and what the account has left, T3 Code's
/// usage card as the desktop has it (`apps/desktop/src/UsagePage.tsx`): the
/// account's rate-limit windows, the totals, and each worktree by what it
/// cost, from the same status polls the list asks for.
///
/// One server's, as the desktop's is: the cost is per worktree and the
/// worktrees are a server's. Like `ProjectDefaultsView`, it has no
/// navigation stack of its own.
struct UsageView: View {
    let server: Server
    /// Goes to a worktree, from its row; rows are not buttons without it.
    var onOpen: ((String) -> Void)?

    @State private var polled: [String: Usage] = [:]
    @State private var asked = false

    private var spent: UsageSummary {
        UsageSummary(sessions: server.sessions, usage: polled)
    }

    var body: some View {
        let spent = spent
        List {
            Section {
                if let windows = spent.windows {
                    ForEach(windows, id: \.label) { w in
                        HStack(spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(UsageWords.windowName(w.label)).foregroundStyle(.ink)
                                if let at = w.resetsAt {
                                    Text("Resets \(UsageWords.resetsIn(at))").font(.caption).foregroundStyle(.dim)
                                }
                            }
                            Spacer()
                            UsageMeter(pct: w.usedPercentage)
                        }
                        .accessibilityElement(children: .combine)
                    }
                } else {
                    Empty(title: asked ? "No limits reported yet" : "Asking…",
                          note: "An agent reports them once it has called the API.")
                }
            } header: {
                Text("This account")
            }
            .listRowBackground(Color.surface)

            Section("Spent") {
                Figure(title: "Cost", value: Text(UsageWords.dollars(spent.cost)))
                Figure(title: "Time working", value: Text(UsageWords.duration(spent.durationMS)))
                Figure(title: "Lines", value: Text("+\(spent.added)").foregroundStyle(.ok)
                    + Text(" −\(spent.removed)").foregroundStyle(.bad))
            }
            .listRowBackground(Color.surface)

            Section {
                if spent.rows.isEmpty {
                    Empty(title: "Nothing spent yet", note: "A worktree's cost shows once its agent has worked.")
                }
                ForEach(spent.rows, id: \.session.name) { row in
                    if let onOpen {
                        Button { onOpen(row.session.name) } label: { WorktreeCost(row: row) }
                            .tint(.ink)
                    } else {
                        WorktreeCost(row: row)
                    }
                }
            } header: {
                Text("By worktree")
            } footer: {
                Text("From the worktrees on this server that are running, as their agents last reported.")
            }
            .listRowBackground(Color.surface)
        }
        .scrollContentBackground(.hidden)
        .background(Color.canvas.ignoresSafeArea())
        .navigationTitle("Usage")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await poll() }
        // Again now and then while it is on screen, as the status line moves.
        .task {
            while !Task.isCancelled {
                await poll()
                try? await Task.sleep(for: .seconds(15))
            }
        }
    }

    /// Each running worktree's usage. Only the running ones: asking execs
    /// into the sandbox, and one that has stopped would be started by being
    /// asked. What a stopped one said last is kept while the screen is open.
    private func poll() async {
        let client = server.api
        let live = server.sessions.filter(\.isLive).map(\.name)
        let heard = await withTaskGroup(of: (String, Usage?).self) { group in
            for name in live {
                group.addTask { (name, try? await client.poll(name).usage) }
            }
            var out: [String: Usage] = [:]
            for await (name, usage) in group {
                if let usage { out[name] = usage }
            }
            return out
        }
        polled.merge(heard) { _, new in new }
        asked = true
    }
}

/// The figures behind the screen, from each worktree's last usage: the
/// account's windows, the sums, and the worktrees by cost, dearest first.
struct UsageSummary {
    struct Row {
        var session: Session
        var usage: Usage
    }

    var rows: [Row]
    /// nil until some worktree has heard them from the API.
    var windows: [Usage.Window]?
    var cost: Double
    var durationMS: Int64
    var added: Int64
    var removed: Int64

    init(sessions: [Session], usage: [String: Usage]) {
        rows = sessions.compactMap { s in usage[s.name].map { Row(session: s, usage: $0) } }
            .sorted { ($0.usage.costUSD ?? 0) > ($1.usage.costUSD ?? 0) }
        // Every worktree on one account reports the same windows; any that
        // has some will do.
        windows = rows.lazy.compactMap(\.usage.windows).first { !$0.isEmpty }
        cost = rows.reduce(0) { $0 + ($1.usage.costUSD ?? 0) }
        durationMS = rows.reduce(0) { $0 + ($1.usage.durationMS ?? 0) }
        added = rows.reduce(0) { $0 + ($1.usage.linesAdded ?? 0) }
        removed = rows.reduce(0) { $0 + ($1.usage.linesRemoved ?? 0) }
    }
}

/// A worktree and what it cost, with what it did under its name.
private struct WorktreeCost: View {
    let row: UsageSummary.Row

    var body: some View {
        let u = row.usage
        let details = [
            u.model.map(modelName),
            u.durationMS.flatMap { $0 > 0 ? UsageWords.duration($0) : nil },
            u.linesAdded != nil || u.linesRemoved != nil ? "+\(u.linesAdded ?? 0) −\(u.linesRemoved ?? 0)" : nil,
            u.contextUsedPercentage.map { "\(Int($0.rounded()))% of context" },
        ].compactMap { $0 }
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.session.label).foregroundStyle(.ink).lineLimit(1)
                if !details.isEmpty {
                    Text(details.joined(separator: " · ")).font(.caption).foregroundStyle(.dim).lineLimit(1)
                }
            }
            Spacer()
            Text(UsageWords.dollars(u.costUSD ?? 0))
                .font(.subheadline.weight(.medium).monospacedDigit())
                .foregroundStyle(.ink)
        }
        .contentShape(Rectangle())
    }
}

private struct Figure: View {
    let title: String
    let value: Text

    var body: some View {
        LabeledContent {
            value.font(.subheadline.weight(.medium).monospacedDigit()).foregroundStyle(.ink)
        } label: {
            Text(title).foregroundStyle(.ink)
        }
    }
}

private struct Empty: View {
    let title: String
    let note: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).foregroundStyle(.ink)
            Text(note).font(.caption).foregroundStyle(.dim)
        }
    }
}

/// A window's use as a bar, amber from four fifths, as the desktop's is.
private struct UsageMeter: View {
    let pct: Double

    var body: some View {
        let used = min(100, max(0, pct))
        HStack(spacing: 10) {
            Capsule()
                .fill(Color.surface2)
                .overlay(alignment: .leading) {
                    GeometryReader { g in
                        Capsule()
                            .fill(used >= 80 ? Color.attention : Color.accentColor)
                            .frame(width: g.size.width * used / 100)
                    }
                }
                .frame(width: 96, height: 6)
            Text("\(Int(used.rounded()))%")
                .font(.subheadline.weight(.medium).monospacedDigit())
                .foregroundStyle(.ink)
                .frame(minWidth: 40, alignment: .trailing)
        }
    }
}

/// The screen's figures in words.
enum UsageWords {
    /// "5h" and "7d", as the agent names its windows, said as a person would.
    static func windowName(_ label: String) -> String {
        switch label {
        case "5h": "Five hours"
        case "7d": "Seven days"
        default: label
        }
    }

    static func dollars(_ n: Double) -> String { String(format: "$%.2f", n) }

    static func duration(_ ms: Int64) -> String {
        let minutes = Int((Double(ms) / 60_000).rounded())
        if minutes < 60 { return "\(minutes)m" }
        return minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes / 60)h \(minutes % 60)m"
    }

    /// When a window starts afresh, relative while it is soon.
    static func resetsIn(_ at: Int64, now: Date = .now) -> String {
        let minutes = Int(((Double(at) - now.timeIntervalSince1970) / 60).rounded())
        if minutes <= 0 { return "now" }
        if minutes < 60 { return "in \(minutes)m" }
        if minutes < 24 * 60 { return "in \(Int((Double(minutes) / 60).rounded()))h" }
        return Date(timeIntervalSince1970: TimeInterval(at)).formatted(.dateTime.weekday(.abbreviated).hour().minute())
    }
}
