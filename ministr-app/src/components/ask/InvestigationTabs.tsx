import { cn } from "../../lib/utils";
import { BrutalClose, BrutalNew } from "../ui/brutal-icons";
import type { Investigation } from "../../lib/investigations";

interface InvestigationTabsProps {
  investigations: Investigation[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}

/**
 * Browser-tab metaphor for inquiry threads. Sits above the answer area in
 * the center pane. Each tab represents one investigation against the
 * current corpus; activating a tab restores its pinned sources and query
 * history.
 *
 * Empty state: no tabs yet. The user starts an investigation either by
 * pinning a source (lazy-create) or by asking a question (also
 * lazy-create). The "+" button forces a new blank investigation.
 */
export function InvestigationTabs({
  investigations,
  activeId,
  onSelect,
  onClose,
  onNew,
}: InvestigationTabsProps) {
  if (investigations.length === 0) {
    return (
      <div className="flex items-center justify-end shrink-0 px-2 py-1 border-b-2 border-border bg-surface">
        <button
          onClick={onNew}
          title="New investigation"
          aria-label="New investigation"
          className={cn(
            "inline-flex items-center gap-1 px-2 py-0.5 cursor-pointer transition-none rounded-sm",
            "border border-border-soft bg-surface text-text-muted",
            "hover:text-text hover:border-border",
            "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
          )}
        >
          <BrutalNew className="h-3 w-3" />
          New
        </button>
      </div>
    );
  }

  return (
    <div className="flex items-stretch gap-[2px] shrink-0 border-b-2 border-border bg-surface overflow-x-auto">
      {investigations.map((inv) => {
        const active = inv.id === activeId;
        return (
          <div
            key={inv.id}
            onClick={() => onSelect(inv.id)}
            className={cn(
              "group flex items-center gap-1.5 px-3 py-1.5 cursor-pointer transition-none shrink-0 max-w-[260px]",
              "border-r border-border-soft",
              active
                ? "bg-surface-overlay border-b-2 border-b-accent -mb-[2px]"
                : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
            )}
          >
            <span className="font-sans text-xs font-medium text-text truncate">
              {inv.title}
            </span>
            {inv.pinnedSourceIds.length > 0 && (
              <span className="font-mono text-mono-mini font-semibold tabular-nums text-text-dim shrink-0">
                {inv.pinnedSourceIds.length}
              </span>
            )}
            <button
              onClick={(e) => {
                e.stopPropagation();
                onClose(inv.id);
              }}
              title="Close investigation"
              aria-label="Close investigation"
              className={cn(
                "grid h-4 w-4 shrink-0 place-items-center cursor-pointer transition-none rounded-sm",
                "text-text-dim hover:text-danger",
                active ? "opacity-100" : "opacity-0 group-hover:opacity-100",
              )}
            >
              <BrutalClose className="h-2.5 w-2.5" />
            </button>
          </div>
        );
      })}
      <button
        onClick={onNew}
        title="New investigation"
        aria-label="New investigation"
        className={cn(
          "grid h-auto w-8 shrink-0 place-items-center cursor-pointer transition-none",
          "text-text-muted hover:text-text hover:bg-surface-overlay",
        )}
      >
        <BrutalNew className="h-3 w-3" />
      </button>
    </div>
  );
}
