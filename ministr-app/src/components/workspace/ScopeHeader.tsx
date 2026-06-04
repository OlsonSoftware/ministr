import type { ReactNode } from "react";
import { motion } from "motion/react";
import { Boxes, Layers } from "lucide-react";
import { useWorkspace } from "./WorkspaceContext";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { type Tone, corpusTone, isCorpusLive } from "../../lib/status";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { StatusDot } from "../ui/status-dot";
import { MetricTile } from "../ui/metric-tile";

/**
 * The scope header — the COMMAND DECK: the persistent identity of the
 * workspace's central object (a Project, or Fleet = the collection). Every
 * facet sits beneath it, so switching facets visibly keeps the SAME object in
 * view — the "one context" integration test made visible.
 *
 * It is a layered banner, not a flat strip: a glowing status medallion + a
 * confident name + a live pill on the left, and a divided "vital readout" of
 * the object's stats on the right. It reads only `useWorkspace()`, never a
 * per-facet selection. Built entirely from v4 tokens + ui atoms (StatusDot,
 * MetricTile); tone colour stays on non-text (glyphs/dots/borders/glow) so the
 * readout numbers keep full AA contrast.
 */
export function ScopeHeader() {
  const { isFleet, activeProject, corpora } = useWorkspace();

  if (isFleet) {
    const projects = corpora.length;
    const files = corpora.reduce((n, c) => n + c.files_indexed, 0);
    const symbols = corpora.reduce((n, c) => n + c.symbols_count, 0);
    const live = corpora.filter((c) => isCorpusLive(c)).length;
    return (
      <Banner
        live={live > 0}
        animationKey="fleet"
        identity={
          <Identity
            medallion={<Medallion icon={Layers} tone="accent" live={live > 0} />}
            title="Fleet"
            subtitle="all projects · zoomed out"
          />
        }
        vitals={
          <Vitals>
            <MetricTile variant="cell" label="projects" value={fmt(projects)} />
            <LiveCell count={live} />
            <MetricTile variant="cell" label="files" value={fmt(files)} />
            <MetricTile variant="cell" label="symbols" value={fmt(symbols)} />
          </Vitals>
        }
      />
    );
  }

  if (!activeProject) {
    return (
      <Banner
        animationKey="none"
        identity={
          <div className="flex items-center gap-3">
            <Medallion icon={Boxes} tone="muted" live={false} />
            <div className="font-sans text-sm text-text-dim">
              No project selected
            </div>
          </div>
        }
      />
    );
  }

  const c = activeProject;
  const root = corpusRoot(c.paths);
  const tone = corpusTone(c);
  const live = isCorpusLive(c);
  return (
    <Banner
      live={live}
      animationKey={c.id}
      identity={
        <Identity
          medallion={<Medallion icon={Boxes} tone={tone} live={live} dot />}
          title={corpusLabel(c)}
          titleMono
          subtitle={root ?? undefined}
          subtitleMono
          badge={
            c.active_sessions > 0 ? (
              <LivePill count={c.active_sessions} />
            ) : null
          }
        />
      }
      vitals={
        <Vitals>
          <MetricTile
            variant="cell"
            label="files"
            value={fmt(c.files_indexed)}
          />
          <MetricTile
            variant="cell"
            label="sections"
            value={fmt(c.sections_count)}
          />
          <MetricTile
            variant="cell"
            label="symbols"
            value={fmt(c.symbols_count)}
          />
          <LiveCell count={c.active_sessions} />
          {c.model && (
            <MetricTile variant="cell" label="model" value={c.model} />
          )}
        </Vitals>
      }
    />
  );
}

function fmt(n: number): string {
  return n.toLocaleString();
}

// ── The banner shell — depth (raised tier + shadow + accent hairline) ────────

function Banner({
  identity,
  vitals,
  live = false,
  animationKey,
}: {
  identity: ReactNode;
  vitals?: ReactNode;
  live?: boolean;
  animationKey: string;
}) {
  return (
    <header className="relative flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-border bg-surface-raised px-5 py-3 shadow-sm">
      {/* A faint accent hairline along the top edge — brighter when the
          scoped object is live — gives the deck its lit top edge. */}
      <span
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent to-transparent",
          live ? "via-accent/50" : "via-border-hover",
        )}
      />
      <motion.div
        key={animationKey}
        initial={{ opacity: 0, y: -3 }}
        animate={{ opacity: 1, y: 0 }}
        transition={spring}
        className="flex min-w-0 items-center"
      >
        {identity}
      </motion.div>
      {vitals}
    </header>
  );
}

// ── Identity (medallion + title + subtitle + optional live badge) ────────────

function Identity({
  medallion,
  title,
  titleMono = false,
  subtitle,
  subtitleMono = false,
  badge,
}: {
  medallion: ReactNode;
  title: string;
  titleMono?: boolean;
  subtitle?: string;
  subtitleMono?: boolean;
  badge?: ReactNode;
}) {
  return (
    <div className="flex min-w-0 items-center gap-3">
      {medallion}
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "truncate text-[15px] font-semibold text-text",
              titleMono ? "font-mono" : "font-sans",
            )}
          >
            {title}
          </span>
          {badge}
        </div>
        {subtitle && (
          <div
            className={cn(
              "truncate text-mono-mini text-text-dim",
              subtitleMono ? "font-mono" : "font-sans",
            )}
          >
            {subtitle}
          </div>
        )}
      </div>
    </div>
  );
}

/** The glowing identity medallion — a rounded tile holding the object glyph,
 *  with an optional corner status dot and a soft accent glow when live. */
function Medallion({
  icon: Icon,
  tone,
  live,
  dot = false,
}: {
  icon: typeof Boxes;
  tone: Tone;
  live: boolean;
  dot?: boolean;
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "relative grid h-11 w-11 shrink-0 place-items-center rounded-xl border bg-surface-overlay",
        live
          ? "border-accent/50 text-accent shadow-[var(--glow-soft)]"
          : "border-border text-text-muted",
      )}
    >
      <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
      {dot && (
        <span className="absolute -right-1 -top-1 grid place-items-center rounded-full bg-surface-raised p-0.5">
          <StatusDot tone={tone} pulse={live ? "live" : "off"} size="md" />
        </span>
      )}
    </span>
  );
}

/** A pill that announces live agents on the scoped object. Accent flavour on
 *  the border/dot; the text stays high-contrast (AA-safe). */
function LivePill({ count }: { count: number }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-accent/40 bg-accent/10 px-2 py-0.5 font-mono text-mono-micro font-medium uppercase tracking-[0.06em] text-text">
      <StatusDot tone="accent" pulse="live" />
      {count} live
    </span>
  );
}

// ── Vitals — the divided readout cluster ─────────────────────────────────────

function Vitals({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-stretch divide-x divide-border overflow-hidden rounded-lg border border-border bg-surface">
      {children}
    </div>
  );
}

/** The live-agents vital — number + a pulsing accent dot when any are live. */
function LiveCell({ count }: { count: number }) {
  return (
    <MetricTile
      variant="cell"
      label="live agents"
      value={
        <span className="inline-flex items-center gap-1.5">
          {fmt(count)}
          {count > 0 && <StatusDot tone="accent" pulse="live" size="md" />}
        </span>
      }
    />
  );
}
