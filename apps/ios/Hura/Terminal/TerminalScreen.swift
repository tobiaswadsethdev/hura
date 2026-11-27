import HuraKit
import SwiftTerm
import SwiftUI
import UIKit

/// A tmux session in a sandbox: the agent's own screen in a terminal session,
/// or a shell beside it. SwiftTerm draws it; `TerminalKeyBar`, over the
/// keyboard, carries the keys a phone lacks (arrows, esc, ctrl and alt, tab),
/// a Paste and the way to put the keyboard away. With the keyboard away, a
/// button in the corner brings it back.
///
/// A swipe scrolls tmux's history, and typing -- or the Latest button --
/// brings it back to the live screen. A long press selects as text selects
/// everywhere else on the phone, magnifier and handles and all, and a pinch
/// changes the size of the type. What a program in the sandbox copies through
/// OSC 52 lands on the pasteboard. See `ScrollingTerminalView` for all of it.
///
/// Light or dark as the app is, unless Settings keeps terminals dark.
struct TerminalScreen: View {
    let channel: TerminalChannel
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var appScheme
    @AppStorage(TerminalLook.darkKey) private var alwaysDark = false
    @AppStorage(TerminalLook.sizeKey) private var fontSize = TerminalLook.defaultSize
    @State private var control = TerminalControl()

    private var scheme: ColorScheme { alwaysDark ? .dark : appScheme }

    var body: some View {
        let look = TerminalLook(scheme)
        ZStack {
            TerminalHost(channel: channel, control: control, look: look, fontSize: fontSize) { fontSize = $0 }
                .ignoresSafeArea(.container, edges: .bottom)
        }
        .overlay(alignment: .top) { banner }
        .overlay(alignment: .bottomTrailing) {
            VStack(alignment: .trailing, spacing: 12) {
                latest
                showKeyboard
            }
            .padding(16)
        }
        .animation(.snappy(duration: 0.2), value: control.inHistory)
        .animation(.snappy(duration: 0.2), value: control.selecting)
        .animation(.snappy(duration: 0.2), value: control.keyboard)
        .background(SwiftUI.Color(look.background))
        .environment(\.colorScheme, scheme)
        // For the key bar, which the keyboard hosts outside this view's
        // environment.
        .onChange(of: look, initial: true) { control.look = look }
        .onAppear { channel.start() }
        .onDisappear { channel.stop() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { channel.stop() } else if phase == .active { channel.start() }
        }
    }

    /// What is wrong with the connection, or the way out of a selection: over
    /// the screen rather than above it, so neither resizes the terminal --
    /// which would resize the pty, and the agent's screen with it.
    @ViewBuilder
    private var banner: some View {
        if control.selecting {
            HStack(spacing: 0) {
                Button { control.view?.copyAll() } label: {
                    Label("Copy All", systemImage: "doc.on.doc").padding(.horizontal, 14).padding(.vertical, 8)
                }
                Divider().frame(height: 18)
                Button { control.view?.endSelecting() } label: {
                    Text("Done").bold().padding(.horizontal, 14).padding(.vertical, 8)
                }
            }
            .font(.footnote)
            .glass(interactive: true)
            .padding(.top, 8)
            .transition(.move(edge: .top).combined(with: .opacity))
        } else {
            switch channel.phase {
            case let .offline(why), let .closed(why):
                Label(why, glyph: channel.phase == .closed(why) ? .closed : .offline)
                    .font(.footnote)
                    .lineLimit(2)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .glass()
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
            case .connecting, .live:
                EmptyView()
            }
        }
    }

    @ViewBuilder
    private var latest: some View {
        if control.inHistory && !control.selecting {
            Button { control.view?.latest() } label: {
                Label("Latest", systemImage: "arrow.down.to.line")
                    .font(.footnote.weight(.semibold))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
            }
            .glass(interactive: true)
            .transition(.scale(scale: 0.8).combined(with: .opacity))
        }
    }

