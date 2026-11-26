// The pieces a settings page is made of, after T3 Code's (`settingsLayout.tsx`,
// `SettingsGroup.tsx` and `WorkspacePageContainer.tsx`, MIT, see
// `ui/LICENSE-T3CODE`): a page at a reading width, sections with a title over
// a rounded group, and rows in the group, each a title, a sentence under it
// and its control at the right. Below 32rem a row's control goes under its
// words instead.

import type { ComponentPropsWithoutRef, ReactNode } from "react";

import { cn } from "../lib/cn";

/// A page's scroll and its column. `notice` is the sentence over everything,
/// where T3 Code says whose settings these are.
export function SettingsPage({
  children,
  notice,
  wide = false,
}: {
  children: ReactNode;
  notice?: ReactNode;
  wide?: boolean;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable_both-edges]" data-settings-page-scroll>
      <div
        className={cn(
          "mx-auto flex w-full flex-col gap-8 px-5 pt-6 pb-12 sm:px-6",
          wide ? "max-w-5xl" : "max-w-4xl",
        )}
      >
        {notice && <p className="px-3 text-xs text-muted-foreground sm:px-4">{notice}</p>}
        {children}
      </div>
    </div>
  );
}

export function SettingsSection({
  title,
  description,
  action,
  children,
  className,
  ...props
}: Omit<ComponentPropsWithoutRef<"section">, "title"> & {
  title: string;
  /// Under the title, for a section whose rows need saying why they are
  /// together.
  description?: ReactNode;
  /// At the title's right.
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section {...props} className={cn("space-y-2.5 rounded-xl transition-colors duration-500 data-found:bg-primary/8", className)}>
      <div className="flex min-h-7 flex-wrap items-start justify-between gap-x-4 gap-y-1 px-3 sm:px-4">
        <div className="min-w-0">
          <h2 className="flex min-h-7 items-center gap-2 text-base font-semibold text-foreground">{title}</h2>
          {description && <p className="max-w-2xl text-xs text-muted-foreground">{description}</p>}
        </div>
        {action && <div className="flex min-h-7 min-w-7 items-center justify-end">{action}</div>}
      </div>
      <SettingsGroup>{children}</SettingsGroup>
    </section>
  );
}

/// The rounded box a section's rows sit in, a hairline between each.
export function SettingsGroup({ className, ...props }: ComponentPropsWithoutRef<"div">) {
  return (
    <div
      data-slot="settings-group"
      {...props}
      className={cn(
        "relative overflow-visible rounded-xl border border-border/60 bg-card/40 text-foreground shadow-xs/5",
        "*:not-first:border-t *:not-first:border-border/50 [&>[data-slot=settings-row]]:rounded-none",
        className,
      )}
    />
  );
}

export function SettingsRow({
  title,
  description,
  status,
  control,
  children,
  className,
  ...props
}: Omit<ComponentPropsWithoutRef<"div">, "title"> & {
  title: ReactNode;
  description?: ReactNode;
  /// A line under the description: what the value now is, or went wrong.
  status?: ReactNode;
  control?: ReactNode;
  /// Under the row's own line, the width of the row: a list it owns.
  children?: ReactNode;
}) {
  return (
    <div
      {...props}
      data-slot="settings-row"
      className={cn(
        "group/settings-row @container/settings-row rounded-xl px-3 transition-colors duration-500 data-found:bg-primary/8 sm:px-4",
        children ? "pt-3 pb-3" : "py-3",
        className,
      )}
    >
      <div className="flex flex-col gap-3 @min-[32rem]/settings-row:grid @min-[32rem]/settings-row:grid-cols-[minmax(0,1fr)_auto] @min-[32rem]/settings-row:items-center @min-[32rem]/settings-row:gap-8">
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex min-h-5 items-center gap-1.5">
            <h3 className="text-sm font-medium text-foreground">{title}</h3>
          </div>
          {description && (
            <p className="max-w-2xl text-xs leading-normal text-pretty text-muted-foreground">{description}</p>
          )}
          {status && <div className="pt-0.5 text-xs text-muted-foreground">{status}</div>}
        </div>
        {control && (
          <div className="flex w-full min-w-0 shrink-0 items-center gap-2 @min-[32rem]/settings-row:w-auto @min-[32rem]/settings-row:justify-end">
            {control}
          </div>
        )}
      </div>
      {children}
    </div>
  );
}

/// A row of choices as T3 Code's segmented control draws them.
export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-lg bg-foreground/5 p-0.5 ring-1 ring-border">
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          className={cn(
            "h-6.5 cursor-pointer rounded-md px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground",
            o.value === value && "bg-background text-foreground shadow-xs/5 ring-1 ring-border",
          )}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
