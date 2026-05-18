/**
 * Session-domain derivations: the single source of truth for turning a
 * `SessionDetail` into the things the UI actually renders — a colour
 * (`Tone`), a plain-language verdict, vitals, time-series buckets, and
 * budget projections.
 *
 * Why this module exists
 * ----------------------
 * The daemon emits `pressure_level` as exactly `"normal" | "elevated" |
 * "critical"` (Rust `PressureLevel`, snake_case). The rest of the
 * frontend's pressure vocabulary (`./pressure`) is the five-bucket
 * `"none" | "low" | "medium" | "high" | "critical"` colour ramp. The old
 * code fed the raw daemon string straight into `pressureTone()` /
 * `BudgetRing`, so `"normal"` and `"elevated"` matched no key and the most
 * common sessions rendered grey / colourless.
 *
 * The fix, centralised here:
 *  - **Colour** is derived from numeric `utilization` vs the budget
 *    thresholds — always correct, five-level granularity.
 *  - **Verdict text** is mapped from the authoritative raw enum to plain
 *    words (no "pressure/normal/elevated" jargon in the UI).
 *
 * Everything is pure and null-safe: missing data / divide-by-zero return
 * `null` so callers render an em-dash rather than `NaN`.
 */

import type { Tone } from "./status";
import type { ActivityEvent, SessionDetail } from "./types";
import { PRESSURE_CRITICAL, PRESSURE_ELEVATED, type Pressure } from "./pressure";

export type { Pressure } from "./pressure";

/** The daemon's authoritative pressure enum, as it arrives on the wire
 *  (`SessionDetail.pressure_level`). Snake_case, three states only. */
export type RawPressure = "normal" | "elevated" | "critical";

// ── Budget thresholds ───────────────────────────────────────────────────────

/** Budget thresholds + capacity for a session. Until the Rust side
 *  surfaces them per-session (see `thresholdsFor`), these fall back to the
 *  canonical defaults that mirror `BudgetConfig::default` in
 *  `ministr-core/src/session/budget.rs`. */
export interface BudgetThresholds {
  /** Utilization (0..1) at/above which the daemon marks "elevated". */
  pressureThreshold: number;
  /** Utilization (0..1) at/above which the daemon marks "critical". */
  criticalThreshold: number;
}

export const DEFAULT_THRESHOLDS: BudgetThresholds = {
  pressureThreshold: PRESSURE_ELEVATED, // 0.80
  criticalThreshold: PRESSURE_CRITICAL, // 0.95
};

/** Resolve the budget thresholds for a session, preferring daemon-reported
 *  values (a newer daemon may not be running yet — hence optional) and
 *  falling back to the canonical defaults. */
export function thresholdsFor(
  session: SessionDetail | null,
): BudgetThresholds {
  return {
    pressureThreshold:
      session?.pressure_threshold ?? DEFAULT_THRESHOLDS.pressureThreshold,
    criticalThreshold:
      session?.critical_threshold ?? DEFAULT_THRESHOLDS.criticalThreshold,
  };
}

/** Total context-window budget in tokens. Prefers the daemon-reported
 *  window; otherwise reconstructs it from used + remaining (which is what
 *  the daemon itself divides to compute `utilization`). */
export function capacityOf(session: SessionDetail | null): number {
  if (!session) return 0;
  const reported = session.context_window_tokens;
  if (typeof reported === "number" && reported > 0) return reported;
  return session.tokens_used + session.tokens_remaining;
}

// ── Pressure → Tone (the canonical, total mapping) ──────────────────────────

const PRESSURE_TO_TONE: Record<Pressure, Tone> = {
  none: "muted",
  low: "success",
  medium: "accent",
  high: "warning",
  critical: "danger",
};

/** Total `Pressure` → `Tone` map. Unlike the string-keyed `pressureTone`
 *  in `./status`, this is exhaustive over the `Pressure` union — no
 *  silent `"muted"` fallback, so it cannot regress. */
export function pressureToTone(p: Pressure): Tone {
  return PRESSURE_TO_TONE[p];
}

/**
 * Map a 0..1 utilization to the five-bucket colour ramp, honouring the
 * (optionally daemon-reported) budget thresholds. With default thresholds
 * this is identical to `pressureFromUtilization` in `./pressure`; this
 * variant exists so the session UI tracks the *real* thresholds once Rust
 * surfaces them.
 */
export function pressureFromUtil(
  util: number,
  thresholds: BudgetThresholds = DEFAULT_THRESHOLDS,
): Pressure {
  if (util >= thresholds.criticalThreshold) return "critical";
  if (util >= thresholds.pressureThreshold) return "high";
  if (util >= 0.4) return "medium";
  if (util > 0) return "low";
  return "none";
}

