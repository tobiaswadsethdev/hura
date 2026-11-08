// The app's side of hura cloud (hura's `docs/cloud.md`): signing in with an
// emailed code, the account and its devices, and the trackers kept there.
// The service has an ordinary publicly trusted certificate, so this uses
// URLSession and the system trust store, as the relay does.
//
// Signing in is optional. Without it, trackers and their tokens stay in this
// phone's Keychain and are read from here, as they always were.

import Foundation

/// This phone signed in to an account: the refresh token it keeps until it
/// signs out, in the Keychain. Access tokens last an hour and are only ever
/// held in memory.
public struct CloudSession: Codable, Equatable, Sendable {
    public var base: URL
    public var email: String
    public var account: String
    public var device: String
    public var refreshToken: String

    public init(base: URL, email: String, account: String, device: String, refreshToken: String) {
        self.base = base
        self.email = email
        self.account = account
        self.device = device
        self.refreshToken = refreshToken
    }
}

extension CloudSession: CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable {
    public var description: String { "CloudSession(\(email), \(base.absoluteString))" }
    public var debugDescription: String { description }
    public var customMirror: Mirror {
        Mirror(self, children: ["email": email, "base": base, "refreshToken": "<redacted>"])
    }
}

public protocol CloudSessionStore: Sendable {
    func load() throws -> CloudSession?
    func save(_ session: CloudSession?) throws
}

public struct KeychainCloudSessionStore: CloudSessionStore {
    let items: KeychainItems

    /// In the app's own group, beside the trackers: nothing else needs it.
    public init(service: String = "dev.tobiaswadseth.hura.cloud") {
        items = KeychainItems(service: service, accessGroup: nil)
    }

    public func load() throws -> CloudSession? {
        guard let data = try items.get(account: "session") else { return nil }
        return try JSONDecoder().decode(CloudSession.self, from: data)
    }

    public func save(_ session: CloudSession?) throws {
        if let session {
            try items.set(JSONEncoder().encode(session), account: "session")
        } else {
            try items.delete(account: "session")
        }
    }
}

public final class InMemoryCloudSessionStore: CloudSessionStore, @unchecked Sendable {
    private var session: CloudSession?
    private let lock = NSLock()
    public init(_ session: CloudSession? = nil) { self.session = session }
    public func load() throws -> CloudSession? { lock.withLock { session } }
    public func save(_ s: CloudSession?) throws { lock.withLock { session = s } }
}

/// What the service said, in words it means to be shown.
public struct CloudError: Error, Equatable, LocalizedError {
    public var status: Int
    public var message: String

    public var errorDescription: String? { message }
    /// The device was signed out, here or from another of the account's.
    public var isSignedOut: Bool { status == 401 }
}

public struct CloudAccount: Decodable, Equatable, Sendable {
    public struct SignIn: Decodable, Equatable, Sendable {
        /// `email` or `apple`.
        public var provider: String
        public var email: String?
    }

    public var account: String
    public var plan: String
    public var signIns: [SignIn]

    enum CodingKeys: String, CodingKey {
        case account, plan
        case signIns = "sign_ins"
    }
}

public struct CloudDevice: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var platform: String
    public var createdAt: Date
    /// When it last renewed its access token: about when it was last used.
    public var lastSeenAt: Date
    /// The device asking.
    public var current: Bool

    enum CodingKeys: String, CodingKey {
        case id, name, platform, current
        case createdAt = "created_at"
        case lastSeenAt = "last_seen_at"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        platform = try c.decode(String.self, forKey: .platform)
        current = try c.decode(Bool.self, forKey: .current)
        createdAt = Date(timeIntervalSince1970: try c.decode(Double.self, forKey: .createdAt))
        lastSeenAt = Date(timeIntervalSince1970: try c.decode(Double.self, forKey: .lastSeenAt))
    }
}

