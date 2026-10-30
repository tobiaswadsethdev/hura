import HuraKit
import SwiftUI

/// The paired servers: the app's first screen.
struct ServerListView: View {
    @Environment(AppModel.self) private var model
    @Binding var pendingPairURL: String?
    @State private var pairing = false
    @State private var path = NavigationPath()
    @State private var removing: Server?
    @State private var error: String?

    var body: some View {
        NavigationStack(path: $path) {
            List {
                ForEach(model.servers) { s in
                    NavigationLink(value: Route.server(s.id)) {
                        ServerRow(server: s)
                    }
                    .swipeActions {
                        Button("Remove", role: .destructive) { removing = s }
                    }
                }
            }
            .overlay {
                if model.servers.isEmpty {
                    ContentUnavailableView {
                        Label("No servers", systemImage: "shippingbox")
                    } description: {
                        Text("Run `hurad pair phone --host <address>` on the machine running hurad, then scan the code it shows.")
                    } actions: {
                        Button("Pair a Server") { pairing = true }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
            .navigationTitle("Servers")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { pairing = true } label: { Label("Pair", systemImage: "qrcode.viewfinder") }
                }
            }
            .navigationDestination(for: Route.self) { route in
                switch route {
                case let .server(id):
                    if let s = model.server(id) { ServerView(server: s) }
                case let .session(serverID, name):
                    if let s = model.server(serverID) { SessionView(server: s, name: name) }
                }
            }
            .sheet(isPresented: $pairing) {
                PairView(initialURL: pendingPairURL) { server in
                    pairing = false
                    path.append(Route.server(server.id))
                }
            }
            .onChange(of: model.pendingRoute) { _, route in
                // Tapping a notification.
                guard let route else { return }
                model.pendingRoute = nil
                pairing = false
                path = NavigationPath(route)
            }
            .onChange(of: pendingPairURL) { _, url in
                if url != nil { pairing = true }
            }
            .onChange(of: pairing) { _, open in
                if !open { pendingPairURL = nil }
            }
            .confirmationDialog("Remove \(removing?.record.name ?? "")?", isPresented: .constant(removing != nil),
                                titleVisibility: .visible, presenting: removing) { s in
                Button("Remove", role: .destructive) {
                    removing = nil
                    Task {
                        do {
                            try await model.remove(s)
                        } catch {
                            self.error = error.localizedDescription
                        }
                    }
                }
                Button("Cancel", role: .cancel) { removing = nil }
            } message: { _ in
                Text("The token is deleted from this phone's Keychain. To stop the server accepting it, run `hurad revoke` there.")
            }
            .alert("Couldn't remove it", isPresented: .constant(error != nil)) {
                Button("OK") { error = nil }
            } message: {
                Text(error ?? "")
            }
        }
    }
}

enum Route: Hashable {
    case server(String)
    case session(serverID: String, name: String)
}
