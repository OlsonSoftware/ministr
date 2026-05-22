#!/usr/bin/env python3
"""F5.2-b/c — mock OIDC IdP for the e2e harness.

Implements the minimum subset of OIDC endpoints the cloud's /oidc/login
+ /oidc/callback handlers exercise:

- GET /.well-known/openid-configuration — discovery doc pointing at
  this server's own /authorize, /token, /jwks.json endpoints. The
  cloud's openidconnect crate fetches this once per discovery TTL
  (~1h) and caches it.

- GET /jwks.json — returns the public key the cloud uses to verify
  the ID token's signature. Reads the JWK n/e from env vars set by
  the harness (which generated the keypair with openssl genrsa).

- GET /authorize?…&state=X&nonce=Y&redirect_uri=Z — "auto-approve"
  mode: immediately redirects (302) to `Z?code=<code>&state=X`. The
  code is opaque; we store `code -> nonce` in-process so /token can
  echo the right nonce back into the JWT.

- POST /token (form-encoded) — exchanges `code` for `{access_token,
  token_type, id_token, expires_in}`. The id_token is an RS256 JWT
  carrying `iss`, `sub`, `aud`, `exp`, `iat`, `nonce`, `email`,
  `email_verified=true`. Signed via `openssl dgst -sha256 -sign`
  against the harness's private PEM (no Python crypto deps).

Env vars (all set by `scripts/e2e-cloud-local.sh`):

  OIDC_PRIVATE_KEY_PATH  — absolute path to the RSA-2048 private PEM
  OIDC_JWK_N             — base64url-encoded modulus (no padding)
  OIDC_JWK_E             — base64url-encoded exponent (e.g. "AQAB")
  OIDC_JWK_KID           — key id string, included in JWT header
  OIDC_FIXED_EMAIL       — email claim minted into every ID token
  OIDC_FIXED_SUBJECT     — sub claim minted into every ID token
  OIDC_FIXED_CLIENT_ID   — aud claim minted into every ID token
                           (must match `org_oidc_configs.client_id`)
  OIDC_FIXED_GROUPS      — F5.2-f. Comma-separated list of group
                           names to include in the JWT's `groups`
                           claim. Empty / unset emits no groups
                           claim so the callback's group-role-map
                           extraction sees an empty user-group set
                           (and the no-mapping path stays valid).

Usage:
    python3 scripts/e2e-oidc-mock-idp.py <port>
"""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit, urlunsplit


