import HuraKit
import SwiftUI

/// How the rows put a turn's length and a divider into words.
enum ChatFormat {
    static func duration(_ ms: Int64) -> String {
        let s = Int(ms / 1000)
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60)m \(s % 60)s" }
        return "\(s / 3600)h \(s % 3600 / 60)m"
    }

    static func dividerText(_ reason: String) -> String {
        switch reason {
        case "compact": "Conversation compacted"
        case "clear": "Conversation cleared"
        case "resume": "Resumed a conversation"
        case "fork": "Forked the conversation"
        default: reason
        }
    }

    /// A day, relative to today while that is shorter.
    static func day(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "Today" }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        let thisYear = calendar.isDate(date, equalTo: .now, toGranularity: .year)
        return date.formatted(thisYear ? .dateTime.weekday(.abbreviated).day().month(.abbreviated)
                                       : .dateTime.weekday(.abbreviated).day().month(.abbreviated).year())
    }
}

/// How long something has been going on, counted up once a second: a turn,
/// or a tool call. Nothing until `after` has passed, so the many calls that
/// take a moment don't flicker a "0s". The start is by the sandbox's clock and
/// now by the phone's; one behind the other reads as nothing, not a negative.
struct Elapsed: View {
    let since: Date
    var after: TimeInterval = 0

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let s = context.date.timeIntervalSince(since)
            if s >= after {
                Text(ChatFormat.duration(Int64(max(0, s) * 1000)))
                    .font(.caption.monospacedDigit())
                    // Read out once, not every second.
                    .accessibilityHidden(true)
            }
        }
    }
}

/// The conversation's rows, a view of their own so that state the list
/// doesn't use (scroll position, the draft) leaves them alone.
struct ChatRows: View {
    let items: [ChatItem]
    let cwd: String
    let working: Bool
    /// The host says when a message is read; without it there are times
    /// and no receipts.
    var receipts = false

    @Environment(\.foundRow) private var found

    var body: some View {
        let current = currentTurn
        let took = Self.took(items)
        ForEach(items) { item in
            let working = working && current.contains(item.id)
            if found?.id == item.id {
                row(item, working: working, took: took[item.id]).modifier(FoundMark(nonce: found?.nonce ?? 0)).id(item.id)
            } else {
                row(item, working: working, took: took[item.id]).id(item.id)
            }
        }
    }

    /// The rows from your last message on: the only ones that `working`
    /// changes. Given to every row, it drew the whole history again, and
    /// measured it again, each time a turn started or ended.
    private var currentTurn: Set<String> {
        let start = items.lastIndex { if case .prompt = $0 { true } else { false } } ?? items.startIndex
        return Set(items[start...].map(\.id))
    }

    /// How long each turn's work took, by the steps row it is folded into:
    /// Claude Code's own count, from the end of the turn, which comes after
    /// the answer the steps led to. As `took` in the desktop's `group`.
    static func took(_ items: [ChatItem]) -> [String: Int64] {
        var out: [String: Int64] = [:]
        for (i, item) in items.enumerated() {
            guard case let .steps(id, _, _) = item else { continue }
            var next = i + 1
            while next < items.count, case .text = items[next] { next += 1 }
            if next < items.count, case let .turnEnd(_, ms, _) = items[next] { out[id] = ms }
        }
        return out
    }

    @ViewBuilder
    private func row(_ item: ChatItem, working: Bool, took: Int64?) -> some View {
        switch item {
        case let .prompt(id, prompt):
            HStack {
                Spacer(minLength: 40)
                VStack(alignment: .trailing, spacing: 4) {
                    ForEach(Array(prompt.pictures.enumerated()), id: \.offset) { _, picture in
                        PictureView(picture: picture, maxHeight: 200)
                    }
                    ForEach(prompt.files, id: \.self) { name in
                        Label(name, glyph: .file)
                            .font(.caption).foregroundStyle(Color.dim)
                    }
                    if !prompt.text.isEmpty {
                        PromptBubble(text: prompt.text, seq: ChatItem.seq(of: id))
                    }
                    ReceiptLine(prompt: prompt, receipts: receipts, working: working)
                }
            }
        case let .text(_, text, at):
            VStack(alignment: .leading, spacing: 4) {
                MarkdownText(text: text)
                if let at {
                    Text(at, format: .dateTime.hour().minute())
                        .font(.caption2)
                        .foregroundStyle(Color.dimmer)
                }
            }
        case let .thinking(_, text):
            ThinkingView(text: text)
        case let .tool(_, call):
            ToolCard(call: call, cwd: cwd, working: working)
        case let .shell(_, run):
            ShellCard(run: run)
        case let .day(_, date):
            Rule(text: ChatFormat.day(date))
        case let .turnEnd(_, ms, at):
            // When the turn ended and what it took, small at the end of it:
            // its work, folded, already says "Worked for".
            Text([at.map { $0.formatted(date: .omitted, time: .shortened) }, ChatFormat.duration(ms)]
                .compactMap { $0 }.joined(separator: " · "))
                .font(.caption2.monospacedDigit())
                .foregroundStyle(Color.dimmer)
                .frame(maxWidth: .infinity, alignment: .trailing)
        case let .divider(_, reason):
            Rule(text: ChatFormat.dividerText(reason))
        case let .notice(_, text, isError):
            Text(text)
                .font(.caption)
                .foregroundStyle(isError ? Color.bad : Color.dimmer)
                .textSelection(.enabled)
        case let .steps(_, steps, live):
            StepsRow(items: steps, live: live, took: took, cwd: cwd, working: working)
        }
    }
}

