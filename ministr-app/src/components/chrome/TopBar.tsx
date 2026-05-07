import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import { ProjectPicker } from "./ProjectPicker";
import { DaemonDot } from "../shell/DaemonDot";

interface Props {
  status: DaemonStatus | null;
  error: string | null;
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onAddProject: () => void;
  onOpenLogs?: () => void;
}

/**
 * Top chrome — wordmark on the left, project picker in the middle,
 * status dot on the right. Replaces the old WorkspaceShell banner +
 * StatusBar pair.
 */
export function TopBar({
  status,
  error,
  corpora,
  activeCorpusId,
  onSelectCorpus,
  onAddProject,
  onOpenLogs,
}: Props) {
  return (
    <header
      className="flex items-center gap-3 border-b-2 border-border bg-surface px-3 h-12 shrink-0"
      role="banner"
    >
      <div className="ministr-wordmark shrink-0 select-none">ministr</div>

      <div className="h-6 w-px bg-border-soft shrink-0" aria-hidden />

      <ProjectPicker
        corpora={corpora}
        activeId={activeCorpusId}
        onSelect={onSelectCorpus}
        onAddProject={onAddProject}
      />

      <div className="flex-1" />

      <DaemonDot status={status} error={error} onOpenLogs={onOpenLogs} />
    </header>
  );
}
