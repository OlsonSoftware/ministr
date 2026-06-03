/**
 * TendSurface — the project's CARE facet (AAA-VISION OOUX).
 *
 * Tend is the verb "look after this project": its freshness/health, the
 * per-project embedding config, the indexed paths, a one-keystroke reindex,
 * and its sharing attribute. It is ALWAYS scoped to the spine (the one
 * selected project) — it never re-picks a corpus, satisfying the integration
 * test "one context" (AAA-VISION DoD #1).
 *
 * Built from scratch on the v4 design tokens + ui/ atoms (Card, Badge,
 * Button, MetricTile, ContentTray, ConfirmDialog) — it is NOT a re-skin of
 * ProjectsSurface's master-detail nor SettingsSurface's sidebar. The
 * per-corpus config writes the SAME `.ministr.toml` [corpus] seam the CLI +
 * daemon read (via the `set_corpus_config` command, which reindexes).
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Box,
  Clock,
  Cloud,
  Code2,
  FolderOpen,
  Lock,
  RefreshCw,
  Sprout,
  Zap,
} from "lucide-react";

import { useWorkspace } from "../workspace/WorkspaceContext";
import type { CorpusInfo } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { formatRelativeTime } from "../../lib/format";
import { corpusStatusBadge } from "../../lib/status";
import { cn } from "../../lib/utils";

import { AdaptiveSurface } from "../ui/adaptive-surface";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { ContentTray } from "../ui/content-tray";
import { EmptyState } from "../ui/empty-state";
import { MetricTile } from "../ui/metric-tile";
import { useToast } from "../shell/ToastTray";

interface Props {
  /** Re-fetch daemon status after a mutation (reindex / config save). */
  onRefresh: () => void;
}

export function TendSurface({ onRefresh }: Props) {
  const { activeProject, isFleet } = useWorkspace();

  // Tend is project-scoped care. On the Fleet (no spine project) there is
  // nothing to tend — point the user back at a project.
  if (isFleet || !activeProject) {
    return (
      <AdaptiveSurface>
        <div className="h-full grid place-items-center min-h-0 p-6">
          <EmptyState
            icon={Sprout}
            title="Tend a project"
            hint="Pick a project from the spine to care for its index — health, config, paths, and sharing."
          />
        </div>
      </AdaptiveSurface>
    );
  }

  return <TendBody corpus={activeProject} onRefresh={onRefresh} />;
}

