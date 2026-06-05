/**
 * SymbolImpact — the symbol inspector's call-hierarchy + coverage facet.
 *
 * Surfaces the shipped-but-previously-invisible FL3 (call hierarchy) and FL6
 * (test↔code mapping) as a first-class facet of the symbol object: who CALLS
 * this symbol (the incoming blast radius + risk), what it CALLS (outgoing), and
 * which TESTS exercise it (FL6). Three lanes, every node a clickable hop into
 * the stacked inspector. Built fresh from the v4 tokens/atoms; embedded below
 * the neighborhood in the shared EntityPanel symbol view.
 *
 * Pure `SymbolImpact` renders from props (Storybook); `SymbolImpactConnector`
 * wires the `symbol_impact` invoke.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FlaskConical,
  PhoneIncoming,
  PhoneOutgoing,
  ShieldAlert,
  TriangleAlert,
  Waypoints,
} from "@/components/ui/icons";

import type { ImpactedSymbol, SymbolImpact as SymbolImpactData } from "../../lib/types";
import { cn } from "../../lib/utils";
import { BlastRadiusMap } from "./BlastRadiusMap";
import { VizFrame } from "../ui/viz-frame";

/** Max nodes rendered per lane before a "+N more" tail (the inspector is a
 *  narrow scroll column; the full set is always one MCP call away). */
const LANE_CAP = 8;

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

const RISK_META: Record<SymbolImpactData["risk"], { label: string; chip: string; icon: typeof TriangleAlert }> = {
  low: { label: "Low", chip: "border-success/40 bg-success/10 text-success", icon: ShieldAlert },
  medium: { label: "Medium", chip: "border-warning/40 bg-warning/10 text-warning", icon: TriangleAlert },
  high: { label: "High", chip: "border-danger/40 bg-danger/10 text-danger", icon: TriangleAlert },
};

export interface SymbolImpactProps {
  data: SymbolImpactData | null;
  loading?: boolean;
  /** Descend into a caller / callee / test symbol (stacked navigation). */
  onOpenSymbol: (node: ImpactedSymbol) => void;
}

export function SymbolImpact({ data, loading = false, onOpenSymbol }: SymbolImpactProps) {
  if (loading) {
    return (
      <section className="rounded-lg border border-border-soft bg-surface-sunken/40 p-4">
        <BlockHeader />
        <p className="mt-2 font-mono text-mono-mini text-text-dim">
          Tracing the call graph<span className="ministr-blink">_</span>
        </p>
      </section>
    );
  }
  if (!data) return null;

  const risk = RISK_META[data.risk];
  const covered = data.tests.length > 0;
  return (
    <section className="rounded-lg border border-border-soft bg-surface-sunken/40 p-4 space-y-3">
      <BlockHeader />

      {/* Summary readout — risk + the blast counts + coverage, in one line. */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-mono-micro text-text-dim">
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-semibold uppercase tracking-[0.06em]",
            risk.chip,
          )}
          title="Aggregate blast-radius risk"
        >
          <risk.icon className="h-3 w-3" strokeWidth={2.25} />
          {risk.label} risk
        </span>
        <span className="flex items-center gap-1">
          <span className="tabular-nums font-semibold text-text">{data.incoming_symbols}</span>
          can break
        </span>
        <span aria-hidden className="text-border">·</span>
        <span className="flex items-center gap-1">
          <span className="tabular-nums font-semibold text-text">{data.outgoing_symbols}</span>
          reached
        </span>
        <span aria-hidden className="text-border">·</span>
        <span className={cn("flex items-center gap-1", covered ? "text-success" : "text-warning")}>
          {covered ? (
            <>
              <FlaskConical className="h-3 w-3" strokeWidth={2.25} />
              <span className="tabular-nums font-semibold">{data.tests.length}</span>
              <span className="text-text-dim">covered</span>
            </>
          ) : (
            <>
              <TriangleAlert className="h-3 w-3" strokeWidth={2.25} />
              no coverage
            </>
          )}
        </span>
      </div>

      {/* The Blast-Radius Map — the at-a-glance call graph (hero), on the
          shared VizFrame so it reads as one family with the other vizzes. */}
      <VizFrame>
        <BlastRadiusMap data={data} onOpenSymbol={onOpenSymbol} />
      </VizFrame>

      <Lane
        title="Called by"
        sub="incoming — the blast radius"
        icon={PhoneIncoming}
        tone="text-danger"
        nodes={data.incoming}
        total={data.incoming_symbols}
        badge={
          data.incoming.length > 0 ? (
            <span
              className={cn(
                "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em]",
                risk.chip,
              )}
              title="Aggregate blast-radius risk"
            >
              <risk.icon className="h-3 w-3" strokeWidth={2.25} />
              {risk.label} risk
            </span>
          ) : null
        }
        emptyHint="Nothing calls this — a leaf, entry point, or dynamically dispatched."
        onOpenSymbol={onOpenSymbol}
      />

      <Lane
        title="Calls"
        sub="outgoing — what it reaches"
        icon={PhoneOutgoing}
        tone="text-accent"
        nodes={data.outgoing}
        total={data.outgoing_symbols}
        emptyHint="Reaches nothing in the tracked call graph."
        onOpenSymbol={onOpenSymbol}
      />

      <Lane
        title="Covered by"
        sub="tests that exercise this symbol"
        icon={FlaskConical}
        tone={data.tests.length > 0 ? "text-success" : "text-warning"}
        nodes={data.tests}
        total={data.tests.length}
        badge={
          data.tests.length === 0 ? (
            <span className="inline-flex items-center gap-1 rounded border border-warning/40 bg-warning/10 px-1.5 py-0.5 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em] text-warning">
              <TriangleAlert className="h-3 w-3" strokeWidth={2.25} />
              No coverage
            </span>
          ) : null
        }
        emptyHint="No test transitively exercises this symbol — a coverage gap before you change it."
        onOpenSymbol={onOpenSymbol}
      />
    </section>
  );
}

