import HuraKit
import SwiftUI

/// Making a project: picking a repository from the server's disk, once per
/// repository. The checkouts are the server's, from its `repo_roots`.
struct NewProjectView: View {
    let server: Server
    @Environment(\.dismiss) private var dismiss
    @State private var listing: Listing?
    @State private var query = ""
    @State private var busy = false
    @State private var error: String?

    private var rows: [LocalRepo] {
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard let repos = listing?.repos else { return [] }
        return needle.isEmpty ? repos : repos.filter { $0.display.lowercased().contains(needle) }
    }

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section { Text(error).foregroundStyle(.red).font(.callout) }
                }
                if let listing {
                    if listing.repos.isEmpty {
                        ContentUnavailableView {
                            Label("No repositories", systemImage: "folder")
                        } description: {
                            Text("Nothing in \(listing.roots.isEmpty ? "any configured root" : listing.roots.joined(separator: ", ")). `repo_roots` in the server's config file is where it looks.")
                        }
                    }
                    ForEach(rows) { repo in
                        Button {
                            pick(repo)
                        } label: {
                            RepoRow(repo: repo)
                        }
                        .tint(.primary)
                        .disabled(repo.origin == nil || busy)
                    }
                } else if error == nil {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Filter repositories")
            .navigationTitle("New Project")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .task {
                do { listing = try await server.api.repos() } catch { self.error = error.localizedDescription }
            }
        }
    }

    private func pick(_ repo: LocalRepo) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await server.newProject(from: repo)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

private struct RepoRow: View {
    let repo: LocalRepo

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(repo.name).font(.body.weight(.medium)).foregroundStyle(.primary)
                Spacer()
                Text(repo.branch ?? "detached").font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Text(repo.display).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            // A checkout with no origin has nothing for the sandbox to clone.
            Text(repo.origin ?? "No origin, so it can't be cloned")
                .font(.caption2.monospaced())
                .foregroundStyle(repo.origin == nil ? .orange : .secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}
