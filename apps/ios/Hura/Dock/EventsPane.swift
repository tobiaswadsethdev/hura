import HuraKit
import SwiftUI

/// What the sandbox runtime decided, grouped by the endpoint it was about,
/// with an allow or a block beside each: the evidence that the policy is
/// doing anything, and the place to change it from. Re-read every few
/// seconds, as the desktop's pane is.
struct EventsPane: View {
    enum Filter: String, CaseIterable { case all = "All", denied = "Denied", allowed = "Allowed" }
    enum Mode: String, CaseIterable { case endpoints = "Endpoints", log = "Log" }

    let server: Server
    let name: String
    @State private var feed: [PolicyEvent]?
    @State private var policy: PolicyView?
    @State private var error: String?
    @State private var failed: String?
    @State private var filter = Filter.all
    @State private var mode = Mode.endpoints
    @State private var asking: Asking?

    var body: some View {
        Loaded(value: feed, error: error) { feed in
            List {
                Section {
                    Picker("Show", selection: $mode) {
                        ForEach(Mode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    Picker("Filter", selection: $filter) {
                        ForEach(Filter.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }
                .listRowSeparator(.hidden)
                if feed.isEmpty {
                    Text("Nothing yet. Decisions show here as the sandbox makes them.").foregroundStyle(.secondary)
                } else if mode == .endpoints {
                    ForEach(groups(feed)) { g in endpointRow(g) }
                } else {
                    ForEach(Array(feed.filter(passes).enumerated()), id: \.offset) { _, e in
                        logRow(e)
                    }
                }
            }
            .listStyle(.plain)
        }
        .task {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(3))
            }
        }
        .refreshable { await load() }
        .sheet(item: $asking) { a in
            PolicyChangeSheet(asking: a, standing: standing(a.endpoint)) { everywhere, routes in
                let api = server.api
                switch a.kind {
                case .block: return try await api.block(name, endpoint: a.endpoint, everywhere: everywhere)
                case .allow:
                    if let routes { return try await api.allowPaths(name, endpoint: a.endpoint, routes: routes, everywhere: everywhere) }
                    return try await api.allow(name, endpoint: a.endpoint, everywhere: everywhere)
                }
            } onDone: { view in
                policy = view
                asking = nil
            }
        }
        .failure($failed)
    }

    // MARK: Rows

    private func endpointRow(_ g: EndpointGroup) -> some View {
        let s = standing(g.endpoint)
        let denied = g.events[0].verdict == "Denied" && !s.whole
        // A tap opens what can be done about it, as right-clicking the row
        // does on the desktop.
        return Menu {
            actions(g.endpoint, requests: g.requests, denied: g.denied > 0)
        } label: {
            endpointLabel(g, s: s, denied: denied)
        }
        .tint(.primary)
        .swipeActions {
            if g.denied > 0, !s.whole {
                Button("Allow") { asking = Asking(kind: .allow, endpoint: g.endpoint, requests: g.requests) }.tint(.green)
            }
            if s.open, !s.denied {
                Button("Block") { asking = Asking(kind: .block, endpoint: g.endpoint, requests: []) }.tint(.red)
            }
        }
    }

    private func endpointLabel(_ g: EndpointGroup, s: Standing, denied: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: denied ? "xmark.shield.fill" : "checkmark.shield.fill")
                    .foregroundStyle(denied ? .red : .green)
                Text(g.endpoint).font(.callout.monospaced()).lineLimit(1).truncationMode(.middle)
                Spacer()
                Text(ago(g.lastSeen)).font(.caption).foregroundStyle(.secondary)
            }
            HStack(spacing: 10) {
                if g.denied > 0 { Text("\(g.denied) denied").foregroundStyle(.red) }
                if g.allowed > 0 { Text("\(g.allowed) allowed").foregroundStyle(.green) }
                if let listed = s.listed { Text(listed == "allow" ? "allowed for new sessions" : "blocked for new sessions").foregroundStyle(.secondary) }
            }
            .font(.caption)
            ForEach(g.requests.prefix(3), id: \.self) { r in
                Text("\(r.method) \(r.path)").font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private func logRow(_ e: PolicyEvent) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(e.verdict == "Denied" ? "DENY" : e.verdict == "Allowed" ? "allow" : "-")
                    .font(.caption.monospaced().weight(.bold))
                    .foregroundStyle(e.verdict == "Denied" ? .red : e.verdict == "Allowed" ? .green : .secondary)
                Text(e.class).font(.caption.monospaced()).foregroundStyle(.secondary)
                if e.count > 1 { Text("×\(e.count)").font(.caption) }
                Spacer()
                Text(clock(e.at)).font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Text(e.subject).font(.caption.monospaced()).textSelection(.enabled)
            if let reason = e.reason { Text(reason).font(.caption2).foregroundStyle(.secondary) }
        }
        .contextMenu {
            if let endpoint = e.endpoint {
                actions(endpoint, requests: e.request.map { [$0] } ?? [], denied: e.verdict == "Denied")
            }
            Button("Copy Line", systemImage: "doc.on.doc") { UIPasteboard.general.string = e.subject }
        }
    }

