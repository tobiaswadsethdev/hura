import SwiftUI
import UIKit

/// Light, dark, or whichever the phone is in: the app's own setting, kept on
/// this phone (Account → Settings).
///
/// Applied to the windows themselves, not with `preferredColorScheme`. That
/// reaches sheets and alerts too, and going back to the system's from a
/// choice made here works, which `preferredColorScheme(nil)` does not
/// reliably do.
enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark

    static let key = "appearance"

    var id: Self { self }

    var label: String {
        switch self {
        case .system: "System"
        case .light: "Light"
        case .dark: "Dark"
        }
    }

    var style: UIUserInterfaceStyle {
        switch self {
        case .system: .unspecified
        case .light: .light
        case .dark: .dark
        }
    }

    /// Every window of the app takes it.
    @MainActor
    func apply() {
        for scene in UIApplication.shared.connectedScenes {
            guard let scene = scene as? UIWindowScene else { continue }
            for window in scene.windows { window.overrideUserInterfaceStyle = style }
        }
    }
}

/// The app's own settings, which are this phone's rather than a server's.
struct AppSettingsView: View {
    @AppStorage(Appearance.key) private var appearance: Appearance = .system
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Appearance", selection: $appearance) {
                        ForEach(Appearance.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                } header: {
                    Text("Appearance")
                } footer: {
                    Text("System follows the phone's own setting. Terminals stay dark either way.")
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        .onChange(of: appearance) { appearance.apply() }
    }
}

extension Color {
    /// A recess for code in a card: darker than the card in dark mode, as it
    /// always was, and lighter than it in light, where darker would be muddy.
    static let well = Color(UIColor { traits in
        traits.userInterfaceStyle == .dark ? UIColor.black.withAlphaComponent(0.4)
            : UIColor.white.withAlphaComponent(0.65)
    })
}
