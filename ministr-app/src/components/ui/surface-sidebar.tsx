import { type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import { cn } from "../../lib/utils";
import { swift } from "../../lib/motion";
import { AdaptiveSurface } from "./adaptive-surface";

export interface SidebarItem {
  id: string;
  label: string;
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
}

interface Props {
  title: string;
  items: readonly SidebarItem[];
  active: string;
  onSelect: (id: string) => void;
  children: ReactNode;
  /** Full-bleed content: the active section is an edge-to-edge layout that
   *  manages its own height + scrolling (e.g. the Code IDE surface). Drops the
   *  default padding/scroll so the child's `h-full` chain resolves to the full
   *  surface height. Padded, self-scrolling sections (Server, Logs) omit it. */
  fill?: boolean;
}

export function SurfaceSidebar({
  title,
  items,
  active,
  onSelect,
  children,
  fill = false,
}: Props) {
  return (
    <AdaptiveSurface bleed={fill}>
      <div className="h-full flex flex-col @min-[900px]/surface:flex-row min-h-0">
        {/* Sidebar nav — wide viewports */}
        <nav
          aria-label="Section navigation"
          className="hidden @min-[900px]/surface:flex flex-col w-[200px] shrink-0 border-r border-border-soft p-4 pt-5"
        >
          <span className="font-sans text-xs font-semibold text-text-dim px-3 mb-3 uppercase tracking-wide">
            {title}
          </span>
          <div className="flex flex-col gap-0.5">
            {items.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                type="button"
                onClick={() => onSelect(id)}
                aria-current={active === id ? "page" : undefined}
                className={cn(
                  "flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium text-left transition-colors duration-150",
                  active === id
                    ? "bg-accent/10 text-accent"
                    : "text-text-muted hover:text-text hover:bg-surface-overlay",
                )}
              >
                <Icon className="h-4 w-4 shrink-0" strokeWidth={1.8} />
                {label}
              </button>
            ))}
          </div>
        </nav>

        {/* Tab bar — narrow viewports */}
        <nav
          aria-label="Section navigation"
          className="flex @min-[900px]/surface:hidden border-b border-border-soft shrink-0"
        >
          {items.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => onSelect(id)}
              aria-current={active === id ? "page" : undefined}
              className={cn(
                "flex-1 flex items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-medium transition-colors duration-150",
                active === id
                  ? "text-text border-b-2 border-accent -mb-[1px]"
                  : "text-text-muted hover:text-text",
              )}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" strokeWidth={1.8} />
              {label}
            </button>
          ))}
        </nav>

        {/* Active view with animated transition. `fill` sections own their
            height + scrolling (edge-to-edge IDE layout); others get the
            default padding + vertical scroll. */}
        <div
          className={cn(
            "flex-1 min-h-0",
            fill ? "overflow-hidden" : "overflow-y-auto p-5",
          )}
        >
          <AnimatePresence mode="wait">
            <motion.div
              key={active}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={swift}
              className={fill ? "h-full min-h-0" : undefined}
            >
              {children}
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </AdaptiveSurface>
  );
}
