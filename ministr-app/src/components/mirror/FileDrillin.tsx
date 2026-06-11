import { useEffect, useState } from "react";
import { indexedFile, readFile } from "../../lib/ipc";
import type { IndexedFileResponse } from "../../lib/ipc";
import type { TrustState } from "../ui/trust";
import { TrustMark } from "../ui/TrustMark";
import { ActionChip } from "../ui/ActionChip";
import { StatusBanner } from "../ui/StatusBanner";

/**
 * File drill-in (gui-rw-file-drillin) — the file as your AI sees it:
 * the STORED sections retrieval actually serves, never re-derived.
 * Stale files get an honest banner + a toggle to your current file.
 */
export function FileDrillin({
  corpusId,
  path,
  state,
  onBack,
}: {
  corpusId: string;
  path: string;
  state: TrustState;
  onBack: () => void;
}) {
  const [aiView, setAiView] = useState<IndexedFileResponse | null>(null);
  const [diskView, setDiskView] = useState<string | null>(null);
  const [pane, setPane] = useState<"ai" | "mine">("ai");

  useEffect(() => {
    let alive = true;
    void indexedFile(corpusId, path).then((r) => alive && setAiView(r));
    return () => {
      alive = false;
    };
  }, [corpusId, path]);

  useEffect(() => {
    if (pane !== "mine" || diskView !== null) return;
    let alive = true;
    void readFile(corpusId, path)
      .then((r) => alive && setDiskView(r.content))
      .catch(() => alive && setDiskView("(couldn’t read the file from disk)"));
    return () => {
      alive = false;
    };
  }, [pane, diskView, corpusId, path]);

  const name = path.split("/").pop() ?? path;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <header className="flex items-center gap-3">
        <ActionChip onClick={onBack} aria-label="back to the file tree">
          ‹
        </ActionChip>
        <TrustMark state={state} />
        <h2 className="font-mono text-base text-ink">{name}</h2>
      </header>

      {state === "stale" ? (
        <StatusBanner
          state="stale"
          headline="Your AI sees an older version of this file"
          sub="you changed it after ministr last read it — Catch up on the project page fixes this"
          action={
            <div className="flex gap-2">
              <ActionChip
                variant={pane === "ai" ? "primary" : "quiet"}
                onClick={() => setPane("ai")}
              >
                As your AI sees it
              </ActionChip>
              <ActionChip
                variant={pane === "mine" ? "primary" : "quiet"}
                onClick={() => setPane("mine")}
              >
                My current file
              </ActionChip>
            </div>
          }
        />
      ) : null}

      {pane === "mine" ? (
        <pre className="overflow-auto rounded-lg border border-line bg-sunken p-3 font-mono text-xs text-ink">
          {diskView ?? "loading…"}
        </pre>
      ) : (
        <section
          aria-label="the file as your AI sees it"
          className="space-y-3 overflow-auto"
        >
          {aiView?.found === false ? (
            <p className="text-sm text-dim">
              your AI hasn’t seen this file yet — it isn’t in the index
            </p>
          ) : null}
          {aiView?.sections.map((s, i) => (
            <div
              key={i}
              className="rounded-lg border border-line bg-surface p-3"
            >
              {s.heading ? (
                <p className="mb-2 text-xs uppercase tracking-[0.08em] text-dim">
                  {s.heading}
                </p>
              ) : null}
              <pre className="overflow-x-auto font-mono text-xs whitespace-pre-wrap text-ink">
                {s.text}
              </pre>
            </div>
          ))}
          {aiView === null ? <p className="text-sm text-dim">loading…</p> : null}
        </section>
      )}
    </div>
  );
}
