/**
 * FleetSurface — the bespoke COLLECTION view of the Project object (AAA-VISION).
 *
 * The Fleet is not a project manager; it's the zoomed-out control deck for
 * *all* your indexes at once. So it does what a master-detail list cannot —
 * it speaks at the collection level:
 *
 *   · FLEET VITALS  — one aggregate readout (projects · files · vectors ·
 *                     symbols · live agents) — the identity of the whole fleet.
 *   · A SELF-PRIORITIZING CONSTELLATION — project cells auto-sorted by demand
 *                     (live agents → indexing → freshest → biggest), so the
 *                     fleet surfaces what needs you, like a mission board.
 *   · RELATIVE INDEX-MASS — each cell's vectors drawn against the fleet's
 *                     largest index: a cross-project comparison that only
 *                     exists in a collection.
 *   · FRESHNESS HEALTH — an age-toned pip per project so drift is legible
 *                     across the whole fleet at a glance.
 *
 * Click a cell to ZOOM IN — it becomes the spine and its facets appear. Built
 * fresh from v4 tokens + ui/ atoms (Card, Badge, StatusDot, Button, MetricTile,
 * EmptyState, ConfirmDialog) — it is NOT a re-skin of the retired master-detail
 * ProjectsSurface. `FleetDeck` is pure (renders from props for Storybook);
 * `FleetSurface` is the live connector.
 */
import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Boxes,
  Code2,
  Compass,
  FileText,
  FolderOpen,
  LayoutGrid,
  Loader2,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Zap,
} from "@/components/ui/icons";

import type { CorpusInfo } from "../../lib/types";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { formatRelativeTime } from "../../lib/format";
import { type Tone, toneCssVar } from "../../lib/status";
import { cn } from "../../lib/utils";

import { AdaptiveSurface } from "../ui/adaptive-surface";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { EmptyState } from "../ui/empty-state";
import { FacetHeader } from "../ui/facet-header";
import { MetricTile } from "../ui/metric-tile";
import { StatusDot } from "../ui/status-dot";
import { ViewSwitch, type ViewOption } from "../ui/view-switch";
import { VizFrame } from "../ui/viz-frame";
import { useToast } from "../shell/ToastTray";

const DAY = 86_400;

type FleetView = "grid" | "map";

const FLEET_VIEWS: ViewOption<FleetView>[] = [
  { id: "grid", label: "Grid", icon: LayoutGrid, hint: "Manage projects — per-cell actions" },
  { id: "map", label: "Map", icon: Compass, hint: "Star-map — index mass & status at a glance" },
];

interface DeckProps {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  busyAdd?: boolean;
  busyScan?: boolean;
  /** Initial view mode — Storybook renders the star-map directly with "map". */
  initialView?: FleetView;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onScan: () => void;
  onReindex: (corpus: CorpusInfo) => void;
  onRemove: (corpus: CorpusInfo) => void;
}

/** Liveness rank for the self-prioritizing sort — higher floats up. */
function demandRank(c: CorpusInfo): number {
  if (c.status.state === "error") return 4;
  if (c.active_sessions > 0) return 3;
  if (c.status.state === "indexing" || c.status.state === "queued") return 2;
  if (c.warming) return 1;
  return 0;
}

/** Age → freshness tone: today is green, this week accent, this month warns,
 *  older/never goes muted. The fleet's drift, readable at a glance. */
function freshnessTone(lastIndexed?: number): Tone {
  if (!lastIndexed) return "muted";
  const ageDays = (Date.now() / 1000 - lastIndexed) / DAY;
  if (ageDays < 1) return "success";
  if (ageDays < 7) return "accent";
  if (ageDays < 30) return "warning";
  return "muted";
}

