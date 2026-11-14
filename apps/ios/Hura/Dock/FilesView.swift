import HuraKit
import SwiftUI

/// One directory of the working copy. Every listing is an exec into the
/// sandbox, so the tree is read a directory at a time, as it is opened.
struct DirView: View {
    let server: Server
    let name: String
    let path: String
    @State private var dir: Dir?
    @State private var error: String?

    var body: some View {
        Loaded(value: dir, error: error) { dir in
            List {
                if dir.entries.isEmpty {
                    Text("Empty").foregroundStyle(.secondary)
                }
                ForEach(sorted(dir.entries), id: \.self) { e in
                    let full = path.isEmpty ? e.name : "\(path)/\(e.name)"
                    NavigationLink {
                        Group {
                            if e.dir {
                                DirView(server: server, name: name, path: full)
                                    .navigationTitle(e.name)
                                    .navigationBarTitleDisplayMode(.inline)
                            } else {
                                FileView(server: server, name: name, path: full)
                            }
                        }
                        // Pushed over a session, which hides the tabs.
                        .toolbar(.hidden, for: .tabBar)
                    } label: {
                        Label(e.name, glyph: e.dir ? .folder : icon(e.name))
                    }
                    .contextMenu {
                        Button("Copy Path", systemImage: "doc.on.doc") { UIPasteboard.general.string = full }
                    }
                }
            }
            .listStyle(.plain)
        }
        .refreshable { await load() }
        .task { await load() }
    }

    private func sorted(_ entries: [Dir.Entry]) -> [Dir.Entry] {
        entries.sorted { ($0.dir ? 0 : 1, $0.name.lowercased()) < ($1.dir ? 0 : 1, $1.name.lowercased()) }
    }

    private func load() async {
        do {
            dir = try await server.api.files(name, path: path)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

func icon(_ file: String) -> Glyph {
    switch (file as NSString).pathExtension.lowercased() {
    case "md", "txt": .fileText
    case "json", "toml", "yaml", "yml", "lock": .fileConfig
    case "png", "jpg", "jpeg", "gif", "svg", "webp": .fileImage
    default: .file
    }
}

/// A file of the working copy, read-only: the agent owns it.
struct FileView: View {
    let server: Server
    let name: String
    let path: String
    @State private var file: FileText?
    @State private var error: String?
    @State private var wrap = false

    var body: some View {
        Loaded(value: file, error: error) { file in
            if let image = file.image {
                ScrollView {
                    VStack(spacing: 8) {
                        PictureView(picture: image, maxHeight: .infinity)
                        Text(ByteCountFormatter.string(fromByteCount: Int64(file.bytes), countStyle: .file))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .padding()
                }
            } else if file.binary {
                ContentUnavailableView("Binary file", image: Glyph.binary.name,
                                       description: Text("\(file.bytes) bytes, so there's nothing to show."))
            } else {
                VStack(spacing: 0) {
                    if file.truncated {
                        Text("Showing the first part; it is \(file.bytes) bytes.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity)
                            .padding(6)
                            .background(.bar)
                    }
                    CodeText(text: file.text, wrap: wrap)
                }
            }
        }
        .navigationTitle((path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Toggle("Wrap Lines", isOn: $wrap)
                    Button("Copy Path", systemImage: "doc.on.doc") { UIPasteboard.general.string = path }
                    if let file, !file.binary {
                        Button("Copy Contents", systemImage: "doc.on.clipboard") { UIPasteboard.general.string = file.text }
                    }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .task {
            do { file = try await server.api.file(name, path: path) } catch { self.error = error.localizedDescription }
        }
    }
}

/// Text with line numbers, monospaced, scrolling both ways unless it wraps.
struct CodeText: View {
    let text: String
    let wrap: Bool

    private var lines: [String] {
        var l = text.components(separatedBy: "\n")
        if l.last == "" { l.removeLast() }
        return l
    }

    var body: some View {
        let lines = self.lines
        let width = String(lines.count).count
        ScrollView(wrap ? .vertical : [.vertical, .horizontal]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                    CodeLine(number: String(i + 1).leftPad(width), text: line, wrap: wrap)
                }
            }
            // As wide as the widest line, said rather than left to the stack:
            // a lazy one measures only the rows it has made, and settles on
            // the screen's width, with nothing past it to scroll to.
            .frame(minWidth: wrap ? nil : Self.width(of: lines, numbers: width), alignment: .leading)
            .font(.system(size: 12, design: .monospaced))
            .textSelection(.enabled)
            .padding(10)
        }
        // A scroll view that scrolls both ways centres what's smaller than it.
        .defaultScrollAnchor(.topLeading)
    }

    /// The font the lines are drawn in, to measure them with.
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    /// How wide a row is for the widest of `lines`, numbered with `numbers`
    /// digits. Measured rather than counted, for tabs and wide characters,
    /// but only the longest few, which are the only ones it can be.
    static func width(of lines: [String], numbers: Int) -> CGFloat {
        let attributes: [NSAttributedString.Key: Any] = [.font: font]
        let longest = lines.indices.sorted { lines[$0].utf16.count > lines[$1].utf16.count }.prefix(20)
        let text = longest.map { (lines[$0] as NSString).size(withAttributes: attributes).width }.max() ?? 0
        let number = (String(repeating: "0", count: numbers) as NSString).size(withAttributes: attributes).width
        return ceil(number + 10 + text) + 2
    }
}

/// One line: its number, and its text.
///
/// A line wider than the screen is the one to look after. Its number keeps
/// its own width, or the stack squeezes it to make room and it breaks onto a
/// line of its own; and unwrapped, the whole row keeps its full width, or
/// what overflows is centred and the text starts left of every other line.
struct CodeLine: View {
    let number: String
    let text: String
    let wrap: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(number)
                .foregroundStyle(.tertiary)
                .fixedSize()
            Text(text.isEmpty ? " " : text)
                .fixedSize(horizontal: !wrap, vertical: true)
                .frame(maxWidth: wrap ? .infinity : nil, alignment: .leading)
        }
        .fixedSize(horizontal: !wrap, vertical: false)
    }
}

extension String {
    func leftPad(_ width: Int) -> String {
        count >= width ? self : String(repeating: " ", count: width - count) + self
    }
}
