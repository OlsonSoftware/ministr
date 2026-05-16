/**
 * AiAssistantsPanel — in-app MCP setup wizard.
 *
 * One row per supported client (Claude Code, Cursor, VS Code Copilot,
 * Codex). Each row shows the detected install + configuration state,
 * the path of the file ministr would write or has written, and the
 * actions appropriate for the row's state. The Connect action writes
 * the config file via the Rust `mcp_write_config` command and
 * auto-tests the connection.
 *
 * Used in two places:
 *   - Settings → AI assistants (this component, via SettingsSurface)
 *   - Onboarding step 3 (planned — currently a stub; can drop this
 *     component in once the surface is ready)
 */
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  Check,
  Loader2,
  RefreshCw,
  Sparkles,
} from "lucide-react";

import type { CorpusInfo } from "../../lib/types";
import { corpusRoot } from "../../lib/corpus";
import { cn } from "../../lib/utils";
import {
  useMcpClients,
  type McpClientState,
  type McpClientView,
} from "../../hooks/useMcpClients";
import { Button } from "../ui/button";

interface Props {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
}

export function AiAssistantsPanel({ corpora, activeCorpusId }: Props) {
  const corpus =
    corpora.find((c) => c.id === activeCorpusId) ?? corpora[0] ?? null;
  const projectRoot = corpus ? corpusRoot(corpus.paths) : null;

  const { views, loading, busy, error, connect, runTest, refresh } =
    useMcpClients(projectRoot);

  if (!corpus) {
    return (
      <div className="space-y-4">
        <Header />
        <div className="border border-border-soft bg-surface p-4">
          <p className="font-sans text-sm text-text-muted">
            Add a project first — the wizard writes per-project config files
            (and a user-global one for Codex). Visit Projects to add one.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <Header projectRoot={projectRoot} onRefresh={refresh} loading={loading} />

      {error && (
        <div className="border border-danger bg-surface p-3 flex items-start gap-2">
          <AlertTriangle
            className="h-4 w-4 text-danger shrink-0 mt-0.5"
            strokeWidth={2.5}
          />
          <p className="font-mono text-mono-mini text-danger">{error}</p>
        </div>
      )}

      <ul className="space-y-2.5">
        {views.map((view) => (
          <ClientRow
            key={view.info.id}
            view={view}
            busy={busy === view.info.id}
            onConnect={() => connect(view.info.id)}
            onTest={() => runTest(view.info.id)}
          />
        ))}
      </ul>
    </div>
  );
}

function Header({
  projectRoot,
  onRefresh,
  loading,
}: {
  projectRoot?: string | null;
  onRefresh?: () => void;
  loading?: boolean;
}) {
  return (
    <header className="flex items-start justify-between gap-3">
      <div className="space-y-1">
        <h2 className="font-mono text-sm font-bold uppercase tracking-[0.08em] text-text">
          AI assistants
        </h2>
        <p className="font-sans text-sm text-text-muted">
          Connect ministr to the AI tools you use — one click each.
        </p>
        {projectRoot && (
          <p className="font-mono text-mono-mini text-text-dim truncate max-w-[60ch]">
            Configuring against{" "}
            <span className="text-text">{projectRoot}</span>
          </p>
        )}
      </div>
      {onRefresh && (
        <Button
          variant="outline"
          size="sm"
          onClick={onRefresh}
          disabled={loading}
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
          ) : (
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
          )}
          Refresh
        </Button>
      )}
    </header>
  );
}

function ClientRow({
  view,
  busy,
  onConnect,
  onTest,
}: {
  view: McpClientView;
  busy: boolean;
  onConnect: () => void;
  onTest: () => void;
}) {
  const { info, state, lastTest, lastTestAt } = view;
  const tone = stateTone(state);

  return (
    <li
      className={cn(
        "border bg-surface p-4 space-y-3",
        tone === "success" && "border-success",
        tone === "warning" && "border-warning",
        tone === "muted" && "border-border-soft",
        tone === "danger" && "border-border-soft",
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <StateDot tone={tone} />
            <span className="font-mono text-sm font-bold tracking-[0.08em] text-text">
              {info.display_name}
            </span>
          </div>
          <p className="font-mono text-mono-mini text-text-dim mt-1">
            {stateLabel(state)} ·{" "}
            <span className="text-text">{info.config_path}</span>
          </p>
          {lastTest && lastTestAt && (
            <p
              className={cn(
                "font-mono text-mono-mini mt-1",
                lastTest.ok ? "text-success" : "text-text-muted",
              )}
            >
              {formatTestStamp(lastTestAt)} · {lastTest.message}
            </p>
          )}
          {lastTest?.manual_verify_needed && (
            <p className="font-sans italic text-mono-mini text-text-muted mt-1">
              Editor clients can't be reached programmatically. Restart{" "}
              {info.display_name} and hit Re-test to confirm it picked up the
              new config.
            </p>
          )}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <RowActions
            state={state}
            busy={busy}
            configPath={info.config_path}
            onConnect={onConnect}
            onTest={onTest}
          />
        </div>
      </div>
    </li>
  );
}

function RowActions({
  state,
  busy,
  configPath,
  onConnect,
  onTest,
}: {
  state: McpClientState;
  busy: boolean;
  configPath: string;
  onConnect: () => void;
  onTest: () => void;
}) {
  if (state === "not_installed") {
    return (
      <span className="font-mono text-mono-mini italic text-text-dim">
        Not installed
      </span>
    );
  }

  if (state === "not_configured") {
    return (
      <Button size="sm" onClick={onConnect} disabled={busy}>
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
        ) : (
          <Sparkles className="h-3.5 w-3.5" strokeWidth={2} />
        )}
        Connect
      </Button>
    );
  }

  // configured | connected — show re-test + open file
  return (
    <>
      <Button variant="outline" size="sm" onClick={onTest} disabled={busy}>
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
        ) : (
          <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
        )}
        Re-test
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          invoke("open_path", { path: configPath }).catch(() => {
            /* swallow — user gets no feedback if open_path fails, which
             * is fine for a convenience action. The path itself is shown
             * directly above the button so they can copy it manually. */
          });
        }}
      >
        Open file
      </Button>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────────────
// Visual helpers

function StateDot({
  tone,
}: {
  tone: "success" | "warning" | "muted" | "danger";
}) {
  if (tone === "success") {
    return (
      <span className="inline-flex h-4 w-4 items-center justify-center bg-success text-white">
        <Check className="h-3 w-3" strokeWidth={3} />
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      className={cn(
        "h-2.5 w-2.5 rounded-full inline-block",
        tone === "warning" && "bg-warning",
        tone === "muted" && "bg-text-dim",
        tone === "danger" && "bg-danger",
      )}
    />
  );
}

function stateTone(
  state: McpClientState,
): "success" | "warning" | "muted" | "danger" {
  switch (state) {
    case "connected":
      return "success";
    case "configured":
      return "warning";
    case "not_configured":
      return "muted";
    case "not_installed":
      return "danger";
  }
}

function stateLabel(state: McpClientState): string {
  switch (state) {
    case "connected":
      return "Connected";
    case "configured":
      return "Config written, manual verification needed";
    case "not_configured":
      return "Not configured";
    case "not_installed":
      return "Not installed";
  }
}

function formatTestStamp(at: number): string {
  const diff = (Date.now() - at) / 1000;
  if (diff < 60) return `${Math.max(1, Math.round(diff))}s ago`;
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`;
  return new Date(at).toLocaleTimeString();
}
