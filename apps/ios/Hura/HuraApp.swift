import HuraKit
import SwiftUI

@main
struct HuraApp: App {
    @UIApplicationDelegateAdaptor private var delegate: AppDelegate
    private var model: AppModel { delegate.model }
    @State private var lock = AppLock()
    @State private var account = AccountModel()
    private var trackers: TrackersModel { delegate.trackers }
    @State private var tab = 0
    @State private var pendingPairURL: String?
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ZStack {
                TabView(selection: $tab) {
                    ServerListView(pendingPairURL: $pendingPairURL)
                        .tabItem { Label("Servers", glyph: .server) }
                        .tag(0)
                    InboxView()
                        .tabItem { Label("Inbox", glyph: .inbox) }
                        .badge(trackers.unseenCount)
                        .tag(1)
                    AccountView()
                        .tabItem { Label("Account", glyph: .account) }
                        .tag(2)
                }
                .onChange(of: tab) { if tab == 1 { trackers.markSeen() } }
                .onChange(of: trackers.inbox) { if tab == 1 { trackers.markSeen() } }
                // A notification, or a worktree started from a ticket, goes
                // to the servers.
                .onChange(of: model.pendingRoute) { _, route in
                    if route != nil { tab = 0 }
                }
                // A notification about a ticket or pull request.
                .onChange(of: trackers.showInbox) { _, show in
                    if show {
                        tab = 1
                        trackers.showInbox = false
                    }
                }
                .onChange(of: trackers.showAccount) { _, show in
                    if show {
                        tab = 2
                        trackers.showAccount = false
                    }
                }
                // Covers the content while locked, and whenever the app isn't
                // active so the app switcher snapshot shows nothing.
                if lock.isLocked || scenePhase != .active {
                    LockView(lock: lock)
                        .transition(.opacity)
                }
            }
            .animation(.default, value: lock.isLocked)
            .environment(model)
            .environment(lock)
            .environment(trackers)
            .environment(account)
            .task {
                // Launch: scenePhase may already be .active, so onChange
                // wouldn't fire.
                model.resumeAll()
                trackers.resume()
                await lock.unlock()
                #if DEBUG
                await pairFromLaunchArguments()
                #endif
            }
            .onOpenURL { url in
                // A hura:// pairing string opens the pairing screen; nothing
                // is sent until the user confirms.
                if url.scheme == "hura" {
                    pendingPairURL = url.absoluteString
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active:
                model.resumeAll()
                trackers.resume()
                Task { await lock.unlock() }
            case .background:
                lock.lock()
                model.suspendAll()
                trackers.suspend()
            default:
                break
            }
        }
    }
}

#if DEBUG
extension HuraApp {
    /// Debug builds only: pair with the server a launch argument names, and
    /// sign in to the hura cloud another names, with no screen to confirm
    /// either on --
    ///
    ///     xcrun simctl launch <device> dev.tobiaswadseth.hura.ios \
    ///         -HuraPair 'hura://127.0.0.1:18555/<token>#<fingerprint>' \
    ///         -HuraCloud http://127.0.0.1:18556 -HuraNoLock YES
    ///
    /// so the app can be looked at in a simulator against stand-ins
    /// (`Tools/fake-hurad.py` is both) with nothing to tap. The pairing is the
    /// real one -- the certificate is pinned and the token checked -- only the
    /// confirmation is skipped; the sign-in is the emailed-code one, with a
    /// code the stand-in takes.
    ///
    /// `-HuraOpen inbox` or `account` opens that tab; `server` the first
    /// server's list, and a session's name that session, by the same route a
    /// tapped notification takes.
    @MainActor
    func pairFromLaunchArguments() async {
        let defaults = UserDefaults.standard
        if let raw = defaults.string(forKey: "HuraPair"),
           let pairing = try? PairingString(raw),
           let record = try? await Pairing.pair(pairing) {
            _ = try? model.add(record)
        }
        if let raw = defaults.string(forKey: "HuraCloud"), let base = URL(string: raw),
           await Cloud.shared.current?.base != base {
            await Cloud.shared.signOut()
            _ = try? await Cloud.shared.verify(email: "you@example.com", code: "000000",
                                              deviceName: UIDevice.current.name, base: base)
            await account.load()
            await trackers.refresh()
        }
        guard let open = defaults.string(forKey: "HuraOpen") else { return }
        // After the first frame: a screen only hears a change made while it is
        // on screen, and at launch it is not yet.
        try? await Task.sleep(for: .milliseconds(500))
        switch open {
        case "inbox": tab = 1
        case "account": tab = 2
        default:
            guard let server = model.servers.first else { return }
            let id = server.record.serverID
            model.pendingRoute = open == "server" ? [.server(id)] : [.server(id), .session(serverID: id, name: open)]
        }
    }
}
#endif

struct LockView: View {
    let lock: AppLock

    var body: some View {
        // On the system background, as the launch screen is, so opening the
        // app goes from one to the other without a change of colour: the
        // app's icon and the way in.
        ZStack {
            Color(.systemBackground).ignoresSafeArea()
            VStack(spacing: 28) {
                Image("Logo")
                    .resizable()
                    .frame(width: 112, height: 112)
                    .clipShape(RoundedRectangle(cornerRadius: 25, style: .continuous))
                    // A hairline, as iOS draws round a dark icon on black,
                    // or its edge is lost; nothing on white.
                    .overlay(RoundedRectangle(cornerRadius: 25, style: .continuous)
                        .strokeBorder(.white.opacity(0.12), lineWidth: 0.5))
                    .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
                    .accessibilityLabel("hura")
                if lock.isLocked {
                    VStack(spacing: 12) {
                        Button {
                            Task { await lock.unlock() }
                        } label: {
                            Label("Unlock", systemImage: "faceid")
                                .frame(minWidth: 180)
                        }
                        .buttonStyle(.borderedProminent)
                        .buttonBorderShape(.capsule)
                        .controlSize(.large)
                        .disabled(lock.isAuthenticating)
                        if let error = lock.lastError {
                            Text(error)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.center)
                                .padding(.horizontal)
                        }
                    }
                }
            }
        }
    }
}
