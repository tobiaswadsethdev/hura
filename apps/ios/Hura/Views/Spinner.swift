import SwiftUI

/// Something under way, T3 Code's way, as the desktop now draws it (`Working`
/// in `apps/desktop/src/icons.tsx`): lucide's `LoaderCircle`, an arc of a
/// circle with a gap at the top right, turning once a second. It took the
/// place of the three turning dots everywhere something waits. Drawn in the
/// foreground style, so it takes the colour of where it is, and still for
/// someone who asked for less motion.
struct Spinner: View {
    var size: CGFloat = 14
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { context in
            let turn = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1)
            LoaderCircle()
                .stroke(style: StrokeStyle(lineWidth: max(1.25, size * 1.75 / 14), lineCap: .round))
                .rotationEffect(.degrees(reduceMotion ? 0 : turn * 360))
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// Lucide's `loader-circle`: `M21 12a9 9 0 1 1-6.219-8.56` in its 24-unit box,
/// a circle of radius 9 from three o'clock round to just short of one.
private struct LoaderCircle: Shape {
    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / 24
        let center = CGPoint(x: rect.midX, y: rect.midY)
        var p = Path()
        // From 0° (three o'clock) clockwise in screen terms to the end point
        // (14.781, 3.44), which is -72° from the centre.
        p.addArc(center: center, radius: 9 * scale, startAngle: .degrees(0), endAngle: .degrees(-72), clockwise: false)
        return p
    }
}

extension View {
    /// A running tool call's label, swept by a light, T3 Code's
    /// `live-tool-shine` as the desktop has it: the dim text with a band of
    /// the full one passing across it every 2.2 seconds, in place of a
    /// spinner beside it.
    func shine(_ on: Bool = true) -> some View {
        modifier(Shine(on: on))
    }
}

private struct Shine: ViewModifier {
    let on: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        if on && !reduceMotion {
            content
                .foregroundStyle(Color.dim)
                .overlay {
                    TimelineView(.animation) { context in
                        let t = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.2) / 2.2
                        GeometryReader { g in
                            let band: CGFloat = 72
                            LinearGradient(colors: [.clear, Color.ink, .clear], startPoint: .leading, endPoint: .trailing)
                                .frame(width: band)
                                .offset(x: -band + (g.size.width + band * 2) * t)
                        }
                    }
                    .mask(content)
                    .allowsHitTesting(false)
                }
        } else {
            content
        }
    }
}
