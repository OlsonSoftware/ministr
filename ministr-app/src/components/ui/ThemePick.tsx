import { useEffect, useRef, useState } from "react";
import type { ThemePref } from "../../lib/theme";
import { loadPref, setThemePref } from "../../lib/theme";

const CHOICES: { pref: ThemePref; label: string }[] = [
  { pref: "system", label: "Match my Mac" },
  { pref: "light", label: "Light" },
  { pref: "dark", label: "Dark" },
];

/**
 * Appearance — a DEMOTED settings affordance (Clear Glass v5 C4). On
 * Home this used to be a three-button segmented control that out-shouted
 * the Brand and was the only control on the empty state. It is now a
 * single quiet, icon-only button (the half-filled "appearance" disc, in
 * neutral dim) that reveals the System/Light/Dark choices in a small
 * popover. Pure presentation — the class flip lives in lib/theme.
 */
export function ThemePick() {
  const [active, setActive] = useState<ThemePref>(loadPref);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // Escape closes; selection and outside-click (the backdrop) also close.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-label="Appearance"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex items-center rounded-md p-1.5 text-dim transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        <AppearanceIcon />
      </button>

      {open ? (
        <>
          {/* click-away backdrop */}
          <button
            type="button"
            aria-hidden
            tabIndex={-1}
            className="fixed inset-0 z-0 cursor-default"
            onClick={() => setOpen(false)}
          />
          <div
            role="radiogroup"
            aria-label="appearance"
            className="absolute right-0 z-10 mt-1 flex w-40 flex-col gap-0.5 rounded-md border border-line bg-surface p-1 text-sm shadow-sm"
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
                  setOpen(false);
                }}
                className={`flex items-center justify-between rounded px-2 py-1.5 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand ${
                  active === pref
                    ? "bg-sunken text-ink"
                    : "text-dim hover:text-ink"
                }`}
              >
                {label}
                {active === pref ? <span aria-hidden>✓</span> : null}
              </button>
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}

/** The half-filled disc — the conventional, neutral "appearance" mark. */
function AppearanceIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-4"
      aria-hidden
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <circle cx="8" cy="8" r="6" />
      <path d="M8 2a6 6 0 0 1 0 12z" fill="currentColor" stroke="none" />
    </svg>
  );
}
