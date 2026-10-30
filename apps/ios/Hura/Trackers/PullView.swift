import HuraKit
import SwiftUI

/// A pull request in full: what it is, its checks and reviewers, its
/// conversation (with replies), a review to give, and its files. Read with
/// the tracker's own token, as the desktop reads it.
struct PullView: View {
    let pull: PullRequestSummary
    @Binding var starting: WorkOrigin?
    @Environment(TrackersModel.self) private var trackers
    @Environment(\.openURL) private var openURL
    @State private var detail: PullDetail?
    @State private var error: String?
    @State private var failed: String?
    @State private var busy: String?
    @State private var tab = 0
    @State private var draft = ""
    @State private var replying: PullThread?
    @State private var voting = false

    private var stored: StoredTracker? { trackers.tracker(named: pull.tracker) }

    var body: some View {
        Loaded(value: detail, error: error) { d in
            List {
                Section {
                    Text(d.title).font(.headline).textSelection(.enabled)
                    HStack(spacing: 8) {
                        Text(d.state).font(.caption.weight(.semibold))
                            .foregroundStyle(d.state == "open" ? .green : d.state == "merged" ? .purple : .secondary)
                        if d.draft { Text("draft").font(.caption) }
                        if d.conflicts { Label("conflicts", systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange) }
                        Spacer()
                        if let a = d.additions, let r = d.deletions {
                            Text("+\(a)").foregroundStyle(.green).font(.caption.monospaced())
                            Text("−\(r)").foregroundStyle(.red).font(.caption.monospaced())
                        }
                    }
                    LabeledContent("Branch") { Text("\(d.branch) → \(d.base)").font(.caption.monospaced()) }
                    if let a = d.author { LabeledContent("Author", value: a.me ? "\(a.name) (you)" : a.name) }
                    ForEach(d.warnings, id: \.self) { Text($0).font(.caption).foregroundStyle(.orange) }
                }
                Picker("Show", selection: $tab) {
                    Text("Conversation").tag(0)
                    Text("Files\(d.changedFiles.map { " · \($0)" } ?? "")").tag(1)
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                if tab == 0 {
                    conversation(d)
                } else if let stored {
                    PullFilesSection(tracker: stored, pull: pull)
                }
            }
        }
        .navigationTitle(pullName(pull))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    if !pull.fork, detail?.state ?? "open" == "open" {
                        Button("Work on It", systemImage: "hammer") { starting = .pull(pull, review: false) }
                        Button("Review It", systemImage: "eye") { starting = .pull(pull, review: true) }
                    }
                    if let d = detail, !d.offers.isEmpty {
                        Button("Give a Review…", systemImage: "checkmark.seal") { voting = true }
                    }
                    Button("Open in the Browser", systemImage: "safari") { openURL(URL(string: pull.url)!) }
                    Button("Copy Branch", systemImage: "doc.on.doc") { UIPasteboard.general.string = pull.branch }
                    Button("Copy Link", systemImage: "link") { UIPasteboard.general.string = pull.url }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .task { await load() }
        .refreshable { await load() }
        .sheet(item: $replying) { t in
            CommentEditor(title: "Reply", text: "", lost: []) { text in
                change(t.id) { try await TrackerCore.pullComment($0, repo: pull.repo, id: pull.id, body: text, thread: t.id) }
            }
        }
        .sheet(isPresented: $voting) {
            if let d = detail {
                VoteSheet(offers: d.offers, mine: d.myVote) { verdict, body in
                    change("vote") { try await TrackerCore.pullVote($0, repo: pull.repo, id: pull.id, verdict: verdict, body: body) }
                }
            }
        }
        .failure($failed)
    }

    @ViewBuilder
    private func conversation(_ d: PullDetail) -> some View {
        Section("Description") {
            if d.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text("No description").foregroundStyle(.secondary)
            } else {
                MarkdownText(text: d.body)
            }
        }
        if !d.checks.isEmpty {
            Section("Checks") {
                ForEach(Array(d.checks.enumerated()), id: \.offset) { _, c in
                    HStack {
                        Image(systemName: checkIcon(c.state)).foregroundStyle(checkColor(c.state))
                        Text(c.name).font(.callout)
                        if c.required { Text("required").font(.caption2).foregroundStyle(.secondary) }
                        Spacer()
                        if let url = c.url.flatMap(URL.init(string:)) {
                            Button { openURL(url) } label: { Image(systemName: "arrow.up.right.square") }.buttonStyle(.borderless)
                        }
                    }
                }
            }
        }
        if !d.reviewers.isEmpty {
            Section("Reviewers") {
                ForEach(Array(d.reviewers.enumerated()), id: \.offset) { _, r in
                    LabeledContent(r.who.me ? "\(r.who.name) (you)" : r.who.name, value: verdictSaid(r.verdict) + (r.required ? " · required" : ""))
                }
            }
        }
        Section {
            if d.older > 0 {
                Text("\(d.older) older thread\(d.older == 1 ? "" : "s") not shown").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(d.conversation) { t in thread(t) }
            VStack(alignment: .leading) {
                TextField("Comment (Markdown)", text: $draft, axis: .vertical).lineLimit(2...8)
                HStack {
                    Spacer()
                    Button("Comment") {
                        let body = draft
                        change("comment") {
                            try await TrackerCore.pullComment($0, repo: pull.repo, id: pull.id, body: body, thread: nil)
                            await MainActor.run { draft = "" }
                        }
                    }
                    .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy != nil)
                }
            }
        } header: {
            Text("Conversation")
        }
    }

