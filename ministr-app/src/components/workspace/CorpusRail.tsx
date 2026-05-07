import { forwardRef, useEffect, useRef, type ReactNode } from "react";
import { Plus, Settings as SettingsIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { Badge } from "../ui/badge";
import { BrutalNew, BrutalProjects } from "../ui/brutal-icons";
import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import type { Investigation } from "../../lib/investigations";

interface CorpusRailProps {
  status: DaemonStatus | null;
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onAddProject: () => void;
  /** Open the full project-management drawer (rich list, re-index, remove, etc.). */
  onManageProjects: () => void;
  /** Investigations for the active corpus. */
  investigations: Investigation[];
  activeInvestigationId: string | null;
  onSelectInvestigation: (id: string) => void;
  onNewInvestigation: () => void;
  onCloseInvestigation: (id: string) => void;
}

/**
 * Left workspace pane — corpus picker + active investigations strip.
 *
 * Replaces the old `Rail` (5 tab icons) and absorbs the corpus-switching
 * role of the old `CorpusPill`. Picks a corpus, shows live indexing dots,
 * and surfaces in-flight investigations against the active corpus.
 *
 * Keeps the rail itself a focused picker — heavy project-management UI
 * (re-index, remove, paths) is one click away via the gear button.
 */
export function CorpusRail({
  status,
  activeCorpusId,
  onSelectCorpus,
  onAddProject,
  onManageProjects,
  investigations,
  activeInvestigationId,
  onSelectInvestigation,
  onNewInvestigation,
  onCloseInvestigation,
}: CorpusRailProps) {
  const corpora = status?.corpora ?? [];
  const activeCardRef = useRef<HTMLButtonElement>(null);

  // Keep the active corpus in view when it changes externally.
  useEffect(() => {
    activeCardRef.current?.scrollIntoView({ block: "nearest" });
  }, [activeCorpusId]);

  return (
    <nav
      aria-label="Corpus rail"
      className={cn(
        "flex flex-col h-full min-h-0 bg-surface",
        "border-r-2 border-border",
      )}
    >
      {/* Header — wordmark + add/manage actions. */}
      <header className="flex items-center justify-between gap-1 border-b-2 border-border px-3 py-2 shrink-0">
        <span className="ministr-wordmark">ministr</span>
        <div className="flex items-center gap-1">
          <button
            onClick={onAddProject}
            title="Add project"
            aria-label="Add project"
            className={cn(
              "grid h-7 w-7 place-items-center cursor-pointer transition-none rounded-sm",
              "border border-border-soft bg-surface text-text-muted",
              "hover:text-text hover:border-border",
            )}
          >
            <Plus className="h-3.5 w-3.5" strokeWidth={2.5} />
          </button>
          <button
            onClick={onManageProjects}
            title="Manage projects"
            aria-label="Manage projects"
            className={cn(
              "grid h-7 w-7 place-items-center cursor-pointer transition-none rounded-sm",
              "border border-border-soft bg-surface text-text-muted",
              "hover:text-text hover:border-border",
            )}
          >
            <SettingsIcon className="h-3.5 w-3.5" strokeWidth={2} />
          </button>
        </div>
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto">
        <SectionHeading>Corpora</SectionHeading>
        {corpora.length === 0 ? (
          <EmptyHint>
            No projects.
            <br />
            Click + to add one.
          </EmptyHint>
        ) : (
          <ul className="px-2 pb-2 space-y-1">
            {corpora.map((c) => {
              const active = c.id === activeCorpusId;
              return (
                <li key={c.id}>
                  <CorpusItem
                    ref={active ? activeCardRef : undefined}
                    corpus={c}
                    active={active}
                    onClick={() => onSelectCorpus(c.id)}
                  />
                </li>
              );
            })}
          </ul>
        )}

        {activeCorpusId && (
          <>
            <SectionHeading
              right={
                <button
                  onClick={onNewInvestigation}
                  title="New investigation"
                  aria-label="New investigation"
                  className={cn(
                    "grid h-5 w-5 place-items-center cursor-pointer transition-none rounded-sm",
                    "text-text-muted hover:text-text hover:bg-surface-overlay",
                  )}
                >
                  <BrutalNew className="h-3 w-3" />
                </button>
              }
            >
              Investigations
            </SectionHeading>
            {investigations.length === 0 ? (
              <EmptyHint>
                Pin a source or ask a question to start an investigation.
              </EmptyHint>
            ) : (
              <ul className="px-2 pb-2 space-y-1">
                {investigations.map((inv) => {
                  const isActive = inv.id === activeInvestigationId;
                  return (
                    <li key={inv.id}>
                      <InvestigationItem
                        title={inv.title}
                        pinCount={inv.pinnedSourceIds.length}
                        active={isActive}
                        onClick={() => onSelectInvestigation(inv.id)}
                        onClose={() => onCloseInvestigation(inv.id)}
                      />
                    </li>
                  );
                })}
              </ul>
            )}
          </>
        )}
      </div>
    </nav>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Bits

function SectionHeading({
  children,
  right,
}: {
  children: ReactNode;
  right?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2 px-3 pt-3 pb-1.5">
      <h3 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text-dim">
        {children}
      </h3>
      {right}
    </div>
  );
}

function EmptyHint({ children }: { children: ReactNode }) {
  return (
    <p className="px-3 pb-3 font-serif text-xs italic text-text-dim">
      {children}
    </p>
  );
}

const CorpusItem = forwardRef<
  HTMLButtonElement,
  {
    corpus: CorpusInfo;
    active: boolean;
    onClick: () => void;
  }
>(function CorpusItem({ corpus, active, onClick }, ref) {
  const indexing = corpus.status.state === "indexing";
  return (
    <button
      ref={ref}
      onClick={onClick}
      className={cn(
        "w-full text-left px-2 py-1.5 cursor-pointer transition-none rounded-sm",
        "border border-transparent",
        active
          ? "bg-surface-overlay border-accent"
          : "hover:bg-surface-overlay hover:border-border-soft",
      )}
      title={corpusRoot(corpus.paths)}
    >
      <div className="flex items-center justify-between gap-2 min-w-0">
        <div className="flex items-center gap-1.5 min-w-0 flex-1">
          <BrutalProjects
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              active ? "text-accent" : "text-text-dim",
            )}
          />
          <span className="font-mono text-xs font-semibold text-text truncate">
            {corpusLabel(corpus)}
          </span>
        </div>
        {indexing && (
          <span
            className="h-1.5 w-1.5 bg-warning ministr-blink shrink-0"
            aria-label="Indexing"
            title="Indexing"
          />
        )}
        {!indexing && corpus.status.state === "error" && (
          <Badge variant="danger" dot>
            ERR
          </Badge>
        )}
      </div>
      <p className="font-mono text-mono-mini text-text-dim truncate mt-0.5 pl-5">
        {corpus.sections_count.toLocaleString()} sec ·{" "}
        {(corpus.symbols_count ?? 0).toLocaleString()} sym
      </p>
    </button>
  );
});

function InvestigationItem({
  title,
  pinCount,
  active,
  onClick,
  onClose,
}: {
  title: string;
  pinCount: number;
  active: boolean;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center gap-1 px-2 py-1.5 cursor-pointer transition-none rounded-sm",
        "border border-transparent",
        active
          ? "bg-surface-overlay border-accent"
          : "hover:bg-surface-overlay hover:border-border-soft",
      )}
      onClick={onClick}
    >
      <span className="font-sans text-xs text-text truncate flex-1">
        {title}
      </span>
      {pinCount > 0 && (
        <span className="font-mono text-mono-mini font-semibold tabular-nums text-text-dim shrink-0">
          {pinCount}
        </span>
      )}
      <button
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        title="Close investigation"
        aria-label="Close investigation"
        className={cn(
          "grid h-4 w-4 place-items-center text-text-dim opacity-0 group-hover:opacity-100",
          "hover:text-danger cursor-pointer transition-none rounded-sm",
        )}
      >
        ×
      </button>
    </div>
  );
}