/// A hairline across the column with a few words on it: a day, or the
/// conversation compacted.
private struct Rule: View {
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Rectangle().fill(Color.line).frame(height: 1)
            Text(text).font(.caption).foregroundStyle(Color.dimmer).fixedSize()
            Rectangle().fill(Color.line).frame(height: 1)
        }
    }
}

extension ChatItem {
    /// The entry a row was drawn from: its id starts with the entry's seq.
    static func seq(of id: String) -> UInt64? {
        id.split(separator: ".").first.flatMap { UInt64($0) }
    }
}

/// A turn's steps as T3 Code's work log: one line saying how long the work
/// took ("Worked for 4m 46s", or "Working for" and a clock while it runs),
/// and the steps under it as a line each. Shut until it is tapped, after which
/// it stays as it was put; while the turn runs, the step it is on shows under
/// the line, so the work can be watched without a wall of rows that snaps shut
/// when it ends. As the desktop's `StepsCard` in `chat/Items.tsx`.
struct StepsRow: View {
    let items: [ChatItem]
    let live: Bool
    /// How long the turn took, once it has ended.
    var took: Int64?
    let cwd: String
    let working: Bool
    @State private var open: Bool?
    @Environment(\.foundRow) private var found
    @Environment(\.turnStarted) private var started

    private var shown: Bool { open ?? false }
    private var calls: [ToolCall] {
        items.compactMap { if case let .tool(_, call) = $0 { call } else { nil } }
    }

    var body: some View {
        let failed = calls.filter { $0.failed && !$0.stopped }.count
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.snappy) { open = !shown }
            } label: {
                HStack(spacing: 4) {
                    head
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .rotationEffect(.degrees(shown ? 90 : 0))
                    if failed > 0 {
                        Text("\(failed) failed").font(.caption).foregroundStyle(Color.bad).padding(.leading, 4)
                    }
                    Spacer(minLength: 0)
                }
                .font(.subheadline.monospacedDigit())
                .foregroundStyle(Color.dim)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            // What the steps came to, which the line only says when it has
            // no time to say.
            .accessibilityHint(ChatItem.summary(items))
            // Opened for the row a search led to, each time it leads there.
            .task(id: found) {
                if let found, items.contains(where: { $0.id == found.id }) { open = true }
            }
            if shown {
                VStack(alignment: .leading, spacing: 2) {
                    // Erased, because the rows draw this row.
                    AnyView(ChatRows(items: items, cwd: cwd, working: working))
                }
            } else if live {
                Latest(items: items, cwd: cwd, working: working)
            }
        }
        .padding(.bottom, 8)
        .overlay(alignment: .bottom) { Rectangle().fill(Color.line).frame(height: 1) }
    }

    /// How long it took, or has taken so far; what it came to where the
    /// transcript does not say when the turn began.
    @ViewBuilder
    private var head: some View {
        if live, let started {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text("Working for \(ChatFormat.duration(Int64(max(0, context.date.timeIntervalSince(started)) * 1000)))")
            }
        } else if !live, let took {
            Text("Worked for \(ChatFormat.duration(took))")
        } else {
            Text(ChatItem.summary(items))
        }
    }
}

/// What a turn under way is doing, under its folded steps: the step it is
/// on, as its own row, and what the agent thought or said just before it, cut
/// to three lines. As the desktop's `Latest`.
private struct Latest: View {
    let items: [ChatItem]
    let cwd: String
    let working: Bool

