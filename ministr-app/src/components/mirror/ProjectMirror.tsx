import { useEffect, useMemo, useState } from "react";
import { corpusFreshness, recentActivity } from "../../lib/ipc";
import type { CorpusInfo } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";
import { buildTree, leafNote, summarize } from "../../lib/trustSummary";
import type { TreeNode } from "../../lib/trustSummary";
import { StatusBanner } from "../ui/StatusBanner";
import { ActionChip } from "../ui/ActionChip";
import { BackButton } from "../ui/BackButton";
import { CatchUp } from "../ui/CatchUp";
import { TreeRow } from "../ui/TreeRow";
import { TrustMark } from "../ui/TrustMark";
import { RailRow, RailSection } from "../ui/Rail";
import { FileDrillin } from "./FileDrillin";
import { derivePresence } from "../../lib/presence";
import { LiveDot } from "../ui/LiveDot";
import { ConnectionNote } from "../ui/ConnectionNote";
import { ExpertConfig } from "./ExpertConfig";
import { IndexingInstrument } from "../ui/IndexingInstrument";
import { Screen } from "../ui/Screen";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

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
  const { data: fresh, error: connError } = usePoll(
    () => corpusFreshness(corpus.id),
    4_000,
  );
  const { data: activity } = usePoll(() => recentActivity(30), 4_000);
  // Live indexing progress for the full instrument in the banner
  // (gui-indexing-instrument).
  const { progress } = useIngestionProgress(1_000);
  const liveProgress = progress.get(corpus.id);
  const [pendingAt, setPendingAt] = useState<number | null>(null);

  // Optimism yields to real data (or a 15s safety net).
  useEffect(() => {
    if (pendingAt === null || !fresh) return;
    if (fresh.indexing || Date.now() - pendingAt > 15_000) setPendingAt(null);
  }, [fresh, pendingAt]);
  const presence = derivePresence(activity ?? [], corpus.id, Date.now());

  const summary = fresh
    ? summarize(corpus.display_name, {
        ...fresh,
        indexing: fresh.indexing || pendingAt !== null,
      })
    : null;
  const tree = useMemo(
    () => (fresh ? buildTree(fresh.files, fresh.indexing) : []),
    [fresh],
  );
  const [openFile, setOpenFile] = useState<TreeNode | null>(null);

  // Escape closes the drill-in and restores focus to the row that
  // opened it (gui-rw-keyboard-flow); App's Escape handler only fires
  // when nothing here consumed the key (defaultPrevented).
  useEffect(() => {
    if (!openFile) return;
    const path = openFile.path;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      setOpenFile(null);
      requestAnimationFrame(() => {
        document
          .querySelector<HTMLElement>(`[data-tree-path="${CSS.escape(path)}"]`)
          ?.focus();
      });
    };
    // Capture phase: keydown targets the focused element, so this
    // fires on the way DOWN — before App's bubble-phase Escape handler
    // ever sees the event, guaranteeing the drill-in consumes it first.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [openFile]);

  // Live presence rides in the trust-footer when an agent is reading;
  // otherwise Screen falls back to its default "ministr running" footer.
  const footer =
    presence?.kind === "live" ? (
      <div className="border-t border-line pt-3">
        <LiveDot label={presence.sentence} />
      </div>
    ) : presence?.kind === "recent" ? (
      <div className="border-t border-line pt-3 text-sm text-dim">
        {presence.sentence}
      </div>
    ) : undefined;

  return (
    <Screen
      width="5xl"
      align="center"
      footer={footer}
      header={
        <div className="flex items-center gap-3">
          <BackButton onClick={onBack} label="All projects" />
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
        </div>
      }
    >
      {summary ? (
        <StatusBanner
          state={summary.state}
          headline={summary.headline}
          sub={summary.sub}
          action={
            summary.state === "stale" ? (
              <CatchUp corpusId={corpus.id} onAccepted={() => setPendingAt(Date.now())} />
            ) : undefined
          }
          footer={
            summary.state === "updating" && liveProgress?.running ? (
              <IndexingInstrument progress={liveProgress} />
            ) : undefined
          }
        />
      ) : null}

      {connError && fresh ? <ConnectionNote /> : null}
      {openFile ? (
        <FileDrillin
          corpusId={corpus.id}
          path={openFile.path}
          state={openFile.state}
          onBack={() => setOpenFile(null)}
        />
      ) : (
      <div className="flex gap-4">
        <section
          aria-label="files as your AI sees them"
          className="min-w-0 flex-1 rounded-lg border border-line bg-surface p-1"
          onKeyDown={treeKeyNav}
        >
          {tree.map((node) => (
            <TreeBranch key={node.path} node={node} level={0} onOpenFile={setOpenFile} />
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
              <RailRow label="agents connected">
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
              <div className="mt-3 px-2">
                <ExpertConfig
                  corpusId={corpus.id}
                  model={corpus.model}
                  onSaved={() => setPendingAt(Date.now())}
                />
              </div>
            </div>
          </details>
        </aside>
      </div>
      )}
    </Screen>
  );
}

/**
 * One collapsible branch. Directories collapse (collapsed-by-default
 * when healthy — the quiet-until-it-isn't rule made structural: only
 * subtrees with something to say start open).
 */
function TreeBranch({
  node,
  level,
  onOpenFile,
}: {
  node: TreeNode;
  level: number;
  onOpenFile?: (node: TreeNode) => void;
}) {
  const [open, setOpen] = useState(node.state !== "ok" || level === 0);

  if (node.isFile) {
    return (
      <button
        type="button"
        data-tree-row
        data-tree-path={node.path}
        onClick={() => onOpenFile?.(node)}
        className="w-full cursor-pointer rounded-md text-left focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        <TreeRow
          name={node.name}
          state={node.state}
          level={level}
          note={leafNote(node.raw, node.state === "updating")}
          disclosure="navigates"
        />
      </button>
    );
  }
  return (
    <div>
      <button
        type="button"
        data-tree-row
        data-tree-dir
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="w-full cursor-pointer rounded-md text-left focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        <TreeRow
          name={`${node.name}/`}
          state={node.state}
          level={level}
          note={open ? undefined : node.state === "ok" ? undefined : "needs a look"}
          disclosure={open ? "expanded" : "expandable"}
        />
      </button>
      {open
        ? node.children.map((c) => (
            <TreeBranch
              key={c.path}
              node={c}
              level={level + 1}
              onOpenFile={onOpenFile}
            />
          ))
        : null}
    </div>
  );
}

/** Roving arrow-key navigation over the rendered tree rows
 *  (gui-rw-keyboard-flow). Deliberately NOT role=tree: the rendered
 *  structure can't satisfy the ARIA tree pattern's required children,
 *  and bad ARIA is worse than honest buttons with aria-expanded. */
function treeKeyNav(e: React.KeyboardEvent<HTMLElement>) {
  const keys = ["ArrowDown", "ArrowUp", "Home", "End", "ArrowRight", "ArrowLeft"];
  if (!keys.includes(e.key)) return;
  const rows = [
    ...e.currentTarget.querySelectorAll<HTMLButtonElement>("[data-tree-row]"),
  ];
  const current = document.activeElement as HTMLButtonElement | null;
  const idx = current ? rows.indexOf(current) : -1;
  if (idx === -1) return;

  const isDir = current!.hasAttribute("data-tree-dir");
  const expanded = current!.getAttribute("aria-expanded") === "true";

  if (e.key === "ArrowRight") {
    e.preventDefault();
    if (isDir && !expanded) current!.click();
    else rows[idx + 1]?.focus();
    return;
  }
  if (e.key === "ArrowLeft") {
    e.preventDefault();
    if (isDir && expanded) current!.click();
    return;
  }
  e.preventDefault();
  const next =
    e.key === "Home"
      ? rows[0]
      : e.key === "End"
        ? rows[rows.length - 1]
        : rows[idx + (e.key === "ArrowDown" ? 1 : -1)];
  next?.focus();
}
