import HuraKit
import SwiftUI

/// What a project's new worktrees open with, each the server's until set
/// here: the desktop's Settings, Projects (`ProjectDefaults` on the server,
/// so the phone and the desktop agree). The choices are the draft's, from the
/// same `new-options`, and "Server's" says what the server would pick.
///
/// Neither a sheet nor a page of its own: it has no navigation stack, so a
/// sheet puts it in one, and a push is already in one.
struct ProjectDefaultsView: View {
    let server: Server
    let project: Project
    @Environment(\.dismiss) private var dismiss

    @State private var options: NewOptions?
    @State private var defaults = ProjectDefaults()
    /// What the server has, to say whether there is anything to save.
    @State private var saved = ProjectDefaults()
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        Form {
            if let error {
                Section { Text(error).foregroundStyle(.bad).font(.callout) }
                    .listRowBackground(Color.surface)
            }
            if let options {
                rows(options)
            } else if error == nil {
                WaitingRow().listRowBackground(Color.clear)
            }
        }
        .scrollContentBackground(.hidden)
        .background(Color.canvas.ignoresSafeArea())
        .navigationTitle(project.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                if busy {
                    ProgressView()
                } else {
                    Button("Save", action: save)
                        .disabled(options == nil || defaults == saved)
                }
            }
        }
        .task { await load() }
    }

    @ViewBuilder
    private func rows(_ options: NewOptions) -> some View {
        Section {
            Picker(selection: $defaults.interface) {
                Text("Server's (\(word(options.defaultInterface)))").tag(String?.none)
                Text("Chat").tag(String?.some("chat"))
                Text("Terminal").tag(String?.some("terminal"))
            } label: {
                Label("Agent", glyph: .agent)
            }
            if options.worktrees == true {
                Picker(selection: $defaults.worktree) {
                    Text("Server's (Sandbox)").tag(Bool?.none)
                    Text("Sandbox").tag(Bool?.some(false))
                    Text("Worktree").tag(Bool?.some(true))
                } label: {
                    Label("Runs in", glyph: defaults.worktree == true ? .unsandboxed : .policy)
                }
            }
            Picker(selection: $defaults.policy) {
                Text("Server's (\(options.defaultPolicy))").tag(String?.none)
                ForEach(options.policies, id: \.spec) { Text($0.spec).tag(String?.some($0.spec)) }
                // One set on the desktop, or before the config file lost it:
                // still shown, so saving here does not quietly drop it.
                if let p = defaults.policy, !options.policies.contains(where: { $0.spec == p }) {
                    Text(p).tag(String?.some(p))
                }
            } label: {
                Label("Network policy", glyph: .policy)
            }
            LabeledContent {
                TextField("The checkout's", text: Binding(
                    get: { defaults.base ?? "" },
                    set: { defaults.base = $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }))
                    .multilineTextAlignment(.trailing)
                    .font(.body.monospaced())
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            } label: {
                Label("Base branch", glyph: .branch)
            }
        } header: {
            Text("A new worktree")
        } footer: {
            Text("Each is the server's until set here. A blank base branch is the one the checkout on the server is on.")
        }
        .listRowBackground(Color.surface)

        if !options.toolchains.isEmpty {
            listSection("Toolchains", own: $defaults.toolchains, all: options.toolchains.map { ($0.name, $0.summary) },
                        serverWord: "The checkout's",
                        footer: "The checkout's are the ones its files point at, looked for when a worktree is made.")
        }
        listSection("Credentials", own: $defaults.providers, all: options.providers.map { ($0.name, $0.kind) },
                    serverWord: "The server's (\(options.defaultProviders.isEmpty ? "none" : options.defaultProviders.joined(separator: ", ")))",
                    footer: "The server's are the ones its config ticks, or the ones the checkout names.")

        if !defaults.isEmpty {
            Section {
                Button("Use the Server's for All", role: .destructive) { defaults = ProjectDefaults() }
            }
            .listRowBackground(Color.surface)
        }
    }

    /// A list that is the server's until one is ticked off it: then this
    /// project's, from the ticks.
    private func listSection(_ title: String, own: Binding<[String]?>, all: [(String, String)],
                             serverWord: String, footer: String) -> some View {
        Section {
            Toggle(serverWord, isOn: Binding(
                get: { own.wrappedValue == nil },
                set: { own.wrappedValue = $0 ? nil : [] }))
            if let chosen = own.wrappedValue {
                ForEach(all, id: \.0) { name, note in
                    Toggle(isOn: Binding(
                        get: { chosen.contains(name) },
                        set: { on in
                            // In the server's order, so the same choice is
                            // always the same list.
                            let next = on ? chosen + [name] : chosen.filter { $0 != name }
                            own.wrappedValue = all.map(\.0).filter(next.contains)
                        })) {
                        VStack(alignment: .leading) {
                            Text(name)
                            Text(note).font(.caption).foregroundStyle(.dim)
                        }
                    }
                }
                if all.isEmpty { Text("None configured").foregroundStyle(.dim) }
            }
        } header: {
            Text(title)
        } footer: {
            Text(footer)
        }
        .listRowBackground(Color.surface)
    }

    private func word(_ interface: String) -> String { interface == "terminal" ? "Terminal" : "Chat" }

    private func load() async {
        // The server's latest, since the project handed in may be from
        // before the desktop changed it.
        saved = (server.projects.first { $0.name == project.name } ?? project).defaults
        defaults = saved
        do {
            options = try await server.api.newOptions()
        } catch {
            self.error = error.userMessage
        }
    }

    private func save() {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await server.setProjectDefaults(project.name, defaults)
                dismiss()
            } catch {
                self.error = error.userMessage
            }
        }
    }
}
