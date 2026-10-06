// What a session is: the record, as it describes itself.
//
// Everything here is stored on the session rather than derived, which is the
// point of it -- the skills it was given, the MCP servers it was created with
// and the toolchains its image carries are all facts about *that* session, and
// they stay true after the config file that named them has changed.
//
// Read top down in the order the questions come: what is it doing, what has
// it cost, where is the work, and what was it given to do it with. It used to
// be one sixteen-row grid in monospace, which answered all four at the same
// volume -- the task, a sentence, wrapped as code, and the context meter, the
// one number that says a session is about to compact, came last.

import type { Session } from "../gen/Session";
import type { Usage } from "../gen/Usage";
import {
  Agent,
  Branch,
  Integrations,
  Policy,
  Repo,
  Sandbox,
  Secret,
  Skill,
  Toolchain,
  Tracker,
} from "../icons";

export function Facts({ session, usage }: { session: Session; usage: Usage | null }) {
  return (
    <div className="facts-pane">
      <section className="facts-task">
        {session.task ? <p>{session.task}</p> : <p className="none">started at a prompt, with no task</p>}
        {session.ticket && (
          <a className="pill fixed ticket" href={session.ticket.url} title={session.ticket.url}>
            <Tracker />
            {session.ticket.key}
          </a>
        )}
      </section>

      {/* What this session has spent, which is the half of the status line
          payload that *is* about the session -- the rate-limit windows beside
          it in the header are the account's. `null` until the agent's status
          line has run once, and a session that has spent nothing says so
          rather than showing $0.00 as though it were a measurement. */}
      {usage ? <Spend usage={usage} /> : <p className="facts-note">Spending shows here once the agent has reported.</p>}

      <section>
        <h4>worktree</h4>
        <Fact icon={Branch} label="branch">
          <span className="mono">{session.work_branch}</span>
          <span className="quiet"> from {session.base_branch ?? "the remote's default"}</span>
        </Fact>
        <Fact icon={Repo} label="repo">
          <span className="mono" title={session.repo}>
            {repoName(session.repo)}
          </span>
        </Fact>
        <Fact icon={Sandbox} label="sandbox">
          <span className="mono">{session.sandbox}</span>
        </Fact>
      </section>

      <section>
        <h4>given</h4>
        <Fact icon={Agent} label="agent">
          <span className="mono">{session.agent}</span>
          {usage?.model && <span className="quiet mono"> {usage.model}</span>}
        </Fact>
        <Fact icon={Policy} label="policy">
          {session.policy ? <span className="mono">{session.policy}</span> : <None>none recorded</None>}
        </Fact>
        <Chips icon={Toolchain} label="toolchains" values={session.toolchains} />
        <Chips icon={Secret} label="providers" values={session.providers} />
        <Chips
          icon={Skill}
          label="skills"
          values={session.skills.map((s) => s.name)}
          titles={session.skills.map((s) => s.source)}
        />
        <Chips
          icon={Integrations}
          label="mcp"
          values={session.mcp.map((m) => m.name)}
          titles={session.mcp.map((m) => m.url)}
        />
      </section>
    </div>
  );
}

/// The three numbers, and the context meter under them.
function Spend({ usage }: { usage: Usage }) {
  const context = usage.context_used_percentage;
  return (
    <section className="facts-spend">
      <div className="tiles">
        <Tile label="cost" value={usage.cost_usd === null ? notYet : `$${usage.cost_usd.toFixed(2)}`} />
        <Tile label="worked" value={usage.duration_ms === null ? notYet : duration(usage.duration_ms)} />
        <Tile
          label="lines"
          value={
            usage.lines_added === null ? (
              notYet
            ) : (
              <>
                <span className="added">+{usage.lines_added}</span>{" "}
                <span className="removed">−{usage.lines_removed ?? 0}</span>
              </>
            )
          }
        />
      </div>
      {/* How full the context is, which is what says whether this session is
          about to compact -- the most useful number in the payload. A meter
          rather than a figure, and it turns yellow, then red, as it fills:
          the warning is the point of it. */}
      {context !== null && (
        <div className="meter-row">
          <span className="meter-label">context</span>
          <span
            className={`meter ${context >= 90 ? "bad" : context >= 70 ? "warn" : ""}`}
            role="meter"
            aria-label="context used"
            aria-valuenow={Math.round(context)}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <span style={{ width: `${Math.min(100, Math.max(0, context))}%` }} />
          </span>
          <span className="meter-value">
            {Math.round(context)}%
            {usage.context_size ? <span className="quiet"> of {Math.round(usage.context_size / 1000)}k</span> : null}
          </span>
        </div>
      )}
    </section>
  );
}

/// A tile the agent has not reported a number for. Words, not a dash: a
/// dash in a tile reads as zero as often as it reads as missing.
const notYet = <span className="quiet">not yet</span>;

function Tile({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="tile">
      <span className="tile-value">{value}</span>
      <span className="tile-label">{label}</span>
    </div>
  );
}

type Glyph = React.ComponentType<{ className?: string }>;

function Fact({ icon: Icon, label, children }: { icon: Glyph; label: string; children: React.ReactNode }) {
  return (
    <div className="fact">
      <Icon className="fact-glyph" />
      <span className="fact-label">{label}</span>
      <span className="fact-value">{children}</span>
    </div>
  );
}

/// An empty list says so rather than being omitted. "no skills" and "this pane
/// forgot to render skills" look identical when the row is missing, and the
/// first is a thing worth knowing.
function Chips({
  icon,
  label,
  values,
  titles,
}: {
  icon: Glyph;
  label: string;
  values: string[];
  titles?: string[];
}) {
  return (
    <Fact icon={icon} label={label}>
      {values.length === 0 ? (
        <None>none</None>
      ) : (
        <span className="fact-chips">
          {values.map((v, i) => (
            <span key={v} className="pill fixed" title={titles?.[i]}>
              {v}
            </span>
          ))}
        </span>
      )}
    </Fact>
  );
}

const None = ({ children }: { children: React.ReactNode }) => <span className="none">{children}</span>;

/// `owner/repo` out of a clone URL, https or ssh. The whole URL is the
/// tooltip; what fits in a row is which repository, not how it is reached.
function repoName(url: string): string {
  const path = url
    .replace(/\.git$/, "")
    .replace(/^[a-z+]+:\/\/[^/]+\//i, "")
    .replace(/^[^@]+@[^:]+:/, "");
  return path || url;
}

/// Minutes and seconds of wall clock. Not a relative age like the tree's: this
/// is how long the agent has been *working*, which is a duration and not a
/// point in time.
function duration(ms: number): string {
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${secs % 60}s`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}
