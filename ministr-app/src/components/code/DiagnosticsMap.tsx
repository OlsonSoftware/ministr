/**
 * DiagnosticsMap — the Explore "Diagnostics" lens: the agentic VERIFY stage.
 *
 * `ministr_diagnostics` runs the project's OWN toolchain(s) — cargo, tsc,
 * eslint, ruff, go vet, … (plus any SARIF-emitting tool) — and returns
 * structured findings, never raw build logs. Every diagnostic is a first-class
 * object: a severity, a rule/error code, a message, the tool that produced it,
 * and (via the FL1 occurrence index) the enclosing symbol. Errors-first,
 * grouped by file: click a row to jump to the error in the code lens; inspect
 * the enclosing symbol in the shared EntityPanel. Language-agnostic by
 * construction — a TypeScript error and a Rust error render identically.
 * Built fresh from the v4 tokens/atoms.
 *
 * Pure `DiagnosticsMap` renders from props (Storybook); `DiagnosticsMapConnector`
 * wires the `diagnostics` invoke + the shared inspector.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CircleAlert,
  FileCode2,
  Info,
  Lightbulb,
  ShieldCheck,
  Stethoscope,
  TriangleAlert,
} from "lucide-react";

import type { Diagnostic, DiagnosticSeverity, SymbolInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { EmptyState } from "../ui/empty-state";

const SEVERITY_ORDER: DiagnosticSeverity[] = ["error", "warning", "info", "hint"];

const SEVERITY_META: Record<
  DiagnosticSeverity,
  { label: string; icon: typeof Info; rank: number; chip: string; dot: string }
> = {
  error: {
    label: "Error",
    icon: CircleAlert,
    rank: 0,
    chip: "border-danger/40 bg-danger/10 text-danger",
    dot: "bg-danger",
  },
  warning: {
    label: "Warning",
    icon: TriangleAlert,
    rank: 1,
    chip: "border-warning/40 bg-warning/10 text-warning",
    dot: "bg-warning",
  },
  info: {
    label: "Info",
    icon: Info,
    rank: 2,
    chip: "border-accent/40 bg-accent/10 text-accent",
    dot: "bg-accent",
  },
  hint: {
    label: "Hint",
    icon: Lightbulb,
    rank: 3,
    chip: "border-border-soft bg-surface text-text-dim",
    dot: "bg-text-dim",
  },
};

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-2).join("/");
}

function baseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

export interface DiagnosticsMapProps {
  diagnostics: Diagnostic[];
  loading?: boolean;
  /** Inspect the enclosing symbol in the shared EntityPanel (FL1 symbol_id). */
  onInspect: (d: Diagnostic) => void;
  /** Jump to the diagnostic's location in the code lens. */
  onOpenFile: (path: string, line: number) => void;
}