function TendBody({
  corpus,
  onRefresh,
}: {
  corpus: CorpusInfo;
  onRefresh: () => void;
}) {
  const { toast } = useToast();
  const [confirmReindex, setConfirmReindex] = useState(false);
  const { variant: statusVariant, label: statusLabel } =
    corpusStatusBadge(corpus);

  const indexing =
    corpus.status.state === "indexing" ? corpus.status : null;

  async function performReindex() {
    setConfirmReindex(false);
    try {
      await invoke("trigger_reindex", { corpusId: corpus.id });
      toast("Re-indexing started", { tone: "info" });
      onRefresh();
    } catch (e) {
      toast("Re-index failed", { detail: String(e), tone: "danger" });
    }
  }

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col min-h-0">
        {/* ── Care toolbar — status + the one-keystroke action. Identity +
            size stats already live in the shell ScopeHeader above, so Tend
            never re-renders them (OOUX: one context, no duplicate render). ── */}
        <header className="flex items-center justify-between gap-4 px-5 pt-5 pb-3 shrink-0">
          <div className="flex items-center gap-2">
            <Badge variant={statusVariant} dot>
              {statusLabel}
            </Badge>
            {corpus.active_sessions > 0 && (
              <Badge variant="default" dot>
                {corpus.active_sessions} live
              </Badge>
            )}
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirmReindex(true)}
            disabled={!!indexing}
            className="shrink-0"
          >
            <RefreshCw
              className={cn("h-3.5 w-3.5", indexing && "animate-spin")}
              strokeWidth={2}
            />
            {indexing ? "Indexing…" : "Re-index"}
          </Button>
        </header>

        <div className="flex-1 min-h-0 overflow-y-auto px-5 pb-6 space-y-4">
          {/* Live indexing progress — a fresh inline bar derived straight
              from corpus.status (no separate renderer). */}
          {indexing && (
            <ContentTray compact>
              <ReindexProgress
                done={indexing.files_done}
                total={indexing.files_total}
              />
            </ContentTray>
          )}

          {/* ── Health & freshness — the CARE lens (freshness, embedding
              coverage, live load) — complements, never repeats, the header's
              size stats (files/sections/symbols). ─────────────────────────── */}
          <CareSection icon={Sprout} title="Health">
            <div className="grid grid-cols-3 gap-2">
              <MetricTile
                icon={Clock}
                value={
                  corpus.last_indexed
                    ? formatRelativeTime(corpus.last_indexed)
                    : "Never"
                }
                label="Last indexed"
              />
              <MetricTile
                icon={Box}
                value={corpus.embeddings_count.toLocaleString()}
                label="Vectors"
              />
              <MetricTile
                icon={Zap}
                value={corpus.active_sessions.toLocaleString()}
                label="Live agents"
              />
            </div>
          </CareSection>

          {/* ── Per-project embedding config ────────────────────────────── */}
          <CareSection icon={Code2} title="Embedding">
            <EmbeddingConfig corpus={corpus} onSaved={onRefresh} />
          </CareSection>

          {/* ── Indexed paths ───────────────────────────────────────────── */}
          <CareSection icon={FolderOpen} title="Paths">
            <ul className="space-y-1">
              {corpus.paths.map((p) => (
                <li
                  key={p}
                  className="font-mono text-xs text-text-muted truncate rounded-md border border-border-soft bg-surface px-2.5 py-1.5"
                >
                  {p}
                </li>
              ))}
            </ul>
          </CareSection>

          {/* ── Sharing attribute ───────────────────────────────────────── */}
          <CareSection icon={Cloud} title="Sharing">
            <SharingAttribute />
          </CareSection>
        </div>
      </div>

      <ConfirmDialog
        open={confirmReindex}
        title="Re-index project"
        body={
          <>
            <p>
              This drops the existing index for{" "}
              <span className="font-bold">{corpusLabel(corpus)}</span> and
              starts over.
            </p>
            <p className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim mt-2">
              {corpus.files_indexed.toLocaleString()} files ·{" "}
              {corpus.sections_count.toLocaleString()} sections
            </p>
          </>
        }
        confirmLabel="Re-index"
        onCancel={() => setConfirmReindex(false)}
        onConfirm={performReindex}
      />
    </AdaptiveSurface>
  );
}

/** A labelled care section — the repeated shell each Tend concern renders in. */
function CareSection({
  icon: Icon,
  title,
  children,
}: {
  icon: typeof Sprout;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="p-4 space-y-3">
      <div className="flex items-center gap-2">
        <Icon className="h-3.5 w-3.5 text-accent" strokeWidth={2.25} />
        <h2 className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
          {title}
        </h2>
      </div>
      {children}
    </Card>
  );
}

