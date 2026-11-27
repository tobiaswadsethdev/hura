import Foundation
import HuraKit
import Testing
@testable import Hura

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder().decode(type, from: Data(json.utf8))
}

private func options(worktrees: Bool? = true) throws -> NewOptions {
    try decode(NewOptions.self, """
    {"policies": [{"spec": "feature-work", "summary": ""}, {"spec": "locked-down", "summary": ""}],
     "toolchains": [{"name": "node", "summary": ""}, {"name": "rust", "summary": ""}],
     "providers": [{"name": "claude", "kind": "anthropic"}, {"name": "github", "kind": "github"}],
     "providers_error": null, "default_policy": "feature-work", "default_providers": ["claude"],
     "default_base": "main", "skills": [], "mcp": [], "default_interface": "chat",
     "worktrees": \(worktrees.map { "\($0)" } ?? "null")}
    """)
}

private func checkout(branch: String? = "feature", onRemote: Bool = true, toolchains: [String] = ["node"],
                      providers: [String] = []) throws -> Picked {
    try decode(Picked.self, """
    {"facts": {"uncommitted": 0, "unpushed": 0, "base_on_remote": \(onRemote), "toolchains": \(toolchains)},
     "branch": \(branch.map { "\"\($0)\"" } ?? "null"), "providers": \(providers)}
    """)
}

@Test func aDraftOpensWithTheServersDefaultsAndTheCheckoutsAnswers() throws {
    let d = DraftDefaults(project: ProjectDefaults(), options: try options(), checkout: try checkout())
    #expect(d.interface == "chat")
    #expect(d.worktree == false)
    #expect(d.policy == "feature-work")
    // The checkout's branch beats the config's default.
    #expect(d.base == "feature")
    #expect(d.toolchains == ["node"])
    #expect(d.providers == ["claude"])
}

@Test func aProjectsDefaultsComeBeforeTheServersAndTheCheckouts() throws {
    let own = ProjectDefaults(interface: "terminal", worktree: true, policy: "locked-down", base: "develop",
                              toolchains: ["rust"], providers: ["github"])
    let d = DraftDefaults(project: own, options: try options(),
                          checkout: try checkout(toolchains: ["node"], providers: ["claude"]))
    #expect(d == DraftDefaults(project: own, options: try options(), checkout: nil))
    #expect(d.interface == "terminal")
    #expect(d.worktree)
    #expect(d.policy == "locked-down")
    #expect(d.base == "develop")
    #expect(d.toolchains == ["rust"])
    #expect(d.providers == ["github"])
}

@Test func aProjectsWorktreeIsASandboxWhereTheServerAllowsNone() throws {
    let d = DraftDefaults(project: ProjectDefaults(worktree: true), options: try options(worktrees: nil), checkout: nil)
    #expect(!d.worktree)
}

@Test func theBaseIsBlankForABranchNeverPushedAndThePullRequestsForOne() throws {
    let o = try options()
    #expect(DraftDefaults(project: .init(), options: o, checkout: try checkout(onRemote: false)).base == "")
    // A detached HEAD, or no answer yet: the config's.
    #expect(DraftDefaults(project: .init(), options: o, checkout: try checkout(branch: nil)).base == "main")
    #expect(DraftDefaults(project: .init(), options: o, checkout: nil).base == "main")
    #expect(DraftDefaults(project: .init(base: "develop"), options: o, checkout: nil, pullBase: "release").base == "release")
}

@Test func theCheckoutsCredentialsReplaceTheServersWhenItNamesAny() throws {
    let d = DraftDefaults(project: .init(), options: try options(), checkout: try checkout(providers: ["github"]))
    #expect(d.providers == ["github"])
}

@Test func aRepeatsFirstTimeIsTheRulesNextOnTheServersClock() throws {
    // Friday 27 November 2026, 10:00 UTC.
    let now = Date(timeIntervalSince1970: 1_795_773_600)
    var utc = Calendar(identifier: .gregorian)
    utc.timeZone = TimeZone(secondsFromGMT: 0)!
    let parts = { (d: Date) in utc.dateComponents([.weekday, .hour, .minute, .day], from: d) }

    // Past nine today, so tomorrow.
    let daily = try #require(Repeat.fixedTime(hour: 9, minute: 0, weekdays: []).next(after: now, serverOffset: 0))
    #expect(parts(daily).day == 28)
    #expect(parts(daily).hour == 9)
    // On weekdays, over the weekend to Monday.
    let weekdays = try #require(Repeat.fixedTime(hour: 9, minute: 0, weekdays: [1, 2, 3, 4, 5]).next(after: now, serverOffset: 0))
    #expect(parts(weekdays).weekday == 2)
    #expect(parts(weekdays).day == 30)
    // Fridays' has gone today, so a week on.
    let friday = try #require(Repeat.fixedTime(hour: 9, minute: 0, weekdays: [5]).next(after: now, serverOffset: 0))
    #expect(friday == now.addingTimeInterval(7 * 86400 - 3600))
    // Later today, on a server two hours ahead: 13:00 there is 11:00 UTC.
    let later = try #require(Repeat.fixedTime(hour: 13, minute: 0, weekdays: []).next(after: now, serverOffset: 7200))
    #expect(later == now.addingTimeInterval(3600))
    #expect(Repeat.interval(minutes: 60).next(after: now, serverOffset: 0) == now.addingTimeInterval(3600))
}

@Test func theServersClockIsSaidOnlyWhenItDiffers() {
    #expect(clockDifference(serverOffset: 3600, localOffset: 3600) == nil)
    #expect(clockDifference(serverOffset: 0, localOffset: 3600) == "1 hour behind")
    #expect(clockDifference(serverOffset: 19800, localOffset: 0) == "5 hours 30 minutes ahead")
}

@Test func usageSumsTheWorktreesAndListsTheDearestFirst() throws {
    let cheap = try decode(Usage.self, """
    {"cost_usd": 0.5, "duration_ms": 600000, "lines_added": 10, "lines_removed": 2, "windows": []}
    """)
    let dear = try decode(Usage.self, """
    {"cost_usd": 4.25, "duration_ms": 3000000, "lines_added": 300, "lines_removed": 40,
     "windows": [{"label": "5h", "used_percentage": 34, "resets_at": 1}]}
    """)
    let sum = UsageSummary(sessions: [Session(name: "a", state: "idle"), Session(name: "b", state: "running"),
                                      Session(name: "c", state: "idle")],
                           usage: ["a": cheap, "b": dear])
    #expect(sum.rows.map(\.session.name) == ["b", "a"])
    #expect(sum.cost == 4.75)
    #expect(sum.added == 310)
    #expect(sum.removed == 42)
    #expect(UsageWords.duration(sum.durationMS) == "1h")
    #expect(sum.windows?.first?.label == "5h")
    #expect(UsageSummary(sessions: [Session(name: "a", state: "idle")], usage: ["a": cheap]).windows == nil)
}

@Test func usageSaysAWindowsResetRelativeWhileItIsSoon() {
    let now = Date(timeIntervalSince1970: 1_000_000)
    #expect(UsageWords.resetsIn(1_000_000 - 5, now: now) == "now")
    #expect(UsageWords.resetsIn(1_000_000 + 25 * 60, now: now) == "in 25m")
    #expect(UsageWords.resetsIn(1_000_000 + 3 * 3600, now: now) == "in 3h")
    #expect(UsageWords.dollars(1.5) == "$1.50")
}
