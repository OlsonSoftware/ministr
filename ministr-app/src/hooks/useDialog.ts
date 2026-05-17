import { useEffect, useRef } from "react";

/**
 * Shared modal-dialog behaviour for the Cockpit's overlays.
 *
 * Every overlay already paints `role="dialog" aria-modal`, but several
 * (the entity inspector, the shortcut sheet, the destructive-confirm
 * dialog) shipped without the *behaviour* that makes that markup true:
 * Escape didn't close them, focus stayed on the trigger behind the
 * scrim, and Tab walked into the obscured page. EntityPanel even
 * advertised "Close · Esc" with no key handler.
 *
 * `useDialog` centralises the three things a modal must do while open:
 *
 *  1. **Escape closes** — handled on `document` in the capture phase and
 *     `stopImmediatePropagation`'d so it fires exactly once and never
 *     double-runs with the app-level shortcut handler.
 *  2. **Focus moves in and is restored** — focus enters the dialog
 *     (an explicit `initialFocus` element, else the container) on open,
 *     and returns to whatever was focused before, on close/unmount.
 *  3. **Tab is trapped** — Tab / Shift+Tab cycle within the dialog
 *     instead of escaping to the inert background.
 *
 * Attach the returned ref to the dialog container. Safe to layer on an
 * overlay that already manages its own inner focus (e.g. the command
 * palette autofocusing its input): pass that input as `initialFocus`
 * and the trap/restore are purely additive.
 */
export function useDialog<T extends HTMLElement = HTMLDivElement>(
  open: boolean,
  onClose: () => void,
  opts: { initialFocus?: React.RefObject<HTMLElement | null> } = {},
) {
  const containerRef = useRef<T>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const initialFocus = opts.initialFocus;

  useEffect(() => {
    if (!open) return;
    const container = containerRef.current;
    const restoreTo =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    function focusables(): HTMLElement[] {
      if (!container) return [];
      return Array.from(
        container.querySelectorAll<HTMLElement>(
          'a[href],button:not([disabled]),textarea:not([disabled]),' +
            'input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])',
        ),
      ).filter((el) => el.offsetParent !== null || el === document.activeElement);
    }

    // Move focus in. rAF so it lands after the open animation mounts.
    const raf = requestAnimationFrame(() => {
      const target =
        initialFocus?.current ?? focusables()[0] ?? container ?? null;
      target?.focus();
    });

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab" || !container) return;
      const els = focusables();
      if (els.length === 0) {
        e.preventDefault();
        return;
      }
      const first = els[0];
      const last = els[els.length - 1];
      const active = document.activeElement as HTMLElement | null;
      // Wrap, and pull focus back in if it has somehow escaped.
      if (e.shiftKey && (active === first || !container.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || !container.contains(active))) {
        e.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("keydown", onKeyDown, true);
      // Restore focus only if it's still inside the (closing) dialog,
      // so we don't yank focus the user has since moved elsewhere.
      const a = document.activeElement;
      if (!a || a === document.body || container?.contains(a)) {
        restoreTo?.focus?.();
      }
    };
  }, [open, initialFocus]);

  return containerRef;
}
