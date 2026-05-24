"use client";

import { useCallback, useState } from "react";
import { useAuth, validateToken } from "@/lib/auth";

export function LoginForm() {
  const { login, isAuthenticated, logout, endpoint: savedEndpoint } = useAuth();
  const [token, setToken] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      const t = token.trim();
      if (!t) return;

      setBusy(true);
      setError(null);

      const ep = endpoint.trim() || undefined;
      const target =
        ep ?? process.env.NEXT_PUBLIC_MINISTR_CLOUD_BASE_URL ?? "https://mcp.ministr.ai";

      const valid = await validateToken(target, t);
      if (!valid) {
        setError(
          "Token rejected — check the key is active and the endpoint is reachable.",
        );
        setBusy(false);
        return;
      }

      login(t, ep);
      setBusy(false);

      const returnUrl = new URLSearchParams(window.location.search).get(
        "returnUrl",
      );
      if (returnUrl && returnUrl.startsWith("/")) {
        window.location.href = returnUrl;
      }
    },
    [token, endpoint, login],
  );

  if (isAuthenticated) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
        <div
          style={{
            padding: "1rem",
            border: "1px solid var(--rule)",
            borderRadius: "0.5rem",
            background: "var(--bg-2)",
          }}
        >
          <p
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "0.75rem",
              color: "var(--ink-2)",
            }}
          >
            Signed in &middot; {savedEndpoint}
          </p>
        </div>
        <button
          type="button"
          onClick={logout}
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "var(--ink-2)",
            background: "none",
            border: "1px solid var(--rule)",
            borderRadius: "0.5rem",
            padding: "0.5rem 1rem",
            cursor: "pointer",
          }}
        >
          Sign out
        </button>
      </div>
    );
  }

  return (
    <form
      onSubmit={handleSubmit}
      style={{ display: "flex", flexDirection: "column", gap: "1rem" }}
    >
      <label style={{ display: "flex", flexDirection: "column", gap: "0.375rem" }}>
        <span
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.6875rem",
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: "0.08em",
            color: "var(--ink-2)",
          }}
        >
          API key
        </span>
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="mst_pk_..."
          required
          autoComplete="off"
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.875rem",
            padding: "0.625rem 0.75rem",
            borderRadius: "0.5rem",
            border: "1px solid var(--rule)",
            background: "var(--bg-2)",
            color: "var(--ink)",
            outline: "none",
          }}
        />
      </label>

      <details>
        <summary
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.6875rem",
            color: "var(--muted)",
            cursor: "pointer",
          }}
        >
          Custom endpoint (optional)
        </summary>
        <input
          type="url"
          value={endpoint}
          onChange={(e) => setEndpoint(e.target.value)}
          placeholder="https://mcp.ministr.ai"
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.875rem",
            padding: "0.625rem 0.75rem",
            borderRadius: "0.5rem",
            border: "1px solid var(--rule)",
            background: "var(--bg-2)",
            color: "var(--ink)",
            outline: "none",
            width: "100%",
            marginTop: "0.5rem",
          }}
        />
      </details>

      {error && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.75rem",
            color: "#ef4444",
          }}
        >
          {error}
        </p>
      )}

      <button
        type="submit"
        disabled={busy || !token.trim()}
        style={{
          fontFamily: "var(--font-mono), monospace",
          fontSize: "0.875rem",
          fontWeight: 600,
          padding: "0.625rem 1.25rem",
          borderRadius: "0.5rem",
          border: "none",
          background: "var(--amber)",
          color: "var(--bg)",
          cursor: busy ? "wait" : "pointer",
          opacity: busy || !token.trim() ? 0.5 : 1,
          transition: "opacity 150ms",
        }}
      >
        {busy ? "Validating…" : "Sign in"}
      </button>
    </form>
  );
}
