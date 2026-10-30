import SwiftUI

/// A state in a capsule, coloured by what it asks of you.
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
        case "running", "idle": .green
        case "working", "starting": .blue
        case "needs_input": .orange
        case "failed": .red
        default: .secondary
        }
    }
}
