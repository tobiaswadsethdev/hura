import HuraKit
import SwiftUI

/// Claude's replies as Markdown: paragraphs, lists and inline styling through
/// AttributedString; headings, tables and fenced code blocks as their own
/// views.
struct MarkdownText: View {
    let text: String
    @Environment(\.download) private var download

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(Self.segments(text).enumerated()), id: \.offset) { _, segment in
                switch segment {
                case let .prose(p):
                    Text(download == nil ? Self.attributed(p) : Self.linkingPaths(Self.attributed(p)))
                        .fixedSize(horizontal: false, vertical: true)
                case let .heading(level, h):
                    Text(Self.attributed(h))
                        .font(level <= 2 ? .title3.weight(.semibold) : .headline)
                        .padding(.top, 4)
                case let .code(c):
                    CodeBlock(text: c)
                case let .table(rows):
                    TableView(rows: rows)
                case let .image(alt, src):
                    MarkdownPicture(alt: alt, src: src)
                }
            }
        }
        .textSelection(.enabled)
        .environment(\.openURL, OpenURLAction { url in
            guard url.scheme == Self.downloadScheme, let download else { return .systemAction }
            download(url.path)
            return .handled
        })
    }

    /// The scheme of a link that downloads a path rather than opening a page.
    nonisolated static let downloadScheme = "hura-download"

    /// Code that is nothing but an absolute path, a file's or a folder's, as a
    /// link that takes it out of the sandbox: that is how an agent says where
    /// it put something -- `/tmp/report.md` -- and where someone looks for it.
    nonisolated static func linkingPaths(_ text: AttributedString) -> AttributedString {
        var text = text
        let code = text.runs.filter { $0.inlinePresentationIntent?.contains(.code) == true }.map(\.range)
        for range in code {
            let path = String(text[range].characters)
            guard Fetched.isPath(path) else { continue }
            var url = URLComponents()
            url.scheme = downloadScheme
            url.path = path
            text[range].link = url.url
        }
        return text
    }

    enum Segment: Equatable {
        case prose(String)
        case heading(Int, String)
        case code(String)
        case table([[String]])
        /// `![alt](src)` on a line of its own.
        case image(alt: String, src: String)
    }

    /// Splits on ``` fences. An unclosed fence (a reply still being
    /// written) runs to the end.
    nonisolated static func segments(_ text: String) -> [Segment] {
        var out: [Segment] = []
        var prose: [String] = [], code: [String] = []
        var inCode = false
        func flushProse() {
            let p = prose.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !p.isEmpty { out.append(.prose(p)) }
            prose = []
        }
        let lines = text.components(separatedBy: "\n")
        var i = 0
        while i < lines.count {
            let line = lines[i]
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            i += 1
            if !inCode, let (level, h) = heading(trimmed) {
                flushProse()
                out.append(.heading(level, h))
                continue
            }
            if !inCode, let (alt, src) = image(trimmed) {
                flushProse()
                out.append(.image(alt: alt, src: src))
                continue
            }
            if !inCode, trimmed.hasPrefix("|"), i < lines.count, isSeparator(lines[i]) {
                flushProse()
                var rows = [cells(trimmed)]
                i += 1 // the separator
                while i < lines.count, lines[i].trimmingCharacters(in: .whitespaces).hasPrefix("|") {
                    rows.append(cells(lines[i]))
                    i += 1
                }
                out.append(.table(rows))
                continue
            }
            if trimmed.hasPrefix("```") {
                if inCode {
                    out.append(.code(code.joined(separator: "\n")))
                    code = []
                } else {
                    flushProse()
                }
                inCode.toggle()
            } else if inCode {
                code.append(line)
            } else {
                prose.append(line)
            }
        }
        if inCode { out.append(.code(code.joined(separator: "\n"))) }
        flushProse()
        return out
    }

    nonisolated static func heading(_ line: String) -> (Int, String)? {
        let hashes = line.prefix { $0 == "#" }.count
        guard (1...6).contains(hashes), line.dropFirst(hashes).first == " " else { return nil }
        return (hashes, String(line.dropFirst(hashes + 1)))
    }

    /// `![alt](src)`, or `![alt](src "title")`, and nothing else on the line.
    nonisolated static func image(_ line: String) -> (String, String)? {
        guard line.hasPrefix("!["), line.hasSuffix(")"), let close = line.range(of: "](") else { return nil }
        let alt = String(line[line.index(line.startIndex, offsetBy: 2)..<close.lowerBound])
        var src = String(line[close.upperBound..<line.index(before: line.endIndex)])
        if src.hasPrefix("<"), let end = src.firstIndex(of: ">") {
            src = String(src[src.index(after: src.startIndex)..<end])
        } else if let space = src.firstIndex(of: " ") {
            src = String(src[..<space])
        }
        return src.isEmpty || alt.contains("]") ? nil : (alt, src)
    }

    nonisolated static func isSeparator(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        return t.hasPrefix("|") && t.contains("-") && t.allSatisfy { "|-: ".contains($0) }
    }

    nonisolated static func cells(_ line: String) -> [String] {
        var t = line.trimmingCharacters(in: .whitespaces)
        if t.hasPrefix("|") { t.removeFirst() }
        if t.hasSuffix("|") { t.removeLast() }
        return t.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    nonisolated static func attributed(_ s: String) -> AttributedString {
        // Headings and list markers stay as typed; inline styling, code and
        // links are rendered.
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(s)
    }
}

struct CodeBlock: View {
    let text: String

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(text)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(10)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.sunken, in: RoundedRectangle(cornerRadius: Radius.control))
        .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line, lineWidth: 1))
    }
}

struct TableView: View {
    let rows: [[String]]

    var body: some View {
        let columns = rows.map(\.count).max() ?? 0
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
                ForEach(Array(rows.enumerated()), id: \.offset) { r, row in
                    GridRow {
                        ForEach(0..<columns, id: \.self) { c in
                            Text(MarkdownText.attributed(c < row.count ? row[c] : ""))
                                .fontWeight(r == 0 ? .semibold : .regular)
                        }
                    }
                    if r == 0 { Divider() }
                }
            }
            .font(.subheadline)
            .padding(10)
        }
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
    }
}
