import AppIntents
import Foundation

// Allow and Deny on a session's Live Activity. This file is compiled into both
// the app and the widget extension: the widget's buttons name the intents,
// and because they are `LiveActivityIntent`s iOS runs `perform` in the app,
// which has the pairing to answer with. The extension never does.
//
// As with the notification's actions, Allow runs something on the server and
// so needs the phone unlocked; Deny only stops something, and doesn't.

struct AllowIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Allow"
    static let isDiscoverable = false
    static let authenticationPolicy = IntentAuthenticationPolicy.requiresAuthentication

    @Parameter(title: "Server") var serverID: String
    @Parameter(title: "Session") var session: String
    @Parameter(title: "Request") var askID: String

    init() {}

    init(serverID: String, session: String, askID: String) {
        self.serverID = serverID
        self.session = session
        self.askID = askID
    }

    func perform() async throws -> some IntentResult {
        #if !HURA_WIDGET
        await LiveActivityAnswer.answer(true, serverID: serverID, session: session, askID: askID)
        #endif
        return .result()
    }
}

struct DenyIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Deny"
    static let isDiscoverable = false
    static let authenticationPolicy = IntentAuthenticationPolicy.alwaysAllowed

    @Parameter(title: "Server") var serverID: String
    @Parameter(title: "Session") var session: String
    @Parameter(title: "Request") var askID: String

    init() {}

    init(serverID: String, session: String, askID: String) {
        self.serverID = serverID
        self.session = session
        self.askID = askID
    }

    func perform() async throws -> some IntentResult {
        #if !HURA_WIDGET
        await LiveActivityAnswer.answer(false, serverID: serverID, session: session, askID: askID)
        #endif
        return .result()
    }
}
