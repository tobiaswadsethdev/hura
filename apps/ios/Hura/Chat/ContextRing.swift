import HuraKit
import SwiftUI

/// How full this conversation's context is, as a ring in the composer, and
/// behind it the account's limits -- the desktop's, in the same place.
///
/// Hollow when the conversation is new, closing as it fills, yellow then red
/// near the top: whether the next message is the one that compacts is asked
/// while it is written. A tap shows the figures. The context is the
/// conversation's, from its state; the limits are the account's, asked for
/// when the ring is opened rather than polled for all the while it is not.
struct ContextRing: View {
    let percentage: Double?
    let size: Int64?
    let usage: (() async -> Usage?)?
    @State private var open = false

    private var pct: Double { min(100, max(0, percentage ?? 0)) }

    var body: some View {
        Button { open = true } label: {
            ZStack {
                Circle().stroke(Color.secondary.opacity(0.35), lineWidth: 2)
                Circle()
                    .trim(from: 0, to: pct / 100)
                    .stroke(fullness(pct, plain: .secondary), style: StrokeStyle(lineWidth: 2, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.easeOut(duration: 0.3), value: pct)
            }
            .frame(width: 15, height: 15)
            .padding(4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(percentage.map { "Context \(Int($0.rounded()))% full" } ?? "Context: not measured yet")
        .popover(isPresented: $open) {
            UsageCard(percentage: percentage, size: size, usage: usage)
                .presentationCompactAdaptation(.popover)
        }
    }
}

/// Yellow from 70, red from 90: the desktop's steps.
private func fullness(_ pct: Double, plain: Color) -> Color {
    pct >= 90 ? .red : pct >= 70 ? .yellow : plain
}

private struct UsageCard: View {
    let percentage: Double?
    let size: Int64?
    let usage: (() async -> Usage?)?
    @State private var windows: [Usage.Window]?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("This conversation").font(.caption).foregroundStyle(.secondary)
            if let percentage {
                Meter(label: "Context", pct: percentage, after: size.map { " of \(tokens($0))" })
            } else {
                Text("Measured once the agent has answered.").font(.footnote).foregroundStyle(.secondary)
            }
            Text("This account").font(.caption).foregroundStyle(.secondary).padding(.top, 6)
            if let windows, !windows.isEmpty {
                ForEach(windows, id: \.label) { w in
                    Meter(label: w.label, pct: w.usedPercentage, after: nil, note: w.resetsAt.map(resets))
                }
            } else if windows == nil {
                ProgressView().controlSize(.small)
            } else {
                Text("No limits reported yet.").font(.footnote).foregroundStyle(.secondary)
            }
        }
        .padding(14)
        .frame(width: 280, alignment: .leading)
        .task { windows = await usage?()?.windows ?? [] }
    }

    /// `200k`, `1M`: a window size as people say it.
    private func tokens(_ n: Int64) -> String {
        n >= 1_000_000 ? "\((Double(n) / 1_000_000).formatted(.number.precision(.fractionLength(0...1))))M" : "\(n / 1000)k"
    }

    private func resets(_ at: Int64) -> String {
        "Resets \(Date(timeIntervalSince1970: TimeInterval(at)).formatted(.relative(presentation: .named)))"
    }
}

/// One percentage as a bar, its label before it and the figure after.
private struct Meter: View {
    let label: String
    let pct: Double
    let after: String?
    var note: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(label).foregroundStyle(.secondary)
                Spacer()
                Text("\(Int(pct.rounded()))%") + Text(after ?? "").foregroundStyle(.secondary)
            }
            .font(.footnote.monospacedDigit())
            ProgressView(value: min(100, max(0, pct)), total: 100)
                .tint(fullness(pct, plain: .accentColor))
            if let note { Text(note).font(.caption2).foregroundStyle(.secondary) }
        }
    }
}
