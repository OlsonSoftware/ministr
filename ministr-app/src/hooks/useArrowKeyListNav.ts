/**
 * useArrowKeyListNav — arrow-key navigation across a list of focusable rows.
 *
 * The Explore lenses render long, grouped lists of focusable rows (findings,
 * diagnostics, seams, …). They're click + Tab + Enter/Space navigable, but
 * walking a long list with Tab is tedious. This hook ADDS ArrowUp/Down/Home/End
 * to move focus among the rows — the W3C APG "developing a keyboard interface"
 * convention (after Tab reaches the group, arrows move within it).
 *
 * It is deliberately ADDITIVE and DOM-only: rows keep their static `tabIndex`
 * and Enter/Space handlers; the hook never mutates tabIndex or React state, so
 * it sidesteps the well-known roving-tabindex/React re-render reset bug
 * (MUI #16644: "do it purely in the DOM, not via state").
 *
 * Returns a CALLBACK REF — attach it to the scroll container. A callback ref
 * (not a RefObject + useEffect) is required because the container is often
 * conditionally rendered (it appears only once data has loaded); the callback
 * fires on the real mount/unmount, so the listener attaches reliably. Tag each
 * row with `data-roving-item`.
 *
 *   const listRef = useArrowKeyListNav<HTMLDivElement>();
 *   <div ref={listRef} className="overflow-y-auto">…<Row data-roving-item />…</div>
 */
import { useCallback, useRef } from "react";

const NAV_KEYS = new Set(["ArrowDown", "ArrowUp", "Home", "End"]);

export function useArrowKeyListNav<T extends HTMLElement = HTMLElement>(
  itemSelector = "[data-roving-item]",
): (node: T | null) => void {
  const cleanupRef = useRef<(() => void) | null>(null);

  return useCallback(
    (node: T | null) => {
      // Detach from any previous element first.
      cleanupRef.current?.();
      cleanupRef.current = null;
      if (!node) return;

      const onKeyDown = (e: KeyboardEvent) => {
        if (!NAV_KEYS.has(e.key) || e.metaKey || e.ctrlKey || e.altKey) return;
        const items = Array.from(node.querySelectorAll<HTMLElement>(itemSelector));
        if (items.length === 0) return;

        const active = document.activeElement as HTMLElement | null;
        const currentIdx = items.findIndex(
          (it) => it === active || it.contains(active),
        );
        // Only hijack arrows when focus is already on one of the list rows — so
        // typing in a lens's own input (e.g. the Changes range box) is untouched.
        if (currentIdx === -1) return;

        let nextIdx: number;
        switch (e.key) {
          case "Home":
            nextIdx = 0;
            break;
          case "End":
            nextIdx = items.length - 1;
            break;
          case "ArrowDown":
            nextIdx = Math.min(items.length - 1, currentIdx + 1);
            break;
          default: // ArrowUp
            nextIdx = Math.max(0, currentIdx - 1);
        }

        const next = items[nextIdx];
        if (next && next !== active) {
          e.preventDefault();
          next.focus();
          next.scrollIntoView?.({ block: "nearest" });
        }
      };

      node.addEventListener("keydown", onKeyDown);
      cleanupRef.current = () => node.removeEventListener("keydown", onKeyDown);
    },
    [itemSelector],
  );
}