function BlockHeader() {
  return (
    <div className="flex items-center gap-2">
      <Waypoints className="h-4 w-4 text-accent" strokeWidth={2} />
      <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-text">Impact</span>
      <span className="font-mono text-mono-micro text-text-dim">call hierarchy &amp; coverage</span>
    </div>
  );
}

function Lane({
  title,
  sub,
  icon: Icon,
  tone,
  nodes,
  total,
  badge,
  emptyHint,
  onOpenSymbol,
}: {
  title: string;
  sub: string;
  icon: typeof PhoneIncoming;
  tone: string;
  nodes: ImpactedSymbol[];
  total: number;
  badge?: React.ReactNode;
  emptyHint: string;
  onOpenSymbol: (node: ImpactedSymbol) => void;
}) {
  const shown = nodes.slice(0, LANE_CAP);
  const extra = nodes.length - shown.length;
  return (
    <div>
      <div className="flex items-center gap-2 border-b border-border-soft pb-1.5">
        <Icon className={cn("h-3.5 w-3.5", tone)} strokeWidth={2} />
        <span className="font-mono text-mono-mini font-bold uppercase tracking-[0.06em] text-text">{title}</span>
        <span className="font-mono text-mono-micro text-text-dim">{sub}</span>
        <span className="ml-auto flex items-center gap-2">
          {badge}
          <span className="font-mono text-mono-micro tabular-nums text-text-dim">
            <span className="font-semibold text-text">{total}</span>
          </span>
        </span>
      </div>
      {nodes.length === 0 ? (
        <p className="px-1 py-2 font-mono text-mono-micro text-text-dim">{emptyHint}</p>
      ) : (
        <ul className="divide-y divide-border-soft/50">
          {shown.map((n) => (
            <NodeRow key={n.symbol_id || `${n.file}:${n.line}`} node={n} onOpen={() => onOpenSymbol(n)} />
          ))}
          {extra > 0 && (
            <li className="px-1 py-1.5 font-mono text-mono-micro text-text-dim">+{extra} more</li>
          )}
        </ul>
      )}
    </div>
  );
}

function NodeRow({ node, onOpen }: { node: ImpactedSymbol; onOpen: () => void }) {
  return (
    // Keep the <li> a real listitem (a11y: list / aria-allowed-role) — the
    // interactive control is a <button> inside it, not a role on the <li>.
    <li>
      <button
        type="button"
        onClick={onOpen}
        title={`Inspect ${node.name}`}
        className="group flex w-full items-center gap-2.5 px-1 py-1.5 text-left cursor-pointer hover:bg-surface-overlay rounded transition-colors duration-150"
      >
        <span
          className="shrink-0 rounded-full border border-border-soft px-1.5 font-mono text-mono-micro tabular-nums text-text-dim"
          title={`${node.depth} hop${node.depth === 1 ? "" : "s"} away`}
        >
          {node.depth}↑
        </span>
        <span className="shrink-0 rounded border border-border-soft bg-surface px-1 font-mono text-mono-micro lowercase tracking-[0.04em] text-text-dim">
          {node.kind || "sym"}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-text group-hover:text-accent">
          {node.name}
        </span>
        <span className="ml-2 max-w-[40%] shrink-0 truncate font-mono text-mono-micro text-text-dim">
          {fileTail(node.file)}
        </span>
      </button>
    </li>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — runs symbol_impact for a symbol.

export function SymbolImpactConnector({
  corpusId,
  symbolId,
  onOpenSymbol,
}: {
  corpusId: string;
  symbolId: string;
  onOpenSymbol: (node: ImpactedSymbol) => void;
}) {
  const [data, setData] = useState<SymbolImpactData | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setData(null);
    invoke<SymbolImpactData>("symbol_impact", { corpusId, symbolId, maxDepth: 3 })
      .then((r) => {
        if (!cancelled) {
          setData(r);
          setLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setData(null);
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, symbolId]);

  return <SymbolImpact data={data} loading={loading} onOpenSymbol={onOpenSymbol} />;
}
