import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Search,
  Home,
  Users,
  ScrollText,
  Settings as SettingsIcon,
  FolderKanban,
  Plus,
  RefreshCw,
  Palette,
  Monitor,
  Moon,
  Sun,
  TreePine,
  GitBranch,
  Cpu,
  Terminal,
  Keyboard,
} from "lucide-react";
import type { DaemonStatus } from "../lib/types";
import { cn } from "../lib/utils";
import { corpusLabel } from "../lib/corpus";

type Tab =
  | "overview"
  | "sessions"
  | "search"
  | "treemap"
  | "symbols"
  | "simulator"
  | "logs"
  | "settings"
  | "projects";

interface Cmd {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string[];
  group: "Navigate" | "Corpus" | "Actions" | "View";
  icon: React.ComponentType<{ className?: string }>;
  run: () => void | Promise<void>;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  status: DaemonStatus | null;
  onNavigate: (tab: Tab) => void;
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
        id: "nav:overview",
        label: "Go to overview",
        hint: "Live cache telemetry",
        shortcut: ["g", "o"],
        group: "Navigate",
        icon: Home,
        run: () => onNavigate("overview"),
      },
      {
        id: "nav:sessions",
        label: "Go to sessions",
        hint: "All active MCP sessions",
        shortcut: ["g", "s"],
        group: "Navigate",
        icon: Users,
        run: () => onNavigate("sessions"),
      },
      {
        id: "nav:search",
        label: "Go to search",
        hint: "Query playground",
        shortcut: ["g", "q"],
        group: "Navigate",
        icon: Search,
        run: () => onNavigate("search"),
      },
      {
        id: "nav:projects",
        label: "Go to projects",
        hint: "Manage all corpora",
        group: "Navigate",
        icon: FolderKanban,
        run: () => onNavigate("projects"),
      },
      {
        id: "nav:treemap",
        label: "Open corpus treemap",
        group: "View",
        icon: TreePine,
        run: () => onNavigate("treemap"),
      },
      {
        id: "nav:symbols",
        label: "Open symbol graph",
        group: "View",
        icon: GitBranch,
        run: () => onNavigate("symbols"),
      },
      {
        id: "nav:simulator",
        label: "Open context simulator",
        group: "View",
        icon: Cpu,
        run: () => onNavigate("simulator"),
      },
      {
        id: "nav:logs",
        label: "Open logs",
        shortcut: ["g", "l"],
        group: "Navigate",
        icon: ScrollText,
        run: () => onNavigate("logs"),
      },
      {
        id: "nav:settings",
        label: "Open settings",
        group: "Navigate",
        icon: SettingsIcon,
        run: () => onNavigate("settings"),
      },
      {
        id: "action:add",
        label: "Add a project…",
        hint: "Open the folder picker",
        group: "Actions",
        icon: Plus,
        run: onAddProject,
      },
      {
        id: "action:refresh",
        label: "Refresh daemon status",
        group: "Actions",
        icon: RefreshCw,
        run: onRefresh,
      },
      {
        id: "action:shortcuts",
        label: "Show keyboard shortcuts",
        shortcut: ["?"],
        group: "Actions",
        icon: Keyboard,
        run: onShowShortcuts,
      },
      {
        id: "theme:system",
        label: "Theme · system",
        group: "Actions",
        icon: Monitor,
        run: () => onThemeChange("system"),
      },
      {
        id: "theme:dark",
        label: "Theme · dark",
        group: "Actions",
        icon: Moon,
        run: () => onThemeChange("dark"),
      },
      {
        id: "theme:light",
        label: "Theme · light",
        group: "Actions",
        icon: Sun,
        run: () => onThemeChange("light"),
      },
      {
        id: "action:palette",
        label: "Copy daemon socket path",
        group: "Actions",
        icon: Terminal,
        run: async () => {
          await navigator.clipboard.writeText("~/.ministr/ministrd.sock");
        },
      },
    ];

    // Corpus-specific commands
    const corpusCmds: Cmd[] = (status?.corpora ?? []).flatMap((c) => {
      const name = corpusLabel(c);
      return [
        {
          id: `corpus:open:${c.id}`,
          label: `Open corpus · ${name}`,
          hint: c.paths[0],
          group: "Corpus",
          icon: FolderKanban,
          run: () => {
            onSelectCorpus(c.id);
            onNavigate("projects");
          },
        },
        {
          id: `corpus:reindex:${c.id}`,
          label: `Re-index corpus · ${name}`,
          group: "Corpus",
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
      [c.label, c.hint, c.group].filter(Boolean).join(" ").toLowerCase().includes(q),
    );
  }, [commands, query]);

  // Group by section in filtered order
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
      className="fixed inset-0 z-[1000] flex items-start justify-center bg-bg/70 backdrop-blur-sm pt-24 px-6 ministr-fade-in"
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      onClick={onClose}
    >
      <div
        className="w-full max-w-xl rounded-2xl border border-border/70 bg-surface/95 shadow-[var(--shadow-lg)] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 border-b border-border/60 px-4 py-3">
          <Search className="h-4 w-4 text-text-dim" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Type a command, corpus, or action…"
            spellCheck={false}
            autoComplete="off"
            className="flex-1 bg-transparent text-sm text-text placeholder:text-text-dim outline-none"
          />
          <kbd className="rounded border border-border/70 bg-surface-overlay px-1.5 py-0.5 text-[10px] font-mono text-text-dim">
            Esc
          </kbd>
        </div>

        <div className="max-h-[min(480px,60vh)] overflow-y-auto p-1.5">
          {filtered.length === 0 ? (
            <div className="px-3 py-10 text-center text-sm text-text-dim">
              No matches for “{query}”
            </div>
          ) : (
            grouped.map(([group, items]) => (
              <div key={group} className="mb-1">
                <div className="px-3 pt-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-text-dim">
                  {group}
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
                        "w-full flex items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm transition-colors cursor-pointer",
                        isActive
                          ? "bg-[var(--color-accent-soft)] text-text"
                          : "hover:bg-surface-overlay/60 text-text",
                      )}
                    >
                      <cmd.icon
                        className={cn(
                          "h-3.5 w-3.5 shrink-0",
                          isActive ? "text-accent" : "text-text-dim",
                        )}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="truncate">{cmd.label}</div>
                        {cmd.hint && (
                          <div className="text-[11px] font-mono text-text-dim truncate">
                            {cmd.hint}
                          </div>
                        )}
                      </div>
                      {cmd.shortcut && (
                        <div className="flex items-center gap-0.5 shrink-0">
                          {cmd.shortcut.map((k, i) => (
                            <kbd
                              key={i}
                              className="rounded border border-border/70 bg-surface-overlay px-1 py-0 text-[10px] font-mono text-text-muted"
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
            ))
          )}
        </div>

        <div className="border-t border-border/60 px-4 py-2 flex items-center gap-3 text-[10px] text-text-dim font-mono">
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border/70 bg-surface-overlay px-1 py-0">↑↓</kbd>
            move
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border/70 bg-surface-overlay px-1 py-0">↵</kbd>
            run
          </span>
          <span className="flex items-center gap-1 ml-auto">
            <Palette className="h-3 w-3" />
            ministr command palette
          </span>
        </div>
      </div>
    </div>
  );
}
