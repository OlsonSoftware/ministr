import { useState } from "react";
import type { ThemePref } from "../../lib/theme";
import { loadPref, setThemePref } from "../../lib/theme";

const CHOICES: { pref: ThemePref; label: string }[] = [
  { pref: "system", label: "match my Mac" },
  { pref: "light", label: "light" },
  { pref: "dark", label: "dark" },
];

/**
 * The quiet System/Light/Dark triple (gui-rw-theme-follow-system).
 * Pure presentation — the actual class flip lives in lib/theme's
 * initTheme setter, passed in by App.
 */
export function ThemePick() {
  const [active, setActive] = useState<ThemePref>(loadPref);

  return (
    <div
      role="radiogroup"
      aria-label="appearance"
      className="flex items-center gap-1 text-xs"
    >
      {CHOICES.map(({ pref, label }) => (
        <button
          key={pref}
          type="button"
          role="radio"
          aria-checked={active === pref}
          onClick={() => {
            setActive(pref);
            setThemePref(pref);
          }}
          className={`rounded-md px-2 py-1 transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand ${
            active === pref
              ? "bg-sunken text-ink"
              : "text-dim hover:text-ink"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
