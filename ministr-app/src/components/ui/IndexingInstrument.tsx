import type { DerivedProgress } from "../../lib/progress";

/**
 * The Indexing Instrument (gui-indexing-instrument) — indexing as a visual
 * instrument, not a sentence. A segmented phase track (discover → parse →
 * embed → finalize) where each segment fills determinately from real daemon
 * counts via [`DerivedProgress`]; the active segment's fill breathes
 * (`pulse-live`, reduced-motion-gated in app.css), fills ease via a
 * motion-safe width transition (static fill under reduced motion), and the
 * whole track resolves to trust-green when the run completes.
 *
 * Honesty rules (§2.5 values never lie): a segment only sweeps
 * indeterminately while its total is genuinely unknown (early discovery);
 * the ETA renders only when the hook supplies one (already gated on a
 * stable rate); a stall says "still working" instead of freezing a number.
 *
 * One component, two variants:
 * - `compact` — the track alone, for inline TrustPanel rows.
 * - `full`    — track + phase/rate/ETA readout + live current-file ticker,
 *               for the Mirror header and ConnectFlow's reading beat.
 */

const PHASES = [
  { key: "discovering", label: "discover" },
  { key: "parsing", label: "parse" },
  { key: "embedding", label: "embed" },
  { key: "finalizing", label: "finalize" },
] as const;

/** Per-segment fill fraction 0..1, or null for honest-indeterminate. */
function segmentFills(p: DerivedProgress): (number | null)[] {
  if (p.complete) return PHASES.map(() => 1);
  const active = PHASES.findIndex((ph) => ph.key === p.phase);
  if (active === -1) return PHASES.map(() => 0); // idle/unknown: empty, no lie
  return PHASES.map((_, i) => {
    if (i < active) return 1;
    if (i > active) return 0;
    return p.percent; // null while the phase total is unknown
  });
}

/** Whole-run fraction for the progressbar value (equal-weight segments). */
function overallFraction(fills: (number | null)[]): number | null {
  if (fills.some((f) => f === null)) return null;
  return (
    (fills as number[]).reduce((acc, f) => acc + f, 0) / PHASES.length
  );
}

function phaseWord(p: DerivedProgress): string {
  if (p.complete) return "up to date";
  // The daemon phases are already gerunds ("discovering"…"finalizing").
  return PHASES.some((ph) => ph.key === p.phase) ? p.phase : "preparing";
}

function readout(p: DerivedProgress): string {
  const parts: string[] = [];
  if (p.ratePerSec !== null && p.running && !p.stalled) {
    const unit =
      p.phase === "embedding" || p.phase === "finalizing"
        ? "embeddings"
        : "files";
    parts.push(`${Math.round(p.ratePerSec)} ${unit}/s`);
  }
  if (p.stalled) parts.push("still working…");
  else if (p.etaSeconds !== null) parts.push(`~${formatEta(p.etaSeconds)} left`);
  return parts.join(" · ");
}

function formatEta(secs: number): string {
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

export function IndexingInstrument({
  progress,
  variant = "full",
}: {
  progress: DerivedProgress;
  variant?: "compact" | "full";
}) {
  const fills = segmentFills(progress);
  const overall = overallFraction(fills);
  const word = phaseWord(progress);
  const sub = readout(progress);
  const valuetext = sub ? `${word} · ${sub}` : word;

  const track = (
    <div
      role="progressbar"
      aria-label="indexing progress"
      aria-valuemin={0}
      aria-valuemax={100}
      {...(overall !== null
        ? { "aria-valuenow": Math.round(overall * 100) }
        : {})}
      aria-valuetext={valuetext}
      className="flex w-full gap-1"
    >
      {PHASES.map((ph, i) => {
        const fill = fills[i];
        const isActive =
          !progress.complete && ph.key === progress.phase && progress.running;
        return (
          <div
            key={ph.key}
            className={`${variant === "compact" ? "h-1" : "h-1.5"} flex-1 overflow-hidden rounded-full bg-sunken`}
          >
            {fill === null ? (
              // Total genuinely unknown: an honest indeterminate sweep
              // (static partial fill under reduced motion, via app.css).
              <div className="beat-sweep h-full w-2/5 rounded-full bg-brand" />
            ) : (
              <div
                className={`h-full rounded-full motion-safe:transition-[width] duration-300 ease-out ${
                  progress.complete ? "bg-ok" : "bg-brand"
                } ${isActive ? "pulse-live" : ""}`}
                style={{ width: `${Math.round(fill * 1000) / 10}%` }}
              />
            )}
          </div>
        );
      })}
    </div>
  );

  if (variant === "compact") return track;

  return (
    <div className="space-y-2">
      <div className="flex items-baseline justify-between gap-4">
        <p className="text-sm text-ink">
          {word}
          {overall !== null ? (
            <span className="ml-2 text-dim">{Math.round(overall * 100)}%</span>
          ) : null}
        </p>
        {sub ? <p className="text-sm text-dim">{sub}</p> : null}
      </div>
      {track}
      {progress.currentFile && progress.running ? (
        <p
          className="truncate font-mono text-xs text-dim"
          aria-label="file being read"
        >
          {progress.currentFile}
        </p>
      ) : null}
    </div>
  );
}
