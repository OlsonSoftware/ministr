import { useEffect, useRef, useState } from "react";
import { cn } from "../../lib/utils";

interface Props {
  label: string;
  value: string | number;
  /** Mark this chip as "alert" (e.g. active sessions) — accent-fill from baseline. */
  accent?: boolean;
}

/**
 * Quiet vitals chip for the TopBar. Default treatment is borderless mono
 * `LABEL · VALUE` text. When `value` changes, the chip flashes accent for
 * ~600ms then settles back to the muted state.
 *
 * Hard-step animation only (`ministr-flash`) — no fades.
 */
export function VitalsChip({ label, value, accent }: Props) {
  const [flash, setFlash] = useState(false);
  const prev = useRef(value);

  useEffect(() => {
    if (prev.current === value) return;
    prev.current = value;
    setFlash(true);
    const id = setTimeout(() => setFlash(false), 600);
    return () => clearTimeout(id);
  }, [value]);

  return (
    <span
      className={cn(
        "inline-flex items-baseline gap-1.5 px-1 py-0.5 font-mono text-xs font-semibold tracking-[0.05em] transition-none",
        accent
          ? "text-accent border-b border-accent"
          : "text-text-muted",
        flash && "ministr-flash",
      )}
    >
      <span className="uppercase text-mono-mini text-text-dim">{label}</span>
      <span className="tabular-nums">{value}</span>
    </span>
  );
}
