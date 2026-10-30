import HuraKit
import SwiftUI

@main
struct HuraApp: App {
    @UIApplicationDelegateAdaptor private var delegate: AppDelegate
    private var model: AppModel { delegate.model }
    @State private var lock = AppLock()
    @State private var trackers = TrackersModel()
    @State private var tab = 0
    @State private var pendingPairURL: String?
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ZStack {
                TabView(selection: $tab) {
                    ServerListView(pendingPairURL: $pendingPairURL)
                        .tabItem { Label("Servers", systemImage: "shippingbox") }
                        .tag(0)
                    InboxView()
                        .tabItem { Label("Inbox", systemImage: "tray") }
                        .badge(trackers.waitingCount)
                        .tag(1)
                }
                // A notification, or a worktree started from a ticket, goes
                // to the servers.
                .onChange(of: model.pendingRoute) { _, route in
                    if route != nil { tab = 0 }
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
            .task {
                // Launch: scenePhase may already be .active, so onChange
                // wouldn't fire.
                model.resumeAll()
                trackers.resume()
                await lock.unlock()
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

struct LockView: View {
    let lock: AppLock

    var body: some View {
        ZStack {
            Color("LaunchBackground").ignoresSafeArea()
            VStack(spacing: 20) {
                Image(systemName: "shippingbox.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(.tint)
                Text("hura")
                    .font(.largeTitle.weight(.semibold))
                    .foregroundStyle(.white)
                if lock.isLocked {
                    Button {
                        Task { await lock.unlock() }
                    } label: {
                        Label("Unlock", systemImage: "faceid")
                            .frame(minWidth: 160)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(lock.isAuthenticating)
                    if let error = lock.lastError {
                        Text(error)
                            .font(.footnote)
                            .foregroundStyle(.white.opacity(0.7))
                            .multilineTextAlignment(.center)
                            .padding(.horizontal)
                    }
                }
            }
        }
    }
}
