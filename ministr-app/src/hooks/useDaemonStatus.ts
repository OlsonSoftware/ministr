import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DaemonStatus } from "../lib/types";

/** Wait for Tauri IPC bridge to be available (handles hard reloads). */
async function waitForTauri(timeoutMs = 5000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if ((window as any).__TAURI_INTERNALS__) return true;
    await new Promise((r) => setTimeout(r, 50));
  }
  return false;
}

/**
 * The daemon-status heartbeat that backs the whole shell.
 *
 * ministr runs hidden in the tray by default, so a fixed `setInterval`
 * would poll forever against an invisible window — wasted IPC, CPU and
 * battery. This mirrors the visibility-aware, backoff-on-error pattern
 * already used by `useSessions`/`useSessionActivity`:
 *
 * - paused while `document.hidden` (re-polls immediately on re-show),
 * - exponential backoff on error up to `MAX_MS` (recovers to the base
 *   cadence on the next success),
 * - `refresh()` forces an immediate poll (used after mutations like
 *   add-project) regardless of the schedule.
 */
export function useDaemonStatus(intervalMs = 2000) {
  const MAX_MS = 30_000;

  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const ready = useRef(false);
  const timer = useRef<number | null>(null);
  const backoff = useRef(intervalMs);
  const mounted = useRef(true);
  // Poll function lives in a ref so the visibilitychange listener and
  // the public `refresh` always call the latest closure without
  // re-binding the listener on every render.
  const pollRef = useRef<() => void>(() => {});

  const schedule = useCallback((ms: number) => {
    if (timer.current !== null) clearTimeout(timer.current);
    // Hidden window: don't arm a timer. `onVisibility` re-polls the
    // instant the window is shown again.
    if (typeof document !== "undefined" && document.hidden) {
      timer.current = null;
      return;
    }
    timer.current = window.setTimeout(() => void pollRef.current(), ms);
  }, []);

  const poll = useCallback(async () => {
    if (!ready.current) {
      ready.current = await waitForTauri();
      if (!ready.current) {
        if (mounted.current) setError("Tauri IPC bridge not available");
        schedule(MAX_MS);
        return;
      }
    }
    try {
      const s = await invoke<DaemonStatus>("daemon_status");
      if (!mounted.current) return;
      setStatus(s);
      setError(null);
      backoff.current = intervalMs;
    } catch (e) {
      const msg = String(e);
      console.error("[ministr] daemon_status failed:", msg);
      if (!mounted.current) return;
      setError(msg);
      backoff.current = Math.min(backoff.current * 2, MAX_MS);
    } finally {
      if (mounted.current) schedule(backoff.current);
    }
  }, [intervalMs, schedule]);

  pollRef.current = () => void poll();

  /** Force an immediate refresh (post-mutation), then resume cadence. */
  const refresh = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    void poll();
  }, [poll]);

  useEffect(() => {
    mounted.current = true;

    function onVisibility() {
      // Became visible with no timer armed (we were paused) → catch up.
      if (!document.hidden && timer.current === null) {
        void pollRef.current();
      }
    }

    void pollRef.current();
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisibility);
    }

    return () => {
      mounted.current = false;
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = null;
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", onVisibility);
      }
    };
  }, []);

  return { status, error, refresh };
}
