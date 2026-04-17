import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Search,
  RefreshCw,
  ArrowDown,
  AlertTriangle,
  AlertCircle,
  Info,
  ScrollText,
} from "lucide-react";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";

type LogLevel = "all" | "error" | "warn" | "info";

function classifyLine(line: string): "error" | "warn" | "info" {
  const upper = line.toUpperCase();
  if (upper.includes(" ERROR ") || upper.includes("ERROR:")) return "error";
  if (upper.includes(" WARN ") || upper.includes("WARN:")) return "warn";
  return "info";
}

export function LogViewer() {
  const [logs, setLogs] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [level, setLevel] = useState<LogLevel>("all");
  const [autoScroll, setAutoScroll] = useState(true);
  const autoScrollRef = useRef(true);
  const prevLogLen = useRef(0);
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      const lines = await invoke<string[]>("read_logs", { lines: 500 });
      if (cancelled) return;
      const changed = lines.length !== prevLogLen.current;
      prevLogLen.current = lines.length;
      setLogs(lines);
      // Only scroll when new content arrived AND auto-scroll is on.
      if (changed && autoScrollRef.current) {
        setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: "smooth" }), 50);
      }
    }
    poll();
    const id = setInterval(poll, 3000);
    return () => { cancelled = true; clearInterval(id); };
  }, []);

  const filtered = logs.filter((line) => {
    if (level !== "all") {
      const lineLevel = classifyLine(line);
      if (level === "error" && lineLevel !== "error") return false;
      if (level === "warn" && lineLevel !== "error" && lineLevel !== "warn") return false;
    }
    if (filter && !line.toLowerCase().includes(filter.toLowerCase())) return false;
    return true;
  });

  const errorCount = logs.filter((l) => classifyLine(l) === "error").length;
  const warnCount = logs.filter((l) => classifyLine(l) === "warn").length;
  const noLogFile = logs.length === 1 && logs[0].includes("No log file");

  const levels: { key: LogLevel; label: string; icon: typeof Info }[] = [
    { key: "all", label: "All", icon: ScrollText },
    { key: "info", label: "Info", icon: Info },
    { key: "warn", label: "Warn", icon: AlertTriangle },
    { key: "error", label: "Error", icon: AlertCircle },
  ];

  return (
    <div className="flex flex-col h-full gap-3 iris-fade-in">
      <header className="flex flex-wrap items-center justify-between gap-3 shrink-0">
        <div>
          <h2 className="text-base font-semibold text-text">Logs</h2>
          <p className="text-xs text-text-dim mt-0.5 flex items-center gap-2">
            {noLogFile ? (
              <span>No log file yet</span>
            ) : (
              <>
                <span className="font-mono tabular-nums">
                  {filtered.length} / {logs.length}
                </span>
                <span>lines</span>
                {errorCount > 0 && (
                  <Badge variant="danger" dot>
                    {errorCount} errors
                  </Badge>
                )}
                {warnCount > 0 && (
                  <Badge variant="warning" dot>
                    {warnCount} warnings
                  </Badge>
                )}
              </>
            )}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-0.5 rounded-lg border border-border/70 bg-surface-raised p-0.5">
            {levels.map(({ key, label, icon: Icon }) => (
              <button
                key={key}
                onClick={() => setLevel(key)}
                className={cn(
                  "inline-flex items-center gap-1 px-2 py-1 text-[11px] font-medium rounded-md transition-all duration-120 cursor-pointer",
                  level === key
                    ? "bg-[var(--color-accent-soft)] text-accent shadow-[inset_0_0_0_1px_var(--color-accent-ring)]"
                    : "text-text-muted hover:text-text hover:bg-surface-overlay/60",
                )}
              >
                <Icon className="h-3 w-3" />
                {label}
              </button>
            ))}
          </div>

          <div className="relative">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
            <input
              type="text"
              placeholder="Filter…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              className="h-8 w-44 pl-8 pr-2.5 text-xs rounded-lg border border-border/70 bg-surface-raised text-text placeholder:text-text-dim font-mono focus:outline-none focus:border-[var(--color-accent-ring)] focus:shadow-[0_0_0_3px_var(--color-accent-soft)]"
            />
          </div>

          <Button
            variant="outline"
            size="icon"
            onClick={async () => {
              const lines = await invoke<string[]>("read_logs", { lines: 500 });
              prevLogLen.current = lines.length;
              setLogs(lines);
              if (autoScrollRef.current) {
                setTimeout(
                  () => bottomRef.current?.scrollIntoView({ behavior: "smooth" }),
                  50,
                );
              }
            }}
            title="Refresh"
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant={autoScroll ? "default" : "outline"}
            size="icon"
            onClick={() => {
              const next = !autoScroll;
              autoScrollRef.current = next;
              setAutoScroll(next);
              if (next) bottomRef.current?.scrollIntoView({ behavior: "smooth" });
            }}
            title={autoScroll ? "Auto-scroll on" : "Auto-scroll off"}
          >
            <ArrowDown className="h-3.5 w-3.5" />
          </Button>
        </div>
      </header>

      <div
        ref={containerRef}
        className="flex-1 overflow-y-auto rounded-xl border border-border/70 bg-surface-sunken p-3 font-mono text-[11px] leading-relaxed shadow-[inset_0_1px_0_rgb(255_255_255/0.02)]"
      >
        {noLogFile ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-3 py-10">
            <div className="grid h-12 w-12 place-items-center rounded-xl bg-surface-overlay text-text-dim">
              <ScrollText className="h-5 w-5" />
            </div>
            <div className="space-y-1">
              <p className="text-sm font-medium text-text">No log file yet</p>
              <p className="max-w-md text-xs text-text-dim font-sans leading-relaxed">
                Logs will appear here once iris has written its first
                entries. The log file lives at{" "}
                <code className="font-mono text-text-muted">
                  ~/.iris/iris.log
                </code>
                .
              </p>
            </div>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex h-full items-center justify-center">
            <p className="text-text-dim text-sm">
              No matching log entries.
            </p>
          </div>
        ) : (
          filtered.map((line, i) => {
            const lineLevel = classifyLine(line);
            return (
              <div
                key={i}
                className={cn(
                  "py-0.5 whitespace-pre-wrap break-all border-l-2 pl-2 -ml-1",
                  lineLevel === "error" && "text-danger border-danger/60",
                  lineLevel === "warn" && "text-warning border-warning/60",
                  lineLevel === "info" && "text-text-muted border-transparent",
                )}
              >
                {line}
              </div>
            );
          })
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
