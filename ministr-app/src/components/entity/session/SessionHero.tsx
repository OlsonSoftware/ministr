import { useState } from "react";
import {
  Activity,
  AlertOctagon,
  AlertTriangle,
  Check,
  Copy,
  Minus,
} from "lucide-react";
import type { SessionDetail } from "../../../lib/types";
import { formatTokens } from "../../../lib/format";
import { toneTextClass } from "../../../lib/status";
import {
  clampPct,
  ENDED_VERDICT,
  pressureVerdict,
  projectCritical,
  type SessionSample,
  sessionStatus,
  thresholdsFor,
} from "../../../lib/sessions";
import { cn } from "../../../lib/utils";
import { BudgetRing } from "../../ui/budget-ring";
import { BudgetBar } from "../../ui/budget-bar";
import { StatusDot } from "../../ui/status-dot";

interface Props {
  session: SessionDetail;
  samples: readonly SessionSample[];
  /** Present in the live `list_sessions` result (vs a history snapshot). */
  isLive: boolean;
  /** The store's last poll failed — numbers are last-known. */
  stale: boolean;
  /** `current_turn` advanced on the latest poll. */
  fresh: boolean;
  parent: SessionDetail | null;
  childCount: number;
  onOpenParent?: () => void;
}

const ICON = {
  normal: Activity,
  elevated: AlertTriangle,
  critical: AlertOctagon,
} as const;

/**
 * The status moment. The one-second answer to "is this agent about to be
 * context-throttled?" — dominant ring + unmissable bar + plain verdict.
 * Colour is utilization-derived (never the raw enum), words come from the
 * authoritative enum, both routed through `lib/sessions`.
 */
export function SessionHero({
  session,
  samples,
  isLive,
  stale,
  fresh,
  parent,
  childCount,
  onOpenParent,
}: Props) {
  const [copied, setCopied] = useState(false);
  const thresholds = thresholdsFor(session);
  const status = sessionStatus(session, thresholds);
  const ended = !isLive;
  const verdict = ended ? ENDED_VERDICT : pressureVerdict(session.pressure_level);
  const Icon = ended
    ? Minus
    : (ICON[session.pressure_level as keyof typeof ICON] ?? Activity);
  const proj = projectCritical(session, samples, thresholds);

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
    <header
      className={cn(
        "bg-surface px-4 py-3.5 space-y-3",
        status.tone === "danger" && !ended
          ? "border-2 border-danger"
          : "border border-border-soft",
      )}
    >
      {(ended || stale) && (
        <div
          role="status"
          className="flex items-center gap-2 -mx-4 -mt-3.5 mb-1 border-b border-border-soft bg-surface-overlay px-4 py-1 font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim"
        >
          <StatusDot tone={ended ? "muted" : "warning"} />
          {ended
            ? "Session ended — last known state"
            : "Stale — daemon not responding, retrying"}
        </div>
      )}

      {/* Identity */}
      <div className="flex items-center gap-2">
        <StatusDot
          tone={status.tone}
          size="md"
          pulse={isLive && !ended ? "live" : "off"}
        />
        <span className="font-mono text-base font-bold text-text break-all flex-1 min-w-0">
          {session.session_id}
        </span>
        <button
          type="button"
          onClick={copyId}
          title="Copy session id"
          aria-label="Copy session id"
          className="grid h-6 w-6 shrink-0 place-items-center border border-border bg-surface text-text-muted hover:text-text hover:border-border-hover cursor-pointer transition-none rounded-sm"
        >
          {copied ? (
            <Check className="h-3 w-3 text-success" strokeWidth={2.5} />
          ) : (
            <Copy className="h-3 w-3" strokeWidth={2} />
          )}
        </button>
      </div>

      {/* Meta — lineage · client · turn (announced politely on change) */}
      <p
        aria-live="polite"
        aria-atomic="true"
        className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-mono-mini uppercase tracking-[0.05em] text-text-dim"
      >
        {session.parent_session_id && (
          <button
            type="button"
            onClick={onOpenParent}
            disabled={!onOpenParent}
            className={cn(
              "uppercase tracking-[0.05em] transition-none",
              onOpenParent
                ? "text-accent hover:underline cursor-pointer"
                : "text-text-dim cursor-default",
            )}
          >
            ↳ subagent of {session.parent_session_id.slice(0, 8)}
          </button>
        )}
        {childCount > 0 && (
          <span>
            {childCount} subagent{childCount === 1 ? "" : "s"}
          </span>
        )}
        {session.client_name && <span>client {session.client_name}</span>}
        <span className={cn(fresh && "ministr-flash text-text")}>
          turn {session.current_turn}
        </span>
      </p>

      {/* Verdict */}
      <div className="flex items-center gap-4">
        <BudgetRing
          utilization={session.utilization}
          pressure={status.pressure}
          size={64}
          stroke={7}
        >
          <span className="font-mono text-lg font-bold tabular-nums text-text leading-none">
            {status.pct}
            <span className="text-xs text-text-dim">%</span>
          </span>
        </BudgetRing>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex items-center gap-2">
            <span
              className={cn(
                "grid h-6 w-6 shrink-0 place-items-center border-2 border-border bg-surface",
                toneTextClass(status.tone),
              )}
            >
              <Icon className="h-3.5 w-3.5" strokeWidth={2.5} />
            </span>
            <span
              className={cn(
                "font-mono text-sm font-bold uppercase tracking-[0.05em]",
                toneTextClass(status.tone),
              )}
            >
              {verdict.word}
            </span>
          </div>
          <p className="font-sans text-sm text-text-muted">
            {verdict.sentence(status.pct)}
          </p>
          <BudgetBar
            utilization={session.utilization}
            size="hero"
            thresholds={thresholds}
          />
          <p className="font-mono text-xs tabular-nums text-text-dim">
            {formatTokens(session.tokens_used)} used ·{" "}
            {formatTokens(session.tokens_remaining)} free
            {!ended && proj && proj.turns != null && (
              <span className={toneTextClass(status.tone)}>
                {" "}
                · ≈ {proj.turns} turn{proj.turns === 1 ? "" : "s"} to limit
              </span>
            )}
            {!ended && proj && proj.turns == null && (
              <span> · stable</span>
            )}
          </p>
        </div>
      </div>
    </header>
  );
}
