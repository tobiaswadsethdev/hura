import Foundation
import Testing
@testable import HuraKit

/// The cases from hura-core's `events.rs`, so the phone reads the feed exactly
/// as the desktop does.
struct EventTargetTests {
    @Test(arguments: [
        ("POST github.com:443/o/r.git/git-receive-pack", "github.com:443"),
        ("example.com:443", "example.com:443"),
        ("/usr/bin/curl(79) -> pastebin.com:443", "pastebin.com:443"),
        ("/usr/bin/node(7) -> GET docs.rs:443/tokio", "docs.rs:443"),
    ])
    func aDecisionYieldsTheEndpointItWasAbout(subject: String, endpoint: String) {
        #expect(PolicyEvent(at: 1, subject: subject).endpoint == endpoint)
    }

    @Test func aRequestTakesItsPortFromTheReason() {
        var e = PolicyEvent(at: 1, subject: "GET github.com/", reason: "endpoint github.com:80 is not allowed by any policy")
        #expect(e.endpoint == "github.com:80")
        e.reason = "endpoint pastebin.com:80 is not allowed by any policy"
        #expect(e.endpoint == nil)
    }

    @Test(arguments: [
        "",
        "sleep(56)",
        "applied policy revision",
        "L7 policy validation warning: 'tls: terminate' is deprecated",
        "localhost:443",
        "pastebin.com:https",
        "pastebin.com:0",
        "pastebin.com:99999",
        "get httpbin.org:443/ip",
        "denied reaching pastebin.com:443",
    ])
    func proseIsNeverMistakenForAnEndpoint(subject: String) {
        #expect(PolicyEvent(at: 1, subject: subject).endpoint == nil)
    }

    @Test func readsTheRequestADenialWasAbout() {
        #expect(PolicyEvent(at: 1, subject: "POST github.com:443/o/r.git/git-receive-pack?x=1").request
            == PathRoute(method: "POST", path: "/o/r.git/git-receive-pack"))
        #expect(PolicyEvent(at: 1, subject: "example.com:443").request == nil)
    }

    @Test func readsAnEventKeptBeforeCounts() throws {
        let old = #"{"at":1787568645,"class":"NET:OPEN","severity":"Info","verdict":"Denied","subject":"/usr/bin/curl(79) -> pastebin.com:443","policy":null,"reason":null}"#
        let e = try JSONDecoder().decode(PolicyEvent.self, from: Data(old.utf8))
        #expect(e.count == 1)
        #expect(e.endpoint == "pastebin.com:443")
    }
}

struct UnifiedDiffTests {
    /// An edit is highlighted where it changed; a line with nothing in common
    /// with its partner, or with none, is not.
    @Test func anEditedLineKnowsWhereItChanged() {
        let lines = UnifiedDiff.lines(original: "let url = 'sqlite'\nkeep\nold\n",
                                      modified: "let url = process.env.DB\nkeep\nnew thing\nextra\n")
        let spans = UnifiedDiff.changedSpans(lines)
        let removed = lines.first { $0.kind == .removed && $0.old == 1 }!
        let added = lines.first { $0.kind == .added && $0.new == 1 }!
        #expect(spans[removed.id] == 10..<18)
        #expect(spans[added.id] == 10..<24)
        // "old" and "new thing" share nothing; "extra" has no partner.
        #expect(spans.count == 2)
        #expect(UnifiedDiff.span(Array("abc"), Array("abc")).map { [$0.0, $0.1] } == [3..<3, 3..<3])
    }

    @Test func linesUpBothSides() {
        let lines = UnifiedDiff.lines(original: "a\nb\nc\n", modified: "a\nB\nc\nd\n")
        #expect(lines.map(\.kind) == [.same, .removed, .added, .same, .added])
        #expect(lines.map(\.old) == [1, 2, nil, 3, nil])
        #expect(lines.map(\.new) == [1, nil, 2, 3, 4])
    }

    @Test func foldsWhatDidNotChange() {
        let original = (1...20).map(String.init).joined(separator: "\n")
        let modified = original.replacingOccurrences(of: "\n10\n", with: "\nten\n")
        let pieces = UnifiedDiff.pieces(UnifiedDiff.lines(original: original, modified: modified), context: 2)
        guard pieces.count == 3, case let .folded(before, _) = pieces[0], case let .lines(shown) = pieces[1],
              case let .folded(after, _) = pieces[2] else {
            Issue.record("\(pieces)")
            return
        }
        #expect(before == 7)
        #expect(shown.count == 6)
        #expect(after == 8)
    }

    @Test func aNewFileIsAllAdded() {
        #expect(UnifiedDiff.lines(original: "", modified: "x\ny").allSatisfy { $0.kind == .added })
    }
}

struct DockWireTests {
    @Test func sendsGitOpsTaggedByDo() throws {
        let data = try JSONEncoder().encode(Op(op: "git", body: ["action": GitOp.stage("src/a.rs")]))
        let v = try JSONDecoder().decode(JSONValue.self, from: data)
        #expect(v["action"] == .object(["do": .string("stage"), "path": .string("src/a.rs")]))
        let push = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(GitOp.push))
        #expect(push == .object(["do": .string("push")]))
    }

    @Test func readsAPolicy() throws {
        let raw = """
        {"template":"standard","rules":[{"id":"r1","allow":true,"hosts":["github.com:443"],"methods":[],"path":null,"global":false}],
         "lists":{"allow":[{"endpoint":"crates.io:443","in_policy":true}],"routes":[],"block":[]}}
        """
        let p = try JSONDecoder().decode(PolicyView.self, from: Data(raw.utf8))
        #expect(p.rules.first?.hosts == ["github.com:443"])
        #expect(p.lists?.allow.first?.inPolicy == true)
        #expect(p.watching == nil, "a server older than watching says nothing of it")

        let watched = try JSONDecoder().decode(PolicyView.self, from: Data("""
        {"template":null,"rules":[],"watching":[{"id":"w1","host":"example.com:443"}],"lists":null}
        """.utf8))
        #expect(watched.watching == [Watching(id: "w1", host: "example.com:443")])
    }
}
