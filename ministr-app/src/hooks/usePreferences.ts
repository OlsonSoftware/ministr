import { useEffect, useState } from "react";

const DENSITY_KEY = "ministr-density";

export type Density = "comfortable" | "compact";

/** Density preference (affects card padding globally). */
export function useDensity() {
  const [density, setDensityRaw] = useState<Density>(() => {
    try {
      const v = localStorage.getItem(DENSITY_KEY);
      if (v === "comfortable" || v === "compact") return v;
    } catch {
      /* ignore */
    }
    return "comfortable";
  });
  function setDensity(d: Density) {
    setDensityRaw(d);
    try {
      localStorage.setItem(DENSITY_KEY, d);
    } catch {
      /* ignore */
    }
  }

  // Reflect density on the document so global CSS can switch padding via the
  // `[data-density="compact"]` selector if components opt in.
  useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

  return { density, setDensity };
}

/** Reset all preferences. */
export function resetPreferences() {
  try {
    localStorage.removeItem(DENSITY_KEY);
    localStorage.removeItem("ministr-theme");
    localStorage.removeItem("ministr-active-corpus");
    localStorage.removeItem("ministr-sessions-drawer-open");
    // Persisted ended-session history (24h window). Without this the
    // "Reset preferences" action reports success but stale entries
    // reappear on reload from cache.
    localStorage.removeItem("ministr-sessions-history-v1");
  } catch {
    /* ignore */
  }
}
