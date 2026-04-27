import type { CorpusInfo } from "../../lib/types";
import { corpusLabel } from "../../lib/corpus";
import { cn } from "../../lib/utils";

interface CorpusSelectProps {
  /** Currently-selected corpus id. */
  value: string;
  onChange: (id: string) => void;
  corpora: readonly CorpusInfo[];
  /** Disable the control while a parent is loading. */
  disabled?: boolean;
  className?: string;
  /** Optional aria-label for screen readers when there's no visible
   *  label nearby (most call sites are flanked by enough context that
   *  a label is unnecessary). */
  ariaLabel?: string;
}

/** Native `<select>` of the daemon's corpora, rendering the human
 *  label via `lib/corpus.ts::corpusLabel`. The styling matches the
 *  app's other compact form controls. */
export function CorpusSelect({
  value,
  onChange,
  corpora,
  disabled,
  className,
  ariaLabel,
}: CorpusSelectProps) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
      aria-label={ariaLabel}
      className={cn(
        "h-8 rounded-md border border-border/70 bg-surface-raised px-2.5 text-xs font-mono text-text cursor-pointer",
        "focus:outline-none focus:border-[var(--color-accent-ring)]",
        "disabled:cursor-not-allowed disabled:opacity-60",
        className,
      )}
    >
      {corpora.map((c) => (
        <option key={c.id} value={c.id}>
          {corpusLabel(c)}
        </option>
      ))}
    </select>
  );
}
