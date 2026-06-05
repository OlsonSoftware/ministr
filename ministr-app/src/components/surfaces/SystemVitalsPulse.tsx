/**
 * SystemVitalsPulse — the daemon's MEMORY footprint as a living trend.
 *
 * The Diagnostics deck reads the system's vitals as point-in-time numbers
 * (memory now, uptime, vectors). This adds the missing axis: TIME. A compact
 * area-sparkline of daemon RSS over the recent poll window turns "142 MB" into
 * "142 MB and holding" vs "142 MB and climbing" — the difference between a
 * healthy process and a leak. Paired with a delta indicator (the 2026
 * real-time-dashboard idiom: sparkline for the trend, Δ for the change).
 *
 * Pure + deterministic: renders only from the `series` prop (oldest → newest),
 * so Storybook and the live connector drive the same component. The trend is
 * the shared `Sparkline` atom in `smooth` mode (area fill + current-edge dot,
 * redrawn each poll → reduced-motion-safe by construction). Styled as an inset
 * (bg-surface) so it sits cleanly ON the raised Diagnostics deck.
 */
import { HardDrive } from "@/components/ui/icons";
import { cn } from "../../lib/utils";
import { Sparkline } from "../ui/sparkline";
import { StatusDot } from "../ui/status-dot";

export interface SystemVitalsPulseProps {
  /** Daemon RSS samples in MB, oldest → newest. */
  series: number[];
  className?: string;
}

export function SystemVitalsPulse({ series, className }: SystemVitalsPulseProps) {
  const clean = Array.isArray(series)
    ? series.filter((n) => Number.isFinite(n))
    : [];
  const n = clean.length;
  const current = n > 0 ? clean[n - 1] : null;
  const peak = n > 0 ? Math.max(...clean) : 0;
  const delta = n > 1 ? clean[n - 1] - clean[0] : 0;
  // "Warming" until there are at least two samples to draw a trend between.
  const warming = n < 2;

  const round = (mb: number) => Math.round(mb).toLocaleString();
  const signed = (mb: number) => {
    const r = Math.round(mb) || 0; // collapse -0 → 0
    return `${r >= 0 ? "+" : "−"}${Math.abs(r).toLocaleString()}`;
  };

  const aria = warming
    ? "Daemon memory: collecting samples."
    : `Daemon memory over the last ${n} samples: now ${round(current!)} megabytes, peak ${round(peak)}, ${delta >= 0 ? "up" : "down"} ${Math.abs(Math.round(delta))} since the window start.`;

  return (
    <div
      className={cn(
        "rounded-lg border border-border bg-surface px-3 py-2.5",
        className,
      )}
    >
      {/* Header: label + live dot · readout (now / peak / Δ). */}
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <div className="flex items-center gap-1.5">
          <HardDrive className="h-3 w-3 text-text-dim" strokeWidth={2} />
          <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
            Memory
          </span>
          {!warming && <StatusDot tone="accent" pulse="live" />}
        </div>
        <div className="flex items-baseline gap-3">
          <Stat value={current != null ? round(current) : "—"} label="MB now" />
          <Stat value={n > 0 ? round(peak) : "—"} label="peak" />
          <Stat value={warming ? "—" : signed(delta)} label="Δ window" />
        </div>
      </div>

      {/* The trend. */}
      <div className="mt-2">
        {warming ? (
          <div className="flex h-[52px] items-center justify-center font-mono text-mono-micro tracking-[0.08em] text-text-dim">
            Collecting samples…
          </div>
        ) : (
          <Sparkline
            data={clean}
            smooth
            tone="accent"
            height={52}
            ariaLabel={aria}
          />
        )}
      </div>
    </div>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <span className="flex items-baseline gap-1">
      <span className="font-mono text-xs font-semibold tabular-nums text-text">
        {value}
      </span>
      <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        {label}
      </span>
    </span>
  );
}
