/**
 * SolidMap — the Explore "Quality" lens: SOLID / architecture-smell findings.
 *
 * The daemon's SOLID detector flags six kinds of structural smell — DRY/OCP
 * near-duplicate clusters, SRP low-cohesion containers, ISP fat interfaces, DIP
 * concrete cross-package dependencies, Fowler's Shotgun Surgery, and cyclic
 * package dependencies. Rather than six bespoke layouts, every finding is
 * NORMALISED (summarise()) into one shape — a principle badge, a headline, a
 * one-line subtitle, a metric, and the involved symbols — so they read as one
 * coherent backlog of things worth refactoring. Each involved symbol opens in
 * the shared EntityPanel (confirm before acting). Built fresh from v4 tokens.
 *
 * Pure `SolidMap` renders from props (Storybook); `SolidMapConnector` wires the
 * `solid_findings` invoke + the shared inspector.
 */
import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronRight,
  Copy,
  Layers,
  Repeat,
  Scissors,
  ShieldCheck,
  Unplug,
  Workflow,
  Zap,
} from "@/components/ui/icons";

import type { SolidFinding, SolidSymbolRef } from "../../lib/types";
import type { SymbolInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useCachedQuery } from "../../hooks/useCachedQuery";
import { useArrowKeyListNav } from "../../hooks/useArrowKeyListNav";
import { LensHeader, LensLoading, LensEmpty, LensRerunButton } from "../ui/lens-frame";

// ── Principle → display meta. ──────────────────────────────────────────────
const PRINCIPLE_META: Record<
  string,
  { label: string; icon: typeof Copy; order: number }
> = {
  dry_ocp: { label: "DRY / OCP", icon: Copy, order: 0 },
  srp: { label: "SRP", icon: Scissors, order: 1 },
  isp: { label: "ISP", icon: Unplug, order: 2 },
  dip: { label: "DIP", icon: Layers, order: 3 },
  shotgun_surgery: { label: "Shotgun surgery", icon: Zap, order: 4 },
  cyclic_dependency: { label: "Cyclic dependency", icon: Repeat, order: 5 },
};

function principleMeta(p: string) {
  return (
    PRINCIPLE_META[p] ?? {
      label: p.replace(/_/g, " "),
      icon: Workflow,
      order: 99,
    }
  );
}

interface Normalized {
  principle: string;
  title: string;
  subtitle: string;
  metric: string;
  symbols: SolidSymbolRef[];
}

/** Collapse any of the six SolidFinding variants into one renderable shape. */
function summarise(f: SolidFinding): Normalized {
  switch (f.type) {
    case "redundancy":
      return {
        principle: f.principle,
        title: `${f.members_total} near-duplicate ${f.canonical.kind}s`,
        subtitle: `canonical: ${f.canonical.name}${f.cross_module ? " · across modules" : ""}`,
        metric: `${Math.round(f.avg_cosine * 100)}% cos`,
        symbols: dedupe([f.canonical, ...f.members]),
      };
    case "low_cohesion":
      return {
        principle: f.principle,
        title: f.container.name,
        subtitle: `${f.components.length} cohesion clusters · ${f.method_count} methods`,
        metric: `${f.components.length} parts`,
        symbols: dedupe([
          f.container,
          ...f.components.flatMap((c) => c.members),
        ]),
      };
    case "fat_interface":
      return {
        principle: f.principle,
        title: f.interface.name,
        subtitle: `${f.unused_methods.length} unused methods · ${f.under_using_implementors.length} under-using impls`,
        metric: `${f.method_count} methods`,
        symbols: dedupe([f.interface, ...f.under_using_implementors]),
      };
    case "concrete_dependency":
      return {
        principle: f.principle,
        title: `${f.consumer.name} → ${f.concrete_target.name}`,
        subtitle: f.suggested_abstraction
          ? `abstract via ${f.suggested_abstraction.name}`
          : "concrete cross-package dependency",
        metric: "DIP",
        symbols: dedupe([
          f.consumer,
          f.concrete_target,
          ...(f.suggested_abstraction ? [f.suggested_abstraction] : []),
        ]),
      };
    case "shotgun_surgery":
      return {
        principle: f.principle,
        title: f.name,
        subtitle: `${f.sites_total} parallel ${f.kind} sites`,
        metric: `${f.sites_total} sites`,
        symbols: dedupe(f.sites),
      };
    case "cyclic_dependency":
      return {
        principle: f.principle,
        title: `${f.packages.length}-package cycle`,
        subtitle: f.packages.join(" → "),
        metric: `${f.edge_count} edges`,
        symbols: dedupe(
          f.example_edges.flatMap((e) => [e.example_from, e.example_to]),
        ),
      };
  }
}

