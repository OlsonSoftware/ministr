/**
 * AboutPanel — version, maintenance, and the danger zone.
 *
 * Open-data-folder / open-log / re-run-onboarding plus the two
 * type-to-confirm destructive actions (reset preferences, clear cache).
 * Uses the unified ConfirmDialog rather than the old local
 * TypedConfirmModal — that finishes the "single confirmation pattern"
 * goal from the plan.
 */
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ExternalLink,
  FolderOpen,
  RefreshCw,
  Rocket,
  ScrollText,
  Trash2,
} from "lucide-react";

import type { DaemonStatus } from "../../lib/types";
import { resetPreferences } from "../../hooks/usePreferences";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { useToast } from "../shell/ToastTray";
import { SettingsSection, MaintAction, formatUptime } from "./settings-primitives";

// Public, brand-owned destination. ministr is closed-source: never link
// to the (private) GitHub repo from shipped UI.
const RELEASES_URL = "https://ministr.ai";
const DATA_DIR = "~/.ministr/";

interface Props {
  status: DaemonStatus;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs?: () => void;
}

export function AboutPanel({
  status,
  onShowOnboarding,
  onRefresh,
  onOpenLogs,
}: Props) {
  const { toast } = useToast();
  const [confirmReset, setConfirmReset] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  async function openDataFolder() {
    try {
      await invoke("open_path", { path: DATA_DIR });
      toast("DATA FOLDER OPENED", { tone: "info" });
    } catch (e) {
      toast("OPEN FAILED", { detail: String(e), tone: "danger" });
    }
  }

  async function openLogFile() {
    if (status.log_path) {
      try {
        await invoke("open_path", { path: status.log_path });
        toast("LOG FILE OPENED", { tone: "info" });
        return;
      } catch {
        /* fall back to the in-app log viewer */
      }
    }
    onOpenLogs?.();
  }

  async function rerunOnboarding() {
    await invoke("reset_onboarding");
    onShowOnboarding();
  }

  function performResetPreferences() {
    resetPreferences();
    setConfirmReset(false);
    toast("PREFERENCES RESET", { tone: "info" });
    // Force-reload so theme/density re-init from defaults.
    setTimeout(() => window.location.reload(), 200);
  }

  async function performClearCache() {
    setConfirmClear(false);
    let cleared = 0;
    for (const c of status.corpora) {
      try {
        await invoke("trigger_reindex", { corpusId: c.id });
        cleared++;
      } catch {
        /* skip */
      }
    }
    toast("CACHE CLEARED", {
      detail: `${cleared} ${cleared === 1 ? "project" : "projects"} re-indexing`,
      tone: "info",
    });
    onRefresh();
  }

  function checkForUpdates() {
    invoke("open_path", { path: RELEASES_URL }).catch(() => {});
  }

  function copyVersion() {
    navigator.clipboard
      .writeText(`ministr v${status.version}`)
      .then(() => toast("VERSION COPIED", { tone: "info" }))
      .catch(() => {});
  }

  return (
    <div className="space-y-4">
      <SettingsSection title="Maintenance" />
        <div className="grid grid-cols-2 md:grid-cols-3 gap-0 bg-surface-sunken rounded-lg overflow-hidden">
          <MaintAction
            icon={FolderOpen}
            label="OPEN DATA FOLDER"
            onClick={openDataFolder}
          />
          <MaintAction
            icon={ScrollText}
            label="OPEN LOG FILE"
            onClick={openLogFile}
          />
          <MaintAction
            icon={Rocket}
            label="RE-RUN ONBOARDING"
            onClick={rerunOnboarding}
          />
          <MaintAction
            icon={RefreshCw}
            label="RESET PREFERENCES"
            danger
            onClick={() => setConfirmReset(true)}
          />
          <MaintAction
            icon={Trash2}
            label="CLEAR CACHE"
            danger
            onClick={() => setConfirmClear(true)}
          />
        </div>

      <footer className="flex items-center justify-between gap-3 border-t border-border-soft pt-4 mt-6 font-mono text-xs uppercase tracking-[0.08em] text-text-dim">
        <button
          onClick={copyVersion}
          title="Click to copy version"
          className="text-text-dim hover:text-text cursor-pointer"
        >
          MINISTR · v{status.version} · UPTIME{" "}
          <span className="tabular-nums text-text-dim">
            {formatUptime(status.uptime_secs)}
          </span>
        </button>
        <button
          onClick={checkForUpdates}
          className="inline-flex items-center gap-1 border border-border bg-surface px-2 py-0.5 text-text hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
        >
          <ExternalLink className="h-3 w-3" strokeWidth={2.5} />
          Check for updates
        </button>
      </footer>

      <ConfirmDialog
        open={confirmReset}
        title="Reset preferences"
        tone="danger"
        confirmLabel="Reset"
        confirmToken="RESET"
        body={
          <>
            <p>
              Clears local-storage preferences (theme, default tab, density,
              drawer state, active project).
            </p>
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-2">
              Your project list is not touched.
            </p>
          </>
        }
        onCancel={() => setConfirmReset(false)}
        onConfirm={performResetPreferences}
      />

      <ConfirmDialog
        open={confirmClear}
        title="Clear cache"
        tone="danger"
        confirmLabel="Clear cache"
        confirmToken="CLEAR CACHE"
        body={
          <>
            <p>
              Drops indexes for{" "}
              <span className="font-bold">
                all {status.corpora.length}{" "}
                {status.corpora.length === 1 ? "project" : "projects"}
              </span>{" "}
              and triggers a re-index of each.
            </p>
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-2">
              Source files on disk are not touched.
            </p>
          </>
        }
        onCancel={() => setConfirmClear(false)}
        onConfirm={performClearCache}
      />
    </div>
  );
}