    /// With the keyboard put away, the key bar goes with it; this brings both
    /// back, as T3 Code's does. Not while selecting, which has its own Done,
    /// and not once the session has closed, when there is nothing to type to.
    @ViewBuilder
    private var showKeyboard: some View {
        if !control.keyboard && !control.selecting && !closed {
            Button { control.view?.showKeyboard() } label: {
                Image(systemName: "keyboard")
                    .font(.system(size: 20))
                    .foregroundStyle(SwiftUI.Color(TerminalLook(scheme).foreground))
                    .frame(width: 48, height: 48)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .glass(in: Circle(), interactive: true)
            .accessibilityLabel("Show keyboard")
            .transition(.scale(scale: 0.8).combined(with: .opacity))
        }
    }

    private var closed: Bool {
        if case .closed = channel.phase { true } else { false }
    }
}

/// What the SwiftUI around the terminal needs of it: whether to offer the way
/// back from tmux's history, whether a selection is up, whether the keyboard
/// is, which modifier the key bar has armed, and the view to tell.
@MainActor
@Observable
final class TerminalControl {
    @ObservationIgnored weak var view: ScrollingTerminalView?
    var inHistory = false
    var selecting = false
    var keyboard = false
    var armed: TerminalModifier?
    /// The terminal's colours, for the key bar.
    var look: TerminalLook?
}

/// A terminal's colours, for the app's light and dark, and the same as the
/// desktop's (`panes/Terminal.tsx`): the system's hues for the sixteen a
/// program names by number.
///
/// SwiftTerm has nothing like xterm's minimum contrast, so in light the
/// palette carries all of it: every hue deep enough to read on white, and
/// `white` a grey that can still be text. A program that picks its own
/// colours for a dark terminal -- Claude Code's dark theme does -- is still
/// pale on white, which is what "Keep terminals dark" is for.
struct TerminalLook: Equatable {
    static let darkKey = "terminalsDark"
    static let sizeKey = "terminalFontSize"
    static let defaultSize = 12.0
    static let sizes = 8.0...22.0

    let dark: Bool
    let background: UIColor
    let foreground: UIColor
    let selection: UIColor
    let ansi: [SwiftTerm.Color]

    init(_ scheme: ColorScheme) {
        dark = scheme == .dark
        if dark {
            // Black and white, which are the system's backgrounds, so the
            // navigation bar over the terminal is the terminal's colour.
            background = UIColor(hex: 0x000000)
            foreground = UIColor(hex: 0xE4E4E6)
            selection = UIColor(red: 10 / 255, green: 132 / 255, blue: 1, alpha: 0.45)
            ansi = [
                0x1C1C1E, 0xFF453A, 0x32D74B, 0xFFD60A, 0x0A84FF, 0xBF5AF2, 0x64D2FF, 0xD1D1D6,
                0x8E8E93, 0xFF6961, 0x30DB5B, 0xFFE066, 0x409CFF, 0xDA8FFF, 0x70D7FF, 0xFFFFFF,
            ].map(SwiftTerm.Color.init(hex:))
        } else {
            background = UIColor(hex: 0xFFFFFF)
            foreground = UIColor(hex: 0x1D1D1F)
            selection = UIColor(red: 0, green: 122 / 255, blue: 1, alpha: 0.25)
            ansi = [
                0x1D1D1F, 0xD70015, 0x248A3D, 0xA05A00, 0x0040DD, 0x8944AB, 0x0071A4, 0x8E8E93,
                0x6E6E73, 0xE5342B, 0x1E7B34, 0xB25000, 0x0055D4, 0x9A3FC4, 0x00668F, 0x48484A,
            ].map(SwiftTerm.Color.init(hex:))
        }
    }

