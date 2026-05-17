import type React from "react";
import { cn } from "../../lib/utils";
import { headingChapter, labelSmallCap } from "../../lib/ui-tokens";

/**
 * Zone — labelled section primitive. Cockpit panel: rounded, hairline,
 * tier-2 header. Two title tones share the same frame so adjacent zones
 * line up:
 * - **mono** (default) — mono-caps label header (stats / key-value).
 * - **serif** — sans heading header (prose-heavy: Settings groups).
 *   (Name kept for compat; "serif" is now the Cockpit sans heading.)
 */
interface ZoneProps {
  title: string;
  subtitle?: string;
  tone?: "mono" | "serif";
  className?: string;
  children: React.ReactNode;
}

export function Zone({
  title,
  subtitle,
  tone = "mono",
  className,
  children,
}: ZoneProps) {
  if (tone === "serif") {
    const t = sentenceCase(title);
    const s = subtitle ? sentenceCase(subtitle) : undefined;
    return (
      <section
        className={cn(
          "overflow-hidden rounded-lg border border-border bg-surface",
          className,
        )}
      >
        <header className="flex items-baseline justify-between gap-3 border-b border-border bg-surface-overlay px-3.5 py-2.5">
          <h3 className={headingChapter}>{t}</h3>
          {s && <span className="font-sans text-xs text-text-dim">{s}</span>}
        </header>
        <div>{children}</div>
      </section>
    );
  }

  return (
    <section
      className={cn(
        "overflow-hidden rounded-lg border border-border bg-surface",
        className,
      )}
    >
      <header className="flex items-center justify-between border-b border-border bg-surface-overlay px-3.5 py-2">
        <span className={labelSmallCap}>{title}</span>
        {subtitle && (
          <span className="font-mono text-xs tracking-[0.08em] text-text-dim">
            {subtitle}
          </span>
        )}
      </header>
      {children}
    </section>
  );
}

/** Convert ALL_CAPS legacy titles to sentence case for heading rendering. */
function sentenceCase(s: string): string {
  if (/^[A-Z][A-Z\s\-—·]+$/.test(s)) {
    return s.charAt(0) + s.slice(1).toLowerCase();
  }
  return s;
}
