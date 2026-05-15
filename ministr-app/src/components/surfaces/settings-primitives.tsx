/**
 * Shared row primitives for the Settings sub-panels.
 *
 * Extracted from the old monolithic Settings.tsx when it was split into
 * General / Server / About. Kept in one file because each primitive is
 * tiny and they're only ever used together by the settings panels.
 */
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "../../lib/utils";

export function PrefRow({
  label,
  description,
  icon: Icon,
  children,
}: {
  label: string;
  description?: string;
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  children: React.ReactNode;
}) {
  const sentence = /^[A-Z][A-Z\s\-—·]+$/.test(label)
    ? label.charAt(0) + label.slice(1).toLowerCase()
    : label;
  return (
    <div className="flex items-center justify-between gap-4 border-b border-border-soft last:border-b-0 px-3 py-3">
      <div className="min-w-0 flex-1 flex items-start gap-2">
        {Icon && (
          <Icon
            className="h-3.5 w-3.5 text-text-dim mt-0.5 shrink-0"
            strokeWidth={2}
          />
        )}
        <div className="min-w-0">
          <p className="font-sans text-sm font-semibold text-text">
            {sentence}
          </p>
          {description && (
            <p className="font-sans text-xs text-text-dim mt-0.5">
              {description}
            </p>
          )}
        </div>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

export function MetaRow({
  label,
  value,
  truncate,
}: {
  label: string;
  value: string;
  truncate?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border-soft last:border-b-0 px-3 py-1.5">
      <span className="font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim shrink-0">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-xs tabular-nums text-text text-right",
          truncate && "truncate",
        )}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

export function MaintAction({
  icon: Icon,
  label,
  danger,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "border border-border-soft px-3 py-3 flex flex-col items-center gap-2 cursor-pointer transition-none -ml-[1px] -mt-[1px] first:ml-0 first:mt-0",
        "bg-surface text-text-muted",
        danger
          ? "hover:bg-danger hover:text-white hover:border-danger"
          : "hover:bg-surface-overlay hover:text-text hover:border-border",
      )}
    >
      <Icon className="h-4 w-4" strokeWidth={2} />
      <span className="font-sans text-xs font-medium text-center">{label}</span>
    </button>
  );
}

export function DiagnosticSection({
  icon: Icon,
  label,
  hint,
  expanded,
  onToggle,
  isLast,
  children,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  hint: string;
  expanded: boolean;
  onToggle: () => void;
  isLast: boolean;
  children: React.ReactNode;
}) {
  return (
    <>
      <button
        onClick={onToggle}
        className={cn(
          "flex w-full items-center gap-2 px-3 py-2 cursor-pointer hover:bg-surface-overlay transition-none text-left",
          !isLast || expanded ? "border-b border-border-soft" : "",
        )}
      >
        {expanded ? (
          <ChevronDown
            className="h-3.5 w-3.5 text-text-dim shrink-0"
            strokeWidth={2.5}
          />
        ) : (
          <ChevronRight
            className="h-3.5 w-3.5 text-text-dim shrink-0"
            strokeWidth={2.5}
          />
        )}
        <Icon
          className="h-3.5 w-3.5 text-text-dim shrink-0"
          strokeWidth={2}
        />
        <span className="font-sans text-sm font-semibold text-text">
          {label}
        </span>
        <span className="font-sans text-xs text-text-dim truncate">
          · {hint}
        </span>
      </button>
      {expanded && (
        <div
          className={cn("px-3 py-3", !isLast && "border-b border-border-soft")}
        >
          {children}
        </div>
      )}
    </>
  );
}

export function formatUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}
