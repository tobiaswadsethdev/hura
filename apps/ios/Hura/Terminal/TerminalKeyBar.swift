import SwiftUI
import UIKit

/// The row over the keyboard with the keys a phone lacks, in T3 Code's shape
/// (`ThreadTerminalRouteScreen.tsx` and `ComposerToolbar.tsx`, MIT,
/// github.com/pingdotgg/t3code): the arrows, esc, ctrl and alt, tab, CLEAR
/// and a few marks in a row that scrolls, with Paste and the way to put the
/// keyboard away fixed at its end.
///
/// The terminal's colours rather than the app's, so that a terminal kept dark
/// has a dark bar on it too.
struct TerminalKeyBar: View {
    /// The key's height, and so the bar's with its padding and hairline.
    static let keyHeight: CGFloat = 44
    static let padding: CGFloat = 4
    static let height = keyHeight + padding * 2 + 1

    let control: TerminalControl

    var body: some View {
        let look = control.look ?? TerminalLook(.dark)
        let ink = SwiftUI.Color(look.foreground)
        let paper = SwiftUI.Color(look.background)
        HStack(spacing: 6) {
            FadingRow(fade: paper) {
                ForEach(TerminalKey.bar) { key in
                    let armed = control.armed.map { key.action == .modifier($0) } ?? false
                    Button { control.view?.press(key) } label: {
                        Text(caps(key) ? key.label.uppercased() : key.label)
                    }
                    .buttonStyle(KeyStyle(ink: ink, paper: paper, armed: armed,
                                          minWidth: key.label.count > 1 ? 56 : 44))
                    .accessibilityLabel(Self.spoken[key.id] ?? key.label)
                    .accessibilityAddTraits(armed ? .isSelected : [])
                }
            }
            Button { control.view?.paste() } label: {
                Image(systemName: "doc.on.clipboard")
            }
            .buttonStyle(KeyStyle(ink: ink, paper: paper, armed: false, minWidth: 44, round: true))
            .accessibilityLabel("Paste")
            Button { control.view?.hideKeyboard() } label: {
                Image(systemName: "keyboard.chevron.compact.down")
            }
            .buttonStyle(KeyStyle(ink: ink, paper: paper, armed: false, minWidth: 44, round: true))
            .accessibilityLabel("Dismiss keyboard")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, Self.padding)
        .frame(maxWidth: .infinity, minHeight: Self.height, alignment: .center)
        .background(paper)
        .overlay(alignment: .top) { Rectangle().fill(ink.opacity(0.15)).frame(height: 1) }
        .environment(\.colorScheme, look.dark ? .dark : .light)
    }

    /// The modifiers and CLEAR are words a terminal user reads in capitals.
    private func caps(_ key: TerminalKey) -> Bool {
        switch key.action {
        case .modifier, .clear: true
        case .send: false
        }
    }

    /// What VoiceOver says for a key whose label is a mark.
    private static let spoken = [
        "up": "Up arrow", "down": "Down arrow", "left": "Left arrow", "right": "Right arrow",
        "esc": "Escape", "ctrl": "Control", "alt": "Alt", "tab": "Tab", "clear": "Clear screen",
        "tilde": "Tilde", "pipe": "Vertical bar", "slash": "Slash", "dash": "Dash",
    ]
}

/// A key: a pill in the terminal's ink, filled with it while armed.
private struct KeyStyle: ButtonStyle {
    let ink: SwiftUI.Color
    let paper: SwiftUI.Color
    let armed: Bool
    let minWidth: CGFloat
    var round = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(round ? .system(size: 16) : .subheadline.weight(.bold))
            .lineLimit(1)
            .foregroundStyle(armed ? paper : ink)
            .padding(.horizontal, round ? 0 : 14)
            .frame(minWidth: minWidth, maxWidth: round ? minWidth : 120)
            .frame(height: TerminalKeyBar.keyHeight)
            .background(armed ? ink : ink.opacity(0.08), in: Capsule())
            .overlay(Capsule().strokeBorder(ink.opacity(armed ? 1 : 0.14), lineWidth: 1))
            .contentShape(Capsule())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

/// A row that scrolls sideways, with its ends fading into the bar where there
/// is more to scroll to, as T3 Code's `ComposerToolbarScroller`.
private struct FadingRow<Content: View>: View {
    let fade: SwiftUI.Color
    @ViewBuilder let content: Content
    @State private var width: CGFloat = 0
    @State private var extent: (min: CGFloat, max: CGFloat) = (0, 0)

    private static var fadeWidth: CGFloat { 18 }
    /// A few points of slack, so a row scrolled all the way doesn't keep a
    /// sliver of fade from rounding.
    private static var slack: CGFloat { 4 }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) { content }
                .padding(.trailing, 2)
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(Self.space)) } action: {
                    extent = ($0.minX, $0.maxX)
                }
        }
        .coordinateSpace(.named(Self.space))
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .overlay(alignment: .leading) {
            if extent.min < -Self.slack { edge(from: fade, to: fade.opacity(0)) }
        }
        .overlay(alignment: .trailing) {
            if extent.max > width + Self.slack { edge(from: fade.opacity(0), to: fade) }
        }
    }

    private static var space: String { "terminal-keys" }

    private func edge(from: SwiftUI.Color, to: SwiftUI.Color) -> some View {
        LinearGradient(colors: [from, to], startPoint: .leading, endPoint: .trailing)
            .frame(width: Self.fadeWidth)
            .allowsHitTesting(false)
    }
}

/// The bar as the keyboard's accessory, so it rides on the keyboard as it
/// comes and goes, and is gone with it.
final class TerminalKeyBarHost: UIView {
    private let host: UIHostingController<TerminalKeyBar>

    init(control: TerminalControl) {
        host = UIHostingController(rootView: TerminalKeyBar(control: control))
        super.init(frame: CGRect(x: 0, y: 0, width: 0, height: TerminalKeyBar.height))
        autoresizingMask = .flexibleWidth
        // Its own size, not one the hosting controller works out from the
        // view: an accessory that changes height makes the keyboard jump.
        host.sizingOptions = []
        if #available(iOS 16.4, *) { host.safeAreaRegions = [] }
        host.view.backgroundColor = .clear
        host.view.frame = bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        addSubview(host.view)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: TerminalKeyBar.height)
    }
}
