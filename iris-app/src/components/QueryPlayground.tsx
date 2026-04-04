import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, Code2, FileText, ChevronRight } from "lucide-react";
import { Card } from "./ui/card";
import type { DaemonStatus, SearchResult, SymbolInfo } from "../lib/types";

interface Props {
  status: DaemonStatus;
}

type Mode = "semantic" | "symbols";

export function QueryPlayground({ status }: Props) {
  const [mode, setMode] = useState<Mode>("semantic");
  const [query, setQuery] = useState("");
  const [corpusId, setCorpusId] = useState(status.corpora[0]?.id ?? "");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [symbols, setSymbols] = useState<SymbolInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);

  async function search() {
    if (!query.trim() || !corpusId) return;
    setLoading(true);
    try {
      if (mode === "semantic") {
        const r = await invoke<SearchResult[]>("search_corpus", {
          corpusId,
          query: query.trim(),
          topK: 20,
        });
        setResults(r);
        setSymbols([]);
      } else {
        const r = await invoke<SymbolInfo[]>("search_symbols", {
          corpusId,
          query: query.trim(),
          kind: null,
        });
        setSymbols(r);
        setResults([]);
      }
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider flex items-center gap-2">
        <Search className="h-4 w-4" /> Query Playground
      </h2>

      {/* Controls */}
      <div className="flex gap-2 items-center">
        <select
          value={corpusId}
          onChange={(e) => setCorpusId(e.target.value)}
          className="text-xs bg-surface-raised border border-border rounded px-2 py-1.5"
        >
          {status.corpora.map((c) => (
            <option key={c.id} value={c.id}>
              {c.id}
            </option>
          ))}
        </select>

        <div className="flex text-xs border border-border rounded overflow-hidden">
          <button
            onClick={() => setMode("semantic")}
            className={`px-2.5 py-1.5 transition-colors cursor-pointer ${mode === "semantic" ? "bg-accent/10 text-accent" : "text-text-dim hover:bg-surface-overlay"}`}
          >
            Semantic
          </button>
          <button
            onClick={() => setMode("symbols")}
            className={`px-2.5 py-1.5 transition-colors cursor-pointer ${mode === "symbols" ? "bg-accent/10 text-accent" : "text-text-dim hover:bg-surface-overlay"}`}
          >
            Symbols
          </button>
        </div>
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          search();
        }}
        className="flex gap-2"
      >
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={mode === "semantic" ? "Search sections..." : "Search symbols..."}
          className="flex-1 text-sm bg-surface-raised border border-border rounded px-3 py-1.5 placeholder:text-text-dim/50 focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <button
          type="submit"
          disabled={loading}
          className="px-3 py-1.5 text-sm bg-accent text-white rounded hover:bg-accent/90 disabled:opacity-50 cursor-pointer"
        >
          {loading ? "..." : "Search"}
        </button>
      </form>

      {/* Semantic results */}
      {results.length > 0 && (
        <div className="space-y-2">
          {results.map((r, i) => (
            <Card
              key={`${r.content_id}-${i}`}
              className="cursor-pointer hover:border-accent/30 transition-colors"
              onClick={() => setExpanded(expanded === r.content_id ? null : r.content_id)}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 min-w-0">
                  <FileText className="h-3.5 w-3.5 text-text-dim shrink-0" />
                  <span className="text-xs font-mono truncate">{r.content_id}</span>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <span className="text-xs text-text-dim">{r.resolution}</span>
                  <ScoreBadge score={r.score} />
                </div>
              </div>

              {r.heading_path.length > 0 && (
                <div className="flex items-center gap-1 mt-1 text-xs text-text-dim">
                  {r.heading_path.map((h, j) => (
                    <span key={j} className="flex items-center gap-1">
                      {j > 0 && <ChevronRight className="h-3 w-3" />}
                      {h}
                    </span>
                  ))}
                </div>
              )}

              {expanded === r.content_id && (
                <pre className="mt-2 text-xs bg-surface-overlay rounded p-2 overflow-x-auto whitespace-pre-wrap max-h-64 overflow-y-auto">
                  {r.text}
                </pre>
              )}
            </Card>
          ))}
        </div>
      )}

      {/* Symbol results */}
      {symbols.length > 0 && (
        <div className="space-y-2">
          {symbols.map((s) => (
            <Card key={s.id}>
              <div className="flex items-center gap-2">
                <Code2 className="h-3.5 w-3.5 text-accent shrink-0" />
                <span className="text-sm font-medium">{s.name}</span>
                <span className="text-xs text-text-dim px-1.5 py-0.5 rounded bg-surface-overlay">
                  {s.kind}
                </span>
                <span className="text-xs text-text-dim">{s.visibility}</span>
              </div>
              <div className="text-xs font-mono text-text-dim mt-1 truncate">
                {s.module_path} — {s.file_path}
              </div>
              {s.signature && (
                <pre className="text-xs bg-surface-overlay rounded p-1.5 mt-1 overflow-x-auto">
                  {s.signature}
                </pre>
              )}
              {s.doc_comment && (
                <p className="text-xs text-text-dim mt-1 line-clamp-2">{s.doc_comment}</p>
              )}
            </Card>
          ))}
        </div>
      )}

      {results.length === 0 && symbols.length === 0 && !loading && query && (
        <p className="text-sm text-text-dim text-center py-4">No results.</p>
      )}
    </div>
  );
}

function ScoreBadge({ score }: { score: number }) {
  const pct = Math.round(score * 100);
  const color =
    pct >= 80 ? "text-green-500" : pct >= 60 ? "text-accent" : pct >= 40 ? "text-warning" : "text-text-dim";
  return <span className={`text-xs font-mono ${color}`}>{pct}%</span>;
}
