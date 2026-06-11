"use client";

import { useCallback, useEffect, useState } from "react";
import { useAuth } from "@/lib/auth";

type BetaStatus = "requested" | "approved" | "revoked" | null;

const mono: React.CSSProperties = {
  fontFamily: "var(--font-mono), monospace",
};

const inputStyle: React.CSSProperties = {
  ...mono,
  fontSize: "14px",
  padding: "0.625rem 0.75rem",
  border: "1px solid var(--rule)",
  background: "var(--bg-2)",
  color: "var(--ink)",
  outline: "none",
};

function FeedbackLine() {
  return (
    <p style={{ ...mono, fontSize: "13px", color: "var(--ink-2)" }}>
      Found a bug or have feedback? Write to{" "}
      <a
        href="mailto:beta@ministr.ai"
        style={{ color: "var(--amber)", textDecoration: "underline" }}
      >
        beta@ministr.ai
      </a>
      .
    </p>
  );
}

/** Request-access form — the signed-out (and not-yet-requested) view. */
function RequestForm({ endpoint }: { endpoint: string }) {
  const [email, setEmail] = useState("");
  const [githubLogin, setGithubLogin] = useState("");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      setBusy(true);
      setError(null);
      try {
        const res = await fetch(`${endpoint}/api/v1/beta/request`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            email: email.trim(),
            github_login: githubLogin.trim() || undefined,
          }),
        });
        if (!res.ok) {
          const body = (await res.json().catch(() => null)) as {
            error?: string;
          } | null;
          setError(body?.error ?? `Request failed (${res.status}).`);
          setBusy(false);
          return;
        }
        setDone(true);
      } catch {
        setError("Could not reach the server -- try again in a minute.");
      }
      setBusy(false);
    },
    [email, githubLogin, endpoint],
  );

  if (done) {
    return (
      <div
        style={{
          padding: "1rem",
          border: "1px solid var(--rule)",
          background: "var(--bg-2)",
        }}
      >
        <p style={{ ...mono, fontSize: "13px", color: "var(--ink)" }}>
          Request received. We&apos;ll email you when you&apos;re in --
          then come back here and sign in with GitHub.
        </p>
      </div>
    );
  }

  return (
    <form
      onSubmit={handleSubmit}
      style={{ display: "flex", flexDirection: "column", gap: "1rem" }}
    >
      <label
        style={{ display: "flex", flexDirection: "column", gap: "0.375rem" }}
      >
        <span className="v2-meta">Email</span>
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          required
          autoComplete="email"
          style={inputStyle}
        />
      </label>
      <label
        style={{ display: "flex", flexDirection: "column", gap: "0.375rem" }}
      >
        <span className="v2-meta">GitHub username (optional)</span>
        <input
          type="text"
          value={githubLogin}
          onChange={(e) => setGithubLogin(e.target.value)}
          placeholder="octocat"
          autoComplete="off"
          style={inputStyle}
        />
      </label>
      {error && (
        <p style={{ ...mono, fontSize: "13px", color: "#ef4444" }}>{error}</p>
      )}
      <button
        type="submit"
        disabled={busy || !email.trim()}
        className="v2-btn v2-btn-primary"
        style={{
          cursor: busy ? "wait" : "pointer",
          opacity: busy || !email.trim() ? 0.5 : 1,
          transition: "opacity 150ms",
        }}
      >
        {busy ? "Sending..." : "Request access"}
      </button>
    </form>
  );
}

