import HuraKit
import SwiftUI

/// The places on a server that aren't a worktree: what it holds for its
/// sessions, its defaults, and what it is. The desktop window's
/// integrations, settings and about screens.
enum ServerScreen: String, Hashable, Identifiable {
    case integrations, settings, about
    var id: String { rawValue }
}

// MARK: Integrations

/// MCP servers, the secrets they're given, and the skills every new session
/// gets. Every change answers with the whole view re-read, since a stored
/// secret is usually what a container was waiting for.
struct IntegrationsView: View {
    let server: Server
    @State private var view: Integrations?
    @State private var error: String?
    @State private var failed: String?
    @State private var busy: String?
    @State private var setting: NamedSecret?
    @State private var adding = false

    var body: some View {
        Loaded(value: view, error: error) { view in
            List {
                Section {
                    if view.mcp.isEmpty {
                        Text("None. An [[mcp]] table in the server's config file adds one.").foregroundStyle(.secondary)
                    }
                    ForEach(view.mcp) { mcpRow($0) }
                } header: {
                    Text("MCP servers")
                } footer: {
                    Text("Tools the agent can call. They run on the server, in their own containers, holding their own credentials, and the sandbox is granted one endpoint for each. An MCP server is something the agent can do with your credentials: fine for Jira, a way out of the sandbox for a filesystem or Docker server.")
                }
                Section {
                    ForEach(view.secrets) { s in
                        Button { setting = s } label: { secretRow(s) }
                            .tint(.primary)
                            .swipeActions {
                                if s.set {
                                    Button("Forget", role: .destructive) {
                                        act("secret:\(s.name)") { try await server.api.secret(s.name, value: nil) }
                                    }
                                }
                            }
                    }
                    Button("Add a Secret", systemImage: "plus") { adding = true }
                } header: {
                    Text("Secrets")
                } footer: {
                    Text("Held by the server and given to the containers above as environment. A value goes in and never comes back out.")
                }
                Section {
                    if !view.configuredSkills.isEmpty {
                        LabeledContent("From the config file", value: view.configuredSkills.joined(separator: ", "))
                    }
                    if view.skills.isEmpty && view.configuredSkills.isEmpty {
                        Text("None").foregroundStyle(.secondary)
                    }
                    ForEach(view.skills) { s in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(s.name)
                            Text(s.origin).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                        }
                        .swipeActions {
                            Button("Forget", role: .destructive) {
                                act("skill:\(s.name)") { try await server.api.forgetSkill(s.name) }
                            }
                        }
                    }
                } header: {
                    Text("Skills")
                } footer: {
                    Text("Copied into every new session. Skills are pushed to the server from a computer's ~/.claude/skills, by the desktop window; a phone has none of its own to push.")
                }
            }
        }
        .navigationTitle("Integrations")
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $setting) { s in
            SecretSheet(name: s.name, fixedName: true, replacing: s.set) { name, value in
                act("secret:\(name)") { try await server.api.secret(name, value: value) }
            }
        }
        .sheet(isPresented: $adding) {
            SecretSheet(name: "", fixedName: false, replacing: false) { name, value in
                act("secret:\(name)") { try await server.api.secret(name, value: value) }
            }
        }
        .failure($failed)
    }

    private func mcpRow(_ s: McpStatus) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(s.name).font(.body.weight(.medium))
                Text(s.managed ? s.state : "external")
                    .font(.caption.weight(.medium))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(stateColor(s).opacity(0.18), in: Capsule())
                    .foregroundStyle(stateColor(s))
                Spacer()
                if busy == "mcp:\(s.name)" {
                    ProgressView()
                } else if s.managed {
                    if s.state == "running" {
                        Button("Restart", systemImage: "arrow.clockwise") { mcp(s, "restart") }
                            .labelStyle(.iconOnly)
                        Button("Stop", systemImage: "stop.fill") { mcp(s, "stop") }
                            .labelStyle(.iconOnly)
                    } else {
                        Button("Start", systemImage: "play.fill") { mcp(s, "start") }
                            .labelStyle(.iconOnly)
                    }
                }
            }
            .buttonStyle(.borderless)
            Text(s.image ?? s.url).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            if let problem = s.problem { Text(problem).font(.caption).foregroundStyle(.orange) }
            if let log = s.log {
                Text(log).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(8).textSelection(.enabled)
            }
        }
    }

    private func stateColor(_ s: McpStatus) -> Color {
        guard s.managed else { return .secondary }
        switch s.state {
        case "running": return .green
        case "crashing", "detached": return .orange
        default: return .secondary
        }
    }

    private func secretRow(_ s: NamedSecret) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(s.name).font(.callout.monospaced())
                Text(s.usedBy.isEmpty ? "Nothing uses it" : "Used by \(s.usedBy.joined(separator: ", "))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if busy == "secret:\(s.name)" {
                ProgressView()
            } else {
                Text(s.set ? "stored" : "NOT set").font(.caption.weight(.semibold)).foregroundStyle(s.set ? .green : .red)
            }
        }
    }

    private func mcp(_ s: McpStatus, _ action: String) {
        act("mcp:\(s.name)") { try await server.api.mcp(s.name, action: action) }
    }

    private func act(_ key: String, _ work: @escaping () async throws -> Integrations) {
        busy = key
        Task {
            defer { busy = nil }
            do { view = try await work() } catch { failed = error.localizedDescription }
        }
    }

    private func load() async {
        do {
            view = try await server.api.integrations()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// Storing a secret: a name and a value that goes one way.
private struct SecretSheet: View {
    @State var name: String
    let fixedName: Bool
    let replacing: Bool
    let store: (String, String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var value = ""

    init(name: String, fixedName: Bool, replacing: Bool, store: @escaping (String, String) -> Void) {
        _name = State(initialValue: name)
        self.fixedName = fixedName
        self.replacing = replacing
        self.store = store
    }

    private var ready: Bool {
        name.trimmingCharacters(in: .whitespaces).wholeMatch(of: /[A-Za-z_][A-Za-z0-9_]*/) != nil && !value.isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if fixedName {
                        Text(name).font(.body.monospaced())
                    } else {
                        TextField("ANOTHER_TOKEN", text: $name)
                            .font(.body.monospaced())
                            .textInputAutocapitalization(.characters)
                            .autocorrectionDisabled()
                    }
                    SecureField(replacing ? "Replace it" : "Paste the value", text: $value)
                } footer: {
                    Text("The server keeps it and never sends it back.")
                }
            }
            .navigationTitle(replacing ? "Replace Secret" : "Store Secret")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Store") {
                        store(name.trimmingCharacters(in: .whitespaces), value)
                        dismiss()
                    }
                    .disabled(!ready)
                }
            }
        }
        .presentationDetents([.medium])
    }
}

