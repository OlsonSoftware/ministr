/**
 * FileTree — the Code surface's side bar.
 *
 * Single responsibility: list a corpus's indexed files (`list_corpus_files`)
 * as a collapsible folder tree and report selection. A filter box flattens to
 * matching files for large corpora. It owns no navigation — it just calls
 * `onSelect(path)`.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, ChevronDown, File as FileIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import type { FileInfo } from "../../lib/types";

interface Props {
  corpusId: string;
  activePath: string | null;
  onSelect: (path: string) => void;
}

interface DirNode {
  name: string;
  path: string;
  dirs: Map<string, DirNode>;
  files: { name: string; path: string }[];
}

function newDir(name: string, path: string): DirNode {
  return { name, path, dirs: new Map(), files: [] };
}

function buildTree(files: FileInfo[]): DirNode {
  const root = newDir("", "");
  for (const f of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
    const segments = f.path.split("/").filter(Boolean);
    if (segments.length === 0) continue;
    let dir = root;
    for (let i = 0; i < segments.length - 1; i++) {
      const seg = segments[i];
      const childPath = dir.path ? `${dir.path}/${seg}` : seg;
      let child = dir.dirs.get(seg);
      if (!child) {
        child = newDir(seg, childPath);
        dir.dirs.set(seg, child);
      }
      dir = child;
    }
    dir.files.push({ name: segments[segments.length - 1], path: f.path });
  }
  return root;
}

/** Directory paths on the way to `path` (so we can auto-expand to the active file). */
function ancestorDirs(path: string): Set<string> {
  const segments = path.split("/").filter(Boolean);
  const out = new Set<string>();
  let acc = "";
  for (let i = 0; i < segments.length - 1; i++) {
    acc = acc ? `${acc}/${segments[i]}` : segments[i];
    out.add(acc);
  }
  return out;
}

export function FileTree({ corpusId, activePath, onSelect }: Props) {
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!corpusId) return;
    let cancelled = false;
    setLoading(true);
    invoke<FileInfo[]>("list_corpus_files", { corpusId })
      .then((r) => {
        if (!cancelled) setFiles(r);
      })
      .catch(() => {
        if (!cancelled) setFiles([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [corpusId]);

  // Auto-expand the path to the active file.
  useEffect(() => {
    if (!activePath) return;
    setExpanded((prev) => new Set([...prev, ...ancestorDirs(activePath)]));
  }, [activePath]);

  const tree = useMemo(() => buildTree(files), [files]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return null;
    return files
      .filter((f) => f.path.toLowerCase().includes(q))
      .sort((a, b) => a.path.localeCompare(b.path))
      .slice(0, 300);
  }, [files, filter]);

  function toggle(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  return (
    <div className="flex h-full min-h-0 flex-col border-r border-border-soft bg-surface">
      <div className="shrink-0 border-b border-border-soft p-2">
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="filter files"
          className="h-8 w-full rounded-md border border-border-soft bg-surface-sunken px-2 text-xs font-sans text-text placeholder:text-text-dim focus:border-accent focus:outline-none transition-colors duration-150 ease-out"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {loading ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">Loading_</p>
        ) : files.length === 0 ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">No files indexed.</p>
        ) : filtered ? (
          filtered.length === 0 ? (
            <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">No matches.</p>
          ) : (
            filtered.map((f) => (
              <FileRow
                key={f.path}
                name={f.path}
                depth={0}
                active={f.path === activePath}
                onSelect={() => onSelect(f.path)}
              />
            ))
          )
        ) : (
          <DirChildren
            dir={tree}
            depth={0}
            expanded={expanded}
            activePath={activePath}
            onToggle={toggle}
            onSelect={onSelect}
          />
        )}
      </div>
    </div>
  );
}

function DirChildren({
  dir,
  depth,
  expanded,
  activePath,
  onToggle,
  onSelect,
}: {
  dir: DirNode;
  depth: number;
  expanded: Set<string>;
  activePath: string | null;
  onToggle: (path: string) => void;
  onSelect: (path: string) => void;
}) {
  return (
    <>
      {[...dir.dirs.values()].map((child) => {
        const isOpen = expanded.has(child.path);
        return (
          <div key={child.path}>
            <button
              type="button"
              onClick={() => onToggle(child.path)}
              style={{ paddingLeft: `${depth * 12 + 8}px` }}
              className="flex w-full items-center gap-1 py-1 pr-2 text-left font-mono text-xs text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
            >
              {isOpen ? (
                <ChevronDown className="h-3 w-3 shrink-0" strokeWidth={2} />
              ) : (
                <ChevronRight className="h-3 w-3 shrink-0" strokeWidth={2} />
              )}
              <span className="truncate">{child.name}</span>
            </button>
            {isOpen && (
              <DirChildren
                dir={child}
                depth={depth + 1}
                expanded={expanded}
                activePath={activePath}
                onToggle={onToggle}
                onSelect={onSelect}
              />
            )}
          </div>
        );
      })}
      {dir.files.map((f) => (
        <FileRow
          key={f.path}
          name={f.name}
          depth={depth}
          active={f.path === activePath}
          onSelect={() => onSelect(f.path)}
        />
      ))}
    </>
  );
}

function FileRow({
  name,
  depth,
  active,
  onSelect,
}: {
  name: string;
  depth: number;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      style={{ paddingLeft: `${depth * 12 + 8}px` }}
      className={cn(
        "relative flex w-full items-center gap-1.5 py-1 pr-2 text-left font-mono text-xs cursor-pointer transition-colors duration-150 ease-out",
        active
          ? "bg-surface-overlay text-text"
          : "text-text-muted hover:bg-surface-overlay hover:text-text",
      )}
    >
      {active && <span className="absolute left-0 top-0 bottom-0 w-[2px] bg-accent" />}
      <FileIcon className="h-3 w-3 shrink-0 text-text-dim" strokeWidth={2} />
      <span className="truncate">{name}</span>
    </button>
  );
}
