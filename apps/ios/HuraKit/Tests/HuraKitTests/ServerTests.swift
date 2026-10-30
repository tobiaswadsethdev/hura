import CryptoKit
import Foundation
import Testing
@testable import HuraKit

struct PairingTests {
    static let fp = String(repeating: "ab", count: 32)

    @Test func parsesAPairingString() throws {
        let p = try PairingString("  hura://box.lan:17671/8f3ce21a#\(Self.fp.uppercased())\n")
        #expect(p == PairingString(host: "box.lan", port: "17671", token: "8f3ce21a", fingerprint: Self.fp))
    }

    @Test func parsesAnIPv6Address() throws {
        let p = try PairingString("hura://[fd7a:115c::1]:17671/tok#\(Self.fp)")
        #expect(p.host == "fd7a:115c::1")
        #expect(p.port == "17671")
    }

    @Test(arguments: [
        "hedera://pair?v=1",
        "hura://box.lan:17671/tok",
        "hura://box.lan:17671/#\(String(repeating: "ab", count: 32))",
        "hura://box.lan:17671/tok#short",
        "hura://box.lan:17671/a/b#\(String(repeating: "ab", count: 32))",
    ])
    func refusesWhatIsNotOne(raw: String) {
        #expect(throws: PairingString.ParseError.self) { try PairingString(raw) }
    }

    @Test func serverIDsAreWhatTheRelayTakes() {
        for _ in 0..<20 {
            let id = ServerRecord.newServerID()
            #expect(id.wholeMatch(of: /h_[a-z2-7]{16}/) != nil, "\(id)")
        }
    }

    @Test func aRecordKeepsItsTokenOutOfDescriptions() {
        let r = ServerRecord(serverID: "h_aaaabbbbccccdddd", name: "studio", host: "box", port: "1",
                             fingerprint: Self.fp, token: "secret-token")
        #expect(!"\(r)".contains("secret-token"))
        #expect(!String(reflecting: r).contains("secret-token"))
        var dumped = ""
        dump(r, to: &dumped)
        #expect(!dumped.contains("secret-token"))
    }
}

struct WireTests {
    @Test func pinsTheWholeCertificatesHash() {
        // SHA-256("abc").
        #expect(Pinning.certificateHash(Data("abc".utf8))
            == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    }

    @Test func readsASessionAsHuradWritesIt() throws {
        let json = #"{"name":"fix-tests","sandbox":"x","tmux":"t","repo":"r","work_branch":"hura/fix-tests","task":"Fix the tests","state":"waiting","interface":"chat","created_at":1}"#
        let s = try JSONDecoder().decode(Session.self, from: Data(json.utf8))
        #expect(s == Session(name: "fix-tests", state: "waiting", workBranch: "hura/fix-tests",
                             task: "Fix the tests", interface: "chat"))
        #expect(s.isChat && s.isLive)
        // An older record without the field is a terminal session.
        let old = try JSONDecoder().decode(Session.self, from: Data(#"{"name":"a","state":"dead","work_branch":"b"}"#.utf8))
        #expect(old.interface == "terminal" && !old.isLive)
    }

    @Test func readsAPollAsHuradWritesIt() throws {
        let json = #"{"reply":"poll","stat":null,"status":{"state":"waiting","detail":"Bash","source":"hook","ask":"toolu_01","prompt":"Bash: npm test"},"pane":null,"usage":null,"ports":[]}"#
        let p = try JSONDecoder().decode(Poll.self, from: Data(json.utf8))
        #expect(p.status == AgentReport(state: "waiting", detail: "Bash", ask: "toolu_01", prompt: "Bash: npm test"))
    }

    @Test func readsAPushView() throws {
        let json = #"{"reply":"push","configured":true,"relay_url":"https://r","label":"studio","prefs":{"claude_needs_input":true},"encrypted":true,"broken":null,"last_success_at":0,"last_error":null,"queued":2}"#
        let s = try JSONDecoder().decode(PushView.self, from: Data(json.utf8)).status
        #expect(s == PushStatus(configured: true, label: "studio", prefs: ["claude_needs_input": true],
                                broken: nil, lastError: nil, encrypted: true, queued: 2))
    }
}

/// `crates/hurad/src/push/push-crypto.json`: sealed by hurad, and opened here.
/// The file is the contract between the two ends.
struct PushCryptoTests {
    static func vectors() -> [[String: Any]] {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // HuraKitTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // HuraKit
            .deletingLastPathComponent() // ios
            .deletingLastPathComponent() // apps
            .deletingLastPathComponent() // repository root
            .appendingPathComponent("crates/hurad/src/push/push-crypto.json")
        let data = try! Data(contentsOf: url)
        return try! JSONSerialization.jsonObject(with: data) as! [[String: Any]]
    }

    @Test func agreesWithHurad() throws {
        let vectors = Self.vectors()
        #expect(vectors.count >= 5)
        for v in vectors {
            let name = v["name"] as! String
            let key = try #require(Data(base64URL: v["key"] as! String))
            let payload: [AnyHashable: Any] = ["hura": [
                "kind": v["kind"]!, "server_id": v["server_id"]!, "session": v["session"] as Any,
                "ask_id": v["ask_id"] as Any, "enc": v["encrypted"]!,
            ].compactMapValues { $0 is NSNull ? nil : $0 }]
            let keys = InMemoryPushKeyStore()
            try keys.save(key, serverID: v["server_id"] as! String)
            let text = PushCrypto.open(payload: payload, keys: keys)
            if v["fails"] as? Bool == true {
                #expect(text == nil, "\(name)")
            } else {
                let expected = try JSONDecoder().decode(PushCrypto.Text.self,
                                                        from: Data((v["plaintext"] as! String).utf8))
                #expect(text == expected, "\(name)")
                #expect(text?.title == clipped(v["title"] as! String), "\(name)")
            }
        }
    }

    /// What hurad clips a title to: 100 code points, the last an ellipsis.
    private func clipped(_ s: String) -> String {
        s.count <= 100 ? s : String(s.prefix(99)) + "…"
    }
}

/// Against a running hurad: `HURA_PAIRING='hura://…' swift test --filter Live`.
/// Skipped otherwise.
private let huraPairing = ProcessInfo.processInfo.environment["HURA_PAIRING"]

@MainActor
@Suite(.enabled(if: huraPairing != nil, "set HURA_PAIRING to run"))
struct LiveTests {
    @Test func pairsAndAsks() async throws {
        let url = try PairingString(huraPairing!)
        let record = try await Pairing.pair(url, name: "live")
        #expect(record.name == "live")
        let client = ServerClient(record: record)
        #expect(try await client.hello().server == "hurad")
        #expect(!(try await client.pushStatus().configured))
        let server = Server(record: record)
        await server.refresh()
        #expect(server.state == .online)
    }

    @Test func refusesAWrongTokenAndAWrongCertificate() async throws {
        var url = try PairingString(huraPairing!)
        url.token = "nope"
        await #expect(throws: PairingError.tokenRefused) { try await Pairing.pair(url) }
        url = try PairingString(huraPairing!)
        url.fingerprint = String(repeating: "0", count: 64)
        await #expect(throws: PairingError.pinMismatch) { try await Pairing.pair(url) }
    }
}
