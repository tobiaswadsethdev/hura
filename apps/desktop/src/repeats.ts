// Worktrees that come round again: the choices the create form offers, the
// rule each becomes (`hura_core::queue::Repeat`), and the rule said back in
// words for the sidebar.
//
// A time of day in a rule is on the server's clock, as T3 Code's are, since
// the server is what starts it and it keeps going with nothing connected.
// What you pick is on this machine's clock, so it is moved by the difference
// between the two when they differ (a server in another time zone), and moved
// back to show it. The server says its offset in its queue (`utc_offset`);
// one that does not say cannot repeat a worktree at all.

import type { Repeat } from "./gen/Repeat";

export type Cadence = "daily" | "weekdays" | `weekly-${number}` | "hourly";

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// The create form's choices, in the order it lists them.
export const CADENCES: { value: Cadence; label: string }[] = [
  { value: "daily", label: "Every Day" },
  { value: "weekdays", label: "Every Weekday" },
  ...DAYS.map((day, i) => ({ value: `weekly-${i}` as Cadence, label: `Every ${day}` })),
  { value: "hourly", label: "Every Hour" },
];

/// This machine's offset from UTC now, in seconds, as the server says its own.
const localOffset = () => -new Date().getTimezoneOffset() * 60;

/// The server's offset, from the last queue the window read. See `App`.
let serverOffset: number | null = null;
export function noteServerOffset(offset: number | null | undefined) {
  serverOffset = offset ?? null;
}

/// Whether the server can repeat at all: one that says its clock.
export function serverCanRepeat(): boolean {
  return serverOffset !== null;
}

/// How far the server's clock is ahead of this one, in minutes.
const ahead = () => Math.round(((serverOffset ?? localOffset()) - localOffset()) / 60);

/// A time of day and its days, moved by `minutes`: across midnight the days
/// move with it.
function shift(hour: number, minute: number, weekdays: number[], minutes: number) {
  const total = hour * 60 + minute + minutes;
  const days = Math.floor(total / 1440);
  const within = ((total % 1440) + 1440) % 1440;
  return {
    hour: Math.floor(within / 60),
    minute: within % 60,
    weekdays: weekdays.map((d) => (((d + days) % 7) + 7) % 7).sort((a, b) => a - b),
  };
}

/// The rule for a choice at `time` ("09:00") on this machine's clock.
export function repeatOf(cadence: Cadence, time: string): Repeat {
  if (cadence === "hourly") return { type: "interval", minutes: 60 };
  const [h, m] = time.split(":").map(Number);
  const weekdays =
    cadence === "daily" ? [] : cadence === "weekdays" ? [1, 2, 3, 4, 5] : [Number(cadence.slice("weekly-".length))];
  return { type: "fixed_time", ...shift(h || 0, m || 0, weekdays, ahead()) };
}

/// Whether the server keeps another clock than this machine, which a time of
/// day picked here has to be read against.
export function otherClock(): boolean {
  return ahead() !== 0;
}

/// A rule in words, on this machine's clock: "Every weekday at 09:00".
export function describeRepeat(rule: Repeat): string {
  if (rule.type === "interval") {
    return rule.minutes % 60 === 0
      ? rule.minutes === 60
        ? "Every hour"
        : `Every ${rule.minutes / 60} hours`
      : `Every ${rule.minutes} minutes`;
  }
  const here = shift(rule.hour, rule.minute, rule.weekdays, -ahead());
  const time = new Date(2000, 0, 1, here.hour, here.minute).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  const days = here.weekdays;
  const which =
    days.length === 0 || days.length === 7
      ? "Every day"
      : days.join() === "1,2,3,4,5"
        ? "Every weekday"
        : days.join() === "0,6"
          ? "Every weekend"
          : `Every ${days.map((d) => DAYS[d]).join(", ")}`;
  return `${which} at ${time}`;
}
