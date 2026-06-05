import { useState } from "react";
import { Check, Copy } from "@/components/ui/icons";
import { motion } from "motion/react";
import type { SessionDetail } from "../../../lib/types";
import { fadeRise } from "../../../lib/motion";
import { ENDED_VERDICT, pressureVerdict, sessionStatus } from "../../../lib/sessions";
import { toneTextClass } from "../../../lib/status";
import { relative } from "../../../lib/time";
import { cn } from "../../../lib/utils";
import { StatusDot } from "../../ui/status-dot";

interface Props {
  session: SessionDetail;
  /** Present in the live `list_sessions` result (vs a history snapshot). */
  isLive: boolean;
  /** The store's last poll failed — numbers are last-known. */
  stale: boolean;
  /** `current_turn` advanced on the latest poll. */
  fresh: boolean;
  parent: SessionDetail | null;
  childCount: number;
  /** ms epoch — used to compute the "started Xs ago" chip; defaults to now. */
  startedMs?: number;
  onOpenParent?: () => void;
}

/**
 * Single-line identity strip — the new hero under the code-intelligence
 * framing. Replaces the old token-budget-dominated hero. Big donut +
 * usage block live in §Token usage now (collapsed by default).
 *
 * Layout: status dot + session id (truncate) + verdict chip + chips for
 * client / turn / delivered / lineage / age. Chips wrap on narrow widths;
 * the lowest-priority `started Xs ago` chip hides below `sm`.
 */
export function SessionHeroStrip({
  session,
  isLive,
  stale,
  fresh,
  parent,
  childCount,
  startedMs,
  onOpenParent,
}: Props) {
  const [copied, setCopied] = useState(false);
  const status = sessionStatus(session);
  const ended = !isLive;
  const verdict = ended ? ENDED_VERDICT : pressureVerdict(session.pressure_level);

  const copyId = () => {
    navigator.clipboard.writeText(session.session_id).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1000);
      },
      () => {},
    );
  };

  return (
    <motion.header
      variants={fadeRise}
      initial="initial"
      animate="animate"
      className={cn(
        "overflow-hidden rounded-lg bg-surface",
        status.tone === "danger" && !ended
          ? "border border-danger ring-1 ring-danger"
          : "border border-border",
      )}
    >
      {(ended || stale) && (
        <div
          role="status"
          className="flex items-center gap-2 border-b border-border bg-surface-overlay px-3.5 py-1.5 font-mono text-mono-mini uppercase tracking-[0.06em] text-text-dim"
        >
          <StatusDot tone={ended ? "muted" : "warning"} />
          {ended
            ? "Session ended — last known state"
            : "Stale — daemon not responding, retrying"}
        </div>
      )}

      <div
        aria-live="polite"
        aria-atomic="true"
        className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-3.5 py-2.5"
      >
        {/* Identity: dot + session id + copy button */}
        <div className="flex items-center gap-2 min-w-0 flex-shrink basis-[12rem]">
          <StatusDot
            tone={status.tone}
            size="md"
            pulse={isLive && !ended ? "live" : "off"}
          />
          <span
            className="font-mono text-sm font-bold text-text truncate min-w-0"
            title={session.session_id}
          >
            {session.session_id}
          </span>
          <button
            type="button"
            onClick={copyId}
            title="Copy session id"
            aria-label="Copy session id"
            className="grid h-5 w-5 shrink-0 place-items-center rounded-md border border-border bg-surface text-text-muted hover:text-text hover:border-border-hover cursor-pointer transition-colors duration-150"
          >
            {copied ? (
              <Check className="h-3 w-3 text-success" strokeWidth={2.5} />
            ) : (
              <Copy className="h-3 w-3" strokeWidth={2} />
            )}
          </button>
        </div>

        {/* Verdict chip (status word + utilization%) */}
        <span
          className={cn(
            "whitespace-nowrap rounded-md border border-border-soft px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.08em] shrink-0",
            toneTextClass(status.tone),
          )}
        >
          {verdict.word} · {status.pct}%
        </span>

        {/* Live chip */}
        {isLive && !ended && (
          <span className="whitespace-nowrap rounded-md border border-border-soft px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.08em] text-success shrink-0">
            live
          </span>
        )}

        {/* Lineage chips */}
        {session.parent_session_id && (
          <button
            type="button"
            onClick={onOpenParent}
            disabled={!onOpenParent}
            className={cn(
              "whitespace-nowrap rounded-md border border-border-soft px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.08em] shrink-0 transition-colors duration-150 ease-out",
              onOpenParent
                ? "text-accent hover:underline cursor-pointer"
                : "text-text-dim cursor-default",
            )}
          >
            ↳ subagent of {session.parent_session_id.slice(0, 8)}
          </button>
        )}
        {childCount > 0 && (
          <span className="whitespace-nowrap rounded-md border border-border-soft px-1.5 py-0.5 font-mono text-mono-mini uppercase tracking-[0.08em] text-text-muted shrink-0">
            {childCount} subagent{childCount === 1 ? "" : "s"}
          </span>
        )}

        {/* Client */}
        {session.client_name && (
          <span className="whitespace-nowrap font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim shrink-0">
            client {session.client_name}
          </span>
        )}

        {/* Turn (pulses on fresh) */}
        <span
          className={cn(
            "whitespace-nowrap font-mono text-mono-mini uppercase tracking-[0.08em] tabular-nums shrink-0",
            fresh ? "ministr-pulse text-text" : "text-text-dim",
          )}
        >
          turn {session.current_turn}
        </span>

        {/* Delivered count (when non-zero) */}
        {session.delivered_count > 0 && (
          <span className="whitespace-nowrap font-mono text-mono-mini uppercase tracking-[0.08em] tabular-nums text-text-dim shrink-0">
            {session.delivered_count} in context
          </span>
        )}

        {/* Started ago (hidden on very narrow widths) */}
        {startedMs !== undefined && (
          <span className="hidden sm:inline whitespace-nowrap font-mono text-mono-mini uppercase tracking-[0.08em] tabular-nums text-text-dim shrink-0">
            started {relative(Date.now(), startedMs)}
          </span>
        )}
      </div>
    </motion.header>
  );
}
