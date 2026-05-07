import { type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../../lib/utils";
import { DaemonDot } from "../shell/DaemonDot";
import { BrutalDrawer } from "../ui/brutal-icons";
import type { DaemonStatus, CorpusInfo } from "../../lib/types";

interface StatusBarProps {
  status: DaemonStatus | null;
  error: string | null;
  activeCorpus: CorpusInfo | null;
  /** Open the bottom logs drawer. */
  onOpenLogs: () => void;
  /** Open the bottom session-vitals drawer. */
  onOpenSession: () => void;
  /** Open the bottom indexing-detail drawer. */
  onOpenIndexing: () => void;
  /** Open the command palette. */
  onOpenPalette: () => void;
  /** Open the settings modal. */
  onOpenSettings: () => void;
}

/**
 * Persistent bottom status bar — replaces the tab-shell's TopBar VitalsChips
 * and the buried Settings → Diagnostics → Logs path with a single ambient
 * surface. Every chip is a button; every button opens a drawer or modal.
 *
 * Layout (left → right):
 *   ● daemon-dot · CORPUS-NAME · MEM xxMB · ▣ BUDGET · ⟳ INDEX · ⌘K · ⚙
 */
export function StatusBar({
  status,
  error,
  activeCorpus,
  onOpenLogs,
  onOpenSession,
  onOpenIndexing,
  onOpenPalette,
  onOpenSettings,
}: StatusBarProps) {
  const indexingDetail = describeIndexing(status?.corpora ?? []);
  const indexingActive = indexingDetail !== null;

  return (
    <footer
      className={cn(
        "flex items-center justify-between gap-2 shrink-0",
        "border-t-2 border-border bg-surface px-3 py-1",
      )}
      role="contentinfo"
      aria-label="Workspace status"
    >
      {/* LEFT — daemon + corpus + memory. */}
      <div className="flex items-center gap-2 min-w-0">
        <DaemonDot
          status={status}
          error={error}
          onOpenLogs={async () => {
            // Try to open the log file in the OS default editor first; the
            // drawer is the in-app fallback so users always get *some*
            // surface even when open_path fails (sandboxed envs etc.).
            if (status?.log_path) {
              try {
                await invoke("open_path", { path: status.log_path });
              } catch {
                /* fall through to drawer */
              }
            }
            onOpenLogs();
          }}
        />

        {activeCorpus && (
          <Chip
            label="CORPUS"
            value={
              activeCorpus.display_name ??
              basename(activeCorpus.paths[0] ?? "")
            }
            title={activeCorpus.paths[0]}
          />
        )}

        {status && (
          <Chip
            label="MEM"
            value={`${status.memory_mb.toFixed(0)}MB`}
            onClick={onOpenSession}
            title="Session vitals"
          />
        )}
      </div>

      {/* CENTER — budget + indexing. */}
      <div className="flex items-center gap-2 min-w-0">
        {status && status.total_sessions > 0 && (
          <Chip
            label="SESSIONS"
            value={status.total_sessions}
            accent
            onClick={onOpenSession}
            title="Open session vitals"
          />
        )}

        {indexingActive && (
          <Chip
            label="INDEXING"
            value={indexingDetail}
            live
            onClick={onOpenIndexing}
            title="Open indexing detail"
          />
        )}
      </div>

      {/* RIGHT — palette + drawer + settings. */}
      <div className="flex items-center gap-2 shrink-0">
        <button
          onClick={onOpenLogs}
          title="Open daemon log drawer"
          aria-label="Open daemon log drawer"
          className={cn(
            "grid h-7 w-7 place-items-center cursor-pointer transition-none rounded-sm",
            "border border-border-soft bg-surface text-text-muted",
            "hover:text-text hover:border-border",
          )}
        >
          <BrutalDrawer className="h-4 w-4" />
        </button>

        <button
          onClick={onOpenPalette}
          title="Command palette (⌘K)"
          className={cn(
            "inline-flex items-center gap-2 cursor-pointer transition-none rounded-sm",
            "border border-border-soft bg-surface px-2 py-1",
            "text-text-muted hover:text-text hover:border-border",
          )}
        >
          <kbd
            className={cn(
              "border border-border-soft bg-surface-overlay px-1",
              "text-mono-mini font-mono text-text-dim rounded-sm",
            )}
          >
            ⌘K
          </kbd>
        </button>

        <button
          onClick={onOpenSettings}
          title="Settings"
          aria-label="Open settings"
          className={cn(
            "grid h-7 w-7 place-items-center cursor-pointer transition-none rounded-sm",
            "border border-border-soft bg-surface text-text-muted",
            "hover:text-text hover:border-border font-mono text-base leading-none",
          )}
        >
          ⚙
        </button>
      </div>
    </footer>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Bits

function Chip({
  label,
  value,
  onClick,
  title,
  accent,
  live,
}: {
  label: string;
  value: string | number;
  onClick?: () => void;
  title?: string;
  accent?: boolean;
  live?: boolean;
}): ReactNode {
  const Tag = onClick ? "button" : "div";
  return (
    <Tag
      onClick={onClick}
      title={title}
      className={cn(
        "inline-flex items-center gap-1.5 px-2 py-1 rounded-sm",
        "border border-border-soft text-text-muted",
        live ? "bg-accent-live text-accent-fg-on" : "bg-surface",
        accent && !live && "border-accent text-text",
        onClick && "cursor-pointer hover:text-text hover:border-border",
        "transition-none",
      )}
    >
      <span className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text-dim">
        {label}
      </span>
      <span className="font-mono text-xs font-semibold tabular-nums text-text">
        {value}
      </span>
    </Tag>
  );
}

function describeIndexing(corpora: CorpusInfo[]): string | null {
  const indexing = corpora.filter((c) => c.status.state === "indexing");
  if (indexing.length === 0) return null;
  if (indexing.length === 1) {
    const s = indexing[0].status;
    if (s.state === "indexing") {
      return `${s.files_done}/${s.files_total}`;
    }
  }
  return `${indexing.length} corpora`;
}

function basename(p: string): string {
  if (!p) return "";
  const idx = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return idx >= 0 ? p.slice(idx + 1) : p;
}
