import { MessageSquare, FolderOpen, Activity, Cloud, Settings as SettingsIcon } from "lucide-react";
import { motion } from "motion/react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";

export type SurfaceId = "ask" | "projects" | "sessions" | "cloud" | "settings";

interface Item {
  id: SurfaceId;
  label: string;
  icon: typeof MessageSquare;
  chord: string;
}

const ITEMS: Item[] = [
  { id: "ask", label: "Ask", icon: MessageSquare, chord: "g a" },
  { id: "projects", label: "Projects", icon: FolderOpen, chord: "g p" },
  { id: "sessions", label: "Sessions", icon: Activity, chord: "g s" },
  { id: "cloud", label: "Cloud", icon: Cloud, chord: "g c" },
  { id: "settings", label: "Settings", icon: SettingsIcon, chord: "g ," },
];

interface Props {
  active: SurfaceId;
  onSelect: (id: SurfaceId) => void;
}

/**
 * Cockpit nav rail. Icon-only column; the active indicator is a single
 * shared-layout pill that springs between items. Label + chord show as
 * a tooltip on hover.
 */
export function Sidebar({ active, onSelect }: Props) {
  return (
    <nav
      aria-label="Primary"
      className="flex flex-col items-center gap-1 border-r border-border bg-surface py-3 w-14 shrink-0"
    >
      {ITEMS.map((item) => {
        const Icon = item.icon;
        const isActive = item.id === active;
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onSelect(item.id)}
            title={`${item.label} · ${item.chord}`}
            aria-label={item.label}
            aria-current={isActive ? "page" : undefined}
            className={cn(
              "group relative grid place-items-center h-10 w-10 rounded-lg cursor-pointer",
              "transition-colors duration-150 ease-out",
              isActive
                ? "text-[var(--color-accent-fg-on)]"
                : "text-text-dim hover:text-text hover:bg-surface-overlay",
            )}
          >
            {isActive && (
              <motion.span
                layoutId="nav-active"
                transition={spring}
                className="absolute inset-0 rounded-lg bg-accent shadow-[var(--glow-soft)]"
              />
            )}
            <motion.span
              whileTap={{ scale: 0.86 }}
              className="relative z-10 grid place-items-center"
            >
              <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
            </motion.span>
          </button>
        );
      })}
    </nav>
  );
}
