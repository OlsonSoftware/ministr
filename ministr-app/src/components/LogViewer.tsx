import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertCircle,
  AlertTriangle,
  ArrowDown,
  Info,
  Pause,
  Play,
  RefreshCw,
  ScrollText,
} from "lucide-react";
import { Button } from "./ui/button";
import { H1 } from "./ui/heading";
import { cn } from "../lib/utils";
import { useToast } from "./shell/ToastTray";

type LogLevel = "error" | "warn" | "info";

interface ParsedLine {
  raw: string;
  level: LogLevel;
  /** Leading timestamp + level chunk (e.g. `2026-04-30T14:32:18Z INFO`). */
  meta: string;
  /** Tail of the line — the message portion. */
  message: string;
}

const TIMESTAMP_RE =
  /^(\S+\s+(?:INFO|WARN|WARNING|ERROR|DEBUG|TRACE)[\w:.\-]*)\s+(.*)$/i;

const FILE_RE = /([\w.\\/_\-]+\.(?:rs|tsx?|jsx?|py|go|java|kt|swift|c|cpp|cs|rb|md|toml|json|yaml|yml))(?::(\d+))?/g;
const SESSION_RE = /\bsession[-_][a-f0-9]{6,}\b/g;
const CORPUS_RE = /\bcorpus[-_][a-f0-9]{6,}\b/g;

function classifyLine(line: string): LogLevel {
  const upper = line.toUpperCase();
  if (upper.includes(" ERROR ") || upper.includes("ERROR:")) return "error";
  if (upper.includes(" WARN ") || upper.includes("WARN:")) return "warn";
  return "info";
}

function parseLine(line: string): ParsedLine {
  const level = classifyLine(line);
  const m = line.match(TIMESTAMP_RE);
  if (m) return { raw: line, level, meta: m[1], message: m[2] };
  return { raw: line, level, meta: "", message: line };
}

const ALL_LEVELS: LogLevel[] = ["error", "warn", "info"];

