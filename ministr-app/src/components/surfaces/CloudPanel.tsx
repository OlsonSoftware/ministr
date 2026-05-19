/**
 * CloudPanel — settings UI for the `mcp.ministr.ai` remote MCP connection.
 *
 * v1 scope: endpoint configuration, manual Bearer-token entry, live
 * `/healthz` probe. The full OAuth deep-link flow + SSE indexer events
 * are deliberate follow-ups; this surface is the slot they land in.
 *
 * SOLID note: the panel is purely a renderer over [`cloudClient`]
 * (`src/lib/cloudClient.ts`). All Tauri ↔ HTTP plumbing lives there.
 */

import { useCallback, useEffect, useState } from "react";
import { Check, CloudOff, Loader2, RefreshCw, ShieldAlert } from "lucide-react";

import { Button } from "../ui/button";
import {
  cloudClient,
  type CloudHealth,
  type CloudStatus,
} from "../../lib/cloudClient";
import { cn } from "../../lib/utils";

const DEFAULT_ENDPOINT = "https://mcp.ministr.ai";

export function CloudPanel() {
  const [status, setStatus] = useState<CloudStatus | null>(null);
  const [endpointDraft, setEndpointDraft] = useState("");
  const [tokenDraft, setTokenDraft] = useState("");
  const [health, setHealth] = useState<CloudHealth | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    null | "save-endpoint" | "save-token" | "probe" | "disconnect"
  >(null);

  const refreshStatus = useCallback(async () => {
    const s = await cloudClient.status();
    setStatus(s);
    setEndpointDraft(s.endpoint || DEFAULT_ENDPOINT);
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const onSaveEndpoint = async () => {
    setBusy("save-endpoint");
    try {
      await cloudClient.setEndpoint(endpointDraft);
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  const onSaveToken = async () => {
    setBusy("save-token");
    try {
      await cloudClient.setBearerToken(tokenDraft);
      setTokenDraft("");
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  const onProbe = async () => {
    setBusy("probe");
    setHealthError(null);
    try {
      const h = await cloudClient.healthCheck();
      setHealth(h);
    } catch (e) {
      setHealth(null);
      setHealthError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onDisconnect = async () => {
    setBusy("disconnect");
    try {
      await cloudClient.disconnect();
      setHealth(null);
      setHealthError(null);
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex flex-col gap-6 max-w-2xl">
      <header className="flex flex-col gap-1">
        <h2 className="font-mono text-sm font-semibold uppercase tracking-[0.08em] text-text">
          ministr Cloud
        </h2>
        <p className="text-sm text-text-muted">
          Connect this desktop app to a remote ministr deployment (default:
          <span className="font-mono text-text"> mcp.ministr.ai</span>). The
          connection is per-machine; nothing is shared with other ministr
          users.
        </p>
      </header>

      <section className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Endpoint
          </span>
          <input
            type="url"
            value={endpointDraft}
            onChange={(e) => setEndpointDraft(e.target.value)}
            placeholder={DEFAULT_ENDPOINT}
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          />
        </label>
        <div className="flex gap-2">
          <Button
            size="sm"
            onClick={onSaveEndpoint}
            disabled={busy === "save-endpoint" || endpointDraft === (status?.endpoint ?? "")}
          >
            {busy === "save-endpoint" ? <Loader2 className="size-3.5 animate-spin" /> : null}
            Save endpoint
          </Button>
          <Button size="sm" variant="ghost" onClick={refreshStatus}>
            <RefreshCw className="size-3.5" />
            Reload
          </Button>
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Bearer token
          </span>
          <input
            type="password"
            value={tokenDraft}
            onChange={(e) => setTokenDraft(e.target.value)}
            placeholder={
              status?.authenticated
                ? "•••••••• (token saved — type to replace)"
                : "Paste a token from the remote OAuth flow"
            }
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          />
        </label>
        <div className="flex gap-2">
          <Button
            size="sm"
            onClick={onSaveToken}
            disabled={busy === "save-token" || tokenDraft.trim() === ""}
          >
            {busy === "save-token" ? <Loader2 className="size-3.5 animate-spin" /> : null}
            Save token
          </Button>
        </div>
        <p className="text-xs text-text-muted flex items-start gap-1.5">
          <ShieldAlert className="size-3.5 mt-0.5 shrink-0" />
          Tokens live in <span className="font-mono text-text">~/.ministr/cloud.json</span>
          {" "}with owner-only permissions. OS-keychain storage is a follow-up.
        </p>
      </section>

      <section className="flex flex-col gap-3 border-t border-border-soft pt-5">
        <div className="flex items-center justify-between">
          <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Connection
          </h3>
          <Button
            size="sm"
            variant="outline"
            onClick={onProbe}
            disabled={busy === "probe" || !status?.configured}
          >
            {busy === "probe" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Ping /healthz
          </Button>
        </div>
        <ConnectionStatus
          status={status}
          health={health}
          healthError={healthError}
        />
      </section>

      <section className="flex flex-col gap-2 border-t border-border-soft pt-5">
        <Button
          size="sm"
          variant="danger"
          onClick={onDisconnect}
          disabled={busy === "disconnect" || !status?.configured}
        >
          <CloudOff className="size-3.5" />
          Disconnect & clear local credentials
        </Button>
      </section>
    </div>
  );
}

interface ConnectionStatusProps {
  status: CloudStatus | null;
  health: CloudHealth | null;
  healthError: string | null;
}

function ConnectionStatus({ status, health, healthError }: ConnectionStatusProps) {
  if (!status?.configured) {
    return (
      <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
        No endpoint configured.
      </div>
    );
  }
  if (healthError) {
    return (
      <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-text flex items-start gap-2">
        <ShieldAlert className="size-4 mt-0.5 shrink-0 text-danger" />
        <div className="flex flex-col gap-0.5">
          <span className="font-medium">Probe failed.</span>
          <span className="font-mono text-xs text-text-muted">{healthError}</span>
        </div>
      </div>
    );
  }
  if (health) {
    return (
      <div className="rounded-md border border-border bg-surface px-3 py-2 text-sm flex items-center gap-3">
        <Check className="size-4 text-accent shrink-0" />
        <span className="text-text">{health.status}</span>
        <span className="text-text-muted">·</span>
        <LatencyChip ms={health.latency_ms} />
        <span className="text-text-muted">·</span>
        <span className="font-mono text-xs text-text-muted">v{health.version || "?"}</span>
        <span className="text-text-muted">·</span>
        <span className="text-xs text-text-muted">
          {health.corpus_count} {health.corpus_count === 1 ? "corpus" : "corpora"}
        </span>
      </div>
    );
  }
  return (
    <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
      Not yet probed. Click <span className="font-mono">Ping /healthz</span>.
    </div>
  );
}

function LatencyChip({ ms }: { ms: number }) {
  const tone =
    ms < 150 ? "text-accent" : ms < 500 ? "text-text" : "text-danger";
  return (
    <span className={cn("font-mono text-xs", tone)}>{ms} ms</span>
  );
}
