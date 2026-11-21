import HuraKit
import SwiftUI
import UIKit

// Taking a file out of a sandbox onto the phone: a screenshot the agent took,
// a report it wrote, or a folder of them as a zip the server makes. Fetched
// whole, written under its own name, and handed to
// the share sheet, which is where Save to Files, Photos and AirDrop already
// are. The desktop saves into Downloads instead (`Download.tsx`).

extension EnvironmentValues {
    /// Takes a path out of the worktree's sandbox: relative to the working
    /// copy, or absolute anywhere in it. Unset where there is no worktree,
    /// and then nothing offers to.
    @Entry var download: (@MainActor (String) -> Void)?
}

extension View {
    /// `download` for one worktree, and the share sheet it ends in.
    func downloads(server: Server, name: String) -> some View {
        modifier(Downloads(server: server, name: name))
    }
}

private struct Downloads: ViewModifier {
    let server: Server
    let name: String
    /// The file on its way, by name; one at a time.
    @State private var fetching: String?
    @State private var shared: Shared?
    @State private var error: String?

    func body(content: Content) -> some View {
        content
            .environment(\.download) { path in start(path) }
            // A few megabytes from a sandbox on another machine is long enough
            // to wonder whether the tap took.
            .overlay(alignment: .bottom) {
                if let fetching {
                    HStack(spacing: 8) {
                        ProgressView()
                        Text("Downloading \(fetching)…").font(.subheadline).lineLimit(1)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .glass()
                    .padding(.bottom, 12)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .animation(.snappy, value: fetching)
            .sheet(item: $shared, onDismiss: clean) { s in
                ShareSheet(url: s.url).ignoresSafeArea()
            }
            .alert("Couldn't download that", isPresented: .constant(error != nil)) {
                Button("OK") { error = nil }
            } message: {
                Text(error ?? "")
            }
    }

    private func start(_ path: String) {
        guard fetching == nil else { return }
        fetching = (path as NSString).lastPathComponent
        Task {
            defer { fetching = nil }
            do {
                let file = try await server.api.download(name, path: path)
                shared = Shared(url: try file.save(in: Self.folder))
            } catch {
                self.error = error.userMessage
            }
        }
    }

    /// The copy was only ever for the share sheet; whatever it was saved to
    /// has its own now.
    private func clean() {
        try? FileManager.default.removeItem(at: Self.folder)
    }

    private static var folder: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("downloads", isDirectory: true)
    }
}

private struct Shared: Identifiable {
    let url: URL
    var id: URL { url }
}

/// The system's share sheet, for a file: SwiftUI's `ShareLink` wants the item
/// before it is tapped, and this one is only fetched once it has been.
private struct ShareSheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}

/// A path typed or pasted in, file or folder, anywhere in the sandbox: for
/// what the agent made outside the working copy -- in `/tmp`, say -- and never
/// named in the conversation, where it would have been a link.
struct DownloadPathRow: View {
    @Environment(\.download) private var download
    @State private var asking = false
    @State private var path = ""

    var body: some View {
        if let download {
            Button { asking = true } label: {
                Label("Download a Path…", systemImage: "square.and.arrow.down")
                    .foregroundStyle(.secondary)
            }
            .alert("Download a Path", isPresented: $asking) {
                TextField("/tmp/report.md", text: $path)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.body.monospaced())
                Button("Cancel", role: .cancel) {}
                Button("Download") {
                    let p = path.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !p.isEmpty { download(p) }
                }
            } message: {
                Text("A file, or a folder as a zip, anywhere in the sandbox.")
            }
        }
    }
}

/// "Download" for a menu, which takes `path` out of the sandbox. A view of its
/// own so it reads `download` from where it is drawn: a view cannot read what
/// its own body puts in the environment.
struct DownloadButton: View {
    let path: String
    /// Says it comes as a zip.
    var folder = false
    @Environment(\.download) private var download

    var body: some View {
        if let download {
            Button(folder ? "Download as Zip" : "Download", systemImage: "square.and.arrow.down") { download(path) }
        }
    }
}