    /// What the desktop's menu offers for an endpoint (`itemsFor` in
    /// Events.tsx). A denial by a rule's paths has no one-tap allow, since
    /// that would open the whole host.
    @ViewBuilder
    private func actions(_ endpoint: String, requests: [PathRoute], denied: Bool) -> some View {
        let s = standing(endpoint)
        if denied, !s.whole {
            if !requests.isEmpty {
                Button(requests.count == 1 ? "Allow This Path…" : "Allow These Paths…", systemImage: "checkmark.shield") {
                    asking = Asking(kind: .allow, endpoint: endpoint, requests: requests, scope: .paths)
                }
                Button("Allow the Whole Host…") { asking = Asking(kind: .allow, endpoint: endpoint, requests: requests, scope: .host) }
            } else {
                Button("Allow in This Session", systemImage: "checkmark.shield") { quick(endpoint, everywhere: false) }
                Button("Allow in Every New Session Too", systemImage: "checkmark.shield") { quick(endpoint, everywhere: true) }
                Button("Allow Only Some Paths…") { asking = Asking(kind: .allow, endpoint: endpoint, requests: [], scope: .paths) }
            }
        }
        if s.open, !s.denied {
            Button("Block…", systemImage: "xmark.shield", role: .destructive) {
                asking = Asking(kind: .block, endpoint: endpoint, requests: [])
            }
        }
        Button("Copy Endpoint", systemImage: "doc.on.doc") { UIPasteboard.general.string = endpoint }
    }

    private func quick(_ endpoint: String, everywhere: Bool) {
        Task {
            do { policy = try await server.api.allow(name, endpoint: endpoint, everywhere: everywhere) } catch {
                failed = error.localizedDescription
            }
        }
    }

    // MARK: Reading

    private func passes(_ e: PolicyEvent) -> Bool {
        switch filter {
        case .all: true
        case .denied: e.verdict == "Denied"
        case .allowed: e.verdict == "Allowed"
        }
    }

    struct EndpointGroup: Identifiable {
        var endpoint: String
        var events: [PolicyEvent] = []
        var denied: Int64 = 0
        var allowed: Int64 = 0
        var requests: [PathRoute] = []
        var id: String { endpoint }
        var lastSeen: Int64 { events.map { max($0.last, $0.at) }.max() ?? 0 }
    }

    private func groups(_ feed: [PolicyEvent]) -> [EndpointGroup] {
        var by: [String: EndpointGroup] = [:]
        for e in feed {
            guard let endpoint = e.endpoint else { continue }
            var g = by[endpoint] ?? EndpointGroup(endpoint: endpoint)
            g.events.append(e)
            if e.verdict == "Denied" {
                g.denied += e.count
                if let r = e.request, !g.requests.contains(r) { g.requests.append(r) }
            } else if e.verdict == "Allowed" {
                g.allowed += e.count
            }
            by[endpoint] = g
        }
        return by.values
            .filter { g in filter == .all || (filter == .denied ? g.denied > 0 : g.allowed > 0) }
            .sorted { $0.lastSeen > $1.lastSeen }
    }

