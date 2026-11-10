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

/// The conversation's rows, a view of their own so that state the list
/// doesn't use (scroll position, the draft) leaves them alone.
struct ChatRows: View {
    let items: [ChatItem]
    let cwd: String
    let working: Bool
    /// The host says when a message is read; without it there are times
    /// and no receipts.
    var receipts = false

    var body: some View {
        ForEach(items) { item in
            row(item).id(item.id)
        }
    }

    @ViewBuilder
    private func row(_ item: ChatItem) -> some View {
        switch item {
        case let .prompt(_, prompt):
            HStack {
                Spacer(minLength: 40)
                VStack(alignment: .trailing, spacing: 4) {
                    ForEach(Array(prompt.pictures.enumerated()), id: \.offset) { _, picture in
                        PictureView(picture: picture, maxHeight: 200)
                    }
                    ForEach(prompt.files, id: \.self) { name in
                        Label(name, glyph: .file)
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if !prompt.text.isEmpty {
                        PromptBubble(text: prompt.text)
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
                        .foregroundStyle(.tertiary)
                }
            }
        case let .thinking(_, text):
            ThinkingView(text: text)
        case let .tool(_, call):
            ToolCard(call: call, cwd: cwd, working: working)
        case let .day(_, date):
            HStack {
                VStack { Divider() }
                Text(ChatFormat.day(date)).font(.caption).foregroundStyle(.secondary).fixedSize()
                VStack { Divider() }
            }
        case let .turnEnd(_, ms, at):
            Text([at.map { $0.formatted(date: .omitted, time: .shortened) }, "Worked for \(ChatFormat.duration(ms))"]
                .compactMap { $0 }.joined(separator: " · "))
                .font(.caption)
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity)
        case let .divider(_, reason):
            HStack {
                VStack { Divider() }
                Text(ChatFormat.dividerText(reason)).font(.caption).foregroundStyle(.secondary).fixedSize()
                VStack { Divider() }
            }
        case let .notice(_, text, isError):
            Text(text)
                .font(.caption)
                .foregroundStyle(isError ? Color.red.opacity(0.85) : .secondary)
                .textSelection(.enabled)
        }
    }
}

/// What you sent, as a message bubble: as Messages and the desktop draw it,
/// the accent, solid, and white on it. A long one shows its first lines,
/// fading out, with the rest a tap away, as the desktop's does (`Prompt` in
/// `apps/desktop/src/chat/Items.tsx`): a pasted log or a page of instructions
/// would otherwise fill the screen and push the conversation out of sight.
///
/// Long is measured rather than counted, since how many lines some text takes
/// depends on how wide the screen is, and is measured again when that changes.
struct PromptBubble: View {
    let text: String
    @State private var all = false
    /// How tall the text is in full, and cut at `longLines`, at the width it
    /// has. It is long when the two differ.
    @State private var full: CGFloat = 0
    @State private var upToLong: CGFloat = 0

    /// The desktop's numbers: five lines shown, and cut only when that hides
    /// at least two, since hiding one saves no more room than the button to
    /// show it takes.
    private static let shownLines = 5
    private static let longLines = shownLines + 2

    private var long: Bool { full > upToLong + 1 }
    private var cut: Bool { long && !all }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text)
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
        .padding(.vertical, 7)
        .foregroundStyle(.white)
        .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
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
            Text(text)
                .fixedSize(horizontal: false, vertical: true)
                .measuredHeight($full)
            Text(text)
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
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.snappy) { open.toggle() }
            } label: {
                Label("Thinking", systemImage: open ? "chevron.down" : "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            if open {
                Text(text)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
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
        .foregroundStyle(.tertiary)
    }
}
