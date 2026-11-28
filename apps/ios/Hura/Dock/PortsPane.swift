import HuraKit
import SafariServices
import SwiftUI

/// What the sandbox listens on, who holds each port, and previews: a port
/// the server shares at an address of its own, opened in Safari ("Shared
/// previews" in docs/server.md). Safari keeps it with hura behind it, which a
/// preview on this phone's own loopback could not: iOS stops an app's
/// listeners soon after it leaves the screen. That one is still here, in the
/// in-app browser, for a server too old to share and from a row's menu. Also
/// the sandbox's processes, which can be stopped. The desktop's ports pane.
struct PortsPane: View {
    let server: Server
    let name: String
    @State private var view: PortsView?
    @State private var listening: [Poll.Listening] = []
    @State private var error: String?
    @State private var failed: String?
    @State private var busy: Int?
    @State private var browsing: URL?
    @State private var typed = ""
    @State private var killing: Row?
    @State private var killingProcess: PortProcess?
    @Environment(\.openURL) private var openURL

    struct Row: Identifiable {
        var port: Int
        var host: String
        var listening = false
        var owner: PortOwner?
        var forward: PortForward?
        var preview: PreviewHub.Preview?
        var id: Int { port }
    }

    private var rows: [Row] {
        var by: [Int: Row] = [:]
        for l in listening { by[l.port, default: Row(port: l.port, host: l.host)].listening = true }
        for l in view?.listening ?? [] {
            by[l.port, default: Row(port: l.port, host: l.host)].listening = true
            by[l.port]?.owner = l.owner
        }
        for f in view?.forwards ?? [] { by[f.port, default: Row(port: f.port, host: f.host)].forward = f }
        for p in server.previews.previews where p.session == name {
            by[p.port, default: Row(port: p.port, host: "v4")].preview = p
        }
        return by.values.sorted { $0.port < $1.port }
    }