export function LogViewer() {
  const [logs, setLogs] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [regex, setRegex] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [enabledLevels, setEnabledLevels] = useState<Set<LogLevel>>(
    new Set(ALL_LEVELS),
  );
  const [autoScroll, setAutoScroll] = useState(true);
  const [paused, setPaused] = useState(false);
  const [pausedQueue, setPausedQueue] = useState<string[]>([]);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [showJump, setShowJump] = useState(false);
  const [newLineCount, setNewLineCount] = useState(0);

  const autoScrollRef = useRef(true);
  const pausedRef = useRef(false);
  const prevLogLen = useRef(0);
  const userScrolledUp = useRef(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const { toast } = useToast();

  // Keep refs in sync.
  useEffect(() => {
    autoScrollRef.current = autoScroll;
  }, [autoScroll]);
  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  // Poll daemon log file.
  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const lines = await invoke<string[]>("read_logs", { lines: 500 });
        if (cancelled) return;
        if (pausedRef.current) {
          // Buffer new lines but don't render.
          const delta = lines.length - prevLogLen.current;
          if (delta > 0) {
            const fresh = lines.slice(-delta);
            setPausedQueue((q) => [...q, ...fresh]);
            prevLogLen.current = lines.length;
          }
          return;
        }
        const changed = lines.length !== prevLogLen.current;
        const delta = Math.max(0, lines.length - prevLogLen.current);
        prevLogLen.current = lines.length;
        setLogs(lines);
        if (changed) {
          if (autoScrollRef.current && !userScrolledUp.current) {
            setTimeout(
              () => bottomRef.current?.scrollIntoView({ behavior: "auto" }),
              50,
            );
          } else if (userScrolledUp.current && delta > 0) {
            setShowJump(true);
            setNewLineCount((n) => n + delta);
          }
        }
      } catch {
        /* ignore */
      }
    }
    poll();
    const id = setInterval(poll, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  // Detect user scroll-up to suppress auto-scroll.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    function onScroll() {
      const distance = el!.scrollHeight - el!.scrollTop - el!.clientHeight;
      const atBottom = distance < 24;
      userScrolledUp.current = !atBottom;
      if (atBottom) {
        setShowJump(false);
        setNewLineCount(0);
      }
    }
    el.addEventListener("scroll", onScroll);
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  function jumpToBottom() {
    bottomRef.current?.scrollIntoView({ behavior: "auto" });
    userScrolledUp.current = false;
    setShowJump(false);
    setNewLineCount(0);
  }

  function resumeFeed() {
    if (pausedQueue.length > 0) {
      setLogs((prev) => {
        const merged = [...prev, ...pausedQueue].slice(-500);
        prevLogLen.current = merged.length;
        return merged;
      });
      setPausedQueue([]);
      // Auto-scroll on resume only when user is at bottom.
      if (!userScrolledUp.current) {
        setTimeout(
          () => bottomRef.current?.scrollIntoView({ behavior: "auto" }),
          50,
        );
      }
    }
    setPaused(false);
  }

  // Build regex from filter input, validating once per change.
  const filterRegex = useMemo(() => {
    if (!regex || !filter) return null;
    try {
      return { re: new RegExp(filter, caseSensitive ? "" : "i"), invalid: false };
    } catch {
      return { re: null, invalid: true };
    }
  }, [filter, regex, caseSensitive]);

  function lineMatchesFilter(line: string): boolean {
    if (!filter) return true;
    if (filterRegex) {
      if (filterRegex.invalid || !filterRegex.re) return true;
      return filterRegex.re.test(line);
    }
    if (caseSensitive) return line.includes(filter);
    return line.toLowerCase().includes(filter.toLowerCase());
  }

  const parsed = useMemo<ParsedLine[]>(
    () => logs.map((l) => parseLine(l)),
    [logs],
  );

  const filtered = useMemo(() => {
    return parsed
      .map((p, idx) => ({ p, idx }))
      .filter(({ p }) => enabledLevels.has(p.level) && lineMatchesFilter(p.raw));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [parsed, enabledLevels, filter, regex, caseSensitive]);

  const errorCount = parsed.filter((p) => p.level === "error").length;
  const warnCount = parsed.filter((p) => p.level === "warn").length;
  const noLogFile = logs.length === 1 && logs[0]?.includes("No log file");

  function toggleLevel(level: LogLevel) {
    setEnabledLevels((prev) => {
      const next = new Set(prev);
      if (next.has(level)) next.delete(level);
      else next.add(level);
      return next;
    });
  }
  function invertLevels() {
    setEnabledLevels((prev) => {
      const next = new Set<LogLevel>();
      for (const l of ALL_LEVELS) if (!prev.has(l)) next.add(l);
      return next;
    });
  }

  function copyLine(line: string) {
    navigator.clipboard.writeText(line).catch(() => {});
    toast("LINE COPIED", { tone: "info" });
  }

  function toggleExpand(i: number) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  }

  return (
    <div className="flex flex-col h-full gap-3 min-h-0">
      {/* Header */}
      <header className="flex flex-wrap items-center justify-between gap-3 shrink-0">
        <div>
          <H1>Logs</H1>
          <p className="font-sans text-xs tracking-[0.08em] text-text-dim mt-1">
            Daemon tail
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Severity multi-select */}
          <div className="flex items-stretch gap-0">
            {(
              [
                { key: "info" as const, label: "Info", icon: Info },
                { key: "warn" as const, label: "Warn", icon: AlertTriangle },
                { key: "error" as const, label: "Error", icon: AlertCircle },
              ]
            ).map(({ key, label, icon: Icon }) => {
              const active = enabledLevels.has(key);
              return (
                <button
                  key={key}
                  onClick={() => toggleLevel(key)}
                  className={cn(
                    "inline-flex items-center gap-1 border border-border-soft px-2 py-1 text-sm font-sans font-medium transition-colors duration-150 ease-out cursor-pointer -ml-[1px] first:ml-0",
                    active
                      ? "border-accent bg-surface-overlay text-text z-10 relative"
                      : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
                  )}
                >
                  <Icon className="h-3 w-3" strokeWidth={2} />
                  {label}
                </button>
              );
            })}
            <button
              onClick={invertLevels}
              title="Invert severity selection"
              className="border border-border-soft px-2 py-1 text-sm font-sans font-medium transition-colors duration-150 ease-out cursor-pointer -ml-[1px] bg-surface text-text-muted hover:bg-surface-overlay hover:text-text"
            >
              Invert
            </button>
          </div>

          {/* Filter input + regex/case toggles */}
          <div className="flex items-stretch gap-0">
            <input
              type="text"
              placeholder={regex ? "regex" : "filter"}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              className={cn(
                "h-8 w-44 px-2 text-sm border bg-surface text-text placeholder:text-text-dim font-sans focus:outline-none focus:border-accent transition-colors duration-150 ease-out",
                filterRegex?.invalid ? "border-danger" : "border-border-soft",
              )}
            />
            <button
              onClick={() => setRegex((v) => !v)}
              title={regex ? "Regex on" : "Regex off"}
              className={cn(
                "border border-border-soft px-1.5 text-xs font-mono font-semibold uppercase tracking-[0.08em] transition-colors duration-150 ease-out cursor-pointer -ml-[1px]",
                regex
                  ? "border-accent bg-surface-overlay text-text z-10 relative"
                  : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
              )}
            >
              re
            </button>
            <button
              onClick={() => setCaseSensitive((v) => !v)}
              title={caseSensitive ? "Case-sensitive" : "Case-insensitive"}
              className={cn(
                "border border-border-soft px-1.5 text-xs font-mono font-semibold transition-colors duration-150 ease-out cursor-pointer -ml-[1px]",
                caseSensitive
                  ? "border-accent bg-surface-overlay text-text z-10 relative"
                  : "bg-surface text-text-muted hover:bg-surface-overlay hover:text-text",
              )}
            >
              Aa
            </button>
          </div>

          <Button
            variant="outline"
            size="icon"
            onClick={async () => {
              const lines = await invoke<string[]>("read_logs", { lines: 500 });
              prevLogLen.current = lines.length;
              setLogs(lines);
              setPausedQueue([]);
              if (autoScrollRef.current) {
                setTimeout(
                  () => bottomRef.current?.scrollIntoView({ behavior: "auto" }),
                  50,
                );
              }
            }}
            title="Refresh"
          >
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2.5} />
          </Button>

          <Button
            variant={autoScroll ? "default" : "outline"}
            size="icon"
            onClick={() => {
              const next = !autoScroll;
              setAutoScroll(next);
              if (next) jumpToBottom();
            }}
            title={autoScroll ? "Auto-scroll on" : "Auto-scroll off"}
          >
            <ArrowDown className="h-3.5 w-3.5" strokeWidth={2.5} />
          </Button>

          {/* Pause / resume */}
          <button
            onClick={() => (paused ? resumeFeed() : setPaused(true))}
            className={cn(
              "inline-flex items-center gap-1.5 border border-border px-2 py-1 font-mono text-xs font-bold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out",
              paused
                ? "bg-warning text-[var(--color-accent-fg-on)] shadow-sm"
                : "bg-surface text-text hover:bg-surface-overlay",
            )}
            title={paused ? "Resume feed" : "Pause feed"}
          >
            {paused ? (
              <Play className="h-3 w-3" strokeWidth={2.5} />
            ) : (
              <Pause className="h-3 w-3" strokeWidth={2.5} />
            )}
            {paused
              ? `RESUME${pausedQueue.length > 0 ? ` · +${pausedQueue.length}` : ""}`
              : "PAUSE"}
          </button>
        </div>
      </header>

      {/* Log body */}
      <div
        ref={containerRef}
        className="relative flex-1 overflow-y-auto border border-border-soft bg-surface-sunken font-mono text-mono-mini leading-relaxed"
      >
        {noLogFile ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-3 py-10">
            <div className="grid h-12 w-12 place-items-center border border-border-soft bg-surface-overlay text-text">
              <ScrollText className="h-5 w-5" strokeWidth={2.5} />
            </div>
            <div className="space-y-1">
              <p className="font-sans text-sm font-bold tracking-[0.08em] text-text">
                No log file yet
              </p>
              <p className="max-w-md text-xs text-text-dim font-sans leading-relaxed">
                Logs will appear here once ministr has written its first
                entries. The log file lives at{" "}
                <code className="font-mono text-text-muted">
                  ~/.ministr/ministr.log
                </code>
                .
              </p>
            </div>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex h-full items-center justify-center">
            <p className="font-sans text-xs tracking-[0.08em] text-text-dim">
              No matching lines
            </p>
          </div>
        ) : (
          filtered.map(({ p, idx }) => (
            <LogRow
              key={idx}
              parsed={p}
              expanded={expanded.has(idx)}
              onToggleExpand={() => toggleExpand(idx)}
              onCopy={() => copyLine(p.raw)}
            />
          ))
        )}
        <div ref={bottomRef} />

        {/* New-lines indicator */}
        {showJump && (
          <button
            onClick={jumpToBottom}
            className="sticky bottom-3 left-auto right-3 ml-auto block border border-border bg-accent text-[var(--color-accent-fg-on)] shadow-sm px-2 py-1 font-mono text-xs font-bold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out"
          >
            ↓ {newLineCount} NEW · CLICK TO JUMP
          </button>
        )}
      </div>

      {/* Footer */}
      <footer className="flex items-center justify-between gap-3 border-t border-border bg-surface-overlay px-3 py-1 shrink-0 font-mono text-xs uppercase tracking-[0.08em] text-text-dim">
        <span>
          TAIL · {filtered.length}/{logs.length} LINES
        </span>
        <span className="flex items-center gap-3">
          <span className="text-danger">{errorCount} ERRORS</span>
          <span className="text-warning">{warnCount} WARNINGS</span>
          {paused && (
            <span className="font-mono text-warning">
              PAUSED · +{pausedQueue.length}
            </span>
          )}
        </span>
      </footer>
    </div>
  );
}

