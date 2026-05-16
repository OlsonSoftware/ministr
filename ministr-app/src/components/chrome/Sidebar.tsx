import { MessageSquare, FolderOpen, Settings as SettingsIcon } from "lucide-react";
import { cn } from "../../lib/utils";

export type SurfaceId = "ask" | "projects" | "settings";

interface Item {
  id: SurfaceId;
  label: string;
  icon: typeof MessageSquare;
  /** Two-key chord shown on hover (g + …). */
  chord: string;
}

const ITEMS: Item[] = [
  { id: "ask", label: "Ask", icon: MessageSquare, chord: "g a" },
  { id: "projects", label: "Projects", icon: FolderOpen, chord: "g p" },
  { id: "settings", label: "Settings", icon: SettingsIcon, chord: "g ," },
];

interface Props {
  active: SurfaceId;
  onSelect: (id: SurfaceId) => void;
}

/**
 * Three-item sidebar rail. Each row is a square icon button with a
 * left accent bar when active. Labels appear on hover (tooltip via
 * native title; a future iteration may inline them when the rail
 * widens past a threshold).
 */
export function Sidebar({ active, onSelect }: Props) {
  return (
    <nav
      aria-label="Primary"
      className="flex flex-col items-stretch gap-0 border-r-2 border-border bg-surface-overlay py-2 w-12 shrink-0"
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
              // w-full (not w-12): the rail's content box is 46px after
              // the 2px right border, so a fixed w-12 (48px) button plus
              // its 3px left accent bar overflowed the rail. Filling the
              // content box keeps the active item inside the sidebar.
              "relative grid place-items-center h-12 w-full cursor-pointer transition-none",
              "border-l-[3px] box-border",
              isActive
                ? "border-l-accent bg-surface text-text"
                : "border-l-transparent text-text-muted hover:text-text hover:bg-surface",
            )}
          >
            <Icon className="h-5 w-5" strokeWidth={2} />
          </button>
        );
      })}
    </nav>
  );
}