export function FleetDeck({
  corpora,
  activeCorpusId,
  busyAdd = false,
  busyScan = false,
  initialView,
  onSelect,
  onAdd,
  onScan,
  onReindex,
  onRemove,
}: DeckProps) {
  const [view, setView] = useState<FleetView>(initialView ?? "grid");
  const vitals = useMemo(() => {
    let files = 0;
    let vectors = 0;
    let symbols = 0;
    let live = 0;
    let maxVectors = 0;
    for (const c of corpora) {
      files += c.files_indexed;
      vectors += c.embeddings_count;
      symbols += c.symbols_count ?? 0;
      live += c.active_sessions;
      if (c.embeddings_count > maxVectors) maxVectors = c.embeddings_count;
    }
    return { files, vectors, symbols, live, maxVectors };
  }, [corpora]);

  // The constellation self-prioritizes: demand first, then freshness, then mass.
  const ordered = useMemo(
    () =>
      [...corpora].sort((a, b) => {
        const d = demandRank(b) - demandRank(a);
        if (d !== 0) return d;
        const fa = a.last_indexed ?? 0;
        const fb = b.last_indexed ?? 0;
        if (fb !== fa) return fb - fa;
        return b.embeddings_count - a.embeddings_count;
      }),
    [corpora],
  );

  if (corpora.length === 0) {
    return (
      <AdaptiveSurface>
        <div className="h-full grid place-items-center min-h-0 p-6">
          <EmptyState
            accent
            icon={FolderOpen}
            title="An empty fleet"
            hint={
              <>
                Point ministr at any folder, or scan{" "}
                <span className="font-mono not-italic">~/Code</span> for projects
                with a <span className="font-mono not-italic">.ministr.toml</span>.
              </>
            }
            action={
              <div className="flex items-center gap-2">
                <Button onClick={onAdd} disabled={busyAdd} size="lg">
                  <Plus className="h-4 w-4" strokeWidth={2} />
                  Add your first project
                </Button>
                <Button
                  variant="outline"
                  size="lg"
                  onClick={onScan}
                  disabled={busyScan}
                >
                  <Search className="h-4 w-4" strokeWidth={2} />
                  {busyScan ? "Scanning…" : "Scan ~/Code"}
                </Button>
              </div>
            }
          />
        </div>
      </AdaptiveSurface>
    );
  }

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col min-h-0">
        {/* ── Fleet vitals — the collection's identity, not any one project. ── */}
        <FacetHeader
          title="Fleet"
          glance={
            <>
              {corpora.length} {corpora.length === 1 ? "project" : "projects"}
              {vitals.live > 0 && (
                <>
                  {" · "}
                  <span className="text-accent">{vitals.live} live</span>
                </>
              )}
            </>
          }
          actions={
            <>
              <ViewSwitch
                value={view}
                onChange={setView}
                options={FLEET_VIEWS}
                ariaLabel="Fleet view"
              />
              <Button variant="outline" size="sm" onClick={onScan} disabled={busyScan}>
                {busyScan ? (
                  <Loader2 className="h-4 w-4 animate-spin" strokeWidth={2} />
                ) : (
                  <Search className="h-4 w-4" strokeWidth={2} />
                )}
                {busyScan ? "Scanning…" : "Scan"}
              </Button>
              <Button size="sm" onClick={onAdd} disabled={busyAdd}>
                <Plus className="h-4 w-4" strokeWidth={2} />
                Add project
              </Button>
            </>
          }
        >
          <div className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-4">
            <MetricTile
              variant="cell"
              className="bg-surface"
              icon={FileText}
              label="Files"
              value={vitals.files.toLocaleString()}
            />
            <MetricTile
              variant="cell"
              className="bg-surface"
              icon={Boxes}
              label="Vectors"
              value={vitals.vectors.toLocaleString()}
            />
            <MetricTile
              variant="cell"
              className="bg-surface"
              icon={Code2}
              label="Symbols"
              value={vitals.symbols.toLocaleString()}
            />
            <MetricTile
              variant="cell"
              className="bg-surface"
              icon={Zap}
              label="Live agents"
              tone={vitals.live > 0 ? "accent" : undefined}
              value={vitals.live.toLocaleString()}
            />
          </div>
        </FacetHeader>

        {/* ── The constellation — self-sorted project instruments. Two reads of
            the same fleet: a packed STAR-MAP (mass + status at a glance, zoom
            on click) or the management GRID (per-project actions). ────────── */}
        {view === "map" ? (
          <div className="flex-1 min-h-0 overflow-y-auto px-5 pb-5">
            <FleetConstellation
              corpora={ordered}
              activeCorpusId={activeCorpusId}
              onSelect={onSelect}
            />
          </div>
        ) : (
          <div className="flex-1 min-h-0 overflow-y-auto px-5 pb-5">
            <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3 items-start">
              {ordered.map((c) => (
                <li key={c.id}>
                  <FleetCell
                    corpus={c}
                    selected={c.id === activeCorpusId}
                    massPct={
                      vitals.maxVectors > 0
                        ? Math.round((c.embeddings_count / vitals.maxVectors) * 100)
                        : 0
                    }
                    onSelect={() => onSelect(c.id)}
                    onReindex={() => onReindex(c)}
                    onRemove={() => onRemove(c)}
                  />
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </AdaptiveSurface>
  );
}

// ── The star-map — projects packed into a constellation ──────────────────────
//
// The Fleet's docstring calls itself a "constellation"; this view makes that
// literal. Each project is a BUBBLE whose area is its index mass (vectors),
// deterministically PACKED (largest at the centre, the rest spiral out
// collision-free) and TONED by status (live=accent + a pulsing halo, indexing=
// warning, error=danger, warming=muted, ready=success). Click a bubble to ZOOM
// IN (it becomes the spine). The SAME circle-packing idiom the Observatory uses
// for a codebase's modules — a ministr corpus is a constellation at every zoom.

const MAP_W = 640;
const MAP_H = 360;
const MAP_PAD = 26;
const STAR_MIN_R = 16;
const STAR_MAX_R = 70;
const STAR_GAP = 5;
const STAR_GOLDEN = Math.PI * (3 - Math.sqrt(5)); // phyllotaxis spiral
const STAR_STEP = 1.1;
const STAR_MAX_SPIRAL = 6000;

interface StarBubble {
  id: string;
  label: string;
  tone: Tone;
  live: boolean;
  statusLabel: string;
  vectors: number;
  files: number;
  x: number;
  y: number;
  r: number;
}

/** Deterministic circle pack: largest index at the centre, each next placed at
 *  the first collision-free point along a golden-angle spiral, then the cluster
 *  uniformly scaled to fit + centred in the viewBox. Mirrors CodebaseConstellation. */
function buildStarLayout(corpora: CorpusInfo[]): StarBubble[] {
  if (corpora.length === 0) return [];
  const maxVec = corpora.reduce((m, c) => Math.max(m, c.embeddings_count), 0) || 1;
  const ordered = [...corpora].sort(
    (a, b) =>
      b.embeddings_count - a.embeddings_count ||
      corpusLabel(a).localeCompare(corpusLabel(b)),
  );
  const raw: StarBubble[] = ordered.map((c) => {
    const st = statusFor(c);
    return {
      id: c.id,
      label: corpusLabel(c),
      tone: st.tone,
      live: st.pulse,
      statusLabel: st.label,
      vectors: c.embeddings_count,
      files: c.files_indexed,
      r: STAR_MIN_R + (STAR_MAX_R - STAR_MIN_R) * Math.sqrt(c.embeddings_count / maxVec),
      x: 0,
      y: 0,
    };
  });

  const placed: StarBubble[] = [];
  for (let k = 0; k < raw.length; k++) {
    const c = raw[k];
    if (k === 0) {
      placed.push(c);
      continue;
    }
    let found = false;
    for (let step = 0; step < STAR_MAX_SPIRAL; step++) {
      const rad = step * STAR_STEP;
      const ang = step * STAR_GOLDEN;
      const x = Math.cos(ang) * rad;
      const y = Math.sin(ang) * rad;
      let ok = true;
      for (const p of placed) {
        if (Math.hypot(x - p.x, y - p.y) < c.r + p.r + STAR_GAP) {
          ok = false;
          break;
        }
      }
      if (ok) {
        c.x = x;
        c.y = y;
        found = true;
        break;
      }
    }
    if (!found) {
      c.x = (k % 2 ? 1 : -1) * STAR_MAX_R * 2 * k;
      c.y = 0;
    }
    placed.push(c);
  }

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const c of placed) {
    minX = Math.min(minX, c.x - c.r);
    minY = Math.min(minY, c.y - c.r);
    maxX = Math.max(maxX, c.x + c.r);
    maxY = Math.max(maxY, c.y + c.r);
  }
  const bw = maxX - minX || 1;
  const bh = maxY - minY || 1;
  const scale = Math.min((MAP_W - 2 * MAP_PAD) / bw, (MAP_H - 2 * MAP_PAD) / bh);
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  return placed.map((c) => ({
    ...c,
    x: MAP_W / 2 + (c.x - cx) * scale,
    y: MAP_H / 2 + (c.y - cy) * scale,
    r: c.r * scale,
  }));
}

/** Truncate a project label to the bubble's text room (~6px per mono char). */
function fitStar(label: string, r: number): string {
  const max = Math.max(3, Math.floor((r * 1.7) / 6));
  return label.length > max ? `${label.slice(0, max - 1)}…` : label;
}

export interface FleetConstellationProps {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  /** Zoom into a project (it becomes the spine). */
  onSelect: (id: string) => void;
}

export function FleetConstellation({
  corpora,
  activeCorpusId,
  onSelect,
}: FleetConstellationProps) {
  const bubbles = useMemo(() => buildStarLayout(corpora), [corpora]);
  const [hover, setHover] = useState<string | null>(null);

  if (bubbles.length === 0) return null;

  const totalVec = corpora.reduce((s, c) => s + c.embeddings_count, 0);
  const liveCount = corpora.filter((c) => statusFor(c).pulse).length;
  const hovered = hover ? bubbles.find((b) => b.id === hover) : null;

  return (
    <VizFrame
      readout={
        <>
          <span>
            <span className="tabular-nums font-semibold text-text">{corpora.length}</span>{" "}
            {corpora.length === 1 ? "project" : "projects"} ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {totalVec.toLocaleString()}
            </span>{" "}
            vectors
            {liveCount > 0 && (
              <>
                {" · "}
                <span className="tabular-nums font-semibold text-accent">{liveCount}</span> live
              </>
            )}
          </span>
          <span>sized by index mass · click to zoom in</span>
        </>
      }
    >
      <div className="-mx-1">
        <svg
          viewBox={`0 0 ${MAP_W} ${MAP_H}`}
          className="w-full"
          style={{ maxHeight: MAP_H }}
          role="group"
          aria-label={`Fleet star-map: ${corpora.length} project${corpora.length === 1 ? "" : "s"} drawn as bubbles sized by index mass; ${liveCount} with live agents.`}
        >
          {bubbles.map((b) => {
            const selected = b.id === activeCorpusId;
            const lit = !hover || hover === b.id;
            const big = b.r > 30;
            const mid = b.r > 18;
            return (
              <g
                key={b.id}
                role="button"
                tabIndex={0}
                aria-label={`${b.label}: ${b.vectors.toLocaleString()} vectors, ${b.files.toLocaleString()} files, ${b.statusLabel}.${selected ? " Current project." : " Open this project."}`}
                style={{
                  color: toneCssVar(b.tone),
                  opacity: lit ? 1 : 0.4,
                  transition: "opacity 150ms ease-out",
                  cursor: "pointer",
                }}
                onMouseEnter={() => setHover(b.id)}
                onMouseLeave={() => setHover(null)}
                onFocus={() => setHover(b.id)}
                onBlur={() => setHover(null)}
                onClick={() => onSelect(b.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(b.id);
                  }
                }}
              >
                <title>{`${b.label} · ${b.vectors.toLocaleString()} vec · ${b.statusLabel}`}</title>
                {/* Live halo — opacity-pulse only (reduced-motion-safe), behind the bubble. */}
                {b.live && (
                  <circle
                    cx={b.x}
                    cy={b.y}
                    r={b.r + 5}
                    fill="none"
                    stroke="currentColor"
                    strokeOpacity={0.5}
                    strokeWidth={1.5}
                    className="motion-safe:animate-pulse"
                  />
                )}
                <circle
                  cx={b.x}
                  cy={b.y}
                  r={b.r}
                  fill="currentColor"
                  fillOpacity={0.16}
                  stroke="currentColor"
                  strokeOpacity={hover === b.id || selected ? 1 : 0.6}
                  strokeWidth={selected ? 2.5 : 1.5}
                />
                {/* Selected spine ring — the zoomed-in object. */}
                {selected && (
                  <circle
                    cx={b.x}
                    cy={b.y}
                    r={b.r + 3.5}
                    fill="none"
                    stroke="currentColor"
                    strokeOpacity={0.85}
                    strokeWidth={1}
                  />
                )}
                {big ? (
                  <>
                    <text
                      x={b.x}
                      y={b.y - 1}
                      textAnchor="middle"
                      className="fill-text font-mono font-semibold"
                      style={{ fontSize: 11 }}
                    >
                      {fitStar(b.label, b.r)}
                    </text>
                    <text
                      x={b.x}
                      y={b.y + 12}
                      textAnchor="middle"
                      className="fill-text-dim font-mono tabular-nums"
                      style={{ fontSize: 9 }}
                    >
                      {b.vectors.toLocaleString()} vec
                    </text>
                  </>
                ) : mid ? (
                  <text
                    x={b.x}
                    y={b.y + 3}
                    textAnchor="middle"
                    className="fill-text font-mono font-semibold"
                    style={{ fontSize: 9 }}
                  >
                    {fitStar(b.label, b.r)}
                  </text>
                ) : null}
              </g>
            );
          })}

          {/* Hover caption — bottom-anchored so it never occludes the field. */}
          {hovered && (
            <text
              x={MAP_W / 2}
              y={MAP_H - 6}
              textAnchor="middle"
              className="fill-text font-mono"
              style={{ fontSize: 11 }}
            >
              {`${hovered.label} · ${hovered.vectors.toLocaleString()} vec · ${hovered.statusLabel}`}
            </text>
          )}
        </svg>
      </div>
    </VizFrame>
  );
}

// ── One project instrument in the constellation ─────────────────────────────

function statusFor(c: CorpusInfo): { tone: Tone; pulse: boolean; label: string } {
  if (c.status.state === "error") return { tone: "danger", pulse: false, label: "Error" };
  if (c.active_sessions > 0) return { tone: "accent", pulse: true, label: "Live" };
  if (c.status.state === "indexing") return { tone: "warning", pulse: false, label: "Indexing" };
  if (c.status.state === "queued") return { tone: "accent", pulse: false, label: "Queued" };
  if (c.warming) return { tone: "muted", pulse: false, label: "Warming" };
  return { tone: "success", pulse: false, label: "Ready" };
}

function FleetCell({
  corpus,
  selected,
  massPct,
  onSelect,
  onReindex,
  onRemove,
}: {
  corpus: CorpusInfo;
  selected: boolean;
  massPct: number;
  onSelect: () => void;
  onReindex: () => void;
  onRemove: () => void;
}) {
  const st = statusFor(corpus);
  const indexing =
    corpus.status.state === "indexing" ? corpus.status : null;
  const indexPct = indexing
    ? indexing.files_total > 0
      ? Math.min(100, Math.round((indexing.files_done / indexing.files_total) * 100))
      : 0
    : 0;
  const freshTone = freshnessTone(corpus.last_indexed);

  return (
    <Card
      hover="lift"
      onClick={onSelect}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "group cursor-pointer p-4 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
        // Live projects glow; the selected spine keeps a steady accent ring.
        st.label === "Live" && "border-accent/50",
        selected && "border-accent shadow-[var(--glow-soft)]",
      )}
    >
      {/* Identity row */}
      <div className="flex items-center gap-2">
        <StatusDot tone={st.tone} pulse={st.pulse ? "live" : "off"} size="md" />
        <span className="font-mono text-sm font-bold tracking-[0.04em] text-text truncate min-w-0 flex-1">
          {corpusLabel(corpus)}
        </span>
        {corpus.active_sessions > 0 ? (
          <Badge variant="default" dot>
            {corpus.active_sessions} live
          </Badge>
        ) : (
          <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            {st.label}
          </span>
        )}
        {/* Hover quick-actions */}
        <div className="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-150">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={(e) => {
              e.stopPropagation();
              onReindex();
            }}
            title="Re-index"
            aria-label={`Re-index ${corpusLabel(corpus)}`}
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
            aria-label={`Remove ${corpusLabel(corpus)}`}
            className="hover:text-danger"
          >
            <Trash2 className="h-3.5 w-3.5" strokeWidth={2.5} />
          </Button>
        </div>
      </div>

      <p className="font-mono text-[10px] text-text-dim truncate mt-1">
        {corpusRoot(corpus.paths)}
      </p>

      {/* Index mass (or live indexing progress) — the bespoke collection signal */}
      <div className="mt-3 space-y-1">
        <div className="flex items-center justify-between font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
          <span>{indexing ? "Indexing" : "Index mass"}</span>
          <span className="tabular-nums text-text">
            {indexing
              ? `${indexing.files_done.toLocaleString()} / ${indexing.files_total.toLocaleString()}`
              : `${corpus.embeddings_count.toLocaleString()} vec`}
          </span>
        </div>
        <div className="h-1.5 rounded-full bg-surface-overlay overflow-hidden">
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-500 ease-out",
              indexing ? "bg-warning animate-pulse" : "bg-accent",
            )}
            style={{ width: `${indexing ? indexPct : massPct}%` }}
          />
        </div>
      </div>

      {/* Footer — size + freshness pip */}
      <div className="mt-3 flex items-center justify-between gap-2 font-mono text-[10px] text-text-dim">
        <span className="truncate min-w-0">
          {corpus.files_indexed.toLocaleString()} files
          {(corpus.symbols_count ?? 0) > 0 &&
            ` · ${(corpus.symbols_count ?? 0).toLocaleString()} symbols`}
        </span>
        <span className="flex items-center gap-1.5 shrink-0">
          <StatusDot tone={freshTone} />
          {corpus.last_indexed ? formatRelativeTime(corpus.last_indexed) : "never"}
        </span>
      </div>

      {corpus.status.state === "error" && (
        <p className="mt-2 flex items-start gap-1.5 font-mono text-[10px] text-danger">
          <AlertTriangle className="h-3 w-3 shrink-0 mt-0.5" strokeWidth={2.5} />
          {corpus.status.message}
        </p>
      )}
    </Card>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — live invoke + the unified ConfirmDialog flows. Drop-in for the