/** The colour a session should render in, derived from utilization (the
 *  authoritative numeric signal) — never from the raw enum string. */
export function utilizationTone(
  util: number,
  thresholds?: BudgetThresholds,
): Tone {
  return pressureToTone(pressureFromUtil(util, thresholds));
}

const STATUS_LABEL: Record<Tone, string> = {
  muted: "IDLE",
  success: "OK",
  accent: "ACTIVE",
  warning: "TIGHT",
  danger: "CRITICAL",
};

/** Compact one-word status for tight spots (card header, lineage tag)
 *  where the full verdict ("UNDER PRESSURE") is too long. */
export function statusLabel(tone: Tone): string {
  return STATUS_LABEL[tone];
}

// ── Plain-language verdict (from the authoritative raw enum) ─────────────────

export interface PressureVerdict {
  /** UPPERCASE one/two-word status for the hero (no jargon). */
  word: string;
  /** Plain-language sentence; receives the integer utilization %. */
  sentence: (pct: number) => string;
}

const VERDICTS: Record<RawPressure, PressureVerdict> = {
  normal: {
    word: "HEALTHY",
    sentence: (p) => `${p}% of the context window in use`,
  },
  elevated: {
    word: "UNDER PRESSURE",
    sentence: (p) => `${p}% used — approaching the limit`,
  },
  critical: {
    word: "EVICTING",
    sentence: (p) => `${p}% used — trimming context to stay under the limit`,
  },
};

/** Verdict shown for a session that has ended / is a historical snapshot.
 *  Selected explicitly by the drawer, never inferred from the enum. */
export const ENDED_VERDICT: PressureVerdict = {
  word: "ENDED",
  sentence: (p) => `Final state — ${p}% of the window was in use`,
};

/** Plain-language verdict for the authoritative raw enum. Unknown values
 *  fall back to the healthy copy, but callers should pair this with
 *  `utilizationTone` so the *colour* (the dominant signal) stays correct
 *  regardless. */
export function pressureVerdict(raw: string): PressureVerdict {
  return VERDICTS[raw as RawPressure] ?? VERDICTS.normal;
}

// ── Composite status (ergonomic for the hero / card) ────────────────────────

export interface SessionStatus {
  tone: Tone;
  /** Utilization-derived five-bucket pressure (for sparkline ranking). */
  pressure: Pressure;
  verdict: PressureVerdict;
  /** Integer utilization percent (0..100), clamped. */
  pct: number;
}

/** One call → everything the hero/card needs to render the status moment.
 *  Colour from utilization; words from the raw enum. */
export function sessionStatus(
  session: Pick<SessionDetail, "utilization" | "pressure_level">,
  thresholds: BudgetThresholds = DEFAULT_THRESHOLDS,
): SessionStatus {
  const pressure = pressureFromUtil(session.utilization, thresholds);
  return {
    tone: pressureToTone(pressure),
    pressure,
    verdict: pressureVerdict(session.pressure_level),
    pct: clampPct(session.utilization * 100),
  };
}

// ── Numeric helpers ─────────────────────────────────────────────────────────

/** Division that yields `null` (→ render "—") instead of `Infinity`/`NaN`
 *  when the denominator is non-positive. */
export function safeDiv(n: number, d: number): number | null {
  return d > 0 ? n / d : null;
}

/** Clamp a percentage into 0..100 and round to an integer. */
export function clampPct(pct: number): number {
  return Math.round(Math.max(0, Math.min(100, pct)));
}

// ── Vitals ──────────────────────────────────────────────────────────────────

export interface SessionVitals {
  pct: number;
  tone: Tone;
  pressure: Pressure;
  tokensUsed: number;
  tokensFree: number;
  capacity: number;
  tokensSaved: number;
  dedupHits: number;
  /** `dedup_hits / total_deliveries` (0..1) or `null`. */
  cacheHitRate: number | null;
  evictions: number;
  compressions: number;
}

/** Derive every glanceable number for a session in one memo-friendly pass.
 *  Returns `null` when there is no session so callers show a skeleton. */
export function deriveVitals(
  session: SessionDetail | null,
  thresholds?: BudgetThresholds,
): SessionVitals | null {
  if (!session) return null;
  const t = thresholds ?? thresholdsFor(session);
  const pressure = pressureFromUtil(session.utilization, t);
  return {
    pct: clampPct(session.utilization * 100),
    tone: pressureToTone(pressure),
    pressure,
    tokensUsed: session.tokens_used,
    tokensFree: session.tokens_remaining,
    capacity: capacityOf(session),
    tokensSaved: session.total_tokens_saved,
    dedupHits: session.dedup_hits,
    cacheHitRate: safeDiv(session.dedup_hits, session.total_deliveries),
    evictions: session.total_evictions,
    compressions: session.total_compressions,
  };
}

