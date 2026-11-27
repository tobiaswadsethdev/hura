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

struct RepeatTests {
    @Test func readsAndWritesTheServersShape() throws {
        let raw = """
        {"reply":"queue","limit":2,"utc_offset":3600,"sessions":[
          {"id":1,"new":{"project":"hura","name":"nightly","title":null,"repo":"r","task":"t"},
           "at":1800000000,"added":5,"started":false,"repeat":{"type":"fixed_time","hour":9,"minute":30,"weekdays":[1,2,3,4,5]},"runs":3}],
         "messages":[{"id":2,"session":"s","text":"status?","at":1800000000,"added":6,"repeat":{"type":"interval","minutes":60},"runs":0}]}
        """
        let q = try JSONDecoder().decode(QueueView.self, from: Data(raw.utf8))
        #expect(q.canRepeat)
        #expect(q.sessions[0].repeat == .fixedTime(hour: 9, minute: 30, weekdays: [1, 2, 3, 4, 5]))
        #expect(q.sessions[0].runs == 3)
        #expect(q.messages[0].repeat == .interval(minutes: 60))
        let back = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(Repeat.fixedTime(hour: 9, minute: 0, weekdays: [])))
        #expect(back["type"] == .string("fixed_time"))
        #expect(back["hour"] == .number(9))
    }

    @Test func anOlderServerCannotRepeat() throws {
        let q = try JSONDecoder().decode(QueueView.self, from: Data(QueueTests.raw.utf8))
        #expect(!q.canRepeat)
        #expect(q.sessions[0].repeat == nil)
    }

    /// A time picked on a phone an hour behind the server is an hour later on
    /// the server's clock, and said back as it was picked; across midnight the
    /// day moves with it.
    @Test func movesATimeOntoTheServersClock() {
        let rule = Cadence.weekly(1).rule(hour: 23, minute: 30, serverOffset: 3600, localOffset: 0)
        #expect(rule == .fixedTime(hour: 0, minute: 30, weekdays: [2]))
        #expect(rule.describe(serverOffset: 3600, localOffset: 0) == "Every Monday at 23:30")
        #expect(Cadence.weekdays.rule(hour: 9, minute: 0, serverOffset: 0, localOffset: 0)
            .describe(serverOffset: 0, localOffset: 0) == "Every weekday at 09:00")
        #expect(Cadence.hourly.rule(hour: 9, minute: 0, serverOffset: 0) == .interval(minutes: 60))
    }
}