    private func thread(_ t: PullThread) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                if let path = t.path {
                    Text("\(path)\(t.line.map { ":\($0)" } ?? "")").font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                }
                if t.resolved { Text("resolved").font(.caption2).foregroundStyle(.green) }
                if t.outdated { Text("outdated").font(.caption2).foregroundStyle(.secondary) }
                if let v = t.verdict { Text(verdictSaid(v)).font(.caption2.weight(.semibold)) }
                Spacer()
                if busy == t.id { ProgressView() }
            }
            ForEach(Array(t.comments.enumerated()), id: \.offset) { _, c in
                VStack(alignment: .leading, spacing: 2) {
                    Text(c.author.map { $0.me ? "\($0.name) (you)" : $0.name } ?? "Someone").font(.caption.weight(.semibold))
                    MarkdownText(text: c.body)
                }
            }
        }
        .swipeActions {
            if t.canReply { Button("Reply") { replying = t }.tint(.blue) }
        }
        .contextMenu {
            if t.canReply { Button("Reply", systemImage: "arrowshape.turn.up.left") { replying = t } }
        }
    }

    private func checkIcon(_ s: String) -> String {
        switch s {
        case "passing": "checkmark.circle.fill"
        case "failing": "xmark.circle.fill"
        case "pending": "clock.fill"
        default: "minus.circle"
        }
    }

    private func checkColor(_ s: String) -> Color {
        switch s {
        case "passing": .green
        case "failing": .red
        case "pending": .yellow
        default: .secondary
        }
    }

    private func load() async {
        guard let stored else {
            error = "No tracker called \(pull.tracker) on this phone."
            return
        }
        do {
            detail = try await TrackerCore.pull(stored, repo: pull.repo, id: pull.id)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func change(_ key: String, _ work: @escaping (StoredTracker) async throws -> Void) {
        guard let stored else { return }
        busy = key
        Task {
            defer { busy = nil }
            do {
                try await work(stored)
                await load()
            } catch {
                failed = error.localizedDescription
            }
        }
    }
}

/// A verdict as the desktop says it.
func verdictSaid(_ v: String) -> String {
    switch v {
    case "approved": "approved"
    case "suggestions": "approved with suggestions"
    case "changes-requested": "asked for changes"
    case "waiting": "waiting for the author"
    case "rejected": "rejected"
    case "commented": "commented"
    case "dismissed": "dismissed"
    default: "not yet"
    }
}

func verdictGive(_ v: String) -> String {
    switch v {
    case "approved": "Approve"
    case "suggestions": "Approve with Suggestions"
    case "changes-requested": "Request Changes"
    case "waiting": "Wait for Author"
    case "rejected": "Reject"
    case "commented": "Comment"
    case "dismissed": "Dismiss"
    default: "Reset My Vote"
    }
}

/// A review: a verdict from what the tracker offers, and what to say with it.
private struct VoteSheet: View {
    let offers: [String]
    let mine: String?
    let give: (String, String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var verdict = ""
    @State private var body_ = ""

    var body: some View {
        NavigationStack {
            Form {
                if let mine { Section { LabeledContent("Your vote now", value: verdictSaid(mine)) } }
                Picker("Verdict", selection: $verdict) {
                    ForEach(offers, id: \.self) { Text(verdictGive($0)).tag($0) }
                }
                .pickerStyle(.inline)
                Section("Say something with it") {
                    TextField("Optional (Markdown)", text: $body_, axis: .vertical).lineLimit(3...10)
                }
            }
            .navigationTitle("Review")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Send") {
                        give(verdict, body_)
                        dismiss()
                    }
                    .disabled(verdict.isEmpty)
                }
            }
            .onAppear { verdict = offers.first ?? "" }
        }
    }
}

