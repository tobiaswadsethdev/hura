import HuraKit
import SwiftUI

/// A project's scripts, to add, change and take away: a name and the command
/// to run, each in a shell of its own from the worktree's menu. Kept on this
/// phone (`ProjectScripts`), as the desktop keeps its own in the window.
struct ScriptsEditor: View {
    let project: String
    @State var scripts: [ProjectScript]
    let save: ([ProjectScript]) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach($scripts) { $s in
                        VStack(alignment: .leading, spacing: 6) {
                            TextField("Name", text: $s.name)
                                .font(.body.weight(.medium))
                            TextField("Command, such as npm run dev", text: $s.command, axis: .vertical)
                                .font(.callout.monospaced())
                                .foregroundStyle(Color.dim)
                                .autocorrectionDisabled()
                                .textInputAutocapitalization(.never)
                        }
                        .padding(.vertical, 4)
                    }
                    .onDelete { scripts.remove(atOffsets: $0) }
                    .onMove { scripts.move(fromOffsets: $0, toOffset: $1) }
                    Button("Add Script", systemImage: "plus") {
                        scripts.append(ProjectScript(name: "", command: ""))
                    }
                } footer: {
                    Text("Each runs in a new shell in this project's worktrees, from the worktree's menu. Kept on this iPhone.")
                }
                .listRowBackground(Color.surface)
            }
            .scrollContentBackground(.hidden)
            .background(Color.canvas)
            .navigationTitle("\(project) Scripts")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        save(scripts.filter { !$0.command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
                        dismiss()
                    }
                }
            }
        }
    }
}
