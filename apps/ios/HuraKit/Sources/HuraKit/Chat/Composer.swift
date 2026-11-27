// What the composer works out from what is typed and what the conversation
// holds, without a screen: the `@` being typed, where an edited message forks
// from, and the commands the agent left running. The desktop's are in
// `apps/desktop/src/panes/Chat.tsx` and `chat/transcript.ts`.

import Foundation

/// A file or folder being mentioned: `@` and what follows it, as the last
/// word of the message. Claude Code reads `@path` as a file to look at.
public struct Mention: Equatable, Sendable {
    /// What is typed after the `@`, to find paths by.
    public var query: String
    /// The `@word` in the text, which a path picked replaces.
    public var range: Range<String.Index>
}

extension ComposerInput {
    /// The mention being typed: the word at the end of the text, when it
    /// starts with `@`. The end and not the cursor, since a text field on the
    /// phone does not say where its cursor is; a mention is nearly always
    /// typed last. An address (`me@example.com`) does not start with one.
    public static func mention(_ text: String) -> Mention? {
        let start = text.lastIndex(where: \.isWhitespace).map(text.index(after:)) ?? text.startIndex
        let word = text[start...]
        guard word.hasPrefix("@") else { return nil }
        return Mention(query: String(word.dropFirst()), range: start..<text.endIndex)
    }

    /// The text with the mention being typed replaced by `path`, and a space
    /// after it to go on typing.
    public static func mentioning(_ path: String, in text: String) -> String {
        guard let m = mention(text) else { return text }
        var out = text
        out.replaceSubrange(m.range, with: "@\(path) ")
        return out
    }
}

extension ChatTranscript {
    /// The last thing the agent said before the entry at `seq`, by the uuid
    /// Claude Code gave it: where a conversation forks to edit the message at
    /// `seq`. Its own messages only, not a subagent's. Nil before anything was
    /// said, when there is nothing to keep and a plain conversation will do.
    /// As `assistantBefore` in the desktop's `panes/Chat.tsx`.
    public static func assistantBefore(_ entries: [TranscriptEntry], seq: UInt64) -> String? {
        var at: String?
        for e in entries {
            if e.seq >= seq { break }
            if e.kind == TranscriptEntry.Kind.assistant, e.parent == nil, let uuid = e.uuid { at = uuid }
        }
        return at
    }
}

/// A command the agent left running in the background: a Bash call with
/// `run_in_background`, which returns at once while the command goes on, and
/// which Claude Code follows as a `local_bash` task. Its output goes to a file
/// in the sandbox, which is where the phone reads it from (`tailFile`). As
/// `Command` in the desktop's `transcript.ts`.
public struct BackgroundCommand: Identifiable, Equatable, Sendable {
    /// The call's id, or the task's when no call came through.
    public var id: String
    /// What Claude Code calls it: the id `BashOutput` and `KillShell` name.
    public var taskID: String?
    public var command: String
    /// What the agent said it was for, when it said.
    public var description: String
    /// `running`, `completed`, `failed` or `stopped`, as a subagent's.
    public var status: String
    public var started: Date?
    public var ended: Date?
    /// Its output file in the sandbox, when the transcript says where.
    public var output: String?
    /// What Claude Code said about it when it ended.
    public var summary: String?

    public var running: Bool { status == "running" }

    /// What to call it in a list: what it is for, or the command itself.
    public var title: String { description.isEmpty ? command : description }

