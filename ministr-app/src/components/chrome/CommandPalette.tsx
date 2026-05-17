/**
 * CommandPalette — a real command system with mode prefixes.
 *
 *   (none)  everything: navigation + actions + projects
 *   >       actions only (add project, open logs via nav…)
 *   @       switch project
 *   #       open a live session
 *   ?       jump to Ask
 *
 * Fuzzy-ish substring match on label + keywords; arrow keys navigate;
 * Enter/click activates; Esc closes. Spring-animated open/close.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Eye,
  FileText,
  FolderOpen,
  MessageSquare,
  Plus,
  RefreshCw,
  Settings as SettingsIcon,
  Sparkles,
  SunMoon,
  type LucideIcon,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import type { CorpusInfo } from "../../lib/types";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { popIn, scrim } from "../../lib/motion";
import { clampPct } from "../../lib/sessions";
import { cn } from "../../lib/utils";
import { useSessions } from "../../hooks/useSessions";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useDialog } from "../../hooks/useDialog";
import type { SurfaceId } from "./Sidebar";

interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  keywords: string;
  icon: LucideIcon;
  /** Which prefix-mode this item belongs to ("" = always). */
  mode: "" | ">" | "@" | "#" | "?";
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
  onOpenLogs: () => void;
  onReindexActive: () => void;
  onCycleTheme: () => void;
}

const MODE_HINT: Record<string, string> = {
  "": "Type to search · > actions · @ projects · # sessions · ? ask",
  ">": "Actions",
  "@": "Switch project",
  "#": "Open a live session",
  "?": "Ask the codebase",
};

