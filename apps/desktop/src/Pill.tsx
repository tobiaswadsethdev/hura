// A toggle that looks like what it is: on is filled and ticked, off is an
// outline with a plus where the tick goes. A checkbox and a label beside it was
// two controls for one choice, and a column of them was most of the height of
// the create form.
//
// The plus is there so the tick has a place to go: with nothing in it when
// off, turning a pill on widened it by the tick and pushed along every pill
// after it in the row.

import { Chosen, Plus } from "./icons";

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
      {on ? <Chosen /> : <Plus className="pill-add" />}
      {children}
    </button>
  );
}
