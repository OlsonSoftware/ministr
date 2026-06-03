import type { CorpusInfo } from "../../lib/types";
import { toCorpusViewModel } from "../../lib/corpusFleet";
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
  // Single source of truth: the shared corpus view model. Phase-aware %
  // (files while parsing, vectors while embedding) instead of a files-only
  // ratio computed inline.
  const vm = toCorpusViewModel(corpus);
  const tone = corpusTone(corpus);
  const pct = vm.isIndexing ? vm.primary.pct : null;

  return (
    <button
      onClick={onClick}
      className={cn(
        "group relative inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-xs font-medium cursor-pointer shrink-0",
        "transition-colors duration-150 ease-out",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        selected
          ? "border-accent bg-accent text-[var(--color-accent-fg-on)] shadow-[var(--glow-soft)]"
          : "border-border bg-surface text-text hover:bg-surface-overlay hover:border-border-hover",
        className,
      )}
      title={corpus.paths.join(" · ")}
    >
      <StatusDot tone={tone} pulse={isCorpusLive(corpus) ? "live" : "off"} />
      <span className="font-mono font-semibold tracking-[0.04em] max-w-[140px] truncate">
        {vm.label}
      </span>
      <span className="text-xs tabular-nums font-mono opacity-80">
        {vm.sectionsIndexed.toLocaleString()}
      </span>
      {pct !== null && (
        <span className="text-xs tabular-nums font-mono">
          {pct.toFixed(0)}%
        </span>
      )}
      {vm.sessions > 0 && (
        <span className="text-xs tabular-nums font-mono">
          {vm.sessions}↯
        </span>
      )}
    </button>
  );
}
