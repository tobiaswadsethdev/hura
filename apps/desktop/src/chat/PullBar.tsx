// The pull request a chat's worktree is on, over the composer as Claude
// Code's desktop app has it: what it is and where it stands, and a click
// opens it beside the sidebar, as its mark in the tree does.

import type { PullRequest as Pull } from "../gen/PullRequest";
import { PullDraft, PullRequest } from "../icons";
import { ChecksMark, ReviewWord } from "../Inbox";
import { pullName } from "../inboxNotify";

export function PullBar({ pull, onOpen }: { pull: Pull | null; onOpen: (pull: Pull) => void }) {
  if (!pull) return null;
  const Mark = pull.draft ? PullDraft : PullRequest;
  return (
    <button
      className="chat-pull"
      title={`${pullName(pull)} ${pull.title}\n${pull.branch} → ${pull.base}`}
      onClick={() => onOpen(pull)}
    >
      <Mark className={`inbox-pull-mark${pull.draft ? " draft" : ""}`} aria-label={pull.draft ? "Draft" : "Pull request"} />
      <span className="chat-pull-key">{pull.key}</span>
      <span className="chat-pull-title">{pull.title}</span>
      <span className="chat-pull-state">
        {pull.conflicts && <span className="inbox-conflicts">conflicts</span>}
        <ChecksMark checks={pull.checks} />
        <ReviewWord pull={pull} />
      </span>
    </button>
  );
}
