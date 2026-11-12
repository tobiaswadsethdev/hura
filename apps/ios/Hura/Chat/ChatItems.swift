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

/// A `!` command you ran in the sandbox, and how it ended once it has.
struct ShellRun: Equatable {
    var command: String
    var at: Date?
    /// What it printed; nil while it runs.
    var output: String?
    /// Nil when it did not exit by itself: stopped, or out of time.
    var code: Int?
    var stopped: Bool
    var truncated: Bool

    var running: Bool { output == nil }
    var failed: Bool { output != nil && !stopped && code != 0 }
}

/// What the chat shows, folded from transcript entries.
enum ChatItem: Identifiable, Equatable {
    case prompt(id: String, prompt: Prompt)
    case shell(id: String, run: ShellRun)
    /// The first thing said on a day, so a time is never read as the wrong one.
    case day(id: String, date: Date)
    /// What Claude said, and when.
    case text(id: String, text: String, at: Date?)
    case thinking(id: String, text: String)
    case tool(id: String, call: ToolCall)
    case turnEnd(id: String, ms: Int64, at: Date?)
    case divider(id: String, reason: String)
    case notice(id: String, text: String, isError: Bool)
    /// The steps of a turn on the way to its answer, folded into one row: see
    /// `group`. `live` while the turn they are in is still running.
    indirect case steps(id: String, items: [ChatItem], live: Bool)

    var id: String {
        switch self {
        case let .prompt(id, _), let .shell(id, _), let .day(id, _), let .text(id, _, _), let .thinking(id, _),
             let .tool(id, _), let .turnEnd(id, _, _), let .divider(id, _), let .notice(id, _, _), let .steps(id, _, _):
            id
        }
    }

    /// One of a turn's steps, to be folded with the others: what the agent
    /// did and thought on the way. Not what you said or ran, not a failure,
    /// and not a plan or a question, which wait on you or are what you
    /// answered. As the desktop's `isStep` in `transcript.ts`.
    private var isStep: Bool {
        switch self {
        case .thinking, .text: true
        case let .tool(_, call): call.name != "ExitPlanMode" && call.name != "AskUserQuestion"
        case let .notice(_, _, isError): !isError
        default: false
        }
    }

    /// The items with each turn's steps folded into one `steps` row, so a
    /// turn reads as what you said, one row for the work, and what Claude
    /// said at the end, as the desktop draws it (`group` in `transcript.ts`).
    ///
    /// The answer is the text a run of steps ends with and stays out of the
    /// fold. A run of one is left as it is: a row to open a single card is a
    /// tap for nothing. `working` makes the last run live, unless a turn has
    /// ended after it.
    static func group(_ items: [ChatItem], working: Bool) -> [ChatItem] {
        var out: [ChatItem] = []
        var run: [ChatItem] = []
        func flush(live: Bool) {
            var end = run.count
            while end > 0, case .text = run[end - 1] { end -= 1 }
            let steps = Array(run[..<end])
            if steps.count > 1 {
                out.append(.steps(id: "\(steps[0].id).steps", items: steps, live: live))
            } else {
                out += steps
            }
            out += run[end...]
            run = []
        }
        for item in items {
            if item.isStep {
                run.append(item)
            } else {
                flush(live: false)
                out.append(item)
            }
        }
        flush(live: working)
        return out
    }

    /// Where an entry is drawn: the row to mark, and the folded turn it is
    /// in, which has to open for the row to be seen. As `locate` in the
    /// desktop's `transcript.ts`: a row's id starts with its entry's seq, so
    /// the entry a search found is the first row whose id does; one that
    /// draws nothing here -- a subagent's message, an answer folded into its
    /// call -- falls back to the last row before it.
    struct Located: Equatable {
        var id: String
        var steps: String?
    }

    static func locate(_ items: [ChatItem], seq: UInt64) -> Located? {
        func of(_ item: ChatItem) -> UInt64? { item.id.split(separator: ".").first.flatMap { UInt64($0) } }
        var near: (at: UInt64, found: Located)?
        func walk(_ list: [ChatItem], steps: String?) -> Located? {
            for item in list {
                switch item {
                case let .steps(id, inner, _):
                    if let found = walk(inner, steps: id) { return found }
                case .day:
                    continue
                default:
                    guard let at = of(item) else { continue }
                    if at == seq { return Located(id: item.id, steps: steps) }
                    if at < seq, at >= (near?.at ?? 0) { near = (at, Located(id: item.id, steps: steps)) }
                }
            }
            return nil
        }
        return walk(items, steps: nil) ?? near?.found
    }

