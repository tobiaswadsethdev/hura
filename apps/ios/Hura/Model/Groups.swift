import Foundation

/// A group of related worktrees within a project, as the desktop has them
/// (`apps/desktop/src/groups.ts`): a feature split across several, or a batch
/// of tickets worked together. Named and foldable, and only a way of arranging
/// the list.
struct WorktreeGroup: Codable, Equatable, Identifiable {
    let id: String
    var name: String
    /// Shut, so the list says only how many it holds and how many need you.
    /// Kept with the group, so it goes when the group does.
    var folded = false
}

/// One server's groups, and where they are kept: in this phone's defaults,
/// per server, the way the triage is (`TriageBook`). A project's groups are in
/// the order they were made, and each worktree's group is kept by its session
/// name.
///
/// A worktree whose group has gone, or that has gone itself, is simply not in
/// one, as on the desktop: nothing has to be tidied for the list to be right.
struct GroupBook: Codable, Equatable {
    /// By project name.
    private(set) var groups: [String: [WorktreeGroup]] = [:]
    /// Each worktree's group, by session name.
    private(set) var of: [String: String] = [:]

    func groups(in project: String) -> [WorktreeGroup] { groups[project] ?? [] }

    /// The group a worktree is in, when it is one of its project's. One kept
    /// for a group that has been removed, or of another project, counts for
    /// nothing.
    func group(of session: String, in project: String?) -> String? {
        guard let id = of[session], let project, groups(in: project).contains(where: { $0.id == id }) else { return nil }
        return id
    }

    /// Makes a group at the end of the project's, with these worktrees in it,
    /// and answers its id.
    @discardableResult
    mutating func create(in project: String, named name: String, with sessions: [String] = [],
                         id: String = UUID().uuidString) -> String {
        groups[project, default: []].append(WorktreeGroup(id: id, name: Self.tidy(name)))
        for session in sessions { of[session] = id }
        return id
    }

    /// A name left empty keeps the old one, as leaving the desktop's field
    /// empty does.
    mutating func rename(_ id: String, in project: String, to name: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        edit(id, in: project) { $0.name = name }
    }

    mutating func fold(_ id: String, in project: String) {
        edit(id, in: project) { $0.folded.toggle() }
    }

    /// The group goes; its worktrees stay, back among the project's own.
    mutating func remove(_ id: String, in project: String) {
        groups[project]?.removeAll { $0.id == id }
        if groups[project]?.isEmpty ?? false { groups[project] = nil }
        of = of.filter { $0.value != id }
    }

    /// Puts a worktree in a group, which takes it out of the one it was in, or
    /// with `nil` out of any.
    mutating func assign(_ session: String, to id: String?) {
        of[session] = id
    }

    /// Joining from a worktree's menu: picking the group it is already in
    /// takes it out again, as the desktop's menu does.
    mutating func toggle(_ session: String, in id: String, project: String) {
        assign(session, to: group(of: session, in: project) == id ? nil : id)
    }

    /// Forgets one: a worktree that has been deleted.
    mutating func forget(_ session: String) {
        of[session] = nil
    }

    /// A new group left unnamed is called what the desktop calls it, since a
    /// header with no name has nothing to read.
    private static func tidy(_ name: String) -> String {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? "New group" : name
    }

    private mutating func edit(_ id: String, in project: String, _ change: (inout WorktreeGroup) -> Void) {
        guard let i = groups[project]?.firstIndex(where: { $0.id == id }) else { return }
        change(&groups[project]![i])
    }

    private static func key(_ server: String) -> String { "hura.groups.\(server)" }

    static func load(server: String, from defaults: UserDefaults = .standard) -> GroupBook {
        guard let data = defaults.data(forKey: key(server)),
              let book = try? JSONDecoder().decode(GroupBook.self, from: data)
        else { return GroupBook() }
        return book
    }

    func save(server: String, to defaults: UserDefaults = .standard) {
        // The list works without it; it just will not remember.
        guard let data = try? JSONEncoder().encode(self) else { return }
        defaults.set(data, forKey: Self.key(server))
    }
}

// MARK: Arranging a project's worktrees

/// A project's worktrees as its folder draws them: its groups first, each
/// with the worktrees in it, then the ones in none.
struct Arrangement<Row> {
    struct Grouped: Identifiable {
        let group: WorktreeGroup
        let rows: [Row]
        var id: String { group.id }
    }

    let groups: [Grouped]
    let rest: [Row]
}

extension GroupBook {
    /// Sorts the rows a folder holds into its project's groups, keeping their
    /// order within each. The rows are the ones the folder draws: a pinned or
    /// settled worktree is on its shelf rather than in its group, as on the
    /// desktop, so it is left out before this is asked.
    func arrange<Row>(_ rows: [Row], in project: String?, name: (Row) -> String) -> Arrangement<Row> {
        let mine = project.map(groups(in:)) ?? []
        let grouped = mine.map { g in
            Arrangement.Grouped(group: g, rows: rows.filter { group(of: name($0), in: project) == g.id })
        }
        return Arrangement(groups: grouped, rest: rows.filter { group(of: name($0), in: project) == nil })
    }

    /// The order to send the server after a drag within one group, or among
    /// the ungrouped: `part` is the rows dragged among, a subsequence of
    /// `all`, the folder's worktrees in order. The part is moved within the
    /// places it already holds, so the rest of the project keeps its order
    /// and the server is told the whole folder rather than a piece of it.
    static func reordered(_ all: [String], moving part: [String], from: IndexSet, to: Int) -> [String] {
        // A list's move, as `onMove` means it: `to` is a place in the part
        // before the moved rows were taken out of it.
        var moved = part.indices.filter { !from.contains($0) }.map { part[$0] }
        moved.insert(contentsOf: from.map { part[$0] }, at: to - from.count(in: 0..<to))
        let members = Set(part)
        var next = moved.makeIterator()
        return all.map { members.contains($0) ? (next.next() ?? $0) : $0 }
    }
}
