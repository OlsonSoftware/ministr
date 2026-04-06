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
import { Progress } from "./ui/progress";
import { useState } from "react";

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
      console.log("[iris] remove_project", corpusId);
      await invoke("remove_project", { corpusId });
      console.log("[iris] remove_project OK, refreshing...");
      await onRefresh();
      console.log("[iris] refresh after remove OK");
    } catch (err) {
      console.error("[iris] remove_project error:", err);
    }
  }

  async function reindex(e: React.MouseEvent, corpusId: string) {
    e.stopPropagation();
    try {
      console.log("[iris] trigger_reindex", corpusId);
      await invoke("trigger_reindex", { corpusId });
      console.log("[iris] trigger_reindex OK, refreshing...");
      await onRefresh();
      console.log("[iris] refresh after reindex OK");
    } catch (err) {
      console.error("[iris] trigger_reindex error:", err);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
          Projects ({corpora.length})
        </h2>
        <Button size="sm" onClick={addProject} disabled={adding}>
          <Plus className="mr-1 h-3.5 w-3.5" />
          Add
        </Button>
      </div>

      {corpora.length === 0 ? (
        <Card className="text-center py-8">
          <FolderOpen className="mx-auto h-8 w-8 text-text-dim mb-2" />
          <p className="text-text-muted text-sm">No projects registered</p>
          <p className="text-text-dim text-xs mt-1">
            Click "Add" or use the tray menu
          </p>
        </Card>
      ) : (
        corpora.map((corpus) => (
          <Card
            key={corpus.id}
            className={`cursor-pointer transition-colors hover:border-border-hover ${
              selectedId === corpus.id ? "border-accent/40" : ""
            }`}
            onClick={() => onSelect(corpus.id)}
          >
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-medium text-sm truncate">
                    {projectName(corpus.paths)}
                  </span>
                  {statusBadge(corpus.status)}
                </div>
                <p className="text-xs text-text-dim font-mono truncate">
                  {projectRoot(corpus.paths)}
                </p>
              </div>
              <div className="flex items-center gap-1 shrink-0">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={(e) => reindex(e, corpus.id)}
                  title="Re-index"
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="danger"
                  size="sm"
                  onClick={(e) => removeProject(e, corpus.id)}
                  title="Remove"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>

            <div className="flex flex-wrap gap-x-4 gap-y-1 mt-2 text-xs text-text-dim">
              <span className="flex items-center gap-1">
                <FileText className="h-3 w-3" /> {corpus.files_indexed} files
              </span>
              <span className="flex items-center gap-1">
                <Layers className="h-3 w-3" /> {corpus.sections_count} sections
              </span>
              <span className="flex items-center gap-1">
                <Code2 className="h-3 w-3" /> {corpus.symbols_count ?? 0} symbols
              </span>
              <span className="flex items-center gap-1">
                <Box className="h-3 w-3" /> {corpus.embeddings_count} vectors
              </span>
              {corpus.active_sessions > 0 && (
                <span className="flex items-center gap-1 text-accent">
                  <Users className="h-3 w-3" /> {corpus.active_sessions} {corpus.active_sessions === 1 ? "session" : "sessions"}
                </span>
              )}
              {corpus.last_indexed && (
                <span className="flex items-center gap-1" title={new Date(corpus.last_indexed * 1000).toLocaleString()}>
                  <Clock className="h-3 w-3" /> {formatRelativeTime(corpus.last_indexed)}
                </span>
              )}
            </div>

            {corpus.status.state === "indexing" && (
              <div className="mt-2">
                <div className="flex justify-between text-xs text-warning mb-1">
                  <span>Indexing...</span>
                  <span>
                    {corpus.status.files_done}/{corpus.status.files_total}
                  </span>
                </div>
                <Progress
                  value={
                    corpus.status.files_total > 0
                      ? (corpus.status.files_done / corpus.status.files_total) * 100
                      : 0
                  }
                />
              </div>
            )}

            {corpus.status.state === "error" && (
              <p className="mt-2 text-xs text-danger">{corpus.status.message}</p>
            )}
          </Card>
        ))
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
