"use client";

import { useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useAuth } from "@/lib/auth";

export function CallbackHandler() {
  const params = useSearchParams();
  const token = params?.get("token") ?? null;
  const error = params?.get("error") ?? null;
  const { login } = useAuth();
  const [status, setStatus] = useState<"processing" | "done" | "error">(
    "processing",
  );

  useEffect(() => {
    if (error) {
      setStatus("error");
      return;
    }
    if (!token) {
      setStatus("error");
      return;
    }
    login(token);
    setStatus("done");
    // A page that initiated sign-in (e.g. /beta) can stash a return
    // path; honor same-origin paths only.
    let returnTo = "/";
    try {
      const stored = sessionStorage.getItem("ministr-auth-return");
      if (stored?.startsWith("/")) returnTo = stored;
      sessionStorage.removeItem("ministr-auth-return");
    } catch {
      /* private browsing */
    }
    setTimeout(() => {
      window.location.href = returnTo;
    }, 500);
  }, [token, error, login]);

  return (
    <section
      className="v2-section"
      style={{ paddingTop: '64px', textAlign: 'center' }}
    >
      {status === "processing" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "14px",
            color: "var(--ink-2)",
          }}
        >
          Completing sign-in...
        </p>
      )}
      {status === "done" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "14px",
            color: "#34d399",
          }}
        >
          Signed in. Redirecting...
        </p>
      )}
      {status === "error" && (
        <div style={{ display: "flex", flexDirection: "column", gap: "1rem", alignItems: "center" }}>
          <p
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "14px",
              color: "#ef4444",
            }}
          >
            {error ?? "No token received."}
          </p>
          <a
            href="/login/"
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "13px",
              color: "var(--amber)",
              textDecoration: "underline",
            }}
          >
            Try signing in again
          </a>
        </div>
      )}
    </section>
  );
}
