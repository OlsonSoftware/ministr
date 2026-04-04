import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, RefreshCw } from "lucide-react";
import { Button } from "./ui/button";

export function LogViewer() {
  const [logs, setLogs] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const bottomRef = useRef<HTMLDivElement>(null);

  async function refresh() {
    const lines = await invoke<string[]>("read_logs", { lines: 500 });
    setLogs(lines);
    setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: "smooth" }), 50);
  }

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 5000);
    return () => clearInterval(id);
  }, []);

  const filtered = filter
    ? logs.filter((l) => l.toLowerCase().includes(filter.toLowerCase()))
    : logs;

  return (
    <div className="flex flex-col h-full gap-3">
      <div className="flex items-center justify-between shrink-0">
        <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">
          Logs
        </h2>
        <div className="flex items-center gap-2">
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
          <Button variant="ghost" size="sm" onClick={refresh}>
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto rounded-lg border border-border bg-surface-raised p-3 font-mono text-xs leading-relaxed">
        {filtered.length === 0 ? (
          <p className="text-text-dim">No log entries.</p>
        ) : (
          filtered.map((line, i) => (
            <div
              key={i}
              className={`py-0.5 ${
                line.includes("ERROR") || line.includes("error")
                  ? "text-danger"
                  : line.includes("WARN") || line.includes("warn")
                    ? "text-warning"
                    : "text-text-dim"
              }`}
            >
              {line}
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