    private func standing(_ endpoint: String) -> Standing {
        Standing(policy: policy, endpoint: endpoint)
    }

    private func load() async {
        do {
            async let p = server.api.policy(name)
            feed = try await server.api.events(name)
            if let p = try? await p { policy = p }
            error = nil
        } catch {
            if feed == nil { self.error = error.localizedDescription }
        }
    }
}

/// Where an endpoint stands in a session's policy (`standingOf` in Events.tsx).
struct Standing {
    /// Some allow reaches it.
    var open = false
    /// An allow reaches all of it, not only some paths.
    var whole = false
    /// A deny on the whole host.
    var denied = false
    /// `allow` or `block` on the lists new sessions start from.
    var listed: String?

    init(policy: PolicyView?, endpoint: String) {
        let rules = (policy?.rules ?? []).filter { $0.hosts.contains { hostMatches($0, endpoint) } }
        denied = rules.contains { !$0.allow && $0.methods.isEmpty }
        open = !denied && rules.contains(where: \.allow)
        whole = !denied && rules.contains { $0.allow && $0.methods.isEmpty }
        if let lists = policy?.lists {
            if lists.allow.contains(where: { $0.endpoint == endpoint }) || lists.routes.contains(where: { $0.endpoint == endpoint }) {
                listed = "allow"
            } else if lists.block.contains(where: { $0.endpoint == endpoint }) {
                listed = "block"
            }
        }
    }
}

/// Whether a rule's host pattern covers an endpoint: `**`, `**.example.com`,
/// `*.example.com` (one label) or the host itself, with the port when the
/// pattern has one.
func hostMatches(_ resource: String, _ endpoint: String) -> Bool {
    let (host, port) = splitEndpoint(endpoint)
    var pattern = resource.lowercased()
    if let m = pattern.wholeMatch(of: /(.*):(\d+)/) {
        if String(m.2) != port { return false }
        pattern = String(m.1)
    }
    let h = host.lowercased()
    if pattern == "**" { return true }
    if pattern.hasPrefix("**.") { return h.hasSuffix(String(pattern.dropFirst(2))) }
    if pattern.hasPrefix("*.") {
        let suffix = String(pattern.dropFirst(1))
        guard h.hasSuffix(suffix) else { return false }
        let label = h.dropLast(suffix.count)
        return !label.isEmpty && !label.contains(".")
    }
    return h == pattern
}

func splitEndpoint(_ endpoint: String) -> (String, String) {
    guard let i = endpoint.lastIndex(of: ":") else { return (endpoint, "") }
    return (String(endpoint[..<i]), String(endpoint[endpoint.index(after: i)...]))
}

func clock(_ at: Int64) -> String {
    let s = Int(at % 86_400)
    return String(format: "%02d:%02d:%02d", s / 3600, s / 60 % 60, s % 60)
}

func ago(_ at: Int64) -> String {
    let s = max(0, Int(Date().timeIntervalSince1970) - Int(at))
    if s < 60 { return "\(s)s" }
    if s < 3600 { return "\(s / 60)m" }
    if s < 86400 { return "\(s / 3600)h" }
    return "\(s / 86400)d"
}

/// An allow or a block about to be made.
struct Asking: Identifiable {
    enum Kind { case allow, block }
    enum Scope { case host, paths }
    var kind: Kind
    var endpoint: String
    var requests: [PathRoute]
    var scope: Scope?
    var id: String { "\(kind)\(endpoint)" }
}

/// The question an allow or a block asks: how much of the endpoint, and
/// whether every new session gets it too.
struct PolicyChangeSheet: View {
    let asking: Asking
    let standing: Standing
    let run: (_ everywhere: Bool, _ routes: [PathRoute]?) async throws -> PolicyView
    let onDone: (PolicyView) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var scope: Asking.Scope = .host
    @State private var routes: [PathRoute] = []
    @State private var everywhere = false
    @State private var busy = false
    @State private var error: String?

