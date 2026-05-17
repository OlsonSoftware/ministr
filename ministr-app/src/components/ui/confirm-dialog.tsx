/**
 * ConfirmDialog — the single confirmation primitive for destructive
 * actions. Type-to-confirm is opt-in via `confirmToken`. Cockpit modal:
 * rounded, hairline, spring pop via the shared motion presets.
 */
import { useRef, useState } from "react";
import { AlertTriangle, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { Button } from "./button";
import { popIn, scrim } from "../../lib/motion";
import { transitionInteractive } from "../../lib/ui-tokens";
import { useDialog } from "../../hooks/useDialog";
import { cn } from "../../lib/utils";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  body: React.ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "danger" | "default";
  confirmToken?: string;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  tone = "default",
  confirmToken,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  const [typed, setTyped] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  // Escape cancels, focus enters the dialog (the token field when
  // type-to-confirm is required, else the safe Cancel button) and is
  // restored to the trigger on close, Tab stays inside.
  const dialogRef = useDialog<HTMLDivElement>(open, onCancel, {
    initialFocus: confirmToken ? inputRef : cancelRef,
  });

  const danger = tone === "danger";
  const requiresToken = !!confirmToken;
  const tokenMatches = requiresToken
    ? typed.trim().toUpperCase() === confirmToken.toUpperCase()
    : true;

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          variants={scrim}
          initial="initial"
          animate="animate"
          exit="exit"
          className="fixed inset-0 z-[1100] flex items-start justify-center bg-black/50 backdrop-blur-[2px] px-6"
          style={{ paddingTop: "20vh" }}
          role="dialog"
          aria-modal="true"
          aria-label={title}
          onClick={onCancel}
        >
          <motion.div
            ref={dialogRef}
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className={cn(
              "w-full max-w-md overflow-hidden rounded-xl border bg-surface shadow-lg origin-top",
              danger ? "border-danger/60" : "border-border",
            )}
            onClick={(e) => e.stopPropagation()}
          >
            <div
              className={cn(
                "flex items-center justify-between border-b bg-surface-overlay px-4 py-2.5",
                danger ? "border-danger/40" : "border-border",
              )}
            >
              <span
                className={cn(
                  "inline-flex items-center gap-2 font-sans text-sm font-semibold",
                  danger ? "text-danger" : "text-text",
                )}
              >
                {danger && (
                  <AlertTriangle className="h-3.5 w-3.5" strokeWidth={2} />
                )}
                {title}
              </span>
              <button
                type="button"
                onClick={onCancel}
                aria-label="Close"
                className={cn(
                  "grid h-6 w-6 place-items-center rounded-md border border-border text-text-muted hover:bg-surface hover:text-text cursor-pointer",
                  transitionInteractive,
                )}
              >
                <X className="h-3 w-3" strokeWidth={2} />
              </button>
            </div>
            <div className="p-4">
              <div className="font-sans text-sm text-text-muted leading-relaxed">
                {body}
              </div>

              {requiresToken && (
                <div className="mt-4">
                  <label className="font-mono text-xs uppercase tracking-[0.08em] text-text-dim block mb-1.5">
                    Type{" "}
                    <span className="text-text font-semibold">
                      {confirmToken}
                    </span>{" "}
                    to confirm
                  </label>
                  <input
                    ref={inputRef}
                    value={typed}
                    onChange={(e) => setTyped(e.target.value)}
                    placeholder={confirmToken}
                    className={cn(
                      "h-9 w-full rounded-md border border-border bg-surface px-2.5 text-xs font-mono uppercase text-text placeholder:text-text-dim",
                      "focus:outline-none focus:border-accent focus:shadow-[var(--glow-soft)]",
                      transitionInteractive,
                    )}
                  />
                </div>
              )}

              <div className="flex items-center gap-2 mt-4 justify-end">
                <Button
                  ref={cancelRef}
                  variant="outline"
                  size="sm"
                  onClick={onCancel}
                >
                  {cancelLabel}
                </Button>
                <Button
                  variant={danger ? "danger" : "default"}
                  size="sm"
                  onClick={onConfirm}
                  disabled={!tokenMatches}
                >
                  {confirmLabel}
                </Button>
              </div>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
