/**
 * DiffRipple — a branch diff's blast radius as a RIPPLE.
 *
 * ChangesMap lists what a range changed (with git-blame authors) and what it can
 * break (the union blast radius by call-graph depth); this is the at-a-glance
 * picture that SHOWS the blast. A bespoke deterministic SVG radial wave: the
 * change sits at the core (risk-toned); the changed seed symbols ring it as
 * author-coloured dots; impacted symbols ripple OUTWARD on concentric rings by
 * call-graph depth — a dense outer ripple = a far-reaching change. Every dot is
 * keyboard-reachable and inspects its symbol (same as a row).
 *
 * No physics, no dependency — deterministic polar layout. Pairs with the lists
 * (ChangesMap owns the data; this is the additive gestalt).
 */
import { useMemo, useState } from "react";
import { GitCompareArrows, Waypoints } from "@/components/ui/icons";
import type { ChangedSymbol, DiffImpact, ImpactedSymbol } from "../../lib/types";
import { cn } from "../../lib/utils";
import { VizFrame } from "../ui/viz-frame";

// ── Layout (SVG user units; the svg scales to its container width). ──────────
const W = 460;
const H = 300;
const CX = W / 2;
const CY = H / 2;
const CORE_R = 26;
const SEED_R = CORE_R + 15; // radius of the changed-seed dot ring
const MARGIN = 30;
const MAX_R = Math.min(CX, CY) - MARGIN;
const RING_CAP = 24; // impacted dots per depth ring before a "+N" marker
const SEED_CAP = 12;
const TAU = Math.PI * 2;

type Risk = DiffImpact["risk"];

/** Risk → the tone painted on the core ring + halo (a NON-TEXT mark, so it
 *  encodes risk without a tone-text-on-tint AA problem; the textual risk pill
 *  lives in the lens's GlanceRow above). */
const RISK_RING: Record<Risk, string> = {
  low: "text-success",
  medium: "text-warning",
  high: "text-danger",
};
/** Risk as a NON-TEXT dot (tone on a mark, never tone-text-on-tint → AA-safe).
 *  The textual risk pill lives in the lens's GlanceRow above. */
const RISK_DOT: Record<Risk, string> = { low: "bg-success", medium: "bg-warning", high: "bg-danger" };

/** Deterministic hue from a name (mirrors ChangesMap's author chips) so each
 *  author keeps a stable colour across the ripple + the list. */
function authorHue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i += 1) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

function fileTail(path: string): string {
  return path.replace(/\\/g, "/").split("/").slice(-1)[0] ?? path;
}

function polar(r: number, angle: number): { x: number; y: number } {
  return { x: CX + r * Math.cos(angle), y: CY + r * Math.sin(angle) };
}

interface Dot {
  id: string;
  x: number;
  y: number;
  name: string;
  caption: string;
  ariaLabel: string;
  onClick: () => void;
  /** Author-hue fill for seed dots; impacted dots use the accent class. */
  fillHsl?: string;
}

export interface DiffRippleProps {
  data: DiffImpact;
  onInspect: (symbolId: string, name: string, kind: string, file: string) => void;
}

