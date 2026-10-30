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

    private static func split(_ s: String) -> [String] {
        guard !s.isEmpty else { return [] }
        var parts = s.components(separatedBy: "\n")
        if parts.last == "" { parts.removeLast() }
        return parts
    }
}
