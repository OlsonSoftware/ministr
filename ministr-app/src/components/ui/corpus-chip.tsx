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
        "group relative inline-flex items-center gap-2 border-2 px-3 py-1.5 text-xs font-medium cursor-pointer shrink-0 transition-none",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        selected
          ? "border-border bg-accent text-[var(--color-accent-fg-on)] shadow-sm"
          : "border-border bg-surface text-text hover:bg-surface-overlay hover:text-text",
        className,
      )}
      title={corpus.paths.join(" · ")}
    >
      <StatusDot tone={tone} pulse={isCorpusLive(corpus) ? "live" : "off"} />
      <span className="font-mono font-bold tracking-[0.05em] max-w-[140px] truncate">
        {name}
      </span>
      <span className="text-xs tabular-nums font-mono opacity-80">
        {corpus.sections_count.toLocaleString()}
      </span>
      {pct !== null && (
        <span className="text-xs tabular-nums font-mono">
          {pct.toFixed(0)}%
        </span>
      )}
      {corpus.active_sessions > 0 && (
        <span className="text-xs tabular-nums font-mono">
          {corpus.active_sessions}↯
        </span>
      )}
    </button>
  );
}
