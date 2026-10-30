import HuraKit
import SwiftUI

/// Starting a worktree in a project: the task, and what the sandbox gets. The
/// same form as the desktop window's, with the same defaults from the server.
/// Where a worktree's work comes from, when it isn't typed.
enum WorkOrigin: Hashable {
    case ticket(TrackerTask)
    /// A pull request, to work on (its pushes update it) or to review.
    case pull(PullRequestSummary, review: Bool)
}

struct NewWorktreeView: View {
    let server: Server
    let project: Project
    var origin: WorkOrigin?
    /// Opens the new worktree once it exists.
    var onCreated: (String) -> Void = { _ in }
    @Environment(\.dismiss) private var dismiss

    @State private var options: NewOptions?
    @State private var facts: Facts?
    @State private var task = ""
    @State private var name = ""
    @State private var base = ""
    @State private var policy = ""
    @State private var agentInterface = "chat"
    @State private var toolchains: Set<String> = []
    @State private var providers: Set<String> = []
    @State private var busy = false
    @State private var error: String?
    /// The work branch a ticket or a pull request names; empty otherwise.
    @State private var branch = ""

    var body: some View {
        NavigationStack {
            Form {
                if let error {
                    Section { Text(error).foregroundStyle(.red).font(.callout) }
                }
                if let options {
                    form(options)
                } else if error == nil {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .navigationTitle("New Worktree")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if busy {
                        ProgressView()
                    } else {
                        Button("Start", action: submit).disabled(options == nil)
                    }
                }
            }
            .interactiveDismissDisabled(busy)
            .task { await load() }
        }
    }

