import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { AnimatePresence, motion, useAnimationControls } from "motion/react";
import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";
import { focusRing, glassPanel } from "../../lib/ui-tokens";

export type ToastTone = "info" | "success" | "danger";

/** Per-severity command-deck vocabulary. Tone lives on NON-text only (the
 *  medallion glyph + its rim, the left spine, the countdown fill) so the
 *  label/detail stay high-contrast text-text/text-text-dim (AA-safe). The
 *  medallion is QUIET (no glow) — a toast is transient, not a "live" object. */
const TONE: Record<
  ToastTone,
  { icon: typeof Info; spine: string; rim: string; glyph: string; fill: string }
> = {
  info: {
    icon: Info,
    spine: "border-l-accent",
    rim: "border-accent/40",
    glyph: "text-accent",
    fill: "bg-accent",
  },
  success: {
    icon: CheckCircle2,
    spine: "border-l-success",
    rim: "border-success/40",
    glyph: "text-success",
    fill: "bg-success",
  },
  danger: {
    icon: AlertTriangle,
    spine: "border-l-danger",
    rim: "border-danger/40",
    glyph: "text-danger",
    fill: "bg-danger",
  },
};

/** Auto-dismiss budget (ms). A fault deserves longer to be read than a
 *  routine confirmation. */
const DURATION: Record<ToastTone, number> = {
  info: 4000,
  success: 4000,
  danger: 6500,
};

export interface Toast {
  id: number;
  label: string;
  detail?: string;
  tone: ToastTone;
}

interface ToastContextShape {
  toast: (label: string, opts?: { detail?: string; tone?: ToastTone }) => void;
}

const ToastCtx = createContext<ToastContextShape | null>(null);

let nextId = 1;

/**
 * Cockpit toast tray. Bottom-left, spring-stacked notifications on the
 * Liquid-Glass tier (DESIGN.md §4). Each toast carries a per-severity
 * command-deck identity (medallion + tone spine + countdown) and owns its
 * own auto-dismiss lifecycle (see {@link ToastItem}).
 */
export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback<ToastContextShape["toast"]>((label, opts) => {
    const id = nextId++;
    setToasts((list) => [
      ...list,
      { id, label, detail: opts?.detail, tone: opts?.tone ?? "info" },
    ]);
  }, []);

  return (
    <ToastCtx.Provider value={{ toast }}>
      {children}
      <div
        className="fixed bottom-4 left-4 z-[1100] flex flex-col gap-2 pointer-events-none"
        aria-live="polite"
      >
        <AnimatePresence initial={false}>
          {toasts.map((t) => (
            <ToastItem key={t.id} toast={t} onDismiss={() => dismiss(t.id)} />
          ))}
        </AnimatePresence>
      </div>
    </ToastCtx.Provider>
  );
}

/** One notification on the glass tier. Exported for the Storybook catalog so a
 *  story can render the severities statically (with a no-op dismiss). */
export function ToastItem({
  toast,
  onDismiss,
}: {
  toast: Toast;
  onDismiss: () => void;
}) {
  const { icon: Icon, spine, rim, glyph, fill } = TONE[toast.tone];
  const duration = DURATION[toast.tone];

  // The auto-dismiss timer is deliberately a plain setTimeout, NOT coupled to
  // the countdown animation — under prefers-reduced-motion the MotionConfig at
  // the app root snaps transforms to their end state, which would otherwise
  // fire dismissal instantly. The countdown bar is decorative; the clock here
  // is the source of truth. Hover/focus pauses it (with the remaining budget).
  const controls = useAnimationControls();
  const timerRef = useRef<number | null>(null);
  const remainingRef = useRef(duration);
  const startedRef = useRef(0);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const schedule = useCallback(
    (ms: number) => {
      clearTimer();
      startedRef.current = performance.now();
      remainingRef.current = ms;
      timerRef.current = window.setTimeout(onDismiss, ms);
      controls.start({ scaleX: 0 }, { duration: ms / 1000, ease: "linear" });
    },
    [clearTimer, controls, onDismiss],
  );

  useEffect(() => {
    schedule(duration);
    return clearTimer;
    // schedule/clearTimer are stable; run once per toast.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pause = useCallback(() => {
    clearTimer();
    remainingRef.current -= performance.now() - startedRef.current;
    controls.stop();
  }, [clearTimer, controls]);

  const resume = useCallback(() => {
    if (remainingRef.current > 0) schedule(remainingRef.current);
  }, [schedule]);

  return (
    <motion.button
      layout
      initial={{ opacity: 0, x: -24, scale: 0.96 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, x: -24, scale: 0.96 }}
      transition={spring}
      onClick={onDismiss}
      onMouseEnter={pause}
      onMouseLeave={resume}
      onFocus={pause}
      onBlur={resume}
      aria-label={`Dismiss notification: ${toast.label}`}
      className={cn(
        glassPanel,
        focusRing,
        // glassPanel owns the radius (radius-xl); overflow-hidden clips the
        // countdown bar to that corner. Safe: the specular is an INSET shadow
        // (not a pseudo) and the focus ring is an outline — neither is clipped.
        "group pointer-events-auto relative block overflow-hidden border-l-2 text-left",
        spine,
      )}
    >
      <div className="flex items-start gap-3 py-3 pl-3 pr-3 min-w-[260px] max-w-[420px]">
        <span
          aria-hidden
          className={cn(
            "mt-px grid h-7 w-7 shrink-0 place-items-center rounded-lg border bg-surface-overlay",
            rim,
            glyph,
          )}
        >
          <Icon className="h-4 w-4" strokeWidth={2} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-sans text-sm font-semibold text-text leading-snug">
            {toast.label}
          </div>
          {toast.detail && (
            <div className="font-mono text-xs text-text-dim mt-0.5 truncate">
              {toast.detail}
            </div>
          )}
        </div>
        <X
          aria-hidden
          strokeWidth={2.5}
          className="mt-px h-3.5 w-3.5 shrink-0 text-text-dim transition-colors duration-150 group-hover:text-text"
        />
      </div>
      {/* Countdown to auto-dismiss — decorative; the setTimeout above is the
          real clock. Snaps (not animates) under reduced motion. */}
      <span
        aria-hidden
        className="pointer-events-none absolute inset-x-0 bottom-0 h-[2px] bg-border/50"
      >
        <motion.span
          className={cn("block h-full origin-left", fill)}
          style={{ originX: 0 }}
          initial={{ scaleX: 1 }}
          animate={controls}
        />
      </span>
    </motion.button>
  );
}

/** Hook accessor; returns no-op when used outside the provider. */
export function useToast(): ToastContextShape {
  const ctx = useContext(ToastCtx);
  return ctx ?? { toast: () => {} };
}

/** Optional helper for components that need a toast on mount only. */
export function useToastOnce(label: string, deps: unknown[] = []) {
  const { toast } = useToast();
  useEffect(() => {
    toast(label);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
