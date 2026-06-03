/**
 * CommandPalette — the workspace's PRIMARY nav (aaa-chrome).
 *
 * Raycast model: go to any facet, project, or action by typing. Results are
 * grouped (Go to · Projects · Sessions · Actions) and speak the integrated
 * IA's vocabulary — the four facets (Ask/Explore/Activity/Tend) plus the
 * Fleet collection and the global Account area — not the retired 6-surface
 * rail. Mode prefixes scope the search:
 *
 *   (none)  everything, grouped
 *   >       actions only
 *   @       switch project (instant keyboard project switch)
 *   #       open a live session
 *   ?       ask the codebase
 *
 * Fuzzy-ish substring match on label + keywords; arrow keys navigate across
 * groups; Enter/click activates; Esc closes. Spring-animated open/close.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  CircleUser,
  Compass,
  Eye,
  FileText,
  FolderOpen,
  Layers,
  MessageSquare,
  Plus,
  RefreshCw,
  Sprout,
  Sparkles,
  SunMoon,
  type LucideIcon,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import type { CorpusInfo } from "../../lib/types";
import { corpusLabel, corpusRoot } from "../../lib/corpus";
import { popIn, scrim } from "../../lib/motion";
import { clampPct } from "../../lib/sessions";
import { overlayScrim } from "../../lib/ui-tokens";
import { cn } from "../../lib/utils";
import { useSessions } from "../../hooks/useSessions";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import { useDialog } from "../../hooks/useDialog";
import type { SurfaceId } from "./Sidebar";

type CommandCategory = "Go to" | "Projects" | "Sessions" | "Actions";

interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  keywords: string;
  icon: LucideIcon;
  /** Which prefix-mode this item belongs to ("" = always). */
  mode: "" | ">" | "@" | "#" | "?";
  /** Section the item is grouped under in the results list. */
  category: CommandCategory;
  run: () => void;
}

/** Render order for the grouped results. */
const CATEGORY_ORDER: CommandCategory[] = [
  "Go to",
  "Projects",
  "Sessions",
  "Actions",
];

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
  "": "Go anywhere · > actions · @ projects · # sessions · ? ask",
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
      // ── Facets — the verbs applied to the spine project. ───────────────
      {
        id: "nav:ask",
        label: "Ask",
        hint: "Converse with the project — cited answers",
        keywords: "ask question query facet conversation",
        icon: MessageSquare,
        mode: "",
        category: "Go to",
        run: () => onNavigate("ask"),
      },
      {
        id: "nav:explore",
        label: "Explore",
        hint: "Browse the index — symbols, bridges, source",
        keywords: "explore index symbols bridges code browser facet",
        icon: Compass,
        mode: "",
        category: "Go to",
        run: () => onNavigate("explore"),
      },
      {
        id: "nav:activity",
        label: "Activity",
        hint: "Live agents, indexing, recent deliveries",
        keywords: "activity sessions agents live monitor board facet",
        icon: Activity,
        mode: "",
        category: "Go to",
        run: () => onNavigate("sessions"),
      },
      {
        id: "nav:tend",
        label: "Tend",
        hint: "Care for the project — health, config, reindex, sharing",
        keywords: "tend care health config model reindex paths sharing facet settings",
        icon: Sprout,
        mode: "",
        category: "Go to",
        run: () => onNavigate("settings"),
      },
      // ── Cross-cutting areas. ───────────────────────────────────────────
      {
        id: "nav:fleet",
        label: "Fleet",
        hint: "All projects — the collection view",
        keywords: "fleet projects collection all corpora overview",
        icon: Layers,
        mode: "",
        category: "Go to",
        run: () => onNavigate("projects"),
      },
      {
        id: "nav:account",
        label: "Account",
        hint: "Global settings, cloud, system",
        keywords: "account settings global theme cloud server logs about billing",
        icon: CircleUser,
        mode: "",
        category: "Go to",
        run: () => onNavigate("cloud"),
      },
      // ── Actions. ───────────────────────────────────────────────────────
      {
        id: "action:add-project",
        label: "Add project…",
        hint: "Open the system folder picker",
        keywords: "add new project import",
        icon: Plus,
        mode: ">",
        category: "Actions",
        run: onAddProject,
      },
      {
        id: "action:reindex",
        label: "Re-index active project",
        hint: "Rebuild the active project's index",
        keywords: "reindex rebuild index refresh",
        icon: RefreshCw,
        mode: ">",
        category: "Actions",
        run: onReindexActive,
      },
      {
        id: "action:open-logs",
        label: "Open logs",
        hint: "Reveal the daemon log file",
        keywords: "logs log file debug troubleshoot",
        icon: FileText,
        mode: ">",
        category: "Actions",
        run: onOpenLogs,
      },
      {
        id: "action:cycle-theme",
        label: "Cycle theme",
        hint: "System → Dark → Light",
        keywords: "theme dark light system appearance",
        icon: SunMoon,
        mode: ">",
        category: "Actions",
        run: onCycleTheme,
      },
      {
        id: "action:ask",
        label: "Ask the codebase…",
        hint: "Open the Ask facet",
        keywords: "ask question",
        icon: Sparkles,
        mode: "?",
        category: "Actions",
        run: () => onNavigate("ask"),
      },
    ];
    for (const c of corpora) {
      if (c.id === activeCorpusId) continue;
      list.push({
        id: `corpus:${c.id}`,
        label: `Switch to ${corpusLabel(c)}`,
        hint: corpusRoot(c.paths),
        keywords: `${corpusLabel(c)} ${c.id} switch project spine`,
        icon: FolderOpen,
        mode: "@",
        category: "Projects",
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
        category: "Sessions",
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
        category: "Actions",
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
    const matched = items.filter((i) => {
      if (mode && i.mode !== mode) return false;
      if (!term) return true;
      return (
        i.label.toLowerCase().includes(term) ||
        i.keywords.toLowerCase().includes(term)
      );
    });
    // Order by section so the flat keyboard index matches the grouped
    // visual order (Array.prototype.sort is stable → intra-group order kept).
    return matched.sort(
      (a, b) =>
        CATEGORY_ORDER.indexOf(a.category) - CATEGORY_ORDER.indexOf(b.category),
    );
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
          className={cn(overlayScrim, "z-[1300] flex items-start justify-center px-6")}
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
                  // Raycast-style section header whenever the category changes
                  // (skipped in a single-mode view where it's redundant).
                  const showHeader =
                    !mode && (idx === 0 || filtered[idx - 1].category !== item.category);
                  return (
                    <li key={item.id}>
                      {showHeader && (
                        <div
                          className="px-3 pt-2 pb-1 font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim select-none"
                          aria-hidden
                        >
                          {item.category}
                        </div>
                      )}
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
