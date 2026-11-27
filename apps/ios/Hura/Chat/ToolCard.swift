import HuraKit
import SwiftUI

/// One tool call as a line of T3 Code's work log: a glyph for its kind, what
/// it did, and a chevron, with its answer indented under it when opened.
struct ToolCard: View {
    let call: ToolCall
    let cwd: String
    /// Claude is still working, so a call without a result is running.
    let working: Bool
    @State private var expanded: Bool?
    @State private var showAll = false
    @Environment(\.download) private var download
    @Environment(\.backgroundCommands) private var backgroundCommands

    private var label: ToolLabel { .of(call, cwd: cwd) }
    private var isOpen: Bool { expanded ?? opensItself }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                header
                // Beside the line rather than in it: a button inside the
                // line's own would open the call instead.
                if let command, let backgroundCommands {
                    Button { backgroundCommands.open(command) } label: {
                        Text("Output")
                            .font(.caption.weight(.medium))
                            .foregroundStyle(Color.ink)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Color.surface2, in: Capsule())
                            .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Output of \(command.title)")
                }
            }
            if isOpen {
                // Under the line it answers, past its glyph.
                detail
                    .padding(.leading, 26)
                    .padding(.top, 2)
                    .padding(.bottom, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    /// The call's line, which opens and closes it.
    private var header: some View {
        Button {
            // At once: see `Steady`.
            Steady.toggle { expanded = !isOpen }
        } label: {
            HStack(spacing: 8) {
                Image(glyph: label.icon)
                    .frame(width: 18)
                    .foregroundStyle(failedHard ? Color.bad : Color.dim)
                Text(label.verb)
                    .foregroundStyle(failedHard ? Color.bad : Color.ink)
                    // One word, never shortened: the subject gives way instead.
                    .fixedSize()
                Text(label.subject)
                    .foregroundStyle(Color.dim)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(1)
                    // Running, its label shines rather than a spinner turning
                    // beside it, as T3 Code's does.
                    .shine(call.result == nil && working)
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(Color.dimmer)
                    .rotationEffect(.degrees(isOpen ? 90 : 0))
                    .animation(.snappy, value: isOpen)
                Spacer(minLength: 4)
                if inBackground { backgroundStatus } else { status }
            }
            .font(.subheadline)
            .contentShape(Rectangle())
            .padding(.vertical, 5)
        }
        .buttonStyle(.plain)
        .contextMenu {
            if let download {
                ForEach(files, id: \.self) { f in
                    Button("Download \((f as NSString).lastPathComponent)", systemImage: "square.and.arrow.down") {
                        download(f)
                    }
                }
            }
        }
    }

    /// A Bash call left running in the background. Its result comes back at
    /// once and says only that it started; the command goes on, and how it is
    /// doing is the task's to say, as on the desktop (`chat/Items.tsx`).
    private var inBackground: Bool {
        call.name == "Bash" && call.input["run_in_background"]?.bool == true
    }

    /// The command the call started, where its output can be read.
    private var command: BackgroundCommand? {
        inBackground ? backgroundCommands?.started(by: call.id) : nil
    }

    private var backgroundStatus: some View {
        HStack(spacing: 5) {
            if let command {
                WorkStatus(status: command.status)
            } else if failedHard {
                Image(glyph: .failing).foregroundStyle(Color.bad)
            }
            Text("background")
                .font(.caption)
                .foregroundStyle(Color.dimmer)
                .fixedSize()
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("In the background, \(command.map { $0.running ? "running" : $0.status } ?? (failedHard ? "failed" : "started"))")
    }

    /// The files the call names, by absolute path: what it wrote, and what its
    /// answer says it saved -- a browser tool's screenshot, most often, which
    /// lands in `/tmp` and is named nowhere else. As the desktop's `filesOf`.
    private var files: [String] {
        guard let result = call.result, !call.failed else { return [] }
        let named = ["file_path", "notebook_path", "filename"].compactMap { call.input[$0]?.string }
            .filter { $0.hasPrefix("/") }
        var seen = Set<String>()
        return (named + Fetched.paths(in: result.text ?? "")).filter { seen.insert($0).inserted }
    }

    /// Calls whose files are the agent's own working rather than the point of
    /// them: still in the menu, but not a row of buttons under every read.
    private static let keeps: Set<String> = ["Read", "Edit", "MultiEdit", "Grep", "Glob"]

    /// Failed, and not because it was stopped.
    private var failedHard: Bool { call.failed && !call.stopped }

    @ViewBuilder
    private var status: some View {
        if call.result == nil {
            if working {
                if let at = call.at {
                    Elapsed(since: at, after: 5).foregroundStyle(Color.dimmer)
                }
            }
        } else if call.stopped {
            Text("stopped").font(.caption).foregroundStyle(Color.dimmer)
        } else if call.failed {
            Text("failed").font(.caption).foregroundStyle(Color.bad)
        }
    }

    /// Errors, todos and small edits are worth seeing without a tap.
    /// A call that failed stays shut, as on the desktop: its header says so.
    private var opensItself: Bool {
        if call.result?.pictures?.isEmpty == false { return true }
        switch call.name {
        case "TodoWrite", "SubagentHandback": return true
        case "Edit":
            // A diff has a line for each line of the longer side; skip the
            // diff itself when that alone rules it out.
            let lines = { (key: String) in call.input[key]?.string?.reduce(1) { $1 == "\n" ? $0 + 1 : $0 } ?? 1 }
            return max(lines("old_string"), lines("new_string")) <= 12 && diff.count <= 12
        default: return false
        }
    }

    private var diff: [LineDiff.Line] {
        LineDiff.diff(call.input["old_string"]?.string ?? "", call.input["new_string"]?.string ?? "")
    }

    @ViewBuilder
    private var detail: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch call.name {
            case "Bash":
                CodeBlock(text: "$ " + (call.input["command"]?.string ?? ""))
                output
            case "Edit":
                DiffView(lines: diff)
                errorOutput
            case "MultiEdit":
                if case let .array(edits)? = call.input["edits"] {
                    ForEach(Array(edits.enumerated()), id: \.offset) { _, e in
                        DiffView(lines: LineDiff.diff(e["old_string"]?.string ?? "", e["new_string"]?.string ?? ""))
                    }
                }
                errorOutput
            case "Write":
                CodeBlock(text: lastLines(call.input["content"]?.string ?? "", 40, fromStart: true))
                errorOutput
            case "TodoWrite":
                TodoList(todos: call.input["todos"])
            // A background subagent's report to the agent that started it.
            case "SubagentHandback":
                MarkdownText(text: call.input["message"]?.string ?? "")
            default:
                if call.input != .object([:]) {
                    CodeBlock(text: Self.pretty(call.input))
                }
                output
            }
            ForEach(Array((call.result?.pictures ?? []).enumerated()), id: \.offset) { _, p in
                PictureView(picture: p)
            }
            if let download, !Self.keeps.contains(call.name), !files.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(files, id: \.self) { f in
                            Button { download(f) } label: {
                                Label((f as NSString).lastPathComponent, systemImage: "square.and.arrow.down")
                                    .font(.footnote.monospaced())
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(Color.surface2, in: Capsule())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            if call.inputTruncated {
                Text("Input cut to 16 KiB per field.").font(.caption2).foregroundStyle(Color.dim)
            }
        }
    }

    @ViewBuilder
    private var output: some View {
        if let r = call.result {
            let text = r.text ?? ""
            let lines = text.components(separatedBy: "\n")
            if !text.isEmpty {
                CodeBlock(text: showAll ? text : lastLines(text, 30))
                    .foregroundStyle(call.failed && !call.stopped ? Color.bad : Color.ink)
                if lines.count > 30 && !showAll {
                    Button("Show \(lines.count - 30) earlier lines") { showAll = true }
                        .font(.caption)
                }
            }
            if let n = r.images, r.pictures == nil {
                Label("\(n) image\(n == 1 ? "" : "s")", systemImage: "photo").font(.caption).foregroundStyle(Color.dim)
            }
            if r.truncated == true {
                Text("Output cut to 16 KiB.").font(.caption2).foregroundStyle(Color.dim)
            }
        }
    }

    @ViewBuilder
    private var errorOutput: some View {
        if call.failed, let text = call.result?.text {
            Text(text).font(.caption).foregroundStyle(call.stopped ? Color.dim : Color.bad)
        }
    }

    private func lastLines(_ text: String, _ n: Int, fromStart: Bool = false) -> String {
        let lines = text.components(separatedBy: "\n")
        guard lines.count > n else { return text }
        return fromStart
            ? lines.prefix(n).joined(separator: "\n") + "\n… \(lines.count - n) more lines"
            : lines.suffix(n).joined(separator: "\n")
    }

    static func pretty(_ v: JSONValue) -> String {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return (try? enc.encode(v)).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }
}

/// A `!` command you ran, and what it printed. Open, unlike a tool call: you
/// ran it to see this.
struct ShellCard: View {
    let run: ShellRun
    @State private var open = true
    @State private var showAll = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                // At once: see `Steady`.
                Steady.toggle { open.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(glyph: .agentTerminal)
                        .frame(width: 18)
                        .foregroundStyle(Color.dim)
                    Text("You ran").fontWeight(.semibold)
                    Text(run.command)
                        .font(.system(.subheadline, design: .monospaced))
                        .foregroundStyle(Color.dim)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 4)
                    status
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(Color.dimmer)
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .animation(.snappy, value: open)
                }
                .font(.subheadline)
                .contentShape(Rectangle())
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
            }
            .buttonStyle(.plain)
            if open, let printed = run.output {
                Rectangle().fill(Color.line).frame(height: 1)
                output(printed)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        // A card, unlike the agent's steps: you ran it to see this.
        .card(fill: .surface)
        .overlay(RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
            .strokeBorder(run.failed ? Color.bad.opacity(0.5) : .clear, lineWidth: 1))
    }

    /// The end of it, which is where a command says how it went, with the
    /// rest a tap away.
    @ViewBuilder
    private func output(_ printed: String) -> some View {
        let text = printed.hasSuffix("\n") ? String(printed.dropLast()) : printed
        let lines = text.components(separatedBy: "\n")
        VStack(alignment: .leading, spacing: 8) {
            if text.isEmpty {
                Text("No output").font(.caption).foregroundStyle(Color.dim)
            } else {
                CodeBlock(text: showAll || lines.count <= 30 ? text : lines.suffix(30).joined(separator: "\n"))
                if lines.count > 30 && !showAll {
                    Button("Show \(lines.count - 30) earlier lines") { showAll = true }
                        .font(.caption)
                }
            }
        }
    }

    @ViewBuilder
    private var status: some View {
        if run.running {
            Spinner().foregroundStyle(Color.working)
        } else if run.stopped {
            Text("stopped").font(.caption).foregroundStyle(Color.dimmer)
        } else if run.failed {
            Text(run.code.map { "exit \($0)" } ?? "did not finish")
                .font(.caption.monospaced())
                .foregroundStyle(Color.bad)
        }
    }
}

