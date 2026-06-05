/**
 * CodebaseConstellation — the SHAPE of the codebase, at a glance.
 *
 * The Observatory shows identity, size, languages and notable files; this is
 * the structural picture: the indexed files grouped into top-level modules,
 * each a BUBBLE whose area is its index mass (section count) and which is
 * deterministically PACKED into a constellation (largest at the centre, the
 * rest spiral out collision-free). Big modules dominate the field; click a
 * bubble to open its largest file. A fresh enclosure/packing idiom — no
 * physics, no dependency, no backend (reuses the file list the Observatory
 * already holds).
 */
import { useMemo, useState } from "react";
import type { FileInfo } from "../../lib/types";
import { cn } from "../../lib/utils";
import { VizFrame } from "../ui/viz-frame";

// ── Layout (SVG user units; the svg scales to its container width). ──────────
const W = 600;
const H = 300;
const PAD = 20;
const MIN_R = 9;
const MAX_R = 56;
const GAP = 3;
const MAX_BUBBLES = 18;
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5)); // ≈2.39996 — phyllotaxis
const RADIAL_STEP = 1.1;
const MAX_SPIRAL_STEPS = 6000;

/** Path segments that are generic roots — descend past them so monorepo crates
 *  AND src-rooted apps both group into meaningful modules. */
const GENERIC_ROOTS = new Set(["src", "lib", "app", "packages", "crates", "source", "pkg"]);

function moduleKey(path: string, deeper: boolean): string {
  const segs = path.replace(/\\/g, "/").split("/").filter(Boolean);
  if (segs.length <= 1) return "(root)";
  let i = 0;
  if (GENERIC_ROOTS.has(segs[0]) && segs.length > 1) i = 1;
  if (deeper && segs.length > i + 1) i += 1;
  return segs[i] ?? segs[segs.length - 1];
}

interface ModuleAgg {
  name: string;
  files: number;
  sections: number;
  largestFile: string;
  largestSections: number;
}

interface Bubble extends ModuleAgg {
  x: number;
  y: number;
  r: number;
  rank: number;
  /** Display label — the shared module prefix stripped so e.g. ministr-core,
   *  ministr-app read as core, app inside the bubbles. */
  display: string;
}

/** The DOMINANT module prefix — the first token (up to the first `-`/`/`) that
 *  a majority of modules share, so e.g. ministr-core / ministr-app read as core
 *  / app while outliers (docs, eval) keep their full name. */
function commonModulePrefix(names: string[]): string {
  if (names.length < 3) return "";
  const tally = new Map<string, number>();
  for (const n of names) {
    const m = n.match(/^.+?[-/]/); // up to (and including) the FIRST - or /
    if (m) tally.set(m[0], (tally.get(m[0]) ?? 0) + 1);
  }
  let best = "";
  let bestCount = 0;
  for (const [p, c] of tally) {
    if (c > bestCount) {
      best = p;
      bestCount = c;
    }
  }
  return bestCount >= Math.ceil(names.length / 2) && best.length >= 3 ? best : "";
}

interface Layout {
  bubbles: Bubble[];
  moduleCount: number;
  fileCount: number;
  overflow: number;
}

function aggregate(files: FileInfo[], deeper: boolean): Map<string, ModuleAgg> {
  const m = new Map<string, ModuleAgg>();
  for (const f of files) {
    const key = moduleKey(f.path, deeper);
    let a = m.get(key);
    if (!a) {
      a = { name: key, files: 0, sections: 0, largestFile: f.path, largestSections: -1 };
      m.set(key, a);
    }
    a.files += 1;
    a.sections += f.section_count;
    if (f.section_count > a.largestSections) {
      a.largestSections = f.section_count;
      a.largestFile = f.path;
    }
  }
  return m;
}

/** Deterministic circle pack: largest bubble at the centre, each next placed at
 *  the first collision-free point along a golden-angle spiral. Then the whole
 *  cluster is uniformly scaled to fit + centred in the viewBox. */
