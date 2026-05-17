import { X } from "lucide-react";
import { shortcutGroups } from "../lib/shortcuts";
import { useDialog } from "../hooks/useDialog";

interface ShortcutSheetProps {
  open: boolean;
  onClose: () => void;
}

// Build sections from the canonical shortcut map. Esc is global (not in
// the map because it's contextual — it closes whatever dialog is open),
// so we splice it into the Interface group manually.
const SECTIONS = (() => {
  const groups = shortcutGroups();
  return groups.map((g) =>
    g.title === "Interface"
      ? {
          ...g,
          items: [
            ...g.items.map((s) => ({ keys: s.keys, label: s.label })),
            { keys: ["Esc"], label: "Close dialog" },
          ],
        }
      : {
          ...g,
          items: g.items.map((s) => ({ keys: s.keys, label: s.label })),
        },
  );
})();

export function ShortcutSheet({ open, onClose }: ShortcutSheetProps) {
  const panelRef = useDialog<HTMLDivElement>(open, onClose);
  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[1000] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "10vh" }}
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
      onClick={onClose}
    >
      <div
        ref={panelRef}
        className="w-full max-w-md border border-border-soft bg-surface shadow-md overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3 border-b border-border-soft bg-surface-overlay">
          <h2 className="font-sans text-lg font-bold text-text">
            Keyboard shortcuts
          </h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="grid h-6 w-6 place-items-center border border-border-soft text-text-muted hover:text-text hover:border-border cursor-pointer transition-colors duration-150 ease-out rounded-md"
          >
            <X className="h-3 w-3" strokeWidth={2}/>
          </button>
        </div>

        <div className="p-5 space-y-6">
          {SECTIONS.map((section, sectionIdx) => (
            <div key={section.title}>
              <div className="flex items-baseline gap-3 mb-2">
                <span className="font-sans text-base font-normal text-text-dim tabular-nums shrink-0 w-6">
                  §{sectionIdx + 1}
                </span>
                <h3 className="font-sans text-base font-bold text-text">
                  {section.title}
                </h3>
              </div>
              <div className="space-y-0 pl-9">
                {section.items.map((item) => (
                  <div
                    key={item.label}
                    className="flex items-center justify-between gap-3 border-b border-border-soft last:border-b-0 py-1.5"
                  >
                    <span className="font-sans text-sm text-text-muted">{item.label}</span>
                    <div className="flex items-center gap-1 shrink-0">
                      {item.keys.map((k, i) => (
                        <kbd
                          key={i}
                          className="border border-border-soft bg-surface-overlay px-1.5 py-0 font-mono text-xs font-semibold text-text min-w-[22px] text-center leading-tight rounded-md"
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

        <div className="px-5 py-3 border-t border-border-soft bg-surface-overlay font-sans text-sm italic text-text-dim">
          On Windows / Linux, use Ctrl where ⌘ is shown.
        </div>
      </div>
    </div>
  );
}
