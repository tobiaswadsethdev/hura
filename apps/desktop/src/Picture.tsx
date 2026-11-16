// An image that opens on its own, over the window, when clicked: a screenshot
// fitted to the width of a message is too small to read, as on the phone.

import { useEffect, useRef, useState, type ImgHTMLAttributes } from "react";
import { createPortal } from "react-dom";
import { Close } from "./icons";

/// An `<img>` a click enlarges. Takes what an `<img>` takes.
export function Zoomable({ className, ...img }: ImgHTMLAttributes<HTMLImageElement>) {
  const [open, setOpen] = useState(false);
  const show = (e: React.SyntheticEvent) => {
    // Not also the card or row it is in, which may open on a click of its own.
    e.stopPropagation();
    e.preventDefault();
    setOpen(true);
  };
  return (
    <>
      <img
        {...img}
        className={className ? `${className} zoomable` : "zoomable"}
        role="button"
        tabIndex={0}
        onClick={show}
        onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && show(e)}
      />
      {open && img.src && <Enlarged src={img.src} alt={img.alt} onClose={() => setOpen(false)} />}
    </>
  );
}

/// The image as large as the window has room for, and no larger than it is.
/// A click anywhere or Escape puts it away.
function Enlarged({ src, alt, onClose }: { src: string; alt?: string; onClose: () => void }) {
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => close.current?.focus(), []);

  useEffect(() => {
    // Capturing, so it is answered before the screen under it sees Escape.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  // A portal's events still bubble through React to whatever the image is in.
  const stop = (e: React.SyntheticEvent) => e.stopPropagation();
  return createPortal(
    <div
      className="picture-scrim"
      role="dialog"
      aria-modal="true"
      aria-label={alt || "Image"}
      onMouseDown={stop}
      onClick={(e) => {
        stop(e);
        onClose();
      }}
    >
      <img src={src} alt={alt ?? ""} />
      <button ref={close} className="picture-close" aria-label="Close" title="Close (Esc)">
        <Close />
      </button>
    </div>,
    document.body,
  );
}
