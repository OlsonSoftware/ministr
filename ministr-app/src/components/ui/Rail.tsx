import type { ReactNode } from "react";

/**
 * RailSection / RailRow — config-where-you-look (DESIGN.md §7). The rail
 * label is the design system's ONLY sanctioned uppercase (§5).
 */
export function RailSection({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <section aria-label={label} className="space-y-1">
      <h3 className="px-2 text-xs uppercase tracking-[0.08em] text-dim">
        {label}
      </h3>
      <div className="rounded-lg border border-line bg-surface">{children}</div>
    </section>
  );
}

export function RailRow({
  label,
  children,
}: {
  label: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-line px-3 py-2 text-sm last:border-b-0">
      <span className="text-ink">{label}</span>
      {children ? <span className="shrink-0 text-dim">{children}</span> : null}
    </div>
  );
}
