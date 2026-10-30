import HuraKit
import SwiftUI

/// The session's network policy: its own rules by host, the runtime's rules
/// for every sandbox (read-only), and the lists every new session starts
/// from. A deny outranks every allow.
struct PolicyPane: View {
    let server: Server
    let name: String
    @State private var view: PolicyView?
    @State private var error: String?
    @State private var failed: String?
    @State private var busy: String?
    @State private var blocking: String?
    @State private var removing: (rule: PolicyRule, others: [String])?

    var body: some View {
        Loaded(value: view, error: error) { view in
            List {
                Section {
                    LabeledContent("Template", value: view.template ?? "none recorded")
                    LabeledContent("Rules of its own", value: "\(view.rules.filter { !$0.global }.count)")
                }
                let own = byHost(view.rules.filter { !$0.global })
                Section("This session") {
                    if own.isEmpty {
                        Text("No rules of its own: nothing in this sandbox has egress.").foregroundStyle(.secondary)
                    }
                    ForEach(own, id: \.host) { group in hostRows(group.host, group.rules, readOnly: false) }
                }
                let global = byHost(view.rules.filter(\.global))
                if !global.isEmpty {
                    Section {
                        ForEach(global, id: \.host) { group in hostRows(group.host, group.rules, readOnly: true) }
                    } header: {
                        Text("Every sandbox")
                    } footer: {
                        Text("From the sandbox runtime's own policy; changed with `sbx policy`, not here.")
                    }
                }
                if let lists = view.lists { listsSection(lists) }
                Section {
                } footer: {
                    Text("Rules are for the whole sandbox, not for one program in it. A deny outranks every allow.")
                }
            }
        }
        .refreshable { await load() }
        .task { await load() }
        .confirmationDialog("Block \(blocking ?? "")?", isPresented: .constant(blocking != nil), titleVisibility: .visible,
                            presenting: blocking) { endpoint in
            Button("Block", role: .destructive) {
                blocking = nil
                change(endpoint) { try await server.api.block(name, endpoint: endpoint, everywhere: false) }
            }
            Button("Cancel", role: .cancel) { blocking = nil }
        } message: { endpoint in
            Text("\(endpoint) is denied to everything in this sandbox, whatever opens it, and anything that depends on it stops working.")
        }
        .confirmationDialog("Remove this rule?", isPresented: .constant(removing != nil), titleVisibility: .visible,
                            presenting: removing) { r in
            Button("Remove", role: .destructive) {
                removing = nil
                change(r.rule.id) { try await server.api.removeRule(name, id: r.rule.id) }
            }
            Button("Cancel", role: .cancel) { removing = nil }
        } message: { r in
            Text("It also names \(r.others.joined(separator: ", ")), which lose it too.")
        }
        .failure($failed)
    }

