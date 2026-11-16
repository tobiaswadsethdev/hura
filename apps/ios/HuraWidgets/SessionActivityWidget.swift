import ActivityKit
import AppIntents
import HuraKit
import SwiftUI
import WidgetKit

/// A session on the Lock Screen and in the Dynamic Island: what it is doing,
/// for how long the turn has run, and, while it waits on a permission, Allow
/// and Deny.
///
/// The relay sends the text sealed, as it does a notification's, so it is
/// opened here, each time the activity is drawn, with the key the app keeps
/// for the server in the Keychain group this extension shares. When it can't
/// be, the activity says why: there is nowhere else a failure here shows.
struct SessionActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: SessionActivityAttributes.self) { context in
            LockScreenView(shown: Shown(context))
                .activityBackgroundTint(Color.black.opacity(0.82))
                .activitySystemActionForegroundColor(.white)
                .widgetURL(SessionActivity.url(serverID: context.attributes.serverID,
                                               session: context.attributes.session))
        } dynamicIsland: { context in
            let shown = Shown(context)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Badged(shown: shown, size: 40)
                        .padding(.leading, 6)
                        .padding(.top, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    StatusPill(shown: shown)
                        .padding(.trailing, 6)
                        .padding(.top, 4)
                }
                DynamicIslandExpandedRegion(.center) {
                    Heading(shown: shown)
                        .padding(.top, 4)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 10) {
                        Said(shown: shown, lines: 2)
                        Answers(shown: shown)
                    }
                    .padding(.horizontal, 6)
                    .padding(.top, 4)
                }
            } compactLeading: {
                PhaseMark(phase: shown.phase, size: 18)
            } compactTrailing: {
                Clock(shown: shown)
                    .font(.caption.weight(.semibold).monospacedDigit())
                    .foregroundStyle(shown.phase.color)
                    .frame(maxWidth: 46)
            } minimal: {
                PhaseMark(phase: shown.phase, size: 16)
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
    var turnStart: Date?
    var askID: String?
    var title: String
    var subtitle: String
    var body: String
    /// Why the text couldn't be read, when it couldn't.
    var problem: String?
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
        turnStart = state.turnStart
        askID = state.askID
        let opened = state.opened(serverID: serverID, session: session, keys: keys)
        let text = try? opened.get()
        readable = text != nil
        if case let .failure(why) = opened, why != .none { problem = why.why }
        title = text?.title ?? session
        subtitle = PushCrypto.subtitle(text?.subtitle, serverName: (try? keys.name(serverID: serverID)) ?? nil)
        body = text?.body ?? phase.generic
    }
}

struct LockScreenView: View {
    let shown: Shown

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 12) {
                Badged(shown: shown, size: 44)
                Heading(shown: shown)
                Spacer(minLength: 6)
                StatusPill(shown: shown)
            }
            Said(shown: shown, lines: 3)
            Answers(shown: shown)
        }
        .padding(16)
        .background {
            // The phase's colour, rising from the corner the mark is in, so
            // what the session needs reads before any of the words do.
            LinearGradient(colors: [shown.phase.color.opacity(0.30), shown.phase.color.opacity(0.06), .clear],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
        }
    }
}

/// The session's title and, under it, where it is: this phone's name for the
/// server, and the project.
struct Heading: View {
    let shown: Shown

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(shown.title)
                .font(.headline)
                .foregroundStyle(.white)
                .lineLimit(1)
            if !shown.subtitle.isEmpty {
                Text(shown.subtitle)
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.6))
                    .lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// What the session says: the request, the question, or how it is getting on.
