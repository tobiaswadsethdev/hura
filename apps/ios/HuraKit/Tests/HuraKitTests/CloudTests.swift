import CryptoKit
import Foundation
import Testing
@testable import HuraKit

/// The app's side of hura cloud, against a stand-in for the service.
struct CloudTests {
    /// Answers by method and path, and keeps every request it was sent.
    final class Service: @unchecked Sendable {
        typealias Answer = (status: Int, body: String)
        private let lock = NSLock()
        private var answers: [String: [Answer]] = [:]
        /// Routes whose last answer has been given at least once.
        private var given: Set<String> = []
        private(set) var sent: [URLRequest] = []

        /// Queued per route: each request takes the next, and the last repeats
        /// until another is queued after it has been given.
        func on(_ route: String, _ status: Int = 200, _ body: String) {
            lock.withLock {
                if given.remove(route) != nil { answers[route] = [] }
                answers[route, default: []].append((status, body))
            }
        }

        func requests(_ route: String) -> [URLRequest] {
            lock.withLock { sent.filter { "\($0.httpMethod ?? "") \($0.url?.path ?? "")" == route } }
        }

        func body(_ r: URLRequest) -> JSONValue? {
            r.httpBody.flatMap { try? JSONDecoder().decode(JSONValue.self, from: $0) }
        }

