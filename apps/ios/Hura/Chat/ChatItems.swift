import Foundation
import HuraKit

/// A tool call joined with its result, which arrives in a later entry.
struct ToolCall: Equatable {
    var id: String
    var name: String
    var input: JSONValue
    var inputTruncated: Bool
    var result: TranscriptBlock?

    var failed: Bool { result?.isError == true }
    /// Claude Code reports a tool cut short by Esc as an error; it's not one.
    var stopped: Bool {
        guard failed, let text = result?.text else { return false }
        return text.contains("interrupted by user") || text.contains("The user doesn't want to proceed")
            || text.contains("doesn't want to take this action")
    }
}

/// What you said, with the pictures you sent and the names of the other
/// files; when it was sent, and when Claude took it into a turn.
struct Prompt: Equatable {
    var text: String
    var pictures: [Picture]
    var files: [String]
    var sent: Date?
    var read: Date?
}

/// What the chat shows, folded from transcript entries.
enum ChatItem: Identifiable, Equatable {
    case prompt(id: String, prompt: Prompt)
    /// The first thing said on a day, so a time is never read as the wrong one.
    case day(id: String, date: Date)
    /// What Claude said, and when.
    case text(id: String, text: String, at: Date?)
    case thinking(id: String, text: String)
    case tool(id: String, call: ToolCall)
    case turnEnd(id: String, ms: Int64, at: Date?)
    case divider(id: String, reason: String)
    case notice(id: String, text: String, isError: Bool)

    var id: String {
        switch self {
        case let .prompt(id, _), let .day(id, _), let .text(id, _, _), let .thinking(id, _), let .tool(id, _),
             let .turnEnd(id, _, _), let .divider(id, _), let .notice(id, _, _):
            id
        }
    }

    /// Rebuilds the list from scratch; ids are `seq.block`, so SwiftUI
    /// diffs it cheaply. Your messages Claude hasn't read yet come back
    /// apart, to be drawn after everything else.
    ///
    /// A message is placed where Claude read it rather than where it was
    /// sent, as on the desktop (`transcript.ts`): one sent mid-turn arrives
    /// after that turn, beside the reply to it. Only one the host promised a
    /// receipt for waits, and not one older than the last message read, which
    /// was dropped rather than read.
    ///
    /// `thread` picks whose messages: the conversation's, or with a call's id
    /// those of the subagent it started.
    static func fold(_ entries: [TranscriptEntry], thread: String? = nil) -> (items: [ChatItem], queued: [ChatItem]) {
        var results: [String: TranscriptBlock] = [:]
        for e in entries where e.kind == TranscriptEntry.Kind.toolResult {
            for b in e.blocks ?? [] {
                if let id = b.toolUseID { results[id] = b }
            }
        }
        // When each prompt was read, by its uuid: the receipt comes after it.
        var read: [String: Date] = [:]
        var lastRead: UInt64 = 0
        for e in entries where e.kind == TranscriptEntry.Kind.read {
            if let uuid = e.uuid { read[uuid] = e.at ?? .distantPast }
            lastRead = e.seq
        }
        var items: [ChatItem] = []
        var day: Date?
        var held: [(uuid: String, id: String, prompt: Prompt)] = []
        let entries = entries.filter { $0.parent == thread }
        func place(_ id: String, _ prompt: Prompt, at: Date?) {
            if let at {
                if day.map({ !Calendar.current.isDate($0, inSameDayAs: at) }) ?? true {
                    items.append(.day(id: "\(id).day", date: at))
                }
                day = at
            }
            items.append(.prompt(id: id, prompt: prompt))
        }
        for e in entries {
            let blocks = e.blocks ?? []
            func id(_ i: Int) -> String { "\(e.seq).\(i)" }
            switch e.kind {
            case TranscriptEntry.Kind.prompt:
                let text = blocks.filter { $0.type == "text" }.compactMap(\.text).joined(separator: "\n\n")
                let pictures = blocks.filter { $0.type == "image" }.flatMap { $0.pictures ?? [] }
                let files = blocks.filter { $0.type == "file" }.map { $0.name ?? $0.text ?? "file" }
                let prompt = Prompt(text: text, pictures: pictures, files: files, sent: e.at,
                                    read: e.uuid.flatMap { read[$0] })
                if let uuid = e.uuid, read[uuid] != nil || (e.receipt == true && e.seq > lastRead) {
                    held.append((uuid, id(0), prompt))
                } else {
                    place(id(0), prompt, at: e.at)
                }
            case TranscriptEntry.Kind.read:
                if let i = held.firstIndex(where: { $0.uuid == e.uuid }) {
                    let h = held.remove(at: i)
                    place(h.id, h.prompt, at: e.at)
                }
            case TranscriptEntry.Kind.assistant:
                for (i, b) in blocks.enumerated() {
                    switch b.type {
                    case "text": items.append(.text(id: id(i), text: b.text ?? "", at: e.at))
                    case "thinking": items.append(.thinking(id: id(i), text: b.text ?? ""))
                    case "tool_use":
                        guard let toolID = b.id else { continue }
                        items.append(.tool(id: id(i), call: ToolCall(
                            id: toolID, name: b.name ?? "tool", input: b.input ?? .object([:]),
                            inputTruncated: b.truncated == true, result: results[toolID])))
                    default: continue
                    }
                }
            case TranscriptEntry.Kind.turnEnd:
                if let ms = e.durationMS { items.append(.turnEnd(id: id(0), ms: ms, at: e.at)) }
            case TranscriptEntry.Kind.divider:
                items.append(.divider(id: id(0), reason: e.reason ?? ""))
            case TranscriptEntry.Kind.notice:
                items.append(.notice(id: id(0), text: blocks.compactMap(\.text).joined(separator: "\n"), isError: e.level == "error"))
            default:
                // tool_result is folded into its call; unknown kinds are skipped.
                continue
            }
        }
        return (items, held.map { .prompt(id: $0.id, prompt: $0.prompt) })
    }
}

