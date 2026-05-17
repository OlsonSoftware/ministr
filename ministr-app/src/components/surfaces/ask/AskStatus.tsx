import { Loader2 } from "lucide-react";
import { motion } from "motion/react";
import { fadeRise } from "../../../lib/motion";
import { statusLabel, type AskPhaseName } from "./internals";

interface Props {
  phase: AskPhaseName;
  /** When true the status block renders the cache-hit short-circuit. */
  cached?: boolean;
}

/**
 * Plain-English status for the Ask pipeline (3 perceptible states).
 * Choreographed: the strip springs in and the label crossfades as the
 * phase advances.
 */
export function AskStatus({ phase, cached = false }: Props) {
  if (cached && phase === "done") {
    return (
      <motion.div
        variants={fadeRise}
        initial="initial"
        animate="animate"
        className="flex items-center gap-2 rounded-lg border border-accent/50 bg-accent-soft px-3 py-2"
      >
        <span className="h-1.5 w-1.5 rounded-full bg-accent ministr-pulse" />
        <span className="font-mono text-xs font-medium uppercase tracking-[0.06em] text-accent">
          From cache
        </span>
        <span className="font-sans text-xs text-text-dim">
          we already had this one
        </span>
      </motion.div>
    );
  }

  const label = statusLabel(phase);
  if (!label) return null;

  return (
    <motion.div
      variants={fadeRise}
      initial="initial"
      animate="animate"
      className="flex items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2"
      role="status"
      aria-live="polite"
    >
      <Loader2 className="h-3.5 w-3.5 text-accent animate-spin" strokeWidth={2.5} />
      <motion.span
        key={label}
        initial={{ opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        className="font-sans text-sm font-medium text-text"
      >
        {label}
      </motion.span>
    </motion.div>
  );
}