/** A fresh determinate progress bar driven by the indexing file counts. */
function ReindexProgress({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
        <span>Indexing</span>
        <span className="tabular-nums text-text">
          {done.toLocaleString()} / {total.toLocaleString()}
        </span>
      </div>
      <div
        className="h-1.5 rounded-full bg-surface-overlay overflow-hidden"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-300 ease-out"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Per-corpus embedding config — fresh form over the set_corpus_config seam.

interface SupportedModel {
  name: string;
  dimension: number;
  description: string;
  code_optimized: boolean;
}

/** Blank → null (leave the knob untouched); invalid → null (never write
 *  garbage) — mirrors `RepoConfig::set_corpus_config`'s `None` semantics. */
function parseKnob(raw: string): number | null {
  const t = raw.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isInteger(n) && n >= 0 ? n : null;
}

function EmbeddingConfig({
  corpus,
  onSaved,
}: {
  corpus: CorpusInfo;
  onSaved: () => void;
}) {
  const [models, setModels] = useState<SupportedModel[] | null>(null);
  const [model, setModel] = useState(corpus.model ?? "");
  const [dimension, setDimension] = useState("");
  const [rerankDepth, setRerankDepth] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<SupportedModel[]>("list_supported_models")
      .then((m) => !cancelled && setModels(m))
      .catch((e) => {
        console.error("[ministr] list_supported_models error:", e);
        if (!cancelled) setModels([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-sync if the corpus's effective model changes under us (e.g. a reindex
  // completed and the status poll refreshed the spine).
  useEffect(() => {
    setModel(corpus.model ?? "");
  }, [corpus.model]);

  const knownModel = useMemo(
    () => models?.some((m) => m.name === model) ?? false,
    [models, model],
  );

  const dirty =
    model.trim() !== (corpus.model ?? "").trim() ||
    dimension.trim() !== "" ||
    rerankDepth.trim() !== "";

  const inputCls =
    "h-8 px-2 rounded-md border border-border bg-surface font-mono text-xs text-text focus:outline-none focus:border-border-hover";

  async function save() {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      await invoke("set_corpus_config", {
        corpusId: corpus.id,
        model: model.trim() === "" ? null : model.trim(),
        dimension: parseKnob(dimension),
        rerankDepth: parseKnob(rerankDepth),
      });
      setSaved(true);
      setDimension("");
      setRerankDepth("");
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-2.5">
      <label className="flex flex-col gap-1">
        <span className="font-mono text-xs text-text-dim">model</span>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          disabled={saving}
          className={cn(inputCls, "cursor-pointer")}
        >
          <option value="">(daemon default)</option>
          {model && !knownModel ? <option value={model}>{model}</option> : null}
          {(models ?? []).map((m) => (
            <option key={m.name} value={m.name}>
              {m.name}
              {m.code_optimized ? " · code" : ""} ({m.dimension}d)
            </option>
          ))}
        </select>
      </label>

      <div className="grid grid-cols-2 gap-2">
        <label className="flex flex-col gap-1">
          <span className="font-mono text-xs text-text-dim">dimension</span>
          <input
            inputMode="numeric"
            value={dimension}
            onChange={(e) => setDimension(e.target.value)}
            disabled={saving}
            placeholder="full"
            className={inputCls}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="font-mono text-xs text-text-dim">rerank_depth</span>
          <input
            inputMode="numeric"
            value={rerankDepth}
            onChange={(e) => setRerankDepth(e.target.value)}
            disabled={saving}
            placeholder="100"
            className={inputCls}
          />
        </label>
      </div>

      <div className="flex items-center gap-2 pt-0.5">
        <Button size="sm" disabled={saving || !dirty} onClick={save}>
          {saving ? "Reindexing…" : "Save + reindex"}
        </Button>
        {saved && (
          <span className="font-mono text-xs text-text-dim">
            saved — reindexing
          </span>
        )}
        {error && (
          <span className="font-mono text-xs text-danger break-all">
            {error}
          </span>
        )}
      </div>

      <p className="font-mono text-[10px] leading-snug text-text-dim">
        Writes <code>.ministr.toml</code> [corpus] and re-indexes.
        dimension/rerank_depth only apply to Matryoshka-capable models.
      </p>
    </div>
  );
}

/** Sharing is a project ATTRIBUTE, not a destination. Until a cloud account is
 *  connected (Account area — aaa-cloud), every project is local-only; this
 *  states that honestly rather than faking a sync control. */
function SharingAttribute() {
  return (
    <div className="flex items-start gap-3 rounded-md border border-border-soft bg-surface px-3 py-2.5">
      <span
        className="grid place-items-center h-7 w-7 shrink-0 rounded-md border border-border bg-surface-overlay text-text-dim"
        aria-hidden
      >
        <Lock className="h-3.5 w-3.5" strokeWidth={2} />
      </span>
      <div className="min-w-0">
        <p className="font-sans text-sm font-medium text-text">Local only</p>
        <p className="font-sans text-xs text-text-dim mt-0.5">
          This index lives on this machine. Connect a cloud account to sync or
          share it — managed from Account.
        </p>
      </div>
    </div>
  );
}
