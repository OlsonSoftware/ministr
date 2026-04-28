/**
 * Pressure-level helpers for client-side display.
 *
 * The thresholds here mirror the Rust source of truth in
 * `ministr-core/src/session/budget.rs::BudgetConfig::default` (0.80
 * elevated, 0.95 critical). The TS-side simulator and any other
 * client-only display widgets should import from here so a future
 * threshold tweak in Rust isn't silently out of sync.
 */

/** Utilization at or above which pressure is considered elevated. */
export const PRESSURE_ELEVATED = 0.8;

/** Utilization at or above which pressure is critical. */
export const PRESSURE_CRITICAL = 0.95;

/** Five-bucket pressure label used by the simulator and BudgetRing.
 *  Mirrors the daemon's enum but adds finer-grained `low`/`medium`
 *  buckets for the UI's color ramp below the official elevated cutoff. */
export type Pressure = "none" | "low" | "medium" | "high" | "critical";

/**
 * Map a 0..1 utilization ratio to a pressure label.
 *
 * Matches the daemon for `critical`/`high` (using the canonical 0.95 /
 * 0.80 thresholds). Below that, the UI splits the "normal" range into
 * `medium`/`low`/`none` purely for color grading on the client; those
 * buckets have no protocol meaning.
 */
export function pressureFromUtilization(util: number): Pressure {
  if (util >= PRESSURE_CRITICAL) return "critical";
  if (util >= PRESSURE_ELEVATED) return "high";
  if (util >= 0.4) return "medium";
  if (util > 0) return "low";
  return "none";
}
