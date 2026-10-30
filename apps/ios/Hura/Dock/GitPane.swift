import HuraKit
import SwiftUI

/// The worktree's git: the branch and where it stands against its upstream,
/// what is staged and what isn't, a commit, the remote, and the review being
/// written. Every change answers with git's own words and the status re-read,
/// since the agent is editing while this runs.
struct GitPane: View {
    let server: Server
    let name: String
    @State private var answer: GitAnswer?
    @State private var comments: [Comment] = []
    @State private var error: String?
    @State private var failed: String?
    @State private var running: String?
    @State private var message = ""
    @State private var discarding: String?

    var body: some View {
        Loaded(value: answer, error: error) { answer in
            List {
                header(answer)
                let s = answer.status
                if s.staged.isEmpty && s.unstaged.isEmpty {
                    Section {
                        Label("Nothing to commit", systemImage: "checkmark.circle").foregroundStyle(.green)
                    }
                } else {
                    Section {
                        TextField("Commit message", text: $message, axis: .vertical)
                            .lineLimit(1...5)
                        Button {
                            run("commit", [.commit(message)])
                        } label: {
                            Label(s.staged.isEmpty ? "Stage something first" : "Commit \(s.staged.count)",
                                  systemImage: "checkmark.seal")
                        }
                        .disabled(running != nil || s.staged.isEmpty
                            || message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    changes("Staged", s.staged, against: .staged, verb: "Unstage") { .unstage($0) }
                    changes("Changes", s.unstaged, against: .worktree, verb: "Stage") { .stage($0) }
                }
                review
            }
            .listStyle(.insetGrouped)
        }
        .refreshable { await load() }
        .task { await load() }
        .confirmationDialog("Throw away your changes?", isPresented: .constant(discarding != nil),
                            titleVisibility: .visible, presenting: discarding) { path in
            Button("Discard Changes", role: .destructive) {
                discarding = nil
                run("discard", [.discard(path)])
            }
            Button("Cancel", role: .cancel) { discarding = nil }
        } message: { path in
            Text("\(path) goes back to what it was, and the agent may be part-way through writing it.")
        }
        .failure($failed)
    }

    @ViewBuilder
    private func header(_ answer: GitAnswer) -> some View {
        let s = answer.status
        Section {
            HStack {
                Label(s.branch, systemImage: "arrow.triangle.branch").font(.body.monospaced())
                Spacer()
                if running != nil { ProgressView() }
            }
            if let up = s.upstream {
                HStack(spacing: 10) {
                    Text("→ \(up)").font(.caption.monospaced()).foregroundStyle(.secondary)
                    if s.ahead > 0 { Text("\(s.ahead) ahead").font(.caption).foregroundStyle(.blue) }
                    if s.behind > 0 { Text("\(s.behind) behind").font(.caption).foregroundStyle(.orange) }
                    if s.ahead == 0 && s.behind == 0 { Text("in sync").font(.caption).foregroundStyle(.secondary) }
                }
            } else {
                Text("Not on the remote yet").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                remote("Fetch", "arrow.down.circle", .fetch)
                remote("Pull", "arrow.down.to.line", .pull)
                remote(s.upstream == nil ? "Publish" : "Push", "arrow.up.circle", .push)
            }
            .buttonStyle(.bordered)
            .font(.callout)
            let said = answer.said.trimmingCharacters(in: .whitespacesAndNewlines)
            if !said.isEmpty {
                Text(said).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
            }
        }
    }

    private func remote(_ title: String, _ icon: String, _ op: GitOp) -> some View {
        Button {
            run(title, [op])
        } label: {
            Label(title, systemImage: icon).frame(maxWidth: .infinity)
        }
        .disabled(running != nil)
    }

    @ViewBuilder
    private func changes(_ title: String, _ files: [ChangedFile], against: Against, verb: String,
                         op: @escaping (String) -> GitOp) -> some View {
        if !files.isEmpty {
            Section {
                ForEach(files, id: \.self) { f in
                    NavigationLink {
                        FileDiffView(server: server, name: name, path: f.path, against: against)
                    } label: {
                        ChangeRow(file: f)
                    }
                    .swipeActions(edge: .trailing) {
                        Button(verb) { run(verb, [op(f.path)]) }.tint(verb == "Stage" ? .green : .orange)
                        if against == .worktree {
                            Button("Discard", role: .destructive) { discarding = f.path }
                        }
                    }
                    .contextMenu {
                        Button(verb, systemImage: verb == "Stage" ? "plus" : "minus") { run(verb, [op(f.path)]) }
                        NavigationLink {
                            FileDiffView(server: server, name: name, path: f.path, against: .base)
                        } label: {
                            Label("Diff Against Base", systemImage: "arrow.left.arrow.right")
                        }
                        Button("Copy Path", systemImage: "doc.on.doc") { UIPasteboard.general.string = f.path }
                        if against == .worktree {
                            Button("Discard Changes…", systemImage: "arrow.uturn.backward", role: .destructive) {
                                discarding = f.path
                            }
                        }
                    }
                }
            } header: {
                HStack {
                    Text("\(title) · \(files.count)")
                    Spacer()
                    Button("\(verb) All") { run(verb, files.map { op($0.path) }) }
                        .font(.caption)
                        .textCase(nil)
                        .disabled(running != nil)
                }
            }
        }
    }

    @ViewBuilder
    private var review: some View {
        if !comments.isEmpty {
            Section {
                ForEach(comments) { c in
                    VStack(alignment: .leading, spacing: 3) {
                        Text("\(c.file):\(c.line)").font(.caption.monospaced()).foregroundStyle(.secondary)
                        Text(c.body).font(.callout)
                    }
                    .swipeActions {
                        Button("Remove", role: .destructive) {
                            act { comments = try await server.api.uncomment(name, id: c.id) }
                        }
                    }
                }
                Button {
                    act {
                        _ = try await server.api.sendComments(name)
                        comments = try await server.api.comments(name)
                    }
                } label: {
                    Label("Send Review to the Agent", systemImage: "paperplane")
                }
            } header: {
                Text("Review · \(comments.count)")
            } footer: {
                Text("Sent as one message, and then forgotten. Add remarks by tapping a line in a diff.")
            }
        }
    }

    private func load() async {
        do {
            async let c = server.api.comments(name)
            answer = try await server.api.gitStatus(name)
            comments = (try? await c) ?? comments
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func run(_ what: String, _ ops: [GitOp]) {
        running = what
        Task {
            defer { running = nil }
            do {
                for op in ops { answer = try await server.api.git(name, op) }
                if ops.contains(where: { if case .commit = $0 { true } else { false } }) { message = "" }
            } catch {
                failed = error.localizedDescription
                await load()
            }
        }
    }

    private func act(_ work: @escaping () async throws -> Void) {
        Task {
            do { try await work() } catch { failed = error.localizedDescription }
        }
    }
}

struct ChangeRow: View {
    let file: ChangedFile

    var body: some View {
        HStack(spacing: 10) {
            Text(mark)
                .font(.caption.monospaced().weight(.bold))
                .foregroundStyle(color)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 1) {
                Text((file.path as NSString).lastPathComponent).font(.callout)
                let dir = (file.path as NSString).deletingLastPathComponent
                if !dir.isEmpty {
                    Text(dir).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                }
            }
        }
    }

    private var mark: String {
        switch file.change {
        case "added": "A"
        case "modified": "M"
        case "deleted": "D"
        case "renamed": "R"
        case "untracked": "U"
        case "conflicted": "!"
        default: "?"
        }
    }

    private var color: Color {
        switch file.change {
        case "added", "untracked": .green
        case "deleted", "conflicted": .red
        case "renamed": .blue
        default: .orange
        }
    }
}

/// One file's diff in one column, with the review's remarks on it. Tap a line
/// of the new side to remark on it, as clicking one does on the desktop.
struct FileDiffView: View {
    let server: Server
    let name: String
    let path: String
    @State var against: Against
    @State private var diff: FileDiff?
    @State private var pieces: [UnifiedDiff.Piece] = []
    @State private var comments: [Comment] = []
    @State private var error: String?
    @State private var failed: String?
    @State private var writing: UnifiedDiff.Line?
    @State private var draft = ""
    @State private var unfolded: Set<Int> = []
    @FocusState private var typing: Bool

    init(server: Server, name: String, path: String, against: Against) {
        self.server = server
        self.name = name
        self.path = path
        _against = State(initialValue: against)
    }

    private var mine: [Comment] { comments.filter { $0.file == path } }

    var body: some View {
        Loaded(value: diff, error: error) { diff in
            if diff.binary {
                ContentUnavailableView("Binary file", systemImage: "doc.questionmark",
                                       description: Text("There's no diff to show."))
            } else {
                VStack(spacing: 0) {
                    Text("\(diff.originalLabel) → \(diff.modifiedLabel)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(6)
                        .background(.bar)
                    ScrollView([.vertical, .horizontal]) {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(pieces) { piece in
                                switch piece {
                                case let .lines(lines):
                                    ForEach(lines) { line in row(line) }
                                case let .folded(count, from):
                                    if unfolded.contains(from) {
                                        ForEach(allLines(diff)[from..<(from + count)]) { line in row(line) }
                                    } else {
                                        Button("⋯ \(count) unchanged line\(count == 1 ? "" : "s")") { unfolded.insert(from) }
                                            .font(.caption)
                                            .padding(.vertical, 6)
                                            .padding(.leading, 10)
                                    }
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
        .navigationTitle((path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Picker("Compare", selection: $against) {
                        Text("Staged against HEAD").tag(Against.staged)
                        Text("Working copy against the index").tag(Against.worktree)
                        Text("Against the base branch").tag(Against.base)
                    }
                    Button("Copy Path", systemImage: "doc.on.doc") { UIPasteboard.general.string = path }
                } label: {
                    Label("Compare", systemImage: "arrow.left.arrow.right")
                }
            }
        }
        .sheet(item: $writing) { line in
            NavigationStack {
                Form {
                    Section {
                        Text(line.text).font(.caption.monospaced())
                    } header: {
                        Text("\(path):\(line.new ?? 0)")
                    }
                    Section {
                        TextField("Your remark", text: $draft, axis: .vertical)
                            .lineLimit(3...10)
                            .focused($typing)
                            .onAppear { typing = true }
                    } footer: {
                        Text("Kept with the review until you send it to the agent from Git.")
                    }
                }
                .navigationTitle("Comment")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { writing = nil } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Add") { add(line) }
                            .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .presentationDetents([.medium])
        }
        .task(id: against) { await load() }
        .failure($failed)
    }

    private func allLines(_ diff: FileDiff) -> [UnifiedDiff.Line] {
        UnifiedDiff.lines(original: diff.original, modified: diff.modified)
    }

    @ViewBuilder
    private func row(_ line: UnifiedDiff.Line) -> some View {
        let remarks = line.kind != .removed ? mine.filter { $0.line == line.new } : []
        HStack(spacing: 6) {
            Text(line.old.map(String.init) ?? "").frame(width: 34, alignment: .trailing).foregroundStyle(.tertiary)
            Text(line.new.map(String.init) ?? "").frame(width: 34, alignment: .trailing).foregroundStyle(.tertiary)
            Text(line.kind == .added ? "+" : line.kind == .removed ? "-" : " ")
                .foregroundStyle(line.kind == .added ? .green : line.kind == .removed ? .red : .secondary)
            Text(line.text.isEmpty ? " " : line.text).fixedSize()
            Spacer(minLength: 10)
        }
        .padding(.vertical, 1)
        .background(line.kind == .added ? Color.green.opacity(0.15) : line.kind == .removed ? Color.red.opacity(0.15) : .clear)
        .contentShape(Rectangle())
        .onTapGesture {
            guard line.new != nil else { return }
            draft = ""
            writing = line
        }
        ForEach(remarks) { c in
            HStack(alignment: .top, spacing: 6) {
                Image(systemName: "text.bubble.fill").foregroundStyle(.blue)
                // A width of its own: the diff scrolls sideways, so there is
                // no edge for the text to wrap at.
                Text(c.body)
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(width: 240, alignment: .leading)
                Button {
                    Task {
                        do { comments = try await server.api.uncomment(name, id: c.id) } catch { failed = error.localizedDescription }
                    }
                } label: {
                    Image(systemName: "xmark.circle").foregroundStyle(.secondary)
                }
                .accessibilityLabel("Remove the comment on line \(c.line)")
            }
            .padding(6)
            .background(Color.blue.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
            .padding(.leading, 84)
            .padding(.vertical, 2)
        }
    }

    private func load() async {
        do {
            async let c = server.api.comments(name)
            let d = try await server.api.gitDiff(name, path: path, against: against)
            pieces = UnifiedDiff.pieces(UnifiedDiff.lines(original: d.original, modified: d.modified))
            unfolded = []
            diff = d
            comments = (try? await c) ?? comments
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func add(_ line: UnifiedDiff.Line) {
        let body = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                comments = try await server.api.comment(name, NewComment(file: path, line: line.new ?? 0,
                                                                         excerpt: line.text, body: body))
                writing = nil
            } catch {
                failed = error.localizedDescription
            }
        }
    }
}
