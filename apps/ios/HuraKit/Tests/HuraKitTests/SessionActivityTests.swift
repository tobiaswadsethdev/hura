import CryptoKit
import Foundation
import Testing
@testable import HuraKit

/// A session's Live Activity, as the relay's ActivityKit pushes describe it
/// (`docs/relay-api.md`, "Live Activities").
struct SessionActivityTests {
    let serverID = "h_aaaabbbbccccdddd"

    /// Sealed as hurad seals it: the notification scheme, under the
    /// `session_activity` kind.
    func seal(_ json: String, key: Data, session: String, askID: String?) throws -> String {
        let aad = Data("hura-push-v1\n\(serverID)\nsession_activity\n\(session)\n\(askID ?? "")".utf8)
        let box = try AES.GCM.seal(Data(json.utf8), using: SymmetricKey(data: key), authenticating: aad)
        return box.combined!.base64URL
    }

    @Test func readsTheContentStateTheRelaySends() throws {
        let json = #"{"phase":"permission","at":1763300000,"ask_id":"toolu_01abc","enc":"abc"}"#
        let state = try JSONDecoder().decode(SessionActivityState.self, from: Data(json.utf8))
        #expect(state.phase == .permission)
        #expect(state.askID == "toolu_01abc")
        #expect(state.since == Date(timeIntervalSince1970: 1_763_300_000))

        let plain = #"{"phase":"done","at":1763300000,"title":"Fix the tests","body":"Claude finished"}"#
        let done = try JSONDecoder().decode(SessionActivityState.self, from: Data(plain.utf8))
        let text = done.text(serverID: serverID, session: "fix-tests", keys: InMemoryPushKeyStore())
        #expect(text?.title == "Fix the tests")
        #expect(text?.body == "Claude finished")
        #expect(done.phase.isOver)
    }

    @Test func aPhaseItDoesNotKnowReadsAsWorking() throws {
        let json = #"{"phase":"compacting","at":1}"#
        let state = try JSONDecoder().decode(SessionActivityState.self, from: Data(json.utf8))
        #expect(state.phase == .working)
    }

    @Test func opensSealedTextWithTheServersKey() throws {
        let keys = InMemoryPushKeyStore()
        let key = PushCrypto.newKey()
        try keys.save(key, serverID: serverID)
        let enc = try seal(#"{"title":"Fix the tests","subtitle":"scope-creep","body":"Bash: npm test"}"#,
                           key: key, session: "fix-tests", askID: "toolu_1")
        let state = SessionActivityState(phase: .permission, askID: "toolu_1", enc: enc)

        let text = state.text(serverID: serverID, session: "fix-tests", keys: keys)
        #expect(text?.body == "Bash: npm test")
        #expect(text?.subtitle == "scope-creep")

        // Bound to its session and its request: moved onto another, it reads
        // as nothing, and the activity offers no Allow.
        #expect(state.text(serverID: serverID, session: "other", keys: keys) == nil)
        var moved = state
        moved.askID = "toolu_2"
        #expect(moved.text(serverID: serverID, session: "fix-tests", keys: keys) == nil)
        // And without the key, nothing either.
        #expect(state.text(serverID: serverID, session: "fix-tests", keys: InMemoryPushKeyStore()) == nil)
    }

    @Test func anAnswerShowsTheSessionWorkingInTheClear() {
        let text = PushCrypto.Text(title: "Fix the tests", subtitle: "scope-creep", body: "Bash: npm test")
        let next = SessionActivityState(phase: .permission, askID: "toolu_1", enc: "x")
            .answered(text, body: "Allowed", now: Date(timeIntervalSince1970: 10))
        #expect(next.phase == .working)
        #expect(next.askID == nil)
        #expect(next.enc == nil)
        #expect(next.at == 10)
        #expect(next.text(serverID: serverID, session: "fix-tests", keys: InMemoryPushKeyStore())?.title == "Fix the tests")
    }

    @Test func aTappedActivityNamesItsSession() {
        let url = SessionActivity.url(serverID: serverID, session: "fix-tests")
        #expect(url.absoluteString == "hura://session?server=h_aaaabbbbccccdddd&name=fix-tests")
        let target = SessionActivity.target(of: url)
        #expect(target?.serverID == serverID)
        #expect(target?.session == "fix-tests")
        // A pairing string is not one.
        #expect(SessionActivity.target(of: URL(string: "hura://127.0.0.1:8443/tok#fp")!) == nil)
    }

    @Test func tokensGoToTheRelayAsHex() {
        #expect(SessionActivity.hex(Data([0x00, 0xab, 0xff])) == "00abff")
    }
}
