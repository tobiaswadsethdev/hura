import ActivityKit
import Foundation
import HuraKit

/// The Live Activity, on the phone's side: one for every session, T3 Code's,
/// which the relay starts, updates and ends (`docs/relay-api.md`, "One
/// activity for every session"). All the app does is hand the relay the tokens
/// ActivityKit gives out for it, and update it itself straight after an answer
/// from it, so the Lock Screen doesn't keep offering Allow until the server's
/// next look.
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
            for await token in Activity<AgentsActivityAttributes>.pushToStartTokenUpdates {
                await push.activityStartTokenChanged(token)
            }
        }
        for activity in Activity<AgentsActivityAttributes>.activities { watch(activity) }
        Task {
            for await activity in Activity<AgentsActivityAttributes>.activityUpdates { watch(activity) }
        }
        Task { await Self.tidy() }
    }

    /// Each time the app comes to the front: the relay is told the activity's
    /// token again. iOS doesn't always give the app the time to tell it when a
    /// push starts one, and until it hears, the relay can only keep the rows.
    func resumed() {
        guard #available(iOS 17.2, *) else { return }
        for activity in Activity<AgentsActivityAttributes>.activities {
            guard let token = activity.pushToken, activity.activityState == .active else { continue }
            Task { await push.agentsActivityTokenChanged(token) }
        }
        Task { await Self.tidy() }
    }

    /// One activity. A session's own, from before there was one for all of
    /// them, is no longer updated once the relay hears this app's style, and
    /// goes; of two of the one, the newest still going stays.
    nonisolated private static func tidy() async {
        for activity in Activity<SessionActivityAttributes>.activities {
            await activity.end(nil, dismissalPolicy: .immediate)
        }
        let shown = Activity<AgentsActivityAttributes>.activities.filter {
            $0.activityState == .active || $0.activityState == .ended
        }
        guard shown.count > 1 else { return }
        let keep = shown.last { $0.activityState == .active } ?? shown.last
        for activity in shown where activity.id != keep?.id {
            await activity.end(nil, dismissalPolicy: .immediate)
        }
    }

    private func watch(_ activity: Activity<AgentsActivityAttributes>) {
        guard watching.insert(activity.id).inserted else { return }
        Task {
            for await token in activity.pushTokenUpdates {
                await push.agentsActivityTokenChanged(token)
            }
        }
        Task {
            for await state in activity.activityStateUpdates where state == .ended || state == .dismissed {
                await push.agentsActivityEnded(activity.pushToken)
                watching.remove(activity.id)
                break
            }
        }
    }

    /// After Allow or Deny on the activity: that session's row shows it
    /// working again, with what was said, until the server says what came
    /// next.
    static func answered(_ allow: Bool, serverID: String, session: String, askID: String) async {
        let keys = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)
        for activity in Activity<AgentsActivityAttributes>.activities {
            guard let next = activity.content.state.answered(serverID: serverID, session: session, askID: askID,
                                                             allow: allow, keys: keys)
            else { continue }
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

    /// Three made-up sessions in the one activity, the first in `phase`,
    /// started here rather than by a push. Their text is sealed as hurad
    /// seals it, under a key kept where the real ones are, so the widget has
    /// to open it as it opens a real one.
    static func demo(_ phase: SessionPhase) {
        let serverID = demoServerID
        let keys = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)
        do {
            let key = try keys.key(serverID: serverID) ?? PushCrypto.newKey()
            try keys.save(key, serverID: serverID)
            try keys.saveName("devbox", serverID: serverID)
            // Where the widget reads it on a phone (`ActivityKeyFile`).
            try ActivityKeyFile.shared?.save(key, serverID: serverID)
            try ActivityKeyFile.shared?.saveName("devbox", serverID: serverID)
            func row(_ session: String, _ title: String, _ project: String, _ phase: SessionPhase, _ body: String,
                     ago: TimeInterval) throws -> AgentRow {
                let askID = phase == .permission ? "toolu_demo" : nil
                let enc = try PushCrypto.seal(PushCrypto.Text(title: title, subtitle: project, body: body), key: key,
                                              serverID: serverID, kind: SessionActivity.kind, session: session,
                                              askID: askID)
                return AgentRow(serverID: serverID, session: session,
                                state: SessionActivityState(phase: phase, at: Date().addingTimeInterval(-ago),
                                                            since: Date().addingTimeInterval(-ago - 240),
                                                            askID: askID, enc: enc))
            }
            let state = AgentsActivityState(rows: [
                try row("fix-flaky-login-test", "Fix the flaky login test", "scope-creep", phase,
                        phase == .permission ? "Bash: npm test -- --run login.spec.ts" : phase.status, ago: 73),
                try row("cache-backend", "Pick a cache backend", "scope-creep", .working, "Claude is working", ago: 140),
                try row("docs-pass", "Tidy the README", "hura", .done, "Claude finished", ago: 600),
            ])
            _ = try Activity.request(attributes: AgentsActivityAttributes(),
                                     content: ActivityContent(state: state, staleDate: nil))
        } catch {
            print("hura: couldn't start the demo activity: \(error)")
        }
    }
}
#endif
