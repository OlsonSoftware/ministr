/**
 * FileTree — the Code surface's side bar.
 *
 * Single responsibility: list a corpus's indexed files (`list_corpus_files`)
 * as a collapsible folder tree and report selection. A filter box flattens to
 * matching files for large corpora. It owns no navigation — it just calls
 * `onSelect(path)`.
 *
 * Performance (aaa-explore-perf-filetree): the visible tree is flattened to a
 * uniform-height `Row[]` and VIRTUALIZED — only the on-screen window of rows is
 * mounted, so a 10k-file corpus stays at a few dozen DOM nodes and scrolls/
 * expands smoothly. The filter runs through `useDeferredValue`, so typing never
 * blocks on recomputing the list (React renders the new list at lower priority).
 */
import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, ChevronDown, File as FileIcon } from "@/components/ui/icons";
import { cn } from "../../lib/utils";
import type { FileInfo } from "../../lib/types";
import { commonDirPrefix, stripPrefix } from "./paths";

/** Per-nested-level indentation, in px. Kept tight so deep trees stay readable. */
const INDENT_STEP = 8;
const INDENT_BASE = 6;
function indentPx(depth: number): number {
  return depth * INDENT_STEP + INDENT_BASE;
}

/**
 * Fixed row height, in px. Every row (dir or file) is exactly this tall so the
 * windower can map scroll offset → row index by arithmetic alone (no per-row
 * measurement). Matches the previous `py-1` + `text-xs` visual height.
 */
const ROW_H = 24;
/** Extra rows rendered above/below the viewport to cover fast scrolls. */
const OVERSCAN = 8;

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

/**
 * Build the display tree, rooted at `prefix` (the corpus' common ancestor):
 * directory structure comes from each path *with the prefix stripped*, while
 * every file leaf keeps its FULL `path` as the key handed back to `onSelect` /
 * `read_file`. Directory node paths are prefix-relative so expand/collapse
 * state and {@link ancestorDirs} agree.
 */