export function DiagnosticsMap({
  diagnostics = [],
  loading = false,
  onInspect,
  onOpenFile,
}: DiagnosticsMapProps) {
  const [sevFilter, setSevFilter] = useState<DiagnosticSeverity | null>(null);

  const counts = useMemo(() => {
    const c: Record<DiagnosticSeverity, number> = {
      error: 0,
      warning: 0,
      info: 0,
      hint: 0,
    };
    for (const d of diagnostics) c[d.severity] += 1;
    return c;
  }, [diagnostics]);

  const filtered = useMemo(
    () =>
      sevFilter ? diagnostics.filter((d) => d.severity === sevFilter) : diagnostics,
    [diagnostics, sevFilter],
  );

  // Group by file; files with the most errors (then warnings, then count) first.
  const groups = useMemo(() => {
    const byFile = new Map<string, Diagnostic[]>();
    for (const d of filtered) {
      const arr = byFile.get(d.file);
      if (arr) arr.push(d);
      else byFile.set(d.file, [d]);
    }
    const weight = (ds: Diagnostic[]) => {
      let e = 0;
      let w = 0;
      for (const d of ds) {
        if (d.severity === "error") e += 1;
        else if (d.severity === "warning") w += 1;
      }
      return e * 1_000_000 + w * 1_000 + ds.length;
    };
    for (const arr of byFile.values()) {
      arr.sort(
        (a, b) =>
          SEVERITY_META[a.severity].rank - SEVERITY_META[b.severity].rank ||
          a.line_start - b.line_start,
      );
    }
    return [...byFile.entries()].sort((a, b) => weight(b[1]) - weight(a[1]));
  }, [filtered]);

  if (loading) {
    return (
      <div className="grid h-full place-items-center">
        <span className="font-mono text-sm text-text-dim">
          Running the toolchain<span className="ministr-blink">_</span>
        </span>
      </div>
    );
  }

  if (diagnostics.length === 0) {
    return (
      <div className="grid h-full place-items-center p-6">
        <EmptyState
          icon={ShieldCheck}
          accent
          title="Clean build"
          hint="The project's toolchain reports no compiler or linter findings — or no toolchain was detected for this corpus. Run the verify step after an edit to catch regressions here."
        />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* ── Glance header + severity filters. ──────────────────────────── */}
      <header className="shrink-0 border-b border-border-soft bg-surface px-4 py-3 space-y-2.5">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <div
            className={cn(
              "flex items-center gap-2",
              counts.error > 0
                ? "text-danger"
                : counts.warning > 0
                  ? "text-warning"
                  : "text-success",
            )}
          >
            <Stethoscope className="h-4 w-4" strokeWidth={2} />
            <span className="font-mono text-xs font-bold uppercase tracking-[0.08em]">
              Diagnostics
            </span>
          </div>
          <span className="font-mono text-mono-mini text-text-dim">
            <span
              className={cn(
                "tabular-nums font-semibold",
                counts.error > 0 ? "text-danger" : "text-text",
              )}
            >
              {counts.error}
            </span>{" "}
            errors ·{" "}
            <span
              className={cn(
                "tabular-nums font-semibold",
                counts.warning > 0 ? "text-warning" : "text-text",
              )}
            >
              {counts.warning}
            </span>{" "}
            warnings ·{" "}
            <span className="tabular-nums font-semibold text-text">
              {groups.length}
            </span>{" "}
            files
          </span>
        </div>

        <div className="flex flex-wrap gap-1.5">
          <SevChip
            label="All"
            count={diagnostics.length}
            active={sevFilter === null}
            onClick={() => setSevFilter(null)}
          />
          {SEVERITY_ORDER.filter((s) => counts[s] > 0).map((s) => (
            <SevChip
              key={s}
              label={SEVERITY_META[s].label}
              count={counts[s]}
              tone={s}
              active={sevFilter === s}
              onClick={() => setSevFilter(sevFilter === s ? null : s)}
            />
          ))}
        </div>

        <p className="font-mono text-mono-micro text-text-dim">
          The project&apos;s own toolchain (cargo · tsc · eslint · ruff · go vet
          · …) normalised to one shape — structured findings, never raw logs.
        </p>
      </header>

      {/* ── Diagnostics, grouped by file (errors-first). ───────────────── */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {groups.map(([file, group]) => {
          const fe = group.filter((d) => d.severity === "error").length;
          const fw = group.filter((d) => d.severity === "warning").length;
          return (
            <section
              key={file}
              className="border-b border-border-soft last:border-b-0"
            >
              <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border-soft bg-surface-overlay/95 px-4 py-1.5 backdrop-blur">
                <FileCode2
                  className="h-3.5 w-3.5 text-text-dim"
                  strokeWidth={2}
                />
                <button
                  type="button"
                  onClick={() => onOpenFile(file, group[0]?.line_start ?? 1)}
                  title={`Open ${file}`}
                  className="truncate font-mono text-mono-micro font-bold uppercase tracking-[0.06em] text-text hover:text-accent cursor-pointer transition-colors duration-150"
                >
                  {baseName(file)}
                </button>
                <span className="truncate font-mono text-mono-micro text-text-dim">
                  {fileTail(file)}
                </span>
                <span className="ml-auto flex shrink-0 items-center gap-2 font-mono text-mono-micro tabular-nums">
                  {fe > 0 && <span className="text-danger">{fe} err</span>}
                  {fw > 0 && <span className="text-warning">{fw} warn</span>}
                </span>
              </header>
              <div className="divide-y divide-border-soft/60">
                {group.map((d, i) => (
                  <DiagnosticRow
                    key={`${d.file}:${d.line_start}:${d.col_start}:${i}`}
                    diag={d}
                    onInspect={() => onInspect(d)}
                    onOpenFile={onOpenFile}
                  />
                ))}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function DiagnosticRow({
  diag,
  onInspect,
  onOpenFile,
}: {
  diag: Diagnostic;
  onInspect: () => void;
  onOpenFile: (path: string, line: number) => void;
}) {
  const meta = SEVERITY_META[diag.severity];
  const Icon = meta.icon;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpenFile(diag.file, diag.line_start)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpenFile(diag.file, diag.line_start);
        }
      }}
      title={`Open ${diag.file}:${diag.line_start}`}
      className="group flex items-center gap-2.5 px-4 py-2 cursor-pointer hover:bg-surface-overlay transition-colors duration-150 ease-out"
    >
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-1 rounded border px-1 font-mono text-mono-micro font-semibold uppercase tracking-[0.06em]",
          meta.chip,
        )}
        title={meta.label}
      >
        <Icon className="h-3 w-3" strokeWidth={2.25} />
        {diag.code ?? meta.label}
      </span>
      <span className="truncate font-mono text-xs text-text">
        {diag.message}
      </span>
      <span className="flex-1" />
      <span
        className="shrink-0 rounded-full border border-border-soft px-1.5 font-mono text-mono-micro lowercase tracking-[0.04em] text-text-dim"
        title="Reporting tool"
      >
        {diag.source}
      </span>
      {diag.symbol_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onInspect();
          }}
          title="Inspect the enclosing symbol"
          className="shrink-0 rounded border border-border-soft px-1 font-mono text-mono-micro uppercase tracking-[0.06em] text-text-dim hover:border-accent hover:text-accent cursor-pointer transition-colors duration-150"
        >
          symbol
        </button>
      )}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onOpenFile(diag.file, diag.line_start);
        }}
        title={`Open ${diag.file}:${diag.line_start}`}
        className="shrink-0 font-mono text-mono-micro tabular-nums text-text-dim hover:text-accent cursor-pointer transition-colors duration-150"
      >
        :{diag.line_start}
      </button>
    </div>
  );
}

