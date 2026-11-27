import HuraKit
import SwiftUI

/// The drawer on the composer's top edge, as T3 Code has it: what fits what
/// is being typed (a slash command, a file to mention) or the prompts put
/// aside, one list at a time, on the card itself rather than floating over
/// the conversation.
struct ComposerDrawer<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) { content }
                    .padding(.vertical, 4)
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(maxHeight: 220)
            .fixedSize(horizontal: false, vertical: true)
            Rectangle().fill(Color.line).frame(height: 1)
        }
    }
}

/// One row of the drawer: a glyph, the thing, and a line about it.
private struct DrawerRow<Trailing: View>: View {
    var glyph: Glyph?
    let title: Text
    var detail: String?
    var mono = false
    let pick: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button(action: pick) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    if let glyph {
                        Image(glyph: glyph).foregroundStyle(Color.dim).frame(width: 18)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        title
                            .font(mono ? .subheadline.monospaced() : .subheadline)
                            .foregroundStyle(Color.ink)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if let detail, !detail.isEmpty {
                            Text(detail).font(.caption).foregroundStyle(Color.dim).lineLimit(2)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            trailing
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }
}

/// The slash commands that fit what is typed: each one's name and what it
/// takes, then what it does. A tap completes the name.
struct CommandSuggestions: View {
    let commands: [SlashCommand]
    let pick: (SlashCommand) -> Void

    var body: some View {
        ForEach(commands) { c in
            DrawerRow(title: Text("/\(c.name)") + Text(c.argumentHint.isEmpty ? "" : "  \(c.argumentHint)")
                .foregroundColor(Color.dimmer),
                      detail: about(c), mono: true) { pick(c) } trailing: { EmptyView() }
        }
    }

    private func about(_ c: SlashCommand) -> String {
        guard !c.aliases.isEmpty else { return c.description }
        let also = "also " + c.aliases.map { "/\($0)" }.joined(separator: ", ")
        return c.description.isEmpty ? also : "\(c.description) (\(also))"
    }
}

/// What the server found for an `@`: paths in the working copy, folders
/// ending in `/`, best first. A tap puts `@path` in the message.
struct MentionSuggestions: View {
    let paths: [String]
    let error: String?
    let searching: Bool
    let pick: (String) -> Void

    var body: some View {
        if let error {
            note(error, color: .bad)
        } else if paths.isEmpty {
            note(searching ? "Looking…" : "No files match", color: .dim)
        } else {
            ForEach(paths, id: \.self) { path in
                let folder = path.hasSuffix("/")
                let trimmed = folder ? String(path.dropLast()) : path
                let name = (trimmed as NSString).lastPathComponent
                let parent = (trimmed as NSString).deletingLastPathComponent
                DrawerRow(glyph: folder ? .folder : .file,
                          title: Text(name + (folder ? "/" : "")) + Text(parent.isEmpty ? "" : "  \(parent)")
                              .foregroundColor(Color.dimmer)) {
                    pick(path)
                } trailing: { EmptyView() }
            }
        }
    }

    private func note(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(color)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
    }
}

/// The prompts put aside, newest first: a tap takes one back into the box,
/// and the bin throws one away.
struct StashList: View {
    let stashed: [Stashed]
    let take: (Stashed) -> Void
    let delete: (Stashed) -> Void

    var body: some View {
        ForEach(stashed) { s in
            DrawerRow(title: Text(s.text.split(whereSeparator: \.isNewline).first.map(String.init) ?? s.text),
                      detail: s.at.formatted(.relative(presentation: .named))) {
                take(s)
            } trailing: {
                Button { delete(s) } label: {
                    Image(systemName: "trash").font(.footnote).foregroundStyle(Color.dimmer)
                        .frame(width: 30, height: 30)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel("Delete")
            }
        }
    }
}

/// A bar on the composer saying what sending will do when it is not the
/// usual: an earlier message being edited, which forks the conversation.
struct ComposerBar: View {
    let glyph: Glyph
    let text: String
    let cancel: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(glyph: glyph).foregroundStyle(Color.dim)
            Text(text).font(.caption).foregroundStyle(Color.dim).lineLimit(2)
            Spacer(minLength: 0)
            Button(action: cancel) {
                Image(systemName: "xmark").font(.caption.weight(.semibold)).foregroundStyle(Color.dim)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel")
        }
        .padding(.leading, 14)
        .padding(.trailing, 6)
        .padding(.vertical, 4)
        .background(Color.surface2.opacity(0.6))
    }
}

/// A message waiting on the server to be said, on your side at the end of
/// the conversation as T3 Code draws it: your bubble, dashed, with when it
/// will be said (or the rule it comes round by), Edit and Remove.
struct WaitingBubble: View {
    let message: QueuedMessage
    /// The server's offset from UTC, for a repeat's time of day.
    var utcOffset: Int?
    var edit: (() -> Void)?
    var remove: (() -> Void)?

    var body: some View {
        HStack {
            Spacer(minLength: 40)
            VStack(alignment: .trailing, spacing: 4) {
                Text(message.text)
                    .lineLimit(6)
                    .foregroundStyle(Color.ink)
                    .padding(.horizontal, 13)
                    .padding(.vertical, 8)
                    .overlay(RoundedRectangle(cornerRadius: Radius.composer, style: .continuous)
                        .strokeBorder(Color.lineStrong, style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                HStack(spacing: 10) {
                    Label(when, systemImage: message.repeat != nil ? "repeat" : message.at == nil ? "hourglass" : "clock")
                        .labelStyle(.titleAndIcon)
                        .foregroundStyle(Color.dimmer)
                    if let edit {
                        Button("Edit", action: edit).foregroundStyle(Color.dim)
                    }
                    if let remove {
                        Button("Remove", role: .destructive, action: remove).foregroundStyle(Color.dim)
                    }
                }
                .font(.caption2)
                .buttonStyle(.plain)
            }
        }
        .accessibilityElement(children: .contain)
    }

    /// When it will be said, in words.
    private var when: String {
        if let rule = message.repeat {
            return rule.describe(serverOffset: utcOffset ?? TimeZone.current.secondsFromGMT())
        }
        guard let at = message.at else { return "After this turn" }
        return Calendar.current.isDateInToday(at)
            ? at.formatted(date: .omitted, time: .shortened)
            : at.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated).hour().minute())
    }
}

/// The side command, T3 Code's side chat as one of the composer's own: a
/// question asked beside the conversation, in a fork of it, that does not
/// interrupt it. As `SIDE_COMMAND` in the desktop's `panes/Chat.tsx`.
let sideCommand = SlashCommand(name: "btw",
                               description: "Ask on the side, with this conversation's context, without interrupting it",
                               argumentHint: "<question>", aliases: ["side"])
