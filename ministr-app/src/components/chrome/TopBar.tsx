import { Search } from "lucide-react";
import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import { useSessions } from "../../hooks/useSessions";
import { ProjectPicker } from "./ProjectPicker";
import { DaemonDot } from "../shell/DaemonDot";
import { NumberTicker } from "../ui/number-ticker";
import { cn } from "../../lib/utils";

interface Props {
  status: DaemonStatus | null;
  error: string | null;
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onAddProject: () => void;
  onOpenLogs?: () => void;
  onOpenPalette?: () => void;
}

function fmtUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

/**
 * Top chrome — wordmark, project picker, a live vitals cluster
 * (sessions / memory / uptime as spring tickers), command entry, and
 * the daemon status dot.
 */
export function TopBar({
  status,
  error,
  corpora,
  activeCorpusId,
  onSelectCorpus,
  onAddProject,
  onOpenLogs,
  onOpenPalette,
}: Props) {
  const { sessions } = useSessions();

  return (
    <header
      className="flex items-center gap-3 border-b border-border bg-surface px-3 h-12 shrink-0"
      role="banner"
    >
      <div className="ministr-wordmark shrink-0 select-none">ministr</div>

      <div className="h-5 w-px bg-border shrink-0" aria-hidden />

      <ProjectPicker
        corpora={corpora}
        activeId={activeCorpusId}
        onSelect={onSelectCorpus}
        onAddProject={onAddProject}
      />

      <div className="flex-1" />

      {status && (
        <div className="hidden md:flex items-center gap-4 font-mono text-mono-mini text-text-dim">
          <Vital label="sessions">
            <NumberTicker
              value={sessions.length}
              flashOnChange
              className="text-text"
            />
          </Vital>
          <Vital label="mem">
            <NumberTicker
              value={status.memory_mb}
              format={(n) => `${Math.round(n)}MB`}
              className="text-text"
            />
          </Vital>
          <Vital label="up">
            <span className="font-mono text-text tabular-nums">
              {fmtUptime(status.uptime_secs)}
            </span>
          </Vital>
        </div>
      )}

      {onOpenPalette && (
        <button
          type="button"
          onClick={onOpenPalette}
          title="Command palette · ⌘K"
          className={cn(
            "hidden sm:inline-flex items-center gap-2 h-8 pl-2.5 pr-2 rounded-md cursor-pointer",
            "border border-border bg-surface text-text-dim",
            "hover:bg-surface-overlay hover:text-text hover:border-border-hover",
            "transition-colors duration-150",
          )}
        >
          <Search className="h-3.5 w-3.5" strokeWidth={2} />
          <kbd className="font-mono text-mono-micro rounded border border-border bg-surface-overlay px-1 py-px">
            ⌘K
          </kbd>
        </button>
      )}

      <div className="h-5 w-px bg-border shrink-0" aria-hidden />

      <DaemonDot status={status} error={error} onOpenLogs={onOpenLogs} />
    </header>
  );
}

function Vital({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="tabular-nums font-semibold">{children}</span>
      <span className="uppercase tracking-[0.08em] text-[0.92em]">{label}</span>
    </span>
  );
}
