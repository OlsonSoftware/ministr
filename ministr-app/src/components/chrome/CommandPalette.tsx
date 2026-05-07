/**
 * CommandPalette — slim ⌘K palette scoped to the 3-surface IA.
 *
 * Replaces the old multi-section palette (search/symbols/bridges/sessions/
 * diagnostics) with the minimum useful set: nav between Ask / Projects /
 * Settings, switch the active project, and add a new project. Anything
 * deeper now lives behind Settings → Developer or the EntityPanel.
 *
 * Items are filtered by case-insensitive substring match on label +
 * keywords; first match is the default selection. Arrow keys navigate;
 * Enter / click activates; Esc closes.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  FolderOpen,
  MessageSquare,
  Plus,
  Settings as SettingsIcon,
  Sparkles,
  type LucideIcon,
} from "lucide-react";

import type { CorpusInfo } from "../../lib/types";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { cn } from "../../lib/utils";
import type { SurfaceId } from "./Sidebar";

interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  keywords: string;
  icon: LucideIcon;
  run: () => void;
}

interface Props {
  open: boolean;
  onClose: () => void;
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onNavigate: (surface: SurfaceId) => void;
  onSelectCorpus: (id: string) => void;
  onAddProject: () => void;
}

export function CommandPalette({
  open,
  onClose,
  corpora,
  activeCorpusId,
  onNavigate,
  onSelectCorpus,
  onAddProject,
}: Props) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset state every open so the palette never reopens with a stale
  // search or a highlight pointing at an item that no longer matches.
  useEffect(() => {
    if (open) {
      setQuery("");
      setHighlight(0);
      // Defer focus to next tick so the input exists in the DOM.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const items = useMemo<CommandItem[]>(() => {
    const list: CommandItem[] = [
      {
        id: "nav:ask",
        label: "Go to Ask",
        hint: "Codebase Q&A",
        keywords: "ask question query",
        icon: MessageSquare,
        run: () => onNavigate("ask"),
      },
      {
        id: "nav:projects",
        label: "Go to Projects",
        hint: "Manage indexed projects",
        keywords: "projects manage corpus",
        icon: FolderOpen,
        run: () => onNavigate("projects"),
      },
      {
        id: "nav:settings",
        label: "Go to Settings",
        hint: "Theme, AI assistants, developer",
        keywords: "settings preferences theme",
        icon: SettingsIcon,
        run: () => onNavigate("settings"),
      },
      {
        id: "action:add-project",
        label: "Add project…",
        hint: "Open the system folder picker",
        keywords: "add new project import",
        icon: Plus,
        run: onAddProject,
      },
    ];
    for (const c of corpora) {
      if (c.id === activeCorpusId) continue;
      list.push({
        id: `corpus:${c.id}`,
        label: `Switch to ${corpusLabel(c)}`,
        hint: corpusRoot(c.paths),
        keywords: `${corpusLabel(c)} ${c.id}`,
        icon: Sparkles,
        run: () => onSelectCorpus(c.id),
      });
    }
    return list;
  }, [corpora, activeCorpusId, onNavigate, onSelectCorpus, onAddProject]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (i) =>
        i.label.toLowerCase().includes(q) ||
        i.keywords.toLowerCase().includes(q),
    );
  }, [items, query]);

  // Keep highlight in range when the filtered set shrinks.
  useEffect(() => {
    if (highlight >= filtered.length) {
      setHighlight(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, highlight]);

  if (!open) return null;

  function activate(item: CommandItem) {
    item.run();
    onClose();
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(filtered.length - 1, h + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(0, h - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[highlight];
      if (item) activate(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }

  return (
    <div
      className="fixed inset-0 z-[1300] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "15vh" }}
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      onClick={onClose}
    >
      <div
        className="w-full max-w-xl border-2 border-border bg-surface shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b-2 border-border bg-surface-overlay">
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Type to search…"
            className={cn(
              "w-full bg-transparent px-3 py-2.5 outline-none",
              "font-mono text-sm text-text placeholder:text-text-dim",
            )}
            autoComplete="off"
          />
        </div>

        {filtered.length === 0 ? (
          <p className="px-3 py-6 font-serif italic text-sm text-text-dim text-center">
            No matches.
          </p>
        ) : (
          <ul role="listbox" className="max-h-[50vh] overflow-y-auto">
            {filtered.map((item, idx) => {
              const Icon = item.icon;
              const active = idx === highlight;
              return (
                <li key={item.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={active}
                    onMouseEnter={() => setHighlight(idx)}
                    onClick={() => activate(item)}
                    className={cn(
                      "flex w-full items-center gap-3 px-3 py-2 text-left cursor-pointer transition-none",
                      active
                        ? "bg-surface-overlay text-text"
                        : "text-text-muted hover:bg-surface-overlay",
                    )}
                  >
                    <Icon className="h-4 w-4 shrink-0" strokeWidth={2} />
                    <div className="min-w-0 flex-1">
                      <div className="font-mono text-sm font-semibold truncate">
                        {item.label}
                      </div>
                      {item.hint && (
                        <div className="font-mono text-mono-mini text-text-dim truncate">
                          {item.hint}
                        </div>
                      )}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        )}

        <footer className="flex items-center justify-end gap-3 border-t-2 border-border bg-surface-overlay px-3 py-1.5 font-mono text-mono-mini text-text-dim">
          <span>↑↓ navigate</span>
          <span>↵ select</span>
          <span>esc close</span>
        </footer>
      </div>
    </div>
  );
}
