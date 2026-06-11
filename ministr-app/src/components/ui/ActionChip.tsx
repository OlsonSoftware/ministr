import type { ButtonHTMLAttributes } from "react";

/**
 * ActionChip — the one button (DESIGN.md §7). Labels state their cost
 * ("Catch up · ~40s", §8). `primary` is the brand-inked emphasis; `quiet`
 * is the default furniture.
 */
export function ActionChip({
  variant = "quiet",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "quiet";
}) {
  const look =
    variant === "primary"
      ? "border-brand text-brand font-medium"
      : "border-line text-ink";
  return (
    <button
      type="button"
      className={`rounded-md border bg-surface px-3 py-1.5 text-sm transition-colors hover:bg-sunken focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand ${look} ${className}`}
      {...props}
    />
  );
}