    static let methods = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "ANY"]

    private var narrowing: Bool { asking.kind == .allow && scope == .paths }
    private var problems: [String?] { routes.map { pathProblem($0.path) } }
    private var ready: Bool { !narrowing || problems.allSatisfy { $0 == nil } }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(asking.endpoint).font(.body.monospaced())
                } header: {
                    Text(asking.kind == .allow ? "Allow" : "Block")
                }
                if asking.kind == .allow {
                    Section {
                        Picker("How much", selection: $scope) {
                            Text("The whole host").tag(Asking.Scope.host)
                            Text("Only these paths").tag(Asking.Scope.paths)
                        }
                        .pickerStyle(.segmented)
                        if narrowing {
                            ForEach(routes.indices, id: \.self) { i in
                                VStack(alignment: .leading) {
                                    HStack {
                                        Picker("Method", selection: $routes[i].method) {
                                            ForEach(Self.methods, id: \.self) { Text($0).tag($0) }
                                        }
                                        .labelsHidden()
                                        .fixedSize()
                                        TextField("/feed/v3/**", text: Binding(
                                            get: { routes[i].path },
                                            set: { routes[i].path = asPath($0, host: splitEndpoint(asking.endpoint).0) }))
                                            .font(.callout.monospaced())
                                            .textInputAutocapitalization(.never)
                                            .autocorrectionDisabled()
                                    }
                                    if let p = problems[i], !p.isEmpty {
                                        Text(p).font(.caption).foregroundStyle(.red)
                                    }
                                }
                                .swipeActions {
                                    if routes.count > 1 {
                                        Button("Remove", role: .destructive) { routes.remove(at: i) }
                                    }
                                }
                            }
                            Button("Another Path", systemImage: "plus") { routes.append(PathRoute(method: "GET", path: "")) }
                        }
                    } footer: {
                        if narrowing {
                            Text("Anything else on the host stays denied and turns up in Events with its path. * is one segment, ** any number.")
                        } else {
                            Text("Every request, from anything in this sandbox.\(standing.denied ? " The deny on it here is lifted first." : "")")
                        }
                    }
                } else {
                    Section {
                        Text("Denied to everything in this sandbox, whatever opens it. Anything that depends on it stops working.")
                            .font(.callout)
                    }
                }
                Section {
                    Toggle("Every new session too", isOn: $everywhere)
                }
                if let error {
                    Section { Text(error).foregroundStyle(.red).font(.callout) }
                }
            }
            .navigationTitle(asking.kind == .allow ? "Allow" : "Block")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy {
                        ProgressView()
                    } else {
                        Button(asking.kind == .allow ? "Allow" : "Block", role: asking.kind == .block ? .destructive : nil, action: go)
                            .disabled(!ready)
                    }
                }
            }
            .onAppear {
                scope = asking.scope ?? (asking.requests.isEmpty ? .host : .paths)
                routes = asking.requests.isEmpty ? [PathRoute(method: "GET", path: "")] : asking.requests
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func go() {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                var wanted: [PathRoute]?
                if narrowing {
                    var unique: [PathRoute] = []
                    for r in routes.map({ PathRoute(method: $0.method, path: $0.path.trimmingCharacters(in: .whitespaces)) })
                        where !unique.contains(r) { unique.append(r) }
                    wanted = unique
                }
                onDone(try await run(everywhere, wanted))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// Why a path can't be allowed, before a round trip says so.
func pathProblem(_ path: String) -> String? {
    let p = path.trimmingCharacters(in: .whitespaces)
    if p.isEmpty { return "" }
    if !p.hasPrefix("/") { return "A path starts with /" }
    if p.contains(where: { "?#%".contains($0) }) { return "The path alone: no query, fragment or %-escapes" }
    if p.split(separator: "/").contains("..") { return "No .. in a path" }
    if p.contains(where: \.isWhitespace) { return "No spaces" }
    return nil
}

/// `https://host/a/b` or `host/a/b` as `/a/b`, when the host is this one.
func asPath(_ text: String, host: String) -> String {
    guard let m = text.wholeMatch(of: /\s*(?:[hH][tT][tT][pP][sS]?:\/\/)?([^\/\s:]+)(?::\d+)?(\/\S*)?\s*/),
          m.1.lowercased() == host.lowercased() else { return text }
    return m.2.map(String.init) ?? "/"
}
