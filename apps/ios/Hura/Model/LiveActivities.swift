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
    static func demo(_ phase: SessionPhase, serverID: String) {
        let body = switch phase {
        case .working: "Claude is working"
        case .permission: "Bash: npm test -- --run login.spec.ts"
        case .input: "Which database should the cache use: Redis or SQLite?"
        case .done: "Claude finished and is waiting for you"
        case .failed: "The session failed"
        case .ended: "The session ended"
        }
        let state = SessionActivityState(phase: phase, at: Date().addingTimeInterval(-73),
                                         askID: phase == .permission ? "toolu_demo" : nil,
                                         title: "Fix the flaky login test", subtitle: "scope-creep", body: body)
        do {
            _ = try Activity.request(attributes: SessionActivityAttributes(serverID: serverID, linkID: "l_demo",
                                                                           session: "fix-flaky-login-test"),
                                     content: ActivityContent(state: state, staleDate: nil))
        } catch {
            print("hura: couldn't start the demo activity: \(error)")
        }
    }
}
#endif
