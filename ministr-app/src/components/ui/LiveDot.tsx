/**
 * LiveDot — presence (DESIGN.md §7). A breathing brand dot paired with a
 * word, because the dot may never be the only signal (§2.1). Motion-safe:
 * the pulse only runs when the user allows motion.
 */
export function LiveDot({ label = "live" }: { label?: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-sm text-dim">
      <span aria-hidden className="size-2 rounded-full bg-brand pulse-live" />
      {label}
    </span>
  );
}
