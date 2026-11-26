// A project's mark: one or two letters on a tile of its own colour, the same
// for the same name every time. Ported from T3 Code (github.com/pingdotgg/t3code:
// apps/web/src/components/ProjectMonogram.tsx, apps/web/src/projectIdentity.ts,
// apps/web/src/projectIconColors.ts and packages/shared/src/projectMonogram.ts),
// MIT licensed: see ../ui/LICENSE-T3CODE.

import { cn } from "../lib/cn";

// Eighteen hues, in T3 Code's order, so a name hashes to the colour it would
// have there. Written out whole so Tailwind finds every class.
const COLORS = [
  "text-gray-600 dark:text-gray-400",
  "text-red-600 dark:text-red-400",
  "text-orange-600 dark:text-orange-400",
  "text-amber-600 dark:text-amber-400",
  "text-yellow-600 dark:text-yellow-400",
  "text-lime-600 dark:text-lime-400",
  "text-green-600 dark:text-green-400",
  "text-emerald-600 dark:text-emerald-400",
  "text-teal-600 dark:text-teal-400",
  "text-cyan-600 dark:text-cyan-400",
  "text-sky-600 dark:text-sky-400",
  "text-blue-600 dark:text-blue-400",
  "text-indigo-600 dark:text-indigo-400",
  "text-violet-600 dark:text-violet-400",
  "text-purple-600 dark:text-purple-400",
  "text-fuchsia-600 dark:text-fuchsia-400",
  "text-pink-600 dark:text-pink-400",
  "text-rose-600 dark:text-rose-400",
];

function colorOf(name: string): string {
  const seed = name.normalize("NFKC").trim().toLocaleLowerCase("en-US") || "project";
  let index = 0;
  for (const glyph of seed) index = (index * 31 + (glyph.codePointAt(0) ?? 0)) % COLORS.length;
  return COLORS[index];
}

// One ideograph or syllable block already reads as a compact mark; two look bulky.
const SINGLE_GLYPH_SCRIPT = /^[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}\p{scx=Hangul}]/u;

function lettersOf(name: string): string {
  const words = name.normalize("NFKC").trim().match(/[\p{L}\p{N}]+/gu) ?? [];
  const firstWord = words[0];
  if (!firstWord) return "PR";
  const glyphs = Array.from(firstWord);
  const first = glyphs[0] ?? "P";
  if (SINGLE_GLYPH_SCRIPT.test(first)) return first;
  const second =
    glyphs.slice(1).find((glyph) => /\p{N}/u.test(glyph)) ??
    (words.length > 1 ? Array.from(words.at(-1) ?? "")[0] : glyphs.at(-1)) ??
    first;
  return Array.from(`${first}${second}`.toUpperCase()).slice(0, 2).join("");
}

export function ProjectMonogram({ name, className }: { name: string; className?: string }) {
  const text = lettersOf(name);
  return (
    <span aria-hidden="true" className={cn("inline-flex size-4 shrink-0 items-center justify-center", className)}>
      <svg
        viewBox="0 0 16 16"
        className={cn("size-full overflow-hidden rounded-[25%] font-mono select-none", colorOf(name))}
        style={{ backgroundColor: "color-mix(in srgb, currentColor 14%, transparent)" }}
      >
        <text
          x="8"
          y="10.8"
          textAnchor="middle"
          fill="currentColor"
          fontSize="8.25"
          fontWeight="700"
          textLength={Array.from(text).length === 1 ? 6 : 12}
          lengthAdjust="spacingAndGlyphs"
          textRendering="geometricPrecision"
        >
          {text}
        </text>
      </svg>
    </span>
  );
}
