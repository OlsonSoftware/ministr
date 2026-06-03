/**
 * AccountSettings — the thin global "Account" area (AAA-VISION).
 *
 * The integrated workspace operates on the project spine through facets;
 * everything that is genuinely GLOBAL lives here, OUT of the project-scoped
 * Tend facet. It opens as an overlay from the chrome account control (and the
 * `cloud` nav verb), keeping it a small surface reachable from anywhere rather
 * than a sibling destination.
 *
 * Two top-level areas, dashboard-FIRST (aaa-cloud): it lands on the CLOUD
 * control room (connection + usage economics + corpora-as-assets + automation),
 * with a segmented toggle to the global SYSTEM prefs (the relocated
 * SettingsSurface). The cloud dashboard answers "is it healthy / what am I
 * spending" at a glance — not a settings list.
 */
import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Cloud, SlidersHorizontal, X } from "lucide-react";

import type { DaemonStatus } from "../../lib/types";
import { useDialog } from "../../hooks/useDialog";
import { popIn, scrim } from "../../lib/motion";
import { cloudClient } from "../../lib/cloudClient";
import { cn } from "../../lib/utils";
import { CloudControlRoomConnector } from "./CloudControlRoom";
import { SettingsSurface } from "./SettingsSurface";

type AccountArea = "cloud" | "system";

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
  const [area, setArea] = useState<AccountArea>("cloud");

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
            aria-label="Account"
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className="relative flex flex-col w-full max-w-4xl h-[80vh] min-h-0 rounded-xl border border-border bg-surface shadow-2xl overflow-hidden"
          >
            <header className="flex items-center justify-between gap-3 border-b border-border px-5 h-12 shrink-0">
              <div className="flex items-center gap-3 min-w-0">
                <h2 className="font-sans text-sm font-semibold text-text">
                  Account
                </h2>
                {/* Segmented area toggle — Cloud is the dashboard-first landing. */}
                <div
                  role="tablist"
                  aria-label="Account area"
                  className="flex items-center gap-0.5 rounded-md border border-border-soft bg-surface-overlay p-0.5"
                >
                  <AreaTab
                    icon={Cloud}
                    label="Cloud"
                    active={area === "cloud"}
                    onClick={() => setArea("cloud")}
                  />
                  <AreaTab
                    icon={SlidersHorizontal}
                    label="System"
                    active={area === "system"}
                    onClick={() => setArea("system")}
                  />
                </div>
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
              {area === "cloud" ? (
                <CloudControlRoomConnector
                  onManageBilling={() => {
                    void cloudClient.billingPortal();
                  }}
                />
              ) : (
                <SettingsSurface
                  status={status}
                  activeCorpusId={activeCorpusId}
                  theme={theme}
                  onThemeChange={onThemeChange}
                  onShowOnboarding={onShowOnboarding}
                  onRefresh={onRefresh}
                  onOpenLogs={onOpenLogs}
                />
              )}
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}

function AreaTab({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: typeof Cloud;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 h-6 px-2 rounded font-mono text-mono-mini uppercase tracking-[0.06em] cursor-pointer transition-colors duration-150",
        active
          ? "bg-surface text-text shadow-sm"
          : "text-text-dim hover:text-text",
      )}
    >
      <Icon className="h-3 w-3" strokeWidth={2} />
      {label}
    </button>
  );
}
