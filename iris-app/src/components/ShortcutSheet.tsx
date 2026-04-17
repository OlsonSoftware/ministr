import { X, Command } from "lucide-react";
import { cn } from "../lib/utils";

interface ShortcutSheetProps {
  open: boolean;
  onClose: () => void;
}

const SECTIONS: {
  title: string;
  items: { keys: string[]; label: string }[];
}[] = [
  {
    title: "Navigation",
    items: [
      { keys: ["⌘", "K"], label: "Open command palette" },
      { keys: ["g", "o"], label: "Go to Overview" },
      { keys: ["g", "s"], label: "Go to Sessions" },
      { keys: ["g", "q"], label: "Go to Search" },
      { keys: ["g", "x"], label: "Go to Explore" },
      { keys: ["g", "l"], label: "Go to Logs" },
      { keys: ["g", ","], label: "Go to Settings" },
    ],
  },
  {
    title: "Interface",
    items: [
      { keys: ["?"], label: "Show this cheatsheet" },
      { keys: ["\\"], label: "Toggle sidebar" },
      { keys: ["Esc"], label: "Close dialog / clear focus" },
    ],
  },
];

export function ShortcutSheet({ open, onClose }: ShortcutSheetProps) {
  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-bg/70 backdrop-blur-sm px-6 iris-fade-in"
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
      onClick={onClose}
    >
      <div
        className="w-full max-w-md rounded-2xl border border-border/70 bg-surface/95 shadow-[var(--shadow-lg)] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-border/60">
          <div className="flex items-center gap-2">
            <div className="grid h-7 w-7 place-items-center rounded-md bg-[var(--color-accent-soft)] text-accent">
              <Command className="h-3.5 w-3.5" />
            </div>
            <h2 className="text-sm font-semibold text-text">
              Keyboard shortcuts
            </h2>
          </div>
          <button
            onClick={onClose}
            aria-label="Close"
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-text-dim hover:bg-surface-overlay hover:text-text cursor-pointer"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="p-5 space-y-5">
          {SECTIONS.map((section) => (
            <div key={section.title}>
              <h3 className="text-[10px] font-semibold uppercase tracking-wider text-text-dim mb-2">
                {section.title}
              </h3>
              <div className="space-y-1">
                {section.items.map((item) => (
                  <div
                    key={item.label}
                    className="flex items-center justify-between gap-3 text-xs py-1"
                  >
                    <span className="text-text-muted">{item.label}</span>
                    <div className="flex items-center gap-1 shrink-0">
                      {item.keys.map((k, i) => (
                        <kbd
                          key={i}
                          className={cn(
                            "rounded-md border border-border bg-surface-overlay px-1.5 py-0.5",
                            "font-mono text-[10px] text-text-muted",
                            "min-w-[22px] text-center leading-tight",
                          )}
                        >
                          {k}
                        </kbd>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        <div className="px-5 py-3 border-t border-border/60 text-[11px] text-text-dim">
          On Windows / Linux, use{" "}
          <kbd className="rounded border border-border/70 bg-surface-overlay px-1 py-0 font-mono">
            Ctrl
          </kbd>{" "}
          where{" "}
          <kbd className="rounded border border-border/70 bg-surface-overlay px-1 py-0 font-mono">
            ⌘
          </kbd>{" "}
          is shown.
        </div>
      </div>
    </div>
  );
}