function buildLayout(files: FileInfo[]): Layout {
  if (files.length === 0) return { bubbles: [], moduleCount: 0, fileCount: 0, overflow: 0 };

  let aggs = [...aggregate(files, false).values()];
  // Adaptive: a single generic root collapses everything into <3 modules —
  // regroup one level deeper so the structure actually reads.
  if (aggs.length < 3) {
    const deep = [...aggregate(files, true).values()];
    if (deep.length > aggs.length) aggs = deep;
  }
  aggs.sort((a, b) => b.sections - a.sections || b.files - a.files || a.name.localeCompare(b.name));

  const moduleCount = aggs.length;
  const overflow = Math.max(0, aggs.length - MAX_BUBBLES);
  const head = aggs.slice(0, MAX_BUBBLES);
  const maxSections = head[0]?.sections || 1;
  const prefix = commonModulePrefix(head.map((a) => a.name));

  // Radius ∝ sqrt(sections) (area-proportional), floored at MIN_R.
  const raw: Bubble[] = head.map((a, rank) => ({
    ...a,
    rank,
    display: prefix && a.name.startsWith(prefix) ? a.name.slice(prefix.length) : a.name,
    r: MIN_R + (MAX_R - MIN_R) * Math.sqrt(a.sections / maxSections),
    x: 0,
    y: 0,
  }));

  // Spiral pack (input already sorted desc by r since sections desc).
  const placed: Bubble[] = [];
  for (let k = 0; k < raw.length; k++) {
    const c = raw[k];
    if (k === 0) {
      c.x = 0;
      c.y = 0;
      placed.push(c);
      continue;
    }
    let found = false;
    for (let step = 0; step < MAX_SPIRAL_STEPS; step++) {
      const rad = step * RADIAL_STEP;
      const ang = step * GOLDEN_ANGLE;
      const x = Math.cos(ang) * rad;
      const y = Math.sin(ang) * rad;
      let ok = true;
      for (const p of placed) {
        if (Math.hypot(x - p.x, y - p.y) < c.r + p.r + GAP) {
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
      c.x = (k % 2 ? 1 : -1) * MAX_R * 2 * k;
      c.y = 0;
    }
    placed.push(c);
  }

  // Fit + centre: uniform scale so the packed bbox fills the viewBox.
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
  const scale = Math.min((W - 2 * PAD) / bw, (H - 2 * PAD) / bh);
  const cxBox = (minX + maxX) / 2;
  const cyBox = (minY + maxY) / 2;
  const bubbles = placed.map((c) => ({
    ...c,
    x: W / 2 + (c.x - cxBox) * scale,
    y: H / 2 + (c.y - cyBox) * scale,
    r: c.r * scale,
  }));

  return { bubbles, moduleCount, fileCount: files.length, overflow };
}

/** How many top-level modules the adaptive grouping finds — the Observatory
 *  gates the STRUCTURE section on this so a single-module corpus shows nothing. */
export function structureModuleCount(files: FileInfo[]): number {
  if (files.length === 0) return 0;
  let n = aggregate(files, false).size;
  if (n < 3) {
    const d = aggregate(files, true).size;
    if (d > n) n = d;
  }
  return n;
}

function baseName(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

/** Truncate a module name to the bubble's text room (~5px per mono char at 10px). */
function fit(name: string, r: number): string {
  const max = Math.max(3, Math.floor((r * 1.7) / 5.4));
  return name.length > max ? `${name.slice(0, max - 1)}…` : name;
}

export interface CodebaseConstellationProps {
  files: FileInfo[];
  /** Open a file in the code lens (the module's largest file). */
  onOpen: (path: string) => void;
}

export function CodebaseConstellation({ files, onOpen }: CodebaseConstellationProps) {
  const layout = useMemo(() => buildLayout(files), [files]);
  const [hover, setHover] = useState<string | null>(null);

  if (layout.bubbles.length < 2) return null;

  const hovered = hover ? layout.bubbles.find((b) => b.name === hover) : null;
  const n = layout.bubbles.length;

  return (
    <VizFrame
      readout={
        <>
          <span>
            <span className="tabular-nums font-semibold text-text">{layout.moduleCount}</span> modules ·{" "}
            <span className="tabular-nums font-semibold text-text">{layout.fileCount.toLocaleString()}</span> files
          </span>
          <span>sized by index mass</span>
        </>
      }
    >
      <div className="-mx-1">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          className="w-full"
          style={{ maxHeight: H }}
          role="group"
          aria-label={`Codebase structure: ${layout.fileCount} files across ${layout.moduleCount} top-level modules, drawn as bubbles sized by their index mass.`}
        >
          {layout.bubbles.map((b) => {
            const lit = !hover || hover === b.name;
            const big = b.r > 26;
            const mid = b.r > 16;
            return (
              <g
                key={b.name}
                role="button"
                tabIndex={0}
                aria-label={`${b.name}: ${b.files} file${b.files === 1 ? "" : "s"}, ${b.sections.toLocaleString()} sections. Open ${baseName(b.largestFile)}.`}
                className="cursor-pointer text-accent"
                style={{ opacity: lit ? 1 : 0.35, transition: "opacity 150ms ease-out" }}
                onMouseEnter={() => setHover(b.name)}
                onMouseLeave={() => setHover(null)}
                onFocus={() => setHover(b.name)}
                onBlur={() => setHover(null)}
                onClick={() => onOpen(b.largestFile)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onOpen(b.largestFile);
                  }
                }}
              >
                <title>{`${b.name} · ${b.files} files · ${b.sections.toLocaleString()} sections`}</title>
                <circle
                  cx={b.x}
                  cy={b.y}
                  r={b.r}
                  fill="currentColor"
                  fillOpacity={0.12 + (1 - b.rank / n) * 0.18}
                  stroke="currentColor"
                  strokeOpacity={hover === b.name ? 1 : 0.55}
                  strokeWidth={1.25}
                />
                {big ? (
                  <>
                    <text
                      x={b.x}
                      y={b.y - 1}
                      textAnchor="middle"
                      className="fill-text font-mono font-semibold"
                      style={{ fontSize: 10.5 }}
                    >
                      {fit(b.display, b.r)}
                    </text>
                    <text
                      x={b.x}
                      y={b.y + 11}
                      textAnchor="middle"
                      className="fill-text-dim font-mono tabular-nums"
                      style={{ fontSize: 8.5 }}
                    >
                      {b.files} files
                    </text>
                  </>
                ) : mid ? (
                  <text
                    x={b.x}
                    y={b.y + 3}
                    textAnchor="middle"
                    className="fill-text font-mono font-semibold"
                    style={{ fontSize: 8.5 }}
                  >
                    {fit(b.display, b.r)}
                  </text>
                ) : null}
              </g>
            );
          })}

          {/* Hover caption — bottom-anchored so it never occludes the field. */}
          {hovered && (
            <text
              x={W / 2}
              y={H - 7}
              textAnchor="middle"
              className="fill-text font-mono"
              style={{ fontSize: 10.5 }}
            >
              {`${hovered.name} · ${hovered.files} files · ${hovered.sections.toLocaleString()} sections`}
            </text>
          )}
        </svg>
      </div>

      {layout.overflow > 0 && (
        <p className="font-mono text-mono-micro text-text-dim">
          +{layout.overflow} smaller module{layout.overflow === 1 ? "" : "s"} not shown
        </p>
      )}
    </VizFrame>
  );
}
