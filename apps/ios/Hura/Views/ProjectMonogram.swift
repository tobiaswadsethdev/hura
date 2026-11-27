import SwiftUI

/// A project's mark: one or two letters on a tile of its own colour, the same
/// for the same name every time, and the same as the desktop's
/// (`apps/desktop/src/shell/ProjectMonogram.tsx`). Ported from T3 Code
/// (github.com/pingdotgg/t3code: apps/web/src/components/ProjectMonogram.tsx
/// and packages/shared/src/projectMonogram.ts), MIT licensed.
struct ProjectMonogram: View {
    let name: String
    var size: CGFloat = 20

    var body: some View {
        let letters = Monogram.letters(of: name)
        let color = Self.colors[Monogram.colorIndex(of: name)]
        Text(letters)
            .font(.system(size: size * 0.5, weight: .bold, design: .monospaced))
            .minimumScaleFactor(0.5)
            .lineLimit(1)
            .foregroundStyle(color)
            .frame(width: size, height: size)
            .background(color.opacity(0.14), in: RoundedRectangle(cornerRadius: size / 4, style: .continuous))
            .accessibilityHidden(true)
    }

    /// Eighteen hues, in T3 Code's order, so a name hashes to the colour it
    /// would have there and on the desktop: Tailwind's 600 in light, its 400
    /// in dark.
    private static let colors: [Color] = [
        (0x4B5563, 0x9CA3AF), // gray
        (0xDC2626, 0xF87171), // red
        (0xEA580C, 0xFB923C), // orange
        (0xD97706, 0xFBBF24), // amber
        (0xCA8A04, 0xFACC15), // yellow
        (0x65A30D, 0xA3E635), // lime
        (0x16A34A, 0x4ADE80), // green
        (0x059669, 0x34D399), // emerald
        (0x0D9488, 0x2DD4BF), // teal
        (0x0891B2, 0x22D3EE), // cyan
        (0x0284C7, 0x38BDF8), // sky
        (0x2563EB, 0x60A5FA), // blue
        (0x4F46E5, 0x818CF8), // indigo
        (0x7C3AED, 0xA78BFA), // violet
        (0x9333EA, 0xC084FC), // purple
        (0xC026D3, 0xE879F9), // fuchsia
        (0xDB2777, 0xF472B6), // pink
        (0xE11D48, 0xFB7185), // rose
    ].map { Color(dark: $0.1, light: $0.0) }
}

/// The working out behind a monogram, apart from the view so it can be
/// tested without one.
enum Monogram {
    /// How many hues there are to pick from: T3 Code's eighteen.
    static let hues = 18

    /// Which of the hues a name is: its code points folded into one number,
    /// as T3 Code does it.
    static func colorIndex(of name: String) -> Int {
        let folded = name.precomposedStringWithCompatibilityMapping
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased(with: Locale(identifier: "en_US"))
        let seed = folded.isEmpty ? "project" : folded
        var index = 0
        for scalar in seed.unicodeScalars {
            index = (index * 31 + Int(scalar.value)) % hues
        }
        return index
    }

    /// The letters on the tile: the first word's first letter, and then a
    /// digit from it, or the last word's first letter, or the first word's
    /// last. A name in Chinese, Japanese or Korean gets its one character,
    /// which already reads as a compact mark where two would look bulky.
    static func letters(of name: String) -> String {
        let words = name.precomposedStringWithCompatibilityMapping
            .unicodeScalars
            .split { !isLetterOrDigit($0) }
            .map { Array($0) }
        guard let first = words.first, let initial = first.first else { return "PR" }
        if isSingleGlyphScript(initial) { return String(initial) }
        let second = first.dropFirst().first(where: isDigit)
            ?? (words.count > 1 ? words[words.count - 1].first : first.last)
            ?? initial
        var out = String.UnicodeScalarView()
        out.append(initial)
        out.append(second)
        return String(String(out).uppercased().unicodeScalars.prefix(2))
    }

    private static func isLetterOrDigit(_ s: Unicode.Scalar) -> Bool {
        switch s.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter,
             .decimalNumber, .letterNumber, .otherNumber: true
        default: false
        }
    }

    private static func isDigit(_ s: Unicode.Scalar) -> Bool {
        switch s.properties.generalCategory {
        case .decimalNumber, .letterNumber, .otherNumber: true
        default: false
        }
    }

    /// Han, Hiragana, Katakana and Hangul, by their blocks: Swift has no way
    /// to ask a character's script.
    private static func isSingleGlyphScript(_ s: Unicode.Scalar) -> Bool {
        switch s.value {
        case 0x3040...0x30FF, 0x31F0...0x31FF, 0xFF66...0xFF9F, // kana
             0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF, 0x20000...0x2FA1F, // Han
             0x1100...0x11FF, 0x3130...0x318F, 0xAC00...0xD7AF: // Hangul
            true
        default:
            false
        }
    }
}
