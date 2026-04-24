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

export function useDaemonStatus(intervalMs = 2000) {
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ready = useRef(false);

  const refresh = useCallback(async () => {
    if (!ready.current) {
      ready.current = await waitForTauri();
      if (!ready.current) {
        setError("Tauri IPC bridge not available");
        return;
      }
    }
    try {
      const s = await invoke<DaemonStatus>("daemon_status");
      setStatus(s);
      setError(null);
    } catch (e) {
      const msg = String(e);
      console.error("[ministr] daemon_status failed:", msg);
      setError(msg);
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, intervalMs);
    return () => clearInterval(id);
  }, [refresh, intervalMs]);

  return { status, error, refresh };
}
