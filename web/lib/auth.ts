"use client";

import { useCallback, useEffect, useState } from "react";

const TOKEN_KEY = "ministr-auth-token";
const ENDPOINT_KEY = "ministr-auth-endpoint";

const DEFAULT_ENDPOINT =
  process.env.NEXT_PUBLIC_MINISTR_CLOUD_BASE_URL ?? "https://mcp.ministr.ai";

export interface AuthState {
  token: string | null;
  endpoint: string;
  isAuthenticated: boolean;
  login: (token: string, endpoint?: string) => void;
  logout: () => void;
}

export function useAuth(): AuthState {
  const [token, setToken] = useState<string | null>(null);
  const [endpoint, setEndpoint] = useState(DEFAULT_ENDPOINT);

  useEffect(() => {
    try {
      const t = localStorage.getItem(TOKEN_KEY);
      const e = localStorage.getItem(ENDPOINT_KEY);
      if (t) setToken(t);
      if (e) setEndpoint(e);
    } catch {
      /* SSR or private browsing */
    }
  }, []);

  const login = useCallback((t: string, e?: string) => {
    try {
      localStorage.setItem(TOKEN_KEY, t);
      if (e) localStorage.setItem(ENDPOINT_KEY, e);
      setToken(t);
      if (e) setEndpoint(e);
    } catch {
      /* private browsing */
    }
  }, []);

  const logout = useCallback(() => {
    try {
      localStorage.removeItem(TOKEN_KEY);
      localStorage.removeItem(ENDPOINT_KEY);
      setToken(null);
      setEndpoint(DEFAULT_ENDPOINT);
    } catch {
      /* private browsing */
    }
  }, []);

  return {
    token,
    endpoint,
    isAuthenticated: token !== null,
    login,
    logout,
  };
}

export async function validateToken(
  endpoint: string,
  token: string,
): Promise<boolean> {
  try {
    const res = await fetch(`${endpoint}/api/v1/corpora`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.ok;
  } catch {
    return false;
  }
}
