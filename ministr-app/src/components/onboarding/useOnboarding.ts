// F2.7 — onboarding wizard state hook.
//
// SOLID: single responsibility — persist the dismissed flag in
// localStorage and expose it as React state. Step completeness lives
// elsewhere (derived from cloud_status, billing, list_corpora) and is
// computed on every render — the wizard never sees stale data.

import { useCallback, useEffect, useState } from "react";

const STORAGE_KEY = "ministr.onboarding.v1";

interface PersistedState {
  /** User explicitly closed the wizard. Stays dismissed across restarts
   *  until the user re-opens it via the "Show onboarding" affordance. */
  dismissed: boolean;
  /** When the wizard was last dismissed (epoch ms). Surfaced so a
   *  future "remind me after X days" flow can re-prompt. F2.7 v0
   *  records the value but doesn't auto-prompt. */
  dismissedAt: number | null;
}

const DEFAULT: PersistedState = { dismissed: false, dismissedAt: null };

function load(): PersistedState {
  if (typeof window === "undefined") return DEFAULT;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw) as Partial<PersistedState>;
    return {
      dismissed: parsed.dismissed === true,
      dismissedAt: typeof parsed.dismissedAt === "number" ? parsed.dismissedAt : null,
    };
  } catch {
    return DEFAULT;
  }
}

function save(state: PersistedState) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* localStorage full or quota — silent fallback */
  }
}

export interface OnboardingHook {
  /** Whether the user has dismissed the wizard. When `true`, the
   *  wizard renders nothing. */
  dismissed: boolean;
  /** Mark the wizard dismissed. Persists immediately. */
  dismiss: () => void;
  /** Re-open the wizard. Surfaced via a "Show onboarding" link in
   *  Settings → Cloud's advanced section. */
  reopen: () => void;
}

export function useOnboarding(): OnboardingHook {
  const [state, setState] = useState<PersistedState>(load);

  // Cross-tab sync — localStorage events fire in other tabs when
  // setItem is called here.
  useEffect(() => {
    const handler = (e: StorageEvent) => {
      if (e.key === STORAGE_KEY) setState(load());
    };
    window.addEventListener("storage", handler);
    return () => window.removeEventListener("storage", handler);
  }, []);

  const dismiss = useCallback(() => {
    const next: PersistedState = { dismissed: true, dismissedAt: Date.now() };
    save(next);
    setState(next);
  }, []);

  const reopen = useCallback(() => {
    const next: PersistedState = { dismissed: false, dismissedAt: null };
    save(next);
    setState(next);
  }, []);

  return { dismissed: state.dismissed, dismiss, reopen };
}
