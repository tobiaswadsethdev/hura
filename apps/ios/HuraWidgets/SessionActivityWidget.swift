import ActivityKit
import AppIntents
import HuraKit
import SwiftUI
import WidgetKit

/// A session on the Lock Screen and in the Dynamic Island: what it is doing,
/// for how long, and, while it waits on a permission, Allow and Deny.
///
/// The relay sends the text sealed, as it does a notification's, so it is
/// opened here, each time the activity is drawn, with the key the app keeps
/// for the server in the Keychain group this extension shares.
struct SessionActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: SessionActivityAttributes.self) { context in
            LockScreenView(shown: Shown(context))
                .padding(16)
                .activityBackgroundTint(nil)
                .widgetURL(SessionActivity.url(serverID: context.attributes.serverID,
                                               session: context.attributes.session))
        } dynamicIsland: { context in
            let shown = Shown(context)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    PhaseMark(phase: shown.phase)
                        .font(.title2)
                        .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    PhaseClock(shown: shown)
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .padding(.trailing, 4)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(shown.title)
                        .font(.headline)
                        .lineLimit(1)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(shown.body)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Answers(shown: shown)
                    }
                    .padding(.horizontal, 4)
                }
            } compactLeading: {
                PhaseMark(phase: shown.phase)
            } compactTrailing: {
                PhaseClock(shown: shown)
                    .font(.caption.monospacedDigit())
                    .frame(maxWidth: 44)
            } minimal: {
                PhaseMark(phase: shown.phase)
            }
            .widgetURL(SessionActivity.url(serverID: context.attributes.serverID, session: context.attributes.session))
            .keylineTint(shown.phase.color)
        }
    }
}

/// What an activity shows, worked out once per drawing.
struct Shown {
    var serverID: String
    var session: String
    var phase: SessionPhase
    var since: Date?
    var askID: String?
    var title: String
    var subtitle: String
    var body: String
    /// Whether the text was read. Allow is only an answer if the screen says
    /// what it allows, so without the text there is only Deny.
    var readable: Bool

    init(_ context: ActivityViewContext<SessionActivityAttributes>) {
        self.init(attributes: context.attributes, state: context.state)
    }

    init(attributes: SessionActivityAttributes, state: SessionActivityState,
         keys: PushKeyStore = KeychainPushKeyStore(accessGroup: KeychainPushKeyStore.sharedGroup)) {
        serverID = attributes.serverID
        session = attributes.session
        phase = state.phase
        since = state.since
        askID = state.askID
        let text = state.text(serverID: serverID, session: session, keys: keys)
        readable = text != nil
        title = text?.title ?? session
        subtitle = PushCrypto.subtitle(text?.subtitle, serverName: (try? keys.name(serverID: serverID)) ?? nil)
        body = text?.body ?? phase.generic
    }
}

struct LockScreenView: View {
    let shown: Shown

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                PhaseMark(phase: shown.phase)
                    .font(.title3)
                VStack(alignment: .leading, spacing: 2) {
                    Text(shown.title)
                        .font(.headline)
                        .lineLimit(1)
                    if !shown.subtitle.isEmpty {
                        Text(shown.subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                PhaseClock(shown: shown)
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                    .frame(maxWidth: 80, alignment: .trailing)
            }
            Text(shown.body)
                .font(.subheadline)
                .lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
            Answers(shown: shown)
        }
    }
}

/// Allow and Deny, while the session waits on a permission request.
struct Answers: View {
    let shown: Shown

    var body: some View {
        if shown.phase == .permission, let ask = shown.askID {
            HStack(spacing: 10) {
                Button(intent: DenyIntent(serverID: shown.serverID, session: shown.session, askID: ask)) {
                    Text("Deny").frame(maxWidth: .infinity)
                }
                .tint(.red)
                if shown.readable {
                    Button(intent: AllowIntent(serverID: shown.serverID, session: shown.session, askID: ask)) {
                        Text("Allow").fontWeight(.semibold).frame(maxWidth: .infinity)
                    }
                    .tint(.green)
                }
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.capsule)
        }
    }
}

/// The phase as one mark, in the app's colours: yellow while working, purple
/// while waiting on you, green when done, red when failed.
struct PhaseMark: View {
    let phase: SessionPhase

    var body: some View {
        Image(systemName: phase.symbol)
            .foregroundStyle(phase.color)
            .accessibilityLabel(phase.label)
    }
}

/// How long it has been working or waiting, counting; once it is over, what
/// it came to.
struct PhaseClock: View {
    let shown: Shown

    var body: some View {
        switch shown.phase {
        case .working, .permission, .input:
            if let since = shown.since {
                Text(since, style: .timer)
            } else {
                Text(shown.phase.label)
            }
        case .done, .failed, .ended:
            Text(shown.phase.label)
        }
    }
}

extension SessionPhase {
    var symbol: String {
        switch self {
        case .working: "ellipsis.circle.fill"
        case .permission: "hand.raised.fill"
        case .input: "questionmark.circle.fill"
        case .done: "checkmark.circle.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .ended: "stop.circle.fill"
        }
    }

    var color: Color {
        switch self {
        case .working: .yellow
        case .permission, .input: .purple
        case .done: .green
        case .failed: .red
        case .ended: .gray
        }
    }

    var label: String {
        switch self {
        case .working: "Working"
        case .permission: "Permission"
        case .input: "Needs input"
        case .done: "Done"
        case .failed: "Failed"
        case .ended: "Ended"
        }
    }

    /// What it says when the text can't be read.
    var generic: String {
        switch self {
        case .working: "Claude is working"
        case .permission: "Claude needs your permission"
        case .input: "Claude needs your input"
        case .done: "Claude finished and is waiting for you"
        case .failed: "The session failed"
        case .ended: "The session ended"
        }
    }
}

#Preview("Permission", as: .content, using: SessionActivityAttributes(serverID: "h_aaaabbbbccccdddd", linkID: "l_x",
                                                                     session: "fix-tests")) {
    SessionActivityWidget()
} contentStates: {
    SessionActivityState(phase: .permission, at: .now.addingTimeInterval(-42), askID: "toolu_1",
                         title: "Fix the flaky login test", subtitle: "scope-creep", body: "Bash: npm test")
    SessionActivityState(phase: .working, at: .now.addingTimeInterval(-300), title: "Fix the flaky login test",
                         subtitle: "scope-creep", body: "Claude is working")
    SessionActivityState(phase: .done, title: "Fix the flaky login test", subtitle: "scope-creep",
                         body: "Claude finished and is waiting for you")
}
