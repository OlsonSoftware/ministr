/**
 * DiagnosticsTreemap — the toolchain findings as a HOT-SPOT MAP.
 *
 * DiagnosticsMap lists findings errors-first by file; this is the at-a-glance
 * picture that answers "WHERE are the problems concentrated?" — a bespoke
 * squarified treemap where every file is a rectangle whose AREA is its finding
 * count and whose COLOR is its worst severity (error=danger, warning=warning,
 * info=accent, hint=muted). Big red tiles are the fires. Top-N files by count;
 * the rest fold into a neutral "+N files" tile. Each tile is keyboard-reachable
 * and jumps to the file's first finding (same as a row); hover/focus lifts it.
 *
 * Deterministic squarified layout (Bruls/Huizing/van Wijk) — no physics, no
 * dependency. Pairs with the grouped list (DiagnosticsMap owns the data + the
 * severity filter; this is the additive gestalt).
 */
import { useMemo, useState } from "react";
import type { Diagnostic, DiagnosticSeverity } from "../../lib/types";
import { cn } from "../../lib/utils";

const MAX_TILES = 22; // files shown before the rest fold into "+N files"
// Band aspect (width / height); the squarify ratios are computed in this space
// so tiles read as squares when the band scales to its container width.
const AR = 3.1;

type Sev = DiagnosticSeverity;

const SEV_RANK: Record<Sev, number> = { error: 0, warning: 1, info: 2, hint: 3 };

/** Worst-severity → tile classes. Tone lives on the fill/border (non-text);
 *  the label text stays high-contrast. */
const TILE_TONE: Record<Sev, { fill: string; border: string; hover: string }> = {
  error: { fill: "bg-danger/15", border: "border-danger/45", hover: "hover:border-danger" },
  warning: { fill: "bg-warning/15", border: "border-warning/45", hover: "hover:border-warning" },
  info: { fill: "bg-accent/12", border: "border-accent/40", hover: "hover:border-accent" },
  hint: { fill: "bg-surface-overlay", border: "border-border-soft", hover: "hover:border-border" },
};

