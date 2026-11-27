import SwiftUI
import UIKit

/// The desktop's palette, which is T3 Code's (github.com/pingdotgg/t3code,
/// MIT), as the custom properties at the top of `apps/desktop/src/style.css`
/// name it: a black canvas with cards one step up in dark, zinc greys on near
/// white in light, one blue for the primary action, and the states in T3
/// Code's hues. Named for what they mean, so a screen asks for `.surface`
/// rather than a grey, and the phone and the desktop are one thing.
///
/// Borders and fills are the foreground at an alpha (`.line`, `.hover`), as on
/// the desktop, so a card can sit on any of these and its edge reads the same.
///
/// Nothing may use a state's colour for anything but that state: the blue
/// that means "an agent is working" cannot also decorate a heading.
extension Color {
    // Surfaces

    /// The canvas: a conversation, a list.
    static let canvas = Color(dark: 0x000000, light: 0xFCFCFC)
    /// Raised: a card, a sheet, the composer.
    static let surface = Color(dark: 0x111111, light: 0xFFFFFF)
    /// A step further: your own message, a chip, a track.
    static let surface2 = Color(dark: 0x1E1E1E, light: 0xEBEBED)
    /// Where code is shown: barely off the canvas.
    static let sunken = Color(dark: 0x0A0A0A, light: 0xF4F4F5)

    // Text

    static let ink = Color(dark: 0xF5F5F5, light: 0x18181B)
    static let dim = Color(dark: 0xA1A1A1, light: 0x71717B)
    /// Third rank: a hint under a label, a count beside a name.
    static let dimmer = Color(dark: 0x737373, light: 0x9F9FA9)

    // Lines and fills, the foreground at an alpha

    static let line = Color(UIColor { $0.isDark ? .white.withAlphaComponent(0.08) : UIColor(hex: 0x09090B, alpha: 0.1) })
    static let lineStrong = Color(UIColor { $0.isDark ? .white.withAlphaComponent(0.11) : UIColor(hex: 0x09090B, alpha: 0.17) })
    static let hover = Color(UIColor { $0.isDark ? .white.withAlphaComponent(0.05) : UIColor(hex: 0x09090B, alpha: 0.045) })
    static let selected = Color(UIColor { $0.isDark ? .white.withAlphaComponent(0.08) : UIColor(hex: 0x09090B, alpha: 0.07) })

    // The states, in T3 Code's hues: info blue for working, approval amber for
    // waiting on you, emerald for done, red for failed.

    /// The desktop's `--working`.
    static let working = Color(dark: 0x51A2FF, light: 0x1447E6)
    /// Waiting on you: `--attention`.
    static let attention = Color(dark: 0xFFB900, light: 0xBB4D00)
    static let ok = Color(dark: 0x00D492, light: 0x007A55)
    static let bad = Color(dark: 0xFF6467, light: 0xC10007)
    static let warn = Color(dark: 0xFE9A00, light: 0xBB4D00)
    /// The stop button's fill, a little deeper than `.bad`.
    static let stop = Color(dark: 0xFB414A, light: 0xE7000B)

    /// A colour in each appearance, as `0xRRGGBB`.
    init(dark: UInt32, light: UInt32) {
        self.init(UIColor { $0.isDark ? UIColor(hex: dark) : UIColor(hex: light) })
    }
}

extension ShapeStyle where Self == Color {
    static var canvas: Color { .canvas }
    static var surface: Color { .surface }
    static var surface2: Color { .surface2 }
    static var ink: Color { .ink }
    static var dim: Color { .dim }
    static var dimmer: Color { .dimmer }
    static var line: Color { .line }
    static var working: Color { .working }
    static var attention: Color { .attention }
    static var ok: Color { .ok }
    static var bad: Color { .bad }
}

/// The desktop's radii: a row 6, a control 8, a card 12, the composer and a
/// sheet 16.
enum Radius {
    static let row: CGFloat = 6
    static let control: CGFloat = 8
    static let card: CGFloat = 12
    static let composer: CGFloat = 16
}

extension View {
    /// A card in T3 Code's shape: the surface, 12 round, with a hairline.
    func card(radius: CGFloat = Radius.card, fill: Color = .surface) -> some View {
        background(fill, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).strokeBorder(Color.line, lineWidth: 1))
    }
}

extension UITraitCollection {
    var isDark: Bool { userInterfaceStyle == .dark }
}

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}
