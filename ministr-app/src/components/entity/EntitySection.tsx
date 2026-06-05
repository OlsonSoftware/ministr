import type { ReactNode } from "react";
import { motion } from "motion/react";
import { fadeRise } from "../../lib/motion";
import { cn } from "../../lib/utils";

interface Props {
  /** Sentence-case section title. */
  title: string;
  /** Optional chapter index — renders as `§N` in the gutter. */
  chapter?: number;
  /** Right-edge count or summary. */
  meta?: string | number;
  /** When true, dims the whole block to indicate empty signal. */
  empty?: boolean;
  children: ReactNode;
}

/**
 * Cockpit section block — rounded panel, hairline border, sans chapter
 * heading, springy mount. Used across every EntityPanel view.
 */
export function EntitySection({
  title,
  chapter,
  meta,
  empty,
  children,
}: Props) {
  return (
    <motion.section
      variants={fadeRise}
      initial="initial"
      animate="animate"
      className="overflow-hidden rounded-lg border border-border bg-surface"
    >
      <header className="flex items-baseline gap-3 border-b border-border bg-surface-overlay px-3.5 py-2.5">
        {chapter !== undefined && (
          <span className="font-mono text-xs font-medium text-accent tabular-nums shrink-0">
            §{chapter}
          </span>
        )}
        <h3 className="font-sans text-base font-semibold tracking-tight text-text leading-snug flex-1 min-w-0">
          {title}
        </h3>
        {meta !== undefined && (
          <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
            {meta}
          </span>
        )}
      </header>
      <div className={cn(empty && "opacity-70")}>{children}</div>
    </motion.section>
  );
}

/** Single-line "loading…" hint for sections waiting on async data. */
export function EntitySectionLoading() {
  return (
    <p className="px-3.5 py-2.5 font-sans text-sm text-text-dim">
      Loading<span className="ministr-blink">_</span>
    </p>
  );
}

/** Single-line "no data" hint for sections that resolved empty. The muted (not
 *  dim) tone keeps the hint above the AA floor even when the parent `empty`
 *  flag softens the whole body. */
export function EntitySectionEmpty({ label }: { label?: string }) {
  return (
    <p className="px-3.5 py-2.5 font-sans text-sm text-text-muted">
      {label ?? "—"}
    </p>
  );
}
