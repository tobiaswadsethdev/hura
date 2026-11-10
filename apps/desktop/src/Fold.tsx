// Things that open and close in place, moving rather than jumping.
//
// A fold's contents are only there while it is open: a transcript has
// hundreds of tool calls, and drawing every one's diff or output to keep it
// hidden would be most of the cost of drawing the conversation. So a fold
// that opens mounts its contents and grows from nothing to their height, and
// one that closes keeps them until it has shrunk away, then lets them go.
//
// The Web Animations API rather than a CSS transition, because neither end of
// the move is a height CSS knows: the open one is whatever the contents turn
// out to be, and a fold turned round halfway starts from wherever it had got
// to.

import { type ReactNode, useLayoutEffect, useRef, useState } from "react";

/// The curve every fold moves on: quick to start, settling gently, so a click
/// answers at once and the end does not land with a bump.
const EASE = "cubic-bezier(0.32, 0.72, 0, 1)";

/// A longer way takes a little longer, so a tall fold does not race and a
/// short one does not dawdle.
const duration = (distance: number) => Math.min(320, 180 + Math.abs(distance) / 5);

/// Asked for at the moment of each move, so turning the setting on in the
/// system takes effect without a reload.
const lessMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

/// Moves `el`'s height, or its `max-height`, from one number of pixels to
/// another, fading it between two opacities if `fade` is given. Nothing, and
/// no animation, when motion is turned down or there is nowhere to go.
///
/// Its contents are clipped while it moves and not after, so a focus ring or a
/// shadow at its edge is cut off only for the moment it takes.
export function slide(
  el: HTMLElement,
  property: "height" | "maxHeight",
  from: number,
  to: number,
  { fade, hold }: { fade?: [number, number]; hold?: boolean } = {},
): Animation | null {
  if (from === to || lessMotion()) {
    // A move turned round to here may have left its clip behind.
    el.style.overflow = "";
    return null;
  }
  el.style.overflow = "hidden";
  const animation = el.animate(
    [
      { [property]: `${from}px`, ...(fade && { opacity: fade[0] }) },
      { [property]: `${to}px`, ...(fade && { opacity: fade[1] }) },
    ],
    // Held at the end when the caller is about to take the element away, so
    // the frame between the end and React removing it is not drawn full size.
    { duration: duration(to - from), easing: EASE, fill: hold ? "forwards" : "none" },
  );
  animation.addEventListener("finish", () => {
    if (!hold) el.style.overflow = "";
  });
  return animation;
}

/// The nearest ancestor that scrolls, which is what has to move for something
/// to stay where it was on screen while the things above it change size.
export function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const y = getComputedStyle(p).overflowY;
    if (y === "auto" || y === "scroll") return p;
  }
  return null;
}

/// Contents that open and close by growing and shrinking, rather than
/// appearing and vanishing. Drawn as it is the first time, so a transcript
/// opened with some cards already open does not animate all of them.
export function Fold({ open, children }: { open: boolean; children: ReactNode }) {
  // Whether the contents are drawn: from the moment it opens until it has
  // finished closing.
  const [drawn, setDrawn] = useState(open);
  if (open && !drawn) setDrawn(true);
  const ref = useRef<HTMLDivElement>(null);
  // What `open` was the last time it moved. A ref rather than a first-render
  // flag, because Strict Mode runs a layout effect twice on mount, and the
  // second run would otherwise take itself for a change.
  const was = useRef(open);

  useLayoutEffect(() => {
    const el = ref.current;
    if (was.current === open || !el) return;
    was.current = open;
    // Turned round halfway: from where the last move had got to.
    const moving = el.getAnimations();
    const height = el.getBoundingClientRect().height;
    const opacity = Number(getComputedStyle(el).opacity);
    for (const a of moving) a.cancel();
    if (open) {
      const to = el.getBoundingClientRect().height;
      slide(el, "height", moving.length ? height : 0, to, { fade: [moving.length ? opacity : 0, 1] });
    } else {
      const shrinking = slide(el, "height", height, 0, { fade: [opacity, 0], hold: true });
      if (shrinking) shrinking.addEventListener("finish", () => setDrawn(false));
      else setDrawn(false);
    }
  }, [open]);

  if (!drawn) return null;
  return (
    <div className="fold" ref={ref}>
      {children}
    </div>
  );
}
