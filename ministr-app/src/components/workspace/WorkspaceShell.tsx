import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { cn } from "../../lib/utils";

interface WorkspaceShellProps {
  /** Left pane — corpus rail. Persistent. */
  rail: ReactNode;
  /** Center pane — conversation/search. Persistent. */
  center: ReactNode;
  /** Right pane — pinned source stack. Persistent. */
  source: ReactNode;
  /** Bottom status bar. Persistent. */
  statusBar: ReactNode;
  /** Optional banner above the panes (e.g. error strip). */
  banner?: ReactNode;
  /**
   * Initial pane widths in CSS-pixel terms. The shell normalizes to flex
   * basis values that respect window resize via min/max ratios.
   */
  defaultRailPx?: number;
  defaultSourcePx?: number;
  /** Minimum pane widths the user can drag to. */
  minRailPx?: number;
  minSourcePx?: number;
  minCenterPx?: number;
}

const STORAGE_KEY = "ministr:workspace:panes:v1";

interface PersistedPanes {
  railPx: number;
  sourcePx: number;
}

/**
 * Three-pane workspace layout with hand-rolled brutalist dividers.
 *
 * No drag handle dot — the divider IS the affordance, with a thick
 * border-strong stroke that turns accent on hover/drag and shows the
 * col-resize cursor. Pane widths persist across sessions via localStorage.
 *
 * Mounts a banner slot above the panes (for error strips) and a status bar
 * slot below.
 */
export function WorkspaceShell({
  rail,
  center,
  source,
  statusBar,
  banner,
  defaultRailPx = 240,
  defaultSourcePx = 360,
  minRailPx = 180,
  minSourcePx = 280,
  minCenterPx = 360,
}: WorkspaceShellProps) {
  const [railPx, setRailPx] = useState(() => loadPanes()?.railPx ?? defaultRailPx);
  const [sourcePx, setSourcePx] = useState(
    () => loadPanes()?.sourcePx ?? defaultSourcePx,
  );

  // Persist on settle, not on every drag tick.
  useEffect(() => {
    const id = window.setTimeout(() => {
      try {
        window.localStorage.setItem(
          STORAGE_KEY,
          JSON.stringify({ railPx, sourcePx } satisfies PersistedPanes),
        );
      } catch {
        /* quota — ignore */
      }
    }, 250);
    return () => window.clearTimeout(id);
  }, [railPx, sourcePx]);

  const containerRef = useRef<HTMLDivElement>(null);

  const railResize = usePaneResize({
    containerRef,
    onUpdate: (deltaPx) =>
      setRailPx((prev) =>
        clamp(prev + deltaPx, minRailPx, maxRail(containerRef.current, sourcePx, minCenterPx)),
      ),
  });

  const sourceResize = usePaneResize({
    containerRef,
    direction: "right",
    onUpdate: (deltaPx) =>
      setSourcePx((prev) =>
        clamp(prev - deltaPx, minSourcePx, maxSource(containerRef.current, railPx, minCenterPx)),
      ),
  });

  return (
    <div className="flex h-screen flex-col bg-bg text-text">
      {banner}

      <div ref={containerRef} className="flex flex-1 min-h-0 overflow-hidden">
        <section
          className="shrink-0 min-w-0"
          style={{ width: `${railPx}px` }}
          aria-label="Corpus rail pane"
        >
          {rail}
        </section>

        <Resizer {...railResize} />

        <section className="flex-1 min-w-0 min-h-0" aria-label="Center pane">
          {center}
        </section>

        <Resizer {...sourceResize} />

        <section
          className="shrink-0 min-w-0"
          style={{ width: `${sourcePx}px` }}
          aria-label="Source pane"
        >
          {source}
        </section>
      </div>

      {statusBar}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Brutal pane resizer

interface ResizerHandlers {
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  active: boolean;
}

function Resizer({ onPointerDown, active }: ResizerHandlers) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      tabIndex={-1}
      onPointerDown={onPointerDown}
      data-active={active || undefined}
      className="workspace-resizer"
    />
  );
}

function usePaneResize(opts: {
  containerRef: React.RefObject<HTMLDivElement | null>;
  direction?: "left" | "right";
  onUpdate: (deltaPx: number) => void;
}): ResizerHandlers {
  const [active, setActive] = useState(false);
  const startX = useRef(0);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      const target = e.currentTarget;
      target.setPointerCapture(e.pointerId);
      startX.current = e.clientX;
      setActive(true);

      let lastX = e.clientX;

      function onMove(ev: PointerEvent) {
        const delta = ev.clientX - lastX;
        lastX = ev.clientX;
        opts.onUpdate(delta);
      }

      function onUp(ev: PointerEvent) {
        target.releasePointerCapture(ev.pointerId);
        setActive(false);
        target.removeEventListener("pointermove", onMove);
        target.removeEventListener("pointerup", onUp);
        target.removeEventListener("pointercancel", onUp);
      }

      target.addEventListener("pointermove", onMove);
      target.addEventListener("pointerup", onUp);
      target.addEventListener("pointercancel", onUp);
    },
    [opts],
  );

  return { onPointerDown, active };
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n));
}

function maxRail(container: HTMLDivElement | null, sourcePx: number, minCenterPx: number): number {
  if (!container) return 480;
  return Math.max(180, container.clientWidth - sourcePx - minCenterPx - 6);
}

function maxSource(container: HTMLDivElement | null, railPx: number, minCenterPx: number): number {
  if (!container) return 600;
  return Math.max(280, container.clientWidth - railPx - minCenterPx - 6);
}

function loadPanes(): PersistedPanes | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedPanes;
    if (typeof parsed.railPx === "number" && typeof parsed.sourcePx === "number") {
      return parsed;
    }
  } catch {
    /* ignore */
  }
  return null;
}
