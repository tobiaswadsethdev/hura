import Foundation
import Testing
@testable import HuraKit

/// hura-core's tracker code, linked in from crates/hura-mobile.
struct TrackerCoreTests {
    @Test func checksATrackerBeforeItIsKept() async throws {
        let named = try await TrackerCore.check(TrackerSource(kind: "git-hub", name: "  "))
        #expect(!named.name.trimmingCharacters(in: .whitespaces).isEmpty)
        await #expect(throws: TrackerError.self) {
            _ = try await TrackerCore.check(TrackerSource(kind: "jira", name: "work"))
        }
    }

    /// Filters go to hura-core and come back trimmed, and one it would
    /// refuse to run is refused before it is kept.
    @Test func checksATrackersFilters() async throws {
        let jira = TrackerSource(kind: "jira", name: "work", site: "https://x.atlassian.net", email: "a@b.c",
                                 hideCreated: true, jqlFilters: [JqlFilter(name: " ready ", jql: "status = Ready")])
        let checked = try await TrackerCore.check(jira)
        #expect(checked.hideCreated)
        #expect(checked.jqlFilters == [JqlFilter(name: "ready", jql: "status = Ready")])
        var github = TrackerSource(kind: "git-hub", name: "gh")
        github.jqlFilters = [JqlFilter(name: "ready", jql: "status = Ready")]
        await #expect(throws: TrackerError.self) { _ = try await TrackerCore.check(github) }
    }

    @Test func namesATicketsBranchUnderAServersPrefix() async throws {
        let b = try await TrackerCore.branch(prefix: "tobias", key: "PROJ-12", title: "Add the changelog")
        #expect(b.hasPrefix("tobias/PROJ-12"))
    }

    @Test func readsNothingFromNoTrackers() async throws {
        let inbox = try await TrackerCore.inbox([], prefix: "hura")
        #expect(inbox.tasks.isEmpty && inbox.pulls.isEmpty)
    }

    @Test func aTrackerWithNoTokenSaysSo() async throws {
        // No request is made without a token, so this needs no network.
        let inbox = try await TrackerCore.inbox([StoredTracker(source: TrackerSource(kind: "git-hub", name: "gh"), token: nil)],
                                                prefix: "hura")
        #expect(!inbox.warnings.isEmpty, "\(inbox)")
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

    @Test func aMergeWithNoTokenSaysSo() async throws {
        let tracker = StoredTracker(source: TrackerSource(kind: "git-hub", name: "gh"), token: nil)
        await #expect(throws: TrackerError.self) {
            try await TrackerCore.pullMerge(tracker, repo: "o/r", id: "1", how: "squash", head: nil, deleteBranch: false)
        }
        await #expect(throws: TrackerError.self) {
            try await TrackerCore.pullAutoMerge(tracker, repo: "o/r", id: "1", how: nil, deleteBranch: false)
        }
    }
}

/// Against GitHub itself, read-only:
/// `HURA_GITHUB_TOKEN="$(gh auth token)" swift test --filter LiveGitHub`.
private let githubToken = ProcessInfo.processInfo.environment["HURA_GITHUB_TOKEN"]

@Suite(.enabled(if: githubToken != nil, "set HURA_GITHUB_TOKEN to run"))
struct LiveGitHubTests {
    @Test func readsAnInboxAndAPullRequestOverTheNetwork() async throws {
        let tracker = StoredTracker(source: TrackerSource(kind: "git-hub", name: "github", repo: "tobiaswadsethdev/hura"),
                                    token: githubToken)
        let inbox = try await TrackerCore.inbox([tracker], prefix: "hura")
        #expect(inbox.read == ["github"], "\(inbox.warnings)")
        // A merged pull request of this repository's own.
        let pull = try await TrackerCore.pull(tracker, repo: "tobiaswadsethdev/hura", id: "66")
        #expect(pull.state == "merged")
        #expect(pull.title.contains("iOS"))
        #expect(pull.merging == nil, "a merged pull request has nothing left to merge")
        let files = try await TrackerCore.pullFiles(tracker, repo: "tobiaswadsethdev/hura", id: "66")
        #expect(files.files.contains { $0.path.hasSuffix("WorktreeView.swift") })
        let file = try #require(files.files.first { $0.path.hasSuffix("TerminalChannel.swift") })
        let diff = try await TrackerCore.pullFile(tracker, files: files, file: file)
        #expect(diff.modified.contains("TerminalChannel"))
    }
}
