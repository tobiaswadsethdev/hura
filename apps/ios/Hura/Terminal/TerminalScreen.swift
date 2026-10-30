import HuraKit
import SwiftTerm
import SwiftUI
import UIKit

/// A tmux session in a sandbox: the agent's own screen in a terminal session,
/// or a shell beside it. SwiftTerm draws it; its keyboard accessory carries
/// the keys a phone lacks (esc, ctrl, tab, arrows).
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
        let view = TerminalView(frame: .zero, font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
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
