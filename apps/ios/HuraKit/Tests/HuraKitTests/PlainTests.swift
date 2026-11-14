import Foundation
import Network
import Testing
@testable import HuraKit

struct PlainTests {
    @Test func aRefusedConnectionSaysWhatToCheck() {
        let said = Plain.network(.posix(.ECONNREFUSED))
        #expect(said.contains("hurad running"))
        #expect(!said.contains("POSIX"))
    }

    @Test func theServersClausesStartLikeSentences() {
        #expect(ServerError.failed(kind: "failed", message: "no session named `x`").errorDescription == "No session named `x`")
        #expect(Plain.sentence("hura cloud is busy") == "hura cloud is busy")
        #expect(Plain.sentence("`sbx` is missing") == "`sbx` is missing")
    }

    @Test func aDecoderNeverSpeaksForItself() throws {
        do {
            _ = try JSONDecoder().decode(Int.self, from: Data("\"x\"".utf8))
            Issue.record("decoded")
        } catch {
            #expect(error.userMessage == "The answer couldn't be read. hura may need updating.")
        }
    }

    @Test func aServerErrorStatusIsNotJustANumber() {
        #expect(ServerError.http(502).errorDescription == "The server ran into a problem answering (HTTP 502).")
    }
}