    static func == (a: Self, b: Self) -> Bool { a.dark == b.dark }
}

private extension UIColor {
    convenience init(hex: UInt32) {
        self.init(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
}

private extension SwiftTerm.Color {
    convenience init(hex: Int) {
        // Sixteen bits a channel: 0xFF * 257 is 0xFFFF.
        self.init(red: UInt16((hex >> 16) & 0xFF) * 257, green: UInt16((hex >> 8) & 0xFF) * 257,
                  blue: UInt16(hex & 0xFF) * 257)
    }
}

/// SwiftTerm's view, fed from the channel.
private struct TerminalHost: UIViewRepresentable {
    let channel: TerminalChannel
    let control: TerminalControl
    let look: TerminalLook
    let fontSize: Double
    let onFontSize: (Double) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(channel: channel) }

    func makeUIView(context: Context) -> TerminalView {
        let view = ScrollingTerminalView(frame: .zero, font: .monospacedSystemFont(ofSize: fontSize, weight: .regular))
        view.terminalDelegate = context.coordinator
        view.apply(look)
        // tmux keeps the scrollback that matters.
        view.getTerminal().changeHistorySize(2000)
        control.view = view
        view.onHistory = { [control] in control.inHistory = $0 }
        view.onSelecting = { [control] in control.selecting = $0 }
        view.onKeyboard = { [control] in control.keyboard = $0 }
        view.onArmed = { [control] in control.armed = $0 }
        view.onFontSize = onFontSize
        // In place of SwiftTerm's own accessory, whose ctrl is the only
        // modifier and whose keys don't follow the terminal's colours.
        view.inputAccessoryView = TerminalKeyBarHost(control: control)
        channel.output = { [weak view] data in
            view?.feed(byteArray: ArraySlice(data))
            view?.fed()
        }
        // The size the view settles at goes to the server once the channel is
        // open; until then it is remembered.
        DispatchQueue.main.async {
            let t = view.getTerminal()
            channel.resize(cols: t.cols, rows: t.rows)
        }
        return view
    }

    func updateUIView(_ view: TerminalView, context: Context) {
        guard let view = view as? ScrollingTerminalView else { return }
        if view.look != look { view.apply(look) }
        view.onFontSize = onFontSize
    }

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
            // A key while tmux shows its history would be taken by copy mode;
            // out of it first. See `ScrollingTerminalView.leavingForInput`.
            if let view = source as? ScrollingTerminalView, view.leavingForInput() {
                channel.send(Data("q".utf8))
            }
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

/// SwiftTerm's view, with a finger that scrolls rather than drags, a long
/// press that selects as the rest of iOS does, and a pinch that sizes the type.
///
/// **Scrolling.** The server attaches with tmux's mouse on, so the wheel
/// scrolls its history -- which is the only history there is: tmux draws on
/// the alternate screen, and this view's own scrollback stays empty. SwiftTerm
/// turns a pan into a mouse *drag* while a program wants the mouse, which in
/// tmux is a selection, so a swipe selected text instead of scrolling and
/// nothing could be read back. Here a vertical pan is a run of wheel clicks
/// instead. `allowMouseReporting` goes off with it, which leaves taps to
/// SwiftTerm.
///
/// Scrolled back, tmux is in copy mode, where keys move about the history
/// instead of reaching the program. A terminal snaps to the bottom when you
/// type, so this does too: see `leavingForInput`.
///
/// **Selecting.** SwiftTerm's own long press offers a bare "Select" menu that
/// takes a word, with no handles and no magnifier, and nothing of it can be
/// reached from here to improve. So it is replaced: a long press lays the
/// screen's text over the screen in a `UITextView`, in the same font on the
/// same rows, and selects in that -- the system's own selection, with its
/// magnifier while the finger moves, its handles after, and Copy, Look Up and
/// Share. It is a snapshot, so output arriving underneath cannot move or drop
/// the selection, which with an agent writing was every moment.
final class ScrollingTerminalView: TerminalView {
    /// How far a finger travels for one wheel click, in rows. tmux scrolls
    /// five lines a click; three rows of travel per click scrolls a little
    /// faster than the finger, which suits a screen this small.
    private static let rowsPerClick: CGFloat = 3

    var onHistory: ((Bool) -> Void)?
    var onSelecting: ((Bool) -> Void)?
    var onFontSize: ((Double) -> Void)?
    var onKeyboard: ((Bool) -> Void)?
    var onArmed: ((TerminalModifier?) -> Void)?
    private(set) var look: TerminalLook?

    /// The modifier the key bar has armed for the next key.
    private var armed: TerminalModifier? {
        didSet { if armed != oldValue { onArmed?(armed) } }
    }

    private var scroll: UIPanGestureRecognizer?
    private let simultaneous = Simultaneous()
    /// Travel not yet spent on a whole click.
    private var carried: CGFloat = 0

    /// Whether this view has scrolled tmux back, and when it last did. The
    /// position tmux draws is only believed after that: see `inHistory`.
    private var scrolledBack = false
    private var scrolledAt = Date.distantPast
    private var shownHistory = false
    /// Set while the terminal itself is answering -- a report, or the wheel
    /// clicks above -- which is not somebody typing.
    private var replying = false

    /// Ours is the only long press: see `addGestureRecognizer`.
    private var addingOwn = false
    private var textLayer: SelectionTextView?
    private var loupe: UITextLoupeSession?
    /// The word first pressed, which a drag extends from in either direction.
    private var anchor: UITextRange?
    private var hadKeyboard = false
    private var pinchFrom: CGFloat = 0

    override init(frame: CGRect, font: UIFont?) {
        super.init(frame: frame, font: font)
        setUp()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setUp()
    }

    private func setUp() {
        allowMouseReporting = false
        addingOwn = true
        let press = UILongPressGestureRecognizer(target: self, action: #selector(pressed(_:)))
        press.minimumPressDuration = 0.35
        addGestureRecognizer(press)
        addGestureRecognizer(UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:))))
        addingOwn = false
    }

