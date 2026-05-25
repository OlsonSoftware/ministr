/**
 * LinkedProjectsPanel — manage the current project's linked projects.
 *
 * A "linked project" is another codebase an AI agent working in *this*
 * project can also query in the same session — without it, each project's
 * index is siloed. Links are stored as `[[linked]]` entries in this
 * project's `.ministr.toml` (format-preserving), so they're version
 * controlled and travel with the repo.
 *
 * The agent discovers links through the `ministr_projects` MCP tool and
 * targets one by passing its label as the `project` argument to any
 * query tool. The agent can only ever reach a project a human linked
 * here — never an arbitrary path.
 */
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, FolderPlus, Link2, Loader2, RefreshCw, Trash2 } from "lucide-react";

import type { CorpusInfo } from "../../lib/types";
import { corpusRoot } from "../../lib/corpus";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { ContentTray } from "../ui/content-tray";

interface LinkedProjectOut {
  path: string;
  label: string | null;
  resolved_label: string;
  exists: boolean;
}

interface Props {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
}

export function LinkedProjectsPanel({ corpora, activeCorpusId }: Props) {
  const corpus =
    corpora.find((c) => c.id === activeCorpusId) ?? corpora[0] ?? null;
  const projectRoot = corpus ? corpusRoot(corpus.paths) : null;

  const [links, setLinks] = useState<LinkedProjectOut[]>([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!projectRoot) return;
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<LinkedProjectOut[]>("linked_projects_list", {
        projectRoot,
      });
      setLinks(result);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, [projectRoot]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const addViaDialog = useCallback(async () => {
    if (!projectRoot) return;
    setBusy("__add__");
    setError(null);
    try {
      const added = await invoke<LinkedProjectOut | null>(
        "linked_project_add_dialog",
        { projectRoot },
      );
      if (added) await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(null);
    }
  }, [projectRoot, refresh]);

  const remove = useCallback(
    async (path: string) => {
      if (!projectRoot) return;
      setBusy(path);
      setError(null);
      try {
        await invoke<boolean>("linked_project_remove", { projectRoot, path });
        await refresh();
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      } finally {
        setBusy(null);
      }
    },
    [projectRoot, refresh],
  );

  if (!corpus || !projectRoot) {
    return (
      <div className="space-y-4">
        <Header />
        <ContentTray>
          <p className="font-sans text-sm text-text-muted">
            Add a project first — links are stored in a project's{" "}
            <code className="font-mono text-mono-mini">.ministr.toml</code>.
            Visit Projects to add one.
          </p>
        </ContentTray>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <Header
        projectRoot={projectRoot}
        onRefresh={refresh}
        loading={loading}
      />

      {error && (
        <div className="border border-danger bg-surface p-3 flex items-start gap-2">
          <AlertTriangle
            className="h-4 w-4 text-danger shrink-0 mt-0.5"
            strokeWidth={2.5}
          />
          <p className="font-mono text-mono-mini text-danger">{error}</p>
        </div>
      )}

      {links.length === 0 ? (
        <ContentTray className="space-y-1">
          <p className="font-sans text-sm text-text-muted">
            No linked projects. Link another codebase so an agent working
            here can query it in the same session.
          </p>
        </ContentTray>
      ) : (
        <ul className="space-y-2.5">
          {links.map((l) => (
            <li
              key={l.path}
              className={cn(
                "rounded-lg p-4 flex items-start justify-between gap-3 transition-colors duration-150",
                l.exists
                  ? "bg-surface-sunken hover:bg-surface-overlay/50"
                  : "bg-warning/10 border border-warning/30",
              )}
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Link2
                    className="h-4 w-4 text-accent shrink-0"
                    strokeWidth={2}
                  />
                  <span className="font-mono text-sm font-bold tracking-[0.08em] text-text">
                    {l.resolved_label}
                  </span>
                </div>
                <p className="font-mono text-mono-mini text-text-dim mt-1 truncate max-w-[60ch]">
                  {l.path}
                </p>
                {!l.exists && (
                  <p className="font-sans italic text-mono-mini text-warning mt-1">
                    Path not found on disk — the agent can't query it until
                    it exists.
                  </p>
                )}
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => remove(l.path)}
                disabled={busy === l.path}
                aria-label={`Unlink ${l.resolved_label}`}
                className="shrink-0"
              >
                {busy === l.path ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" strokeWidth={2} />
                )}
                Unlink
              </Button>
            </li>
          ))}
        </ul>
      )}

      <Button
        variant="outline"
        size="sm"
        onClick={addViaDialog}
        disabled={busy === "__add__"}
      >
        {busy === "__add__" ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
        ) : (
          <FolderPlus className="h-3.5 w-3.5" strokeWidth={2} />
        )}
        Link a project…
      </Button>
    </div>
  );
}

function Header({
  projectRoot,
  onRefresh,
  loading,
}: {
  projectRoot?: string | null;
  onRefresh?: () => void;
  loading?: boolean;
}) {
  return (
    <header className="flex items-start justify-between gap-3">
      <div className="space-y-1">
        <h2 className="font-mono text-sm font-bold uppercase tracking-[0.08em] text-text">
          Linked projects
        </h2>
        <p className="font-sans text-sm text-text-muted">
          Let an agent working in this project also query another
          codebase in the same session.
        </p>
        {projectRoot && (
          <p className="font-mono text-mono-mini text-text-dim truncate max-w-[60ch]">
            Stored in{" "}
            <span className="text-text">{`${projectRoot}/.ministr.toml`}</span>
          </p>
        )}
      </div>
      {onRefresh && (
        <Button
          variant="outline"
          size="sm"
          onClick={onRefresh}
          disabled={loading}
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
          ) : (
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
          )}
          Refresh
        </Button>
      )}
    </header>
  );
}