export function BetaClient() {
  const { token, endpoint, isAuthenticated, logout } = useAuth();
  const [status, setStatus] = useState<BetaStatus>(null);
  const [phase, setPhase] = useState<"loading" | "signed-out" | "ready">(
    "loading",
  );
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    if (!isAuthenticated || !token) {
      // useAuth hydrates from localStorage in an effect; give it a tick
      // before concluding the visitor is signed out.
      const t = setTimeout(() => {
        if (!cancelled) setPhase("signed-out");
      }, 150);
      return () => {
        cancelled = true;
        clearTimeout(t);
      };
    }
    (async () => {
      try {
        const res = await fetch(`${endpoint}/api/v1/beta/status`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (cancelled) return;
        if (res.status === 401) {
          logout();
          setPhase("signed-out");
          return;
        }
        const body = (await res.json().catch(() => null)) as {
          status?: BetaStatus;
        } | null;
        setStatus(body?.status ?? null);
        setPhase("ready");
      } catch {
        if (!cancelled) {
          // Status unknown (offline / endpoint down) -- show the
          // signed-in view with no entitlement rather than a dead end.
          setStatus(null);
          setPhase("ready");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isAuthenticated, token, endpoint, logout]);

  const handleSignIn = useCallback(() => {
    try {
      sessionStorage.setItem("ministr-auth-return", "/beta/");
    } catch {
      /* private browsing */
    }
    const callbackUrl = `${window.location.origin}/auth/callback/`;
    window.location.href = `${endpoint}/auth/github/start?loopback_redirect=${encodeURIComponent(callbackUrl)}&state=ministr-web`;
  }, [endpoint]);

  const handleDownload = useCallback(async () => {
    if (!token) return;
    setDownloading(true);
    setDownloadError(null);
    try {
      const res = await fetch(
        `${endpoint}/api/v1/beta/download?format=json`,
        { headers: { Authorization: `Bearer ${token}` } },
      );
      if (!res.ok) {
        const body = (await res.json().catch(() => null)) as {
          error?: string;
        } | null;
        setDownloadError(body?.error ?? `Download failed (${res.status}).`);
        setDownloading(false);
        return;
      }
      const body = (await res.json()) as { url: string; name: string };
      window.location.assign(body.url);
    } catch {
      setDownloadError("Could not reach the server -- try again in a minute.");
    }
    setDownloading(false);
  }, [token, endpoint]);

  if (phase === "loading") {
    return (
      <p style={{ ...mono, fontSize: "13px", color: "var(--ink-2)" }}>
        Loading...
      </p>
    );
  }

  if (phase === "signed-out") {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem" }}>
        <RequestForm endpoint={endpoint} />
        <div style={{ display: "flex", alignItems: "center", gap: "0.75rem" }}>
          <hr
            style={{ flex: 1, border: "none", borderTop: "1px solid var(--rule)" }}
          />
          <span
            style={{
              ...mono,
              fontSize: "12px",
              letterSpacing: "0.04em",
              color: "var(--ink-2)",
            }}
          >
            already approved?
          </span>
          <hr
            style={{ flex: 1, border: "none", borderTop: "1px solid var(--rule)" }}
          />
        </div>
        <button
          type="button"
          onClick={handleSignIn}
          className="v2-btn"
          style={{ cursor: "pointer" }}
        >
          Sign in with GitHub
        </button>
      </div>
    );
  }

  // phase === "ready": signed in, entitlement known.
  if (status === "approved") {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem" }}>
        <div
          style={{
            padding: "1rem",
            border: "1px solid var(--rule)",
            background: "var(--bg-2)",
          }}
        >
          <p style={{ ...mono, fontSize: "13px", color: "var(--ink)" }}>
            You&apos;re in. The build is signed and notarized; macOS will
            verify it on first launch.
          </p>
        </div>
        <button
          type="button"
          onClick={handleDownload}
          disabled={downloading}
          className="v2-btn v2-btn-primary"
          style={{
            cursor: downloading ? "wait" : "pointer",
            opacity: downloading ? 0.5 : 1,
            transition: "opacity 150ms",
          }}
        >
          {downloading ? "Fetching build..." : "Download for macOS"}
        </button>
        {downloadError && (
          <p style={{ ...mono, fontSize: "13px", color: "#ef4444" }}>
            {downloadError}
          </p>
        )}
        <FeedbackLine />
      </div>
    );
  }

  if (status === "requested") {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem" }}>
        <div
          style={{
            padding: "1rem",
            border: "1px solid var(--rule)",
            background: "var(--bg-2)",
          }}
        >
          <p style={{ ...mono, fontSize: "13px", color: "var(--ink)" }}>
            Your request is in the queue. We&apos;ll email you when
            you&apos;re approved -- this page unlocks automatically.
          </p>
        </div>
        <FeedbackLine />
      </div>
    );
  }

  if (status === "revoked") {
    return (
      <div
        style={{
          padding: "1rem",
          border: "1px solid var(--rule)",
          background: "var(--bg-2)",
        }}
      >
        <p style={{ ...mono, fontSize: "13px", color: "var(--ink)" }}>
          Beta access for this account has ended. Questions?{" "}
          <a
            href="mailto:beta@ministr.ai"
            style={{ color: "var(--amber)", textDecoration: "underline" }}
          >
            beta@ministr.ai
          </a>
          .
        </p>
      </div>
    );
  }

  // Signed in, no allowlist row yet.
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem" }}>
      <p style={{ ...mono, fontSize: "13px", color: "var(--ink-2)" }}>
        You&apos;re signed in, but this account isn&apos;t on the beta
        list yet. Request access below.
      </p>
      <RequestForm endpoint={endpoint} />
    </div>
  );
}
