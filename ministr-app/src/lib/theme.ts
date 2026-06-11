/**
 * Theme: follow the OS by default, with a persisted System/Light/Dark
 * override (gui-rw-theme-follow-system). The resolve logic is a pure
 * function so the behavior is unit-testable without touching the
 * dual-theme test harness's documentElement.
 */

export type ThemePref = "system" | "light" | "dark";

const STORAGE_KEY = "ministr-theme";

export function resolveDark(pref: ThemePref, systemDark: boolean): boolean {
  if (pref === "dark") return true;
  if (pref === "light") return false;
  return systemDark;
}

export function loadPref(): ThemePref {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    // storage unavailable → follow system
  }
  return "system";
}

function apply(dark: boolean) {
  document.documentElement.classList.toggle("dark", dark);
}

let setter: ((pref: ThemePref) => void) | null = null;

/**
 * Apply the persisted preference and follow live OS changes. Call once
 * before first render; ThemePick mutates via [`setThemePref`].
 */
export function initTheme(): void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  let pref = loadPref();

  apply(resolveDark(pref, media.matches));
  media.addEventListener("change", (e) => {
    apply(resolveDark(pref, e.matches));
  });

  setter = (next: ThemePref) => {
    pref = next;
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // storage unavailable → the choice just doesn't persist
    }
    apply(resolveDark(pref, media.matches));
  };
}

/** Set + persist the preference (no-op before initTheme, e.g. in stories). */
export function setThemePref(pref: ThemePref): void {
  setter?.(pref);
}
