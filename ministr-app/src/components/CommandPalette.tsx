import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Search,
  Sparkles,
  Users,
  ScrollText,
  Settings as SettingsIcon,
  FolderKanban,
  Plus,
  RefreshCw,
  Monitor,
  Moon,
  Sun,
  TreePine,
  GitBranch,
  Network,
  Terminal,
  Keyboard,
} from "lucide-react";
import type { DaemonStatus } from "../lib/types";
import type { ExploreMode } from "./ExploreView";
import { cn } from "../lib/utils";
import { corpusLabel } from "../lib/corpus";
import { shortcutKeys } from "../lib/shortcuts";

type Tab =
  | "ask"
  | "explore"
  | "projects"
  | "sessions"
  | "settings";

interface Cmd {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string[];
  group: "NAV" | "CORPUS" | "ACTIONS" | "THEME";
  icon: React.ComponentType<{ className?: string }>;
  run: () => void | Promise<void>;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  status: DaemonStatus | null;
  onNavigate: (tab: Tab) => void;
  /** Deep-link into the Explore tab on a specific mode (sections /
   *  symbols / bridges). Caller routes to tab=explore + sets the mode. */
  onNavigateExplore: (mode?: ExploreMode) => void;
  /** Open Settings and scroll to a Diagnostics zone. After Phase 4 of
   *  the consolidation pass, Logs and the Context Simulator live inside
   *  Settings rather than as separate routes; this is how the palette
   *  reaches them. */
  onOpenDiagnostics: (target: "logs" | "simulator") => void;
  onAddProject: () => void;
  onSelectCorpus: (id: string) => void;
  onShowShortcuts: () => void;
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onRefresh: () => void;
}

