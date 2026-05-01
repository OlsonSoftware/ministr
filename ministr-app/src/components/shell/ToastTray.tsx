import { createContext, useCallback, useContext, useEffect, useState } from "react";
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
 * Brutalist toast tray.
 *
 * Bottom-left, fixed, 2px-bordered, accent left-bar, 2-second auto-dismiss.
 * Tone determines the left bar color: info=accent, success=success, danger=danger.
 * Content is mono uppercase. No fades — toasts disappear in zero ms.
 */
export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback<ToastContextShape["toast"]>((label, opts) => {
    const id = nextId++;
    const t: Toast = {
      id,
      label,
      detail: opts?.detail,
      tone: opts?.tone ?? "info",
    };
    setToasts((list) => [...list, t]);
    setTimeout(() => dismiss(id), 2000);
  }, [dismiss]);

  return (
    <ToastCtx.Provider value={{ toast }}>
      {children}
      <div
        className="fixed bottom-3 left-3 z-[1100] flex flex-col gap-2 pointer-events-none"
        aria-live="polite"
      >
        {toasts.map((t) => (
          <ToastItem key={t.id} toast={t} onDismiss={() => dismiss(t.id)} />
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: () => void }) {
  const barClass =
    toast.tone === "success"
      ? "bg-success"
      : toast.tone === "danger"
        ? "bg-danger"
        : "bg-accent";
  return (
    <button
      onClick={onDismiss}
      className={cn(
        "pointer-events-auto flex items-stretch border border-border-soft bg-surface shadow-[var(--shadow-sm)] cursor-pointer transition-none text-left",
      )}
    >
      <div className={cn("w-[3px] shrink-0", barClass)} />
      <div className="px-3 py-2 min-w-[220px] max-w-[420px]">
        <div className="font-sans text-sm font-semibold text-text">
          {toast.label}
        </div>
        {toast.detail && (
          <div className="font-mono text-xs text-text-dim mt-0.5 truncate">
            {toast.detail}
          </div>
        )}
      </div>
    </button>
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
