// Class names for the components rebuilt in T3 Code's shape: T3 Code's `cn`,
// `cva`'s joining with `tailwind-merge`'s resolution of conflicting utilities,
// told about the two text sizes `tailwind.css` adds so it does not take
// `text-2xs` for a colour.

import { type CxOptions, cx } from "class-variance-authority";
import { extendTailwindMerge } from "tailwind-merge";

const twMerge = extendTailwindMerge({
  extend: { theme: { text: ["2xs", "3xs"] } },
});

export function cn(...inputs: CxOptions) {
  return twMerge(cx(inputs));
}
