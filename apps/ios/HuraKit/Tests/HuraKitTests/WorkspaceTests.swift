import Foundation
import Testing
@testable import HuraKit

/// Requests as hura-proto's `Request` reads them: tagged by `op`, the fields
/// beside the tag.
struct WorkspaceTests {
    private func json<T: Encodable>(_ value: T) throws -> [String: JSONValue] {
        let data = try JSONEncoder().encode(value)
        guard case let .object(o) = try JSONDecoder().decode(JSONValue.self, from: data) else {
            Issue.record("not an object")
            return [:]
        }
        return o
    }

    @Test func flattensTheFieldsBesideTheOp() throws {
        #expect(try json(Op(op: "destroy", body: Named(name: "fix-tests"))) == [
            "op": .string("destroy"), "name": .string("fix-tests"),
        ])
        #expect(try json(Op(op: "projects", body: Nothing())) == ["op": .string("projects")])
    }

    @Test func sendsANewSessionWithEveryField() throws {
        let s = NewSession(project: "hura", name: nil, repo: "git@github.com:me/hura.git", task: "fix the tests",
                           base: "main", policy: "standard", providers: ["github"], toolchains: ["rust"],
                           interface: "chat")
        #expect(try json(Op(op: "create", body: s)) == [
            "op": .string("create"),
            "project": .string("hura"),
            "name": .null,
            "repo": .string("git@github.com:me/hura.git"),
            "task": .string("fix the tests"),
            "base": .string("main"),
            "branch": .null,
            "policy": .string("standard"),
            "providers": .array([.string("github")]),
            "toolchains": .array([.string("rust")]),
            "start": .bool(true),
            "interface": .string("chat"),
            "ticket": .null,
            "pull": .null,
        ])
    }

    @Test func sendsADecisionWhole() throws {
        let d = ChatDecision(allow: true, answers: ["Which order?": "Workspace first"])
        #expect(try json(d) == [
            "allow": .bool(true),
            "always": .bool(false),
            "message": .null,
            "answers": .object(["Which order?": .string("Workspace first")]),
            "mode": .null,
        ])
    }

    @Test func readsTheCreateFormsOptions() throws {
        let raw = """
        {"policies":[{"spec":"standard","summary":"the usual"}],
         "toolchains":[{"name":"rust","summary":"cargo"}],
         "providers":[{"name":"github","kind":"oauth"}],"providers_error":null,
         "default_policy":"standard","default_providers":["github"],"default_base":null,
         "skills":["review"],"mcp":[],"default_interface":"chat"}
        """
        let o = try JSONDecoder().decode(NewOptions.self, from: Data(raw.utf8))
        #expect(o.policies.first?.spec == "standard")
        #expect(o.defaultInterface == "chat")
        #expect(o.defaultBase == nil)
    }

    @Test func readsASessionsProjectAndAge() throws {
        let raw = """
        {"name":"fix-tests","sandbox":"hura-fix-tests","tmux":"agent","repo":"git@x:y.git","project":"hura",
         "base_branch":null,"work_branch":"tobias/fix-tests","task":"","ticket":null,"pull":null,"policy":null,
         "providers":[],"skills":[],"mcp":[],"toolchains":["rust"],"agent":"claude","interface":"chat",
         "created_at":1761840000,"state":"failed","failure":"clone refused"}
        """
        let s = try JSONDecoder().decode(Session.self, from: Data(raw.utf8))
        #expect(s.project == "hura")
        #expect(s.createdAt == 1_761_840_000)
        #expect(s.failure == "clone refused")
        #expect(s.toolchains == ["rust"])
    }

    @Test @MainActor func groupsWorktreesUnderTheirProjects() async {
        let server = Server(record: ServerRecord(serverID: "h_aaaabbbbccccdddd", name: "studio", host: "box", port: "1",
                                                 fingerprint: String(repeating: "ab", count: 32), token: "t"))
        server.load(projects: [Project(name: "hura", repo: "r", path: "/p")],
                    sessions: [Session(name: "old", state: "idle", project: "hura", createdAt: 1),
                               Session(name: "new", state: "idle", project: "hura", createdAt: 2),
                               Session(name: "stray", state: "idle", project: "gone"),
                               Session(name: "cli", state: "idle")])
        #expect(server.worktrees(in: server.projects[0]).map(\.name) == ["new", "old"])
        #expect(Set(server.unfiled.map(\.name)) == ["stray", "cli"])
    }
}
