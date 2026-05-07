/**
 * ConfirmDialog — single confirmation primitive for destructive actions.
 *
 * Replaces the two parallel patterns the app used to ship:
 *   - `ModalShell` + `RemoveConfirmModal` / `ReindexConfirmModal` in
 *     `ProjectList.tsx`.
 *   - `TypedConfirmModal` in `Settings.tsx` (used for reset-onboarding /
 *     factory-reset).
 *
 * Type-to-confirm is opt-in via the `confirmToken` prop. When present the
 * confirm button is disabled until the user types the token (case-insensitive,
 * trimmed). Used for high-blast-radius actions like project removal.
 */
import { useState } from "react";
import { AlertTriangle, X } from "lucide-react";
import { Button } from "./button";
import { cn } from "../../lib/utils";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  /** Body content — string or JSX. Rendered inside the modal body. */
  body: React.ReactNode;
  /** Visible label for the confirm button. Defaults to "Confirm". */
  confirmLabel?: string;
  /** Visible label for the cancel button. Defaults to "Cancel". */
  cancelLabel?: string;
  /** Visual treatment. `danger` styles header + confirm button red. */
  tone?: "danger" | "default";
  /**
   * Optional type-to-confirm token. When set, the confirm button is
   * disabled until the user types this (case-insensitive). Used for
   * high-blast-radius actions like project removal.
   */
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

  if (!open) return null;

  const danger = tone === "danger";
  const requiresToken = !!confirmToken;
  const tokenMatches = requiresToken
    ? typed.trim().toUpperCase() === confirmToken.toUpperCase()
    : true;

  return (
    <div
      className="fixed inset-0 z-[1100] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "20vh" }}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onClick={onCancel}
    >
      <div
        className={cn(
          "w-full max-w-md border-2 bg-surface shadow-lg",
          danger ? "border-danger" : "border-border",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className={cn(
            "flex items-center justify-between border-b-2 bg-surface-overlay px-3 py-2",
            danger ? "border-danger" : "border-border",
          )}
        >
          <span
            className={cn(
              "inline-flex items-center gap-2 font-mono text-mono-mini font-bold uppercase tracking-[0.05em]",
              danger ? "text-danger" : "text-text",
            )}
          >
            {danger && <AlertTriangle className="h-3 w-3" strokeWidth={2.5} />}
            {title}
          </span>
          <button
            type="button"
            onClick={onCancel}
            aria-label="Close"
            className="grid h-6 w-6 place-items-center border-2 border-border hover:bg-surface-overlay hover:text-text cursor-pointer transition-none"
          >
            <X className="h-3 w-3" strokeWidth={2.5} />
          </button>
        </div>
        <div className="p-4">
          <div className="font-mono text-xs text-text leading-relaxed">
            {body}
          </div>

          {requiresToken && (
            <div className="mt-4">
              <label className="font-mono text-xs tracking-[0.05em] text-text-dim block mb-1">
                TYPE{" "}
                <span className="text-text font-bold">{confirmToken}</span> TO
                CONFIRM
              </label>
              <input
                autoFocus
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                placeholder={confirmToken}
                className="h-9 w-full border border-border-soft bg-surface px-2 text-xs font-mono uppercase text-text placeholder:text-text-dim focus:outline-none focus:bg-surface-overlay transition-none"
              />
            </div>
          )}

          <div className="flex items-center gap-2 mt-4 justify-end">
            <Button variant="outline" size="sm" onClick={onCancel}>
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
      </div>
    </div>
  );
}
