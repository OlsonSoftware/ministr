import { useEffect, useState } from "react";

const DEFAULT_TAB_KEY = "ministr-default-tab";
const DENSITY_KEY = "ministr-density";

export type DefaultTab =
  | "ask"
  | "explore"
  | "projects"
  | "sessions";

export type Density = "comfortable" | "compact";

const VALID_DEFAULT_TABS: DefaultTab[] = [
  "ask",
  "explore",
  "projects",
  "sessions",
];

/**
 * Display options for the Settings → Default tab dropdown.
 * Keep in sync with [`DefaultTab`] / [`VALID_DEFAULT_TABS`] — this is the
 * single source of truth so adding a tab elsewhere doesn't silently
 * leave the dropdown stale.
 */
export const DEFAULT_TAB_OPTIONS: { value: DefaultTab; label: string }[] = [
  { value: "ask", label: "ASK" },
  { value: "explore", label: "EXPLORE" },
  { value: "projects", label: "PROJECTS" },
  { value: "sessions", label: "SESSIONS" },
];

/** Default-tab-on-launch preference, persisted to localStorage. */
export function useDefaultTab() {
  const [defaultTab, setDefaultTabRaw] = useState<DefaultTab>(() => {
    try {
      const v = localStorage.getItem(DEFAULT_TAB_KEY);
      if (v && VALID_DEFAULT_TABS.includes(v as DefaultTab)) return v as DefaultTab;
    } catch {
      /* ignore */
    }
    return "ask";
  });
  function setDefaultTab(t: DefaultTab) {
    setDefaultTabRaw(t);
    try {
      localStorage.setItem(DEFAULT_TAB_KEY, t);
    } catch {
      /* ignore */
    }
  }
  return { defaultTab, setDefaultTab };
}

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
    localStorage.removeItem(DEFAULT_TAB_KEY);
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
