import { useEffect, useRef, useState } from "react";
import type { ThemePref } from "../../lib/theme";
import { loadPref, setThemePref } from "../../lib/theme";
import { daemonStatus, openExternal } from "../../lib/ipc";
import { usePoll } from "../../lib/usePoll";

/** Where "Documentation" goes — the public docs site. */
const DOCS_URL = "https://ministr.ai/docs";

const APPEARANCE: { pref: ThemePref; label: string }[] = [
  { pref: "system", label: "Match my Mac" },
  { pref: "light", label: "Light" },
  { pref: "dark", label: "Dark" },
];

/**
 * Settings & About — the app's one home for preferences, version, daemon
 * status, and help (gui-ux-settings-help-daemon-surface). A first-timer
 * could previously see no version, no daemon health, no docs link, and the
 * appearance control floated label-less in the header. This consolidates all
 * of it behind one labeled "Settings" trigger (NN/G: an icon needs a word).
 *
 * It invents no backend: there's no daemon-restart command, so it shows the
 * TRUE running version + live status + a relaunch hint rather than a fake
 * "Restart" button. The version comes from the daemon itself (the binary's
 * own, not the app bundle's Info.plist, which can drift).
 */
export function SettingsMenu() {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

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
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-sm text-dim transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        <GearIcon />
        Settings
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
          {/* The panel polls the daemon — mounted only while open, so the
              header costs nothing when the menu is closed. */}
          <SettingsPanel onClose={() => setOpen(false)} />
        </>
      ) : null}
    </div>
  );
}

function SettingsPanel({ onClose }: { onClose: () => void }) {
  const [appearance, setAppearance] = useState<ThemePref>(loadPref);
  const { data: status, error } = usePoll(daemonStatus, 5_000);
  const daemonDown = error != null && status == null;

  return (
    <div
      aria-label="settings and about"
      className="absolute right-0 z-10 mt-1 flex w-64 flex-col gap-3 rounded-md border border-line bg-surface p-3 text-sm shadow-sm"
    >
      <section aria-label="appearance" className="space-y-1">
        <SectionLabel>Appearance</SectionLabel>
        <div role="radiogroup" aria-label="appearance" className="flex flex-col gap-0.5">
          {APPEARANCE.map(({ pref, label }) => (
            <button
              key={pref}
              type="button"
              role="radio"
              aria-checked={appearance === pref}
              onClick={() => {
                setAppearance(pref);
                setThemePref(pref);
              }}
              className={`flex items-center justify-between rounded px-2 py-1.5 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand ${
                appearance === pref
                  ? "bg-sunken text-ink"
                  : "text-dim hover:text-ink"
              }`}
            >
              {label}
              {appearance === pref ? <span aria-hidden>✓</span> : null}
            </button>
          ))}
        </div>
      </section>

      <hr className="border-line" />

      <section aria-label="about ministr" className="space-y-1">
        <SectionLabel>ministr</SectionLabel>
        {daemonDown ? (
          <p className="px-2 text-dim">
            ministr isn’t running — relaunch this app to reconnect.
          </p>
        ) : status ? (
          <div className="space-y-0.5 px-2 text-dim">
            <p className="text-ink">Version {status.version}</p>
            <p>
              Running · up {formatUptime(status.uptime_secs)} ·{" "}
              {Math.round(status.memory_mb)} MB
            </p>
          </div>
        ) : (
          <p className="px-2 text-dim">Checking daemon…</p>
        )}
        {status?.log_path ? (
          <button
            type="button"
            onClick={() => void openExternal(status.log_path!)}
            className="w-full rounded px-2 py-1.5 text-left text-dim transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
          >
            Open log file
          </button>
        ) : null}
      </section>

      <hr className="border-line" />

      <button
        type="button"
        onClick={() => {
          void openExternal(DOCS_URL);
          onClose();
        }}
        className="flex w-full items-center justify-between rounded px-2 py-1.5 text-left text-dim transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brand"
      >
        Documentation
        <span aria-hidden>↗</span>
      </button>
    </div>
  );
}

function SectionLabel({ children }: { children: string }) {
  return (
    <p className="px-2 text-xs font-medium uppercase tracking-wide text-dim">
      {children}
    </p>
  );
}

/** Compact human uptime: "45s" · "12m" · "3h 20m" · "2d 4h". */
function formatUptime(secs: number): string {
  if (secs < 60) return `${Math.max(0, Math.floor(secs))}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}

/** A neutral gear — the conventional "settings" mark. */
function GearIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-4"
      aria-hidden
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <circle cx="8" cy="8" r="2.25" />
      <path d="M8 1.5v1.6M8 12.9v1.6M14.5 8h-1.6M3.1 8H1.5M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1M12.6 12.6l-1.1-1.1M4.5 4.5 3.4 3.4" />
    </svg>
  );
}
