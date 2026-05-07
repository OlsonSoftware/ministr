import type React from "react";
import { cn } from "../../lib/utils";

/**
 * Zone — labelled section primitive used across the app's "field manual"
 * surfaces (Projects, Settings, future Diagnostics, etc.).
 *
 * Two tones, both sharing the same outer frame so adjacent zones still
 * line up:
 *
 * - **mono** (default) — compact mono-caps title with a thicker header
 *   underline. The original ProjectDetail look; right for stats /
 *   key-value zones where the header is a label, not a heading.
 * - **serif** — Plex Serif sentence-case title with a softer underline.
 *   Right for prose-heavy zones (Settings groups, prefs).
 *
 * Both tones auto-sentence-case ALL_CAPS titles when `tone === "serif"`,
 * so legacy callers passing "PREFERENCES" still render correctly.
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
      <section className={cn("border border-border-soft bg-surface", className)}>
        <header className="flex items-baseline justify-between gap-3 border-b border-border-soft bg-surface-overlay px-3 py-2">
          <h3 className="font-serif text-base font-bold text-text">{t}</h3>
          {s && (
            <span className="font-serif text-xs italic text-text-dim">{s}</span>
          )}
        </header>
        <div>{children}</div>
      </section>
    );
  }

  return (
    <section className={cn("border border-border-soft bg-surface", className)}>
      <header className="flex items-center justify-between border-b-2 border-border bg-surface-overlay px-3 py-1.5">
        <span className="font-mono text-[0.6875rem] font-bold tracking-[0.05em] text-text">
          {title}
        </span>
        {subtitle && (
          <span className="font-mono text-xs tracking-[0.05em] text-text-dim">
            {subtitle}
          </span>
        )}
      </header>
      {children}
    </section>
  );
}

/** Convert ALL_CAPS legacy titles to sentence case for serif rendering. */
function sentenceCase(s: string): string {
  if (/^[A-Z][A-Z\s\-—·]+$/.test(s)) {
    return s.charAt(0) + s.slice(1).toLowerCase();
  }
  return s;
}
