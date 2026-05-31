/**
 * Track an element's content-box width via `ResizeObserver`.
 *
 * Drives the Code surface's responsive right panel: measuring the *actual*
 * surface width (rather than the window) is what lets the panel decide between
 * an inline third column and a slide-over drawer at the real breakpoint, with
 * the 60px nav rail already subtracted.
 */
import { useEffect, useState, type RefObject } from "react";

export function useContainerWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) setWidth(entry.contentRect.width);
    });
    observer.observe(el);
    setWidth(el.getBoundingClientRect().width);
    return () => observer.disconnect();
  }, [ref]);

  return width;
}
