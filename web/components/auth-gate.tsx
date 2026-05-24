"use client";

import { type ReactNode } from "react";
import { useAuth } from "@/lib/auth";

interface AuthGateProps {
  children: ReactNode;
  fallback?: ReactNode;
}

export function AuthGate({ children, fallback }: AuthGateProps) {
  const { isAuthenticated } = useAuth();

  if (!isAuthenticated) {
    return (
      fallback ?? (
        <div className="flex flex-col items-center justify-center gap-4 py-20">
          <p className="font-sans text-lg text-[var(--ink-2)]">
            Sign in to access this page.
          </p>
          <a
            href="/login/"
            className="inline-block rounded-lg bg-[var(--amber)] px-5 py-2.5 font-mono text-sm font-semibold text-[var(--bg)] hover:opacity-90 transition-opacity"
          >
            Sign in
          </a>
        </div>
      )
    );
  }

  return <>{children}</>;
}
