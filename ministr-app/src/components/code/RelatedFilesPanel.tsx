/**
 * RelatedFilesPanel — contextual "where does this file connect?" navigation.
 *
 * Single responsibility: given the file currently open in the viewer, surface
 * the other files it's related to *through the symbol graph* and let the user
 * jump to them. For each symbol the file defines we pull `symbol_references`
 * and bucket every edge by direction relative to this file:
 *   • incoming — another file references a symbol defined here (callers)
 *   • outgoing — a symbol here references something defined in another file
 * Files are deduped with a per-file edge count; clicking one opens it.
 *
 * Derived entirely from existing commands (no backend change). The per-symbol
 * fan-out is bounded so a large file stays responsive.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, ArrowLeftRight, FileSymlink } from "@/components/ui/icons";
import { cn } from "../../lib/utils";
import type { FileContent, SymbolRef } from "../../lib/types";

interface Props {
  corpusId: string;
  file: FileContent;
  onOpen: (path: string) => void;
}

type Direction = "incoming" | "outgoing";

interface RelatedFile {
  path: string;
  count: number;
  direction: Direction | "both";
}

/** Cap how many of the file's symbols we expand references for. */
const MAX_SYMBOLS = 40;

function mergeDirection(a: RelatedFile["direction"], b: Direction): RelatedFile["direction"] {
  return a === b ? a : "both";
}

export function RelatedFilesPanel({ corpusId, file, onOpen }: Props) {
  const [related, setRelated] = useState<RelatedFile[]>([]);
  const [loading, setLoading] = useState(false);

  const symbolIds = useMemo(
    () => file.symbol_spans.slice(0, MAX_SYMBOLS).map((s) => s.id),
    [file.symbol_spans],
  );

  useEffect(() => {
    if (!corpusId || symbolIds.length === 0) {
      setRelated([]);
      return;
    }
    let cancelled = false;
    setLoading(true);

    Promise.all(
      symbolIds.map((symbolId) =>
        invoke<SymbolRef[]>("symbol_references", { corpusId, symbolId }).catch(
          () => [] as SymbolRef[],
        ),
      ),
    )
      .then((lists) => {
        if (cancelled) return;
        const byFile = new Map<string, RelatedFile>();
        for (const refs of lists) {
          for (const r of refs) {
            // Determine the OTHER file + this edge's direction relative to us.
            let other: string | null = null;
            let direction: Direction | null = null;
            if (r.to_file === file.path && r.from_file !== file.path) {
              other = r.from_file;
              direction = "incoming";
            } else if (r.from_file === file.path && r.to_file !== file.path) {
              other = r.to_file;
              direction = "outgoing";
            }
            if (!other || !direction) continue;
            const existing = byFile.get(other);
            if (existing) {
              existing.count += 1;
              existing.direction = mergeDirection(existing.direction, direction);
            } else {
              byFile.set(other, { path: other, count: 1, direction });
            }
          }
        }
        const out = [...byFile.values()].sort(
          (a, b) => b.count - a.count || a.path.localeCompare(b.path),
        );
        setRelated(out);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [corpusId, file.path, symbolIds]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center gap-2 border-b border-border-soft bg-surface-overlay px-3 py-2">
        <FileSymlink className="h-3 w-3 text-accent" strokeWidth={2} />
        <span className="font-mono text-xs font-bold uppercase tracking-[0.08em] text-accent">
          Related files
        </span>
        <span className="ml-auto font-mono text-mono-mini tabular-nums text-text-dim">
          {related.length}
        </span>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && related.length === 0 ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">Loading_</p>
        ) : related.length === 0 ? (
          <p className="px-3 py-3 font-mono text-mono-mini text-text-dim">
            No related files in the symbol graph.
          </p>
        ) : (
          related.map((rf) => (
            <RelatedRow key={rf.path} rf={rf} onOpen={() => onOpen(rf.path)} />
          ))
        )}
      </div>
    </div>
  );
}

function RelatedRow({ rf, onOpen }: { rf: RelatedFile; onOpen: () => void }) {
  const segs = rf.path.split("/").filter(Boolean);
  const name = segs[segs.length - 1] ?? rf.path;
  const parent = segs.slice(0, -1).slice(-2).join("/");
  const dirLabel =
    rf.direction === "incoming"
      ? "used by"
      : rf.direction === "outgoing"
        ? "uses"
        : "both";

  return (
    <button
      type="button"
      onClick={onOpen}
      title={`${dirLabel} · ${rf.path}`}
      className="flex w-full items-center gap-2 border-b border-border-soft px-3 py-1.5 text-left font-mono text-mono-mini text-text-muted last:border-b-0 hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
    >
      {rf.direction === "both" ? (
        <ArrowLeftRight className="h-3 w-3 shrink-0 text-text-dim" strokeWidth={2} />
      ) : (
        <ArrowRight
          className={cn(
            "h-3 w-3 shrink-0",
            rf.direction === "outgoing" ? "text-accent" : "text-text-dim",
          )}
          strokeWidth={2}
        />
      )}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-text">{name}</span>
        {parent && <span className="truncate text-text-dim">{parent}</span>}
      </span>
      <span className="shrink-0 rounded-sm border border-border-soft px-1 text-mono-micro uppercase tracking-[0.08em] opacity-70">
        {dirLabel}
      </span>
      <span className="shrink-0 tabular-nums text-text-dim">{rf.count}</span>
    </button>
  );
}
