import HuraKit
import SwiftUI

/// A pull request in full: what it is, its checks and reviewers, its
/// conversation (with replies), a review to give, merging it now or once it
/// can, and its files. Read with
/// the tracker's own token, as the desktop reads it.
/// A pull request's images, once per launch: it is read again on every
/// refresh, and its images with it.
@MainActor enum PullPictures {
    static var cache: [String: Picture] = [:]
}

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
    @State private var merging: MergeSheet.Mode?
    @State private var cancelling = false

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
                        if d.conflicts { Label("conflicts", glyph: .warning).font(.caption).foregroundStyle(.orange) }
                        Spacer()
                        if let a = d.additions, let r = d.deletions {
                            Text("+\(a)").foregroundStyle(.green).font(.caption.monospacedDigit())
                            Text("−\(r)").foregroundStyle(.red).font(.caption.monospacedDigit())
                        }
                    }
                    LabeledContent("Branch") { Text("\(d.branch) → \(d.base)").font(.caption) }
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
        // Its text's images are the forge's, read through the tracker.
        .environment(\.readForgePicture) { [stored] url in
            guard let stored else { return nil }
            if let cached = PullPictures.cache[url] { return cached }
            let picture = try? await TrackerCore.pullImage(stored, url: url)
            PullPictures.cache[url] = picture
            return picture
        }
        .navigationTitle(pullName(pull))
        // A pull request has the whole screen; back is the way to the tabs.
        .toolbar(.hidden, for: .tabBar)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    if !pull.fork, detail?.state ?? "open" == "open" {
                        Button("Work on It", glyph: .workOn) { starting = .pull(pull, review: false) }
                        Button("Review It", glyph: .review) { starting = .pull(pull, review: true) }
                    }
                    if let d = detail, !d.offers.isEmpty {
                        Button("Give a Review…", glyph: .approve) { voting = true }
                    }
                    if let d = detail, let m = d.merging, !m.methods.isEmpty, !d.draft {
                        let words = MergeWords(kind: d.kind)
                        Button("\(words.now)…", glyph: .merge) { merging = .now }
                        if m.auto == nil, m.canAuto {
                            Button("\(words.auto)…", glyph: .autoMerge) { merging = .auto }
                        }
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
        .sheet(item: $merging) { mode in
            if let d = detail, let m = d.merging {
                MergeSheet(mode: mode, kind: d.kind, base: d.base, merging: m) { how, deleteBranch in
                    change("merge") {
                        switch mode {
                        case .now:
                            try await TrackerCore.pullMerge($0, repo: pull.repo, id: pull.id, how: how, head: m.head,
                                                            deleteBranch: deleteBranch)
                        case .auto:
                            try await TrackerCore.pullAutoMerge($0, repo: pull.repo, id: pull.id, how: how,
                                                                deleteBranch: deleteBranch)
                        }
                    }
                }
            }
        }
        .confirmationDialog(detail.map { "\(MergeWords(kind: $0.kind).cancel)?" } ?? "", isPresented: $cancelling,
                            titleVisibility: .visible) {
            Button(detail.map { MergeWords(kind: $0.kind).cancel } ?? "Cancel", role: .destructive) {
                change("merge") {
                    try await TrackerCore.pullAutoMerge($0, repo: pull.repo, id: pull.id, how: nil, deleteBranch: false)
                }
            }
        } message: {
            Text("It stays open, and merges only when someone merges it.")
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
                        Image(glyph: checkIcon(c.state)).foregroundStyle(checkColor(c.state))
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
        if let m = d.merging { mergeSection(d, m) }
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

    @ViewBuilder
    private func mergeSection(_ d: PullDetail, _ m: PullMerging) -> some View {
        let words = MergeWords(kind: d.kind)
        Section {
            if let auto = m.auto {
                VStack(alignment: .leading, spacing: 3) {
                    Label(words.isSet, glyph: .autoMerge).foregroundStyle(.green)
                    let parts = [auto.by.map { "by \($0.me ? "you" : $0.name)" },
                                 auto.method.map(words.method),
                                 auto.deleteBranch ? "deletes the branch" : nil].compactMap { $0 }
                    if !parts.isEmpty {
                        Text(parts.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Button(words.cancel, systemImage: "xmark.circle", role: .destructive) { cancelling = true }
                    .disabled(busy != nil)
            }
            if d.draft {
                Text("A draft can't be merged until it's published.").font(.callout).foregroundStyle(.secondary)
            } else if m.methods.isEmpty {
                Text("This account can't merge it.").font(.callout).foregroundStyle(.secondary)
            } else {
                if d.conflicts {
                    Label("It conflicts with \(d.base), so it can't merge as it stands.", glyph: .warning)
                        .font(.callout).foregroundStyle(.orange)
                }
                HStack {
                    // Text, not a Label: in a list row the label's icon is
                    // laid out but not drawn, which pushes the title off centre.
                    Button { merging = .now } label: {
                        Text(words.now).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    if m.auto == nil, m.canAuto {
                        Button { merging = .auto } label: {
                            Text(words.autoShort).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .disabled(busy != nil)
            }
        } header: {
            HStack {
                Text(words.section)
                Spacer()
                if busy == "merge" { ProgressView() }
            }
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

    private func checkIcon(_ s: String) -> Glyph {
        switch s {
        case "passing": .passing
        case "failing": .failing
        case "pending": .pending
        default: .skipped
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
            error = "No tracker called \(pull.tracker) in your account."
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

/// What each forge calls merging: GitHub merges and auto-merges, Azure DevOps
/// completes and auto-completes.
private struct MergeWords {
    let azure: Bool

    init(kind: String) { azure = kind == "azure-dev-ops" }

    var section: String { azure ? "Complete" : "Merge" }
    var now: String { azure ? "Complete" : "Merge" }
    var auto: String { azure ? "Set Auto-Complete" : "Enable Auto-Merge" }
    var autoShort: String { azure ? "Auto-Complete" : "Auto-Merge" }
    var isSet: String { azure ? "Auto-complete is set" : "Auto-merge is on" }
    var cancel: String { azure ? "Cancel Auto-Complete" : "Disable Auto-Merge" }

    func method(_ m: String) -> String {
        switch m {
        case "merge": azure ? "Merge (no fast forward)" : "Create a merge commit"
        case "squash": azure ? "Squash commit" : "Squash and merge"
        case "rebase": azure ? "Rebase and fast-forward" : "Rebase and merge"
        case "rebase-merge": "Semi-linear merge"
        default: m
        }
    }
}

/// Merging now, or once it can: which way, and whether the branch goes.
private struct MergeSheet: View {
    enum Mode: String, Identifiable {
        case now, auto
        var id: String { rawValue }
    }

    let mode: Mode
    let kind: String
    let base: String
    let merging: PullMerging
    let go: (String, Bool) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var how = ""
    @State private var deleteBranch = true

    private var words: MergeWords { MergeWords(kind: kind) }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Merge type", selection: $how) {
                    ForEach(merging.methods, id: \.self) { Text(words.method($0)).tag($0) }
                }
                .pickerStyle(.inline)
                Section {
                    if let deletes = merging.deletesBranch {
                        LabeledContent("Branch", value: deletes ? "deleted after merging" : "kept")
                    } else {
                        Toggle("Delete the branch after merging", isOn: $deleteBranch)
                    }
                } footer: {
                    if mode == .auto {
                        Text("It merges into \(base) by itself once its checks and required reviews pass.")
                    } else if let head = merging.head {
                        Text("Merges \(head.prefix(7)) into \(base). If anything was pushed since, it is refused: refresh and look again.")
                    }
                }
            }
            .navigationTitle(mode == .now ? words.now : words.autoShort)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(mode == .now ? words.now : "Set") {
                        go(how, deleteBranch)
                        dismiss()
                    }
                    .disabled(how.isEmpty)
                }
            }
            .onAppear { how = merging.methods.first ?? "" }
        }
        .presentationDetents([.medium, .large])
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
                        PullFileDiffView(tracker: tracker, files: files, file: f).toolbar(.hidden, for: .tabBar)
                    } label: {
                        HStack {
                            ChangeRow(file: ChangedFile(path: f.path, change: f.change))
                            Spacer()
                            if let a = f.additions, let d = f.deletions {
                                Text("+\(a) −\(d)").font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
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

/// One file of a pull request, both sides as one column, drawn as the
/// session's diff is.
private struct PullFileDiffView: View {
    let tracker: StoredTracker
    let files: PullFiles
    let file: PullFile
    @State private var diff: FileDiff?
    @State private var model: DiffModel?
    @State private var error: String?
    @State private var jump: String?

    var body: some View {
        Loaded(value: diff, error: error) { diff in
            if diff.binary {
                ContentUnavailableView("Binary file", image: Glyph.binary.name,
                                       description: Text("There's no diff to show."))
            } else if let model {
                VStack(spacing: 0) {
                    Text(file.oldPath.map { "\($0) → \(file.path)" } ?? "\(diff.originalLabel) → \(diff.modifiedLabel)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                        .frame(maxWidth: .infinity)
                        .padding(6)
                        .background(.bar)
                    DiffLines(model: model, jump: $jump)
                }
            }
        }
        .navigationTitle((file.path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if (model?.changes.count ?? 0) > 1 {
                    Button("Previous Change", systemImage: "chevron.up") { jump = model?.step(-1, from: jump) }
                    Button("Next Change", systemImage: "chevron.down") { jump = model?.step(1, from: jump) }
                }
                Menu {
                    DiffStyleMenu()
                    Button("Copy Path", systemImage: "doc.on.doc") { UIPasteboard.general.string = file.path }
                } label: {
                    Label("View", systemImage: "textformat.size")
                }
            }
        }
        .task {
            do {
                let d = try await TrackerCore.pullFile(tracker, files: files, file: file)
                model = DiffModel(original: d.original, modified: d.modified)
                diff = d
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// A pull request opened from a worktree that is on it rather than from the
/// inbox: the same screen, starting its own work where the inbox would.
struct LinkedPullView: View {
    let pull: PullRequestSummary
    @State private var starting: WorkOrigin?

    var body: some View {
        PullView(pull: pull, starting: $starting)
            .sheet(item: $starting) { origin in StartWorkView(origin: origin) }
    }
}

/// The pull request a session is on, as the desktop's sidebar row shows it:
/// its number and how its checks stand.
struct PullChip: View {
    let pull: PullRequestSummary

    var body: some View {
        HStack(spacing: 3) {
            Image(glyph: .pullRequest)
            Text(pull.key)
            ChecksMark(checks: pull.checks)
        }
        .font(.caption.weight(.medium))
        .foregroundStyle(.secondary)
        .fixedSize()
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Pull request \(pullName(pull))")
    }
}

/// The pull request a chat's session is on, over the composer as Claude
/// Code's desktop app has it: what it is and where it stands, a tap from the
/// whole of it.
struct PullBar: View {
    let pull: PullRequestSummary

    var body: some View {
        NavigationLink(value: pull) {
            HStack(spacing: 8) {
                Image(glyph: .pullRequest)
                    .foregroundStyle(pull.draft ? Color.secondary : Color.green)
                Text(pull.key).fontWeight(.semibold)
                Text(pull.title).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 4)
                if pull.conflicts { Image(glyph: .warning).foregroundStyle(.orange).accessibilityLabel("Conflicts") }
                ChecksMark(checks: pull.checks)
                if let r = pull.review {
                    Text(reviewWord(r))
                        .foregroundStyle(r == "approved" ? .green : r == "changes-requested" ? .red : .secondary)
                        .fixedSize()
                } else if pull.draft {
                    Text("draft").foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.right").font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .font(.footnote)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
        .accessibilityHint("Opens the pull request")
    }
}
