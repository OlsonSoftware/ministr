import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

interface Props {
  /** Sentence-case section title (e.g. "Overview", "References",
   *  "Bridges — export"). Rendered in Plex Serif weight 700.
   *  Legacy callers may pass UPPERCASE strings — they still work, just
   *  the serif heading reads better with sentence case. */
  title: string;
  /** Optional chapter index — renders as `§N` in the gutter alongside
   *  the heading. Field-manual rhythm: §1 Overview, §2 References, etc. */
  chapter?: number;
  /** Right-edge count or summary (e.g. `24` or `0 / 12`). */
  meta?: string | number;
  /** When true, dims the whole block to indicate empty signal. */
  empty?: boolean;
  children: ReactNode;
}

/**
 * Field-manual section block. Hairline border on the container, Plex Serif
 * chapter heading on the header, restrained surface lift on the heading row.
 * No card-level shadow — that gesture is reserved for focused elements.
 */
export function EntitySection({ title, chapter, meta, empty, children }: Props) {
  return (
    <section className="border border-border-soft bg-surface">
      <header className="flex items-baseline gap-3 border-b border-border-soft bg-surface-overlay px-3 py-2">
        {chapter !== undefined && (
          <span className="font-serif text-base font-normal text-text-dim tabular-nums shrink-0 w-6">
            §{chapter}
          </span>
        )}
        <h3 className="font-serif text-base font-bold text-text leading-snug flex-1 min-w-0">
          {title}
        </h3>
        {meta !== undefined && (
          <span className="font-mono text-xs tabular-nums text-text-dim shrink-0">
            {typeof meta === "number" ? meta : meta}
          </span>
        )}
      </header>
      <div className={cn(empty && "opacity-50")}>{children}</div>
    </section>
  );
}

/** Single-line "loading…" hint for sections waiting on async data. */
export function EntitySectionLoading() {
  return (
    <p className="px-3 py-2 font-sans text-sm text-text-dim italic">
      Loading<span className="ministr-blink">_</span>
    </p>
  );
}

/** Single-line "no data" hint for sections that resolved empty. */
export function EntitySectionEmpty({ label }: { label?: string }) {
  return (
    <p className="px-3 py-2 font-sans text-sm text-text-dim italic">
      {label ?? "—"}
    </p>
  );
}
