import { useMemo, useState } from "react";
import { corpusFreshness, triggerReindex } from "../../lib/ipc";
import type { CorpusInfo } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { buildTree, leafNote, summarize } from "../../lib/trustSummary";
import type { TreeNode } from "../../lib/trustSummary";
import { StatusBanner } from "../ui/StatusBanner";
import { ActionChip } from "../ui/ActionChip";
import { TreeRow } from "../ui/TreeRow";
import { TrustMark } from "../ui/TrustMark";
import { RailRow, RailSection } from "../ui/Rail";

/**
 * Project Mirror (UX-BLUEPRINT §3.2) — what your AI sees. The tree IS
 * the trust display: every row's state is hash-verified by the daemon.
 * Presence line lands with gui-rw-session-outcome (the turn stream).
 */
export function ProjectMirror({
  corpus,
  onBack,
  onOpenFeed,
}: {
  corpus: CorpusInfo;
  onBack: () => void;
  onOpenFeed?: () => void;
}) {
  const { data: fresh } = usePoll(
    () => corpusFreshness(corpus.id),
    4_000,
  );

  const summary = fresh ? summarize(corpus.display_name, fresh) : null;
  const tree = useMemo(() => (fresh ? buildTree(fresh.files) : []), [fresh]);

  return (
    <div className="mx-auto flex min-h-screen max-w-5xl flex-col gap-4 p-8">
      <header className="flex items-center gap-3">
        <ActionChip onClick={onBack} aria-label="back to all projects">
          ‹
        </ActionChip>
        <h1 className="text-xl font-semibold tracking-tight text-ink">
          {corpus.display_name}
          <span className="ml-2 text-sm font-normal text-dim">
            what your AI sees
          </span>
        </h1>
        {onOpenFeed ? (
          <span className="ml-auto">
            <ActionChip onClick={onOpenFeed}>What ministr did</ActionChip>
          </span>
        ) : null}
      </header>

      {summary ? (
        <StatusBanner
          state={summary.state}
          headline={summary.headline}
          sub={summary.sub}
          action={
            summary.state === "stale" ? (
              <ActionChip
                variant="primary"
                onClick={() => void triggerReindex(corpus.id)}
              >
                Catch up
              </ActionChip>
            ) : undefined
          }
        />
      ) : null}

      <div className="flex gap-4">
        <section
          aria-label="files as your AI sees them"
          className="min-w-0 flex-1 rounded-lg border border-line bg-surface p-1"
        >
          {tree.map((node) => (
            <TreeBranch key={node.path} node={node} level={0} />
          ))}
          {fresh && tree.length === 0 ? (
            <p className="p-4 text-sm text-dim">nothing indexed yet</p>
          ) : null}
        </section>

        <aside className="w-56 shrink-0 space-y-4">
          <RailSection label="this project">
            <RailRow label="watching">
              <TrustMark state="ok" />
            </RailRow>
            <RailRow label="files">
              {String(corpus.files_indexed)}
            </RailRow>
            {corpus.active_sessions > 0 ? (
              <RailRow label="agents reading">
                {String(corpus.active_sessions)}
              </RailRow>
            ) : null}
          </RailSection>
          <details>
            <summary className="cursor-pointer px-2 text-xs text-dim">
              advanced
            </summary>
            <div className="mt-2">
              <RailSection label="details">
                <RailRow label="id">
                  <span className="font-mono text-xs">{corpus.id.slice(0, 12)}</span>
                </RailRow>
                <RailRow label="sections">
                  {String(corpus.sections_count)}
                </RailRow>
              </RailSection>
            </div>
          </details>
        </aside>
      </div>
    </div>
  );
}

/**
 * One collapsible branch. Directories collapse (collapsed-by-default
 * when healthy — the quiet-until-it-isn't rule made structural: only
 * subtrees with something to say start open).
 */
function TreeBranch({ node, level }: { node: TreeNode; level: number }) {
  const [open, setOpen] = useState(node.state !== "ok" || level === 0);

  if (node.isFile) {
    return (
      <TreeRow
        name={node.name}
        state={node.state}
        level={level}
        note={leafNote(node.raw)}
      />
    );
  }
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="w-full rounded-md text-left focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        <TreeRow
          name={`${node.name}/`}
          state={node.state}
          level={level}
          note={open ? undefined : node.state === "ok" ? undefined : "needs a look"}
        />
      </button>
      {open
        ? node.children.map((c) => (
            <TreeBranch key={c.path} node={c} level={level + 1} />
          ))
        : null}
    </div>
  );
}
