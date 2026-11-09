import AuthenticationServices
import HuraKit
import SwiftUI

/// The account's trackers: connected or added with a token, changed (its
/// token, its filters, whether it shows what you created), or forgotten.
/// Kept in the hura account and the same on every device.
struct TrackersView: View {
    @Environment(TrackersModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var adding = false
    @State private var editing: String?
    @State private var failed: String?
    @State private var connecting: String?
    @Environment(\.webAuthenticationSession) private var webAuthenticationSession

    var body: some View {
        NavigationStack {
            if model.signedIn {
                list
            } else {
                SignInForTrackers()
                    .navigationTitle("Trackers")
                    .navigationBarTitleDisplayMode(.inline)
            }
        }
    }

    private var list: some View {
        List {
            Section {
                if model.trackers.isEmpty {
                    Text("None yet").foregroundStyle(.secondary)
                }
                ForEach(model.trackers, id: \.source.name) { t in
                    Button { editing = t.source.name } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(t.source.name)
                                Text(where_(t.source)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                if !t.source.jqlFilters.isEmpty {
                                    Text(t.source.jqlFilters.map(\.name).joined(separator: " · "))
                                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                            Spacer()
                            if let who = t.connected {
                                Text(t.hasToken ? who : "connect again")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(t.hasToken ? .green : .red)
                            } else {
                                Text(t.hasToken ? "token stored" : "NO token")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(t.hasToken ? .green : .red)
                            }
                        }
                    }
                    .tint(.primary)
                    .swipeActions {
                        Button("Forget", role: .destructive) {
                            Task {
                                do { try await model.forget(t.source.name) } catch { failed = error.localizedDescription }
                            }
                        }
                    }
                }
                ForEach(["github", "jira"].filter(model.connectable.contains), id: \.self) { provider in
                    Button {
                        connect(provider)
                    } label: {
                        Label(provider == "github" ? "Connect GitHub" : "Connect Jira",
                              systemImage: "link")
                    }
                    .disabled(connecting != nil)
                }
                Button(model.connectable.isEmpty ? "Add a Tracker" : "Add with a Token",
                       systemImage: "plus") { adding = true }
            } footer: {
                Text("Tickets assigned to you or created by you, a Jira tracker's own JQL filters, and pull requests you opened or that wait on you. Kept in your hura account and the same on every device signed in to it; their tokens are kept there, encrypted, and never sent back.")
            }
            Section {
                Toggle("Notify When They Change", isOn: Binding(get: { model.notify }, set: { on in
                    Task {
                        do { try await model.setNotify(on) } catch { failed = error.localizedDescription }
                    }
                }))
            } footer: {
                Text("A ticket that changes status, gets a comment from somebody else, becomes yours or turns up in a filter, and a pull request that starts waiting on you or whose checks fail or review comes back. hura cloud reads your trackers every minute or two and tells this iPhone, whether or not hura is open.")
            }
        }
        .navigationTitle("Trackers")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
        }
        .sheet(isPresented: $adding) { AddTrackerView() }
        .sheet(item: Binding(get: { editing.flatMap(model.tracker(named:)).map(Named.init) },
                             set: { editing = $0?.tracker.source.name })) { n in
            TrackerSheet(tracker: n.tracker)
        }
        .failure($failed)
    }

    /// Signs in to the tracker in a browser sheet, and reads the trackers
    /// again once the service sends the browser back.
    private func connect(_ provider: String) {
        connecting = provider
        Task {
            defer { connecting = nil }
            do {
                let url = try await model.connectURL(provider)
                // Safari's session, so someone signed in to GitHub there is
                // not asked again.
                let back = try await webAuthenticationSession.authenticate(using: url, callbackURLScheme: "hura")
                if let e = URLComponents(url: back, resolvingAgainstBaseURL: false)?
                    .queryItems?.first(where: { $0.name == "error" })?.value {
                    failed = e
                }
                await model.refresh()
            } catch let e as ASWebAuthenticationSessionError where e.code == .canceledLogin {
                // Closed: nothing to say.
            } catch {
                failed = error.localizedDescription
            }
        }
    }

    private struct Named: Identifiable {
        var tracker: StoredTracker
        var id: String { tracker.source.name }
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

/// One tracker: what it shows beside what is assigned to you, and its token.
private struct TrackerSheet: View {
    let tracker: StoredTracker
    @Environment(TrackersModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var showCreated: Bool
    @State private var filters: [JqlFilter]
    @State private var token = ""
    @State private var busy = false
    @State private var error: String?

    init(tracker: StoredTracker) {
        self.tracker = tracker
        _showCreated = State(initialValue: !tracker.source.hideCreated)
        _filters = State(initialValue: tracker.source.jqlFilters)
    }

    private var name: String { tracker.source.name }
    private var trimmedToken: String { token.trimmingCharacters(in: .whitespaces) }
    private var edited: Bool { showCreated == tracker.source.hideCreated || filters != tracker.source.jqlFilters }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle("Tickets I Created", isOn: $showCreated)
                } footer: {
                    Text("Off leaves out the tickets you created that nobody has assigned you.")
                }
                if tracker.source.kind == "jira" {
                    Section {
                        ForEach($filters.indices, id: \.self) { i in
                            VStack(alignment: .leading, spacing: 6) {
                                TextField("Name", text: $filters[i].name)
                                TextField("JQL", text: $filters[i].jql, axis: .vertical)
                                    .font(.callout.monospaced())
                                    .lineLimit(1...4)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                            }
                        }
                        .onDelete { filters.remove(atOffsets: $0) }
                        Button("Add a Filter", systemImage: "plus") { filters.append(JqlFilter(name: "", jql: "")) }
                    } header: {
                        Text("Filters")
                    } footer: {
                        Text("Each is a section of its own in the inbox. The JQL is sent as written, so add statusCategory != Done if done tickets should stay out.")
                    }
                }
                if let who = tracker.connected {
                    Section {
                        LabeledContent("Connected as", value: who)
                    } footer: {
                        Text(tracker.hasToken
                             ? "Through its own sign-in, which hura cloud renews. Connect it again from the trackers list to sign in as someone else."
                             : "Its sign-in ran out or was taken back. Connect it again from the trackers list.")
                    }
                } else {
                    Section {
                        SecureField(tracker.hasToken ? "Paste a new token" : "Paste the token", text: $token)
                    } header: {
                        Text("Token")
                    } footer: {
                        Text(tracker.hasToken
                             ? "Leave it empty to keep the one your account has."
                             : "This tracker has no token yet.")
                    }
                }
                if let error {
                    Section { Text(error).foregroundStyle(.red).font(.callout) }
                }
            }
            .navigationTitle(name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else {
                        Button("Save", action: save).disabled(!edited && trimmedToken.isEmpty)
                    }
                }
            }
        }
    }

    private func save() {
        var source = tracker.source
        source.hideCreated = !showCreated
        source.jqlFilters = filters
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                if edited { try await model.edit(name, source) }
                if !trimmedToken.isEmpty { try await model.setToken(name, trimmedToken) }
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
