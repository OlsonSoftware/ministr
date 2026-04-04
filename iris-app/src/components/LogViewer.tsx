import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, RefreshCw, ArrowDown, Filter } from "lucide-react";
import { Button } from "./ui/button";

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

  return (
    <div className="flex flex-col h-full gap-3">
      <div className="flex items-center justify-between shrink-0">
        <div className="flex items-center gap-3">
          <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
            Logs
          </h2>
          {!noLogFile && (
            <span className="text-xs text-text-dim">
              {filtered.length} / {logs.length} lines
              {errorCount > 0 && (
                <span className="text-danger ml-2">{errorCount} errors</span>
              )}
              {warnCount > 0 && (
                <span className="text-warning ml-2">{warnCount} warnings</span>
              )}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1 rounded-md border border-border bg-surface-raised px-1">
            <Filter className="h-3 w-3 text-text-dim" />
            {(["all", "info", "warn", "error"] as const).map((l) => (
              <button
                key={l}
                onClick={() => setLevel(l)}
                className={`px-1.5 py-0.5 text-xs rounded cursor-pointer transition-colors ${
                  level === l
                    ? "bg-accent/20 text-accent"
                    : "text-text-dim hover:text-text"
                }`}
              >
                {l === "all" ? "All" : l.charAt(0).toUpperCase() + l.slice(1)}
              </button>
            ))}
          </div>
          <div className="relative">
            <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
            <input
              type="text"
              placeholder="Filter..."
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              className="h-7 pl-7 pr-2 text-xs rounded-md border border-border bg-surface-raised text-text placeholder:text-text-dim focus:outline-none focus:border-accent"
            />
          </div>
          <Button variant="ghost" size="sm" onClick={async () => {
            const lines = await invoke<string[]>("read_logs", { lines: 500 });
            prevLogLen.current = lines.length;
            setLogs(lines);
            if (autoScrollRef.current) {
              setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: "smooth" }), 50);
            }
          }} title="Refresh">
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant={autoScroll ? "default" : "ghost"}
            size="sm"
            onClick={() => {
              const next = !autoScroll;
              autoScrollRef.current = next;
              setAutoScroll(next);
              if (next) {
                bottomRef.current?.scrollIntoView({ behavior: "smooth" });
              }
            }}
            title={autoScroll ? "Auto-scroll on" : "Auto-scroll off"}
          >
            <ArrowDown className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      <div
        ref={containerRef}
        className="flex-1 overflow-y-auto rounded-lg border border-border bg-surface-raised p-3 font-mono text-xs leading-relaxed"
      >
        {noLogFile ? (
          <div className="text-text-dim space-y-2">
            <p>No log file found.</p>
            <p className="text-text-muted">
              Logs will appear here after the iris tray app is restarted.
              The log file is written to <code className="bg-surface-overlay px-1 py-0.5 rounded">~/.iris/iris.log</code>.
            </p>
          </div>
        ) : filtered.length === 0 ? (
          <p className="text-text-dim">No matching log entries.</p>
        ) : (
          filtered.map((line, i) => {
            const lineLevel = classifyLine(line);
            return (
              <div
                key={i}
                className={`py-0.5 ${
                  lineLevel === "error"
                    ? "text-danger"
                    : lineLevel === "warn"
                      ? "text-warning"
                      : "text-text-dim"
                }`}
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
