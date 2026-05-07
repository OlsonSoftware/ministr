import { useEffect, useRef, type ReactNode } from "react";
import { cn } from "../../lib/utils";
import { BrutalClose } from "../ui/brutal-icons";

interface DrawerProps {
  /** Whether the drawer is mounted/visible. */
  open: boolean;
  /** Called when the user dismisses (Esc / backdrop click / close button). */
  onClose: () => void;
  /** Drawer header label, mono uppercase per design system. */
  title: string;
  /** Optional right-side actions in the header (buttons, badges). */
  headerRight?: ReactNode;
  /** Drawer body content. Drawer manages its own scroll. */
  children: ReactNode;
  /**
   * Drawer height as a viewport fraction. Defaults to 0.55 (55vh) — large
   * enough for a log tail or session vitals to read comfortably without
   * obscuring the workspace context above.
   */
  heightVh?: number;
}

/**
 * Slide-up drawer mounted at the bottom of the viewport. Used by the status
 * bar to surface Logs / Session vitals / Indexing detail without taking the
 * user out of the workspace canvas.
 *
 * Refined-brutalist treatment: thick strong border on top, no rounded
 * corners, motion-data slide on enter only. Backdrop is a subtle scrim, not
 * an opaque overlay — the workspace stays visually present.
 */
export function Drawer({
  open,
  onClose,
  title,
  headerRight,
  children,
  heightVh = 55,
}: DrawerProps) {
  const closeBtnRef = useRef<HTMLButtonElement>(null);

  // Esc to dismiss. Use capture so the drawer eats the key before any
  // workspace handler beneath it interprets it.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        onClose();
      }
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, { capture: true });
  }, [open, onClose]);

  // Move focus into the drawer on open so screen readers + keyboard users
  // land somewhere useful.
  useEffect(() => {
    if (open) closeBtnRef.current?.focus();
  }, [open]);

  if (!open) return null;

  return (
    <>
      {/* Backdrop — subtle scrim. Workspace stays visible above the drawer. */}
      <div
        className="fixed inset-0 z-[1100] bg-black/30"
        onClick={onClose}
        aria-hidden="true"
      />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={cn(
          "fixed left-0 right-0 bottom-0 z-[1101] bg-surface flex flex-col",
          "border-strong border-t border-border shadow-lg",
          "ministr-drawer-in",
        )}
        style={{ height: `${heightVh}vh` }}
      >
        <header className="flex items-center justify-between gap-3 border-b-2 border-border bg-surface-overlay px-4 py-2 shrink-0">
          <h2 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
            {title}
          </h2>
          <div className="flex items-center gap-2">
            {headerRight}
            <button
              ref={closeBtnRef}
              onClick={onClose}
              aria-label="Close drawer"
              title="Close · Esc"
              className={cn(
                "grid h-7 w-7 shrink-0 place-items-center cursor-pointer",
                "border border-border bg-surface text-text-muted",
                "hover:text-text hover:border-border-hover transition-none rounded-sm",
              )}
            >
              <BrutalClose className="h-3.5 w-3.5" />
            </button>
          </div>
        </header>
        <div className="flex-1 min-h-0 overflow-y-auto">{children}</div>
      </aside>
    </>
  );
}
