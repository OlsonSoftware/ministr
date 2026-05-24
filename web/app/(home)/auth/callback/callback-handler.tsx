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
    setTimeout(() => {
      window.location.href = "/";
    }, 500);
  }, [token, error, login]);

  return (
    <main
      className="ministr-v2"
      style={{ padding: "6rem 1.5rem", textAlign: "center" }}
    >
      {status === "processing" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.875rem",
            color: "var(--ink-2)",
          }}
        >
          Completing sign-in&hellip;
        </p>
      )}
      {status === "done" && (
        <p
          style={{
            fontFamily: "var(--font-mono), monospace",
            fontSize: "0.875rem",
            color: "#22c55e",
          }}
        >
          Signed in. Redirecting&hellip;
        </p>
      )}
      {status === "error" && (
        <div style={{ display: "flex", flexDirection: "column", gap: "1rem", alignItems: "center" }}>
          <p
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "0.875rem",
              color: "#ef4444",
            }}
          >
            {error ?? "No token received."}
          </p>
          <a
            href="/login/"
            style={{
              fontFamily: "var(--font-mono), monospace",
              fontSize: "0.75rem",
              color: "var(--amber)",
              textDecoration: "underline",
            }}
          >
            Try signing in again
          </a>
        </div>
      )}
    </main>
  );
}