function SevChip({
  label,
  count,
  tone,
  active,
  onClick,
}: {
  label: string;
  count: number;
  tone?: DiagnosticSeverity;
  active: boolean;
  onClick: () => void;
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
      {tone && (
        <span
          className={cn("h-1.5 w-1.5 rounded-full", SEVERITY_META[tone].dot)}
        />
      )}
      <span className="font-semibold">{label}</span>
      <span className="tabular-nums opacity-70">{count}</span>
    </button>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — fetches diagnostics + wires the shared inspector.

function diagToSymbolInfo(d: Diagnostic): SymbolInfo {
  const id = d.symbol_id ?? "";
  const name = id.split("::").pop() || baseName(d.file);
  return {
    id,
    name,
    kind: "",
    file_path: d.file,
    visibility: "",
    signature: name,
    doc_comment: null,
    module_path: "",
  };
}

export function DiagnosticsMapConnector({
  corpusId,
  onOpenFile,
}: {
  corpusId: string;
  onOpenFile: (path: string, line: number) => void;
}) {
  const { openEntity } = useEntityPanel();
  const [diagnostics, setDiagnostics] = useState<Diagnostic[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDiagnostics(null);
    invoke<Diagnostic[]>("diagnostics", { corpusId, languages: null, limit: 500 })
      .then((r) => {
        if (!cancelled) setDiagnostics(r);
      })
      .catch(() => {
        if (!cancelled) setDiagnostics([]);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  return (
    <DiagnosticsMap
      diagnostics={diagnostics ?? []}
      loading={diagnostics === null}
      onInspect={(d) => {
        if (d.symbol_id) {
          openEntity({ kind: "symbol", corpusId, symbol: diagToSymbolInfo(d) });
        }
      }}
      onOpenFile={onOpenFile}
    />
  );
}
