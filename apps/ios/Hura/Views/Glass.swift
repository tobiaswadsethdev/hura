import SwiftUI

/// Liquid Glass where the system has it, and a material in its shape where it
/// doesn't: the app still runs on iOS 17, whose controls are flat.
extension View {
    /// Glass behind the view, in `shape`. An interactive one answers a touch
    /// as the system's own glass buttons do.
    func glass(in shape: some Shape = Capsule(), interactive: Bool = false, tint: Color? = nil) -> some View {
        modifier(GlassBackground(shape: shape, interactive: interactive, tint: tint))
    }

    /// Glass pieces drawn side by side, which melt into one another as they
    /// near: the system's container for them, when there is one.
    @ViewBuilder
    func glassGroup(spacing: CGFloat? = nil) -> some View {
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: spacing) { self }
        } else {
            self
        }
    }
}

private struct GlassBackground<S: Shape>: ViewModifier {
    let shape: S
    let interactive: Bool
    let tint: Color?

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.glassEffect(Glass.regular.tint(tint).interactive(interactive), in: shape)
        } else {
            content
                .background((tint ?? .clear).opacity(0.5), in: shape)
                .background(.ultraThinMaterial, in: shape)
                .overlay(shape.stroke(Color.white.opacity(0.1), lineWidth: 0.5))
        }
    }
}