function dedupe(refs: SolidSymbolRef[]): SolidSymbolRef[] {
  const seen = new Set<string>();
  const out: SolidSymbolRef[] = [];
  for (const r of refs) {
    const k = r.symbol_id || `${r.file}:${r.line}:${r.name}`;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(r);
  }
  return out;
}

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

export interface SolidMapProps {
  findings: SolidFinding[];
  loading?: boolean;
  /** Re-run the SOLID/architecture audit (a snapshot — re-run after editing). */
  onRefresh?: () => void;
  refreshing?: boolean;
  /** Inspect an involved symbol in the shared EntityPanel. */
  onInspect: (ref: SolidSymbolRef) => void;
  /** Open an involved symbol's file in the code lens (the cross-lens
   *  open-to-code affordance — keeps Quality consistent with the other lenses). */
  onOpenFile?: (path: string, line: number) => void;
}

export function SolidMap({
  findings = [],
  loading = false,
  onRefresh,
  refreshing = false,
  onInspect,
  onOpenFile,
}: SolidMapProps) {
  const [principleFilter, setPrincipleFilter] = useState<string | null>(null);
  const listRef = useArrowKeyListNav<HTMLDivElement>();

  const normalized = useMemo(() => findings.map(summarise), [findings]);

  const principles = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of normalized) m.set(n.principle, (m.get(n.principle) ?? 0) + 1);
    return [...m.entries()].sort(
      (a, b) => principleMeta(a[0]).order - principleMeta(b[0]).order,
    );
  }, [normalized]);

  const visible = useMemo(
    () =>
      principleFilter
        ? normalized.filter((n) => n.principle === principleFilter)
        : normalized,
    [normalized, principleFilter],
  );

  const ordered = useMemo(
    () =>
      [...visible].sort(
        (a, b) => principleMeta(a.principle).order - principleMeta(b.principle).order,
      ),
    [visible],
  );

  if (loading) {
    return <LensLoading label="Auditing the architecture" />;
  }

  if (findings.length === 0) {
    return (
      <LensEmpty
        icon={ShieldCheck}
        accent
        title="No SOLID findings"
        hint="No near-duplicate clusters, low-cohesion containers, fat interfaces, concrete cross-package dependencies, shotgun-surgery families, or import cycles surfaced. The architecture looks tidy."
        action={
          onRefresh ? (
            <LensRerunButton onRefresh={onRefresh} refreshing={refreshing} />
          ) : undefined
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <LensHeader
        icon={ShieldCheck}
        title="Architecture findings"
        tone="accent"
        glance={
          <>
            <span className="tabular-nums font-semibold text-text">
              {findings.length}
            </span>{" "}
            findings ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {principles.length}
            </span>{" "}
            principles
          </>
        }
        hint="Heuristic smells — candidates for refactoring, not failures. Inspect a symbol to judge it in context."
        onRefresh={onRefresh}
        refreshing={refreshing}
      >
        <div className="flex flex-wrap gap-1.5">
          <PrincipleChip
            label="All"
            count={findings.length}
            active={principleFilter === null}
            onClick={() => setPrincipleFilter(null)}
          />
          {principles.map(([p, n]) => (
            <PrincipleChip
              key={p}
              label={principleMeta(p).label}
              icon={principleMeta(p).icon}
              count={n}
              active={principleFilter === p}
              onClick={() => setPrincipleFilter(principleFilter === p ? null : p)}
            />
          ))}
        </div>
      </LensHeader>

      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-3 space-y-2.5">
        {ordered.map((n, i) => (
          <FindingCard
            key={`${n.principle}:${n.title}:${i}`}
            n={n}
            onInspect={onInspect}
            onOpenFile={onOpenFile}
          />
        ))}
      </div>
    </div>
  );
}

function FindingCard({
  n,
  onInspect,
  onOpenFile,
}: {
  n: Normalized;
  onInspect: (ref: SolidSymbolRef) => void;
  onOpenFile?: (path: string, line: number) => void;
}) {
  const meta = principleMeta(n.principle);
  const Icon = meta.icon;
  return (
    <section className="rounded-lg border border-border-soft bg-surface overflow-hidden">
      <header className="flex items-center gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
        <Icon className="h-3.5 w-3.5 text-accent shrink-0" strokeWidth={2.25} />
        <span className="shrink-0 rounded border border-accent/40 bg-surface px-1.5 font-mono text-mono-micro font-bold uppercase tracking-[0.06em] text-accent">
          {meta.label}
        </span>
        <span className="truncate font-mono text-xs font-semibold text-text">
          {n.title}
        </span>
        <span className="ml-auto shrink-0 rounded-full border border-border px-1.5 font-mono text-mono-micro tabular-nums text-text-muted">
          {n.metric}
        </span>
      </header>
      <p className="px-3 py-1.5 font-mono text-mono-mini text-text-dim border-b border-border-soft/60">
        {n.subtitle}
      </p>
      <div className="divide-y divide-border-soft/60">
        {n.symbols.slice(0, 8).map((s) => (
          // Row = inspect the symbol; the file:line chip = open it in the code
          // lens (the shared cross-lens convention — inspect vs. open-to-code).
          <div
            key={s.symbol_id || `${s.file}:${s.line}`}
            className="group flex w-full items-center gap-2 px-3 py-1.5 hover:bg-surface-overlay transition-colors duration-150 ease-out"
          >
            <button
              type="button"
              data-roving-item
              onClick={() => onInspect(s)}
              title={`Inspect ${s.name}`}
              className="flex min-w-0 flex-1 items-center gap-2 text-left cursor-pointer"
            >
              <ChevronRight
                className="h-3 w-3 shrink-0 text-text-dim group-hover:text-accent"
                strokeWidth={2.5}
              />
              <span className="shrink-0 rounded border border-border-soft bg-surface px-1 font-mono text-mono-micro uppercase tracking-[0.06em] text-text-dim">
                {s.kind}
              </span>
              <span className="truncate font-mono text-mono-mini font-semibold text-text">
                {s.name}
              </span>
            </button>
            {onOpenFile ? (
              <button
                type="button"
                onClick={() => onOpenFile(s.file, s.line)}
                title={`Open ${s.file}:${s.line}`}
                className="shrink-0 truncate font-mono text-mono-micro text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
              >
                {fileTail(s.file)}:{s.line}
              </button>
            ) : (
              <span className="shrink-0 truncate font-mono text-mono-micro text-text-dim">
                {fileTail(s.file)}:{s.line}
              </span>
            )}
          </div>
        ))}
        {n.symbols.length > 8 && (
          <p className="px-3 py-1.5 font-mono text-mono-micro text-text-dim">
            +{n.symbols.length - 8} more
          </p>
        )}
      </div>
    </section>
  );
}

function PrincipleChip({
  label,
  count,
  active,
  onClick,
  icon: Icon,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  icon?: typeof Copy;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 font-mono text-mono-mini uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150 ease-out",
        active
          ? "border-accent bg-surface-overlay text-text"
          : "border-border-soft bg-surface text-text-muted hover:border-border hover:text-text",
      )}
    >
      {Icon && <Icon className="h-3 w-3" strokeWidth={2.25} />}
      <span className="font-semibold">{label}</span>
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — fetches SOLID findings + wires the shared inspector.

function solidRefToSymbolInfo(r: SolidSymbolRef): SymbolInfo {
  return {
    id: r.symbol_id,
    name: r.name,
    kind: r.kind,
    file_path: r.file,
    visibility: "",
    signature: `${r.kind} ${r.name}`,
    doc_comment: null,
    module_path: "",
  };
}

export function SolidMapConnector({
  corpusId,
  onOpenFile,
}: {
  corpusId: string;
  onOpenFile?: (path: string, line: number) => void;
}) {
  const { openEntity } = useEntityPanel();
  const { data, loading, refreshing, refresh } = useCachedQuery<SolidFinding[]>(
    corpusId,
    "solid_findings",
    () => invoke<SolidFinding[]>("solid_findings", { corpusId, limit: 200 }),
    [],
  );

  return (
    <SolidMap
      findings={data}
      loading={loading}
      onRefresh={refresh}
      refreshing={refreshing}
      onInspect={(r) =>
        openEntity({ kind: "symbol", corpusId, symbol: solidRefToSymbolInfo(r) })
      }
      onOpenFile={onOpenFile}
    />
  );
}
