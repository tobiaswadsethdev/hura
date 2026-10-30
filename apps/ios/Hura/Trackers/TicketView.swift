import HuraKit
import SwiftUI

/// One ticket in full: its status (which can be moved), its fields (which
/// can be edited), its description and its comments (yours can be edited or
/// deleted), and a start into a worktree. A Jira ticket is read here; the
/// others open in the browser, as on the desktop.
struct TicketView: View {
    let task: TrackerTask
    @Binding var starting: WorkOrigin?
    @Environment(TrackersModel.self) private var trackers
    @Environment(\.openURL) private var openURL
    @State private var issue: TrackerIssue?
    @State private var transitions: [IssueTransition] = []
    @State private var error: String?
    @State private var failed: String?
    @State private var busy: String?
    @State private var draft = ""
    @State private var editing = false
    @State private var rewriting: IssueComment?
    @State private var deleting: IssueComment?

    private var stored: StoredTracker? { trackers.tracker(named: task.tracker) }
    private var readsHere: Bool { task.kind == "jira" }

    var body: some View {
        Group {
            if readsHere {
                Loaded(value: issue, error: error) { issue in content(issue) }
            } else {
                List {
                    header
                    Section {
                        Button("Open in the Browser", systemImage: "safari") { openURL(URL(string: task.url)!) }
                    } footer: {
                        Text("A \(TrackerSource(kind: task.kind).kindLabel) ticket is read in the browser, as on the desktop.")
                    }
                }
            }
        }
        .navigationTitle(task.key)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Start a Worktree", systemImage: "play") { starting = .ticket(task) }
                    if readsHere {
                        Button("Edit Fields", systemImage: "pencil") { editing = true }
                    }
                    Button("Open in the Browser", systemImage: "safari") { openURL(URL(string: task.url)!) }
                    Button("Copy Key", systemImage: "doc.on.doc") { UIPasteboard.general.string = task.key }
                    Button("Copy Link", systemImage: "link") { UIPasteboard.general.string = task.url }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .task { if readsHere { await load() } }
        .refreshable { await load() }
        .sheet(isPresented: $editing, onDismiss: { Task { await load() } }) {
            if let stored { TicketEditView(tracker: stored, key: task.key) }
        }
        .sheet(item: $rewriting) { c in
            CommentEditor(title: "Edit Comment", text: c.markdown?.text ?? "", lost: c.markdown?.lost ?? []) { text in
                change("comment:\(c.id)") { try await TrackerCore.editComment($0, key: task.key, id: c.id, markdown: text) }
            }
        }
        .confirmationDialog("Delete your comment?", isPresented: .constant(deleting != nil), titleVisibility: .visible,
                            presenting: deleting) { c in
            Button("Delete", role: .destructive) {
                deleting = nil
                change("comment:\(c.id)") { try await TrackerCore.deleteComment($0, key: task.key, id: c.id) }
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        }
        .failure($failed)
    }

    private var header: some View {
        Section {
            Text(task.title).font(.headline)
            LabeledContent("Status", value: task.status)
            LabeledContent("Type", value: task.itemType)
            LabeledContent("Tracker", value: task.tracker)
        }
    }

    private func content(_ issue: TrackerIssue) -> some View {
        List {
            Section {
                Text(issue.title).font(.headline).textSelection(.enabled)
                LabeledContent("Status") {
                    Menu {
                        ForEach(transitions) { t in
                            Button(t.name == t.to ? t.to : "\(t.name) → \(t.to)") {
                                change("status") { try await TrackerCore.transition($0, key: task.key, id: t.id) }
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Text(issue.status)
                            if busy == "status" { ProgressView() } else { Image(systemName: "chevron.up.chevron.down").font(.caption) }
                        }
                    }
                    .disabled(transitions.isEmpty)
                }
                LabeledContent("Type", value: issue.itemType)
                if let p = issue.priority { LabeledContent("Priority", value: p) }
                LabeledContent("Assignee", value: issue.assignee.map(person) ?? "Unassigned")
                if let r = issue.reporter { LabeledContent("Reporter", value: person(r)) }
                if !issue.labels.isEmpty { LabeledContent("Labels", value: issue.labels.joined(separator: ", ")) }
                if let parent = issue.parent { LabeledContent("Parent", value: "\(parent.key) \(parent.title)") }
                if let due = issue.due { LabeledContent("Due", value: due) }
            }
            Section("Description") {
                DocView(blocks: issue.description)
            }
            Section {
                ForEach(issue.comments) { c in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(c.author.map(person) ?? "Someone").font(.caption.weight(.semibold))
                            Text(when(c.created)).font(.caption).foregroundStyle(.secondary)
                            if c.edited != nil { Text("· edited").font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            if busy == "comment:\(c.id)" { ProgressView() }
                        }
                        DocView(blocks: c.body)
                    }
                    .swipeActions {
                        if c.author?.me == true {
                            Button("Delete", role: .destructive) { deleting = c }
                            Button("Edit") { rewriting = c }
                        }
                    }
                }
                VStack(alignment: .leading) {
                    TextField("Add a comment (Markdown)", text: $draft, axis: .vertical).lineLimit(2...8)
                    HStack {
                        Spacer()
                        Button("Comment") {
                            let text = draft
                            change("comment") {
                                try await TrackerCore.comment($0, key: task.key, markdown: text)
                                await MainActor.run { draft = "" }
                            }
                        }
                        .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy == "comment")
                    }
                }
            } header: {
                Text(issue.commentsTotal > issue.comments.count
                     ? "Comments · latest \(issue.comments.count) of \(issue.commentsTotal)"
                     : "Comments · \(issue.comments.count)")
            }
        }
    }

