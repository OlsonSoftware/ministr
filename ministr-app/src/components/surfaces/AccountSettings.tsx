/**
 * AccountSettings — the thin global "Account" area (AAA-VISION).
 *
 * The integrated workspace operates on the project spine through facets;
 * everything that is genuinely GLOBAL (theme/density/autostart, AI-assistant
 * config, the daemon server, logs, maintenance) lives here, OUT of the
 * project-scoped Tend facet. It opens as an overlay from the chrome account
 * control (and the `cloud` nav verb), keeping it a small surface reachable
 * from anywhere rather than a sibling destination.
 *
 * This is a fresh overlay SHELL (scrim + panel + header, with full modal a11y
 * via useDialog) that relocates the existing global SettingsSurface as its
 * body — per the chunk acceptance ("global-only prefs relocate to a thin
 * Account area"). The richer Account build (cloud connection, team, billing)
 * is the aaa-cloud chunk.
 */
import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";

import type { DaemonStatus } from "../../lib/types";
import { useDialog } from "../../hooks/useDialog";
import { popIn, scrim } from "../../lib/motion";
import { SettingsSurface } from "./SettingsSurface";

interface Props {
  open: boolean;
  onClose: () => void;
  status: DaemonStatus;
  activeCorpusId: string | null;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

export function AccountSettings({
  open,
  onClose,
  status,
  activeCorpusId,
  theme,
  onThemeChange,
  onShowOnboarding,
  onRefresh,
  onOpenLogs,
}: Props) {
  const ref = useDialog<HTMLDivElement>(open, onClose);

  return (
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-50 grid place-items-center p-4 sm:p-8">
          {/* Scrim */}
          <motion.button
            type="button"
            aria-label="Close account settings"
            onClick={onClose}
            variants={scrim}
            initial="initial"
            animate="animate"
            exit="exit"
            className="absolute inset-0 bg-bg/70 backdrop-blur-sm cursor-default"
          />

          {/* Panel */}
          <motion.div
            ref={ref}
            role="dialog"
            aria-modal="true"
            aria-label="Account and settings"
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className="relative flex flex-col w-full max-w-3xl h-[80vh] min-h-0 rounded-xl border border-border bg-surface shadow-2xl overflow-hidden"
          >
            <header className="flex items-center justify-between gap-3 border-b border-border px-5 h-12 shrink-0">
              <div className="flex items-baseline gap-2 min-w-0">
                <h2 className="font-sans text-sm font-semibold text-text">
                  Account
                </h2>
                <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim truncate">
                  Global settings &amp; system
                </span>
              </div>
              <button
                type="button"
                onClick={onClose}
                aria-label="Close"
                className="grid place-items-center h-7 w-7 rounded-md text-text-dim hover:bg-surface-overlay hover:text-text transition-colors duration-150 cursor-pointer"
              >
                <X className="h-4 w-4" strokeWidth={2} />
              </button>
            </header>

            <div className="flex-1 min-h-0 overflow-hidden">
              <SettingsSurface
                status={status}
                activeCorpusId={activeCorpusId}
                theme={theme}
                onThemeChange={onThemeChange}
                onShowOnboarding={onShowOnboarding}
                onRefresh={onRefresh}
                onOpenLogs={onOpenLogs}
              />
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
