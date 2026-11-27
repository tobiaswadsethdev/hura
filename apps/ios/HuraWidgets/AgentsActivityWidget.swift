import ActivityKit
import AppIntents
import HuraKit
import SwiftUI
import WidgetKit

/// Every session on the Lock Screen and in the Dynamic Island as one activity,
/// laid out as T3 Code's is (github.com/pingdotgg/t3code, MIT;
/// `apps/mobile/src/widgets/AgentActivity.tsx`): how many agents are working
/// and how many need you, over a row for each, the ones waiting on you first.
/// hura keeps one thing T3 Code's doesn't have: Allow and Deny, for the row
/// waiting on a permission.
///
/// Each row's text comes sealed, as a notification's does, and is opened here
/// with its own server's key.
struct AgentsActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentsActivityAttributes.self) { context in
            let shown = AgentsShown(context.state, stale: context.isStale)
            AgentsBanner(shown: shown)
                .activityBackgroundTint(AgentsShown.background)
                .activitySystemActionForegroundColor(.primary)
                .widgetURL(shown.link)
        } dynamicIsland: { context in
            let shown = AgentsShown(context.state, stale: context.isStale)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    HStack(spacing: 5) {
                        Mark(size: 15)
                        Text(shown.state.allDone || shown.stale ? shown.short : "\(shown.state.active)")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundStyle(shown.heroTint)
                    }
                    .padding(.leading, 4)
                    .padding(.vertical, 4)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 5) {
                        ForEach(shown.rows.prefix(3)) { AgentLine(row: $0) }
                        AgentAnswers(row: shown.asking)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                }
            } compactLeading: {
                Mark(size: 14)
            } compactTrailing: {
                // While agents only work, a glyph and no count, as T3 Code's:
                // words only for something to look at.
                if let word = shown.compactWord {
                    Text(word)
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(shown.heroTint)
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                } else {
                    PhaseSymbol(phase: shown.hero?.phase ?? .working, size: 14)
                        .padding(.trailing, 2)
                        .accessibilityLabel(shown.headline)
                }
            } minimal: {
                if let hero = shown.hero {
                    PhaseSymbol(phase: hero.phase, size: 13)
                } else {
                    Mark(size: 11)
                }
            }
            .widgetURL(shown.link)
            .keylineTint(shown.heroTint)
        }
    }
}

/// What the activity shows, worked out once per drawing: each row with its
/// text opened, and an activity past its stale date saying so on every row
/// that hasn't finished, so it can't claim agents still work.
struct AgentsShown {
    struct Row: Identifiable {
        var id: String
        var serverID: String
        var session: String
        var phase: SessionPhase
        /// "Out of date" in place of the phase's, when stale.
        var status: String
        var stale: Bool
        var askID: String?
        var title: String
        var project: String
        /// Whether its text was read: Allow is only an answer if the screen
        /// says what it allows.
        var readable: Bool
    }

    /// Clear, on iOS 26, lets the system's glass show through, as T3 Code's
    /// does; before it, the system's own surface.
    static var background: Color? {
        if #available(iOS 26, *) { return .clear }
        return nil
    }

    var state: AgentsActivityState
    var stale: Bool
    var rows: [Row]

    init(_ state: AgentsActivityState, stale: Bool, keys: PushKeyStore = Shown.keys) {
        self.state = state
        self.stale = stale
        rows = state.rows.map { r in
            let text = r.text(keys: keys)
            let server = (try? keys.name(serverID: r.serverID)) ?? nil
            let outOfDate = stale && !r.phase.isOver
            return Row(id: r.id, serverID: r.serverID, session: r.session, phase: r.phase,
                       status: outOfDate ? "Out of date" : r.phase.status, stale: outOfDate, askID: r.state.askID,
                       title: text?.title ?? r.session, project: text?.subtitle ?? server ?? "",
                       readable: text != nil)
        }
    }

    var hero: Row? {
        guard let h = state.hero else { return nil }
        return rows.first { $0.id == h.id }
    }

    /// The row waiting on a permission, which Allow and Deny answer.
    var asking: Row? { stale ? nil : rows.first { $0.phase == .permission && $0.askID != nil } }

    var heroTint: Color { hero.map { PhaseTint.of($0.phase, stale: $0.stale) } ?? PhaseTint.of(.working, stale: stale) }

    var headline: String { stale && !state.allDone ? "Agent status out of date" : state.headline }
    var short: String { stale && !state.allDone ? "Out of date" : state.short }
    var compactWord: String? { stale ? "Out of date" : state.compactWord }

    /// What a tap opens: the first waiting on you, or the first.
    var link: URL? {
        guard let row = rows.first(where: { $0.phase.isWaiting }) ?? rows.first else { return nil }
        return SessionActivity.url(serverID: row.serverID, session: row.session)
    }
}

