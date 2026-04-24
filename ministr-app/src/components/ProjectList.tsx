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
import type { CorpusInfo, IndexingStatus } from "../lib/types";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Card } from "./ui/card";
import { CorpusChip } from "./ui/corpus-chip";
import { Progress } from "./ui/progress";
import { cn } from "../lib/utils";
import { useEffect, useRef, useState } from "react";

interface ProjectListProps {
  corpora: CorpusInfo[];
  onRefresh: () => void;
  onSelect: (id: string) => void;
  selectedId: string | null;
}

function statusBadge(status: IndexingStatus) {
  switch (status.state) {
    case "idle":
      return <Badge variant="success">Ready</Badge>;
    case "indexing":
      return <Badge variant="warning">Indexing</Badge>;
    case "error":
      return <Badge variant="danger">Error</Badge>;
  }
}

function projectName(paths: string[]): string {
  if (paths.length === 0) return "Unknown";
  const root = projectRoot(paths);
  const parts = root.split("/");
  return parts[parts.length - 1] || root;
}

/** Derive the project root directory from corpus paths. */
function projectRoot(paths: string[]): string {
  if (paths.length === 0) return "";
  if (paths.length === 1) {
    // Single path like /Users/x/project/src → go up to /Users/x/project
    const parts = paths[0].split("/");
    return parts.slice(0, -1).join("/") || paths[0];
  }
  // Multi-path: find common ancestor directory.
  const segments = paths.map((p) => p.split("/"));
  let common = 0;
  outer: for (let i = 0; i < segments[0].length; i++) {
    for (let j = 1; j < segments.length; j++) {
      if (i >= segments[j].length || segments[j][i] !== segments[0][i]) break outer;
    }
    common = i + 1;
  }
  return segments[0].slice(0, common).join("/");
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
        <Card className="flex flex-col items-center justify-center text-center py-12 px-6">
          <div className="grid h-14 w-14 place-items-center rounded-xl bg-[var(--color-accent-soft)] text-accent mb-4">
            <FolderOpen className="h-6 w-6" />
          </div>
          <p className="text-sm font-medium text-text">No projects yet</p>
          <p className="text-xs text-text-dim mt-1 max-w-xs">
            Add a directory containing an <span className="font-mono">.ministr.toml</span>,
            or point ministr at any folder and it will scan for you.
          </p>
          <Button className="mt-5" onClick={addProject} disabled={adding}>
            <Plus className="h-3.5 w-3.5" />
            Add your first project
          </Button>
        </Card>
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
                  isSelected &&
                    "border-[var(--color-accent-ring)] shadow-[0_0_0_3px_var(--color-accent-soft)]",
                )}
                onClick={() => onSelect(corpus.id)}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-semibold text-sm text-text truncate">
                        {projectName(corpus.paths)}
                      </span>
                      {statusBadge(corpus.status)}
                      {corpus.active_sessions > 0 && (
                        <Badge variant="default" dot>
                          {corpus.active_sessions}{" "}
                          {corpus.active_sessions === 1 ? "session" : "sessions"}
                        </Badge>
                      )}
                    </div>
                    <p className="text-xs text-text-dim font-mono truncate mt-0.5">
                      {projectRoot(corpus.paths)}
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
                  <Stat icon={FileText} value={corpus.files_indexed} label="files" />
                  <Stat icon={Layers} value={corpus.sections_count} label="sections" />
                  <Stat icon={Code2} value={corpus.symbols_count ?? 0} label="symbols" />
                  <Stat icon={Box} value={corpus.embeddings_count} label="vectors" />
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
                        <span className="ministr-pulse h-1.5 w-1.5 rounded-full bg-warning" />
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

function Stat({
  icon: Icon,
  value,
  label,
}: {
  icon: React.ComponentType<{ className?: string }>;
  value: number;
  label: string;
}) {
  return (
    <span className="flex items-center gap-1 text-text-muted">
      <Icon className="h-3 w-3 text-text-dim" />
      <span className="tabular-nums font-medium">{value.toLocaleString()}</span>
      <span className="text-text-dim">{label}</span>
    </span>
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
