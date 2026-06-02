/**
 * ProjectsSurface — top-level destination for managing indexed projects.
 *
 * Master-detail layout: list on the left, detail on the right when a card
 * is selected. Live indexing progress is driven by the new
 * `indexing_progress_events` Channel (via `useIndexingProgress`) so the
 * UI no longer polls the daemon every 1.5s.
 *
 * Replaces the legacy `components/ProjectList.tsx` wrapper. Confirmation
 * for both reindex and remove flows through the unified `ConfirmDialog`.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Box,
  Clock,
  Code2,
  FileText,
  FolderOpen,
  Layers,
  Loader2,
  MousePointerClick,
  Plus,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import type { CorpusInfo } from "../../lib/types";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { formatEta, formatRelativeTime } from "../../lib/format";
import { listContainer, listItem, spring } from "../../lib/motion";
import { corpusStatusBadge } from "../../lib/status";
import { cn } from "../../lib/utils";
import {
  useIndexingProgress,
  type IndexingProgressEvent,
} from "../../hooks/useIndexingProgress";

import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { ContentTray } from "../ui/content-tray";
import { EmptyState } from "../ui/empty-state";
import { H1 } from "../ui/heading";
import { MetricTile } from "../ui/metric-tile";
import { Progress } from "../ui/progress";
import { AdaptiveSurface } from "../ui/adaptive-surface";
import { ProjectSessions } from "./ProjectSessions";
import { LinkedProjectsPanel } from "./LinkedProjectsPanel";
import { useToast } from "../shell/ToastTray";

interface Props {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onRefresh: () => void;
}

export function ProjectsSurface({
  corpora,
  activeCorpusId,
  onSelectCorpus,
  onRefresh,
}: Props) {
  const [adding, setAdding] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<CorpusInfo | null>(null);
  const [confirmReindex, setConfirmReindex] = useState<CorpusInfo | null>(null);

  const progress = useIndexingProgress();
  const { toast } = useToast();

  const selected = useMemo(
    () => corpora.find((c) => c.id === activeCorpusId) ?? null,
    [corpora, activeCorpusId],
  );

  async function addProject() {
    setAdding(true);
    try {
      // `null` = the folder picker was cancelled (not an error).
      const res = await invoke<{ corpus_id: string } | null>(
        "add_project_dialog",
      );
      if (!res) return;
      onRefresh();
      toast("Project added", { tone: "success" });
    } catch (e) {
      toast("Couldn’t add project", {
        detail: String(e),
        tone: "danger",
      });
    } finally {
      setAdding(false);
    }
  }

  async function scanForProjects() {
    setScanning(true);
    try {
      const detected = await invoke<{ path: string; name: string }[]>(
        "detect_projects",
      );
      if (detected.length > 0) {
        const paths = detected.map((d) => d.path);
        await invoke("register_projects_batch", { paths });
        onRefresh();
        toast("Projects found", {
          detail: `Added ${detected.length}`,
          tone: "success",
        });
      } else {
        toast("No projects found", {
          detail: "No .ministr.toml under the usual roots",
          tone: "info",
        });
      }
    } catch (e) {
      toast("Scan failed", { detail: String(e), tone: "danger" });
    } finally {
      setScanning(false);
    }
  }

  async function performRemove() {
    const c = confirmRemove;
    if (!c) return;
    setConfirmRemove(null);
    try {
      await invoke("remove_project", { corpusId: c.id });
      await onRefresh();
    } catch (err) {
      console.error("[ministr] remove_project error:", err);
    }
  }

  async function performReindex() {
    const c = confirmReindex;
    if (!c) return;
    setConfirmReindex(null);
    try {
      await invoke("trigger_reindex", { corpusId: c.id });
      await onRefresh();
    } catch (err) {
      console.error("[ministr] trigger_reindex error:", err);
    }
  }

  return (
    <AdaptiveSurface>
    <div className="h-full flex flex-col min-h-0">
      <header className="flex items-center justify-between gap-4 p-5 pb-3 shrink-0">
        <div className="min-w-0">
          <H1>Projects</H1>
          <p className="font-sans text-sm text-text-dim mt-1">
            {corpora.length === 0
              ? "None registered."
              : `${corpora.length} ${corpora.length === 1 ? "project" : "projects"} indexed.`}
          </p>
        </div>
        {corpora.length > 0 && (
          <div className="flex items-center gap-2 shrink-0">
            <Button
              variant="outline"
              size="sm"
              onClick={scanForProjects}
              disabled={scanning}
            >
              {scanning ? (
                <Loader2 className="h-4 w-4 animate-spin" strokeWidth={2} />
              ) : (
                <Search className="h-4 w-4" strokeWidth={2} />
              )}
              {scanning ? "Scanning…" : "Scan"}
            </Button>
            <Button onClick={addProject} disabled={adding} size="sm">
              <Plus className="h-4 w-4" strokeWidth={2} />
              Add project
            </Button>
          </div>
        )}
      </header>

      <div className="flex-1 min-h-0 flex gap-4 px-5 pb-5 min-w-0">
        {corpora.length === 0 ? (
          <div className="flex-1 grid place-items-center min-h-0">
            <EmptyState
              accent
              icon={FolderOpen}
              title="No projects yet"
              hint={
                <>
                  Point ministr at any folder, or pick a directory containing
                  an{" "}
                  <span className="font-mono not-italic">.ministr.toml</span>.
                </>
              }
              action={
                <div className="flex items-center gap-2">
                  <Button onClick={addProject} disabled={adding} size="lg">
                    <Plus className="h-4 w-4" strokeWidth={2} />
                    Add your first project
                  </Button>
                  <Button
                    variant="outline"
                    size="lg"
                    onClick={scanForProjects}
                    disabled={scanning}
                  >
                    <Search className="h-4 w-4" strokeWidth={2} />
                    {scanning ? "Scanning…" : "Scan ~/Code"}
                  </Button>
                </div>
              }
            />
          </div>
        ) : (
          <>
            <ProjectMaster
              corpora={corpora}
              activeCorpusId={activeCorpusId}
              progress={progress}
              onSelect={onSelectCorpus}
              onReindex={(c) => setConfirmReindex(c)}
              onRemove={(c) => setConfirmRemove(c)}
            />

            <ProjectDetail
              corpus={selected}
              progress={selected ? progress[selected.id] : undefined}
              onReindex={() => selected && setConfirmReindex(selected)}
              onRemove={() => selected && setConfirmRemove(selected)}
            />
          </>
        )}
      </div>

      {corpora.length > 0 && (
        <div className="border-t border-border px-5 py-4 shrink-0 overflow-y-auto max-h-[40%]">
          <LinkedProjectsPanel
            corpora={corpora}
            activeCorpusId={activeCorpusId}
          />
        </div>
      )}

      <ConfirmDialog
        open={!!confirmReindex}
        title="Re-index project"
        body={
          confirmReindex && (
            <>
              <p>
                This drops the existing index for{" "}
                <span className="font-bold">
                  {corpusLabel(confirmReindex)}
                </span>{" "}
                and starts over.
              </p>
              <p className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim mt-2">
                {confirmReindex.files_indexed.toLocaleString()} files ·{" "}
                {confirmReindex.sections_count.toLocaleString()} sections
              </p>
            </>
          )
        }
        confirmLabel="Re-index"
        onCancel={() => setConfirmReindex(null)}
        onConfirm={performReindex}
      />

      <ConfirmDialog
        open={!!confirmRemove}
        title="Remove project"
        tone="danger"
        body={
          confirmRemove && (
            <>
              <p>
                This permanently removes{" "}
                <span className="font-bold">{corpusLabel(confirmRemove)}</span>{" "}
                from the registry, including all indexed sections and symbols.
              </p>
              <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-2">
                Source files on disk are not touched.
              </p>
            </>
          )
        }
        confirmLabel="Remove"
        confirmToken={confirmRemove ? corpusLabel(confirmRemove) : undefined}
        onCancel={() => setConfirmRemove(null)}
        onConfirm={performRemove}
      />
    </div>
    </AdaptiveSurface>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Master list

function ProjectMaster({
  corpora,
  activeCorpusId,
  progress,
  onSelect,
  onReindex,
  onRemove,
}: {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  progress: Record<string, IndexingProgressEvent>;
  onSelect: (id: string) => void;
  onReindex: (corpus: CorpusInfo) => void;
  onRemove: (corpus: CorpusInfo) => void;
}) {
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());

  // Scroll the active card into view when selection changes (keyboard nav,
  // tray-launched switch, etc.).
  useEffect(() => {
    if (!activeCorpusId) return;
    const el = cardRefs.current.get(activeCorpusId);
    if (el) el.scrollIntoView({ block: "nearest", behavior: "auto" });
  }, [activeCorpusId]);

  return (
    <motion.div
      variants={listContainer}
      initial="initial"
      animate="animate"
      className="w-[380px] shrink-0 min-h-0 overflow-y-auto pr-1 space-y-2.5"
    >
      <AnimatePresence mode="popLayout">
        {corpora.map((corpus) => (
          <ProjectCard
            key={corpus.id}
            ref={(el) => {
              if (el) cardRefs.current.set(corpus.id, el);
              else cardRefs.current.delete(corpus.id);
            }}
            corpus={corpus}
            progress={progress[corpus.id]}
            selected={corpus.id === activeCorpusId}
            onClick={() => onSelect(corpus.id)}
            onReindex={() => onReindex(corpus)}
            onRemove={() => onRemove(corpus)}
          />
        ))}
      </AnimatePresence>
    </motion.div>
  );
}

interface ProjectCardProps {
  corpus: CorpusInfo;
  progress: IndexingProgressEvent | undefined;
  selected: boolean;
  onClick: () => void;
  onReindex: () => void;
  onRemove: () => void;
  ref?: React.Ref<HTMLDivElement>;
}

function ProjectCard({
  corpus,
  progress,
  selected,
  onClick,
  onReindex,
  onRemove,
  ref,
}: ProjectCardProps) {
  const indexing = corpus.status.state === "indexing";
  const { variant: statusVariant, label: statusLabel } =
    corpusStatusBadge(corpus);
  const filesDone = progress?.files_done ?? 0;
  const filesTotal = progress?.files_total ?? 0;
  // During the GPU-bound embedding phase the parser is backpressured by the
  // bounded parse→embed channel, so the file counter stalls for seconds while
  // embeddings keep streaming in. Drive the live readout by whichever metric is
  // actually moving — files while parsing, vectors while embedding — so the bar
  // stays granular end-to-end. (The backend emits an event per embedding batch;
  // see `indexing_progress_events`.)
  const embDone = progress?.embeddings_done ?? 0;
  const embTotal = progress?.embeddings_total ?? 0;
  const embedding = progress?.phase === "embedding" && embTotal > 0;
  const headDone = embedding ? embDone : filesDone;
  const headTotal = embedding ? embTotal : filesTotal;
  const headUnit = embedding ? "vectors" : "files";
  const pct = headTotal > 0 ? (headDone / headTotal) * 100 : 0;

  return (
    <motion.div
      layout
      layoutId={`project-${corpus.id}`}
      variants={listItem}
      exit="exit"
      transition={spring}
    >
    <Card
      ref={ref}
      hover="lift"
      className={cn(
        "group cursor-pointer p-4",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent focus-visible:z-20",
        selected && "border-accent shadow-sm",
      )}
      onClick={onClick}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 flex-wrap min-w-0">
          <span className="font-mono text-sm font-bold tracking-[0.08em] text-text truncate">
            {corpusLabel(corpus)}
          </span>
          <Badge variant={statusVariant} dot>
            {statusLabel}
          </Badge>
        </div>
        <div className="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-150">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={(e) => {
              e.stopPropagation();
              onReindex();
            }}
            title="Re-index"
            aria-label="Re-index"
          >
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2.5} />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
            title="Remove"
            aria-label="Remove"
            className="hover:text-danger"
          >
            <Trash2 className="h-3.5 w-3.5" strokeWidth={2.5} />
          </Button>
        </div>
      </div>

      <p className="text-mono-mini text-text-dim font-mono truncate mt-1">
        {corpusRoot(corpus.paths)}
      </p>

      {indexing ? (
        <div className="mt-3">
          <div className="flex justify-between text-mono-mini font-mono uppercase tracking-[0.08em] text-warning mb-1.5">
            <span>
              {headDone.toLocaleString()} / {headTotal.toLocaleString()}{" "}
              {headUnit}
            </span>
            <span className="tabular-nums">
              {progress?.estimated_remaining_secs != null
                ? formatEta(progress.estimated_remaining_secs)
                : "ETA …"}
            </span>
          </div>
          <Progress tone="warning" value={pct} />
        </div>
      ) : (
        <div className="flex items-center justify-between mt-2 text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim">
          <span>{corpus.files_indexed.toLocaleString()} files</span>
          {corpus.last_indexed && (
            <span title={new Date(corpus.last_indexed * 1000).toLocaleString()}>
              {formatRelativeTime(corpus.last_indexed)}
            </span>
          )}
        </div>
      )}

      {corpus.status.state === "error" && (
        <p className="mt-3 text-mono-mini text-danger flex items-start gap-1.5 font-mono">
          <AlertTriangle
            className="h-3 w-3 shrink-0 mt-0.5"
            strokeWidth={2.5}
          />
          {corpus.status.message}
        </p>
      )}
    </Card>
    </motion.div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Detail pane

function ProjectDetail({
  corpus,
  progress,
  onReindex,
  onRemove,
}: {
  corpus: CorpusInfo | null;
  progress: IndexingProgressEvent | undefined;
  onReindex: () => void;
  onRemove: () => void;
}) {
  if (!corpus) {
    return (
      <div className="flex-1 grid place-items-center min-h-0 border-l border-border-soft">
        <EmptyState
          icon={MousePointerClick}
          title="No project selected"
          hint="Select a project from the list to see its details."
        />
      </div>
    );
  }

  const indexing = corpus.status.state === "indexing";
  const filesDone = progress?.files_done ?? 0;
  const filesTotal = progress?.files_total ?? 0;
  // During the GPU-bound embedding phase the parser is backpressured by the
  // bounded parse→embed channel, so the file counter stalls for seconds while
  // embeddings keep streaming in. Drive the live readout by whichever metric is
  // actually moving — files while parsing, vectors while embedding — so the bar
  // stays granular end-to-end. (The backend emits an event per embedding batch;
  // see `indexing_progress_events`.)
  const embDone = progress?.embeddings_done ?? 0;
  const embTotal = progress?.embeddings_total ?? 0;
  const embedding = progress?.phase === "embedding" && embTotal > 0;
  const headDone = embedding ? embDone : filesDone;
  const headTotal = embedding ? embTotal : filesTotal;
  const headUnit = embedding ? "vectors" : "files";
  const pct = headTotal > 0 ? (headDone / headTotal) * 100 : 0;
  const { variant: statusVariant, label: statusLabel } =
    corpusStatusBadge(corpus);

  return (
    <div className="flex-1 min-w-0 min-h-0 overflow-y-auto border-l border-border-soft pl-5">
      <div className="space-y-5">
        <div>
          <div className="flex items-center gap-2 flex-wrap">
            <h2 className="font-mono text-lg font-bold tracking-[0.08em] text-text">
              {corpusLabel(corpus)}
            </h2>
            <Badge variant={statusVariant} dot>
              {statusLabel}
            </Badge>
            {corpus.active_sessions > 0 && (
              <Badge variant="default" dot>
                {corpus.active_sessions} session
                {corpus.active_sessions !== 1 ? "s" : ""}
              </Badge>
            )}
          </div>
          <p className="font-mono text-xs text-text-dim mt-1 truncate">
            {corpusRoot(corpus.paths)}
          </p>
        </div>

        {indexing && (
          <ContentTray compact className="space-y-2">
            <div className="flex justify-between text-mono-mini font-mono uppercase tracking-[0.08em] text-warning">
              <span>
                {progress?.phase ? `${progress.phase} · ` : ""}
                {headDone.toLocaleString()} / {headTotal.toLocaleString()}{" "}
                {headUnit}
              </span>
              <span className="tabular-nums">
                {progress?.estimated_remaining_secs != null
                  ? formatEta(progress.estimated_remaining_secs)
                  : "ETA …"}
              </span>
            </div>
            <Progress tone="warning" value={pct} />
            {progress?.current_file && (
              <p className="font-mono text-mono-mini text-text-dim truncate">
                {progress.current_file}
              </p>
            )}
          </ContentTray>
        )}

        <ContentTray compact>
          <div className="grid grid-cols-2 gap-2">
            <MetricTile
              icon={FileText}
              value={corpus.files_indexed.toLocaleString()}
              label="Files"
            />
            <MetricTile
              icon={Layers}
              value={corpus.sections_count.toLocaleString()}
              label="Sections"
            />
            <MetricTile
              icon={Code2}
              value={(corpus.symbols_count ?? 0).toLocaleString()}
              label="Symbols"
            />
            <MetricTile
              icon={Box}
              value={corpus.embeddings_count.toLocaleString()}
              label="Vectors"
            />
          </div>
        </ContentTray>

        {corpus.last_indexed && (
          <div className="flex items-center gap-2 text-mono-mini font-mono uppercase tracking-[0.08em] text-text-dim">
            <Clock className="h-3 w-3" strokeWidth={2.5} />
            <span>Last indexed · {formatRelativeTime(corpus.last_indexed)}</span>
          </div>
        )}

        <div className="flex items-center gap-2 pt-2 border-t border-border-soft">
          <Button variant="outline" size="sm" onClick={onReindex}>
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
            Re-index
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onRemove}
            className="hover:text-danger ml-auto"
          >
            <Trash2 className="h-3.5 w-3.5" strokeWidth={2} />
            Remove
          </Button>
        </div>

        {/* No `key` — ProjectSessions reads the shared session store and
            re-derives its slice on `corpus` change, so switching projects
            is a filter swap, not a remount (no poll restart / loading
            flash). */}
        <ProjectSessions corpus={corpus} />
      </div>
    </div>
  );
}

