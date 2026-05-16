import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import { AnimatePresence, motion } from "motion/react";
import { spring } from "../../lib/motion";
import { cn } from "../../lib/utils";

export type ToastTone = "info" | "success" | "danger";

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
 * Cockpit toast tray. Bottom-left, spring-stacked, accent rim, soft
 * surface, 2.4s auto-dismiss. Tone drives the rim color.
 */
export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback<ToastContextShape["toast"]>(
    (label, opts) => {
      const id = nextId++;
      setToasts((list) => [
        ...list,
        { id, label, detail: opts?.detail, tone: opts?.tone ?? "info" },
      ]);
      setTimeout(() => dismiss(id), 2400);
    },
    [dismiss],
  );

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

function ToastItem({
  toast,
  onDismiss,
}: {
  toast: Toast;
  onDismiss: () => void;
}) {
  const rim =
    toast.tone === "success"
      ? "before:bg-success"
      : toast.tone === "danger"
        ? "before:bg-danger"
        : "before:bg-accent";
  return (
    <motion.button
      layout
      initial={{ opacity: 0, x: -24, scale: 0.96 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, x: -24, scale: 0.96 }}
      transition={spring}
      onClick={onDismiss}
      className={cn(
        "pointer-events-auto relative overflow-hidden text-left",
        "rounded-lg border border-border bg-surface shadow-md backdrop-blur-sm",
        "before:absolute before:inset-y-0 before:left-0 before:w-[3px]",
        rim,
      )}
    >
      <div className="pl-4 pr-3 py-2.5 min-w-[220px] max-w-[420px]">
        <div className="font-sans text-sm font-semibold text-text">
          {toast.label}
        </div>
        {toast.detail && (
          <div className="font-mono text-xs text-text-dim mt-0.5 truncate">
            {toast.detail}
          </div>
        )}
      </div>
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
