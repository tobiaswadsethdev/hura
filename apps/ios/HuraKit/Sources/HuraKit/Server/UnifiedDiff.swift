import Foundation

/// A file's two sides as one column, for a screen too narrow for two: what
/// both have, what only the original has, and what only the modified side
/// has, each with its line numbers. The desktop draws the same diff side by
/// side in Monaco.
public enum UnifiedDiff {
    public struct Line: Equatable, Identifiable, Sendable {
        public enum Kind: Sendable { case same, removed, added }
        public var kind: Kind
        /// 1-based, on the side it is on.
        public var old: Int?
        public var new: Int?
        public var text: String
        public var id: String { "\(old ?? 0):\(new ?? 0)" }
    }

    /// A run of lines, or a gap of unchanged lines folded away.
    public enum Piece: Equatable, Identifiable, Sendable {
        case lines([Line])
        case folded(count: Int, from: Int)

        public var id: String {
            switch self {
            case let .lines(l): "l\(l.first?.id ?? "")"
            case let .folded(_, from): "f\(from)"
            }
        }
    }

    public static func lines(original: String, modified: String) -> [Line] {
        let a = split(original)
        let b = split(modified)
        let diff = b.difference(from: a)
        var removed = Set<Int>()
        var inserted = Set<Int>()
        for change in diff {
            switch change {
            case let .remove(offset, _, _): removed.insert(offset)
            case let .insert(offset, _, _): inserted.insert(offset)
            }
        }
        var out: [Line] = []
        var i = 0, j = 0
        while i < a.count || j < b.count {
            if i < a.count, removed.contains(i) {
                out.append(Line(kind: .removed, old: i + 1, new: nil, text: a[i]))
                i += 1
            } else if j < b.count, inserted.contains(j) {
                out.append(Line(kind: .added, old: nil, new: j + 1, text: b[j]))
                j += 1
            } else if i < a.count, j < b.count {
                out.append(Line(kind: .same, old: i + 1, new: j + 1, text: b[j]))
                i += 1
                j += 1
            } else {
                break
            }
        }
        return out
    }

    /// The changes with `context` unchanged lines around each, and the rest
    /// folded.
    public static func pieces(_ lines: [Line], context: Int = 3) -> [Piece] {
        let changed = lines.indices.filter { lines[$0].kind != .same }
        guard !changed.isEmpty else { return lines.isEmpty ? [] : [.folded(count: lines.count, from: 0)] }
        var keep = [Bool](repeating: false, count: lines.count)
        for c in changed {
            for k in max(0, c - context)...min(lines.count - 1, c + context) { keep[k] = true }
        }
        var out: [Piece] = []
        var run: [Line] = []
        var gap = 0, gapFrom = 0
        for (k, line) in lines.enumerated() {
            if keep[k] {
                if gap > 0 { out.append(.folded(count: gap, from: gapFrom)); gap = 0 }
                run.append(line)
            } else {
                if !run.isEmpty { out.append(.lines(run)); run = [] }
                if gap == 0 { gapFrom = k }
                gap += 1
            }
        }
        if !run.isEmpty { out.append(.lines(run)) }
        if gap > 0 { out.append(.folded(count: gap, from: gapFrom)) }
        return out
    }

    /// Where an edited line changed, for the lines that are edits rather than
    /// whole new or gone ones: a run of removed lines followed by a run of
    /// added ones is paired off in order, and each pair's changed span is
    /// what lies between their common start and common end. Keyed by line
    /// id, in Characters. A pair with nothing in common is left out: the
    /// whole line is the change, and its colour already says so.
    public static func changedSpans(_ lines: [Line]) -> [String: Range<Int>] {
        var out: [String: Range<Int>] = [:]
        var k = 0
        while k < lines.count {
            guard lines[k].kind == .removed else { k += 1; continue }
            var removed: [Line] = []
            while k < lines.count, lines[k].kind == .removed { removed.append(lines[k]); k += 1 }
            var added: [Line] = []
            while k < lines.count, lines[k].kind == .added { added.append(lines[k]); k += 1 }
            for (old, new) in zip(removed, added) {
                guard let (a, b) = span(Array(old.text), Array(new.text)) else { continue }
                out[old.id] = a
                out[new.id] = b
            }
        }
        return out
    }

    /// The differing middles of two lines, or nil when they share nothing at
    /// either end.
    static func span(_ a: [Character], _ b: [Character]) -> (Range<Int>, Range<Int>)? {
        var start = 0
        while start < a.count, start < b.count, a[start] == b[start] { start += 1 }
        var end = 0
        while end < a.count - start, end < b.count - start, a[a.count - 1 - end] == b[b.count - 1 - end] { end += 1 }
        guard start + end > 0 else { return nil }
        return (start..<(a.count - end), start..<(b.count - end))
    }

    private static func split(_ s: String) -> [String] {
        guard !s.isEmpty else { return [] }
        var parts = s.components(separatedBy: "\n")
        if parts.last == "" { parts.removeLast() }
        return parts
    }
}