    var body: some View {
        let last = items.last
        let said = last.flatMap(Self.words) ?? (items.count > 1 ? Self.words(items[items.count - 2]) : nil)
        let step = last.flatMap { Self.words($0) == nil ? $0 : nil }
        VStack(alignment: .leading, spacing: 2) {
            if let said, !said.isEmpty {
                Text(said)
                    .font(.subheadline)
                    .foregroundStyle(Color.dim)
                    .lineLimit(3)
            }
            if let step {
                // A view of its own for each step, rather than this one's
                // open or shut state handed on to the next.
                AnyView(ChatRows(items: [step], cwd: cwd, working: working)).id(step.id)
            }
        }
    }

    private static func words(_ item: ChatItem) -> String? {
        switch item {
        case let .text(_, text, _), let .thinking(_, text): plain(text)
        default: nil
        }
    }

    /// Markdown as one run of plain words, for a preview a few lines long:
    /// its fences, headings, quotes, list marks, bold and code ticks taken off.
    static func plain(_ text: String) -> String {
        text.replacing(/```[^\n]*\n?/, with: "")
            .replacing(/(?m)^\s*(#{1,6}\s+|>\s?|[-*+]\s+|\d+\.\s+)/, with: "")
            .replacing("**", with: "")
            .replacing("`", with: "")
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }
}

extension EnvironmentValues {
    /// When the turn under way began, for its work log's clock.
    @Entry var turnStarted: Date?
    /// Edit a message you sent, by its entry's seq: "Edit from Here". Unset
    /// where a conversation cannot fork, such as a side chat.
    @Entry var editPrompt: EditPrompt?
    /// The commands the agent left running in the background, so the Bash
    /// call that started one can say how it is doing and open its output.
    /// Unset where the sandbox's files can't be read.
    @Entry var backgroundCommands: BackgroundCommands?
}

/// The background commands, as the environment holds them, and the way to
/// open one's output. Equal when the commands are: the closure is a new one
/// each time the conversation is drawn, while what it does is the same.
struct BackgroundCommands: Equatable {
    let all: [BackgroundCommand]
    let open: (BackgroundCommand) -> Void

    /// The command a Bash call started, by the call's id.
    func started(by call: String) -> BackgroundCommand? {
        all.first { $0.id == call }
    }

    static func == (a: BackgroundCommands, b: BackgroundCommands) -> Bool { a.all == b.all }
}

/// "Edit from Here", as the environment holds it. Always equal to another:
/// a closure is a new one each time the conversation is drawn, and every
/// message you sent was drawn again with it, while what it does is the same.
struct EditPrompt: Equatable {
    let edit: (UInt64, String) -> Void

    func callAsFunction(_ seq: UInt64, _ text: String) { edit(seq, text) }
    static func == (_: EditPrompt, _: EditPrompt) -> Bool { true }
}

/// The row a search led to, and which time it led there: see `ChatItem.locate`.
struct FoundRow: Equatable {
    var id: String
    var nonce: Int
}

extension EnvironmentValues {
    @Entry var foundRow: FoundRow?
}

/// The row a search led to, lit for a moment so the eye lands on it.
private struct FoundMark: ViewModifier {
    let nonce: Int
    @State private var lit = true

    func body(content: Content) -> some View {
        content
            .padding(6)
            .background(Color.accentColor.opacity(lit ? 0.22 : 0), in: RoundedRectangle(cornerRadius: 8))
            .padding(-6)
            .task(id: nonce) {
                lit = true
                try? await Task.sleep(for: .seconds(1))
                withAnimation(.easeOut(duration: 1.4)) { lit = false }
            }
    }
}

extension ChatRows {
    /// Your message, with a slash command it starts with set apart as code,
    /// on a patch of the bubble a step up, as the desktop draws it.
    static func bubble(_ text: String) -> AttributedString {
        guard let lead = ComposerInput.leadingCommand(text) else { return AttributedString(text) }
        var command = AttributedString(lead.command)
        command.font = Font.body.monospaced().weight(.semibold)
        command.backgroundColor = Color.selected
        return command + AttributedString(lead.rest)
    }
}

/// What you sent, in T3 Code's quiet bubble: a step up from the canvas, in
/// the text's own colour, rather than the accent, which is kept for what can
/// be done. A long press offers to copy it or to edit it, which forks the
/// conversation from just before it. A long one shows its first lines,
/// fading out, with the rest a tap away, as the desktop's does (`Prompt` in
/// `apps/desktop/src/chat/Items.tsx`): a pasted log or a page of instructions
/// would otherwise fill the screen and push the conversation out of sight.
///
/// Long is measured rather than counted, since how many lines some text takes
/// depends on how wide the screen is, and is measured again when that changes.
struct PromptBubble: View {
    let text: String
    /// The entry it is, for "Edit from Here".
    var seq: UInt64?
    @State private var all = false
    @Environment(\.editPrompt) private var edit
    /// How tall the text is in full, and cut at `longLines`, at the width it
    /// has. It is long when the two differ.
    @State private var full: CGFloat = 0
    @State private var upToLong: CGFloat = 0

