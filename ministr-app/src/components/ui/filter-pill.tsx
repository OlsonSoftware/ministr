import type React from "react";
import { chip, chipActive } from "../../lib/ui-tokens";
import { cn } from "../../lib/utils";

/**
 * Single chip primitive for filter / view-mode / kind selectors. Uses
 * the canonical `chip` / `chipActive` role tokens (rounded-full pill,
 * matches <Badge>). `tone="sans"` swaps the mono caps for sans body.
 */
interface FilterPillProps {
  label?: string;
  count?: number;
  children?: React.ReactNode;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  tone?: "mono" | "sans";
  size?: "sm" | "md";
  className?: string;
}

export function FilterPill({
  label,
  count,
  children,
  active,
  disabled = false,
  onClick,
  tone = "mono",
  size = "sm",
  className,
}: FilterPillProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        active ? chipActive : chip,
        size === "md" && "h-9",
        tone === "sans" && "font-sans text-sm font-medium normal-case tracking-normal",
        disabled && "opacity-40 cursor-not-allowed pointer-events-none",
        className,
      )}
    >
      {children ?? label}
      {typeof count === "number" && count > 0 && (
        <span className="tabular-nums opacity-70">{count}</span>
      )}
    </button>
  );
}