    private func person(_ p: IssuePerson) -> String { p.me ? "\(p.name) (you)" : p.name }

    private func when(_ iso: String?) -> String {
        guard let iso, let d = ISO8601DateFormatter.lenient(iso) else { return iso ?? "" }
        return d.formatted(.relative(presentation: .named))
    }

    private func load() async {
        guard let stored else {
            error = "No tracker called \(task.tracker) on this phone."
            return
        }
        do {
            async let t = try? TrackerCore.transitions(stored, key: task.key)
            issue = try await TrackerCore.issue(stored, key: task.key)
            transitions = await t ?? []
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// One change, then the ticket read again: the answer to a change is
    /// whether it worked, and the ticket says what it now is.
    private func change(_ key: String, _ work: @escaping (StoredTracker) async throws -> Void) {
        guard let stored else { return }
        busy = key
        Task {
            defer { busy = nil }
            do {
                try await work(stored)
                await load()
                await trackers.refresh()
            } catch {
                failed = error.localizedDescription
            }
        }
    }
}

extension ISO8601DateFormatter {
    /// Trackers write their times with and without fractions of a second.
    static func lenient(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: s) { return d }
        f.formatOptions = [.withInternetDateTime]
        if let d = f.date(from: s) { return d }
        // Jira: 2026-10-30T21:00:00.000+0100
        let j = DateFormatter()
        j.locale = Locale(identifier: "en_US_POSIX")
        j.dateFormat = "yyyy-MM-dd'T'HH:mm:ss.SSSZ"
        return j.date(from: s)
    }
}

/// Writing a comment in Markdown, and what converting it would lose.
struct CommentEditor: View {
    let title: String
    @State var text: String
    let lost: [String]
    let save: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    init(title: String, text: String, lost: [String], save: @escaping (String) -> Void) {
        self.title = title
        _text = State(initialValue: text)
        self.lost = lost
        self.save = save
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Markdown", text: $text, axis: .vertical).lineLimit(5...20)
                if !lost.isEmpty {
                    Section {
                        Text("Saving loses what Markdown can't say: \(lost.joined(separator: ", ")).")
                            .font(.caption).foregroundStyle(.orange)
                    }
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        save(text)
                        dismiss()
                    }
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

/// hura-core's reading of a Jira document: paragraphs, headings, lists, code,
/// quotes, panels, tables. Drawn plainly; what has no form here says what it
/// was.
struct DocView: View {
    let blocks: JSONValue

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if case let .array(list) = blocks, !list.isEmpty {
                ForEach(Array(list.enumerated()), id: \.offset) { _, b in block(b) }
            } else {
                Text("Nothing written").foregroundStyle(.secondary)
            }
        }
    }

