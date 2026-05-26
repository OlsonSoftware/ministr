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
}

export function SurfaceSidebar({
  title,
  items,
  active,
  onSelect,
  children,
}: Props) {
  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col @min-[900px]/surface:flex-row min-h-0">
        {/* Sidebar nav — wide viewports */}
        <nav className="hidden @min-[900px]/surface:flex flex-col w-[200px] shrink-0 border-r border-border-soft p-4 pt-5">
          <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim px-3 mb-3">
            {title}
          </span>
          <div className="flex flex-col gap-1">
            {items.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                type="button"
                onClick={() => onSelect(id)}
                className={cn(
                  "flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium text-left transition-colors duration-150 border-l-2",
                  active === id
                    ? "border-accent bg-surface-overlay text-text"
                    : "border-transparent text-text-muted hover:text-text hover:bg-surface-overlay/50",
                )}
              >
                <Icon className="h-4 w-4 shrink-0" strokeWidth={1.8} />
                {label}
              </button>
            ))}
          </div>
        </nav>

        {/* Tab bar — narrow viewports */}
        <div className="flex @min-[900px]/surface:hidden border-b border-border-soft shrink-0">
          {items.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => onSelect(id)}
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
        </div>

        {/* Active view with animated transition */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          <AnimatePresence mode="wait">
            <motion.div
              key={active}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={swift}
            >
              {children}
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </AdaptiveSurface>
  );
}