    @ViewBuilder
    private func form(_ options: NewOptions) -> some View {
        switch origin {
        case let .ticket(t)?:
            Section {
                LabeledContent(t.key) { Text(t.title).lineLimit(2) }
                LabeledContent("Branch") { Text(branch).font(.footnote.monospaced()) }
            } header: {
                Text("From a ticket")
            }
        case let .pull(p, review)?:
            Section {
                LabeledContent(pullName(p)) { Text(p.title).lineLimit(2) }
                LabeledContent("Branch") { Text(p.branch).font(.footnote.monospaced()) }
            } header: {
                Text("From a pull request")
            } footer: {
                Text(review ? "To review it: the agent is asked not to push." : "To work on it: its pushes update the pull request.")
            }
        case nil:
            EmptyView()
        }
        Section {
            TextField("What should the agent do? Leave blank to start it idle.", text: $task, axis: .vertical)
                .lineLimit(4...12)
        } header: {
            Text("Task")
        } footer: {
            Text("In \(project.name), cloned from \(project.repo).")
        }
        Section {
            LabeledContent("Name") {
                TextField("from the task", text: $name)
                    .multilineTextAlignment(.trailing)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            }
            LabeledContent(isPull ? "Merges into" : "Base branch") {
                TextField("default branch", text: $base)
                    .disabled(isPull)
                    .multilineTextAlignment(.trailing)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.body.monospaced())
            }
            Picker("Agent", selection: $agentInterface) {
                Text("Chat").tag("chat")
                Text("Terminal").tag("terminal")
            }
            Picker("Network policy", selection: $policy) {
                ForEach(options.policies, id: \.spec) { p in
                    Text(p.spec).tag(p.spec)
                }
            }
        } footer: {
            if let p = options.policies.first(where: { $0.spec == policy }) {
                Text(p.summary)
            }
        }
        if !options.toolchains.isEmpty {
            Section("Toolchains") {
                ForEach(options.toolchains, id: \.name) { t in
                    Toggle(isOn: member(t.name, of: $toolchains)) {
                        VStack(alignment: .leading) {
                            Text(t.name)
                            Text(t.summary).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        Section {
            ForEach(options.providers, id: \.name) { p in
                Toggle(isOn: member(p.name, of: $providers)) {
                    VStack(alignment: .leading) {
                        Text(p.name)
                        Text(p.kind).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if options.providers.isEmpty {
                Text(options.providersError ?? "None configured").foregroundStyle(.secondary)
            }
        } header: {
            Text("Credentials")
        }
        // Named, not offered: the server's config file decides these.
        Section {
            LabeledContent("Skills", value: options.skills.isEmpty ? "none" : options.skills.joined(separator: ", "))
            LabeledContent("MCP servers", value: options.mcp.isEmpty ? "none" : options.mcp.joined(separator: ", "))
        } footer: {
            Text("Set in the server's config. MCP servers act with the server's credentials.")
        }
        if let facts, facts.uncommitted > 0 || (facts.unpushed ?? 0) > 0 || !facts.baseOnRemote {
            Section {
                if facts.uncommitted > 0 {
                    Label("\(facts.uncommitted) uncommitted change\(facts.uncommitted == 1 ? "" : "s") in the checkout won't be in the worktree.",
                          systemImage: "exclamationmark.triangle")
                }
                if let n = facts.unpushed, n > 0 {
                    Label("\(n) unpushed commit\(n == 1 ? "" : "s") won't be either: the sandbox clones the remote.",
                          systemImage: "exclamationmark.triangle")
                }
                if !facts.baseOnRemote {
                    Label("The checkout's branch isn't on the remote, so the worktree starts from the default branch.",
                          systemImage: "info.circle")
                }
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
    }

    private var isPull: Bool { if case .pull? = origin { true } else { false } }

    private func member(_ value: String, of set: Binding<Set<String>>) -> Binding<Bool> {
        Binding(get: { set.wrappedValue.contains(value) },
                set: { on in if on { set.wrappedValue.insert(value) } else { set.wrappedValue.remove(value) } })
    }

    private func load() async {
        do {
            let o = try await server.api.newOptions()
            policy = o.defaultPolicy
            providers = Set(o.defaultProviders)
            agentInterface = o.defaultInterface
            base = o.defaultBase ?? ""
            options = o
            await prefill()
            // The checkout's own facts: its branch as the base, the
            // toolchains it points at, the providers to tick. A pull
            // request's base is its own.
            var pullBase: String?
            if case let .pull(p, _)? = origin { pullBase = p.base }
            let picked = try await server.api.inspect(path: project.path, branch: pullBase)
            facts = picked.facts
            toolchains = Set(picked.facts.toolchains)
            if pullBase == nil {
                if let b = picked.branch { base = b }
                if !picked.facts.baseOnRemote { base = "" }
            }
            if !picked.providers.isEmpty { providers = Set(picked.providers) }
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// The task, name and branch a ticket or a pull request suggests, as the
    /// desktop's form fills them in.
    private func prefill() async {
        switch origin {
        case let .ticket(t)?:
            task = "\(t.key): \(t.title)\n\n\(t.url)"
            name = t.sessionName
            // The branch under this server's prefix, which is what a session
            // here will be given.
            let prefix = (try? await server.api.settings()).map { $0.settings.branchPrefix ?? $0.defaultBranchPrefix } ?? "hura"
            branch = (try? await TrackerCore.branch(prefix: prefix, key: t.key, title: t.title)) ?? t.branch
        case let .pull(p, review)?:
            task = pullTask(p, review: review)
            branch = p.branch
            base = p.base
        case nil:
            break
        }
    }

    private func submit() {
        guard let options else { return }
        busy = true
        error = nil
        let trimmed = { (s: String) -> String? in
            let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
            return t.isEmpty ? nil : t
        }
        let session = NewSession(project: project.name, name: trimmed(name), repo: project.repo, task: task,
                                 base: trimmed(base), branch: trimmed(branch), policy: policy,
                                 providers: options.providers.map(\.name).filter(providers.contains),
                                 toolchains: options.toolchains.map(\.name).filter(toolchains.contains),
                                 interface: agentInterface, ticket: ticketOrigin, pull: pullOrigin)
        Task {
            defer { busy = false }
            do {
                let created = try await server.create(session)
                dismiss()
                onCreated(created)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private var ticketOrigin: TicketOrigin? {
        guard case let .ticket(t)? = origin else { return nil }
        return TicketOrigin(tracker: t.tracker, kind: t.kind, id: t.id, key: t.key, url: t.url, repo: t.repo)
    }

    private var pullOrigin: PullOrigin? {
        guard case let .pull(p, _)? = origin else { return nil }
        return PullOrigin(tracker: p.tracker, kind: p.kind, id: p.id, key: p.key, url: p.url, repo: p.repo)
    }
}

/// `repo #12`, as the desktop names a pull request.
func pullName(_ p: PullRequestSummary) -> String {
    "\(p.repo.split(separator: "/").last.map(String.init) ?? p.repo) \(p.key)"
}

/// The task a pull request starts with: `pullTask` in the desktop's
/// NewWorktree.tsx.
func pullTask(_ p: PullRequestSummary, review: Bool) -> String {
    let where_ = "This worktree is on the pull request's own branch, `\(p.branch)`, which merges into `\(p.base)`."
    if review {
        return "Review \(pullName(p)): \(p.title)\n\(p.url)\n\n\(where_) "
            + "Read what it changes with `git diff origin/\(p.base)...HEAD` and its commits with "
            + "`git log origin/\(p.base)..HEAD`, run the tests that cover it, and report what you "
            + "find: what is wrong first, then what is risky, then anything smaller, each with its file "
            + "and line. Do not commit, push, or comment on the pull request."
    }
    return "\(pullName(p)): \(p.title)\n\(p.url)\n\n\(where_) Pushing updates the pull request.\n\n"
}
