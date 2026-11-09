import HuraKit
import SwiftUI

/// A state in a capsule, coloured by what it asks of you -- in the desktop's
/// hues, which are the system's: yellow for an agent working, purple for one
/// waiting on you, green for ready, red for failed, and grey for the rest.
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
        case "needs_input": .purple
        case "ready": .green
        case "failed", "dead": .red
        default: .secondary
        }
    }
}

/// What an agent is doing, as one mark: the desktop's sidebar column
/// (`StateDot` in `icons.tsx`). The turning dots while it works or while the
/// sandbox is still being made; a question mark, in purple, while it waits on
/// you -- a shape, so the row is findable in a list without reading it; an
/// alert when it failed; and a plain dot otherwise, green when ready.
struct StateMark: View {
    /// The session's state, with the agent's own in place of `ready` when there
    /// is a report: see `liveState`.
    let state: String

    var body: some View {
        Group {
            switch state {
            case "running": WorkingDots(size: 16).foregroundStyle(Color.working)
            case "creating", "seeding": WorkingDots(size: 16).foregroundStyle(.secondary)
            case "waiting": Image(glyph: .waiting).foregroundStyle(.purple)
            case "failed", "dead": Image(glyph: .failed).foregroundStyle(.red)
            default:
                Circle()
                    .fill(state == "ready" ? Color.green : Color.secondary.opacity(0.6))
                    .frame(width: 8, height: 8)
            }
        }
        .font(.subheadline)
        .frame(width: 18, height: 18)
        .accessibilityLabel(state == "waiting" ? "waiting for input" : state)
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
