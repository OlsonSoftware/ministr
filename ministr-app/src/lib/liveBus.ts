/**
 * liveBus — a thin derivation layer over the shared session store that
 * turns raw poll snapshots into typed lifecycle events the shell can
 * react to (toasts, flashes, ambient motion).
 *
 * It owns no polling and no network: it diffs successive
 * `useSessions()` snapshots. This keeps the "one poll for the whole
 * app" guarantee intact while giving the UI a stream of meaningful
 * moments instead of having every component re-derive them.
 */
import { useEffect, useRef } from "react";
import { useSessions } from "../hooks/useSessions";
import type { SessionDetail } from "./types";

export type LiveEvent =
  | { kind: "session-started"; session: SessionDetail }
  | { kind: "session-ended"; sessionId: string }
  | { kind: "turn-advanced"; session: SessionDetail }
  | { kind: "pressure-critical"; session: SessionDetail };

/**
 * Subscribe to session lifecycle events. The handler is called once per
 * event per poll; it sees a stable identity so passing an inline arrow
 * is fine (we keep the latest in a ref).
 */
export function useLiveEvents(onEvent: (e: LiveEvent) => void): void {
  const { byId, freshIds, loaded } = useSessions();
  const prevIds = useRef<Set<string>>(new Set());
  const prevPressure = useRef<Map<string, string>>(new Map());
  const primed = useRef(false);
  const cb = useRef(onEvent);
  cb.current = onEvent;

  useEffect(() => {
    if (!loaded) return;
    // First loaded snapshot just seeds baseline state — emitting
    // "started" for every already-connected session would be noise.
    if (!primed.current) {
      primed.current = true;
      prevIds.current = new Set(byId.keys());
      for (const [id, s] of byId) {
        prevPressure.current.set(id, s.pressure_level);
      }
      return;
    }
    const seen = new Set<string>();
    for (const [id, s] of byId) {
      seen.add(id);
      if (!prevIds.current.has(id)) {
        cb.current({ kind: "session-started", session: s });
      }
      const prevP = prevPressure.current.get(id);
      if (s.pressure_level === "critical" && prevP !== "critical") {
        cb.current({ kind: "pressure-critical", session: s });
      }
      prevPressure.current.set(id, s.pressure_level);
      if (freshIds.has(id)) {
        cb.current({ kind: "turn-advanced", session: s });
      }
    }
    for (const id of prevIds.current) {
      if (!seen.has(id)) {
        cb.current({ kind: "session-ended", sessionId: id });
        prevPressure.current.delete(id);
      }
    }
    prevIds.current = seen;
  }, [byId, freshIds, loaded]);
}
