// What a session is: the record, as it describes itself.
//
// Everything here is stored on the session rather than derived, which is the
// point of it -- the skills it was given, the MCP servers it was created with
// and the toolchains its image carries are all facts about *that* session, and
// they stay true after the config file that named them has changed.
//
// Read top down in the order the questions come: what is it doing, how far
// has it got, where is the work, and what was it given to do it with. It used to
// be one sixteen-row grid in monospace, which answered all four at the same
// volume -- the task, a sentence, wrapped as code, and the context meter, the
// one number that says a session is about to compact, came last. That meter
// has since moved to the chat composer, beside the message it is about.

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
  Unsandboxed,
} from "../icons";

export function Facts({ session, usage }: { session: Session; usage: Usage | null }) {
  return (
    <div className="facts-pane">
      <section className="facts-task">
        {session.task ? <p>{session.task}</p> : <p className="none">Started at a prompt, with no task</p>}
        {session.ticket && (
          <a className="pill fixed ticket" href={session.ticket.url} title={session.ticket.url}>
            <Tracker />
            {session.ticket.key}
          </a>
        )}
      </section>

      {/* What this session has done. `null` until the agent's status line has
          run once. */}
      {usage ? <Spend usage={usage} /> : <p className="facts-note">Progress shows here once the agent has reported.</p>}

      <section>
        <h4>Worktree</h4>
        <Fact icon={Branch} label="Branch">
          <span className="mono">{session.work_branch}</span>
          <span className="quiet"> from {session.base_branch ?? "the remote's default"}</span>
        </Fact>
        <Fact icon={Repo} label="Repo">
          <span className="mono" title={session.repo}>
            {repoName(session.repo)}
          </span>
        </Fact>
        {session.backend === "worktree" ? (
          <Fact icon={Unsandboxed} label="Not isolated">
            <span className="mono" title={session.workdir ?? undefined}>
              {session.workdir ?? "a worktree on the server"}
            </span>
            <span className="quiet"> on the server, as its user, with no policy</span>
          </Fact>
        ) : (
          <Fact icon={Sandbox} label="Sandbox">
            <span className="mono">{session.sandbox}</span>
          </Fact>
        )}
      </section>

      <section>
        <h4>Given</h4>
        <Fact icon={Agent} label="Agent">
          <span className="mono">{session.agent}</span>
          {usage?.model && <span className="quiet mono"> {usage.model}</span>}
        </Fact>
        <Fact icon={Policy} label="Policy">
          {session.policy ? <span className="mono">{session.policy}</span> : <None>none recorded</None>}
        </Fact>
        <Chips icon={Toolchain} label="Toolchains" values={session.toolchains} />
        <Chips icon={Secret} label="Providers" values={session.providers} />
        <Chips
          icon={Skill}
          label="Skills"
          values={session.skills.map((s) => s.name)}
          titles={session.skills.map((s) => s.source)}
        />
        <Chips
          icon={Integrations}
          label="MCP"
          values={session.mcp.map((m) => m.name)}
          titles={session.mcp.map((m) => m.url)}
        />
      </section>
    </div>
  );
}

/// The two numbers. How full the context is, and the account's limits, are
/// in the chat composer, where the next message is written: that is when they
/// are wanted, and this pane is not open then.
function Spend({ usage }: { usage: Usage }) {
  return (
    <section className="facts-spend">
      <div className="tiles">
        <Tile label="Worked" value={usage.duration_ms === null ? notYet : duration(usage.duration_ms)} />
        <Tile
          label="Lines"
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