// ─── LOG ROW ───────────────────────────────────────────────────────────────

function LogRow({
  parsed,
  expanded,
  onToggleExpand,
  onCopy,
}: {
  parsed: ParsedLine;
  expanded: boolean;
  onToggleExpand: () => void;
  onCopy: () => void;
}) {
  const stripeClass =
    parsed.level === "error"
      ? "border-l-danger text-danger"
      : parsed.level === "warn"
        ? "border-l-warning text-warning"
        : "border-l-transparent text-text-muted";

  return (
    <div
      onClick={onToggleExpand}
      className={cn(
        "group relative flex items-start gap-2 border-l-2 pl-2 pr-2 py-0.5 cursor-pointer transition-colors duration-150 ease-out",
        stripeClass,
        expanded
          ? "whitespace-pre-wrap break-all"
          : "whitespace-pre overflow-hidden",
        "hover:bg-surface-overlay",
      )}
    >
      {parsed.meta && (
        <>
          <span className="font-mono text-text-dim shrink-0">{parsed.meta}</span>
          <span className="font-mono text-text-dim shrink-0">|</span>
        </>
      )}
      <span
        className={cn(
          "min-w-0 flex-1",
          expanded ? "" : "truncate",
        )}
      >
        <DeepLinkedMessage text={parsed.message} />
      </span>

      {/* Hover actions: copy */}
      <button
        onClick={(e) => {
          e.stopPropagation();
          onCopy();
        }}
        title="Copy line"
        className="opacity-0 group-hover:opacity-100 shrink-0 border border-border bg-surface text-text px-1 py-0 font-mono text-mono-micro font-bold uppercase tracking-[0.08em] cursor-pointer transition-colors duration-150 ease-out hover:bg-surface-overlay hover:text-text"
      >
        COPY
      </button>
    </div>
  );
}