    var body: some View {
        List {
            Section {
                if rows.isEmpty {
                    Text("Nothing listening").foregroundStyle(.secondary)
                }
                ForEach(rows) { row in portRow(row) }
            } header: {
                Text("Ports")
            } footer: {
                Text("A preview opens in Safari, at an address the server shares for it behind a private link. Safari warns about the server's certificate the first time: it is the one this phone pairs with. Hold a port to open it in hura instead.")
            }
            Section {
                HStack {
                    TextField("Another port", text: $typed)
                        .keyboardType(.numberPad)
                    Button("Preview") {
                        if let port = Int(typed), (1..<65536).contains(port) { open(port, host: "v4") }
                    }
                    .disabled(Int(typed).map { !(1..<65536).contains($0) } ?? true)
                }
            } footer: {
                Text("For a server that hasn't shown up above, or listens somewhere this can't see.")
            }
            if let processes = view?.processes, !processes.isEmpty {
                Section("Processes") {
                    ForEach(processes) { p in
                        HStack {
                            Text(String(p.pid)).font(.caption.monospaced()).foregroundStyle(.secondary).frame(width: 52, alignment: .leading)
                            Text(short(p.command)).font(.callout.monospaced()).lineLimit(1)
                            Spacer()
                            if p.protected {
                                Text("agent").font(.caption).foregroundStyle(.secondary)
                            } else if busy == -p.pid {
                                Spinner().foregroundStyle(Color.dim)
                            }
                        }
                        .swipeActions {
                            if !p.protected {
                                Button("Stop", role: .destructive) { killingProcess = p }
                            }
                        }
                    }
                }
            }
            if let error {
                Section { Text(error).foregroundStyle(Color.bad).font(.callout) }
            }
        }
        .task {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(5))
            }
        }
        .refreshable { await load() }
        .sheet(item: $browsing) { url in
            SafariView(url: url).ignoresSafeArea()
        }
        .confirmationDialog("Stop the process on \(killing?.port ?? 0)?", isPresented: .constant(killing != nil),
                            titleVisibility: .visible, presenting: killing) { row in
            Button("Stop It", role: .destructive) {
                killing = nil
                act(row.port) { try await server.api.killPort(name, port: row.port) }
            }
            Button("Cancel", role: .cancel) { killing = nil }
        } message: { row in
            Text("\(row.owner?.command ?? "The process") is sent TERM, then KILL if it's still there two seconds later.\(row.owner?.certain == false ? " It was picked because its command line names \(row.port)." : "") If the agent started it, the agent will see it exit.")
        }
        .confirmationDialog("Stop \(killingProcess.map { short($0.command) } ?? "")?", isPresented: .constant(killingProcess != nil),
                            titleVisibility: .visible, presenting: killingProcess) { p in
            Button("Stop It", role: .destructive) {
                killingProcess = nil
                act(-p.pid) { try await server.api.killProcess(name, pid: p.pid) }
            }
            Button("Cancel", role: .cancel) { killingProcess = nil }
        } message: { p in
            Text("\(p.command) (pid \(p.pid)) is sent TERM, then KILL if it's still there two seconds later.")
        }
        .failure($failed)
    }

    private func portRow(_ row: Row) -> some View {
        let live = row.preview != nil || row.forward != nil
        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(glyph: .ports).foregroundStyle(live ? Color.ok : .secondary)
                // Not "\(row.port)", which a Text formats as a number: 5 173.
                Text(String(row.port)).font(.body.monospaced().weight(.medium))
                Text(!row.listening ? "not listening" : row.owner.map { short($0.command) } ?? "")
                    .font(.caption.monospaced())
                    .foregroundStyle(row.owner?.certain == false ? .tertiary : .secondary)
                    .lineLimit(1)
                Spacer()
                if busy == row.port {
                    Spinner().foregroundStyle(Color.dim)
                } else {
                    Button {
                        open(row.port, host: row.host)
                    } label: {
                        Label(row.preview == nil ? "Preview" : "Open", systemImage: "safari")
                            .labelStyle(.titleAndIcon)
                            .lineLimit(1)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .fixedSize()
                    .layoutPriority(1)
                }
            }
            if let p = row.preview {
                Text("localhost:\(String(p.local)) on this phone · \(p.connections) open").font(.caption).foregroundStyle(.secondary)
            } else if let f = row.forward {
                Text("forwarded for \(f.connections) connection\(f.connections == 1 ? "" : "s")").font(.caption).foregroundStyle(.secondary)
            }
        }
        .contextMenu {
            Button("Open in hura", systemImage: "safari") { openHere(row.port, host: row.host) }
            if let p = row.preview {
                Button("Copy Address", systemImage: "doc.on.doc") { UIPasteboard.general.string = p.url.absoluteString }
            }
            if live {
                Button("Stop Forwarding", glyph: .stopForwarding) { stopPreview(row.port) }
            }
            if row.owner != nil {
                Button("Stop the Process…", glyph: .kill, role: .destructive) { killing = row }
            }
        }
        .swipeActions {
            if live { Button("Stop") { stopPreview(row.port) }.tint(Color.warn) }
            if row.owner != nil { Button("Kill", role: .destructive) { killing = row } }
        }
    }

    /// In Safari, shared by the server; in hura's own browser for a server
    /// too old to share.
    private func open(_ port: Int, host: String) {
        busy = port
        Task {
            defer { busy = nil }
            do {
                let share = try await server.api.sharePreview(name, port: port, host: host)
                if let url = share.url(host: server.record.host) {
                    openURL(url)
                    return
                }
            } catch let e where e.isUnsupported {
                // Falls through to this phone's own preview.
            } catch {
                failed = error.userMessage
                return
            }
            await here(port, host: host)
        }
    }

    /// On this phone's loopback, in the in-app browser, while hura is in front.
    private func openHere(_ port: Int, host: String) {
        busy = port
        Task {
            defer { busy = nil }
            await here(port, host: host)
        }
    }

    private func here(_ port: Int, host: String) async {
        do {
            let p = try await server.previews.open(session: name, port: port, host: host)
            browsing = p.url
        } catch {
            failed = error.userMessage
        }
    }

    private func stopPreview(_ port: Int) {
        server.previews.stop(session: name, port: port)
        act(port) { try await server.api.stopForward(name, port: port) }
    }

    private func act(_ key: Int, _ work: @escaping () async throws -> PortsView) {
        busy = key
        Task {
            defer { busy = nil }
            do { view = try await work() } catch { failed = error.userMessage }
        }
    }

    private func load() async {
        async let poll = try? server.api.poll(name)
        do {
            view = try await server.api.ports(name)
            error = nil
        } catch {
            self.error = error.userMessage
        }
        if let p = await poll { listening = p.ports ?? [] }
    }
}

/// A command line cut to what identifies it: `node vite`, `python3 http.server`.
func short(_ command: String) -> String {
    let words = command.split(whereSeparator: \.isWhitespace).map(String.init)
    guard let first = words.first else { return "" }
    let base = { (w: String) in w.split(separator: "/").last.map(String.init) ?? w }
    if let arg = words.dropFirst().first(where: { !$0.hasPrefix("-") }) { return "\(base(first)) \(base(arg))" }
    return base(first)
}

extension URL: @retroactive Identifiable {
    public var id: String { absoluteString }
}

/// Safari's in-app browser, for a preview.
struct SafariView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    func updateUIViewController(_ vc: SFSafariViewController, context: Context) {}
}
