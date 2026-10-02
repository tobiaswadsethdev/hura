// A toggle that looks like what it is: on is filled and ticked, off is an
// outline. A checkbox and a label beside it was two controls for one choice,
// and a column of them was most of the height of the create form.

import { Chosen } from "./icons";

export /// A toggle that looks like what it is: on is filled, off is an outline. A
/// checkbox and a label beside it was two controls for one choice.
function Pill({
  on,
  title,
  onClick,
  children,
}: {
  on: boolean;
  title?: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button type="button" className={`pill${on ? " on" : ""}`} aria-pressed={on} title={title} onClick={onClick}>
      {on && <Chosen />}
      {children}
    </button>
  );
}
