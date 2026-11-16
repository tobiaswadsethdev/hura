// A session as a Live Activity (hura's `docs/relay-api.md`, "Live
// Activities"): what the relay's ActivityKit pushes carry, and reading it.
// The widget that draws the activity and the app that starts and answers it
// both use these.

import Foundation
#if canImport(ActivityKit) && os(iOS)
import ActivityKit
#endif

/// What a session is doing, as far as its Live Activity is concerned.
public enum SessionPhase: String, Codable, Sendable {
    case working
    /// Waiting on a permission request, which Allow and Deny answer.
    case permission
    /// Waiting on anything else: a question, a plan, a terminal's prompt.
    case input
    case done
    case failed
    /// The session is gone.
    case ended

    /// Whether the session is waiting on somebody.
    public var isWaiting: Bool { self == .permission || self == .input }
    /// Whether the activity is over.
    public var isOver: Bool { self == .done || self == .failed || self == .ended }
}

/// An activity's `content-state`, as the relay sends it: the phase, when the
/// session moved into it, the request a permission waits on, and the text,
/// either sealed (`enc`) or plain. The app writes plain text into it itself
/// when it updates an activity locally, since it can read the sealed text.
public struct SessionActivityState: Codable, Hashable, Sendable {
    /// A raw string, so a phase a newer relay sends doesn't fail to decode
    /// and blank the activity: it reads as working.
    public var phaseName: String
    /// Unix seconds. Not a `Date`: ActivityKit decodes the content state with
    /// a plain `JSONDecoder`, which would read a number as seconds since 2001.
    public var at: Double?
    public var askID: String?
    public var enc: String?
    public var title: String?
    public var subtitle: String?
    public var body: String?

    enum CodingKeys: String, CodingKey {
        case phaseName = "phase"
        case at
        case askID = "ask_id"
        case enc, title, subtitle, body
    }

    public init(phase: SessionPhase, at: Date? = nil, askID: String? = nil, enc: String? = nil,
                title: String? = nil, subtitle: String? = nil, body: String? = nil) {
        phaseName = phase.rawValue
        self.at = at?.timeIntervalSince1970
        self.askID = askID
        self.enc = enc
        self.title = title
        self.subtitle = subtitle
        self.body = body
    }

    public var phase: SessionPhase { SessionPhase(rawValue: phaseName) ?? .working }
    public var since: Date? { at.map(Date.init(timeIntervalSince1970:)) }

    /// The text to show: opened with the server's key when it is sealed, or
    /// as it came. Nil when it is sealed and can't be opened (no key, a key
    /// from an older link, tampered with), and for a state with no text.
    public func text(serverID: String, session: String, keys: PushKeyStore) -> PushCrypto.Text? {
        if let enc {
            guard let key = try? keys.key(serverID: serverID) else { return nil }
            return try? PushCrypto.open(enc, key: key, serverID: serverID, kind: SessionActivity.kind,
                                        session: session, askID: askID)
        }
        guard let body else { return nil }
        return PushCrypto.Text(title: title ?? session, subtitle: subtitle, body: body)
    }

    /// The same state with its text in the clear and a new phase: what the app
    /// shows straight after answering, before the server says what came next.
    public func answered(_ text: PushCrypto.Text?, body: String, now: Date = Date()) -> SessionActivityState {
        SessionActivityState(phase: .working, at: now, title: text?.title, subtitle: text?.subtitle, body: body)
    }
}

public enum SessionActivity {
    /// The kind the text is sealed under.
    public static let kind = "session_activity"

    /// What a Live Activity opens when it is tapped: `hura://session?server=…&name=…`.
    public static func url(serverID: String, session: String) -> URL {
        var c = URLComponents()
        c.scheme = "hura"
        c.host = "session"
        c.queryItems = [URLQueryItem(name: "server", value: serverID), URLQueryItem(name: "name", value: session)]
        return c.url!
    }

    /// The server and session a `url(serverID:session:)` names.
    public static func target(of url: URL) -> (serverID: String, session: String)? {
        guard url.scheme == "hura", url.host == "session",
              let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
              let server = items.first(where: { $0.name == "server" })?.value,
              let name = items.first(where: { $0.name == "name" })?.value
        else { return nil }
        return (server, name)
    }

    /// Hex, as the relay takes a token.
    public static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }
}

#if canImport(ActivityKit) && os(iOS)
/// The activity showing one session. Its name matches the relay's
/// `attributes-type`, and its fields the `attributes` it sends, so a push can
/// start one.
public struct SessionActivityAttributes: ActivityAttributes {
    public typealias ContentState = SessionActivityState

    public var serverID: String
    /// The relay link the server's pushes come through, which the app gives
    /// the activity's token to.
    public var linkID: String
    public var session: String

    enum CodingKeys: String, CodingKey {
        case serverID = "server_id"
        case linkID = "link_id"
        case session
    }

    public init(serverID: String, linkID: String, session: String) {
        self.serverID = serverID
        self.linkID = linkID
        self.session = session
    }
}
#endif
