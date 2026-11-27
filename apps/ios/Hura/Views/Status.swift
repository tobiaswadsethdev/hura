import HuraKit
import SwiftUI

/// A state in a capsule, coloured by what it asks of you -- in the desktop's
/// hues, which are T3 Code's: blue for an agent working, amber for one waiting
/// on you, emerald for ready, red for failed, and grey for the rest.
struct StatusBadge: View {
    let status: String

    var body: some View {
        Text(label)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.18), in: Capsule())
            .foregroundStyle(color)
    }

    private var label: String {
        switch status {
        case "needs_input": "needs input"
        default: status
        }
    }

    private var color: Color {
        switch status {
        case "working": Color.working
        case "needs_input": .attention
        case "ready": .ok
        case "failed", "dead": .bad
        default: .secondary
        }
    }
}

/// What an agent is doing, as one mark: the desktop's sidebar column
/// (`StateDot` in `icons.tsx`). A still dashed circle while it works or while the
/// sandbox is still being made; a question mark, in amber, while it waits on
/// you -- a shape, so the row is findable in a list without reading it; an
/// alert when it failed; and a plain dot otherwise, emerald when ready.
struct StateMark: View {
    /// The session's state, with the agent's own in place of `ready` when there
    /// is a report: see `liveState`.
    let state: String

    var body: some View {
        Group {
            switch state {
            // T3 Code's Working, a dashed circle standing still, as the
            // desktop's list draws it: a whole list of them turning read as
            // a page still loading.
            case "running", "creating", "seeding": Image(glyph: .pending).foregroundStyle(Color.working)
            case "waiting": Image(glyph: .waiting).foregroundStyle(Color.attention)
            case "failed", "dead": Image(glyph: .failed).foregroundStyle(Color.bad)
            default:
                Circle()
                    .fill(state == "ready" ? Color.ok : Color.secondary.opacity(0.6))
                    .frame(width: 8, height: 8)
            }
        }
        .font(.subheadline)
        .frame(width: 18, height: 18)
        .accessibilityLabel(state == "waiting" ? "waiting for input" : state)
    }
}

/// A state as the desktop's sidebar says it (`statusOf` in `Tree.tsx`), or nil
/// for one with nothing to say (ready, or idle), whose row shows its age
/// instead.
func stateWord(_ state: String) -> (label: String, color: Color)? {
    switch state {
    case "creating", "seeding": ("Setting up", .working)
    case "running": ("Working", .working)
    case "waiting": ("Needs you", .attention)
    case "failed", "dead": ("Failed", .bad)
    default: nil
    }
}

/// The agent's state where it has reported one, the session record's
/// otherwise -- the record is what says `creating`, `failed` or `dead`, and
/// the only thing a session never polled has.
func liveState(_ session: Session, _ report: AgentReport?) -> String {
    if let report, session.state == "ready" { return report.state }
    return session.state
}

/// How long a session has existed, as the desktop's sidebar says it: `4h`.
func age(_ createdAt: Int64) -> String {
    let secs = max(0, Int64(Date().timeIntervalSince1970) - createdAt)
    if secs < 60 { return "\(secs)s" }
    if secs < 3600 { return "\(secs / 60)m" }
    if secs < 86400 { return "\(secs / 3600)h" }
    return "\(secs / 86400)d"
}