/// The Lock Screen's banner: the mark and the counts over the rows.
struct AgentsBanner: View {
    let shown: AgentsShown

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                HStack(spacing: 0) {
                    Mark(size: 13)
                    Spacer(minLength: 0)
                }
                HStack(spacing: 6) {
                    Text(shown.headline)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(shown.state.allDone ? shown.heroTint : .primary)
                        .lineLimit(1)
                    if let attention = shown.state.attention, !shown.stale {
                        Text("·").font(.system(size: 13)).foregroundStyle(.secondary)
                        Text(attention)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(shown.heroTint)
                            .lineLimit(1)
                    }
                }
            }
            ForEach(shown.rows.prefix(5)) { AgentLine(row: $0) }
            AgentAnswers(row: shown.asking)
                .padding(.top, 4)
        }
        .padding(14)
    }
}

/// A session as one line: its title, its project, and its status in its tint.
struct AgentLine: View {
    let row: AgentsShown.Row

    var body: some View {
        HStack(alignment: .center, spacing: 7) {
            Text(row.title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.primary)
                .lineLimit(1)
            // Plain texts share what room there is, so both shorten together.
            Text(row.project)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(row.status)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(PhaseTint.of(row.phase, stale: row.stale))
                .layoutPriority(1)
        }
    }
}

/// Allow and Deny for the row waiting on a permission.
struct AgentAnswers: View {
    let row: AgentsShown.Row?

    var body: some View {
        if let row, let ask = row.askID {
            HStack(spacing: 10) {
                Button(intent: DenyIntent(serverID: row.serverID, session: row.session, askID: ask)) {
                    Label("Deny", systemImage: "xmark")
                        .font(.footnote.weight(.semibold))
                        .frame(maxWidth: .infinity)
                }
                .tint(PhaseTint.of(.failed, stale: false))
                if row.readable {
                    Button(intent: AllowIntent(serverID: row.serverID, session: row.session, askID: ask)) {
                        Label("Allow", systemImage: "checkmark")
                            .font(.footnote.weight(.semibold))
                            .frame(maxWidth: .infinity)
                    }
                    .tint(PhaseTint.of(.done, stale: false))
                }
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.capsule)
        }
    }
}

/// hura's mark, small, where T3 Code puts its own.
struct Mark: View {
    let size: CGFloat

    var body: some View {
        Image("Logo")
            .resizable()
            .frame(width: size, height: size)
            .clipShape(RoundedRectangle(cornerRadius: size * 0.225, style: .continuous))
            .accessibilityLabel("hura")
    }
}

/// A phase as T3 Code's activity marks it, in SF Symbols.
struct PhaseSymbol: View {
    let phase: SessionPhase
    let size: CGFloat

    var body: some View {
        Image(systemName: name)
            .resizable()
            .scaledToFit()
            .frame(width: size, height: size)
            .foregroundStyle(PhaseTint.of(phase, stale: false))
            .accessibilityLabel(phase.status)
    }

    private var name: String {
        switch phase {
        case .permission: "exclamationmark.circle.fill"
        case .input: "questionmark.circle.fill"
        case .failed: "xmark.octagon.fill"
        case .done: "checkmark.circle.fill"
        case .ended: "clock.arrow.circlepath"
        // T3 Code's Working, which the list's working rows match.
        case .working: "circle.dashed"
        }
    }
}

/// T3 Code's activity tints: the light ones where the activity is drawn light,
/// the pale ones on the dark material iPhone uses; plain secondary when out of
/// date; and the system's own colour where the activity is drawn in one
/// (StandBy at night, an accented Lock Screen).
enum PhaseTint {
    static func of(_ phase: SessionPhase, stale: Bool) -> Color {
        if stale { return .secondary }
        return Color(UIColor { traits in
            let light = traits.userInterfaceStyle == .light
            let hex: UInt32 = switch phase {
            case .permission: light ? 0xD97706 : 0xFCD34D
            case .input: light ? 0x4F46E5 : 0xA5B4FC
            case .failed: light ? 0xDC2626 : 0xFCA5A5
            case .done: light ? 0x059669 : 0x6EE7B7
            case .ended: light ? 0x71717B : 0xA1A1A1
            case .working: light ? 0x0284C7 : 0x7DD3FC
            }
            return UIColor(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                           blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
        })
    }
}

#Preview("Agents", as: .content, using: AgentsActivityAttributes()) {
    AgentsActivityWidget()
} contentStates: {
    AgentsActivityState(rows: [
        AgentRow(serverID: "h_a", session: "fix-tests",
                 state: SessionActivityState(phase: .permission, at: .now, askID: "toolu_1",
                                             title: "Fix the flaky login test", subtitle: "scope-creep", body: "Bash: npm test")),
        AgentRow(serverID: "h_a", session: "cache",
                 state: SessionActivityState(phase: .working, at: .now.addingTimeInterval(-60),
                                             title: "Pick a cache backend", subtitle: "scope-creep", body: "Claude is working")),
        AgentRow(serverID: "h_b", session: "docs",
                 state: SessionActivityState(phase: .done, at: .now.addingTimeInterval(-600),
                                             title: "Tidy the README", subtitle: "hura", body: "Done")),
    ])
}
