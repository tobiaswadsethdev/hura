import Foundation
import Testing
@testable import Hura

@Test func aGroupIsMadeWithItsWorktrees() {
    var book = GroupBook()
    let id = book.create(in: "hura", named: "  Groups ", with: ["a", "b"], id: "g1")
    #expect(id == "g1")
    #expect(book.groups(in: "hura") == [WorktreeGroup(id: "g1", name: "Groups")])
    #expect(book.group(of: "a", in: "hura") == "g1")
    #expect(book.group(of: "b", in: "hura") == "g1")
    #expect(book.group(of: "c", in: "hura") == nil)
    // Unnamed, it is called what the desktop calls it.
    book.create(in: "hura", named: " ", id: "g2")
    #expect(book.groups(in: "hura").map(\.name) == ["Groups", "New group"])
}

@Test func aWorktreeIsOnlyInItsOwnProjectsGroups() {
    var book = GroupBook()
    book.create(in: "hura", named: "Sidebar", with: ["a"], id: "g1")
    #expect(book.group(of: "a", in: "storefront") == nil)
    #expect(book.group(of: "a", in: nil) == nil)
}

@Test func joiningMovesItAndPickingItAgainTakesItOut() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["a"], id: "g1")
    book.create(in: "hura", named: "Two", id: "g2")
    book.toggle("a", in: "g2", project: "hura")
    #expect(book.group(of: "a", in: "hura") == "g2")
    book.toggle("a", in: "g2", project: "hura")
    #expect(book.group(of: "a", in: "hura") == nil)
    book.assign("a", to: "g1")
    book.assign("a", to: nil)
    #expect(book.group(of: "a", in: "hura") == nil)
}

@Test func renamingToNothingKeepsTheName() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", id: "g1")
    book.rename("g1", in: "hura", to: " Phone ")
    #expect(book.groups(in: "hura").first?.name == "Phone")
    book.rename("g1", in: "hura", to: "  ")
    #expect(book.groups(in: "hura").first?.name == "Phone")
}

@Test func removingAGroupKeepsItsWorktrees() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["a", "b"], id: "g1")
    book.create(in: "hura", named: "Two", with: ["c"], id: "g2")
    book.remove("g1", in: "hura")
    #expect(book.groups(in: "hura").map(\.id) == ["g2"])
    #expect(book.group(of: "a", in: "hura") == nil)
    #expect(book.of["a"] == nil && book.of["b"] == nil)
    #expect(book.group(of: "c", in: "hura") == "g2")
    book.remove("g2", in: "hura")
    #expect(book == GroupBook())
}

@Test func aDeletedWorktreeIsForgotten() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["a"], id: "g1")
    book.forget("a")
    #expect(book.of.isEmpty)
    #expect(book.groups(in: "hura").count == 1)
}

@Test func foldingIsKeptWithTheGroup() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", id: "g1")
    book.fold("g1", in: "hura")
    #expect(book.groups(in: "hura").first?.folded == true)
    book.fold("g1", in: "hura")
    #expect(book.groups(in: "hura").first?.folded == false)
}

@Test func aFolderDrawsItsGroupsFirstThenTheRest() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["c", "a"], id: "g1")
    book.create(in: "hura", named: "Empty", id: "g2")
    // One kept for a group since removed counts as in none.
    book.assign("d", to: "gone")
    let arranged = book.arrange(["a", "b", "c", "d"], in: "hura", name: { $0 })
    #expect(arranged.groups.map(\.group.name) == ["One", "Empty"])
    // In the folder's order, not the order they joined in.
    #expect(arranged.groups[0].rows == ["a", "c"])
    #expect(arranged.groups[1].rows.isEmpty)
    #expect(arranged.rest == ["b", "d"])
}

@Test func aSessionInNoProjectIsInNoGroup() {
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["a"], id: "g1")
    let arranged = book.arrange(["a"], in: nil, name: { $0 })
    #expect(arranged.groups.isEmpty)
    #expect(arranged.rest == ["a"])
}

@Test func reorderingAGroupKeepsTheRestOfTheFolderInPlace() {
    let all = ["a", "x", "b", "y", "c"]
    // The group is a, b and c; c dragged to the top of it.
    #expect(GroupBook.reordered(all, moving: ["a", "b", "c"], from: [2], to: 0) == ["c", "x", "a", "y", "b"])
    // a dragged to the end of it, past c: `to` counts it still in place.
    #expect(GroupBook.reordered(all, moving: ["a", "b", "c"], from: [0], to: 3) == ["b", "x", "c", "y", "a"])
    // Among the ungrouped.
    #expect(GroupBook.reordered(all, moving: ["x", "y"], from: [1], to: 0) == ["a", "y", "b", "x", "c"])
    // A whole list, as the unfiled are.
    #expect(GroupBook.reordered(["a", "b"], moving: ["a", "b"], from: [0], to: 2) == ["b", "a"])
}

@Test func groupsAreKeptPerServer() throws {
    let defaults = try #require(UserDefaults(suiteName: "hura.tests.\(UUID().uuidString)"))
    var book = GroupBook()
    book.create(in: "hura", named: "One", with: ["a"], id: "g1")
    book.fold("g1", in: "hura")
    book.save(server: "h_one", to: defaults)
    #expect(GroupBook.load(server: "h_one", from: defaults) == book)
    #expect(GroupBook.load(server: "h_two", from: defaults) == GroupBook())
}
