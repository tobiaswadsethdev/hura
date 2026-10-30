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
}

/// The conversation's rows, a view of their own so that state the list
/// doesn't use (scroll position, the draft) leaves them alone.
struct ChatRows: View {
    let items: [ChatItem]
    let cwd: String
    let working: Bool

    var body: some View {
        ForEach(items) { item in
            row(item).id(item.id)
        }
    }

    @ViewBuilder
    private func row(_ item: ChatItem) -> some View {
        switch item {
        case let .prompt(_, text, images):
            HStack {
                Spacer(minLength: 40)
                VStack(alignment: .trailing, spacing: 4) {
                    if images > 0 {
                        Label("\(images) image\(images == 1 ? "" : "s")", systemImage: "photo")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if !text.isEmpty {
                        Text(text)
                            .textSelection(.enabled)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .background(Color.accentColor.opacity(0.35), in: RoundedRectangle(cornerRadius: 16))
                    }
                }
            }
        case let .text(_, text):
            MarkdownText(text: text)
        case let .thinking(_, text):
            ThinkingView(text: text)
        case let .tool(_, call):
            ToolCard(call: call, cwd: cwd, working: working)
        case let .turnEnd(_, ms):
            Text("Worked for \(ChatFormat.duration(ms))")
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
