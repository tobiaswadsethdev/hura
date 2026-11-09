import Foundation
import Testing
@testable import HuraKit

/// What the phone works out for itself about trackers, and the shapes it
/// reads from and sends to hura-core through hura cloud.
struct TrackerCoreTests {
    /// The same names as hura-core's `tracker::branch`.
    @Test func namesATicketsBranchUnderAServersPrefix() {
        #expect(ticketBranch(prefix: "tobias", key: "PROJ-12", title: "Add the changelog") == "tobias/PROJ-12-add-the-changelog")
        #expect(ticketBranch(prefix: "", key: "PROJ-1", title: "Fix it") == "PROJ-1-fix-it")
        #expect(ticketBranch(prefix: "/tobias/", key: "PROJ-1", title: "Fix it") == "tobias/PROJ-1-fix-it")
        #expect(ticketBranch(prefix: "t", key: "PROJ-9", title: "  ") == "t/PROJ-9")
        #expect(ticketBranch(prefix: "", key: " #61 ", title: "Ünïcode & more!") == "61-n-code-more")
        let long = "Make the nightly reconciliation job cope with a partial batch from upstream"
        #expect(ticketBranch(prefix: "", key: "PROJ-123", title: long) == "PROJ-123-make-the-nightly-reconciliation-job")
    }

    @Test func signedOutThereAreNoTrackersToAsk() async throws {
        let cloud = Cloud(store: InMemoryCloudSessionStore())
        await #expect(throws: TrackerError.self) {
            try await TrackerCore.$cloud.withValue(cloud) { _ = try await TrackerCore.inbox(prefix: "hura") }
        }
    }

    @Test func comparesRepositoriesWhateverTheirSpelling() {
        #expect(bareRepo("git@github.com:Tobias/Hura.git") == bareRepo("https://github.com/tobias/hura"))
        #expect(bareRepo("https://org@dev.azure.com/org/p/_git/r") == "dev.azure.com/org/p/_git/r")
    }

    @Test func sendsAnEditableAsHuraCoreReadsIt() throws {
        let v = try TrackerCore.json(FieldChange(id: "assignee", edit: .user(UserChoice(accountID: "a1", name: "Ann"), assignable: true)))
        #expect(v["edit"]?["kind"]?.string == "user")
        #expect(v["edit"]?["value"]?["account_id"]?.string == "a1")
        let back = try TrackerCore.decode(.object(["kind": .string("read-only"), "value": .string("x")]), as: Editable.self)
        #expect(back == .readOnly("x"))
    }

    @Test func readsHowAPullRequestMergesAsHuraCoreWritesIt() throws {
        let v: JSONValue = .object([
            "methods": .array([.string("squash"), .string("rebase-merge")]),
            "auto": .object(["by": .object(["name": .string("Ann"), "me": .bool(false)]),
                             "method": .string("squash"), "delete_branch": .bool(true)]),
            "can_auto": .bool(true), "deletes_branch": .null, "head": .string("abc1234"),
        ])
        let m = try TrackerCore.decode(v, as: PullMerging.self)
        #expect(m.methods == ["squash", "rebase-merge"])
        #expect(m.auto?.deleteBranch == true && m.auto?.by?.name == "Ann")
        #expect(m.canAuto && m.deletesBranch == nil && m.head == "abc1234")
    }
}
