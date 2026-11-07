import HuraKit
import SwiftTerm
import SwiftUI
import UIKit

/// A tmux session in a sandbox: the agent's own screen in a terminal session,
/// or a shell beside it. SwiftTerm draws it; its keyboard accessory carries
/// the keys a phone lacks (esc, ctrl, tab, arrows).
///
/// A swipe scrolls tmux's history, a long press selects and offers Copy and
/// Paste, and what a program in the sandbox copies through OSC 52 lands on the
/// pasteboard. See `ScrollingTerminalView` for why that takes a subclass.
struct TerminalScreen: View {
    let channel: TerminalChannel
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        VStack(spacing: 0) {
            switch channel.phase {
            case let .offline(why), let .closed(why):
                Label(why, systemImage: channel.phase == .closed(why) ? "xmark.circle" : "wifi.slash")
                    .font(.footnote)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(Color.gray.opacity(0.25))
            case .connecting, .live:
                EmptyView()
            }
            TerminalHost(channel: channel)
                .ignoresSafeArea(.container, edges: .bottom)
        }
        .background(Color.black)
        .environment(\.colorScheme, .dark)
        .onAppear { channel.start() }
        .onDisappear { channel.stop() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { channel.stop() } else if phase == .active { channel.start() }
        }
    }
}

/// SwiftTerm's view, fed from the channel.
private struct TerminalHost: UIViewRepresentable {
    let channel: TerminalChannel

    func makeCoordinator() -> Coordinator { Coordinator(channel: channel) }

    func makeUIView(context: Context) -> TerminalView {
        let view = ScrollingTerminalView(frame: .zero, font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        view.terminalDelegate = context.coordinator
        view.nativeBackgroundColor = .black
        view.nativeForegroundColor = UIColor(white: 0.95, alpha: 1)
        view.backgroundColor = .black
        // tmux keeps the scrollback that matters.
        view.getTerminal().changeHistorySize(2000)
        channel.output = { [weak view] data in
            view?.feed(byteArray: ArraySlice(data))
        }
        // The size the view settles at goes to the server once the channel is
        // open; until then it is remembered.
        DispatchQueue.main.async {
            let t = view.getTerminal()
            channel.resize(cols: t.cols, rows: t.rows)
        }
        return view
    }

    func updateUIView(_ view: TerminalView, context: Context) {}

    static func dismantleUIView(_ view: TerminalView, coordinator: Coordinator) {
        coordinator.channel.output = { _ in }
    }

    // SwiftTerm calls its delegate on the main thread, from UIKit.
    @MainActor
    final class Coordinator: NSObject, @preconcurrency TerminalViewDelegate {
        let channel: TerminalChannel

        init(channel: TerminalChannel) {
            self.channel = channel
        }

        func send(source: TerminalView, data: ArraySlice<UInt8>) {
            channel.send(Data(data))
        }

        func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            channel.resize(cols: newCols, rows: newRows)
        }

        func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
            guard let url = URL(string: link), ["http", "https"].contains(url.scheme) else { return }
            UIApplication.shared.open(url)
        }

        func clipboardCopy(source: TerminalView, content: Data) {
            if let text = String(data: content, encoding: .utf8) { UIPasteboard.general.string = text }
        }

        func setTerminalTitle(source: TerminalView, title: String) {}
        func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        func scrolled(source: TerminalView, position: Double) {}
        func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
    }
}

/// SwiftTerm's view, with a finger that scrolls rather than drags.
///
/// The server attaches with tmux's mouse on, so the wheel scrolls its history
/// -- which is the only history there is: tmux draws on the alternate screen,
/// and this view's own scrollback stays empty. SwiftTerm turns a pan into a
/// mouse *drag* while a program wants the mouse, which in tmux is a selection,
/// so a swipe selected text instead of scrolling and nothing could be read
/// back. Here a vertical pan is a run of wheel clicks instead.
///
/// `allowMouseReporting` goes off with it, which leaves taps and the long
/// press to SwiftTerm: a long press selects on the screen as drawn and offers
/// Copy and Paste, and a selection is no longer dropped on the next line of
/// output, which with an agent writing was every moment.
final class ScrollingTerminalView: TerminalView {
    /// How far a finger travels for one wheel click, in rows. tmux scrolls
    /// five lines a click; three rows of travel per click scrolls a little
    /// faster than the finger, which suits a screen this small.
    private static let rowsPerClick: CGFloat = 3

    private var scroll: UIPanGestureRecognizer?
    private let simultaneous = Simultaneous()
    /// Travel not yet spent on a whole click.
    private var carried: CGFloat = 0

    override init(frame: CGRect, font: UIFont?) {
        super.init(frame: frame, font: font)
        allowMouseReporting = false
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        allowMouseReporting = false
    }

    /// Not `super`'s: that adds the pan that drags.
    override func mouseModeChanged(source: Terminal) {
        if source.mouseMode != .off {
            guard scroll == nil else { return }
            let pan = UIPanGestureRecognizer(target: self, action: #selector(scrolled(_:)))
            pan.delegate = simultaneous
            addGestureRecognizer(pan)
            scroll = pan
        } else if let pan = scroll {
            removeGestureRecognizer(pan)
            scroll = nil
        }
    }

    @objc private func scrolled(_ pan: UIPanGestureRecognizer) {
        let terminal = getTerminal()
        // Dragging a selection's handles is a pan too, and it is the selection's.
        guard !selection.active, terminal.rows > 0 else {
            carried = 0
            return
        }
        switch pan.state {
        case .began:
            carried = 0
        case .changed:
            let row = bounds.height / CGFloat(terminal.rows)
            carried += pan.translation(in: self).y
            pan.setTranslation(.zero, in: self)
            let step = row * Self.rowsPerClick
            let point = pan.location(in: self)
            let col = min(max(Int(point.x / (bounds.width / CGFloat(terminal.cols))), 0), terminal.cols - 1)
            let line = min(max(Int((point.y - contentOffset.y) / row), 0), terminal.rows - 1)
            while abs(carried) >= step {
                // A finger moving down pulls older lines into view: wheel up.
                let up = carried > 0
                let flags = terminal.encodeButton(button: up ? 4 : 5, release: false, shift: false, meta: false, control: false)
                terminal.sendEvent(buttonFlags: flags, x: col, y: line)
                carried -= up ? step : -step
            }
        default:
            carried = 0
        }
    }

    /// The scroll view's own pan is left to run alongside: with nothing in its
    /// scrollback it moves nothing, and refusing it would cost the selection's.
    private final class Simultaneous: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizer(_: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith _: UIGestureRecognizer) -> Bool {
            true
        }
    }
}
