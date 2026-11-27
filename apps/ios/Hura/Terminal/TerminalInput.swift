import Foundation

/// A modifier armed from the terminal's key bar, which applies to the next
/// key, from the bar or the keyboard. No cmd: hura's sandboxes are Linux.
enum TerminalModifier: String, CaseIterable {
    case ctrl
    case alt
}

/// A key on the terminal's key bar.
struct TerminalKey: Identifiable, Equatable {
    enum Action: Equatable {
        /// Bytes for the program, through an armed modifier if there is one.
        case send(String)
        case modifier(TerminalModifier)
        case clear
    }

    let id: String
    let label: String
    let action: Action

    /// What the bar offers, in T3 Code's order: the arrows and esc first,
    /// which a phone has no keys for at all, then the modifiers, then what is
    /// a few taps away on the keyboard's other pages.
    static let bar: [TerminalKey] = [
        TerminalKey(id: "up", label: "↑", action: .send("\u{1B}[A")),
        TerminalKey(id: "down", label: "↓", action: .send("\u{1B}[B")),
        TerminalKey(id: "left", label: "←", action: .send("\u{1B}[D")),
        TerminalKey(id: "right", label: "→", action: .send("\u{1B}[C")),
        TerminalKey(id: "esc", label: "esc", action: .send("\u{1B}")),
        TerminalKey(id: "ctrl", label: "ctrl", action: .modifier(.ctrl)),
        TerminalKey(id: "alt", label: "alt", action: .modifier(.alt)),
        TerminalKey(id: "tab", label: "tab", action: .send("\t")),
        TerminalKey(id: "clear", label: "clear", action: .clear),
        TerminalKey(id: "tilde", label: "~", action: .send("~")),
        TerminalKey(id: "pipe", label: "|", action: .send("|")),
        TerminalKey(id: "slash", label: "/", action: .send("/")),
        TerminalKey(id: "dash", label: "-", action: .send("-")),
    ]
}

/// What a key means once a modifier is applied to it, and what a paste
/// becomes on its way to the program. Ported from T3 Code's
/// `terminalInput.ts` (MIT, github.com/pingdotgg/t3code).
enum TerminalInput {
    /// What to do with a key typed under a modifier.
    enum Modified: Equatable {
        case write(String)
        /// Ctrl+V: the phone's clipboard, since the sandbox has none of its
        /// own for the program to paste from.
        case paste
    }

    /// Ctrl+L, which shells and most full-screen programs take as "draw the
    /// screen again, empty".
    static let clear = "\u{0C}"

    static func modified(_ data: String, by modifier: TerminalModifier) -> Modified {
        if modifier == .ctrl, data.lowercased() == "v" { return .paste }
        // An arrow keeps its direction and says which modifier in xterm's
        // way: 5 for ctrl, 3 for alt.
        let scalars = Array(data.unicodeScalars)
        if scalars.count == 3, data.hasPrefix("\u{1B}["), "ABCD".unicodeScalars.contains(scalars[2]) {
            return .write("\u{1B}[1;\(modifier == .ctrl ? 5 : 3)\(Character(scalars[2]))")
        }
        return .write(modifier == .ctrl ? ctrl(data) : "\u{1B}" + data)
    }

    /// The control character a ctrl chord types: a letter's place in the
    /// alphabet, and the six punctuation marks that sit around the letters in
    /// ASCII. Anything else goes as it is.
    static func ctrl(_ data: String) -> String {
        guard let first = data.unicodeScalars.first else { return data }
        let lower = first.properties.lowercaseMapping
        if lower.count == 1, let c = lower.unicodeScalars.first, ("a" ... "z").contains(c) {
            return String(UnicodeScalar(UInt8(c.value - 96)))
        }
        switch first {
        case "@": return "\u{00}"
        case "[": return "\u{1B}"
        case "\\": return "\u{1C}"
        case "]": return "\u{1D}"
        case "^": return "\u{1E}"
        case "_": return "\u{1F}"
        case "?": return "\u{7F}"
        default: return data
        }
    }

    /// Text from the clipboard, made safe to type: control bytes that could
    /// work the program -- an escape, a Ctrl+C -- become spaces, and a line
    /// break is a carriage return, since a program reading raw keys takes a
    /// line feed for Ctrl+J. Where the program has asked for bracketed paste,
    /// the text is marked as pasted, so a line break doesn't send it; with
    /// the escapes taken out, nothing in it can end the bracket early.
    static func paste(_ text: String, bracketed: Bool = false) -> String {
        var out = String.UnicodeScalarView()
        var scalars = text.unicodeScalars.makeIterator()
        var pending = scalars.next()
        while let s = pending {
            pending = scalars.next()
            switch s.value {
            case 0x0D:
                out.append("\r")
                if pending == "\n" { pending = scalars.next() }
            case 0x0A:
                out.append("\r")
            case 0x09:
                out.append(s)
            case 0x00 ... 0x1F, 0x7F:
                out.append(" ")
            default:
                out.append(s)
            }
        }
        let safe = String(out)
        return bracketed ? "\u{1B}[200~" + safe + "\u{1B}[201~" : safe
    }
}