/// A pull request's files, each opening its diff.
private struct PullFilesSection: View {
    let tracker: StoredTracker
    let pull: PullRequestSummary
    @State private var files: PullFiles?
    @State private var error: String?

    var body: some View {
        Section {
            if let files {
                ForEach(files.files) { f in
                    NavigationLink {
                        PullFileDiffView(tracker: tracker, files: files, file: f)
                    } label: {
                        HStack {
                            ChangeRow(file: ChangedFile(path: f.path, change: f.change))
                            Spacer()
                            if let a = f.additions, let d = f.deletions {
                                Text("+\(a) −\(d)").font(.caption2.monospaced()).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if files.truncated { Text("Only the first files are listed.").font(.caption).foregroundStyle(.secondary) }
            } else if let error {
                Text(error).foregroundStyle(.red).font(.callout)
            } else {
                ProgressView()
            }
        }
        .task {
            do { files = try await TrackerCore.pullFiles(tracker, repo: pull.repo, id: pull.id) } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// One file of a pull request, both sides as one column.
private struct PullFileDiffView: View {
    let tracker: StoredTracker
    let files: PullFiles
    let file: PullFile
    @State private var pieces: [UnifiedDiff.Piece]?
    @State private var binary = false
    @State private var error: String?

    var body: some View {
        Group {
            if binary {
                ContentUnavailableView("Binary file", systemImage: "doc.questionmark")
            } else {
                Loaded(value: pieces, error: error) { pieces in
                    ScrollView([.vertical, .horizontal]) {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(pieces) { piece in
                                switch piece {
                                case let .lines(lines):
                                    ForEach(lines) { line in
                                        HStack(spacing: 6) {
                                            Text(line.old.map(String.init) ?? "").frame(width: 34, alignment: .trailing).foregroundStyle(.tertiary)
                                            Text(line.new.map(String.init) ?? "").frame(width: 34, alignment: .trailing).foregroundStyle(.tertiary)
                                            Text(line.kind == .added ? "+" : line.kind == .removed ? "-" : " ")
                                            Text(line.text.isEmpty ? " " : line.text).fixedSize()
                                        }
                                        .background(line.kind == .added ? Color.green.opacity(0.15)
                                                    : line.kind == .removed ? Color.red.opacity(0.15) : .clear)
                                    }
                                case let .folded(count, _):
                                    Text("⋯ \(count) unchanged").foregroundStyle(.secondary).padding(.vertical, 4)
                                }
                            }
                        }
                        .font(.system(size: 12, design: .monospaced))
                        .padding(.vertical, 6)
                    }
                    .defaultScrollAnchor(.topLeading)
                }
            }
        }
        .navigationTitle((file.path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .task {
            do {
                let d = try await TrackerCore.pullFile(tracker, files: files, file: file)
                binary = d.binary
                pieces = UnifiedDiff.pieces(UnifiedDiff.lines(original: d.original, modified: d.modified))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
