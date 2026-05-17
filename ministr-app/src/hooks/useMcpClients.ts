/**
 * useMcpClients — driver for the MCP wizard.
 *
 * Wraps the three Tauri MCP commands:
 *   - `mcp_detect_clients` — list of detected clients + status
 *   - `mcp_write_config`   — write the per-client config file
 *   - `mcp_test_connection` — live verify (CLI clients) / config check
 *                              (editor clients)
 *
 * Provides a derived `state` per client (`not_installed` /
 * `not_configured` / `configured` / `connected`) so the wizard panel
 * can render without re-deriving across many places.
 */
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface McpClientInfo {
  id: string;
  display_name: string;
  installed: boolean;
  config_path: string;
  configured: boolean;
}

export interface McpTestResult {
  ok: boolean;
  message: string;
  raw_output_truncated: string | null;
  manual_verify_needed: boolean;
}

export type McpClientState =
  | "not_installed"
  | "not_configured"
  | "configured"
  | "connected";

export interface McpClientView {
  info: McpClientInfo;
  state: McpClientState;
  /** Last test result for this client, if any. */
  lastTest: McpTestResult | null;
  /** Wall-clock timestamp (ms) of the last test, if any. */
  lastTestAt: number | null;
}

export function useMcpClients(projectRoot: string | null) {
  const [clients, setClients] = useState<McpClientInfo[]>([]);
  const [tests, setTests] = useState<
    Record<string, { result: McpTestResult; at: number }>
  >({});
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!projectRoot) {
      setClients([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<McpClientInfo[]>("mcp_detect_clients", {
        projectRoot,
      });
      setClients(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [projectRoot]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const connect = useCallback(
    async (clientId: string) => {
      if (!projectRoot) return;
      setBusy(clientId);
      setError(null);
      try {
        await invoke<string>("mcp_write_config", {
          projectRoot,
          clientId,
        });
        await refresh();
        // Auto-test after writing so the user sees ✅ immediately for
        // CLI clients (and the manual-verify hint for editor clients).
        await runTest(clientId);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [projectRoot, refresh],
  );

  const runTest = useCallback(
    async (clientId: string) => {
      if (!projectRoot) return;
      setBusy(clientId);
      setError(null);
      try {
        const result = await invoke<McpTestResult>("mcp_test_connection", {
          projectRoot,
          clientId,
        });
        setTests((prev) => ({
          ...prev,
          [clientId]: { result, at: Date.now() },
        }));
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
      }
    },
    [projectRoot],
  );

  const views: McpClientView[] = clients.map((info) => {
    const test = tests[info.id];
    const lastTest = test?.result ?? null;
    const lastTestAt = test?.at ?? null;
    let state: McpClientState;
    if (!info.installed) state = "not_installed";
    else if (!info.configured) state = "not_configured";
    else if (lastTest?.ok) state = "connected";
    else state = "configured";
    return { info, state, lastTest, lastTestAt };
  });

  return {
    views,
    loading,
    busy,
    error,
    refresh,
    connect,
    runTest,
  };
}
