import HuraKit
import SwiftUI

/// A Jira ticket's fields, as the controls Jira says this account may use,
/// saved together. Only what changed is sent.
struct TicketEditView: View {
    let tracker: StoredTracker
    let key: String
    @Environment(\.dismiss) private var dismiss
    @State private var form: EditForm?
    @State private var edits: [String: Editable] = [:]
    @State private var error: String?
    @State private var saving = false

    private var changed: [EditField] {
        (form?.fields ?? []).filter { f in edits[f.id].map { $0 != f.edit } ?? false }
    }

    var body: some View {
        NavigationStack {
            Loaded(value: form, error: form == nil ? error : nil) { form in
                Form {
                    if let error { Section { Text(error).foregroundStyle(.red).font(.callout) } }
                    ForEach(form.fields) { f in
                        Section {
                            control(f)
                        } header: {
                            Text(f.name + (f.required ? " *" : ""))
                        }
                    }
                }
            }
            .navigationTitle("Edit \(key)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if saving { ProgressView() } else { Button("Save", action: save).disabled(changed.isEmpty) }
                }
            }
            .interactiveDismissDisabled(!changed.isEmpty)
            .task {
                do { form = try await TrackerCore.editForm(tracker, key: key) } catch { self.error = error.localizedDescription }
            }
        }
    }

    private func current(_ f: EditField) -> Editable { edits[f.id] ?? f.edit }

    private func set(_ f: EditField, _ v: Editable) { edits[f.id] = v }

    @ViewBuilder
    private func control(_ f: EditField) -> some View {
        switch current(f) {
        case let .text(v):
            TextField(f.name, text: Binding(get: { v }, set: { set(f, .text($0)) }), axis: .vertical)
        case let .doc(v):
            TextField("Markdown", text: Binding(get: { v.text }, set: { set(f, .doc(.init(text: $0, lost: v.lost))) }),
                      axis: .vertical)
                .lineLimit(f.id == "description" ? 6...20 : 3...10)
            if !v.lost.isEmpty {
                Text("Saving loses what Markdown can't say: \(v.lost.joined(separator: ", ")).").font(.caption).foregroundStyle(.orange)
            }
        case let .number(v):
            TextField(f.name, text: Binding(get: { v.map { $0.formatted() } ?? "" },
                                            set: { set(f, .number(Double($0.replacingOccurrences(of: ",", with: ".")))) }))
                .keyboardType(.decimalPad)
        case let .date(v):
            Toggle("Set", isOn: Binding(get: { v != nil }, set: { on in set(f, .date(on ? day(Date()) : nil)) }))
            if let v, let d = dayDate(v) {
                DatePicker(f.name, selection: Binding(get: { d }, set: { set(f, .date(day($0))) }), displayedComponents: .date)
            }
        case let .select(v, options):
            Picker(f.name, selection: Binding(get: { v ?? "" }, set: { set(f, .select($0.isEmpty ? nil : $0, options: options)) })) {
                if !f.required || v == nil { Text("None").tag("") }
                ForEach(options) { Text($0.name).tag($0.id) }
            }
        case let .multi(v, options):
            ForEach(options) { o in
                Toggle(o.name, isOn: Binding(get: { v.contains(o.id) },
                                             set: { on in set(f, .multi(on ? v + [o.id] : v.filter { $0 != o.id }, options: options)) }))
            }
        case let .labels(v):
            TextField("comma, separated", text: Binding(
                get: { v.joined(separator: ", ") },
                set: { set(f, .labels($0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })) }))
                .textInputAutocapitalization(.never)
        case let .user(v, assignable):
            UserPicker(tracker: tracker, key: key, assignable: assignable, value: v) { set(f, .user($0, assignable: assignable)) }
        case let .readOnly(v):
            Text(v).foregroundStyle(.secondary)
        }
    }

    private func day(_ d: Date) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: d)
    }

    private func dayDate(_ s: String) -> Date? {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f.date(from: String(s.prefix(10)))
    }

    private func save() {
        saving = true
        error = nil
        let changes = changed.map { FieldChange(id: $0.id, edit: edits[$0.id]!) }
        Task {
            defer { saving = false }
            do {
                try await TrackerCore.save(tracker, key: key, changes: changes)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// A person, searched for as it is typed.
private struct UserPicker: View {
    let tracker: StoredTracker
    let key: String
    let assignable: Bool
    let value: UserChoice?
    let pick: (UserChoice?) -> Void
    @State private var query = ""
    @State private var found: [UserChoice] = []

    var body: some View {
        HStack {
            Text(value?.name ?? "Nobody").foregroundStyle(value == nil ? .secondary : .primary)
            Spacer()
            if value != nil { Button("Clear") { pick(nil) }.font(.caption) }
        }
        TextField("Search people", text: $query)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        ForEach(found, id: \.accountID) { u in
            Button(u.name) {
                pick(u)
                query = ""
                found = []
            }
        }
        .task(id: query) {
            let q = query.trimmingCharacters(in: .whitespaces)
            guard q.count >= 2 else { found = []; return }
            try? await Task.sleep(for: .milliseconds(300))
            found = (try? await TrackerCore.users(tracker, key: key, query: q, assignable: assignable)) ?? []
        }
    }
}
