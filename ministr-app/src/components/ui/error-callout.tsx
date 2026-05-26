import type { ReactNode } from "react";
import { AlertTriangle } from "lucide-react";
import { cn } from "../../lib/utils";

interface ErrorCalloutProps {
  message: string;
  title?: string;
  action?: ReactNode;
  className?: string;
}

export function ErrorCallout({
  message,
  title,
  action,
  className,
}: ErrorCalloutProps) {
  return (
    <div
      role="alert"
      className={cn(
        "rounded-lg border border-danger/40 bg-danger/5 p-3 flex items-start gap-2",
        className,
      )}
    >
      <AlertTriangle
        className="h-4 w-4 text-danger shrink-0 mt-0.5"
        strokeWidth={2}
      />
      <div className="flex-1 min-w-0">
        {title && (
          <p className="font-sans text-sm font-medium text-text">{title}</p>
        )}
        <p className="font-mono text-xs text-text-muted break-words">
          {message}
        </p>
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
}