/// How a tool call is labelled: an SF Symbol, a verb and what it acts on.
struct ToolLabel: Equatable {
    var icon: String
    var verb: String
    var subject: String

    static func of(_ call: ToolCall, cwd: String) -> ToolLabel {
        let input = call.input
        let path = input["file_path"]?.string ?? input["path"]?.string ?? input["notebook_path"]?.string
        let shown = path.map { relative($0, to: cwd) } ?? ""
        switch call.name {
        case "Bash": return ToolLabel(icon: "terminal", verb: "Run", subject: input["description"]?.string ?? input["command"]?.string ?? "")
        case "Read": return ToolLabel(icon: "doc.text", verb: "Read", subject: shown)
        case "Edit", "MultiEdit": return ToolLabel(icon: "pencil", verb: "Edit", subject: shown)
        case "Write": return ToolLabel(icon: "doc.badge.plus", verb: "Write", subject: shown)
        case "NotebookEdit": return ToolLabel(icon: "pencil", verb: "Edit notebook", subject: shown)
        case "Glob": return ToolLabel(icon: "magnifyingglass", verb: "Find files", subject: input["pattern"]?.string ?? "")
        case "Grep": return ToolLabel(icon: "magnifyingglass", verb: "Search", subject: input["pattern"]?.string ?? "")
        case "WebFetch": return ToolLabel(icon: "globe", verb: "Fetch", subject: input["url"]?.string ?? "")
        case "WebSearch": return ToolLabel(icon: "globe", verb: "Search the web", subject: input["query"]?.string ?? "")
        case "Task", "Agent": return ToolLabel(icon: "person.2", verb: "Agent", subject: input["description"]?.string ?? "")
        case "SubagentHandback": return ToolLabel(icon: "arrowshape.turn.up.left", verb: "Report", subject: "")
        case "TodoWrite": return ToolLabel(icon: "checklist", verb: "Update todos", subject: "")
        case "ExitPlanMode": return ToolLabel(icon: "list.bullet.clipboard", verb: "Plan", subject: "")
        case "AskUserQuestion": return ToolLabel(icon: "questionmark.bubble", verb: "Ask", subject: "")
        default:
            let parts = call.name.components(separatedBy: "__")
            if parts.count >= 3, parts[0] == "mcp" {
                return ToolLabel(icon: "puzzlepiece.extension", verb: parts[1], subject: parts[2...].joined(separator: "__"))
            }
            return ToolLabel(icon: "wrench.and.screwdriver", verb: call.name, subject: shown)
        }
    }

    static func relative(_ path: String, to cwd: String) -> String {
        let base = cwd.hasSuffix("/") ? cwd : cwd + "/"
        return path.hasPrefix(base) ? String(path.dropFirst(base.count)) : path
    }
}

