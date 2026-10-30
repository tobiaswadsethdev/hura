import HuraKit
import SwiftUI

/// Starting a worktree in a project: the task, and what the sandbox gets. The
/// same form as the desktop window's, with the same defaults from the server.
struct NewWorktreeView: View {
    let server: Server
    let project: Project
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
            LabeledContent("Base branch") {
                TextField("default branch", text: $base)
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
            // The checkout's own facts: its branch as the base, the
            // toolchains it points at, the providers to tick.
            let picked = try await server.api.inspect(path: project.path, branch: nil)
            facts = picked.facts
            toolchains = Set(picked.facts.toolchains)
            if let b = picked.branch { base = b }
            if !picked.facts.baseOnRemote { base = "" }
            if !picked.providers.isEmpty { providers = Set(picked.providers) }
        } catch {
            self.error = error.localizedDescription
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
                                 base: trimmed(base), policy: policy,
                                 providers: options.providers.map(\.name).filter(providers.contains),
                                 toolchains: options.toolchains.map(\.name).filter(toolchains.contains),
                                 interface: agentInterface)
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
}
