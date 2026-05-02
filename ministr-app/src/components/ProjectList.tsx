import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Box,
  Clock,
  Code2,
  FileText,
  FolderOpen,
  Layers,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from "lucide-react";
import type { CorpusInfo, IngestionProgressInfo } from "../lib/types";
import { corpusLabel, corpusRoot } from "../lib/corpus";
import { statusBadge } from "../lib/status";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Card } from "./ui/card";
import { CorpusChip } from "./ui/corpus-chip";
import { EmptyState } from "./ui/empty-state";
import { MetricTile } from "./ui/metric-tile";
import { Progress } from "./ui/progress";
import { cn } from "../lib/utils";
import { useEffect, useRef, useState } from "react";

interface ProjectListProps {
  corpora: CorpusInfo[];
  onRefresh: () => void;
  onSelect: (id: string) => void;
  selectedId: string | null;
}

interface ProgressTrack {
  done: number;
  total: number;
  ts: number;
}

export function ProjectList({
  corpora,
  onRefresh,
  onSelect,
  selectedId,
}: ProjectListProps) {
  const [adding, setAdding] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<CorpusInfo | null>(null);
  const [confirmReindex, setConfirmReindex] = useState<CorpusInfo | null>(null);
  const [progress, setProgress] = useState<
    Record<string, IngestionProgressInfo>
  >({});
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const trackRef = useRef<Map<string, ProgressTrack[]>>(new Map());

  // Scroll the selected corpus card into view when it changes.
  useEffect(() => {
    if (!selectedId) return;
    const el = cardRefs.current.get(selectedId);
    if (el) el.scrollIntoView({ block: "nearest", behavior: "auto" });
  }, [selectedId]);

  // Poll ingestion progress only while at least one corpus is indexing.
  useEffect(() => {
    const anyIndexing = corpora.some((c) => c.status.state === "indexing");
    if (!anyIndexing) {
      setProgress({});
      trackRef.current.clear();
      return;
    }
    let cancelled = false;
    async function tick() {
      try {
        const list = await invoke<IngestionProgressInfo[]>(
          "ingestion_progress",
        );
        if (cancelled) return;
        const map: Record<string, IngestionProgressInfo> = {};
        const now = Date.now();
        for (const p of list) {
          map[p.corpus_id] = p;
          // Track rate over last 30s for ETA.
          const arr = trackRef.current.get(p.corpus_id) ?? [];
          const next = [
            ...arr.filter((t) => now - t.ts < 30_000),
            { done: p.files_done, total: p.files_total, ts: now },
          ];
          trackRef.current.set(p.corpus_id, next);
        }
        setProgress(map);
      } catch {
        /* ignore */
      }
    }
    tick();
    const id = setInterval(tick, 1500);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [corpora]);

  function etaFor(corpusId: string): string | null {
    const arr = trackRef.current.get(corpusId);
    if (!arr || arr.length < 2) return null;
    const first = arr[0];
    const last = arr[arr.length - 1];
    const dt = (last.ts - first.ts) / 1000;
    const dn = last.done - first.done;
    if (dt < 1 || dn <= 0) return null;
    const rate = dn / dt; // files per second
    const remaining = last.total - last.done;
    if (remaining <= 0) return null;
    const seconds = remaining / rate;
    if (seconds < 60) return `ETA ${Math.round(seconds)}s`;
    if (seconds < 3600) return `ETA ${Math.round(seconds / 60)}m`;
    return `ETA ${(seconds / 3600).toFixed(1)}h`;
  }

  async function addProject() {
    setAdding(true);
    try {
      await invoke("add_project_dialog");
      onRefresh();
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
      }
    } catch (e) {
      console.error("[ministr] scan failed", e);
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
    <div className="space-y-4">
      {/* Page title row — quiet */}
      <div className="flex items-center justify-between gap-4">
        <div>
          <h2 className="font-serif text-2xl font-normal text-text leading-tight ">
            Projects
          </h2>
          <p className="font-serif text-sm italic text-text-dim mt-1">
            {corpora.length === 0
              ? "None registered."
              : `${corpora.length} ${corpora.length === 1 ? "corpus" : "corpora"} indexed.`}
          </p>
        </div>
      </div>

      {/* Primary action row — Add Project is the visual anchor */}
      {corpora.length > 0 && (
        <div className="flex items-center gap-2">
          <Button
            size="lg"
            onClick={addProject}
            disabled={adding}
            className="flex-1"
          >
            <Plus className="h-4 w-4" strokeWidth={2} />
            Add project
          </Button>
          <Button
            variant="outline"
            size="lg"
            onClick={scanForProjects}
            disabled={scanning}
          >
            <Search className="h-4 w-4" strokeWidth={2} />
            {scanning ? "Scanning…" : "Scan"}
          </Button>
        </div>
      )}

      {/* Quick-jump chip strip — only when there are enough corpora to benefit */}
      {corpora.length > 6 && (
        <div className="flex gap-2 overflow-x-auto pb-1 -mx-1 px-1">
          {corpora.map((c) => (
            <CorpusChip
              key={c.id}
              corpus={c}
              selected={selectedId === c.id}
              onClick={() => onSelect(c.id)}
            />
          ))}
        </div>
      )}

      {corpora.length === 0 ? (
        <EmptyState
          accent
          icon={FolderOpen}
          title="No projects yet"
          hint={
            <>
              Add a directory containing an{" "}
              <span className="font-mono not-italic">.ministr.toml</span>, or
              point ministr at any folder.
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
                {scanning ? "Scanning…" : "Scan"}
              </Button>
            </div>
          }
        />
      ) : (
        <div className="space-y-2.5">
          {corpora.map((corpus) => {
            const isSelected = selectedId === corpus.id;
            const indexing =
              corpus.status.state === "indexing" ? corpus.status : null;
            const live = progress[corpus.id];
            const eta = indexing ? etaFor(corpus.id) : null;
            const lastIndexed = corpus.last_indexed;
            const indexedAge = lastIndexed
              ? Math.floor(Date.now() / 1000) - lastIndexed
              : null;
            const stale = indexedAge !== null && indexedAge > 7 * 24 * 3600;
            const { variant: statusVariant, label: statusLabel } = statusBadge(
              corpus.status,
            );
            return (
              <Card
                key={corpus.id}
                hover="lift"
                ref={(el) => {
                  if (el) cardRefs.current.set(corpus.id, el);
                  else cardRefs.current.delete(corpus.id);
                }}
                className={cn(
                  "group cursor-pointer p-4",
                  isSelected &&
                    "border-accent shadow-[var(--shadow-sm)]",
                )}
                onClick={() => onSelect(corpus.id)}
              >
                {/* Top line: NAME + STATUS — the anchor */}
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 flex-wrap min-w-0">
                    <span className="font-mono text-base font-bold tracking-[0.05em] text-text truncate">
                      {corpusLabel(corpus)}
                    </span>
                    <Badge variant={statusVariant} dot>
                      {statusLabel}
                    </Badge>
                    {corpus.active_sessions > 0 && (
                      <Badge variant="default" dot>
                        {corpus.active_sessions} SESSION
                        {corpus.active_sessions !== 1 ? "S" : ""}
                      </Badge>
                    )}
                  </div>
                  {/* Hover actions on the right edge */}
                  <div className="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-none">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmReindex(corpus);
                      }}
                      title="Re-index"
                    >
                      <RefreshCw className="h-3.5 w-3.5" strokeWidth={2.5} />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmRemove(corpus);
                      }}
                      title="Remove"
                      className="hover:text-danger"
                    >
                      <Trash2 className="h-3.5 w-3.5" strokeWidth={2.5} />
                    </Button>
                  </div>
                </div>

                {/* Second line: PATH */}
                <p className="text-[0.6875rem] text-text-dim font-mono truncate mt-1">
                  {corpusRoot(corpus.paths)}
                </p>

                {/* Third line: METRIC CHIPS + LAST INDEXED */}
                <div className="flex flex-wrap gap-x-4 gap-y-1.5 mt-2 text-xs text-text-muted">
                  <MetricTile
                    variant="inline"
                    icon={FileText}
                    value={corpus.files_indexed.toLocaleString()}
                    label="files"
                  />
                  <MetricTile
                    variant="inline"
                    icon={Layers}
                    value={corpus.sections_count.toLocaleString()}
                    label="sections"
                  />
                  <MetricTile
                    variant="inline"
                    icon={Code2}
                    value={(corpus.symbols_count ?? 0).toLocaleString()}
                    label="symbols"
                  />
                  <MetricTile
                    variant="inline"
                    icon={Box}
                    value={corpus.embeddings_count.toLocaleString()}
                    label="vectors"
                  />
                  {lastIndexed && (
                    <span
                      className={cn(
                        "flex items-center gap-1 font-mono uppercase tracking-[0.05em] text-xs",
                        stale ? "text-warning" : "text-text-dim",
                      )}
                      title={new Date(lastIndexed * 1000).toLocaleString()}
                    >
                      <Clock className="h-3 w-3" strokeWidth={2.5} />
                      LAST INDEXED · {formatRelativeTime(lastIndexed)}
                    </span>
                  )}
                </div>

                {/* Fourth line: PROGRESS + ETA — only while indexing */}
                {indexing && (
                  <div className="mt-3">
                    <div className="flex justify-between text-xs font-mono uppercase tracking-[0.05em] text-warning mb-1.5">
                      <span className="flex items-center gap-1.5">
                        <span className="h-1.5 w-1.5 bg-warning ministr-blink" />
                        INDEXING
                        {live && live.phase && (
                          <span className="text-text-dim">· {live.phase}</span>
                        )}
                      </span>
                      <span className="tabular-nums">
                        {(live?.files_done ?? indexing.files_done).toLocaleString()}{" "}
                        /{" "}
                        {(live?.files_total ?? indexing.files_total).toLocaleString()}{" "}
                        FILES
                        {eta && <span className="ml-2">· {eta}</span>}
                      </span>
                    </div>
                    <Progress
                      tone="warning"
                      value={
                        indexing.files_total > 0
                          ? ((live?.files_done ?? indexing.files_done) /
                              (live?.files_total ?? indexing.files_total)) *
                            100
                          : 0
                      }
                    />
                  </div>
                )}

                {corpus.status.state === "error" && (
                  <p className="mt-3 text-xs text-danger flex items-start gap-1.5 font-mono">
                    <AlertTriangle
                      className="h-3.5 w-3.5 shrink-0 mt-0.5"
                      strokeWidth={2.5}
                    />
                    {corpus.status.message}
                  </p>
                )}
              </Card>
            );
          })}
        </div>
      )}

      {confirmRemove && (
        <RemoveConfirmModal
          corpus={confirmRemove}
          onCancel={() => setConfirmRemove(null)}
          onConfirm={performRemove}
        />
      )}
      {confirmReindex && (
        <ReindexConfirmModal
          corpus={confirmReindex}
          onCancel={() => setConfirmReindex(null)}
          onConfirm={performReindex}
        />
      )}
    </div>
  );
}

