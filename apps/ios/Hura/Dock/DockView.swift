import HuraKit
import SwiftUI

/// Everything about a worktree that isn't one of its tabs, as the desktop
/// window's dock has it: files and git to work from, and the events, policy
/// and facts that say what is true about it. Events and policy are a tap
/// away on purpose: the isolation being visible is the point.
struct DockView: View {
    enum Pane: String, CaseIterable, Identifiable {
        case files = "Files"
        case git = "Git"
        case events = "Events"
        case policy = "Policy"
        case ports = "Ports"
        case facts = "Facts"

        var id: String { rawValue }
        var icon: String {
            switch self {
            case .files: "folder"
            case .git: "arrow.triangle.branch"
            case .events: "waveform.path.ecg"
            case .policy: "shield.lefthalf.filled"
            case .ports: "point.3.connected.trianglepath.dotted"
            case .facts: "info.circle"
            }
        }
    }

    let server: Server
    let name: String
    @State var pane: Pane = .files

    var body: some View {
        VStack(spacing: 0) {
            Picker("Pane", selection: $pane) {
                ForEach(Pane.allCases) { p in
                    Image(systemName: p.icon).accessibilityLabel(p.rawValue).tag(p)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal)
            .padding(.vertical, 8)
            switch pane {
            case .files: DirView(server: server, name: name, path: "")
            case .git: GitPane(server: server, name: name)
            case .events: EventsPane(server: server, name: name)
            case .policy: PolicyPane(server: server, name: name)
            case .ports: PortsPane(server: server, name: name)
            case .facts: FactsPane(server: server, name: name)
            }
        }
        .navigationTitle(pane.rawValue)
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// Runs an action and keeps its failure for an alert.
struct Failure: ViewModifier {
    @Binding var message: String?

    func body(content: Content) -> some View {
        content.alert("Couldn't do that", isPresented: Binding(get: { message != nil }, set: { if !$0 { message = nil } })) {
            Button("OK") { message = nil }
        } message: {
            Text(message ?? "")
        }
    }
}

extension View {
    func failure(_ message: Binding<String?>) -> some View { modifier(Failure(message: message)) }
}

/// Something read once, with its loading and failure states.
struct Loaded<T, Content: View>: View {
    let value: T?
    let error: String?
    @ViewBuilder let content: (T) -> Content

    var body: some View {
        if let value {
            content(value)
        } else if let error {
            ContentUnavailableView("Couldn't read it", systemImage: "exclamationmark.triangle", description: Text(error))
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
