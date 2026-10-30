import HuraKit
import SwiftUI

/// The Notifications section of a server's screen: on or off, which kinds,
/// and a test push.
struct NotificationSettings: View {
    let server: Server
    @Environment(AppModel.self) private var model
    @State private var prefs: [String: Bool] = [:]
    @State private var encrypted = false
    @State private var working = false
    @State private var message: String?

    private static let kinds: [(key: String, label: String)] = [
        ("claude_needs_input", "Claude needs input or permission"),
        ("claude_turn_finished", "Claude finished a turn"),
        ("claude_failed", "A session failed"),
    ]

    private var online: Bool { server.state == .online }

    private var isOn: Bool { server.record.pushLinkID != nil }
    private var canChange: Bool { online && !working }

    var body: some View {
        Section {
            Toggle(isOn: Binding(get: { isOn }, set: { on in change { try await model.setNotifications(on, for: server) } })) {
                HStack {
                    Text("Notifications")
                    if working { ProgressView() }
                }
            }
            .disabled(!canChange)
            if isOn, encrypted {
                Label("Encrypted end to end", systemImage: "lock.fill")
                    .foregroundStyle(.secondary)
            }
            if isOn {
                ForEach(Self.kinds, id: \.key) { kind in
                    Toggle(kind.label, isOn: Binding(get: { prefs[kind.key] ?? true }, set: { value in
                        var next = prefs
                        next[kind.key] = value
                        change {
                            try await model.setNotifications(true, for: server, prefs: next)
                            prefs = next
                        }
                    }))
                    .disabled(!canChange)
                }
                Button("Send Test Notification") {
                    change {
                        let r = try await model.push.sendTest()
                        message = r.sent ? nil : "Apple refused the test notification (\(r.apnsReason ?? "status \(r.apnsStatus)"))."
                    }
                }
                .disabled(working || model.push.device == nil)
            }
        } header: {
            Text("Notifications")
        } footer: {
            if let message = message ?? model.pushError {
                Text(message).foregroundStyle(.red)
            } else if encrypted {
                Text("When Claude needs you or a session fails, \(server.record.name) notifies this phone through hura's push relay. Only this phone can read the text; the relay and Apple see which kind of notification it is.")
            } else {
                Text("When Claude needs you or a session fails, \(server.record.name) notifies this phone through hura's push relay. Notifications show the session name and a short message.")
            }
        }
        .task(id: online) { await loadStatus() }
    }

    private func loadStatus() async {
        guard online, isOn, let status = try? await server.pushStatus() else { return }
        prefs = status.prefs
        encrypted = status.encrypted
    }

    private func change(_ body: @escaping () async throws -> Void) {
        working = true
        message = nil
        Task {
            defer { working = false }
            do { try await body() } catch { message = error.localizedDescription }
            await loadStatus()
        }
    }
}
