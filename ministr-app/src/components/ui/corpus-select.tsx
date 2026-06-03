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
  ariaLabel?: string;
}

/** Native `<select>` of the daemon's corpora — hairline border, soft corners,
 *  mono caps (§4/§6). Native dropdown (OS-rendered), so no glass tier applies. */
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
        "h-9 rounded-md border border-border bg-surface px-2.5 text-xs font-mono font-medium uppercase tracking-[0.08em] text-text cursor-pointer",
        "transition-colors duration-150 ease-out",
        // §9 WCAG 2.4.13 — keep the global focus-visible ring; add an
        // accent border/surface on keyboard focus (not mouse).
        "focus-visible:border-accent focus-visible:bg-surface-overlay",
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