    /// The desktop's numbers: five lines shown, and cut only when that hides
    /// at least two, since hiding one saves no more room than the button to
    /// show it takes.
    private static let shownLines = 5
    private static let longLines = shownLines + 2

    /// With a slash command it starts with set apart.
    private var words: AttributedString { ChatRows.bubble(text) }

    private var long: Bool { full > upToLong + 1 }
    private var cut: Bool { long && !all }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(words)
                .lineLimit(cut ? Self.shownLines : nil)
                .textSelection(.enabled)
                .mask { fade }
                .background(alignment: .top) { measures }
            if long {
                Button(all ? "Show less" : "Show more") {
                    withAnimation(.snappy) { all.toggle() }
                }
                .buttonStyle(.plain)
                .font(.footnote.weight(.semibold))
                .opacity(0.85)
            }
        }
        .padding(.horizontal, 13)
        .padding(.vertical, 8)
        .foregroundStyle(Color.ink)
        .background(Color.surface2, in: RoundedRectangle(cornerRadius: Radius.composer, style: .continuous))
        .contentShape(.contextMenuPreview, RoundedRectangle(cornerRadius: Radius.composer, style: .continuous))
        // A long press, as Messages has it: the desktop's actions on hover.
        .contextMenu {
            Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = text }
            if let edit, let seq {
                Button("Edit from Here", glyph: .edit) { edit(seq, text) }
            }
        }
    }

    /// The last line fading into the bubble while it is cut, rather than
    /// stopping hard at the ellipsis.
    @ViewBuilder
    private var fade: some View {
        if cut {
            VStack(spacing: 0) {
                Color.black
                LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom)
                    .frame(height: 24)
            }
        } else {
            Color.black
        }
    }

    /// Two copies of the text that nobody sees, wrapped at the width the shown
    /// one has, to be measured: one in full and one cut at `longLines`.
    private var measures: some View {
        ZStack(alignment: .top) {
            Text(words)
                .fixedSize(horizontal: false, vertical: true)
                .measuredHeight($full)
            Text(words)
                .lineLimit(Self.longLines)
                .fixedSize(horizontal: false, vertical: true)
                .measuredHeight($upToLong)
        }
        .hidden()
        .accessibilityHidden(true)
    }
}

private extension View {
    /// Its height, kept in `height` as it changes.
    func measuredHeight(_ height: Binding<CGFloat>) -> some View {
        background {
            GeometryReader { g in
                Color.clear.onChange(of: g.size.height, initial: true) { _, h in height.wrappedValue = h }
            }
        }
    }
}

struct ThinkingView: View {
    let text: String
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.snappy) { open.toggle() }
            } label: {
                // The whole width of the row and some height around the words
                // take the tap: the caption alone was hard to hit.
                // A line of the work log, as a tool call is.
                HStack(spacing: 8) {
                    Image(glyph: .effort).frame(width: 18)
                    Text("Thinking")
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(Color.dimmer)
                        .rotationEffect(.degrees(open ? 90 : 0))
                    Spacer(minLength: 0)
                }
                .font(.subheadline)
                .foregroundStyle(Color.dim)
                .padding(.vertical, 5)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                Text(text)
                    .font(.footnote)
                    .foregroundStyle(Color.dim)
                    .textSelection(.enabled)
                    .padding(.leading, 26)
                    .padding(.bottom, 6)
            }
        }
    }
}

/// Under your message: when it was sent, and what has become of it: in the
/// sandbox, waiting behind the turn in progress, or read. One tick for sent and
/// two for read, as the desktop draws them; waiting behind a turn is the one
/// tick too, and "Queued" beside it said "not read yet" a second time, so it
/// is only what VoiceOver reads.
struct ReceiptLine: View {
    let prompt: Prompt
    let receipts: Bool
    let working: Bool

    var body: some View {
        HStack(spacing: 4) {
            if let sent = prompt.sent { Text(sent, format: .dateTime.hour().minute()) }
            if receipts {
                if prompt.read != nil {
                    Image(glyph: .read).accessibilityLabel("Read")
                } else {
                    Image(glyph: .sent).accessibilityLabel(working ? "Queued" : "Sent")
                }
            }
        }
        .font(.caption2)
        // One grey whatever the state: the ticks say which. A time that
        // brightened on being read made a queued message's look dimmed.
        .foregroundStyle(Color.dimmer)
    }
}