// ─── DEEP-LINK PARSING ────────────────────────────────────────────────────

interface Token {
  type: "text" | "file" | "session" | "corpus";
  value: string;
}

function tokenize(text: string): Token[] {
  // Find all matches by index across three regexes, then weave with text spans.
  const matches: { start: number; end: number; type: Token["type"]; value: string }[] = [];

  for (const m of text.matchAll(FILE_RE)) {
    if (m.index === undefined) continue;
    matches.push({
      start: m.index,
      end: m.index + m[0].length,
      type: "file",
      value: m[0],
    });
  }
  for (const m of text.matchAll(SESSION_RE)) {
    if (m.index === undefined) continue;
    matches.push({
      start: m.index,
      end: m.index + m[0].length,
      type: "session",
      value: m[0],
    });
  }
  for (const m of text.matchAll(CORPUS_RE)) {
    if (m.index === undefined) continue;
    matches.push({
      start: m.index,
      end: m.index + m[0].length,
      type: "corpus",
      value: m[0],
    });
  }

  // Sort and dedupe overlapping ranges (file paths beat the others).
  matches.sort((a, b) => a.start - b.start);
  const filtered: typeof matches = [];
  for (const m of matches) {
    const last = filtered[filtered.length - 1];
    if (!last || m.start >= last.end) filtered.push(m);
  }

  const tokens: Token[] = [];
  let cursor = 0;
  for (const m of filtered) {
    if (m.start > cursor) {
      tokens.push({ type: "text", value: text.slice(cursor, m.start) });
    }
    tokens.push({ type: m.type, value: m.value });
    cursor = m.end;
  }
  if (cursor < text.length) {
    tokens.push({ type: "text", value: text.slice(cursor) });
  }
  return tokens;
}

function DeepLinkedMessage({ text }: { text: string }) {
  const tokens = useMemo(() => tokenize(text), [text]);
  const { toast } = useToast();

  function handleClick(t: Token) {
    // Copy by default — useful even when navigation isn't applicable.
    navigator.clipboard.writeText(t.value).catch(() => {});
    if (t.type === "file") {
      // Hand off to App.tsx via the existing `navigate` window event.
      // The path is informational; we route to Symbols where the file's
      // symbols can be discovered. (Filtering by file_path requires knowing
      // which corpus the file belongs to; we fall back to navigating only.)
      window.dispatchEvent(
        new CustomEvent("ministr-navigate", { detail: "symbols" }),
      );
      toast("FILE COPIED · SYMBOLS", { detail: t.value, tone: "info" });
    } else if (t.type === "session") {
      window.dispatchEvent(
        new CustomEvent("ministr-navigate", { detail: "sessions" }),
      );
      toast("SESSION COPIED · SESSIONS", { detail: t.value, tone: "info" });
    } else {
      toast("COPIED", { detail: t.value, tone: "info" });
    }
  }

  return (
    <>
      {tokens.map((t, i) => {
        if (t.type === "text") return <span key={i}>{t.value}</span>;
        return (
          <span
            key={i}
            className="underline decoration-2 underline-offset-2 cursor-pointer hover:bg-surface-overlay hover:text-text"
            title={`${t.type}: ${t.value}`}
            onClick={(e) => {
              e.stopPropagation();
              handleClick(t);
            }}
          >
            {t.value}
          </span>
        );
      })}
    </>
  );
}