export function DiffRipple({ data, onInspect }: DiffRippleProps) {
  const { seeds, rings, maxDepth } = useMemo(() => {
    // Seed (changed) dots ring the core, coloured by last-touch author.
    const seedSyms = data.changed_symbols.slice(0, SEED_CAP);
    const seeds: Dot[] = seedSyms.map((s: ChangedSymbol, i) => {
      const angle = -Math.PI / 2 + (i / Math.max(seedSyms.length, 1)) * TAU;
      const { x, y } = polar(SEED_R, angle);
      const author = s.last_author ?? s.authors[0]?.name ?? "unknown";
      return {
        id: s.symbol_id || `seed:${s.file}:${s.line}`,
        x,
        y,
        name: s.name,
        caption: `${s.name} · changed by ${author}`,
        ariaLabel: `${s.name}: changed in this diff, last touched by ${author}. Inspect.`,
        onClick: () => onInspect(s.symbol_id, s.name, s.kind, s.file),
        fillHsl: `hsl(${authorHue(author)} 60% 60%)`,
      };
    });

    // Impacted dots ripple outward, grouped onto a ring per call-graph depth.
    const byDepth = new Map<number, ImpactedSymbol[]>();
    for (const s of data.impacted) {
      const arr = byDepth.get(s.depth);
      if (arr) arr.push(s);
      else byDepth.set(s.depth, [s]);
    }
    const depths = [...byDepth.keys()].sort((a, b) => a - b);
    const maxDepth = depths.length ? depths[depths.length - 1] : 1;
    const ringR = (d: number) => SEED_R + ((MAX_R - SEED_R) * d) / maxDepth;

    const rings = depths.map((d) => {
      const syms = byDepth.get(d)!;
      const shown = syms.slice(0, RING_CAP);
      const start = -Math.PI / 2 + d * 0.45; // offset each ring so dots don't align into spokes
      const dots: Dot[] = shown.map((s, i) => {
        const angle = start + (i / Math.max(shown.length, 1)) * TAU;
        const { x, y } = polar(ringR(d), angle);
        return {
          id: s.symbol_id || `imp:${s.file}:${s.line}`,
          x,
          y,
          name: s.name,
          caption: `${s.name} · ${fileTail(s.file)} · ${d} hop${d === 1 ? "" : "s"} away`,
          ariaLabel: `${s.name}: impacted, ${d} hop${d === 1 ? "" : "s"} from a changed symbol. Inspect.`,
          onClick: () => onInspect(s.symbol_id, s.name, s.kind, s.file),
        };
      });
      return { depth: d, r: ringR(d), dots, overflow: syms.length - shown.length };
    });

    return { seeds, rings, maxDepth };
  }, [data, onInspect]);

  const [hover, setHover] = useState<Dot | null>(null);
  const ring = RISK_RING[data.risk];
  const isolated = data.impacted.length === 0;

  return (
    <VizFrame
      icon={GitCompareArrows}
      label="Blast ripple"
      readout={
        <>
          <span className="flex items-center gap-1.5" title="Aggregate blast-radius risk">
            <span className={cn("inline-block h-2 w-2 rounded-full", RISK_DOT[data.risk])} aria-hidden />
            <span className="font-semibold uppercase tracking-[0.06em] text-text">{data.risk} risk</span>
          </span>
          <span className="flex items-center gap-1">
            <span className="tabular-nums font-semibold text-text">{data.changed_symbols.length}</span>
            changed
          </span>
          <span aria-hidden className="text-border">→</span>
          <span className="flex items-center gap-1">
            <Waypoints className="h-3 w-3 text-accent" strokeWidth={2} />
            <span className="tabular-nums font-semibold text-text">{data.impacted_symbols}</span>
            impacted
          </span>
          <span aria-hidden className="text-border">·</span>
          <span className="flex items-center gap-1">
            <span className="tabular-nums font-semibold text-text">{isolated ? 0 : maxDepth}</span>
            hop{maxDepth === 1 ? "" : "s"}
          </span>
          {data.impacted_tests > 0 && (
            <>
              <span aria-hidden className="text-border">·</span>
              <span className="flex items-center gap-1 text-success">
                <span className="tabular-nums font-semibold">{data.impacted_tests}</span> tests
              </span>
            </>
          )}
        </>
      }
    >
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full"
        style={{ maxHeight: H }}
        role="group"
        aria-label={
          isolated
            ? `Blast ripple: ${data.changed_symbols.length} changed symbol${data.changed_symbols.length === 1 ? "" : "s"} — an isolated change, nothing else references it.`
            : `Blast ripple: ${data.changed_symbols.length} changed symbol${data.changed_symbols.length === 1 ? "" : "s"} reach ${data.impacted_symbols} impacted symbol${data.impacted_symbols === 1 ? "" : "s"} across ${maxDepth} call-graph hop${maxDepth === 1 ? "" : "s"}. Risk ${data.risk}.`
        }
      >
        {/* Guide rings + hop labels (under the dots). */}
        <g className="text-border" fill="none" stroke="currentColor">
          {rings.map((r) => (
            <circle key={`g:${r.depth}`} cx={CX} cy={CY} r={r.r} strokeWidth={1} strokeDasharray="2 4" opacity={0.85} />
          ))}
        </g>
        {rings.map((r) => (
          <text
            key={`hl:${r.depth}`}
            x={CX}
            y={CY - r.r - 3}
            textAnchor="middle"
            className="fill-text-dim font-mono"
            style={{ fontSize: 8 }}
          >
            {r.depth} hop{r.depth === 1 ? "" : "s"}
          </text>
        ))}

        {/* Impacted ripple dots. */}
        {rings.map((r) =>
          r.dots.map((d) => (
            <RippleDot key={d.id} dot={d} hovered={hover?.id === d.id} onHover={setHover} accent />
          )),
        )}
        {/* Per-ring overflow markers. */}
        {rings.map((r) =>
          r.overflow > 0 ? (
            <text
              key={`ov:${r.depth}`}
              x={polar(r.r, -Math.PI / 2 + r.depth * 0.45 - 0.18).x}
              y={polar(r.r, -Math.PI / 2 + r.depth * 0.45 - 0.18).y}
              textAnchor="middle"
              className="fill-text-dim font-mono tabular-nums"
              style={{ fontSize: 8 }}
            >
              +{r.overflow}
            </text>
          ) : null,
        )}

        {/* The change core — risk-toned halo + ring. */}
        <g className={ring}>
          <circle cx={CX} cy={CY} r={CORE_R + 11} fill="currentColor" opacity={0.12} />
          <circle cx={CX} cy={CY} r={CORE_R} className="fill-surface-raised" stroke="currentColor" strokeWidth={1.5} />
        </g>
        <text x={CX} y={CY - 1} textAnchor="middle" className="fill-text font-mono" style={{ fontSize: 17, fontWeight: 700 }}>
          {data.changed_symbols.length}
        </text>
        <text x={CX} y={CY + 11} textAnchor="middle" className="fill-text-dim font-mono" style={{ fontSize: 7.5, letterSpacing: 0.4 }}>
          changed
        </text>

        {/* Changed-seed dots (author-coloured), on top. */}
        {seeds.map((d) => (
          <RippleDot key={d.id} dot={d} hovered={hover?.id === d.id} onHover={setHover} seed />
        ))}

        {isolated && (
          <text x={CX} y={H - 10} textAnchor="middle" className="fill-text-dim font-mono" style={{ fontSize: 10 }}>
            Isolated change — nothing else references it.
          </text>
        )}

        {/* Hover caption (bottom-anchored so it never occludes the wave). */}
        {hover && !isolated && (
          <text x={CX} y={H - 8} textAnchor="middle" className="fill-text font-mono" style={{ fontSize: 10.5 }}>
            {hover.caption}
          </text>
        )}
      </svg>
    </VizFrame>
  );
}

function RippleDot({
  dot,
  hovered,
  onHover,
  seed = false,
  accent = false,
}: {
  dot: Dot;
  hovered: boolean;
  onHover: (d: Dot | null) => void;
  seed?: boolean;
  accent?: boolean;
}) {
  const r = (seed ? 4.5 : 4) * (hovered ? 1.5 : 1);
  return (
    <g
      role="button"
      tabIndex={0}
      aria-label={dot.ariaLabel}
      className="cursor-pointer"
      style={{ transition: "r 120ms ease-out" }}
      onMouseEnter={() => onHover(dot)}
      onMouseLeave={() => onHover(null)}
      onFocus={() => onHover(dot)}
      onBlur={() => onHover(null)}
      onClick={dot.onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          dot.onClick();
        }
      }}
    >
      <title>{dot.caption}</title>
      <circle
        cx={dot.x}
        cy={dot.y}
        r={r}
        className={cn(
          accent && "fill-accent",
          "stroke-surface-raised",
        )}
        style={seed ? { fill: dot.fillHsl } : undefined}
        strokeWidth={1.25}
      />
    </g>
  );
}