struct DiffView: View {
    let lines: [LineDiff.Line]
    @State private var width: CGFloat = 0

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(Self.hunks(lines).enumerated()), id: \.offset) { _, hunk in
                    block(hunk)
                }
            }
            // As wide as the box at least, so a block's colour runs to its
            // edge and not only to the end of the longest line.
            .frame(minWidth: width, alignment: .leading)
            .padding(.vertical, 6)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            GeometryReader { g in
                Color.clear.onChange(of: g.size.width, initial: true) { _, w in width = w }
            }
        }
        .background(Color.sunken, in: RoundedRectangle(cornerRadius: Radius.control))
        .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line, lineWidth: 1))
        .textSelection(.enabled)
    }

    /// Changed lines that follow each other, removed and added alike, drawn
    /// as one block with its corners rounded; a shape on each line rounded
    /// every line by itself. Within a block, its lines of each kind.
    private static func hunks(_ lines: [LineDiff.Line]) -> [[[LineDiff.Line]]] {
        var out: [[[LineDiff.Line]]] = []
        for line in lines {
            if let last = out.last?.last?.last, (kind(last) == 0) == (kind(line) == 0) {
                if kind(last) == kind(line) {
                    out[out.count - 1][out[out.count - 1].count - 1].append(line)
                } else {
                    out[out.count - 1].append([line])
                }
            } else {
                out.append([[line]])
            }
        }
        return out
    }

    private static func kind(_ line: LineDiff.Line) -> Int {
        switch line {
        case .same: 0
        case .removed: 1
        case .added: 2
        }
    }

    private func block(_ hunk: [[LineDiff.Line]]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(hunk.enumerated()), id: \.offset) { _, run in
                self.run(run)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    private func run(_ run: [LineDiff.Line]) -> some View {
        let color: Color = switch run[0] {
        case .same: .clear
        case .removed: .red
        case .added: .green
        }
        let text = run.map { line in
            switch line {
            case let .same(t): "  \(t)"
            case let .removed(t): "− \(t)"
            case let .added(t): "+ \(t)"
            }
        }.joined(separator: "\n")
        return Text(text)
            .font(.system(.caption, design: .monospaced))
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            // Square: the block it is in rounds the corners, and a background
            // with no shape given comes out rounded on its own.
            .background(color.opacity(0.15), in: Rectangle())
    }
}

struct TodoList: View {
    let todos: JSONValue?

    var body: some View {
        if case let .array(items)? = todos {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, t in
                    let status = t["status"]?.string
                    Label {
                        Text(t["content"]?.string ?? "")
                            .strikethrough(status == "completed")
                            .foregroundStyle(status == "completed" ? .secondary : .primary)
                    } icon: {
                        Image(glyph: status == "completed" ? .passing
                              : status == "in_progress" ? .doing : .todo)
                            .foregroundStyle(status == "completed" ? .green : .secondary)
                    }
                    .font(.subheadline)
                }
            }
        }
    }
}