// ── Poll-sampled time series ─────────────────────────────────────────────────

/** One poll observation of a live session, retained in a bounded ring by
 *  `useSessions` so the drawer can draw trend sparklines (the daemon keeps
 *  no per-session time series). */
export interface SessionSample {
  /** Epoch ms of the poll. */
  t: number;
  tokensUsed: number;
  /** 0..1. */
  utilization: number;
  turn: number;
}

/**
 * Bucket activity into `bucketCount` equal time windows and sum
 * `tokens_delta` per bucket (oldest → newest). Mirrors the proven
 * windowing of `computeHitRateBuckets` in `ui/activity-feed.tsx`; here we
 * accumulate token cost instead of a hit ratio so it can drive a
 * brutalist burn-rate bar sparkline.
 */
export function computeTokenBuckets(
  events: ActivityEvent[],
  bucketCount: number,
  windowMs: number,
): number[] {
  if (events.length === 0 || bucketCount <= 0) {
    return new Array(Math.max(0, bucketCount)).fill(0);
  }
  const now = Date.now();
  const bucketSize = windowMs / bucketCount;
  const buckets = new Array<number>(bucketCount).fill(0);

  for (const ev of events) {
    const age = now - ev.timestamp_ms;
    if (age < 0 || age > windowMs) continue;
    const idx = Math.min(
      bucketCount - 1,
      Math.max(0, bucketCount - 1 - Math.floor(age / bucketSize)),
    );
    buckets[idx] += ev.tokens_delta ?? 0;
  }
  return buckets;
}

// ── Burn rate & projection ──────────────────────────────────────────────────

export interface BurnRate {
  /** Net tokens added to the context window per second over the retained
   *  sample window. Negative means eviction freed more than was added —
   *  a real, displayable state. `null` with < 2 samples or zero span. */
  tokensPerSec: number | null;
  /** Net tokens per agent turn. `null` when no turn advanced across the
   *  window. */
  tokensPerTurn: number | null;
}

/** Burn rate from the retained sample ring (first vs last over the
 *  window — stable against single-poll jitter). */
export function burnRate(samples: readonly SessionSample[]): BurnRate {
  if (samples.length < 2) {
    return { tokensPerSec: null, tokensPerTurn: null };
  }
  const first = samples[0];
  const last = samples[samples.length - 1];
  const dTokens = last.tokensUsed - first.tokensUsed;
  const dSecs = (last.t - first.t) / 1000;
  const dTurns = last.turn - first.turn;
  return {
    tokensPerSec: dSecs > 0 ? dTokens / dSecs : null,
    tokensPerTurn: dTurns > 0 ? dTokens / dTurns : null,
  };
}

/** Tokens of headroom before the session hits the critical threshold.
 *  Clamped at 0 (already at/over the line). */
export function tokensToCritical(
  tokensUsed: number,
  capacity: number,
  criticalThreshold: number,
): number {
  return Math.max(0, criticalThreshold * capacity - tokensUsed);
}

export interface CriticalProjection {
  /** Tokens of headroom before the critical threshold. */
  tokensRemaining: number;
  /** Estimated agent turns until critical, or `null` when not trending up
   *  (burn ≤ 0) or unknowable. Render "stable" when `null`. */
  turns: number | null;
  /** Estimated seconds until critical, or `null`. */
  seconds: number | null;
}

/** Project when a session will hit its critical threshold, given the
 *  retained samples. Non-positive burn ⇒ `turns`/`seconds` are `null`
 *  ("stable / not trending up"), never a misleading huge ETA. */
export function projectCritical(
  session: SessionDetail | null,
  samples: readonly SessionSample[],
  thresholds?: BudgetThresholds,
): CriticalProjection | null {
  if (!session) return null;
  const t = thresholds ?? thresholdsFor(session);
  const capacity = capacityOf(session);
  const tokensRemaining = tokensToCritical(
    session.tokens_used,
    capacity,
    t.criticalThreshold,
  );
  const burn = burnRate(samples);
  const turns =
    burn.tokensPerTurn != null && burn.tokensPerTurn > 0
      ? Math.max(0, Math.floor(tokensRemaining / burn.tokensPerTurn))
      : null;
  const seconds =
    burn.tokensPerSec != null && burn.tokensPerSec > 0
      ? Math.max(0, Math.floor(tokensRemaining / burn.tokensPerSec))
      : null;
  return { tokensRemaining, turns, seconds };
}
