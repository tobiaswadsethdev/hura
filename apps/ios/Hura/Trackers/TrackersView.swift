import HuraKit
import SwiftUI

/// The trackers this phone reads: added with a token, given a new token, or
/// forgotten. Kept in this phone's Keychain and sent nowhere but the tracker.
struct TrackersView: View {
    @Environment(TrackersModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var adding = false
    @State private var tokenFor: String?
    @State private var failed: String?

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if model.trackers.isEmpty {
                        Text("None yet").foregroundStyle(.secondary)
                    }
                    ForEach(model.trackers, id: \.source.name) { t in
                        Button { tokenFor = t.source.name } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(t.source.name)
                                    Text(where_(t.source)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                                Spacer()
                                Text(t.hasToken ? "token stored" : "NO token")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(t.hasToken ? .green : .red)
                            }
                        }
                        .tint(.primary)
                        .swipeActions {
                            Button("Forget", role: .destructive) {
                                do { try model.forget(t.source.name) } catch { failed = error.localizedDescription }
                            }
                        }
                    }
                    Button("Add a Tracker", systemImage: "plus") { adding = true }
                } footer: {
                    Text("Tickets assigned to you or created by you, and pull requests you opened or that wait on you. Read from this phone with its own tokens, which stay in its Keychain: no server holds them.")
                }
            }
            .navigationTitle("Trackers")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .sheet(isPresented: $adding) { AddTrackerView() }
            .sheet(item: Binding(get: { tokenFor.map(Named.init) }, set: { tokenFor = $0?.name })) { n in
                TokenSheet(name: n.name)
            }
            .failure($failed)
        }
    }

    private struct Named: Identifiable {
        var name: String
        var id: String { name }
    }

    private func where_(_ s: TrackerSource) -> String {
        switch s.kind {
        case "jira": "Jira · \(s.site ?? "") · \(s.email ?? "")"
        case "azure-dev-ops": "Azure DevOps · \(s.org ?? "")/\(s.project ?? "")"
        default: "GitHub · \(s.repo ?? "every repository the token sees")"
        }
    }
}

/// A new tracker: its kind, what that kind needs, and the token.
struct AddTrackerView: View {
    @Environment(TrackersModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var kind = "git-hub"
    @State private var name = ""
    @State private var repo = ""
    @State private var org = ""
    @State private var project = ""
    @State private var site = ""
    @State private var email = ""
    @State private var token = ""
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Picker("Kind", selection: $kind) {
                    Text("GitHub").tag("git-hub")
                    Text("Azure DevOps").tag("azure-dev-ops")
                    Text("Jira").tag("jira")
                }
                Section {
                    switch kind {
                    case "jira":
                        field("Site", "https://your-org.atlassian.net", $site, url: true)
                        field("Email", "you@example.com", $email, email: true)
                    case "azure-dev-ops":
                        field("Organisation", "your-org", $org)
                        field("Project", "YourProject", $project)
                    default:
                        field("Repository", "owner/name, or blank for all", $repo)
                    }
                    field("Name", kindLabel, $name)
                } footer: {
                    Text(kind == "jira" ? "Jira Cloud signs in with your email and an API token."
                         : kind == "azure-dev-ops" ? "A personal access token with Work Items and Code read and write."
                         : "A token that can read your repositories' issues and pull requests.")
                }
                Section("Token") {
                    SecureField("Paste the token", text: $token)
                }
                if let error {
                    Section { Text(error).foregroundStyle(.red).font(.callout) }
                }
            }
            .navigationTitle("Add a Tracker")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Add", action: add) }
                }
            }
        }
    }

    private var kindLabel: String {
        TrackerSource(kind: kind).kindLabel
    }

    private func field(_ label: String, _ placeholder: String, _ text: Binding<String>, url: Bool = false,
                       email: Bool = false) -> some View {
        LabeledContent(label) {
            TextField(placeholder, text: text)
                .multilineTextAlignment(.trailing)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(url ? .URL : email ? .emailAddress : .default)
        }
    }

    private func add() {
        let blank = { (s: String) -> String? in
            let t = s.trimmingCharacters(in: .whitespaces)
            return t.isEmpty ? nil : t
        }
        let source = TrackerSource(kind: kind, name: name,
                                   repo: kind == "git-hub" ? blank(repo) : nil,
                                   org: kind == "azure-dev-ops" ? blank(org) : nil,
                                   project: kind == "azure-dev-ops" ? blank(project) : nil,
                                   site: kind == "jira" ? blank(site) : nil,
                                   email: kind == "jira" ? blank(email) : nil)
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await model.add(source, token: token)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// A tracker's token, replaced.
private struct TokenSheet: View {
    let name: String
    @Environment(TrackersModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var token = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    SecureField("Paste the new token", text: $token)
                } footer: {
                    Text(error ?? "It replaces the one this phone has for \(name).")
                        .foregroundStyle(error == nil ? Color.secondary : Color.red)
                }
            }
            .navigationTitle(name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Store") {
                        Task {
                            do {
                                try await model.setToken(name, token)
                                dismiss()
                            } catch {
                                self.error = error.localizedDescription
                            }
                        }
                    }
                    .disabled(token.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .presentationDetents([.medium])
    }
}
