import ActivityKit
import Foundation
import HuraKit

/// A session's Live Activity, on the phone's side. The relay starts, updates
/// and ends them (`docs/relay-api.md`, "Live Activities"); all the app does is
/// hand it the tokens ActivityKit gives out for that, and update an activity
/// itself straight after an answer from it, so the Lock Screen doesn't keep
/// offering Allow until the server's next look.
///
/// Push-to-start is iOS 17.2's, so before that nothing starts an activity and
/// there is nothing to do.
@MainActor
final class LiveActivities {
    private let push: PushManager
    /// The activities whose tokens are being watched, by ActivityKit's id.
    private var watching: Set<String> = []

    init(push: PushManager) {
        self.push = push
    }

    /// At launch, before anything else: iOS starts the app in the background
    /// when a push has started an activity, so that it can say where to send
    /// the activity's updates, and these are what hear it.
    func start() {
        guard #available(iOS 17.2, *) else { return }
        Task {
            for await token in Activity<SessionActivityAttributes>.pushToStartTokenUpdates {
                await push.activityStartTokenChanged(token)
            }
        }
        for activity in Activity<SessionActivityAttributes>.activities { watch(activity) }
        Task {
            for await activity in Activity<SessionActivityAttributes>.activityUpdates { watch(activity) }
        }
        Task { await Self.dedupe() }
    }

    /// Each time the app comes to the front: the relay is told every
    /// activity's token again. iOS doesn't always give the app the time to
    /// tell it when a push starts one, and until it hears, the relay can only
    /// keep what it would have said (`docs/relay-api.md`, "Live Activities").
    func resumed() {
        guard #available(iOS 17.2, *) else { return }
        for activity in Activity<SessionActivityAttributes>.activities {
            let a = activity.attributes
            guard let token = activity.pushToken, activity.activityState == .active else { continue }
            Task { await push.activityTokenChanged(token, linkID: a.linkID, session: a.session) }
        }
        Task { await Self.dedupe() }
    }

    /// One activity per session. A relay from before it waited for an
    /// activity's token started another on each change, and the stack stayed
    /// on the Lock Screen; the newest is the one the relay can update.
    nonisolated private static func dedupe() async {
        let live = Activity<SessionActivityAttributes>.activities.filter { $0.activityState == .active }
        let bySession = Dictionary(grouping: live) { "\($0.attributes.serverID):\($0.attributes.session)" }
        for (_, same) in bySession where same.count > 1 {
            let newest = same.max { ($0.content.state.at ?? 0) < ($1.content.state.at ?? 0) }
            for activity in same where activity.id != newest?.id {
                await activity.end(nil, dismissalPolicy: .immediate)
            }
        }
    }

    private func watch(_ activity: Activity<SessionActivityAttributes>) {
        guard watching.insert(activity.id).inserted else { return }
        let a = activity.attributes
        Task {
            for await token in activity.pushTokenUpdates {
                await push.activityTokenChanged(token, linkID: a.linkID, session: a.session)
            }
        }
        Task {
            for await state in activity.activityStateUpdates where state == .ended || state == .dismissed {
                await push.activityEnded(linkID: a.linkID, session: a.session)
                watching.remove(activity.id)
                break
            }
        }
    }

    /// After Allow or Deny on an activity: show it working again, with what
    /// was said, until the server says what came next. The text is shown in
    /// the clear because only this phone ever sees a local update.
    static func answered(_ allow: Bool, serverID: String, session: String, askID: String) async {
        let keys = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)
        for activity in Activity<SessionActivityAttributes>.activities
        where activity.attributes.serverID == serverID && activity.attributes.session == session {
            let state = activity.content.state
            guard state.askID == askID else { continue }
            let text = state.text(serverID: serverID, session: session, keys: keys)
            let next = state.answered(text, body: allow ? "Allowed" : "Declined")
            await activity.update(ActivityContent(state: next, staleDate: nil))
        }
    }
}

/// Where the Live Activity's Allow and Deny (`AnswerIntents.swift`) land in
/// the app: the same answer a notification's actions give, to that request
/// and no other.
enum LiveActivityAnswer {
    @MainActor
    static func answer(_ allow: Bool, serverID: String, session: String, askID: String) async {
        guard let model = AppDelegate.current?.model else { return }
        let target = NotificationTarget(serverID: serverID, session: session, askID: askID)
        if await model.answer(allow, to: target) {
            await LiveActivities.answered(allow, serverID: serverID, session: session, askID: askID)
        }
    }
}

#if DEBUG
extension LiveActivities {
    /// A made-up session's activity, started here rather than by a push, for
    /// `-HuraActivity` (`HuraApp.pairFromLaunchArguments`).
    static let demoServerID = "h_demodemodemodemo"

    static func demo(_ phase: SessionPhase) {
        let body = switch phase {
        case .working: "Claude is working"
        case .permission: "Bash: npm test -- --run login.spec.ts"
        case .input: "Which database should the cache use: Redis or SQLite?"
        case .done: "Claude finished and is waiting for you"
        case .failed: "The session failed"
        case .ended: "The session ended"
        }
        // Sealed as hurad seals it, under a key kept where the real ones are,
        // so the widget has to open it as it opens a real one.
        let serverID = demoServerID, session = "fix-flaky-login-test"
        let askID = phase == .permission ? "toolu_demo" : nil
        let keys = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)
        do {
            let key = try keys.key(serverID: serverID) ?? PushCrypto.newKey()
            try keys.save(key, serverID: serverID)
            try keys.saveName("devbox", serverID: serverID)
            // Where the widget reads it on a phone (`ActivityKeyFile`).
            try ActivityKeyFile.shared?.save(key, serverID: serverID)
            try ActivityKeyFile.shared?.saveName("devbox", serverID: serverID)
            let enc = try PushCrypto.seal(PushCrypto.Text(title: "Fix the flaky login test", subtitle: "scope-creep",
                                                          body: body),
                                          key: key, serverID: serverID, kind: SessionActivity.kind, session: session,
                                          askID: askID)
            let state = SessionActivityState(phase: phase, at: Date().addingTimeInterval(-73),
                                             since: Date().addingTimeInterval(-312), askID: askID, enc: enc)
            _ = try Activity.request(attributes: SessionActivityAttributes(serverID: serverID, linkID: "l_demo",
                                                                           session: session),
                                     content: ActivityContent(state: state, staleDate: nil))
        } catch {
            print("hura: couldn't start the demo activity: \(error)")
        }
    }
}
#endif