// Fleet render (same props as the retired ProjectsSurface).

interface Props {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onRefresh: () => void;
}

export function FleetSurface({
  corpora,
  activeCorpusId,
  onSelectCorpus,
  onRefresh,
}: Props) {
  const { toast } = useToast();
  const [busyAdd, setBusyAdd] = useState(false);
  const [busyScan, setBusyScan] = useState(false);
  const [confirmReindex, setConfirmReindex] = useState<CorpusInfo | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<CorpusInfo | null>(null);

  async function addProject() {
    setBusyAdd(true);
    try {
      const res = await invoke<{ corpus_id: string } | null>("add_project_dialog");
      if (res) {
        onRefresh();
        toast("Project added", { tone: "success" });
      }
    } catch (e) {
      toast("Couldn’t add project", { detail: String(e), tone: "danger" });
    } finally {
      setBusyAdd(false);
    }
  }

  async function scanForProjects() {
    setBusyScan(true);
    try {
      const detected = await invoke<{ path: string; name: string }[]>("detect_projects");
      if (detected.length > 0) {
        await invoke("register_projects_batch", { paths: detected.map((d) => d.path) });
        onRefresh();
        toast("Projects found", { detail: `Added ${detected.length}`, tone: "success" });
      } else {
        toast("No projects found", { detail: "Nothing under the usual roots", tone: "info" });
      }
    } catch (e) {
      toast("Scan failed", { detail: String(e), tone: "danger" });
    } finally {
      setBusyScan(false);
    }
  }

  async function performReindex() {
    const c = confirmReindex;
    setConfirmReindex(null);
    if (!c) return;
    try {
      await invoke("trigger_reindex", { corpusId: c.id });
      onRefresh();
    } catch (e) {
      toast("Re-index failed", { detail: String(e), tone: "danger" });
    }
  }

  async function performRemove() {
    const c = confirmRemove;
    setConfirmRemove(null);
    if (!c) return;
    try {
      await invoke("remove_project", { corpusId: c.id });
      onRefresh();
    } catch (e) {
      toast("Remove failed", { detail: String(e), tone: "danger" });
    }
  }

  return (
    <>
      <FleetDeck
        corpora={corpora}
        activeCorpusId={activeCorpusId}
        busyAdd={busyAdd}
        busyScan={busyScan}
        onSelect={onSelectCorpus}
        onAdd={addProject}
        onScan={scanForProjects}
        onReindex={(c) => setConfirmReindex(c)}
        onRemove={(c) => setConfirmRemove(c)}
      />

      <ConfirmDialog
        open={!!confirmReindex}
        title="Re-index project"
        body={
          confirmReindex && (
            <p>
              This drops the existing index for{" "}
              <span className="font-bold">{corpusLabel(confirmReindex)}</span> and
              starts over.
            </p>
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
                This removes{" "}
                <span className="font-bold">{corpusLabel(confirmRemove)}</span>{" "}
                from the registry (indexed sections + symbols).
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
    </>
  );
}