        var send: Cloud.Send {
            { [self] request in
                let route = "\(request.httpMethod ?? "") \(request.url?.path ?? "")"
                let answer: Answer = lock.withLock {
                    sent.append(request)
                    guard var queue = answers[route], !queue.isEmpty else { return (404, #"{"error":"no route"}"#) }
                    let next: Answer
                    if queue.count > 1 {
                        next = queue.removeFirst()
                    } else {
                        next = queue[0]
                        given.insert(route)
                    }
                    answers[route] = queue
                    return next
                }
                let response = HTTPURLResponse(url: request.url!, statusCode: answer.status, httpVersion: nil,
                                               headerFields: nil)!
                return (Data(answer.body.utf8), response)
            }
        }
    }

    static let signedIn = #"{"account":"acc","device":"dev","refresh_token":"hura_rt_r","access_token":"hura_at_1","expires_in":3600}"#

    func signedInCloud(_ service: Service) async throws -> (Cloud, InMemoryCloudSessionStore) {
        service.on("POST /v1/sign-in/email/verify", 200, Self.signedIn)
        let store = InMemoryCloudSessionStore()
        let cloud = Cloud(store: store, send: service.send)
        try await cloud.verify(email: " Me@Example.com ", code: "123456", deviceName: "iPhone")
        return (cloud, store)
    }

    @Test func signingInKeepsTheSessionAndUsesItsToken() async throws {
        let service = Service()
        let (cloud, store) = try await signedInCloud(service)
        #expect(try store.load()?.email == "me@example.com")
        #expect(try store.load()?.refreshToken == "hura_rt_r")
        let verify = try #require(service.requests("POST /v1/sign-in/email/verify").first)
        #expect(service.body(verify)?["device"]?["platform"]?.string == "ios")

        service.on("GET /v1/me", 200, #"{"account":"acc","plan":"free","sign_ins":[{"provider":"email","email":"me@example.com"}]}"#)
        let me = try await cloud.account()
        #expect(me.plan == "free")
        let asked = try #require(service.requests("GET /v1/me").first)
        #expect(asked.value(forHTTPHeaderField: "Authorization") == "Bearer hura_at_1")
    }

    /// An access token the service refuses is renewed with the refresh token,
    /// and the request made again.
    @Test func aRefusedAccessTokenIsRenewedOnce() async throws {
        let service = Service()
        let (cloud, _) = try await signedInCloud(service)
        service.on("GET /v1/devices", 401, #"{"error":"not signed in"}"#)
        service.on("GET /v1/devices", 200, #"[{"id":"dev","name":"iPhone","platform":"ios","created_at":1,"last_seen_at":2,"current":true}]"#)
        service.on("POST /v1/token", 200, #"{"access_token":"hura_at_2","expires_in":3600}"#)
        let devices = try await cloud.devices()
        #expect(devices.map(\.name) == ["iPhone"])
        #expect(service.requests("GET /v1/devices").last?.value(forHTTPHeaderField: "Authorization") == "Bearer hura_at_2")
    }

    /// A refresh token the service refuses means this phone was signed out
    /// from elsewhere: it is forgotten here too.
    @Test func aRefusedRefreshTokenSignsThePhoneOut() async throws {
        let service = Service()
        let (cloud, store) = try await signedInCloud(service)
        service.on("GET /v1/me", 401, #"{"error":"not signed in"}"#)
        service.on("POST /v1/token", 401, #"{"error":"not signed in"}"#)
        await #expect(throws: CloudError.self) { _ = try await cloud.account() }
        #expect(await !cloud.isSignedIn)
        #expect(try store.load() == nil)
    }

    @Test func theServicesTrackersSayWhetherTheyHaveATokenButNeverWhatItIs() async throws {
        let service = Service()
        let (cloud, _) = try await signedInCloud(service)
        service.on("GET /v1/trackers", 200, #"[{"source":{"kind":"git-hub","name":"gh"},"token_set":true},{"source":{"kind":"jira","name":"work","site":"https://x.atlassian.net","email":"a@b.c"},"token_set":false}]"#)
        let trackers = try await cloud.trackers()
        #expect(trackers.map(\.hasToken) == [true, false])
        #expect(trackers.allSatisfy { $0.token == nil })
        // Kept locally, `tokenKept` is not written down.
        let encoded = String(decoding: try JSONEncoder().encode(trackers[0]), as: UTF8.self)
        #expect(!encoded.contains("tokenKept"))
    }

    /// Signed in, a tracker request goes to the service with the tracker
    /// named rather than sent; what needs no tracker is answered here.
    @Test func trackerRequestsGoToTheServiceWhenSignedIn() async throws {
        let service = Service()
        let (cloud, _) = try await signedInCloud(service)
        service.on("POST /v1/tracker", 200, #"{"ok":{"tasks":[],"pulls":[],"warnings":["gh: no token yet"],"read":[],"pulls_read":[],"filters_read":[]}}"#)
        let mine = StoredTracker(source: TrackerSource(kind: "git-hub", name: "gh"), token: "ghp_local")

        let inbox = try await TrackerCore.$cloud.withValue(cloud) {
            try await TrackerCore.inbox([mine], prefix: "hura")
        }
        #expect(inbox.warnings == ["gh: no token yet"])
        let asked = try #require(service.requests("POST /v1/tracker").first)
        #expect(service.body(asked)?["op"]?.string == "inbox")
        #expect(service.body(asked)?["trackers"] == nil, "the service reads its own")

        service.on("POST /v1/tracker", 200, #"{"error":"no tracker called `gh`"}"#)
        await #expect(throws: TrackerError.self) {
            try await TrackerCore.$cloud.withValue(cloud) { _ = try await TrackerCore.issue(mine, key: "1") }
        }
        let issue = try #require(service.requests("POST /v1/tracker").last)
        #expect(service.body(issue)?["tracker"]?.string == "gh", "named, and no token sent")
        #expect(!String(decoding: issue.httpBody ?? Data(), as: UTF8.self).contains("ghp_local"))

        let before = service.sent.count
        let branch = try await TrackerCore.$cloud.withValue(cloud) {
            try await TrackerCore.branch(prefix: "hura", key: "P-1", title: "Add it")
        }
        #expect(branch.hasPrefix("hura/P-1"))
        #expect(service.sent.count == before, "answered here")
    }

    /// A Keychain that refuses the session must not leave a device on the
    /// account that nothing holds the token for.
    @Test func aSessionThatCannotBeKeptIsSignedStraightBackOut() async throws {
        struct Refusing: CloudSessionStore {
            func load() throws -> CloudSession? { nil }
            func save(_ session: CloudSession?) throws { throw KeychainError(status: -34018) }
        }
        let service = Service()
        service.on("POST /v1/sign-in/email/verify", 200, Self.signedIn)
        service.on("POST /v1/sign-out", 204, "")
        let cloud = Cloud(store: Refusing(), send: service.send)
        await #expect(throws: KeychainError.self) {
            try await cloud.verify(email: "me@example.com", code: "123456", deviceName: "iPhone")
        }
        #expect(await !cloud.isSignedIn)
        let out = try #require(service.requests("POST /v1/sign-out").first)
        #expect(out.value(forHTTPHeaderField: "Authorization") == "Bearer hura_at_1")
    }

    @Test func signingOutForgetsTheSessionEvenWhenTheServiceIsAway() async throws {
        let service = Service()
        let (cloud, store) = try await signedInCloud(service)
        service.on("POST /v1/sign-out", 503, "")
        await cloud.signOut()
        #expect(await !cloud.isSignedIn)
        #expect(try store.load() == nil)
    }

    @Test func thePushRouteIsHandedToTheServiceAndReadBack() async throws {
        let service = Service()
        let (cloud, _) = try await signedInCloud(service)
        service.on("PUT /v1/push", 204, "")
        service.on("GET /v1/push", 200, #"{"configured":true,"broken":null}"#)
        try await cloud.setPush(serverID: "h_aaaaaaaaaaaaaaaa", pushToken: "hup_t", pushKey: "a2V5")
        let put = try #require(service.requests("PUT /v1/push").first)
        #expect(service.body(put)?["push_token"]?.string == "hup_t")
        #expect(service.body(put)?["server_id"]?.string == "h_aaaaaaaaaaaaaaaa")
        let route = try await cloud.pushRoute()
        #expect(route.configured && route.broken == nil)
    }

    /// hura cloud's text carries the ticket a tap opens, inside the
    /// ciphertext, as the service encrypts it (`push::encrypt`).
    @Test func aTrackerChangeOpensWithItsTicket() throws {
        let key = PushCrypto.newKey()
        let plain = Data(#"{"title":"P-1 · To Do → Done","body":"Add it","tracker":"work","id":"10042"}"#.utf8)
        let aad = Data("hura-push-v1\nh_aaaaaaaaaaaaaaaa\ntracker_change\n\n".utf8)
        let box = try AES.GCM.seal(plain, using: SymmetricKey(data: key), authenticating: aad)
        let text = try PushCrypto.open(box.combined!.base64URL, key: key, serverID: "h_aaaaaaaaaaaaaaaa",
                                       kind: "tracker_change", session: nil, askID: nil)
        #expect(text.title == "P-1 · To Do → Done")
        #expect(text.tracker == "work" && text.id == "10042")
    }

    @Test func signingInWithAppleSendsTheTokenAndKeepsTheSession() async throws {
        let service = Service()
        service.on("POST /v1/sign-in/apple", 200, Self.signedIn)
        let store = InMemoryCloudSessionStore()
        let cloud = Cloud(store: store, send: service.send)
        try await cloud.signInWithApple(identityToken: "eyJ.x.y", nonce: "n", email: nil, deviceName: "iPhone")
        let sent = try #require(service.requests("POST /v1/sign-in/apple").first)
        #expect(service.body(sent)?["identity_token"]?.string == "eyJ.x.y")
        #expect(service.body(sent)?["nonce"]?.string == "n")
        #expect(try store.load()?.refreshToken == "hura_rt_r")
        #expect(try store.load()?.email == "", "Apple shares an address only on the first sign-in")
    }
}