/// A line diff for Edit cards: longest common subsequence over lines, which
/// is plenty for the small replacements Edit makes.
enum LineDiff {
    enum Line: Equatable {
        case same(String)
        case removed(String)
        case added(String)
    }

    static func diff(_ old: String, _ new: String) -> [Line] {
        let a = old.components(separatedBy: "\n"), b = new.components(separatedBy: "\n")
        // Large edits: show them as a replacement rather than spend time on LCS.
        guard a.count * b.count <= 250_000 else { return a.map(Line.removed) + b.map(Line.added) }
        var lcs = Array(repeating: Array(repeating: 0, count: b.count + 1), count: a.count + 1)
        for i in stride(from: a.count - 1, through: 0, by: -1) {
            for j in stride(from: b.count - 1, through: 0, by: -1) {
                lcs[i][j] = a[i] == b[j] ? lcs[i + 1][j + 1] + 1 : max(lcs[i + 1][j], lcs[i][j + 1])
            }
        }
        var out: [Line] = []
        var i = 0, j = 0
        while i < a.count, j < b.count {
            if a[i] == b[j] {
                out.append(.same(a[i])); i += 1; j += 1
            } else if lcs[i + 1][j] >= lcs[i][j + 1] {
                out.append(.removed(a[i])); i += 1
            } else {
                out.append(.added(b[j])); j += 1
            }
        }
        out += a[i...].map(Line.removed) + b[j...].map(Line.added)
        return out
    }
}

/// A subagent the conversation started, from the task events Claude Code
/// keeps, as the desktop window folds them (`transcript.ts`).
struct AgentInfo: Identifiable, Equatable {
    /// The task's id, which a resumed agent keeps.
    var id: String
    /// The call that started it: its messages carry this as their parent.
    var call: String?
    var description: String
    /// `general-purpose`, `Explore` or a custom agent's name.
    var type: String?
    /// Started in the background, so it outlives the turn that started it.
    var background: Bool
    /// `running`, `completed`, `failed` or `stopped`.
    var status: String
    var started: Date?
    var ended: Date?
    var prompt: String
    var tokens: Int?
    var toolUses: Int?

    var running: Bool { status == "running" }

    /// Subagents only: a background shell's output is a file in the sandbox,
    /// not a conversation, and an ambient task is housekeeping. Oldest first.
    static func all(_ entries: [TranscriptEntry]) -> [AgentInfo] {
        var agents: [String: AgentInfo] = [:]
        var order: [String] = []
        func status(_ s: String?) -> String {
            switch s {
            case "completed", "failed": s!
            case "stopped", "killed": "stopped"
            default: "running"
            }
        }
        for e in entries where e.kind == TranscriptEntry.Kind.task {
            guard let t = e.task, let id = t["task_id"]?.string else { continue }
            switch t["subtype"]?.string {
            case "task_started":
                guard t["task_type"]?.string == "local_agent", t["skip_transcript"]?.bool != true,
                      t["ambient"]?.bool != true else { continue }
                let had = agents[id]
                if had == nil { order.append(id) }
                agents[id] = AgentInfo(
                    id: id,
                    // A resumed agent's messages stay under the call that first started it.
                    call: had?.call ?? t["tool_use_id"]?.string,
                    description: t["description"]?.string ?? had?.description ?? "Agent",
                    type: t["subagent_type"]?.string,
                    background: t["is_backgrounded"]?.bool ?? false,
                    status: "running",
                    started: had?.started ?? e.at,
                    ended: nil,
                    prompt: had?.prompt.isEmpty == false ? had!.prompt : (t["prompt"]?.string ?? ""),
                    tokens: had?.tokens, toolUses: had?.toolUses)
            case "task_updated":
                guard var a = agents[id] else { continue }
                if let s = t["patch"]?["status"]?.string { a.status = status(s) }
                if case let .number(ms)? = t["patch"]?["end_time"] { a.ended = Date(timeIntervalSince1970: ms / 1000) }
                agents[id] = a
            case "task_notification":
                guard var a = agents[id] else { continue }
                a.status = status(t["status"]?.string)
                a.ended = a.ended ?? e.at
                if case let .number(n)? = t["usage"]?["total_tokens"] { a.tokens = Int(n) }
                if case let .number(n)? = t["usage"]?["tool_uses"] { a.toolUses = Int(n) }
                agents[id] = a
            default:
                continue
            }
        }
        return order.compactMap { agents[$0] }
    }
}
