#!/usr/bin/env python3
"""F5.2-b — tiny mock OIDC IdP for the e2e harness.

Serves the minimum subset of OIDC endpoints the cloud's /oidc/login
handler exercises:

- `GET /.well-known/openid-configuration` — issues a discovery doc
  pointing at this server's own /authorize, /token, /jwks.json
  endpoints. The cloud's openidconnect crate fetches this once per
  discovery TTL and caches it.
- `GET /jwks.json` — returns an empty `{"keys": []}` for now. F5.2-c
  will replace this with a fixture RSA public key so the cloud can
  verify a mocked ID token signed by the harness.
- `GET /authorize` — placeholder; F5.2-c uses it.
- `POST /token` — placeholder; F5.2-c uses it.

The handler logs minimal stderr; the harness greps for the harness's
own assertion output, not for this server's logs.

Usage:
    python3 scripts/e2e-oidc-mock-idp.py <port>
"""

from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: e2e-oidc-mock-idp.py <port>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    # Issuer must match the harness's `org_oidc_configs.issuer_url`
    # byte-for-byte (OIDC Discovery 1.0 §4.3 — openidconnect-rs rejects
    # a discovery doc whose `issuer` differs from the URL prefix used
    # to discover it). The harness inserts `http://127.0.0.1:${PORT}`
    # so we use 127.0.0.1 here too. The server binds to 127.0.0.1
    # below as well, so "localhost" and "127.0.0.1" both resolve, but
    # the JSON must use the canonical form the harness configured.
    base = f"http://127.0.0.1:{port}"

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: object) -> None:
            return

        def do_GET(self) -> None:  # noqa: N802
            if self.path == "/.well-known/openid-configuration":
                body = json.dumps(
                    {
                        "issuer": base,
                        "authorization_endpoint": f"{base}/authorize",
                        "token_endpoint": f"{base}/token",
                        "jwks_uri": f"{base}/jwks.json",
                        "response_types_supported": ["code"],
                        "subject_types_supported": ["public"],
                        "id_token_signing_alg_values_supported": ["RS256"],
                        "scopes_supported": ["openid", "email", "profile"],
                    }
                ).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if self.path == "/jwks.json":
                body = json.dumps({"keys": []}).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            self.send_response(404)
            self.end_headers()

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