/// hura cloud, as this phone is signed in to it (or not).
public actor Cloud {
    public static let defaultBase = URL(string: "https://api.gethura.app")!
    public static let shared = Cloud(store: KeychainCloudSessionStore())

    public typealias Send = @Sendable (URLRequest) async throws -> (Data, URLResponse)

    private let store: CloudSessionStore
    private let send: Send
    private var session: CloudSession?
    private var access: (token: String, until: Date)?

    public init(store: CloudSessionStore, send: Send? = nil) {
        self.store = store
        session = try? store.load()
        self.send = send ?? {
            let config = URLSessionConfiguration.ephemeral
            config.timeoutIntervalForRequest = 30
            config.urlCache = nil
            config.httpCookieStorage = nil
            let urlSession = URLSession(configuration: config)
            return { try await urlSession.data(for: $0) }
        }()
    }

    public var current: CloudSession? { session }
    public var isSignedIn: Bool { session != nil }

    // MARK: Signing in

    /// Emails a six-digit code to `email`.
    public func sendCode(to email: String, base: URL = Cloud.defaultBase) async throws {
        _ = try await request(base: base, "POST", "/v1/sign-in/email", body: ["email": .string(email)])
    }

    /// Signs this phone in with the code that was emailed.
    @discardableResult
    public func verify(email: String, code: String, deviceName: String,
                       base: URL = Cloud.defaultBase) async throws -> CloudSession {
        let answer = try await request(base: base, "POST", "/v1/sign-in/email/verify", body: [
            "email": .string(email),
            "code": .string(code),
            "device": .object(["name": .string(deviceName), "platform": .string("ios")]),
        ])
        struct Signed: Decodable {
            var account: String
            var device: String
            var refresh_token: String
            var access_token: String
            var expires_in: Double
        }
        let signed = try JSONDecoder().decode(Signed.self, from: answer)
        let session = CloudSession(base: base, email: email.trimmingCharacters(in: .whitespaces).lowercased(),
                                   account: signed.account, device: signed.device,
                                   refreshToken: signed.refresh_token)
        do {
            try store.save(session)
        } catch {
            // Signed in on the service with nowhere to keep it: sign that
            // device straight back out, rather than leave one in the list
            // that nothing can use.
            _ = try? await request(base: base, "POST", "/v1/sign-out", bearer: signed.access_token)
            throw error
        }
        self.session = session
        access = (signed.access_token, Date().addingTimeInterval(signed.expires_in))
        return session
    }

    /// Signs this phone out: on the service if it answers, and here
    /// regardless.
    public func signOut() async {
        if session != nil {
            _ = try? await authorized("POST", "/v1/sign-out")
        }
        forget()
    }

    // MARK: The account

    public func account() async throws -> CloudAccount {
        try JSONDecoder().decode(CloudAccount.self, from: try await authorized("GET", "/v1/me"))
    }

    public func devices() async throws -> [CloudDevice] {
        try JSONDecoder().decode([CloudDevice].self, from: try await authorized("GET", "/v1/devices"))
    }

    /// Signs another of the account's devices out.
    public func signOut(device id: String) async throws {
        _ = try await authorized("DELETE", "/v1/devices/\(id)")
    }

    // MARK: Trackers

    /// The account's trackers. A token is never sent back: `hasToken` says
    /// whether there is one.
    public func trackers() async throws -> [StoredTracker] {
        try configured(try await authorized("GET", "/v1/trackers"))
    }

    public func addTracker(_ source: TrackerSource, token: String?) async throws -> [StoredTracker] {
        var body: [String: JSONValue] = ["source": try TrackerCore.json(source)]
        if let token { body["token"] = .string(token) }
        return try configured(try await authorized("POST", "/v1/trackers", body: body))
    }

    public func editTracker(_ name: String, _ source: TrackerSource) async throws -> [StoredTracker] {
        try configured(try await authorized("PUT", "/v1/trackers/\(path(name))",
                                            body: ["source": try TrackerCore.json(source)]))
    }

    public func setToken(_ name: String, _ token: String) async throws -> [StoredTracker] {
        try configured(try await authorized("PUT", "/v1/trackers/\(path(name))/token",
                                            body: ["token": .string(token)]))
    }

    public func forgetTracker(_ name: String) async throws -> [StoredTracker] {
        try configured(try await authorized("DELETE", "/v1/trackers/\(path(name))"))
    }

    /// One of hura-core's tracker requests, on the account's trackers, as
    /// `{"ok": ...}` or `{"error": ...}`: what the phone's own copy answers.
    func tracker(_ request: [String: JSONValue]) async throws -> JSONValue {
        let data = try await authorized("POST", "/v1/tracker", body: request)
        return try JSONDecoder().decode(JSONValue.self, from: data)
    }

    // MARK: Requests

    private func configured(_ data: Data) throws -> [StoredTracker] {
        struct Configured: Decodable {
            var source: TrackerSource
            var token_set: Bool
        }
        return try JSONDecoder().decode([Configured].self, from: data).map {
            var t = StoredTracker(source: $0.source, token: nil)
            t.tokenKept = $0.token_set
            return t
        }
    }

    private func path(_ name: String) -> String {
        name.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? name
    }

    /// A request as the signed-in device. An access token that is refused is
    /// renewed once; a refresh token that is refused means this device was
    /// signed out, and it is forgotten here too.
    private func authorized(_ method: String, _ path: String, body: [String: JSONValue]? = nil) async throws -> Data {
        guard let session else { throw CloudError(status: 401, message: "Not signed in.") }
        do {
            return try await request(base: session.base, method, path, body: body, bearer: try await accessToken())
        } catch let e as CloudError where e.isSignedOut {
            access = nil
            return try await request(base: session.base, method, path, body: body, bearer: try await accessToken())
        }
    }

    private func accessToken() async throws -> String {
        if let access, access.until > Date().addingTimeInterval(60) { return access.token }
        guard let session else { throw CloudError(status: 401, message: "Not signed in.") }
        do {
            let answer = try await request(base: session.base, "POST", "/v1/token",
                                           body: ["refresh_token": .string(session.refreshToken)])
            struct Access: Decodable {
                var access_token: String
                var expires_in: Double
            }
            let a = try JSONDecoder().decode(Access.self, from: answer)
            access = (a.access_token, Date().addingTimeInterval(a.expires_in))
            return a.access_token
        } catch let e as CloudError where e.isSignedOut {
            forget()
            throw CloudError(status: 401, message: "This iPhone was signed out of hura cloud.")
        }
    }

    private func forget() {
        try? store.save(nil)
        session = nil
        access = nil
    }

    private func request(base: URL, _ method: String, _ path: String, body: [String: JSONValue]? = nil,
                         bearer: String? = nil) async throws -> Data {
        var r = URLRequest(url: URL(string: path, relativeTo: base)!)
        r.httpMethod = method
        if let bearer { r.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
        if let body {
            r.setValue("application/json", forHTTPHeaderField: "Content-Type")
            r.httpBody = try JSONEncoder().encode(JSONValue.object(body))
        }
        let (data, response) = try await send(r)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            let said = (try? JSONDecoder().decode(JSONValue.self, from: data))?["error"]?.string
            throw CloudError(status: status, message: said ?? "hura cloud answered \(status).")
        }
        return data
    }
}
