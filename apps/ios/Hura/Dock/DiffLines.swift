import HuraKit
import SwiftUI

/// Both sides of a file as one column, worked out once: every line, folded
/// or not, the runs of unchanged ones folded, and the part of each edited
/// line that changed.
struct DiffModel {
    let all: [UnifiedDiff.Line]
    let pieces: [UnifiedDiff.Piece]
    let spans: [String: Range<Int>]

    init(original: String, modified: String) {
        all = UnifiedDiff.lines(original: original, modified: modified)
        pieces = UnifiedDiff.pieces(all)
        spans = UnifiedDiff.changedSpans(all)
    }

    /// The first line of each run of changes on screen, to step between.
    var changes: [String] {
        var out: [String] = []
        var previous: UnifiedDiff.Line.Kind = .same
        for piece in pieces {
            guard case let .lines(lines) = piece else { previous = .same; continue }
            for line in lines {
                if line.kind != .same, previous == .same { out.append(line.id) }
                previous = line.kind
            }
        }
        return out
    }

    /// The change after (or before) `from`, for the previous and next buttons.
    func step(_ by: Int, from: String?) -> String? {
        let all = changes
        guard !all.isEmpty else { return nil }
        let at = from.flatMap { all.firstIndex(of: $0) } ?? (by > 0 ? -1 : all.count)
        return all[min(max(at + by, 0), all.count - 1)]
    }
}

/// Wrap Lines and the text size, for a diff's menu. Remembered across every
/// diff, the session's and a pull request's alike.
struct DiffStyleMenu: View {
    @AppStorage("diff.wrap") private var wrap = false
    @AppStorage("diff.fontSize") private var fontSize: Double = 12

    var body: some View {
        Toggle("Wrap Lines", isOn: $wrap)
        Section("Text Size") {
            Button("Larger", systemImage: "textformat.size.larger") { fontSize = min(fontSize + 1, 20) }
                .disabled(fontSize >= 20)
            Button("Smaller", systemImage: "textformat.size.smaller") { fontSize = max(fontSize - 1, 8) }
                .disabled(fontSize <= 8)
        }
    }
}

/// The measures a diff is drawn to, in the font it is drawn in.
struct DiffLayout {
    var gutter: CGFloat
    var width: CGFloat
    var char: CGFloat

    init(diff lines: [UnifiedDiff.Line], size: CGFloat, wrap: Bool, viewport: CGFloat) {
        char = ("M" as NSString).size(withAttributes: [.font: UIFont.monospacedSystemFont(ofSize: size, weight: .regular)]).width
        let digits = String(lines.map { max($0.old ?? 0, $0.new ?? 0) }.max() ?? 0).count
        gutter = CGFloat(max(digits, 2)) * char + 6
        let longest = lines.map { DiffLayout.shown($0.text).count }.max() ?? 0
        let text = CGFloat(longest + 2) * char
        width = wrap ? viewport : max(viewport, 2 * gutter + 2 * char + text + 12)
    }

    /// Where a line's text starts, for what is drawn under it.
    var indent: CGFloat { 2 * gutter + 2 * char }

    /// Tabs as four spaces: a monospaced column is only as good as every
    /// character being one wide.
    static func shown(_ text: String) -> String {
        text.replacingOccurrences(of: "\t", with: "    ")
    }
}

/// A diff in one column: both line numbers, the line, its colour run to the
/// edge, and the part of an edited line that changed. It scrolls sideways or
/// wraps, folds long runs of unchanged lines, and scrolls to `jump`. `below`
/// draws what goes under a line (the review's remarks); `tap` is a tap on one.
struct DiffLines<Below: View>: View {
    let model: DiffModel
    @Binding var jump: String?
    var tap: ((UnifiedDiff.Line) -> Void)?
    @ViewBuilder var below: (UnifiedDiff.Line, DiffLayout) -> Below
    @State private var unfolded: Set<Int> = []
    @AppStorage("diff.wrap") private var wrap = false
    @AppStorage("diff.fontSize") private var fontSize: Double = 12

    var body: some View {
        GeometryReader { box in
            let layout = DiffLayout(diff: model.all, size: fontSize, wrap: wrap, viewport: box.size.width)
            ScrollViewReader { reader in
                ScrollView(wrap ? .vertical : [.vertical, .horizontal]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(model.pieces) { piece in
                            switch piece {
                            case let .lines(lines):
                                ForEach(lines) { line in row(line, layout) }
                            case let .folded(count, from):
                                if unfolded.contains(from) {
                                    ForEach(model.all[from..<(from + count)]) { line in row(line, layout) }
                                } else {
                                    Button("⋯ \(count) unchanged line\(count == 1 ? "" : "s")") { unfolded.insert(from) }
                                        .font(.caption)
                                        .padding(.vertical, 6)
                                        .padding(.leading, 10)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .background(Color.secondary.opacity(0.08))
                                }
                            }
                        }
                    }
                    // A width the rows can fill: the longest line's when it
                    // scrolls sideways, the screen's when it wraps. Without
                    // one a lazy stack is as wide as what it has drawn so far,
                    // and nothing scrolls.
                    .frame(width: layout.width, alignment: .leading)
                    .font(.system(size: fontSize, design: .monospaced))
                    .padding(.vertical, 6)
                }
                .defaultScrollAnchor(.topLeading)
                .onChange(of: jump) { _, target in
                    guard let target else { return }
                    withAnimation { reader.scrollTo(target, anchor: UnitPoint(x: 0, y: 0.2)) }
                }
            }
        }
    }

    private func attributed(_ line: UnifiedDiff.Line) -> AttributedString {
        let text = DiffLayout.shown(line.text)
        var out = AttributedString(text.isEmpty ? " " : text)
        // The span is in the line's own characters; a tab before it moves it.
        if let span = model.spans[line.id], !line.text.contains("\t"), span.upperBound <= text.count, !span.isEmpty {
            let lower = out.index(out.startIndex, offsetByCharacters: span.lowerBound)
            let upper = out.index(out.startIndex, offsetByCharacters: span.upperBound)
            out[lower..<upper].backgroundColor = line.kind == .added ? Color.green.opacity(0.4) : Color.red.opacity(0.4)
        }
        return out
    }

    @ViewBuilder
    private func row(_ line: UnifiedDiff.Line, _ layout: DiffLayout) -> some View {
        HStack(alignment: .top, spacing: 0) {
            Text(line.old.map(String.init) ?? "").frame(width: layout.gutter, alignment: .trailing).foregroundStyle(.tertiary)
            Text(line.new.map(String.init) ?? "").frame(width: layout.gutter, alignment: .trailing).foregroundStyle(.tertiary)
            Text(line.kind == .added ? "+" : line.kind == .removed ? "-" : " ")
                .foregroundStyle(line.kind == .added ? .green : line.kind == .removed ? .red : .secondary)
                .frame(width: layout.char * 2)
            Text(attributed(line))
                .fixedSize(horizontal: !wrap, vertical: true)
                .frame(maxWidth: wrap ? .infinity : nil, alignment: .leading)
        }
        .padding(.vertical, 1)
        .padding(.trailing, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(line.kind == .added ? Color.green.opacity(0.15) : line.kind == .removed ? Color.red.opacity(0.15) : .clear)
        .contentShape(Rectangle())
        .id(line.id)
        .onTapGesture { tap?(line) }
        below(line, layout)
    }
}

extension DiffLines where Below == EmptyView {
    init(model: DiffModel, jump: Binding<String?>) {
        self.init(model: model, jump: jump, tap: nil) { _, _ in EmptyView() }
    }
}