    /// SwiftTerm's long press is turned away as it is added, in its `init`.
    /// Only that exact class: UIKit's own recognizers are private subclasses.
    override func addGestureRecognizer(_ recognizer: UIGestureRecognizer) {
        if !addingOwn, type(of: recognizer) == UILongPressGestureRecognizer.self { return }
        super.addGestureRecognizer(recognizer)
    }

    func apply(_ look: TerminalLook) {
        self.look = look
        nativeBackgroundColor = look.background
        nativeForegroundColor = look.foreground
        backgroundColor = look.background
        caretColor = look.foreground
        selectedTextBackgroundColor = look.selection
        selectedTextForegroundColor = look.foreground
        installColors(look.ansi)
        textLayer?.backgroundColor = look.background
        textLayer?.textColor = look.foreground
    }

    // MARK: Scrolling

    /// Not `super`'s: that adds the pan that drags.
    override func mouseModeChanged(source: Terminal) {
        if source.mouseMode != .off {
            guard scroll == nil else { return }
            let pan = UIPanGestureRecognizer(target: self, action: #selector(scrolled(_:)))
            // Two fingers are the pinch's.
            pan.maximumNumberOfTouches = 1
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
                if up {
                    scrolledBack = true
                    scrolledAt = .now
                }
                let flags = terminal.encodeButton(button: up ? 4 : 5, release: false, shift: false, meta: false, control: false)
                terminal.sendEvent(buttonFlags: flags, x: col, y: line)
                carried -= up ? step : -step
            }
        default:
            carried = 0
        }
    }

    override func send(source: Terminal, data: ArraySlice<UInt8>) {
        replying = true
        defer { replying = false }
        super.send(source: source, data: data)
    }

