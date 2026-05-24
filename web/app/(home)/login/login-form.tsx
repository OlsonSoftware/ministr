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

  const cloudBase =
    endpoint.trim() ||
    process.env.NEXT_PUBLIC_MINISTR_CLOUD_BASE_URL ||
    "https://mcp.ministr.ai";
  const callbackUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/auth/callback/`
      : "/auth/callback/";
  const githubUrl = `${cloudBase}/auth/github/start?loopback_redirect=${encodeURIComponent(callbackUrl)}&state=ministr-web`;

  return (
    <>
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", marginBottom: "1.5rem" }}>
      <a
        href={githubUrl}
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: "0.5rem",
          fontFamily: "var(--font-mono), monospace",
          fontSize: "0.875rem",
          fontWeight: 600,
          padding: "0.75rem 1.25rem",
          borderRadius: "0.5rem",
          border: "1px solid var(--rule)",
          background: "var(--bg-2)",
          color: "var(--ink)",
          textDecoration: "none",
          cursor: "pointer",
          transition: "background 150ms",
        }}
      >
        <svg width="20" height="20" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
          <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
        </svg>
        Sign in with GitHub
      </a>

      <div style={{ display: "flex", alignItems: "center", gap: "0.75rem" }}>
        <hr style={{ flex: 1, border: "none", borderTop: "1px solid var(--rule)" }} />
        <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: "0.6875rem", color: "var(--muted)" }}>
          or paste an API key
        </span>
        <hr style={{ flex: 1, border: "none", borderTop: "1px solid var(--rule)" }} />
      </div>
    </div>

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
    </>
  );
}
