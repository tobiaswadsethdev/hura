import HuraKit
import SwiftUI

/// Every conversation this phone has seen, searched: what was said, the
/// tools used and the files they touched, as the desktop's search screen
/// (`apps/desktop/src/Search.tsx`). What is searched is kept on the phone as
/// it arrives -- see `ChatHistory` -- so it includes conversations that have
/// ended and sessions that have been deleted.
///
/// A result in a conversation that still exists opens it at the message, by
/// the route a notification takes. One that only this phone still has is
/// read here instead, from what was kept.
struct SearchView: View {
    @Environment(AppModel.self) private var model
    @State private var query = ""
    @State private var found = ChatHistory.Found()
    @State private var searched = ""
    @State private var path = NavigationPath()

    init() {
        #if DEBUG
        // What to search for at launch: see `pairFromLaunchArguments`.
        _query = State(initialValue: UserDefaults.standard.string(forKey: "HuraSearch") ?? "")
        #endif
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                ForEach(found.hits) { hit in
                    Button { open(hit) } label: { HitRow(hit: hit, server: serverName(hit), words: words) }
                        .buttonStyle(.plain)
                }
                if found.more {
                    Text("Only the newest \(found.hits.count) are shown; say more to find fewer.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
            .listStyle(.plain)
            .overlay {
                if query.trimmingCharacters(in: .whitespaces).isEmpty {
                    ContentUnavailableView {
                        Label("Search every conversation", glyph: .search)
                    } description: {
                        Text("Words are looked for in what was said. `file:auth.rs` finds the tool calls that touched a file, `tool:Bash` those of one tool, and \"two words\" a phrase.")
                    }
                } else if found.hits.isEmpty, searched == query {
                    ContentUnavailableView.search(text: query)
                }
            }
            .navigationTitle("Search")
            .searchable(text: $query, prompt: "Words, file:name, tool:Bash")
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            // As it is typed, a moment after the last key.
            .task(id: query) {
                try? await Task.sleep(for: .milliseconds(150))
                guard !Task.isCancelled else { return }
                let q = query
                found = await model.history.search(q)
                searched = q
            }
            .navigationDestination(for: ChatHistory.Hit.self) { KeptChatView(hit: $0) }
        }
    }

    /// The words looked for, for marking them: a query less its `file:` and
    /// `tool:`, phrases whole.
    private var words: [String] {
        query.matches(of: /"([^"]*)"?|(\S+)/).compactMap { m in
            let w = String(m.output.1 ?? m.output.2 ?? "").lowercased()
            return w.isEmpty || w.hasPrefix("file:") || w.hasPrefix("tool:") ? nil : w
        }
    }

    private func serverName(_ hit: ChatHistory.Hit) -> String? {
        guard model.servers.count > 1 else { return nil }
        return model.server(hit.serverID)?.record.name ?? "A server no longer paired"
    }

    /// Whether a result can be opened where it happened: its server still
    /// paired, and the session in its list unless the list is not to hand.
    private func live(_ hit: ChatHistory.Hit) -> Bool {
        guard !hit.removed, !hit.ended, let server = model.server(hit.serverID) else { return false }
        return server.state != .online || server.session(named: hit.session) != nil
    }

    private func open(_ hit: ChatHistory.Hit) {
        guard live(hit) else {
            path.append(hit)
            return
        }
        model.pendingJump = AppModel.Jump(serverID: hit.serverID, session: hit.session, conv: hit.conv,
                                          seq: hit.seq, nonce: Int(Date().timeIntervalSince1970 * 1000) % Int(Int32.max))
        model.pendingRoute = [.server(hit.serverID), .session(serverID: hit.serverID, name: hit.session)]
    }
}

/// One result: where it was, who said it, and what it touched.
private struct HitRow: View {
    let hit: ChatHistory.Hit
    let server: String?
    let words: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(hit.title ?? hit.session).font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(hit.conv == "agent" ? "Agent" : hit.conv).foregroundStyle(.secondary)
                if let project = hit.project { Text(project).foregroundStyle(.secondary).lineLimit(1) }
                if let server { Text(server).foregroundStyle(.secondary).lineLimit(1) }
                if hit.removed {
                    Text("deleted").foregroundStyle(.orange)
                } else if hit.ended {
                    Text("ended").foregroundStyle(.orange)
                }
                Spacer(minLength: 4)
                Text(when).foregroundStyle(.tertiary).monospacedDigit()
            }
            .font(.caption)
            (Text(role).foregroundStyle(hit.role == .you ? Color.accentColor : .secondary) + Text("  ") + Text(marked))
                .font(.callout)
                .lineLimit(4)
            if !hit.tools.isEmpty || !hit.files.isEmpty {
                Text((hit.tools + hit.files.map(Self.relative)).joined(separator: " · "))
                    .font(.caption2.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }

    private var role: String {
        switch hit.role {
        case .you: "You"
        case .claude: "Claude"
        case .agent: "Subagent"
        case .shell: "!"
        case .error: "Error"
        }
    }

    private var when: String {
        Calendar.current.isDateInToday(hit.at)
            ? hit.at.formatted(date: .omitted, time: .shortened)
            : hit.at.formatted(.dateTime.day().month(.abbreviated))
    }

    /// The snippet with the words looked for lit.
    private var marked: AttributedString {
        var text = AttributedString(hit.snippet)
        for w in words {
            var from = text.startIndex
            while let r = text[from...].range(of: w, options: .caseInsensitive) {
                text[r].backgroundColor = .yellow.opacity(0.3)
                from = r.upperBound
            }
        }
        return text
    }

    static func relative(_ path: String) -> String {
        path.hasPrefix("/sandbox/repo/") ? String(path.dropFirst("/sandbox/repo/".count)) : path
    }
}

/// A conversation that exists only on this phone, read from what was kept,
/// at the message found. Its pictures were not kept.
struct KeptChatView: View {
    let hit: ChatHistory.Hit
    @Environment(AppModel.self) private var model
    @State private var items: [ChatItem]?

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    Text(hit.removed ? "The session has been deleted; this is what this phone kept of it."
                                     : "The conversation has ended; this is what this phone kept of it.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let items {
                        if items.isEmpty {
                            ContentUnavailableView("Nothing was kept of it", image: Glyph.chat.name)
                        }
                        ChatRows(items: items, cwd: "/sandbox/repo", working: false)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .environment(\.foundRow, located.map { FoundRow(id: $0.id, nonce: Int(hit.seq)) })
            }
            .overlay { if items == nil { ProgressView() } }
            .task(id: hit) {
                let raw = await model.history.transcript(serverID: hit.serverID, sessionKey: hit.sessionKey,
                                                         convKey: hit.convKey)
                items = ChatItem.group(ChatItem.fold(raw.compactMap(ChatTranscript.entry)).items, working: false)
                if let located { ChatView.reveal(located.id, in: proxy) }
            }
        }
        .background(Color.black)
        .environment(\.colorScheme, .dark)
        .navigationTitle(hit.title ?? hit.session)
        .navigationBarTitleDisplayMode(.inline)
    }

    private var located: ChatItem.Located? {
        items.flatMap { ChatItem.locate($0, seq: hit.seq) }
    }
}
