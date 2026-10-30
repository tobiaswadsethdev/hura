import Foundation
import Network
import Testing
@testable import HuraKit

/// A real connection to the phone's loopback, through a scripted websocket
/// standing in for `hurad`.
@MainActor
struct PreviewTests {
    @Test func tunnelsAConnectionThroughAPortChannel() async throws {
        let socket = ScriptedSocket()
        socket.dropWhenDone = false
        let hub = PreviewHub(record: ChatTests.record, transport: { _ in socket })
        let port = Int.random(in: 40000..<50000)
        let preview = try await hub.open(session: "fix-tests", port: port, host: "v6")
        #expect(preview.local == port, "the same number when it is free")
        defer { hub.stopAll() }

        // A browser connects and asks for a page.
        let client = NWConnection(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(preview.local))!, using: .tcp)
        client.start(queue: .main)
        let request = Data("GET / HTTP/1.0\r\n\r\n".utf8)
        client.send(content: request, completion: .contentProcessed { _ in })

        // The hub opened a port channel for it, naming the loopback.
        try await until { !socket.sent.isEmpty }
        let open = socket.sent[0]
        #expect(open["do"]?.string == "open")
        #expect(open["channel"]?["kind"]?.string == "port")
        #expect(open["channel"]?["port"] == .number(Double(port)))
        #expect(open["channel"]?["host"]?.string == "v6")
        guard case let .number(n)? = open["id"] else { Issue.record("no id"); return }
        let id = Int(n)

        // Nothing is read from the browser until the server has opened it.
        socket.say(#"{"is":"opened","id":\#(id)}"#)
        try await until { socket.sent.count >= 2 }
        #expect(socket.sent[1]["do"]?.string == "input")
        #expect(socket.sent[1]["data"]?.string == request.base64EncodedString())

        // What the service answers reaches the browser.
        let response = Data("HTTP/1.0 200 OK\r\n\r\nhello".utf8)
        socket.say(#"{"is":"output","id":\#(id),"data":"\#(response.base64EncodedString())"}"#)
        let got: Data = try await withCheckedThrowingContinuation { done in
            client.receive(minimumIncompleteLength: response.count, maximumLength: 1 << 16) { data, _, _, error in
                if let error { done.resume(throwing: error) } else { done.resume(returning: data ?? Data()) }
            }
        }
        #expect(got == response)
        #expect(hub.previews.first?.connections == 1)

        // The server closing the channel closes the connection.
        socket.say(#"{"is":"closed","id":\#(id),"reason":null}"#)
        try await until { hub.previews.first?.connections == 0 }
        client.cancel()
    }

    private func until(_ condition: () -> Bool) async throws {
        for _ in 0..<300 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("timed out")
    }
}