interface FileAgg {
  file: string;
  count: number;
  errors: number;
  warnings: number;
  worst: Sev;
  /** Jump target: the first error's line if any, else the earliest line. */
  firstLine: number;
  minLine: number;
  minErrorLine: number | null;
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Aggregate diagnostics per file, sorted by count desc (then errors). */
function aggregate(diagnostics: Diagnostic[]): FileAgg[] {
  const byFile = new Map<string, FileAgg>();
  for (const d of diagnostics) {
    let a = byFile.get(d.file);
    if (!a) {
      a = {
        file: d.file,
        count: 0,
        errors: 0,
        warnings: 0,
        worst: "hint",
        firstLine: d.line_start,
        minLine: d.line_start,
        minErrorLine: null,
      };
      byFile.set(d.file, a);
    }
    a.count += 1;
    if (d.severity === "error") a.errors += 1;
    else if (d.severity === "warning") a.warnings += 1;
    if (SEV_RANK[d.severity] < SEV_RANK[a.worst]) a.worst = d.severity;
    a.minLine = Math.min(a.minLine, d.line_start);
    if (d.severity === "error") {
      a.minErrorLine = a.minErrorLine === null ? d.line_start : Math.min(a.minErrorLine, d.line_start);
    }
  }
  for (const a of byFile.values()) a.firstLine = a.minErrorLine ?? a.minLine;
  return [...byFile.values()].sort(
    (a, b) => b.count - a.count || b.errors - a.errors || a.file.localeCompare(b.file),
  );
}

/** The worst aspect ratio of a row of areas laid against a side of length `s`. */
function worstRatio(areas: number[], s: number): number {
  if (areas.length === 0) return Infinity;
  const sum = areas.reduce((p, c) => p + c, 0);
  const max = Math.max(...areas);
  const min = Math.min(...areas);
  const s2 = s * s;
  const sum2 = sum * sum;
  return Math.max((s2 * max) / sum2, sum2 / (s2 * min));
}

/**
 * Squarified treemap. `values` are tile areas (any scale); they're normalised to
 * the rect's area. Returns one Rect per value, in the rect's coordinate space,
 * packing the rect with no gaps or overlap.
 */
function squarify(values: number[], rect: Rect): Rect[] {
  const out: Rect[] = new Array(values.length);
  const total = values.reduce((p, c) => p + c, 0);
  if (total <= 0) return values.map(() => ({ x: rect.x, y: rect.y, w: 0, h: 0 }));
  const scale = (rect.w * rect.h) / total;
  const areas = values.map((v) => v * scale);

  let { x, y, w, h } = rect;
  let i = 0;
  while (i < areas.length) {
    const side = Math.min(w, h);
    // Greedily grow the current row while the worst aspect ratio improves.
    let row: number[] = [areas[i]];
    let j = i + 1;
    while (j < areas.length) {
      const withNext = [...row, areas[j]];
      if (worstRatio(withNext, side) <= worstRatio(row, side)) {
        row = withNext;
        j++;
      } else break;
    }
    const rowSum = row.reduce((p, c) => p + c, 0);
    if (w >= h) {
      // Lay the row as a column of width = rowSum / h.
      const colW = rowSum / h;
      let yy = y;
      for (let k = 0; k < row.length; k++) {
        const th = row[k] / colW;
        out[i + k] = { x, y: yy, w: colW, h: th };
        yy += th;
      }
      x += colW;
      w -= colW;
    } else {
      const rowH = rowSum / w;
      let xx = x;
      for (let k = 0; k < row.length; k++) {
        const tw = row[k] / rowH;
        out[i + k] = { x: xx, y, w: tw, h: rowH };
        xx += tw;
      }
      y += rowH;
      h -= rowH;
    }
    i += row.length;
  }
  return out;
}

function baseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

export interface DiagnosticsTreemapProps {
  /** The (already severity-filtered) findings — the SAME set the list shows. */
  diagnostics: Diagnostic[];
  /** Jump to a file's first finding (same as a row click). */
  onOpenFile: (path: string, line: number) => void;
}

export function DiagnosticsTreemap({ diagnostics, onOpenFile }: DiagnosticsTreemapProps) {
  const { tiles, fileCount, total } = useMemo(() => {
    const aggs = aggregate(diagnostics);
    const fileCount = aggs.length;
    const total = diagnostics.length;
    let head = aggs;
    let overflow: { files: number; count: number } | null = null;
    if (aggs.length > MAX_TILES) {
      head = aggs.slice(0, MAX_TILES - 1);
      const rest = aggs.slice(MAX_TILES - 1);
      overflow = {
        files: rest.length,
        count: rest.reduce((p, c) => p + c.count, 0),
      };
    }
    const values = [...head.map((a) => a.count), ...(overflow ? [overflow.count] : [])];
    const rects = squarify(values, { x: 0, y: 0, w: AR, h: 1 });
    const tiles = head.map((a, idx) => ({ agg: a, rect: rects[idx], overflow: null as null | typeof overflow }));
    if (overflow) {
      tiles.push({ agg: null as unknown as FileAgg, rect: rects[rects.length - 1], overflow });
    }
    return { tiles, fileCount, total };
  }, [diagnostics]);

  const [hover, setHover] = useState<string | null>(null);

  if (diagnostics.length === 0) return null;

  return (
    <div
      className="relative h-[176px] w-full overflow-hidden rounded-lg border border-border-soft bg-surface-sunken/40"
      role="group"
      aria-label={`Diagnostics hot-spot map: ${total} finding${total === 1 ? "" : "s"} across ${fileCount} file${fileCount === 1 ? "" : "s"}. Each tile is a file sized by its finding count and coloured by its worst severity.`}
    >
      {tiles.map(({ agg, rect, overflow }) => {
        // Convert the AR-space rect to percentages of the container.
        const left = (rect.x / AR) * 100;
        const width = (rect.w / AR) * 100;
        const top = rect.y * 100;
        const height = rect.h * 100;
        const big = width > 13 && height > 22; // room for a label
        const key = overflow ? "__overflow__" : agg.file;

        if (overflow) {
          return (
            <div
              key={key}
              className="absolute grid place-items-center border border-dashed border-border-soft bg-surface-sunken/60 p-1"
              style={{ left: `${left}%`, top: `${top}%`, width: `${width}%`, height: `${height}%` }}
              aria-label={`and ${overflow.files} more files with ${overflow.count} findings`}
            >
              {big && (
                <span className="font-mono text-mono-micro tabular-nums text-text-dim">
                  +{overflow.files} files
                </span>
              )}
            </div>
          );
        }

        const tone = TILE_TONE[agg.worst];
        const lit = !hover || hover === key;
        return (
          <button
            key={key}
            type="button"
            onClick={() => onOpenFile(agg.file, agg.firstLine)}
            onMouseEnter={() => setHover(key)}
            onMouseLeave={() => setHover(null)}
            onFocus={() => setHover(key)}
            onBlur={() => setHover(null)}
            title={`${agg.file} — ${agg.count} finding${agg.count === 1 ? "" : "s"}${agg.errors ? ` · ${agg.errors} error${agg.errors === 1 ? "" : "s"}` : ""}${agg.warnings ? ` · ${agg.warnings} warning${agg.warnings === 1 ? "" : "s"}` : ""}`}
            aria-label={`${baseName(agg.file)}: ${agg.count} finding${agg.count === 1 ? "" : "s"}, ${agg.errors} error${agg.errors === 1 ? "" : "s"}, ${agg.warnings} warning${agg.warnings === 1 ? "" : "s"}; worst severity ${agg.worst}. Open the file.`}
            className={cn(
              "group absolute flex flex-col justify-between overflow-hidden border p-1.5 text-left transition-[opacity,box-shadow] duration-150 ease-out cursor-pointer",
              tone.fill,
              tone.border,
              tone.hover,
              "hover:shadow-[var(--glow-soft)] focus-visible:shadow-[var(--glow-soft)]",
            )}
            style={{
              left: `${left}%`,
              top: `${top}%`,
              width: `${width}%`,
              height: `${height}%`,
              opacity: lit ? 1 : 0.4,
            }}
          >
            {big ? (
              <>
                <span className="truncate font-mono text-mono-micro font-semibold text-text">
                  {baseName(agg.file)}
                </span>
                {/* Tile fill already encodes worst severity; the breakdown text
                    stays high-contrast text-text (small 12px text on a tinted,
                    theme-composited fill is too fragile for text-dim → AA). */}
                <span className="flex items-center gap-1 font-mono text-mono-micro tabular-nums font-semibold text-text">
                  <span>{agg.count}</span>
                  {agg.errors > 0 && <span className="font-normal">·{agg.errors}e</span>}
                  {agg.warnings > 0 && <span className="font-normal">·{agg.warnings}w</span>}
                </span>
              </>
            ) : (
              // Too small for text — a centred count keeps it scannable.
              <span className="m-auto font-mono text-mono-micro tabular-nums font-semibold text-text">
                {agg.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