    @ViewBuilder
    private func hostRows(_ host: String, _ rules: [PolicyRule], readOnly: Bool) -> some View {
        let (h, port) = splitEndpoint(host)
        let endpoint = port.isEmpty || !host.contains(":") ? "\(host):443" : host
        let denied = rules.contains { !$0.allow && $0.methods.isEmpty }
        // A tap opens what can be done to the host and to each of its rules,
        // as right-clicking does on the desktop.
        Menu {
            if !readOnly, !denied {
                Button("Block…", systemImage: "xmark.shield", role: .destructive) { blocking = endpoint }
            }
            if !readOnly {
                ForEach(rules) { r in
                    Button("Remove \(ruleText(r))", systemImage: "minus.circle", role: .destructive) { remove(r, host: host) }
                }
            }
            Button("Copy Host", systemImage: "doc.on.doc") { UIPasteboard.general.string = host }
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                Text(host.contains(":") ? "\(h):\(port)" : host).font(.callout.monospaced().weight(.medium))
                ForEach(rules) { r in
                    HStack(spacing: 8) {
                        Text(r.methods.isEmpty ? "any" : r.methods.joined(separator: ","))
                            .font(.caption.monospaced().weight(.semibold))
                            .foregroundStyle(r.allow ? .green : .red)
                        Text(r.methods.isEmpty ? "every request" : (r.path ?? "/**"))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                        if !r.allow { Text("deny").font(.caption.weight(.bold)).foregroundStyle(.red) }
                        Spacer()
                        if busy == r.id { ProgressView() }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .padding(.vertical, 2)
        }
        .tint(.primary)
        .swipeActions {
            if !readOnly, rules.count == 1, let r = rules.first {
                Button("Remove", role: .destructive) { remove(r, host: host) }
            }
            if !readOnly, !denied {
                Button("Block") { blocking = endpoint }.tint(.red)
            }
        }
    }

    private func ruleText(_ r: PolicyRule) -> String {
        let what = r.methods.isEmpty ? "Every Request" : "\(r.methods.joined(separator: ",")) \(r.path ?? "/**")"
        return r.allow ? what : "Deny"
    }

    @ViewBuilder
    private func listsSection(_ lists: PolicyLists) -> some View {
        Section {
            if lists.allow.isEmpty && lists.routes.isEmpty && lists.block.isEmpty {
                Text("None yet. Allow or block from Events with “Every new session too” on.").foregroundStyle(.secondary)
            }
            ForEach(lists.allow, id: \.endpoint) { a in
                listed(a.endpoint, allow: true, here: a.inPolicy ? "in this session" : "not in this one")
            }
            ForEach(lists.routes, id: \.endpoint) { r in
                VStack(alignment: .leading, spacing: 2) {
                    listed(r.endpoint, allow: true, here: r.inPolicy ? "in this session" : "not in this one")
                    ForEach(r.routes, id: \.self) { route in
                        Text("\(route.method) \(route.path)").font(.caption2.monospaced()).foregroundStyle(.secondary)
                    }
                }
            }
            ForEach(lists.block, id: \.endpoint) { b in
                listed(b.endpoint, allow: false, here: b.inPolicy ? "denied here" : "not denied in this one")
            }
        } header: {
            Text("Every new session")
        } footer: {
            Text("Applied to every new session, so an entry may not be in this one.")
        }
    }

    private func listed(_ endpoint: String, allow: Bool, here: String) -> some View {
        Menu {
            Button("Remove from the List", systemImage: "minus.circle", role: .destructive) {
                change(endpoint) { try await server.api.unlist(name, endpoint: endpoint) }
            }
            Button("Copy Endpoint", systemImage: "doc.on.doc") { UIPasteboard.general.string = endpoint }
        } label: {
            HStack {
                Image(systemName: allow ? "checkmark.shield" : "xmark.shield").foregroundStyle(allow ? .green : .red)
                Text(endpoint).font(.callout.monospaced())
                Spacer()
                if busy == endpoint { ProgressView() } else { Text(here).font(.caption).foregroundStyle(.secondary) }
            }
            .contentShape(Rectangle())
        }
        .tint(.primary)
        .swipeActions {
            Button("Unlist", role: .destructive) {
                change(endpoint) { try await server.api.unlist(name, endpoint: endpoint) }
            }
        }
    }

    private func byHost(_ rules: [PolicyRule]) -> [(host: String, rules: [PolicyRule])] {
        var order: [String] = []
        var by: [String: [PolicyRule]] = [:]
        for r in rules {
            for h in r.hosts {
                if by[h] == nil { order.append(h) }
                by[h, default: []].append(r)
            }
        }
        return order.map { ($0, by[$0]!) }
    }

    private func remove(_ rule: PolicyRule, host: String) {
        let others = rule.hosts.filter { $0 != host }
        if others.isEmpty {
            change(rule.id) { try await server.api.removeRule(name, id: rule.id) }
        } else {
            removing = (rule, others)
        }
    }

    private func change(_ key: String, _ work: @escaping () async throws -> PolicyView) {
        busy = key
        Task {
            defer { busy = nil }
            do { view = try await work() } catch { failed = error.localizedDescription }
        }
    }

    private func load() async {
        do {
            view = try await server.api.policy(name)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}