    /// What a turn's steps came to, in a few words: "Read 3 files, ran 2
    /// commands". Kinds of work in the order they were first done.
    static func summary(_ items: [ChatItem]) -> String {
        var order: [String] = []
        var counts: [String: Int] = [:]
        func add(_ kind: String) {
            if counts[kind] == nil { order.append(kind) }
            counts[kind, default: 0] += 1
        }
        for item in items {
            switch item {
            case .thinking: add("thought")
            case .text: add("message")
            case let .tool(_, call):
                switch call.name {
                case "Read": add("read")
                case "Edit", "MultiEdit", "Write", "NotebookEdit": add("edit")
                case "Bash": add("command")
                case "Grep", "Glob": add("search")
                case "WebFetch", "WebSearch": add("web")
                case "Task", "Agent": add("agent")
                case "TodoWrite": add("todo")
                default: add("tool")
                }
            default: continue
            }
        }
        func n(_ k: Int, _ one: String, _ many: String) -> String { k == 1 ? one : "\(k) \(many)" }
        var parts: [String] = order.compactMap { kind in
            let k = counts[kind] ?? 0
            switch kind {
            case "read": return "read \(n(k, "a file", "files"))"
            case "edit": return "edited \(n(k, "a file", "files"))"
            case "command": return "ran \(n(k, "a command", "commands"))"
            case "search": return "searched \(k == 1 ? "once" : "\(k) times")"
            case "web": return "looked up \(n(k, "a page", "pages"))"
            case "agent": return "started \(n(k, "an agent", "agents"))"
            case "todo": return "updated the plan"
            case "tool": return "used \(n(k, "a tool", "tools"))"
            default: return nil
            }
        }
        // Thinking and messages on the way only say so when nothing else does.
        if parts.isEmpty { parts = [counts["thought"] != nil ? "thought" : "worked"] }
        let said = parts.joined(separator: ", ")
        return said.prefix(1).uppercased() + said.dropFirst()
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
        // How each `!` command ended, by its uuid: the end comes after it.
        var exits: [String: TranscriptEntry] = [:]
        for e in entries where e.kind == TranscriptEntry.Kind.exited {
            if let uuid = e.uuid { exits[uuid] = e }
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
        /// The first thing on a day goes under a line naming the day.
        func newDay(_ id: String, at: Date?) {
            guard let at else { return }
            if day.map({ !Calendar.current.isDate($0, inSameDayAs: at) }) ?? true {
                items.append(.day(id: "\(id).day", date: at))
            }
            day = at
        }
        func place(_ id: String, _ prompt: Prompt, at: Date?) {
            newDay(id, at: at)
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
            case TranscriptEntry.Kind.shell:
                let end = e.uuid.flatMap { exits[$0] }
                newDay(id(0), at: e.at)
                items.append(.shell(id: id(0), run: ShellRun(
                    command: blocks.first?.text ?? "", at: e.at, output: end.map { $0.blocks?.first?.text ?? "" },
                    code: end?.code, stopped: end?.stopped ?? false,
                    truncated: end?.blocks?.first?.truncated ?? false)))
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
    var icon: Glyph
    var verb: String
    var subject: String

    static func of(_ call: ToolCall, cwd: String) -> ToolLabel {
        let input = call.input
        let path = input["file_path"]?.string ?? input["path"]?.string ?? input["notebook_path"]?.string
        let shown = path.map { relative($0, to: cwd) } ?? ""
        switch call.name {
        case "Bash": return ToolLabel(icon: .agentTerminal, verb: "Run", subject: input["description"]?.string ?? input["command"]?.string ?? "")
        case "Read": return ToolLabel(icon: .fileText, verb: "Read", subject: shown)
        case "Edit", "MultiEdit": return ToolLabel(icon: .edit, verb: "Edit", subject: shown)
        case "Write": return ToolLabel(icon: .fileNew, verb: "Write", subject: shown)
        case "NotebookEdit": return ToolLabel(icon: .edit, verb: "Edit notebook", subject: shown)
        case "Glob": return ToolLabel(icon: .search, verb: "Find files", subject: input["pattern"]?.string ?? "")
        case "Grep": return ToolLabel(icon: .search, verb: "Search", subject: input["pattern"]?.string ?? "")
        case "WebFetch": return ToolLabel(icon: .web, verb: "Fetch", subject: input["url"]?.string ?? "")
        case "WebSearch": return ToolLabel(icon: .web, verb: "Search the web", subject: input["query"]?.string ?? "")
        case "Task", "Agent": return ToolLabel(icon: .agent, verb: "Agent", subject: input["description"]?.string ?? "")
        case "SubagentHandback": return ToolLabel(icon: .reply, verb: "Report", subject: "")
        case "TodoWrite": return ToolLabel(icon: .todos, verb: "Update todos", subject: "")
        case "ExitPlanMode": return ToolLabel(icon: .plan, verb: "Plan", subject: "")
        case "AskUserQuestion": return ToolLabel(icon: .question, verb: "Ask", subject: "")
        default:
            let parts = call.name.components(separatedBy: "__")
            if parts.count >= 3, parts[0] == "mcp" {
                return ToolLabel(icon: .integrations, verb: parts[1], subject: parts[2...].joined(separator: "__"))
            }
            return ToolLabel(icon: .tool, verb: call.name, subject: shown)
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
        // \r\n as \n: a stray \r draws as a second line break.
        let a = old.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        let b = new.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
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

    /// What Claude Code names the parent of a typed skill's messages, before
    /// the skill's name: they have no tool call to name.
    static let typedSkill = "forked-command-"

    /// The skill a task runs, when it is a forked one (`context: fork`), as
    /// the desktop's `forkedSkill` reads it. Claude Code marks one
    /// `skip_transcript` and `ambient`, as it does housekeeping, and names it
    /// after the command; but it is the whole of the work while it runs.
    static func forkedSkill(_ t: JSONValue) -> String? {
        guard t["task_type"]?.string == "local_agent",
              t["skip_transcript"]?.bool == true || t["ambient"]?.bool == true,
              let description = t["description"]?.string, description.hasPrefix("/")
        else { return nil }
        let name = description.dropFirst().split(whereSeparator: \.isWhitespace).first.map(String.init) ?? ""
        return name.isEmpty ? nil : name
    }

    /// Subagents only: a background shell's output is a file in the sandbox,
    /// not a conversation, and an ambient task is housekeeping, except a
    /// forked skill. Oldest first.
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
                guard t["task_type"]?.string == "local_agent" else { continue }
                let fork = forkedSkill(t)
                if t["skip_transcript"]?.bool == true || t["ambient"]?.bool == true, fork == nil { continue }
                let had = agents[id]
                if had == nil { order.append(id) }
                agents[id] = AgentInfo(
                    id: id,
                    // A resumed agent's messages stay under the call that first started it.
                    call: had?.call ?? t["tool_use_id"]?.string ?? fork.map { typedSkill + $0 },
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
