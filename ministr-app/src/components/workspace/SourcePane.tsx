import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "../../lib/utils";
import { BrutalClose, BrutalPin } from "../ui/brutal-icons";
import { Badge } from "../ui/badge";

interface SectionDetail {
  section_id: string;
  heading_path: string[];
  text: string;
  summary?: string | null;
  claims_available: number;
}

interface SourcePaneProps {
  corpusId: string | null;
  pinnedSourceIds: string[];
  /** Unpin a source from the active investigation. */
  onUnpin: (sourceId: string) => void;
  /** Clear all pinned sources from the active investigation. */
  onClear: () => void;
}

/**
 * Right workspace pane — pinned source stack.
 *
 * Each pinned source becomes a card showing the heading path, file
 * location, and excerpt. Cards stack vertically and scroll independently
 * of the conversation pane. Pinning a source = locking it into the
 * investigation context so it persists across queries.
 *
 * The full EntityPanel drawer (with breadcrumb navigation, related
 * symbols, etc.) remains the way to drill deeper — clicking the
 * heading path opens it.
 */
export function SourcePane({
  corpusId,
  pinnedSourceIds,
  onUnpin,
  onClear,
}: SourcePaneProps) {
  return (
    <aside
      aria-label="Pinned source pane"
      className={cn(
        "flex flex-col h-full min-h-0 bg-surface",
        "border-l-2 border-border",
      )}
    >
      <header className="flex items-center justify-between gap-2 border-b-2 border-border px-3 py-2 shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <BrutalPin className="h-3.5 w-3.5 text-text-dim shrink-0" />
          <h3 className="font-mono text-mono-mini font-semibold uppercase tracking-[0.05em] text-text">
            Pinned
          </h3>
          {pinnedSourceIds.length > 0 && (
            <Badge variant="default" dot>
              {pinnedSourceIds.length}
            </Badge>
          )}
        </div>
        {pinnedSourceIds.length > 0 && (
          <button
            onClick={onClear}
            title="Clear all pinned sources"
            className={cn(
              "font-mono text-mono-mini font-semibold uppercase tracking-[0.05em]",
              "text-text-dim hover:text-danger cursor-pointer transition-none px-1",
            )}
          >
            Clear
          </button>
        )}
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto">
        {pinnedSourceIds.length === 0 ? (
          <EmptyState />
        ) : (
          <ul className="p-3 space-y-3">
            {pinnedSourceIds.map((id) => (
              <li key={id}>
                <PinnedCard
                  sourceId={id}
                  corpusId={corpusId}
                  onUnpin={() => onUnpin(id)}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
    </aside>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center h-full px-6 text-center gap-2">
      <BrutalPin className="h-8 w-8 text-text-dim" />
      <p className="font-serif text-sm italic text-text-dim max-w-[180px]">
        Pin a citation, symbol, or section to keep it in this investigation.
      </p>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Pinned card

function PinnedCard({
  sourceId,
  corpusId,
  onUnpin,
}: {
  sourceId: string;
  corpusId: string | null;
  onUnpin: () => void;
}) {
  const [detail, setDetail] = useState<SectionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    if (!corpusId) return;
    let cancelled = false;
    invoke<SectionDetail>("get_section_detail", {
      corpusId,
      sectionId: sourceId,
    })
      .then((d) => {
        if (!cancelled) setDetail(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId, sourceId]);

  const headingLabel =
    detail?.heading_path.join(" / ") ?? sourceId.split(/[/\\]/).pop() ?? sourceId;

  return (
    <article
      className={cn(
        "border-2 border-border bg-surface-pinned ministr-pin-in",
        "shadow-xs",
      )}
    >
      <header className="flex items-start justify-between gap-2 px-3 py-2 border-b-2 border-border">
        <button
          onClick={() => setCollapsed((c) => !c)}
          className={cn(
            "flex items-start gap-1 min-w-0 flex-1 text-left cursor-pointer transition-none",
          )}
          aria-expanded={!collapsed}
          aria-label={collapsed ? "Expand source" : "Collapse source"}
        >
          {collapsed ? (
            <ChevronRight
              className="h-3.5 w-3.5 text-text-dim shrink-0 mt-0.5"
              strokeWidth={2.5}
            />
          ) : (
            <ChevronDown
              className="h-3.5 w-3.5 text-text-dim shrink-0 mt-0.5"
              strokeWidth={2.5}
            />
          )}
          <span className="font-mono text-mono-mini font-semibold text-text break-words">
            {headingLabel}
          </span>
        </button>
        <button
          onClick={onUnpin}
          title="Unpin"
          aria-label="Unpin source"
          className={cn(
            "grid h-5 w-5 shrink-0 place-items-center cursor-pointer transition-none rounded-sm",
            "text-text-dim hover:text-danger",
          )}
        >
          <BrutalClose className="h-3 w-3" />
        </button>
      </header>

      {!collapsed && (
        <div className="px-3 py-2.5">
          {error ? (
            <p className="font-mono text-mono-mini text-danger">
              Failed to load: {error}
            </p>
          ) : !detail ? (
            <SkeletonLines />
          ) : (
            <pre
              className={cn(
                "font-mono text-xs leading-relaxed text-text-muted",
                "whitespace-pre-wrap break-words",
                "max-h-[280px] overflow-y-auto",
              )}
            >
              {detail.text}
            </pre>
          )}
        </div>
      )}
    </article>
  );
}

function SkeletonLines() {
  return (
    <div className="space-y-1.5" aria-label="Loading source">
      {[60, 90, 75, 80, 50].map((w, i) => (
        <div
          key={i}
          className="h-2 bg-surface-overlay motion-data"
          style={{ width: `${w}%` }}
        />
      ))}
    </div>
  );
}
