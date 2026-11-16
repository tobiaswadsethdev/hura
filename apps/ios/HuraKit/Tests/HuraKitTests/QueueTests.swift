import Foundation
import Testing
@testable import HuraKit

struct QueueTests {
    /// The reply as `hurad` writes it: the tag beside the fields, and a whole
    /// `NewSession` of which the phone reads a few.
    static let raw = """
    {"reply":"queue","limit":2,
     "sessions":[
      {"id":1,"new":{"project":"hura","name":"fix-login","title":"Fix login","repo":"https://x/r.git","task":"fix it",
        "base":null,"branch":null,"policy":"feature-work","providers":[],"toolchains":[],"start":true,
        "ticket":null,"pull":null,"interface":null,"worktree":false},"at":null,"added":5,"started":false,"failed":null},
      {"id":2,"new":{"project":"hura","name":"later","title":null,"repo":"https://x/r.git","task":"",
        "base":null,"branch":null,"policy":"feature-work","providers":[],"toolchains":[],"start":true},
       "at":1800000000,"added":6,"started":false,"failed":null},
      {"id":3,"new":{"project":null,"name":"next","title":null,"repo":"https://x/r.git","task":"",
        "policy":"feature-work","providers":[],"toolchains":[],"start":true},"at":null,"added":7,"started":false}],
     "messages":[{"id":4,"session":"fix-login","text":"run the tests","at":1800000000,"added":8},
                 {"id":5,"session":"other","text":"hi","at":null,"added":9}]}
    """

    @Test func readsTheQueue() throws {
        let q = try JSONDecoder().decode(QueueView.self, from: Data(Self.raw.utf8))
        #expect(q.limit == 2)
        #expect(q.sessions.map(\.label) == ["Fix login", "later", "next"])
        #expect(q.sessions[1].at == Date(timeIntervalSince1970: 1_800_000_000))
        #expect(q.sessions[2].failed == nil)
        #expect(q.messages(for: "fix-login").map(\.text) == ["run the tests"])
        #expect(q.messages[1].at == nil)
    }

    @Test func placesOnlyWhatWaitsForRoom() throws {
        let q = try JSONDecoder().decode(QueueView.self, from: Data(Self.raw.utf8))
        #expect(q.place(of: q.sessions[0]) == 1)
        #expect(q.place(of: q.sessions[1]) == nil)
        #expect(q.place(of: q.sessions[2]) == 2)
    }

    @Test func sendsAnInstantAsSeconds() throws {
        struct Body: Encodable {
            var name: String
            var text: String
            var at: Int64?
        }
        let data = try JSONEncoder().encode(Op(op: "queue-message", body: Body(name: "s", text: "t", at: 1_800_000_000)))
        let v = try JSONDecoder().decode(JSONValue.self, from: data)
        #expect(v["op"] == .string("queue-message"))
        #expect(v["at"] == .number(1_800_000_000))
    }
}