// MARK: Settings

/// The defaults in the server's config file: what every new session on this
/// server starts from, wherever it was started. Saved together, and read
/// back, since a cleared field comes back as the built-in default.
struct ServerSettingsView: View {
    let server: Server
    @State private var view: SettingsView?
    @State private var draft = ServerSettings()
    @State private var options: NewOptions?
    @State private var error: String?
    @State private var failed: String?
    @State private var saving = false

    @Environment(AppModel.self) private var model

    private var dirty: Bool { view.map { $0.settings != draft } ?? false }

    var body: some View {
        Form {
            // This phone's own: what it calls the server, and whether the
            // server may notify it. Both work while the server is offline.
            Section {
                ServerNameField(server: server)
            } header: {
                Text("On this phone")
            }
            if model.push.isAvailable {
                NotificationSettings(server: server)
            }
            if let view {
                serverSections(view)
            } else if let error {
                Section("Server") { Text(error).foregroundStyle(.secondary).font(.callout) }
            } else {
                Section("Server") { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle("Settings")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                if saving {
                    ProgressView()
                } else if view != nil {
                    Button("Save", action: save).disabled(!dirty)
                }
            }
        }
        .task { await load() }
        .failure($failed)
    }

    /// The defaults in the server's config file.
    @ViewBuilder
    private func serverSections(_ view: SettingsView) -> some View {
                Section {
                    LabeledContent("Branch prefix") {
                        TextField(view.defaultBranchPrefix, text: text(\.branchPrefix))
                            .multilineTextAlignment(.trailing)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                    }
                    Picker("Agent", selection: Binding(get: { draft.interface ?? "" },
                                                       set: { draft.interface = $0.isEmpty ? nil : $0 })) {
                        Text("Default").tag("")
                        Text("Chat").tag("chat")
                        Text("Terminal").tag("terminal")
                    }
                    LabeledContent("Base branch") {
                        TextField("the remote's default", text: text(\.base))
                            .multilineTextAlignment(.trailing)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                    }
                    Picker("Network policy", selection: Binding(get: { draft.policy ?? "" },
                                                                set: { draft.policy = $0.isEmpty ? nil : $0 })) {
                        Text(options.map { "Default (\($0.defaultPolicy))" } ?? "Default").tag("")
                        ForEach(options?.policies ?? [], id: \.spec) { Text($0.spec).tag($0.spec) }
                        if let p = draft.policy, options?.policies.contains(where: { $0.spec == p }) == false {
                            Text(p).tag(p)
                        }
                    }
                } header: {
                    Text("New sessions")
                } footer: {
                    Text("A work branch is named \(draft.branchPrefix ?? view.defaultBranchPrefix)/<name>.")
                }
                Section {
                    Toggle("Pick for each repository", isOn: Binding(get: { draft.providers == nil },
                                                                     set: { draft.providers = $0 ? nil : [] }))
                    if let picked = draft.providers {
                        ForEach(options?.providers ?? [], id: \.name) { p in
                            Toggle(isOn: Binding(get: { picked.contains(p.name) },
                                                 set: { on in draft.providers = on ? (draft.providers ?? []) + [p.name]
                                                                                   : draft.providers?.filter { $0 != p.name } })) {
                                VStack(alignment: .leading) {
                                    Text(p.name)
                                    Text(p.kind).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                        if let e = options?.providersError { Text(e).foregroundStyle(.red).font(.caption) }
                    }
                } header: {
                    Text("Credentials")
                } footer: {
                    Text(draft.providers == nil ? "Each repository gets its own guess." : "Every new session gets these, whatever the repository.")
                }
                Section {
                    Toggle("Update itself", isOn: Binding(get: { draft.autoUpdate ?? true }, set: { draft.autoUpdate = $0 }))
                } header: {
                    Text("This server")
                } footer: {
                    Text("Written to \(view.path)\(view.present ? "" : ", which doesn't exist yet"). Save writes these; the name and notifications above apply at once.")
                }
    }

    /// A text field over an optional string: empty is nil, the default.
    private func text(_ key: WritableKeyPath<ServerSettings, String?>) -> Binding<String> {
        Binding(get: { draft[keyPath: key] ?? "" }, set: { draft[keyPath: key] = $0.isEmpty ? nil : $0 })
    }

    private func load() async {
        do {
            async let o = server.api.newOptions()
            let v = try await server.api.settings()
            view = v
            draft = v.settings
            options = try? await o
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func save() {
        saving = true
        Task {
            defer { saving = false }
            do {
                let v = try await server.api.setSettings(draft)
                view = v
                draft = v.settings
            } catch {
                failed = error.localizedDescription
            }
        }
    }
}

// MARK: About

/// What this app is, and what the server runs.
struct AboutView: View {
    let server: Server?
    @State private var hello: Hello?
    @State private var error: String?

    private var appVersion: String {
        let info = Bundle.main.infoDictionary
        return "\(info?["CFBundleShortVersionString"] as? String ?? "?") (\(info?["CFBundleVersion"] as? String ?? "?"))"
    }

    var body: some View {
        List {
            Section("This app") {
                LabeledContent("hura for iOS", value: appVersion)
                Text("Updates come from TestFlight.").font(.caption).foregroundStyle(.secondary)
            }
            if let server {
                Section(server.record.name) {
                    if let hello {
                        LabeledContent("hurad", value: hello.version)
                        LabeledContent("Protocol", value: "\(hello.protocol)")
                    } else if let error {
                        Text(error).foregroundStyle(.red)
                    } else {
                        ProgressView()
                    }
                    LabeledContent("Address") { Text("\(server.record.host):\(server.record.port)").font(.footnote.monospaced()) }
                    LabeledContent("Certificate") {
                        Text(server.record.fingerprint.prefix(16) + "…").font(.footnote.monospaced())
                    }
                }
                Section {
                } footer: {
                    Text("`hurad update` on the server installs a newer release, and `hurad doctor` says what is wrong with one.")
                }
            }
        }
        .navigationTitle("About")
        .task {
            guard let server else { return }
            do { hello = try await server.api.hello() } catch { self.error = error.localizedDescription }
        }
    }
}

/// A server's name on this phone, saved when editing ends.
struct ServerNameField: View {
    let server: Server
    @Environment(AppModel.self) private var model
    @State private var name = ""
    @State private var failed: String?
    @FocusState private var editing: Bool

    var body: some View {
        LabeledContent("Name") {
            TextField(server.record.host, text: $name)
                .multilineTextAlignment(.trailing)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .focused($editing)
                .onSubmit(save)
        }
        .onAppear { name = server.record.name }
        .onChange(of: editing) { _, now in if !now { save() } }
        .failure($failed)
    }

    private func save() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        // An empty name goes back to what it was rather than to nothing.
        guard !trimmed.isEmpty else {
            name = server.record.name
            return
        }
        do { try model.rename(server, to: trimmed) } catch { failed = error.localizedDescription }
    }
}