    /// Whether tmux is showing its history rather than the live screen.
    ///
    /// tmux says so in one place a client can see: copy mode draws its
    /// position, `[12/340]`, at the right of the top row. A program could draw
    /// the same, which is why this is only asked after a swipe here.
    private func inHistory() -> Bool {
        let top = getTerminal().getLine(row: 0)?.translateToString(trimRight: true) ?? ""
        return top.range(of: #"\[\d+/\d+\]\s*$"#, options: .regularExpression) != nil
    }

    private func show(history: Bool) {
        guard history != shownHistory else { return }
        shownHistory = history
        onHistory?(history)
    }

    /// After every write: whether the way back is still worth offering.
    func fed() {
        guard scrolledBack else { return }
        let now = inHistory()
        // Not yet in it is not the same as out of it: output can arrive
        // between the swipe and tmux's answer to it.
        if !now, Date.now.timeIntervalSince(scrolledAt) > 1 { scrolledBack = false }
        show(history: now)
    }

    /// Before something typed is sent: whether copy mode has to be left first,
    /// with a `q`, which cancels it in tmux's vi and emacs keys alike.
    func leavingForInput() -> Bool {
        guard !replying, scrolledBack else { return false }
        let leave = inHistory()
        scrolledBack = false
        show(history: false)
        return leave
    }

    /// Back to the live screen.
    func latest() {
        let leave = scrolledBack && inHistory()
        scrolledBack = false
        show(history: false)
        if leave { send(txt: "q") }
    }

    // MARK: Keys

    /// A key on the bar. Bytes go the way typing does, through `send(data:)`
    /// to the delegate, so a key in tmux's history leaves it first.
    func press(_ key: TerminalKey) {
        switch key.action {
        case let .modifier(m):
            armed = armed == m ? nil : m
        case .clear:
            armed = nil
            write(TerminalInput.clear)
        case let .send(data):
            typeModified(data)
        }
    }

    /// What is on the clipboard, as T3 Code pastes it: see `TerminalInput.paste`.
    func paste() {
        armed = nil
        guard let text = UIPasteboard.general.string, !text.isEmpty else { return }
        write(TerminalInput.paste(text, bracketed: getTerminal().bracketedPasteMode))
    }

    func hideKeyboard() {
        _ = resignFirstResponder()
    }

    func showKeyboard() {
        _ = becomeFirstResponder()
    }

    private func write(_ text: String) {
        send(data: ArraySlice(Array(text.utf8)))
    }

    /// Through the armed modifier, if there is one, which is then spent.
    private func typeModified(_ data: String) {
        guard let modifier = armed else { return write(data) }
        armed = nil
        switch TerminalInput.modified(data, by: modifier) {
        case let .write(bytes): write(bytes)
        case .paste: paste()
        }
    }

    /// The keyboard's typing, which an armed modifier applies to as well.
    /// SwiftTerm has a ctrl and a meta of its own for this, but its ctrl maps
    /// fewer keys, and the bar's two are one place here.
    override func insertText(_ text: String) {
        guard armed != nil, !text.isEmpty else { return super.insertText(text) }
        // Return is a carriage return to a terminal, under a modifier too.
        typeModified(text == "\n" ? "\r" : text)
    }

    /// Delete under alt is a word back, in shells and editors alike.
    override func deleteBackward() {
        guard armed != nil else { return super.deleteBackward() }
        typeModified("\u{7F}")
    }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        onKeyboard?(isFirstResponder)
        return became
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        onKeyboard?(isFirstResponder)
        return resigned
    }

    // MARK: Selecting

    /// The cell, as SwiftTerm lays it out.
    private var cell: CGSize {
        let t = getTerminal()
        let frame = getOptimalFrameSize()
        return CGSize(width: frame.width / CGFloat(max(t.cols, 1)), height: frame.height / CGFloat(max(t.rows, 1)))
    }

    private func screenText() -> [String] {
        let t = getTerminal()
        return (0 ..< t.rows).map { t.getLine(row: $0)?.translateToString(trimRight: true) ?? "" }
    }

    @objc private func pressed(_ press: UILongPressGestureRecognizer) {
        switch press.state {
        case .began:
            let text = beginSelecting()
            let point = press.location(in: text)
            guard let at = text.closestPosition(to: point) else { return }
            let word = text.tokenizer.rangeEnclosingPosition(at, with: .word, inDirection: .storage(.forward))
                ?? text.textRange(from: at, to: at)
            anchor = word
            text.selectedTextRange = word
            loupe = UITextLoupeSession.begin(at: point, fromSelectionWidgetView: nil, in: text)
        case .changed:
            guard let text = textLayer, let anchor else { return }
            let point = press.location(in: text)
            guard let at = text.closestPosition(to: point) else { return }
            let start = text.compare(at, to: anchor.start) == .orderedAscending ? at : anchor.start
            let end = text.compare(at, to: anchor.end) == .orderedDescending ? at : anchor.end
            text.selectedTextRange = text.textRange(from: start, to: end)
            loupe?.move(to: point, withCaretRect: text.caretRect(for: at), trackingCaret: false)
        default:
            loupe?.invalidate()
            loupe = nil
            anchor = nil
            guard let text = textLayer else { return }
            text.pressing = false
            if text.selectedRange.length == 0 { endSelecting() } else { text.showMenu() }
        }
    }

