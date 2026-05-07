import { Loader2 } from "lucide-react";
import { cn } from "../../../lib/utils";
import { statusLabel, type AskPhaseName } from "./internals";

interface Props {
  phase: AskPhaseName;
  /** When true the status block renders the cache-hit short-circuit. */
  cached?: boolean;
}

/**
 * Plain-English status indicator for the Ask pipeline.
 *
 * The daemon emits 5+ pipeline phases (analyzing → retrieving → reranking
 * → synthesizing → verifying); the user sees three perceptible states:
 *
 *   - "Thinking…"       (analyze + retrieve + rerank)
 *   - "Writing answer…" (synthesize)
 *   - "Checking sources…" (verify)
 *
 * Internal phase names like "HyDE", "rerank", and "verify" stay in the
 * Developer Tools surface — this is the user-facing strip.
 */
export function AskStatus({ phase, cached = false }: Props) {
  if (cached && phase === "done") {
    return (
      <div className="flex items-center gap-2 border border-accent bg-surface-overlay px-3 py-2">
        <span className="h-1.5 w-1.5 rounded-full bg-accent" />
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.05em] text-accent">
          From cache
        </span>
        <span className="font-serif text-xs italic text-text-dim">
          we already had this one
        </span>
      </div>
    );
  }

  const label = statusLabel(phase);
  if (!label) return null;

  return (
    <div
      className={cn(
        "flex items-center gap-2 border border-border-soft bg-surface px-3 py-2",
      )}
      role="status"
      aria-live="polite"
    >
      <Loader2
        className="h-3.5 w-3.5 text-accent animate-spin"
        strokeWidth={2.5}
      />
      <span className="font-sans text-sm font-medium text-text">{label}</span>
    </div>
  );
}
