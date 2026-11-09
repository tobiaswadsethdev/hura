import SwiftUI

/// An agent working: three dots turning round each other, each shrinking as
/// it passes behind the others and swelling as it comes back round. The
/// desktop's mark (`apps/desktop/src/working.css`), from the same frames:
/// measured from Claude Desktop's loader, every dot's centre and scale, as a
/// fraction of the box, thirty times a loop. Drawn in the foreground style,
/// so it takes the colour of where it is.
struct WorkingDots: View {
    var size: CGFloat = 14
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static let loop = 1.8
    /// A dot's diameter at scale 1, as a fraction of the box.
    private static let dot = 0.249
    /// Per dot, (x, y, scale) at even steps through the loop.
    private static let frames: [[(Double, Double, Double)]] = [
        [
            (0.219, 0.338, 0.955), (0.196, 0.324, 0.984), (0.179, 0.316, 0.980), (0.171, 0.312, 0.953), (0.170, 0.310, 0.907),
            (0.179, 0.314, 0.831), (0.210, 0.333, 0.722), (0.267, 0.365, 0.585), (0.352, 0.413, 0.411), (0.459, 0.475, 0.234),
            (0.520, 0.511, 0.262), (0.585, 0.549, 0.478), (0.650, 0.585, 0.675), (0.702, 0.617, 0.827), (0.744, 0.642, 0.921),
            (0.779, 0.662, 0.961), (0.802, 0.675, 0.984), (0.821, 0.683, 0.988), (0.829, 0.688, 0.961), (0.831, 0.690, 0.911),
            (0.817, 0.681, 0.827), (0.777, 0.659, 0.709), (0.712, 0.625, 0.559), (0.630, 0.575, 0.364), (0.536, 0.523, 0.205),
            (0.475, 0.483, 0.211), (0.401, 0.443, 0.407), (0.335, 0.403, 0.602), (0.281, 0.372, 0.760), (0.240, 0.350, 0.878),
        ],
        [
            (0.819, 0.317, 0.797), (0.787, 0.335, 0.679), (0.726, 0.368, 0.533), (0.642, 0.418, 0.343), (0.540, 0.475, 0.201),
            (0.475, 0.513, 0.209), (0.411, 0.551, 0.421), (0.346, 0.586, 0.624), (0.293, 0.618, 0.783), (0.251, 0.642, 0.898),
            (0.219, 0.664, 0.947), (0.196, 0.678, 0.978), (0.182, 0.685, 0.982), (0.173, 0.689, 0.955), (0.171, 0.690, 0.909),
            (0.179, 0.684, 0.837), (0.210, 0.666, 0.726), (0.265, 0.633, 0.593), (0.350, 0.587, 0.411), (0.454, 0.527, 0.238),
            (0.524, 0.488, 0.266), (0.589, 0.451, 0.484), (0.649, 0.416, 0.677), (0.702, 0.385, 0.809), (0.746, 0.358, 0.904),
            (0.781, 0.339, 0.978), (0.805, 0.325, 1.000), (0.817, 0.316, 0.986), (0.827, 0.311, 0.957), (0.828, 0.310, 0.902),
        ],
        [
            (0.498, 0.472, 0.268), (0.501, 0.399, 0.484), (0.500, 0.330, 0.671), (0.500, 0.268, 0.813), (0.499, 0.217, 0.907),
            (0.502, 0.176, 0.967), (0.502, 0.148, 0.992), (0.500, 0.130, 0.984), (0.499, 0.120, 0.957), (0.499, 0.118, 0.900),
            (0.499, 0.137, 0.811), (0.499, 0.183, 0.697), (0.499, 0.254, 0.543), (0.501, 0.351, 0.362), (0.500, 0.458, 0.205),
            (0.501, 0.533, 0.207), (0.499, 0.615, 0.388), (0.501, 0.693, 0.598), (0.500, 0.753, 0.756), (0.500, 0.800, 0.878),
            (0.501, 0.832, 0.955), (0.501, 0.855, 0.984), (0.499, 0.873, 0.984), (0.499, 0.881, 0.959), (0.499, 0.882, 0.911),
            (0.503, 0.869, 0.837), (0.501, 0.833, 0.730), (0.501, 0.770, 0.604), (0.497, 0.672, 0.421), (0.496, 0.552, 0.240),
        ],
    ]

    var body: some View {
        // Still, the three dots where the loop begins.
        TimelineView(.animation(paused: reduceMotion)) { context in
            Canvas { gc, box in
                let t = reduceMotion ? 0 : context.date.timeIntervalSinceReferenceDate
                    .truncatingRemainder(dividingBy: Self.loop) / Self.loop
                for path in Self.frames {
                    let (x, y, s) = Self.at(path, t)
                    let d = Self.dot * s * box.width
                    gc.fill(Path(ellipseIn: CGRect(x: x * box.width - d / 2, y: y * box.height - d / 2,
                                                   width: d, height: d)), with: .foreground)
                }
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel("Working")
    }

    /// Where a dot is `t` of the way round, straight between frames, as CSS
    /// keyframes with linear timing are.
    private static func at(_ path: [(Double, Double, Double)], _ t: Double) -> (Double, Double, Double) {
        let p = t * Double(path.count)
        let i = Int(p) % path.count, f = p - p.rounded(.down)
        let a = path[i], b = path[(i + 1) % path.count]
        return (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
    }
}

extension Color {
    /// The desktop's `--working`, which is the system's yellow.
    static let working = Color.yellow
}