    /// The screen's text over the screen, where the system can select it.
    private func beginSelecting() -> SelectionTextView {
        if let textLayer { return textLayer }
        hadKeyboard = isFirstResponder

        let rows = cell.height
        let style = NSMutableParagraphStyle()
        style.minimumLineHeight = rows
        style.maximumLineHeight = rows
        // Clipped rather than wrapped: a line is never wider than the screen
        // in cells, but a wide glyph can be wider in points than its two.
        style.lineBreakMode = .byClipping
        let text = SelectionTextView()
        text.attributedText = NSAttributedString(string: screenText().joined(separator: "\n"), attributes: [
            .font: font,
            .foregroundColor: look?.foreground ?? .label,
            .paragraphStyle: style,
        ])
        text.backgroundColor = look?.background ?? .systemBackground
        text.tintColor = .tintColor
        text.frame = CGRect(origin: CGPoint(x: 0, y: contentOffset.y), size: bounds.size)
        text.onDone = { [weak self] in self?.endSelecting() }
        addSubview(text)
        _ = text.becomeFirstResponder()
        textLayer = text
        onSelecting?(true)
        return text
    }

    func endSelecting() {
        guard let text = textLayer else { return }
        textLayer = nil
        loupe?.invalidate()
        loupe = nil
        text.onDone = nil
        text.removeFromSuperview()
        onSelecting?(false)
        if hadKeyboard { _ = becomeFirstResponder() }
    }

    func copyAll() {
        var lines = screenText()
        while lines.last?.isEmpty == true { lines.removeLast() }
        UIPasteboard.general.string = lines.joined(separator: "\n")
        endSelecting()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let textLayer else { return }
        // A rotation rewraps the screen under the snapshot, and the snapshot
        // is no longer of anything. The keyboard going, which the selection
        // taking focus does, only makes the screen taller: the snapshot stays
        // where it was, and covers what has been uncovered.
        if textLayer.frame.width != bounds.width { return endSelecting() }
        textLayer.frame = CGRect(origin: CGPoint(x: 0, y: contentOffset.y), size: bounds.size)
    }

    /// While the snapshot is up, every touch is its: none of the terminal's
    /// gestures -- the swipe that scrolls, SwiftTerm's taps, this long press
    /// -- may begin under it.
    override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        if textLayer != nil, recognizer.view === self { return false }
        return super.gestureRecognizerShouldBegin(recognizer)
    }

    // MARK: Type size

    @objc private func pinched(_ pinch: UIPinchGestureRecognizer) {
        switch pinch.state {
        case .began:
            pinchFrom = font.pointSize
        case .changed:
            let size = min(max((pinchFrom * pinch.scale).rounded(), TerminalLook.sizes.lowerBound), TerminalLook.sizes.upperBound)
            if size != font.pointSize { font = .monospacedSystemFont(ofSize: size, weight: .regular) }
        case .ended:
            onFontSize?(Double(font.pointSize))
        default:
            break
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

/// The text a long press selects in: read-only, the screen's size, and gone
/// once the selection is -- copied, or tapped away.
private final class SelectionTextView: UITextView, UITextViewDelegate {
    var onDone: (() -> Void)?
    /// While the long press that made this is still down, an empty selection
    /// is a press on blank space, not a tap away.
    var pressing = true
    private let editMenu = UIEditMenuInteraction(delegate: nil)

    init() {
        super.init(frame: .zero, textContainer: nil)
        isEditable = false
        isSelectable = true
        isScrollEnabled = false
        textContainerInset = .zero
        textContainer.lineFragmentPadding = 0
        dataDetectorTypes = []
        delegate = self
        addInteraction(editMenu)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func showMenu() {
        guard let range = selectedTextRange, !range.isEmpty else { return }
        let rect = firstRect(for: range)
        editMenu.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: CGPoint(x: rect.midX, y: rect.minY)))
    }

    override func copy(_ sender: Any?) {
        super.copy(sender)
        onDone?()
    }

    /// Not passed up to the terminal underneath, whose Paste and Select would
    /// otherwise join the menu: a snapshot is for reading, and its selection
    /// is this one.
    override func target(forAction action: Selector, withSender sender: Any?) -> Any? {
        if action == #selector(paste(_:)) || action == #selector(select(_:)) { return nil }
        return super.target(forAction: action, withSender: sender)
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        guard !pressing, selectedRange.length == 0 else { return }
        // After the tap that cleared it has finished with this view.
        DispatchQueue.main.async { [weak self] in self?.onDone?() }
    }
}
