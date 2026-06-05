import { MessageSquare, FolderOpen, Activity, Cloud, Terminal, Settings as SettingsIcon } from "@/components/ui/icons";
import { motion } from "motion/react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";

export type SurfaceId = "ask" | "projects" | "sessions" | "cloud" | "explore" | "settings";

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
  { id: "explore", label: "Explore", icon: Terminal, chord: "g e" },
  { id: "settings", label: "Settings", icon: SettingsIcon, chord: "g ," },
];

interface Props {
  active: SurfaceId;
  onSelect: (id: SurfaceId) => void;
}

export function Sidebar({ active, onSelect }: Props) {
  return (
    <nav
      aria-label="Main navigation"
      className="flex flex-col items-center gap-0.5 border-r border-border bg-surface py-3 w-[60px] shrink-0"
    >
      {ITEMS.map((item) => {
        const Icon = item.icon;
        const isActive = item.id === active;
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onSelect(item.id)}
            title={`${item.chord}`}
            aria-label={item.label}
            aria-current={isActive ? "page" : undefined}
            className={cn(
              "group relative flex flex-col items-center justify-center gap-0.5 h-12 w-12 rounded-lg cursor-pointer",
              "transition-colors duration-150 ease-out",
              "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent focus-visible:z-20",
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
              className="relative z-10 flex flex-col items-center gap-0.5"
            >
              <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
              <span className="text-[9px] font-medium leading-none select-none tracking-tight">
                {item.label}
              </span>
            </motion.span>
          </button>
        );
      })}
    </nav>
  );
}