    private func block(_ b: JSONValue) -> AnyView {
        switch b["type"]?.string {
        case "paragraph":
            return AnyView(inline(b["content"]).font(.callout))
        case "heading":
            let level = number(b["level"]) ?? 2
            return AnyView(inline(b["content"]).font(level <= 2 ? .headline : .subheadline.weight(.semibold)))
        case "list":
            let ordered = b["ordered"]?.bool ?? false
            let start = number(b["start"]) ?? 1
            guard case let .array(items)? = b["items"] else { return AnyView(EmptyView()) }
            return AnyView(VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, item in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if let checked = item["checked"]?.bool {
                            Image(systemName: checked ? "checkmark.square" : "square")
                        } else {
                            Text(ordered ? "\(start + i)." : "•")
                        }
                        DocView(blocks: item["content"] ?? .array([]))
                    }
                }
            })
        case "code":
            return AnyView(Text(b["text"]?.string ?? "")
                .font(.caption.monospaced())
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6)))
        case "quote", "panel":
            return AnyView(HStack(spacing: 8) {
                Rectangle().fill(Color.secondary.opacity(0.4)).frame(width: 3)
                DocView(blocks: b["content"] ?? .array([]))
            })
        case "expand":
            return AnyView(DisclosureGroup(b["title"]?.string ?? "More") {
                DocView(blocks: b["content"] ?? .array([]))
            })
        case "rule":
            return AnyView(Divider())
        case "table":
            guard case let .array(rows)? = b["rows"] else { return AnyView(EmptyView()) }
            return AnyView(VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    if case let .array(cells) = row {
                        HStack(alignment: .top) {
                            ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                                DocView(blocks: cell["content"] ?? .array([]))
                                    .fontWeight(cell["header"]?.bool == true ? .semibold : .regular)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        Divider()
                    }
                }
            })
        case "attachment":
            return AnyView(Label(b["name"]?.string ?? "attachment", systemImage: "paperclip").font(.caption))
        case "card":
            let url = b["url"]?.string ?? ""
            return AnyView(Link(url, destination: URL(string: url) ?? URL(string: "about:blank")!).font(.caption))
        default:
            return AnyView(EmptyView())
        }
    }

    private func number(_ v: JSONValue?) -> Int? {
        if case let .number(n)? = v { Int(n) } else { nil }
    }

    /// Inline runs as one Text, with what each carries.
    private func inline(_ content: JSONValue?) -> Text {
        guard case let .array(runs)? = content else { return Text("") }
        var out = AttributedString()
        for r in runs {
            var piece: AttributedString
            switch r["type"]?.string {
            case "text":
                piece = AttributedString(r["text"]?.string ?? "")
                if r["bold"]?.bool == true { piece.inlinePresentationIntent = .stronglyEmphasized }
                if r["italic"]?.bool == true { piece.inlinePresentationIntent = (piece.inlinePresentationIntent ?? []).union(.emphasized) }
                if r["code"]?.bool == true { piece.inlinePresentationIntent = (piece.inlinePresentationIntent ?? []).union(.code) }
                if r["strike"]?.bool == true { piece.strikethroughStyle = .single }
                if r["underline"]?.bool == true { piece.underlineStyle = .single }
                if let href = r["href"]?.string, let url = URL(string: href) { piece.link = url }
            case "mention": piece = AttributedString("@\(r["name"]?.string ?? "")")
            case "emoji", "status": piece = AttributedString(r["text"]?.string ?? "")
            case "date":
                if case let .number(at)? = r["at"] {
                    piece = AttributedString(Date(timeIntervalSince1970: at).formatted(date: .abbreviated, time: .omitted))
                } else {
                    piece = AttributedString("")
                }
            case "break": piece = AttributedString("\n")
            default: piece = AttributedString("")
            }
            out += piece
        }
        return Text(out)
    }
}