def b64url(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode("ascii")


def sign_rs256(private_key_path: str, signing_input: bytes) -> bytes:
    """Sign `signing_input` with RS256 by shelling out to openssl.

    Returns raw signature bytes (not base64-encoded). Raises on
    openssl failure — the harness's preflight should have caught a
    missing key or broken openssl.
    """
    proc = subprocess.run(
        [
            "openssl",
            "dgst",
            "-sha256",
            "-sign",
            private_key_path,
        ],
        input=signing_input,
        check=True,
        capture_output=True,
    )
    return proc.stdout


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: e2e-oidc-mock-idp.py <port>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    # Issuer must match the harness's `org_oidc_configs.issuer_url`
    # byte-for-byte (OIDC Discovery 1.0 §4.3 — openidconnect-rs rejects
    # a discovery doc whose `issuer` differs from the URL prefix used
    # to discover it). The harness inserts `http://127.0.0.1:${PORT}`
    # so we use 127.0.0.1 here too.
    base = f"http://127.0.0.1:{port}"

    private_key_path = os.environ.get("OIDC_PRIVATE_KEY_PATH", "")
    jwk_n = os.environ.get("OIDC_JWK_N", "")
    jwk_e = os.environ.get("OIDC_JWK_E", "AQAB")
    jwk_kid = os.environ.get("OIDC_JWK_KID", "e2e-key")
    fixed_email = os.environ.get("OIDC_FIXED_EMAIL", "oidc-test@e2e.test")
    fixed_subject = os.environ.get("OIDC_FIXED_SUBJECT", "e2e-subject-1")
    fixed_client_id = os.environ.get("OIDC_FIXED_CLIENT_ID", "e2e-client")
    fixed_groups_raw = os.environ.get("OIDC_FIXED_GROUPS", "")
    fixed_groups: list[str] = [
        g.strip() for g in fixed_groups_raw.split(",") if g.strip()
    ]

    if private_key_path and not os.path.exists(private_key_path):
        print(
            f"warning: OIDC_PRIVATE_KEY_PATH={private_key_path} does not exist;"
            " /token will fail",
            file=sys.stderr,
        )

    # `code -> {nonce, state}` map populated by /authorize and drained
    # by /token. Codes are single-use; we remove on consumption.
    codes: dict[str, dict[str, str]] = {}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: object) -> None:
            return

        def _send_json(self, body: bytes, status: int = 200) -> None:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _redirect(self, location: str) -> None:
            self.send_response(302)
            self.send_header("Location", location)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def do_GET(self) -> None:  # noqa: N802
            split = urlsplit(self.path)
            path = split.path

            if path == "/.well-known/openid-configuration":
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
                self._send_json(body)
                return

            if path == "/jwks.json":
                # JWKS shape per RFC 7517. `alg` + `use` + `kid` let the
                # openidconnect crate find a candidate key without having
                # to try every entry.
                jwk = {"kty": "RSA", "use": "sig", "alg": "RS256", "kid": jwk_kid}
                if jwk_n:
                    jwk["n"] = jwk_n
                if jwk_e:
                    jwk["e"] = jwk_e
                body = json.dumps({"keys": [jwk] if jwk_n else []}).encode("utf-8")
                self._send_json(body)
                return

            if path == "/authorize":
                qs = parse_qs(split.query)
                state = (qs.get("state") or [""])[0]
                nonce = (qs.get("nonce") or [""])[0]
                redirect_uri = (qs.get("redirect_uri") or [""])[0]
                if not redirect_uri:
                    self._send_json(b'{"error":"missing redirect_uri"}', status=400)
                    return
                code = f"e2e-code-{uuid.uuid4().hex[:16]}"
                codes[code] = {"nonce": nonce, "state": state}
                target_split = urlsplit(redirect_uri)
                # Preserve any existing query on redirect_uri; append
                # `code` + `state` as additional params.
                existing = parse_qs(target_split.query)
                # parse_qs returns lists; collapse to scalars for our
                # additions.
                params = [(k, v[0]) for k, v in existing.items() if v]
                params.append(("code", code))
                if state:
                    params.append(("state", state))
                new_query = urlencode(params, doseq=False)
                location = urlunsplit(
                    (
                        target_split.scheme,
                        target_split.netloc,
                        target_split.path,
                        new_query,
                        target_split.fragment,
                    )
                )
                self._redirect(location)
                return

            self.send_response(404)
            self.end_headers()

        def do_POST(self) -> None:  # noqa: N802
            split = urlsplit(self.path)
            path = split.path
            if path != "/token":
                self.send_response(404)
                self.end_headers()
                return

            content_length = int(self.headers.get("Content-Length", "0") or "0")
            raw = self.rfile.read(content_length) if content_length else b""
            form = parse_qs(raw.decode("utf-8", errors="replace"))
            code = (form.get("code") or [""])[0]
            entry = codes.pop(code, None)
            if entry is None:
                self._send_json(
                    b'{"error":"invalid_grant","error_description":"unknown code"}',
                    status=400,
                )
                return

            now = int(time.time())
            header = {"alg": "RS256", "typ": "JWT", "kid": jwk_kid}
            payload = {
                "iss": base,
                "sub": fixed_subject,
                "aud": fixed_client_id,
                "exp": now + 600,
                "iat": now,
                "nonce": entry["nonce"],
                "email": fixed_email,
                "email_verified": True,
            }
            # F5.2-f — include the groups claim only when at least
            # one group is configured. Empty/unset → no claim, so
            # the callback's group-role-map extraction sees an empty
            # array and falls through to the no-mapping path. Lets
            # the existing F5.2-b/c/d assertions stay valid.
            if fixed_groups:
                payload["groups"] = list(fixed_groups)
            header_b64 = b64url(json.dumps(header, separators=(",", ":")).encode())
            payload_b64 = b64url(
                json.dumps(payload, separators=(",", ":")).encode()
            )
            signing_input = f"{header_b64}.{payload_b64}".encode("ascii")
            try:
                sig = sign_rs256(private_key_path, signing_input)
            except Exception as e:  # noqa: BLE001
                self._send_json(
                    json.dumps(
                        {"error": "server_error", "error_description": str(e)}
                    ).encode(),
                    status=500,
                )
                return
            id_token = f"{header_b64}.{payload_b64}.{b64url(sig)}"
            body = json.dumps(
                {
                    "access_token": "e2e-access-token",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "id_token": id_token,
                }
            ).encode("utf-8")
            self._send_json(body)

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
