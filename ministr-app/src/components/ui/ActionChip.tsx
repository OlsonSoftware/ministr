import type { ButtonHTMLAttributes } from "react";

/**
 * ActionChip — the one button (DESIGN.md §7). Labels state their cost
 * ("Catch up · ~40s", §8). `primary` is the brand-inked emphasis; `quiet`
 * is the default furniture.
 */
export function ActionChip({
  variant = "quiet",
  busy = false,
  className = "",
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "quiet";
  /** In-flight: disabled + a turning mark. Never fire-and-forget. */
  busy?: boolean;
}) {
  const look =
    variant === "primary"
      ? "border-brand text-brand font-medium"
      : "border-line text-ink";
  return (
    <button
      type="button"
      disabled={busy || props.disabled}
      aria-busy={busy || undefined}
      className={`rounded-md border bg-surface px-3 py-1.5 text-sm transition-colors hover:bg-sunken focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand disabled:opacity-60 ${look} ${className}`}
      {...props}
    >
      {busy ? (
        <span aria-hidden className="mr-1.5 inline-block pulse-live">
          ⟳
        </span>
      ) : null}
      {children}
    </button>
  );
}
