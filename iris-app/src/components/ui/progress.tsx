import { cn } from "../../lib/utils";

interface ProgressProps {
  value: number; // 0-100
  className?: string;
}

export function Progress({ value, className }: ProgressProps) {
  return (
    <div className={cn("h-1.5 w-full rounded-full bg-surface-overlay", className)}>
      <div
        className="h-full rounded-full bg-accent transition-all duration-300"
        style={{ width: `${Math.min(100, Math.max(0, value))}%` }}
      />
    </div>
  );
}
