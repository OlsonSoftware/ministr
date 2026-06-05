import { useEffect, useRef, useState } from "react";
import { Zap } from "@/components/ui/icons";
import { AnimatePresence, motion } from "motion/react";
import { popIn } from "../../lib/motion";
import { cn } from "../../lib/utils";
import type { CorpusInfo, SessionDetail } from "../../lib/types";
import { SessionRow } from "./SessionRow";

/**
 * The cross-cutting live ⚡ session layer.
 *
 * A ⚡N affordance in the workspace chrome (tinted by the worst pressure) that
 * opens the live-agents list from ANY facet — sessions are an object surfaced
 * contextually, not a sixth destination. The list is one {@link SessionRow}
 * renderer, sorted by context pressure, each row opening the shared inspector.
 *
 * Pure `sessions` prop (the parent scopes them to project|fleet) so the layer
 * renders populated in Storybook, where the `useSessions` store is Tauri-gated.
 */
export function SessionLayer({
  sessions,
  corpora,
  onOpenSession,
}: {
  sessions: readonly SessionDetail[];
  corpora: CorpusInfo[];
  onOpenSession?: (session: SessionDetail) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onClick(e: MouseEvent) {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const count = sessions.length;
  const hasCritical = sessions.some((s) =>
    (s.pressure_level ?? "").toLowerCase().includes("crit"),
  );
  const sorted = [...sessions].sort((a, b) => b.utilization - a.utilization);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`${count} live ${count === 1 ? "agent" : "agents"}`}
        title={`${count} live ${count === 1 ? "agent" : "agents"}`}
        className={cn(
          "inline-flex items-center gap-1.5 h-8 px-2.5 rounded-md cursor-pointer shrink-0",
          "border border-border bg-surface hover:bg-surface-overlay hover:border-border-hover",
          "transition-colors duration-150",
        )}
      >
        <Zap
          className={cn(
            "h-3.5 w-3.5",
            hasCritical
              ? "text-danger"
              : count > 0
                ? "text-accent"
                : "text-text-dim",
          )}
          strokeWidth={2}
          fill={count > 0 ? "currentColor" : "none"}
        />
        <span className="font-mono text-mono-mini tabular-nums text-text">
          {count}
        </span>
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            role="dialog"
            aria-label="Live agents"
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className={cn(
              "absolute top-full right-0 mt-2 z-50 origin-top-right",
              "w-[320px] overflow-hidden rounded-lg border border-border bg-surface shadow-lg",
            )}
          >
            <header className="flex items-center justify-between border-b border-border bg-surface-overlay px-3 py-2">
              <span className="font-sans text-sm font-semibold text-text">
                Live agents
              </span>
              <span className="font-mono text-mono-mini tabular-nums text-text-dim">
                {count}
              </span>
            </header>
            {count === 0 ? (
              <p className="px-3 py-6 text-center font-sans text-sm text-text-dim">
                No agents connected. Point an MCP client at a project to see it
                here.
              </p>
            ) : (
              <ul className="max-h-[360px] overflow-y-auto p-1">
                {sorted.map((s) => (
                  <li key={s.session_id}>
                    <SessionRow
                      session={s}
                      corpora={corpora}
                      onOpen={(sess) => {
                        onOpenSession?.(sess);
                        setOpen(false);
                      }}
                    />
                  </li>
                ))}
              </ul>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
