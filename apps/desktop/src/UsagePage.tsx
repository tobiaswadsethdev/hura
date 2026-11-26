// What the worktrees have spent and what the account has left, T3 Code's
// usage card: the account's rate-limit windows, the totals, and each worktree
// by what it cost, from the same status polls the sidebar draws from. Drawn
// in the settings card (`settings/SettingsModal.tsx`) without its nav.

import type { Poll } from "./gen/Poll";
import type { Session } from "./gen/Session";
import type { Usage } from "./gen/Usage";
import { cn } from "./lib/cn";
import { SettingsPage, SettingsRow, SettingsSection } from "./settings/layout";

export function UsagePage({
  sessions,
  polls,
  onOpen,
}: {
  sessions: Session[];
  polls: Record<string, Poll>;
  /// Go to a worktree, from its row.
  onOpen: (name: string) => void;
}) {
  const spent = sessions
    .map((s) => ({ s, usage: polls[s.name]?.usage ?? null }))
    .filter((x): x is { s: Session; usage: Usage } => x.usage !== null)
    .sort((a, b) => (b.usage.cost_usd ?? 0) - (a.usage.cost_usd ?? 0));
  const sum = (pick: (u: Usage) => number | null) => spent.reduce((n, x) => n + (pick(x.usage) ?? 0), 0);
  // Every worktree on one account reports the same windows; the one that
  // heard from the API last has the freshest.
  const account = spent.find((x) => x.usage.windows.length > 0) ?? null;

  return (
    <SettingsPage notice="From the worktrees on this server, as their agents last reported.">
      <SettingsSection title="This account">
        {account ? (
          account.usage.windows.map((w) => (
            <SettingsRow
              key={w.label}
              title={w.label}
              description={w.resets_at ? `Resets ${resetsIn(w.resets_at)}` : undefined}
              control={<Meter pct={w.used_percentage} />}
            />
          ))
        ) : (
          <SettingsRow
            title="No limits reported yet"
            description="An agent reports them once it has called the API."
          />
        )}
      </SettingsSection>

      <SettingsSection title="Spent">
        <SettingsRow title="Cost" control={<Figure>{dollars(sum((u) => u.cost_usd))}</Figure>} />
        <SettingsRow title="Time working" control={<Figure>{duration(sum((u) => u.duration_ms))}</Figure>} />
        <SettingsRow
          title="Lines"
          control={
            <Figure>
              <span className="text-success-foreground">+{sum((u) => u.lines_added).toLocaleString()}</span>{" "}
              <span className="text-destructive-foreground">−{sum((u) => u.lines_removed).toLocaleString()}</span>
            </Figure>
          }
        />
      </SettingsSection>

      <SettingsSection title="By worktree">
        {spent.length === 0 ? (
          <SettingsRow title="Nothing spent yet" description="A worktree's cost shows once its agent has worked." />
        ) : (
          spent.map(({ s, usage }) => (
            <SettingsRow
              key={s.name}
              className="cursor-pointer hover:bg-foreground/3"
              onClick={() => onOpen(s.name)}
              title={s.title ?? s.name}
              description={[
                usage.model,
                usage.duration_ms ? duration(usage.duration_ms) : null,
                usage.lines_added !== null || usage.lines_removed !== null
                  ? `+${usage.lines_added ?? 0} −${usage.lines_removed ?? 0}`
                  : null,
                usage.context_used_percentage !== null ? `${Math.round(usage.context_used_percentage)}% of context` : null,
              ]
                .filter(Boolean)
                .join(" · ")}
              control={<Figure>{dollars(usage.cost_usd ?? 0)}</Figure>}
            />
          ))
        )}
      </SettingsSection>
    </SettingsPage>
  );
}

function Figure({ children }: { children: React.ReactNode }) {
  return <span className="text-sm font-medium text-foreground tabular-nums">{children}</span>;
}

/// A window's use as a bar, amber past four fifths.
function Meter({ pct }: { pct: number }) {
  const used = Math.max(0, Math.min(100, pct));
  return (
    <span className="flex items-center gap-3">
      <span className="h-1.5 w-40 overflow-hidden rounded-full bg-foreground/10">
        <span
          className={cn("block h-full rounded-full", used >= 80 ? "bg-warning" : "bg-primary")}
          style={{ width: `${used}%` }}
        />
      </span>
      <span className="w-10 text-right text-sm font-medium text-foreground tabular-nums">{Math.round(used)}%</span>
    </span>
  );
}

const dollars = (n: number) => `$${n.toFixed(2)}`;

function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`;
}

/// When a window starts afresh, relative while it is soon.
function resetsIn(at: number): string {
  const minutes = Math.round((at * 1000 - Date.now()) / 60_000);
  if (minutes <= 0) return "now";
  if (minutes < 60) return `in ${minutes}m`;
  if (minutes < 24 * 60) return `in ${Math.round(minutes / 60)}h`;
  return new Date(at * 1000).toLocaleDateString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });
}