struct Said: View {
    let shown: Shown
    let lines: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(shown.body)
                .font(shown.phase.isWaiting ? .subheadline.weight(.medium) : .subheadline)
                .foregroundStyle(.white.opacity(shown.phase.isWaiting ? 0.95 : 0.8))
                .lineLimit(lines)
            if let problem = shown.problem {
                Text(problem)
                    .font(.caption2)
                    .foregroundStyle(.orange)
                    .lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// hura's icon, with the phase in a badge on its corner: whose activity it is,
/// and what it is doing, in one mark.
struct Badged: View {
    let shown: Shown
    let size: CGFloat

    var body: some View {
        Image("Logo")
            .resizable()
            .frame(width: size, height: size)
            .clipShape(RoundedRectangle(cornerRadius: size * 0.225, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                .strokeBorder(.white.opacity(0.12), lineWidth: 0.5))
            .overlay(alignment: .bottomTrailing) {
                PhaseMark(phase: shown.phase, size: size * 0.32)
                    .padding(size * 0.08)
                    .background(Circle().fill(.black))
                    .overlay(Circle().strokeBorder(shown.phase.color.opacity(0.5), lineWidth: 1))
                    .offset(x: size * 0.16, y: size * 0.16)
            }
            .padding([.trailing, .bottom], size * 0.12)
    }
}

/// The phase in words, in its colour, over the turn's clock.
struct StatusPill: View {
    let shown: Shown

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            Text(shown.phase.label)
                .font(.caption.weight(.semibold))
                .foregroundStyle(shown.phase.color)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(shown.phase.color.opacity(0.18), in: Capsule())
            Clock(shown: shown)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.white.opacity(0.7))
                .multilineTextAlignment(.trailing)
                .frame(maxWidth: 72, alignment: .trailing)
        }
        .fixedSize(horizontal: true, vertical: false)
    }
}

/// How long the turn has run, counting while it does. Once it is over there
/// is nothing to count.
struct Clock: View {
    let shown: Shown

    var body: some View {
        if !shown.phase.isOver, let start = shown.turnStart {
            Text(timerInterval: start...Date.distantFuture, countsDown: false)
        } else {
            Text(shown.phase.isOver ? "" : shown.phase.label)
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
                    Label("Deny", systemImage: "xmark")
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity)
                }
                .tint(.red)
                if shown.readable {
                    Button(intent: AllowIntent(serverID: shown.serverID, session: shown.session, askID: ask)) {
                        Label("Allow", systemImage: "checkmark")
                            .font(.subheadline.weight(.semibold))
                            .frame(maxWidth: .infinity)
                    }
                    .tint(.green)
                }
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.capsule)
        }
    }
}

/// The phase as one mark, as the app draws it: its turning dots, still, while
/// Claude works, and its own icons otherwise.
struct PhaseMark: View {
    let phase: SessionPhase
    let size: CGFloat

    var body: some View {
        Group {
            if phase == .working {
                WorkingDots(size: size)
            } else {
                Image(phase.glyph)
                    .resizable()
                    .scaledToFit()
                    .fontWeight(.semibold)
                    .frame(width: size, height: size)
            }
        }
        .foregroundStyle(phase.color)
        .accessibilityLabel(phase.label)
    }
}

extension SessionPhase {
    /// The app's icon for it (`Glyph`), copied into this extension's assets.
    var glyph: String {
        switch self {
        case .working, .input: "lucide.message-circle-question-mark"
        case .permission: "lucide.hand"
        case .done: "lucide.circle-check"
        case .failed: "lucide.circle-alert"
        case .ended: "lucide.circle-off"
        }
    }

    /// The app's colours: yellow while working, purple while waiting on you,
    /// green when done, red when failed.
    var color: Color {
        switch self {
        case .working: .yellow
        case .permission, .input: Color(red: 0.75, green: 0.45, blue: 1.0)
        case .done: .green
        case .failed: .red
        case .ended: .gray
        }
    }

    var label: String {
        switch self {
        case .working: "Working"
        case .permission: "Permission"
        case .input: "Needs you"
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
    SessionActivityState(phase: .permission, at: .now.addingTimeInterval(-42), since: .now.addingTimeInterval(-300),
                         askID: "toolu_1", title: "Fix the flaky login test", subtitle: "scope-creep",
                         body: "Bash: npm test")
    SessionActivityState(phase: .working, at: .now.addingTimeInterval(-300), title: "Fix the flaky login test",
                         subtitle: "scope-creep", body: "Claude is working")
    SessionActivityState(phase: .done, title: "Fix the flaky login test", subtitle: "scope-creep",
                         body: "Claude finished and is waiting for you")
}
