import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Search,
  Code2,
  FileText,
  ChevronRight,
  Sparkles,
  Hash,
  Loader2,
} from "lucide-react";
import { Card } from "./ui/card";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { CorpusSelect } from "./ui/corpus-select";
import { cn } from "../lib/utils";
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

  const hasResults = results.length > 0 || symbols.length > 0;

  return (
    <div className="space-y-4 ministr-fade-in max-w-3xl">
      <header>
        <h2 className="text-base font-semibold text-text">Search</h2>
        <p className="text-xs text-text-dim mt-0.5">
          Query the daemon directly — same code path that powers{" "}
          <span className="font-mono">ministr_survey</span> and{" "}
          <span className="font-mono">ministr_symbols</span>.
        </p>
      </header>

      <Card className="p-4 space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <CorpusSelect
            value={corpusId}
            onChange={setCorpusId}
            corpora={status.corpora}
            ariaLabel="Search corpus"
          />

          <div className="flex items-center gap-0.5 rounded-lg border border-border/70 bg-surface-raised p-0.5">
            {(
              [
                { key: "semantic" as const, label: "Semantic", icon: Sparkles },
                { key: "symbols" as const, label: "Symbols", icon: Hash },
              ]
            ).map(({ key, label, icon: Icon }) => (
              <button
                key={key}
                onClick={() => setMode(key)}
                className={cn(
                  "inline-flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium rounded-md transition-all duration-120 cursor-pointer",
                  mode === key
                    ? "bg-[var(--color-accent-soft)] text-accent shadow-[inset_0_0_0_1px_var(--color-accent-ring)]"
                    : "text-text-muted hover:text-text hover:bg-surface-overlay/60",
                )}
              >
                <Icon className="h-3 w-3" />
                {label}
              </button>
            ))}
          </div>
        </div>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            search();
          }}
          className="flex gap-2"
        >
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={
                mode === "semantic"
                  ? "Search sections by meaning…"
                  : "Search symbols by name…"
              }
              className={cn(
                "h-9 w-full rounded-md border border-border/70 bg-surface-raised pl-9 pr-3 text-sm",
                "text-text placeholder:text-text-dim font-mono",
                "focus:outline-none focus:border-[var(--color-accent-ring)]",
                "focus:shadow-[0_0_0_3px_var(--color-accent-soft)]",
              )}
            />
          </div>
          <Button type="submit" size="lg" disabled={loading || !query.trim()}>
            {loading && <Loader2 className="h-3.5 w-3.5 ministr-spin" />}
            {loading ? "Searching…" : "Search"}
          </Button>
        </form>
      </Card>

      {results.length > 0 && (
        <div className="space-y-2">
          <p className="text-xs text-text-dim">
            <span className="font-mono tabular-nums">{results.length}</span>{" "}
            semantic matches
          </p>
          {results.map((r, i) => (
            <Card
              key={`${r.content_id}-${i}`}
              hover="lift"
              className="cursor-pointer space-y-1.5"
              onClick={() =>
                setExpanded(expanded === r.content_id ? null : r.content_id)
              }
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 min-w-0">
                  <FileText className="h-3.5 w-3.5 text-text-dim shrink-0" />
                  <span className="text-xs font-mono truncate text-text">
                    {r.content_id}
                  </span>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <Badge variant="muted">{r.resolution}</Badge>
                  <ScoreBadge score={r.score} />
                </div>
              </div>

              {r.heading_path.length > 0 && (
                <div className="flex items-center flex-wrap gap-1 text-[11px] text-text-dim">
                  {r.heading_path.map((h, j) => (
                    <span key={j} className="flex items-center gap-1">
                      {j > 0 && <ChevronRight className="h-3 w-3" />}
                      {h}
                    </span>
                  ))}
                </div>
              )}

              {expanded === r.content_id && (
                <pre className="mt-2 text-[11px] leading-relaxed bg-surface-sunken border border-border/60 rounded-md p-3 overflow-x-auto whitespace-pre-wrap max-h-72 overflow-y-auto font-mono text-text-muted">
                  {r.text}
                </pre>
              )}
            </Card>
          ))}
        </div>
      )}

      {symbols.length > 0 && (
        <div className="space-y-2">
          <p className="text-xs text-text-dim">
            <span className="font-mono tabular-nums">{symbols.length}</span>{" "}
            matching symbols
          </p>
          {symbols.map((s) => (
            <Card key={s.id} hover="lift" className="space-y-1.5">
              <div className="flex items-center gap-2 flex-wrap">
                <Code2 className="h-3.5 w-3.5 text-accent shrink-0" />
                <span className="text-sm font-semibold font-mono text-text">
                  {s.name}
                </span>
                <Badge variant="default">{s.kind}</Badge>
                <Badge variant="muted">{s.visibility}</Badge>
              </div>
              <div className="text-[11px] font-mono text-text-dim truncate">
                {s.module_path}{" "}
                <span className="text-text-dim/70">·</span> {s.file_path}
              </div>
              {s.signature && (
                <pre className="text-[11px] font-mono bg-surface-sunken border border-border/60 rounded-md px-3 py-2 overflow-x-auto text-text-muted">
                  {s.signature}
                </pre>
              )}
              {s.doc_comment && (
                <p className="text-xs text-text-muted leading-relaxed line-clamp-3">
                  {s.doc_comment}
                </p>
              )}
            </Card>
          ))}
        </div>
      )}

      {!hasResults && !loading && query && (
        <Card className="flex flex-col items-center gap-2 py-8 text-center">
          <div className="grid h-10 w-10 place-items-center rounded-lg bg-surface-overlay text-text-dim">
            <Search className="h-4 w-4" />
          </div>
          <p className="text-sm font-medium text-text">No results</p>
          <p className="text-xs text-text-dim max-w-xs">
            Try different wording for a semantic query, or switch to Symbols
            for an exact name match.
          </p>
        </Card>
      )}
    </div>
  );
}

function ScoreBadge({ score }: { score: number }) {
  const pct = Math.round(score * 100);
  const tone =
    pct >= 80
      ? "bg-success/15 text-success border-success/30"
      : pct >= 60
        ? "bg-[var(--color-accent-soft)] text-accent border-[var(--color-accent-ring)]"
        : pct >= 40
          ? "bg-warning/15 text-warning border-warning/30"
          : "bg-surface-overlay text-text-dim border-border";
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-mono font-semibold tabular-nums",
        tone,
      )}
    >
      {pct}%
    </span>
  );
}
