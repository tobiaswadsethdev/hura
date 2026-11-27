import Foundation
import Testing
@testable import HuraKit

struct AgentsActivityTests {
    /// The content state as the relay sends it (`agentsPayload` in
    /// `services/relay/src/notify.ts`).
    static let raw = """
    {"active":2,"waiting":1,"failed":0,"rows":[
      {"server_id":"h_a","session":"docs","phase":"permission","at":1763300000,"ask_id":"toolu_01","enc":"c2VhbGVk"},
      {"server_id":"h_b","session":"fix-tests","phase":"working","at":1763299900,"since":1763299000,
       "title":"Fix the tests","subtitle":"studio","body":"Claude is working"}]}
    """

    @Test func readsTheRelaysState() throws {
        let s = try JSONDecoder().decode(AgentsActivityState.self, from: Data(Self.raw.utf8))
        #expect(s.rows.map(\.id) == ["h_a:docs", "h_b:fix-tests"])
        #expect(s.rows[0].state.askID == "toolu_01")
        #expect(s.rows[0].state.enc == "c2VhbGVk")
        #expect(s.rows[1].state.turnStart == Date(timeIntervalSince1970: 1_763_299_000))
        #expect(s.hero?.session == "docs")
        #expect(s.headline == "2 active agents")
        #expect(s.attention == "1 needs attention")
        #expect(s.compactWord == "Approval")
        // And back, in the same shape, as ActivityKit keeps it.
        let back = try JSONDecoder().decode(AgentsActivityState.self, from: JSONEncoder().encode(s))
        #expect(back == s)
    }

    @Test func ordersWaitingThenFailedThenWorkingThenDone() {
        func row(_ s: String, _ p: SessionPhase, _ at: TimeInterval) -> AgentRow {
            AgentRow(serverID: "h", session: s, state: SessionActivityState(phase: p, at: Date(timeIntervalSince1970: at)))
        }
        let s = AgentsActivityState(rows: [row("done", .done, 9), row("old", .working, 1), row("new", .working, 5),
                                           row("broke", .failed, 2), row("ask", .input, 0)])
        #expect(s.rows.map(\.session) == ["ask", "broke", "new", "old", "done"])
        #expect(s.active == 3)
        #expect(s.waiting == 1)
        #expect(s.short == "3 active")
    }

    @Test func saysHowItEnded() {
        let done = AgentsActivityState(rows: [AgentRow(serverID: "h", session: "a", state: SessionActivityState(phase: .done))])
        #expect(done.allDone)
        #expect(done.headline == "Agent work completed")
        #expect(done.compactWord == "Done")
        let failed = AgentsActivityState(rows: [AgentRow(serverID: "h", session: "a", state: SessionActivityState(phase: .failed))])
        #expect(failed.headline == "Agent work failed")
        #expect(failed.short == "Failed")
        let working = AgentsActivityState(rows: [AgentRow(serverID: "h", session: "a", state: SessionActivityState(phase: .working))])
        #expect(working.compactWord == nil)
        #expect(working.attention == nil)
    }
}