export function CommandPalette({
  open,
  onClose,
  corpora,
  activeCorpusId,
  onNavigate,
  onSelectCorpus,
  onAddProject,
  onOpenLogs,
  onReindexActive,
  onCycleTheme,
}: Props) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { sessions } = useSessions();
  const { openEntity } = useEntityPanel();
  // Adds focus-restore (palette didn't return focus to the trigger) and
  // a Tab trap; the input keeps its own autofocus via `initialFocus`.
  const dialogRef = useDialog<HTMLDivElement>(open, onClose, {
    initialFocus: inputRef,
  });

  useEffect(() => {
    if (open) {
      setQuery("");
      setHighlight(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const mode = (
    ["@", "#", ">", "?"].includes(query[0]) ? query[0] : ""
  ) as CommandItem["mode"];
  const term = (mode ? query.slice(1) : query).trim().toLowerCase();

  const items = useMemo<CommandItem[]>(() => {
    const list: CommandItem[] = [
      {
        id: "nav:ask",
        label: "Go to Ask",
        hint: "Codebase Q&A",
        keywords: "ask question query",
        icon: MessageSquare,
        mode: "",
        run: () => onNavigate("ask"),
      },
      {
        id: "nav:projects",
        label: "Go to Projects",
        hint: "Manage indexed projects",
        keywords: "projects manage corpus",
        icon: FolderOpen,
        mode: "",
        run: () => onNavigate("projects"),
      },
      {
        id: "nav:sessions",
        label: "Go to Sessions",
        hint: "Live agent sessions",
        keywords: "sessions agents live monitor",
        icon: Activity,
        mode: "",
        run: () => onNavigate("sessions"),
      },
      {
        id: "nav:settings",
        label: "Go to Settings",
        hint: "Theme, AI assistants, developer",
        keywords: "settings preferences theme",
        icon: SettingsIcon,
        mode: "",
        run: () => onNavigate("settings"),
      },
      {
        id: "action:add-project",
        label: "Add project…",
        hint: "Open the system folder picker",
        keywords: "add new project import",
        icon: Plus,
        mode: ">",
        run: onAddProject,
      },
      {
        id: "action:reindex",
        label: "Re-index active project",
        hint: "Rebuild the active project's index",
        keywords: "reindex rebuild index refresh",
        icon: RefreshCw,
        mode: ">",
        run: onReindexActive,
      },
      {
        id: "action:open-logs",
        label: "Open logs",
        hint: "Reveal the daemon log file",
        keywords: "logs log file debug troubleshoot",
        icon: FileText,
        mode: ">",
        run: onOpenLogs,
      },
      {
        id: "action:cycle-theme",
        label: "Cycle theme",
        hint: "System → Dark → Light",
        keywords: "theme dark light system appearance",
        icon: SunMoon,
        mode: ">",
        run: onCycleTheme,
      },
      {
        id: "action:ask",
        label: "Ask the codebase…",
        hint: "Open the Ask surface",
        keywords: "ask question",
        icon: Sparkles,
        mode: "?",
        run: () => onNavigate("ask"),
      },
    ];
    for (const c of corpora) {
      if (c.id === activeCorpusId) continue;
      list.push({
        id: `corpus:${c.id}`,
        label: `Switch to ${corpusLabel(c)}`,
        hint: corpusRoot(c.paths),
        keywords: `${corpusLabel(c)} ${c.id}`,
        icon: FolderOpen,
        mode: "@",
        run: () => onSelectCorpus(c.id),
      });
    }
    for (const s of sessions) {
      list.push({
        id: `session:${s.session_id}`,
        label: s.session_id.slice(0, 12),
        hint: `${clampPct(s.utilization * 100)}% · turn ${s.current_turn}${
          s.client_name ? ` · ${s.client_name}` : ""
        }`,
        keywords: `${s.session_id} ${s.client_name ?? ""}`,
        icon: Activity,
        mode: "#",
        run: () =>
          openEntity({
            kind: "session",
            corpusId: s.corpus_id,
            sessionId: s.session_id,
            seed: s,
          }),
      });
    }
    const active = corpora.find((c) => c.id === activeCorpusId);
    if (active) {
      list.push({
        id: "action:inspect-active",
        label: `Inspect ${corpusLabel(active)}`,
        hint: "Open the project inspector",
        keywords: `inspect ${corpusLabel(active)} corpus detail project`,
        icon: Eye,
        mode: ">",
        run: () => openEntity({ kind: "corpus", corpus: active }),
      });
    }
    return list;
  }, [
    corpora,
    activeCorpusId,
    sessions,
    onNavigate,
    onSelectCorpus,
    onAddProject,
    onOpenLogs,
    onReindexActive,
    onCycleTheme,
    openEntity,
  ]);

  const filtered = useMemo(() => {
    return items.filter((i) => {
      if (mode && i.mode !== mode) return false;
      if (!term) return true;
      return (
        i.label.toLowerCase().includes(term) ||
        i.keywords.toLowerCase().includes(term)
      );
    });
  }, [items, mode, term]);

  useEffect(() => {
    if (highlight >= filtered.length) {
      setHighlight(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, highlight]);

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
    <AnimatePresence>
      {open && (
        <motion.div
          key="cmd-scrim"
          variants={scrim}
          initial="initial"
          animate="animate"
          exit="exit"
          className="fixed inset-0 z-[1300] flex items-start justify-center bg-black/50 backdrop-blur-[2px] px-6"
          style={{ paddingTop: "14vh" }}
          role="dialog"
          aria-modal="true"
          aria-label="Command palette"
          onClick={onClose}
        >
          <motion.div
            ref={dialogRef}
            variants={popIn}
            initial="initial"
            animate="animate"
            exit="exit"
            className="w-full max-w-xl overflow-hidden rounded-xl border border-border bg-surface shadow-lg origin-top"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="border-b border-border bg-surface-overlay">
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={onKeyDown}
                placeholder="Search commands…"
                className={cn(
                  "w-full bg-transparent px-4 py-3 outline-none",
                  "font-mono text-sm text-text placeholder:text-text-dim",
                )}
                autoComplete="off"
              />
            </div>

            {filtered.length === 0 ? (
              <p className="px-4 py-8 font-sans text-sm text-text-dim text-center">
                No matches.
              </p>
            ) : (
              <ul role="listbox" className="max-h-[52vh] overflow-y-auto p-1.5">
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
                          "flex w-full items-center gap-3 px-3 py-2 rounded-md text-left cursor-pointer",
                          "transition-colors duration-100",
                          active
                            ? "bg-accent text-[var(--color-accent-fg-on)]"
                            : "text-text-muted hover:bg-surface-overlay",
                        )}
                      >
                        <Icon
                          className="h-4 w-4 shrink-0"
                          strokeWidth={2}
                        />
                        <div className="min-w-0 flex-1">
                          <div className="font-sans text-sm font-medium truncate">
                            {item.label}
                          </div>
                          {item.hint && (
                            <div
                              className={cn(
                                "font-mono text-mono-mini truncate",
                                active
                                  ? "text-[var(--color-accent-fg-on)]/70"
                                  : "text-text-dim",
                              )}
                            >
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

            <footer className="flex items-center justify-between gap-3 border-t border-border bg-surface-overlay px-4 py-2 font-mono text-mono-mini text-text-dim">
              <span className="truncate">{MODE_HINT[mode]}</span>
              <span className="flex gap-3 shrink-0">
                <span>↑↓</span>
                <span>↵</span>
                <span>esc</span>
              </span>
            </footer>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
