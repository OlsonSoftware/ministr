import type { CorpusInfo } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { corpusTone, isCorpusLive } from "../../lib/status";
import { cn } from "../../lib/utils";
import { StatusDot } from "./status-dot";

interface CorpusChipProps {
  corpus: CorpusInfo;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
}

export function CorpusChip({
  corpus,
  selected,
  onClick,
  className,
}: CorpusChipProps) {
  const name = corpusLabel(corpus);
  const state = corpus.status.state;
  const tone = corpusTone(corpus);
  const pct =
    state === "indexing" && corpus.status.files_total > 0
      ? (corpus.status.files_done / corpus.status.files_total) * 100
      : null;

  return (
    <button
      onClick={onClick}
      className={cn(
        "group relative inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-xs font-medium transition-all duration-120 cursor-pointer shrink-0",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-accent-ring)]",
        selected
          ? "border-[var(--color-accent-ring)] bg-[var(--color-accent-soft)] text-accent"
          : "border-border/70 bg-surface-raised/80 text-text hover:border-border-hover hover:bg-surface-overlay/60",
        className,
      )}
      title={corpus.paths.join(" · ")}
    >
      <StatusDot tone={tone} pulse={isCorpusLive(corpus) ? "live" : "off"} />
      <span className="font-mono font-semibold max-w-[140px] truncate">
        {name}
      </span>
      <span className="text-[10px] tabular-nums text-text-dim font-mono">
        {corpus.sections_count.toLocaleString()}
      </span>
      {pct !== null && (
        <span className="text-[10px] tabular-nums text-warning font-mono">
          {pct.toFixed(0)}%
        </span>
      )}
      {corpus.active_sessions > 0 && (
        <span className="text-[10px] tabular-nums text-accent font-mono">
          {corpus.active_sessions}↯
        </span>
      )}
    </button>
  );
}
