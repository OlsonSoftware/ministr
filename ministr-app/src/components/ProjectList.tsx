import { invoke } from "@tauri-apps/api/core";
import {
  FolderOpen,
  Trash2,
  RefreshCw,
  FileText,
  Layers,
  Box,
  Plus,
  Users,
  Code2,
  Clock,
} from "lucide-react";
import type { CorpusInfo } from "../lib/types";
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

export function ProjectList({ corpora, onRefresh, onSelect, selectedId }: ProjectListProps) {
  const [adding, setAdding] = useState(false);
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());

  // Scroll the selected corpus card into view when it changes (e.g. from
  // the chip strip or the tray menu).
  useEffect(() => {
    if (!selectedId) return;
    const el = cardRefs.current.get(selectedId);
    if (el) el.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [selectedId]);

  async function addProject() {
    setAdding(true);
    try {
      await invoke("add_project_dialog");
      onRefresh();
    } finally {
      setAdding(false);
    }
  }

  async function removeProject(e: React.MouseEvent, corpusId: string) {
    e.stopPropagation();
    try {
      console.log("[ministr] remove_project", corpusId);
      await invoke("remove_project", { corpusId });
      console.log("[ministr] remove_project OK, refreshing...");
      await onRefresh();
      console.log("[ministr] refresh after remove OK");
    } catch (err) {
      console.error("[ministr] remove_project error:", err);
    }
  }

  async function reindex(e: React.MouseEvent, corpusId: string) {
    e.stopPropagation();
    try {
      console.log("[ministr] trigger_reindex", corpusId);
      await invoke("trigger_reindex", { corpusId });
      console.log("[ministr] trigger_reindex OK, refreshing...");
      await onRefresh();
      console.log("[ministr] refresh after reindex OK");
    } catch (err) {
      console.error("[ministr] trigger_reindex error:", err);
    }
  }

  return (
    <div className="space-y-4 ministr-fade-in">
      <div className="flex items-end justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-text">Projects</h2>
          <p className="text-xs text-text-dim mt-0.5">
            {corpora.length === 0
              ? "None registered yet"
              : `${corpora.length} ${corpora.length === 1 ? "corpus" : "corpora"} indexed`}
          </p>
        </div>
        <Button size="sm" onClick={addProject} disabled={adding}>
          <Plus className="h-3.5 w-3.5" />
          Add project
        </Button>
      </div>

      {/* Quick-jump chip strip — only shows when there are enough
          corpora to benefit from it. */}
      {corpora.length > 1 && (
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
              <span className="font-mono">.ministr.toml</span>, or point ministr
              at any folder and it will scan for you.
            </>
          }
          action={
            <Button onClick={addProject} disabled={adding}>
              <Plus className="h-3.5 w-3.5" />
              Add your first project
            </Button>
          }
        />
      ) : (
        <div className="space-y-2.5">
          {corpora.map((corpus) => {
            const isSelected = selectedId === corpus.id;
            const indexing =
              corpus.status.state === "indexing" ? corpus.status : null;
            return (
              <Card
                key={corpus.id}
                hover="lift"
                ref={(el) => {
                  if (el) cardRefs.current.set(corpus.id, el);
                  else cardRefs.current.delete(corpus.id);
                }}
                className={cn(
                  "cursor-pointer p-4 ministr-fade-in",
                  isSelected && "border-[var(--color-accent-ring)]",
                )}
                onClick={() => onSelect(corpus.id)}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-semibold text-sm text-text truncate">
                        {corpusLabel(corpus)}
                      </span>
                      {(() => {
                        const { variant, label } = statusBadge(corpus.status);
                        return <Badge variant={variant}>{label}</Badge>;
                      })()}
                      {corpus.active_sessions > 0 && (
                        <Badge variant="default" dot>
                          {corpus.active_sessions}{" "}
                          {corpus.active_sessions === 1 ? "session" : "sessions"}
                        </Badge>
                      )}
                    </div>
                    <p className="text-xs text-text-dim font-mono truncate mt-0.5">
                      {corpusRoot(corpus.paths)}
                    </p>
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={(e) => reindex(e, corpus.id)}
                      title="Re-index"
                    >
                      <RefreshCw className="h-3.5 w-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={(e) => removeProject(e, corpus.id)}
                      title="Remove"
                      className="hover:text-danger"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </div>

                <div className="flex flex-wrap gap-x-4 gap-y-1.5 mt-3 text-xs text-text-muted">
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
                  {corpus.last_indexed && (
                    <span
                      className="flex items-center gap-1 text-text-dim"
                      title={new Date(corpus.last_indexed * 1000).toLocaleString()}
                    >
                      <Clock className="h-3 w-3" />
                      {formatRelativeTime(corpus.last_indexed)}
                    </span>
                  )}
                </div>

                {indexing && (
                  <div className="mt-3">
                    <div className="flex justify-between text-[11px] text-warning mb-1.5">
                      <span className="flex items-center gap-1.5">
                        <span className="h-1.5 w-1.5 rounded-full bg-warning" />
                        Indexing
                      </span>
                      <span className="font-mono tabular-nums">
                        {indexing.files_done} / {indexing.files_total}
                      </span>
                    </div>
                    <Progress
                      glow
                      value={
                        indexing.files_total > 0
                          ? (indexing.files_done / indexing.files_total) * 100
                          : 0
                      }
                    />
                  </div>
                )}

                {corpus.status.state === "error" && (
                  <p className="mt-3 text-xs text-danger flex items-start gap-1.5">
                    <span className="mt-0.5 inline-block h-1.5 w-1.5 rounded-full bg-danger shrink-0" />
                    {corpus.status.message}
                  </p>
                )}
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Format a Unix timestamp as a human-readable relative time string. */
function formatRelativeTime(unixSeconds: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSeconds;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return new Date(unixSeconds * 1000).toLocaleDateString();
}