    /// Each Bash call that asked to run in the background, with what its
    /// answer and Claude Code's task events say about how it went, and any
    /// `local_bash` task no call came through for. Oldest first.
    public static func all(_ entries: [TranscriptEntry]) -> [BackgroundCommand] {
        struct Task {
            var call: String?
            var description: String
            var started: Date?
            var status = "running"
            var ended: Date?
            var output: String?
            var summary: String?
        }
        struct Call {
            var id: String
            var name: String
            var input: JSONValue
            var at: Date?
        }
        func status(_ s: String?) -> String {
            switch s {
            case "completed", "failed": s!
            case "stopped", "killed": "stopped"
            default: "running"
            }
        }
        var calls: [Call] = []
        var results: [String: TranscriptBlock] = [:]
        var tasks: [String: Task] = [:]
        var order: [String] = []
        for e in entries {
            switch e.kind {
            case TranscriptEntry.Kind.assistant:
                for b in e.blocks ?? [] where b.type == "tool_use" {
                    if let id = b.id { calls.append(Call(id: id, name: b.name ?? "", input: b.input ?? .object([:]), at: e.at)) }
                }
            case TranscriptEntry.Kind.toolResult:
                for b in e.blocks ?? [] { if let id = b.toolUseID { results[id] = b } }
            case TranscriptEntry.Kind.task:
                guard let t = e.task, let id = t["task_id"]?.string else { continue }
                switch t["subtype"]?.string {
                case "task_started":
                    guard t["task_type"]?.string == "local_bash" else { continue }
                    if tasks[id] == nil { order.append(id) }
                    tasks[id] = Task(call: t["tool_use_id"]?.string, description: t["description"]?.string ?? "",
                                     started: e.at, output: t["output_file"]?.string)
                case "task_updated":
                    guard var task = tasks[id] else { continue }
                    if let s = t["patch"]?["status"]?.string { task.status = status(s) }
                    if case let .number(ms)? = t["patch"]?["end_time"] { task.ended = Date(timeIntervalSince1970: ms / 1000) }
                    if let d = t["patch"]?["description"]?.string { task.description = d }
                    tasks[id] = task
                case "task_notification":
                    guard var task = tasks[id] else { continue }
                    task.status = status(t["status"]?.string)
                    task.ended = task.ended ?? e.at
                    if let s = t["summary"]?.string, !s.isEmpty { task.summary = s }
                    if let o = t["output_file"]?.string, !o.isEmpty { task.output = o }
                    tasks[id] = task
                default:
                    continue
                }
            default:
                continue
            }
        }
        // Stopped by the agent itself, by the task's id.
        let killed = Set(calls.filter { ($0.name == "KillShell" || $0.name == "KillBash") && results[$0.id]?.isError == false }
            .compactMap { $0.input["shell_id"]?.string ?? $0.input["bash_id"]?.string })
        var out: [BackgroundCommand] = []
        var seen = Set<String>()
        for call in calls where call.name == "Bash" && call.input["run_in_background"]?.bool == true {
            let said = results[call.id]?.text ?? ""
            let named = said.firstMatch(of: /\bID:\s*([\w-]+)/).map { String($0.1) }
            let taskID = order.first { tasks[$0]?.call == call.id } ?? named
            if let taskID { seen.insert(taskID) }
            let task = taskID.flatMap { tasks[$0] }
            let failed = results[call.id]?.isError == true && named == nil
            let written = said.firstMatch(of: /(?i)written to:?\s*(\S+)/).map {
                String($0.1).trimmingCharacters(in: CharacterSet(charactersIn: ".,;"))
            }
            let described = call.input["description"]?.string ?? ""
            out.append(BackgroundCommand(
                id: call.id, taskID: taskID, command: call.input["command"]?.string ?? "",
                description: described.isEmpty ? task?.description ?? "" : described,
                status: failed ? "failed" : taskID.map(killed.contains) == true ? "stopped" : task?.status ?? "running",
                started: call.at, ended: task?.ended, output: task?.output ?? written, summary: task?.summary))
        }
        for id in order where !seen.contains(id) {
            guard let t = tasks[id] else { continue }
            out.append(BackgroundCommand(id: id, taskID: id, command: "", description: t.description,
                                         status: killed.contains(id) ? "stopped" : t.status, started: t.started,
                                         ended: t.ended, output: t.output, summary: t.summary))
        }
        return out
    }
}
