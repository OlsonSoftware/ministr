import { TrustMark } from "./TrustMark";

/**
 * The degraded-connection line (gui-rw-daemon-down-states): shown when
 * a screen still has last-good data but polls are failing. Honest about
 * both facts — what you see is real but old, and recovery is automatic
 * (the poll retries; no button theater).
 */
export function ConnectionNote() {
  return (
    <p role="status" className="flex items-center gap-2 text-xs text-dim">
      <TrustMark state="stale" />
      connection to ministr lost — showing the last good view; reconnecting…
    </p>
  );
}