// ─── CONFIRM MODALS ────────────────────────────────────────────────────────

function ReindexConfirmModal({
  corpus,
  onCancel,
  onConfirm,
}: {
  corpus: CorpusInfo;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <ModalShell title="RE-INDEX" onClose={onCancel}>
      <p className="font-mono text-xs text-text leading-relaxed">
        This drops the existing index for{" "}
        <span className="font-bold">{corpusLabel(corpus)}</span> and
        starts over.
      </p>
      <p className="font-mono text-xs uppercase tracking-[0.05em] text-text-dim mt-2">
        {corpus.files_indexed.toLocaleString()} FILES · {" "}
        {corpus.sections_count.toLocaleString()} SECTIONS
      </p>
      <div className="flex items-center gap-2 mt-4 justify-end">
        <Button variant="outline" size="sm" onClick={onCancel}>
          CANCEL
        </Button>
        <Button size="sm" onClick={onConfirm}>
          RE-INDEX
        </Button>
      </div>
    </ModalShell>
  );
}

function RemoveConfirmModal({
  corpus,
  onCancel,
  onConfirm,
}: {
  corpus: CorpusInfo;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  const expected = corpusLabel(corpus);
  const match = typed.trim() === expected;
  return (
    <ModalShell title="REMOVE PROJECT" onClose={onCancel} tone="danger">
      <p className="font-mono text-xs text-text leading-relaxed">
        This permanently removes{" "}
        <span className="font-bold uppercase">{expected}</span> from the
        registry, including all indexed sections and symbols.
      </p>
      <p className="font-sans text-xs tracking-[0.05em] text-text-dim mt-2">
        Source files on disk are not touched.
      </p>
      <div className="mt-4">
        <label className="font-sans text-xs tracking-[0.05em] text-text-dim block mb-1">
          Type the corpus name to confirm
        </label>
        <input
          autoFocus
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={expected}
          className="h-9 w-full border border-border-soft bg-surface px-2 text-xs font-mono uppercase text-text placeholder:text-text-dim focus:outline-none focus:bg-surface-overlay transition-none"
        />
      </div>
      <div className="flex items-center gap-2 mt-4 justify-end">
        <Button variant="outline" size="sm" onClick={onCancel}>
          CANCEL
        </Button>
        <Button
          variant="danger"
          size="sm"
          onClick={onConfirm}
          disabled={!match}
        >
          REMOVE
        </Button>
      </div>
    </ModalShell>
  );
}

function ModalShell({
  title,
  tone,
  onClose,
  children,
}: {
  title: string;
  tone?: "danger";
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div
      className="fixed inset-0 z-[1100] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "20vh" }}
      role="dialog"
      aria-modal="true"
      onClick={onClose}
    >
      <div
        className={cn(
          "w-full max-w-md border-2 bg-surface shadow-[6px_6px_0_0_var(--shadow-color)]",
          tone === "danger" ? "border-danger" : "border-border",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className={cn(
            "flex items-center justify-between border-b-2 bg-surface-overlay px-3 py-2",
            tone === "danger" ? "border-danger" : "border-border",
          )}
        >
          <span
            className={cn(
              "font-mono text-[0.6875rem] font-bold uppercase tracking-[0.05em]",
              tone === "danger" ? "text-danger" : "text-text",
            )}
          >
            {title}
          </span>
          <button
            onClick={onClose}
            aria-label="Close"
            className="grid h-6 w-6 place-items-center border-2 border-border hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
          >
            <X className="h-3 w-3" strokeWidth={2.5} />
          </button>
        </div>
        <div className="p-4">{children}</div>
      </div>
    </div>
  );
}

/** Format a Unix timestamp as a human-readable relative time string. */
function formatRelativeTime(unixSeconds: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSeconds;
  if (diff < 60) return "JUST NOW";
  if (diff < 3600) return `${Math.floor(diff / 60)}M AGO`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}H AGO`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}D AGO`;
  return new Date(unixSeconds * 1000).toLocaleDateString();
}