function buildTree(files: FileInfo[], prefix: string): DirNode {
  const root = newDir("", "");
  for (const f of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
    const rel = stripPrefix(f.path, prefix);
    const segments = rel.split("/").filter(Boolean);
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

/** Prefix-relative directory paths on the way to `path` (for auto-expand). */
function ancestorDirs(path: string, prefix: string): Set<string> {
  const segments = stripPrefix(path, prefix).split("/").filter(Boolean);
  const out = new Set<string>();
  let acc = "";
  for (let i = 0; i < segments.length - 1; i++) {
    acc = acc ? `${acc}/${segments[i]}` : segments[i];
    out.add(acc);
  }
  return out;
}

/** A single flattened, renderable row — the unit the windower slices over. */
type Row =
  | { kind: "dir"; name: string; path: string; depth: number; open: boolean }
  | { kind: "file"; name: string; path: string; depth: number };

/**
 * Depth-first flatten of the *visible* tree into render order, honoring the
 * `expanded` set: directories first (matching the old per-level ordering), then
 * files; a collapsed directory contributes its own row but none of its subtree.
 */
function flattenTree(dir: DirNode, expanded: Set<string>, depth: number, out: Row[]): void {
  for (const child of dir.dirs.values()) {
    const open = expanded.has(child.path);
    out.push({ kind: "dir", name: child.name, path: child.path, depth, open });
    if (open) flattenTree(child, expanded, depth + 1, out);
  }
  for (const f of dir.files) {
    out.push({ kind: "file", name: f.name, path: f.path, depth });
  }
}

/** Flat, sorted file rows matching `q` — the filtered view (no depth, no cap). */
function filterRows(files: FileInfo[], q: string, prefix: string): Row[] {
  return files
    .filter((f) => f.path.toLowerCase().includes(q))
    .sort((a, b) => a.path.localeCompare(b.path))
    .map((f) => ({ kind: "file" as const, name: stripPrefix(f.path, prefix), path: f.path, depth: 0 }));
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

  // Root the display tree at the highest directory the corpus' files share,
  // so it doesn't start at the filesystem root.
  const prefix = useMemo(() => commonDirPrefix(files.map((f) => f.path)), [files]);

  // Auto-expand the path to the active file.
  useEffect(() => {
    if (!activePath) return;
    setExpanded((prev) => new Set([...prev, ...ancestorDirs(activePath, prefix)]));
  }, [activePath, prefix]);

  const tree = useMemo(() => buildTree(files, prefix), [files, prefix]);

  // Deferred filter: typing updates the input immediately, but the (potentially
  // large) row recompute + render happens at lower priority — the input stays
  // responsive on big corpora (react.dev's canonical large-list fix).
  const deferredFilter = useDeferredValue(filter);
  const query = deferredFilter.trim().toLowerCase();
  const isFiltering = query.length > 0;

  // The single flat row list the windower draws from — filtered files when a
  // query is active, otherwise the expanded folder tree.
  const rows = useMemo<Row[]>(() => {
    if (isFiltering) return filterRows(files, query, prefix);
    const out: Row[] = [];
    flattenTree(tree, expanded, 0, out);
    return out;
  }, [isFiltering, files, query, prefix, tree, expanded]);

  const toggle = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  // ── Fixed-height windowing ────────────────────────────────────────────────
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportH, setViewportH] = useState(600);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setViewportH(el.clientHeight);
    const ro = new ResizeObserver(() => setViewportH(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Jump back to the top whenever the list identity changes (new filter / new
  // corpus), so the window doesn't start scrolled past a now-shorter list.
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    setScrollTop(0);
  }, [query, corpusId]);

  const total = rows.length;
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(total, Math.ceil((scrollTop + viewportH) / ROW_H) + OVERSCAN);
  const visible = rows.slice(start, end);

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
      {prefix && !isFiltering && (
        <div
          className="shrink-0 truncate border-b border-border-soft px-2 py-1 font-mono text-mono-mini text-text-dim"
          title={prefix}
        >
          {prefix}/
        </div>
      )}
      <div
        ref={scrollRef}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        data-testid="file-tree-scroll"
        data-total={total}
        className="min-h-0 flex-1 overflow-y-auto py-1"
      >
        {loading ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">Loading_</p>
        ) : files.length === 0 ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">No files indexed.</p>
        ) : total === 0 ? (
          <p className="px-3 py-2 font-mono text-mono-mini text-text-dim">No matches.</p>
        ) : (
          // Full-height spacer establishes the scrollbar; rows are absolutely
          // positioned at index * ROW_H so only the visible window mounts.
          <div style={{ height: total * ROW_H, position: "relative" }}>
            {visible.map((row, i) => (
              <div
                key={row.path}
                data-filetree-row
                style={{
                  position: "absolute",
                  top: (start + i) * ROW_H,
                  left: 0,
                  right: 0,
                  height: ROW_H,
                }}
              >
                {row.kind === "dir" ? (
                  <DirRow name={row.name} path={row.path} depth={row.depth} open={row.open} onToggle={toggle} />
                ) : (
                  <FileRow
                    name={row.name}
                    path={row.path}
                    depth={row.depth}
                    active={row.path === activePath}
                    onSelect={onSelect}
                  />
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

const DirRow = memo(function DirRow({
  name,
  path,
  depth,
  open,
  onToggle,
}: {
  name: string;
  path: string;
  depth: number;
  open: boolean;
  onToggle: (path: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onToggle(path)}
      style={{ paddingLeft: `${indentPx(depth)}px` }}
      className="flex h-full w-full items-center gap-1 pr-2 text-left font-mono text-xs text-text-muted hover:bg-surface-overlay hover:text-text cursor-pointer transition-colors duration-150 ease-out"
    >
      {open ? (
        <ChevronDown className="h-3 w-3 shrink-0" strokeWidth={2} />
      ) : (
        <ChevronRight className="h-3 w-3 shrink-0" strokeWidth={2} />
      )}
      <span className="truncate">{name}</span>
    </button>
  );
});

const FileRow = memo(function FileRow({
  name,
  path,
  depth,
  active,
  onSelect,
}: {
  name: string;
  path: string;
  depth: number;
  active: boolean;
  onSelect: (path: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(path)}
      style={{ paddingLeft: `${indentPx(depth)}px` }}
      className={cn(
        "relative flex h-full w-full items-center gap-1.5 pr-2 text-left font-mono text-xs cursor-pointer transition-colors duration-150 ease-out",
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
});