export function CommandPalette({
  open,
  onClose,
  status,
  onNavigate,
  onNavigateExplore,
  onOpenDiagnostics,
  onAddProject,
  onSelectCorpus,
  onShowShortcuts,
  onThemeChange,
  onRefresh,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const commands: Cmd[] = useMemo(() => {
    const base: Cmd[] = [
      {
        id: "nav:ask",
        label: "Go to ask",
        hint: "Ask anything about the codebase",
        shortcut: shortcutKeys("nav:ask"),
        group: "NAV",
        icon: Sparkles,
        run: () => onNavigate("ask"),
      },
      {
        id: "nav:explore",
        label: "Go to explore",
        hint: "Sections · symbols · bridges (last used)",
        shortcut: shortcutKeys("nav:explore"),
        group: "NAV",
        icon: Search,
        run: () => onNavigateExplore(),
      },
      {
        id: "nav:explore:symbols",
        label: "Explore: Symbols",
        hint: "Symbol graph + references",
        group: "NAV",
        icon: GitBranch,
        run: () => onNavigateExplore("symbols"),
      },
      {
        id: "nav:explore:bridges",
        label: "Explore: Bridges",
        hint: "Cross-language IPC/FFI map",
        group: "NAV",
        icon: Network,
        run: () => onNavigateExplore("bridges"),
      },
      {
        id: "nav:explore:sections",
        label: "Explore: Sections",
        hint: "Docs · code · prose",
        group: "NAV",
        icon: ScrollText,
        run: () => onNavigateExplore("sections"),
      },
      {
        id: "nav:projects",
        label: "Go to projects",
        hint: "Indexed corpora",
        shortcut: shortcutKeys("nav:projects"),
        group: "NAV",
        icon: FolderKanban,
        run: () => onNavigate("projects"),
      },
      {
        id: "nav:sessions",
        label: "Go to sessions",
        hint: "Live MCP agents",
        shortcut: shortcutKeys("nav:sessions"),
        group: "NAV",
        icon: Users,
        run: () => onNavigate("sessions"),
      },
      {
        id: "nav:logs",
        label: "Open daemon log",
        hint: "Diagnostics zone in Settings",
        shortcut: shortcutKeys("nav:logs"),
        group: "NAV",
        icon: ScrollText,
        run: () => onOpenDiagnostics("logs"),
      },
      {
        id: "nav:simulator",
        label: "Open context simulator",
        hint: "Diagnostics zone in Settings",
        group: "NAV",
        icon: Terminal,
        run: () => onOpenDiagnostics("simulator"),
      },
      {
        id: "nav:settings",
        label: "Go to settings",
        shortcut: shortcutKeys("nav:settings"),
        group: "NAV",
        icon: SettingsIcon,
        run: () => onNavigate("settings"),
      },
      {
        id: "action:add",
        label: "Add a project…",
        hint: "Open the folder picker",
        group: "ACTIONS",
        icon: Plus,
        run: onAddProject,
      },
      {
        id: "action:refresh",
        label: "Refresh daemon status",
        group: "ACTIONS",
        icon: RefreshCw,
        run: onRefresh,
      },
      {
        id: "action:shortcuts",
        label: "Show keyboard shortcuts",
        shortcut: shortcutKeys("toggle:shortcuts"),
        group: "ACTIONS",
        icon: Keyboard,
        run: onShowShortcuts,
      },
      {
        id: "action:socket",
        label: "Copy daemon socket path",
        group: "ACTIONS",
        icon: Terminal,
        run: async () => {
          await navigator.clipboard.writeText("~/.ministr/ministrd.sock");
        },
      },
      {
        id: "theme:system",
        label: "Theme · system",
        group: "THEME",
        icon: Monitor,
        run: () => onThemeChange("system"),
      },
      {
        id: "theme:dark",
        label: "Theme · dark",
        group: "THEME",
        icon: Moon,
        run: () => onThemeChange("dark"),
      },
      {
        id: "theme:light",
        label: "Theme · light",
        group: "THEME",
        icon: Sun,
        run: () => onThemeChange("light"),
      },
    ];

    const corpusCmds: Cmd[] = (status?.corpora ?? []).flatMap((c) => {
      const name = corpusLabel(c);
      return [
        {
          id: `corpus:open:${c.id}`,
          label: `Open corpus · ${name}`,
          hint: c.paths[0],
          group: "CORPUS",
          icon: FolderKanban,
          run: () => {
            onSelectCorpus(c.id);
            onNavigate("projects");
          },
        },
        {
          id: `corpus:reindex:${c.id}`,
          label: `Re-index corpus · ${name}`,
          group: "CORPUS",
          icon: RefreshCw,
          run: async () => {
            await invoke("trigger_reindex", { corpusId: c.id }).catch(() => {});
            onRefresh();
          },
        },
      ];
    });

    return [...base, ...corpusCmds];
  }, [
    status,
    onNavigate,
    onAddProject,
    onSelectCorpus,
    onShowShortcuts,
    onThemeChange,
    onRefresh,
  ]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((c) =>
      [c.label, c.hint, c.group]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(q),
    );
  }, [commands, query]);

  const grouped = useMemo(() => {
    const map = new Map<string, Cmd[]>();
    for (const c of filtered) {
      const list = map.get(c.group) ?? [];
      list.push(c);
      map.set(c.group, list);
    }
    return Array.from(map.entries());
  }, [filtered]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [open]);

  useEffect(() => {
    if (active >= filtered.length) setActive(0);
  }, [filtered.length, active]);

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const cmd = filtered[active];
      if (cmd) {
        cmd.run();
        onClose();
      }
    }
  }

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[1000] flex items-start justify-center bg-black/40 px-6"
      style={{ paddingTop: "10vh" }}
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      onClick={onClose}
    >
      <div
        className="w-full max-w-[720px] border border-border-soft bg-surface shadow-[var(--shadow-md)] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 border-b border-border-soft px-4 py-3 bg-surface-overlay">
          <span className="font-mono text-base font-bold text-accent">{">"}</span>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="type a command, corpus, or action"
            spellCheck={false}
            autoComplete="off"
            className="flex-1 bg-transparent text-base font-sans text-text placeholder:text-text-dim outline-none"
          />
          <kbd
            className="border border-border-soft bg-surface px-1.5 py-0 text-xs font-mono text-text-dim"
            style={{ borderRadius: "var(--radius-pill)" }}
          >
            Esc
          </kbd>
        </div>

        <div className="max-h-[min(480px,60vh)] overflow-y-auto">
          {filtered.length === 0 ? (
            <div className="px-3 py-10 text-center font-serif text-base italic text-text-dim">
              No matches.
            </div>
          ) : (
            grouped.map(([group, items], groupIdx) => {
              const groupLabel = /^[A-Z][A-Z\s\-—·]+$/.test(group)
                ? group.charAt(0) + group.slice(1).toLowerCase()
                : group;
              return (
                <div key={group}>
                  <div className="flex items-baseline gap-3 border-b border-border-soft bg-surface-overlay px-3 py-2">
                    <span className="font-serif text-sm font-normal text-text-dim tabular-nums shrink-0 w-5">
                      §{groupIdx + 1}
                    </span>
                    <h3 className="font-serif text-base font-bold text-text">
                      {groupLabel}
                    </h3>
                  </div>
                  {items.map((cmd) => {
                    const idx = filtered.indexOf(cmd);
                    const isActive = idx === active;
                    return (
                      <button
                        key={cmd.id}
                        onClick={() => {
                          cmd.run();
                          onClose();
                        }}
                        onMouseEnter={() => setActive(idx)}
                        className={cn(
                          "relative w-full flex items-center gap-2.5 border-b border-border-soft px-3 py-2 text-left transition-none cursor-pointer",
                          isActive
                            ? "bg-surface-overlay text-text"
                            : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                        )}
                      >
                        {isActive && (
                          <span className="absolute left-0 top-0 bottom-0 w-[3px] bg-accent" />
                        )}
                        <cmd.icon className="h-3.5 w-3.5 shrink-0" />
                        <div className="flex-1 min-w-0">
                          <div className="truncate font-sans text-sm font-medium text-text">
                            {cmd.label}
                          </div>
                          {cmd.hint && (
                            <div className="font-sans text-xs text-text-dim truncate">
                              {cmd.hint}
                            </div>
                          )}
                        </div>
                        {cmd.shortcut && (
                          <div className="flex items-center gap-0.5 shrink-0">
                            {cmd.shortcut.map((k, i) => (
                              <kbd
                                key={i}
                                className="border border-border-soft bg-surface px-1 py-0 text-xs font-mono text-text-dim"
                                style={{ borderRadius: "var(--radius-pill)" }}
                              >
                                {k}
                              </kbd>
                            ))}
                          </div>
                        )}
                      </button>
                    );
                  })}
                </div>
              );
            })
          )}
        </div>

        <div className="border-t border-border-soft bg-surface-overlay px-4 py-2 flex items-center gap-4 font-sans text-xs text-text-dim">
          <span><span className="font-mono">↑↓</span> Move</span>
          <span><span className="font-mono">↵</span> Run</span>
          <span className="ml-auto font-serif italic">ministr · code intelligence</span>
        </div>
      </div>
    </div>
  );
}
