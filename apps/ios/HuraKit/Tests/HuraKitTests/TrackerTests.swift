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

    @Test func findsTheSessionsPullRequest() throws {
        func pull(_ id: String, branch: String) throws -> PullRequestSummary {
            try JSONDecoder().decode(PullRequestSummary.self, from: Data("""
            {"tracker":"gh","kind":"git-hub","id":"\(id)","key":"#\(id)","title":"t","url":"u","repo":"tobias/hura",
             "repo_url":"https://github.com/tobias/hura","relation":"created","review_requested":false,"author":"me",
             "branch":"\(branch)","base":"main","draft":false,"fork":false,"conflicts":false}
            """.utf8))
        }
        let pulls = [try pull("1", branch: "fix"), try pull("2", branch: "other")]
        var s = Session(name: "s", state: "ready", workBranch: "fix", repo: "git@github.com:Tobias/Hura.git")
        #expect(sessionPull(s, in: pulls)?.id == "1")
        // Started from one wins over the branch.
        s.pull = PullOrigin(tracker: "gh", kind: "git-hub", id: "2", key: "#2", url: "u", repo: "tobias/hura")
        #expect(sessionPull(s, in: pulls)?.id == "2")
        // The same branch in another repository is someone else's.
        let elsewhere = Session(name: "e", state: "ready", workBranch: "fix", repo: "https://github.com/other/hura")
        #expect(sessionPull(elsewhere, in: pulls) == nil)
    }

    /// A pull request as hura-core writes one, yours unless `mine` says not,
    /// with `conversation` as its threads.
    private func pullDetail(_ conversation: String, mine: Bool = true, author: String = "Tobias") throws -> PullDetail {
        try JSONDecoder().decode(PullDetail.self, from: Data("""
        {"tracker":"gh","kind":"git-hub","id":"58","key":"#58","repo":"o/r","title":"","url":"","state":"open",
         "draft":false,"conflicts":false,"fork":false,"author":{"name":"\(author)","me":\(mine)},"branch":"b","base":"main",
         "created":null,"updated":null,"body":"","additions":null,"deletions":null,"changed_files":null,"checks":[],
         "reviewers":[],"conversation":\(conversation),"older":0,"offers":[],"my_vote":null,"mine":\(mine),
         "merging":null,"warnings":[]}
        """.utf8))
    }

    private func thread(path: String? = nil, line: Int? = nil, resolved: Bool = false, verdict: String? = nil,
                        _ comments: [(id: String, name: String, me: Bool, body: String)]) -> String {
        let said = comments.map { c in
            #"{"id":"\#(c.id)","author":{"name":"\#(c.name)","me":\#(c.me)},"body":"\#(c.body)","created":null}"#
        }
        let path = path.map { "\"\($0)\"" } ?? "null"
        let verdict = verdict.map { "\"\($0)\"" } ?? "null"
        return #"{"id":"","path":\#(path),"line":\#(line.map(String.init) ?? "null"),"resolved":\#(resolved),"outdated":false,"verdict":\#(verdict),"author":null,"at":null,"comments":[\#(said.joined(separator: ","))],"can_reply":true}"#
    }

    /// The same choices as hura-core's `pull_remarks`, with the same ids, or
    /// the server's record of what was sent would differ between the phone
    /// and the desktop and an agent would hear a remark twice.
    @Test func sendsOnlyWhatReviewersSaidThatIsStillOpen() throws {
        let d = try pullDetail("[" + [
            thread(path: "a.rs", line: 1, resolved: true, [("t1", "Ada", false, "fixed already")]),
            thread([("c1", "Tobias", true, "note to self")]),
            thread(verdict: "approved", []),
            thread(path: "/src/b.rs", line: 7, [("t2", "Ada", false, "Why three?"), ("t3", "Tobias", true, "Measured.")]),
            thread(verdict: "changes-requested", [("r1", "Ada", false, "One thing first.")]),
        ].joined(separator: ",") + "]")

        let remarks = pullRemarks(d)

        #expect(remarks.count == 2)
        #expect(remarks[0].file == "src/b.rs")
        #expect(remarks[0].line == 7)
        #expect(remarks[0].said.map(\.id) == ["gh/o/r/58/t2", "gh/o/r/58/t3"])
        #expect(remarks[1].file == "")
        #expect(remarks[1].said.first?.who == "Ada, asking for changes")
    }

    /// On somebody else's pull request the author is them, by name.
    @Test func onSomebodyElsesPullRequestTheirWordsAreTheAuthors() throws {
        let d = try pullDetail("[" + [
            thread([("c1", "Sam", false, "ready")]),
            thread([("c2", "Tobias", true, "not yet")]),
        ].joined(separator: ",") + "]", mine: false, author: "Sam")

        #expect(pullRemarks(d).map { $0.said[0].body } == ["not yet"])
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
